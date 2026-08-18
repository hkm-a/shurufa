//! AI 帮写面板：Ctrl+Shift+W 呼出，内嵌题词、调用 Agnes API、回车粘贴草稿。
//!
//! 与 `panel.rs` 共享"置顶弹窗 + 消息循环"骨架，差异在于本面板是
//! 单输入框 + 状态机（输入/请求中/预览/错误），网络走 ureq（HTTP
//! 同步客户端），异步通过 `std::thread::spawn` + `PostMessageW` 回到
//! UI 线程。API key 从环境变量 `AGNES_API_KEY` 读取，**永不落盘、
//! 永不混进日志**；缺环境变量时面板只显示配置提示，不发请求。
//!
//! 本轮改动摘要（皮肤 v2 / 现代化外观 / 主题热切换）：
//! - 删除硬编码 COLOR_* 常量；颜色统一来自共享皮肤（`crate::panel::skin`），
//!   按系统 light/dark 变体取色，主题切换即时生效。
//! - 字号乘 `metrics.font_scale`；`metrics.opacity` < 1 时整体透明；
//!   `apply_appearance` 应用 Win11 圆角 + 深色边框，`ShadowShell` 画阴影。
//! - 新增 `on_theme_changed()`：由 panel.rs 的主题监听窗口统一触发重设+重绘。

use std::cell::RefCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_CENTER, DT_END_ELLIPSIS,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_WORDBREAK, FW_BOLD, FW_NORMAL, HBRUSH, HDC, HFONT,
    HGDIOBJ, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SendInput, SetFocus, UnregisterHotKey, INPUT, INPUT_0, INPUT_KEYBOARD,
    KEYBDINPUT, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_SHIFT, VIRTUAL_KEY, VK_BACK, VK_CONTROL,
    VK_ESCAPE, VK_RETURN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo,
    GetSystemMetrics, GetWindowRect, GetWindowThreadProcessId, KillTimer, LoadCursorW, MoveWindow,
    PostMessageW, RegisterClassW, SetForegroundWindow, SetTimer, ShowWindow, CS_HREDRAW,
    CS_VREDRAW, GUITHREADINFO, IDC_ARROW, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOWNA, WM_APP,
    WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_PAINT, WM_SETTINGCHANGE, WM_TIMER,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::panel::skin::{self, ShadowShell, Skin};

pub const HOTKEY_ID: i32 = 2;
/// 划词润色热键：Ctrl+Shift+R 抓取前台选区进面板润色，回车覆盖选区。
pub const POLISH_HOTKEY_ID: i32 = 3;
/// 划词翻译热键：Ctrl+Shift+T 抓取前台选区进面板翻译，回车覆盖选区。
pub const TRANSLATE_HOTKEY_ID: i32 = 4;
/// Worker 线程把网络结果转回 UI 线程的私有消息。
const WM_AI_DONE: u32 = WM_APP + 71;
/// 流式增量包：LPARAM 携带 Box<(String /*已累积全文*/, bool /*is_final*/)>。
const WM_AI_CHUNK: u32 = WM_APP + 72;
/// 外部进程（设置中心）经 `shurufa-host ai show` 投递的唤起消息。
pub const WM_AI_EXTERNAL_SHOW: u32 = WM_APP + 73;
/// M9-4：窗口跟随定时器（200ms 轮询输入锚点重定位，光标移动面板跟随）。
const AI_FOLLOW_TIMER_ID: usize = 0x5A11;

/// 内置系统提示（默认"正式"）：控制输出端为"可直接粘贴的中文文本片段"。
pub(crate) const SYSTEM_PROMPT: &str = SYSTEM_PROMPT_FORMAL;
/// 提示词模板：按 TEMPLATES 下标索引；尾部约束统一保持"可直接粘贴中文段落"。
const SYSTEM_PROMPT_FORMAL: &str = "你是用户输入法里的‘AI 帮写’助手。用正式、简洁的中文书面语写作，直接输出可粘贴的中文段落，不要解释、不要 Markdown 代码块；除非用户另有要求，控制在 300 字以内。";
const SYSTEM_PROMPT_CHAT: &str = "你是用户输入法里的‘AI 帮写’助手。用轻松自然的口吻聊天，直接输出可粘贴的中文段落，不要解释、不要 Markdown 代码块；除非用户另有要求，控制在 300 字以内。";
const SYSTEM_PROMPT_MAIL: &str = "你是用户输入法里的‘AI 帮写’助手。生成结构完整、礼貌得体的中文邮件正文，直接输出可粘贴的中文段落，不要解释、不要 Markdown 代码块；除非用户另有要求，控制在 300 字以内。";
const SYSTEM_PROMPT_EMOJI: &str = "你是用户输入法里的‘AI 帮写’助手。输出活泼的中文并适量穿插 emoji，直接输出可粘贴的中文段落，不要解释、不要 Markdown 代码块；除非用户另有要求，控制在 300 字以内。";
/// 划词翻译系统提示：把选区原文翻译成中文（原文已是中文时译成英文）。
const SYSTEM_PROMPT_TRANSLATE: &str = "你是用户输入法里的划词翻译助手。把用户选中/输入的文本翻译成中文；若原文已是中文则翻译成英文。只输出译文本身，不要解释、不要加引号、不要 Markdown 代码块；控制在 500 字以内。";
/// 模板 chips: (标签, 系统提示)。选择只影响下一次请求。
const TEMPLATES: &[(&str, &str)] = &[
    ("正式", SYSTEM_PROMPT_FORMAL),
    ("闲聊", SYSTEM_PROMPT_CHAT),
    ("邮件", SYSTEM_PROMPT_MAIL),
    ("Emoji化", SYSTEM_PROMPT_EMOJI),
];
const REQUEST_TIMEOUT_SECS: u64 = 45;
/// 预览区一次至少写入剪贴板的最大字符数；超过仅截断显示，不截断写入。
const PREVIEW_MAX_CHARS: usize = 220;
/// 每次向 UI 线程推流的最小新增字节数（约 13 个汉字）；最终包不受此限。
const CHUNK_MIN_BYTES: usize = 40;

// 96 DPI 基准
const BASE_WIDTH: i32 = 540;
const BASE_TEMPLATE_ROW: i32 = 24;
const BASE_PROMPT_ROW: i32 = 30;
const BASE_HINT_HEIGHT: i32 = 20;
const BASE_PREVIEW_MIN: i32 = 80;
const BASE_PADDING: i32 = 10;
const BASE_FONT: i32 = 16;
const BASE_SMALL_FONT: i32 = 12;

/// 面板调色板：全部来自皮肤候选窗段，禁止在绘制路径写死颜色。
/// 映射：prompt 输入行背景取候选高亮色（与底色拉出层级差），强调/错误都走
/// label（皮肤的强调色），弱化文字走 preedit 灰。
#[derive(Clone, Copy)]
struct Palette {
    bg: u32,
    prompt_bg: u32,
    prompt_hl: u32,
    text: u32,
    dim: u32,
    accent: u32,
    error: u32,
}

fn palette() -> (Palette, skin::Metrics, skin::Shadow) {
    let skin = Skin::current();
    let c = skin.candidate;
    (
        Palette {
            bg: c.background,
            prompt_bg: c.highlight_background,
            prompt_hl: c.label,
            text: c.text,
            dim: c.preedit,
            accent: c.label,
            error: c.label,
        },
        skin.metrics,
        skin.shadow,
    )
}

/// 面板模式：决定标题、选区交互与默认行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelMode {
    /// AI 帮写（Ctrl+Shift+W）：空面板等提示词
    Write,
    /// 划词润色（Ctrl+Shift+R）：选区原文进面板，回车覆盖选区
    Polish,
    /// 划词翻译（Ctrl+Shift+T）：选区原文进面板，AI 翻译，回车覆盖选区
    Translate,
}

struct PanelState {
    hwnd: HWND,
    /// 呼出时的前台窗口，回车粘贴目标
    target: HWND,
    dpi: u32,
    status: Status,
    /// 面板模式（Write / Polish / Translate）
    mode: PanelMode,
    /// 当前模板下标（TEMPLATES）；切换只影响下一次请求
    template: usize,
    /// M9-5：呼出时的前台应用 exe 全路径（Word/WPS 光标场景用于标题提示）
    context: Option<String>,
}

#[derive(Debug, Clone, Default)]
enum Status {
    /// 正在输入提示词；query 是要发送给 Agnes 的 user 消息原文。
    #[default]
    Editing,
    /// 已发出请求；done_started 用于"已等待 X 秒"；partial 累积 SSE delta。
    Pending { started: Instant, partial: String },
    /// 请求成功，等待回车粘贴。
    Preview { prompt: String, draft: String },
    /// 请求失败或不具备配置。
    Failed { reason: String },
    /// 本地配置缺失（环境变量没设）。
    Misconfigured,
}

struct EditingState {
    query: String,
}

thread_local! {
    static PANEL: RefCell<Option<PanelState>> = const { RefCell::new(None) };
    static EDITING: RefCell<EditingState> = const { RefCell::new(EditingState { query: String::new() }) };
    static SHADOW: RefCell<ShadowShell> = RefCell::new(ShadowShell::new());
    /// 面板窗口句柄：主题切换回调靠它找到并重绘面板。
    static PANEL_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
}

/// 最近一次生效的热键门控位图：bit0 = enable_polish_hotkey，bit1 = enable_ai_hotkey，
/// bit2 = enable_translate_hotkey。
/// u8::MAX 哨兵表示"尚未同步"，首次 refresh_hotkey_gates 必然重注册一次。
static LAST_HOTKEY_GATES: AtomicU8 = AtomicU8::new(u8::MAX);

/// 注册全局热键；首选 Ctrl+Shift+W（与"输入法"的 Writer 思路呼应），失败
/// 尝试 Alt+W。线程级注册，由 listener.rs 消息循环分发。
/// 划词润色注册 Ctrl+Shift+R；被占用时静默降级，AI 帮写主入口不受影响。
/// 面板不常驻，进程退出即失效，不做反注册。
///
/// 受设置中心「通用」页 enable_ai_hotkey / enable_polish_hotkey 开关门控
/// （shurufa_options::hotkey_gates()，默认均开启）：关掉的入口不注册，
/// 开关热更新由 refresh_hotkey_gates 按 2 秒轮询接管。
pub fn register_hotkey() -> &'static str {
    let (enable_polish, enable_ai, enable_translate) = shurufa_options::hotkey_gates();
    unsafe {
        let which = if !enable_ai {
            "（设置中已关闭 AI 帮写热键）"
        } else if RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_SHIFT, 0x57).is_ok() {
            "Ctrl+Shift+W"
        } else if RegisterHotKey(None, HOTKEY_ID, MOD_ALT, 0x57).is_ok() {
            "Alt+W"
        } else {
            "（AI 热键注册失败）"
        };
        crate::log_line(&format!("AI 帮写热键注册结果：{which}"));
        let polish = if !enable_polish {
            "（设置中已关闭划词润色热键）"
        } else if RegisterHotKey(None, POLISH_HOTKEY_ID, MOD_CONTROL | MOD_SHIFT, 0x52).is_ok() {
            "Ctrl+Shift+R"
        } else {
            "（划词润色热键被占用）"
        };
        crate::log_line(&format!("划词润色热键：{polish}"));
        let translate = if !enable_translate {
            "（设置中已关闭划词翻译热键）"
        } else if RegisterHotKey(None, TRANSLATE_HOTKEY_ID, MOD_CONTROL | MOD_SHIFT, 0x54).is_ok() {
            "Ctrl+Shift+T"
        } else {
            "（划词翻译热键被占用）"
        };
        crate::log_line(&format!("划词翻译热键：{translate}"));
        which
    }
}

/// 启动时把当前门控写入缓存，避免 2 秒后第一次轮询误判"变化"而重注册。
/// 必须在 register_hotkey 之后调用（listener 主线程）。
pub fn sync_hotkey_gate_cache() {
    let (enable_polish, enable_ai, enable_translate) = shurufa_options::hotkey_gates();
    let bits = (enable_polish as u8) | ((enable_ai as u8) << 1) | ((enable_translate as u8) << 2);
    LAST_HOTKEY_GATES.store(bits, Ordering::Relaxed);
}

/// 热键门控热更新：设置中心开关即改即存，listener 每 2 秒调用本函数，
/// 门控位图变化时反注册再按当前开关重注册。
///
/// 必须在消息循环所在线程调用：RegisterHotKey 把热键关联到调用线程的
/// 消息队列，WM_HOTKEY 只投递给注册线程；跨线程注册会让 listener 的
/// GetMessageW 循环永远收不到按键。
pub fn refresh_hotkey_gates() {
    let (enable_polish, enable_ai, enable_translate) = shurufa_options::hotkey_gates();
    let bits = (enable_polish as u8) | ((enable_ai as u8) << 1) | ((enable_translate as u8) << 2);
    let prev = LAST_HOTKEY_GATES.swap(bits, Ordering::Relaxed);
    if prev == bits {
        return; // 门控无变化：不打扰注册状态，也不刷日志
    }
    crate::log_line(&format!(
        "热键门控变化：AI={enable_ai}，划词润色={enable_polish}，划词翻译={enable_translate}，重注册"
    ));
    unsafe {
        let _ = UnregisterHotKey(None, HOTKEY_ID);
        let _ = UnregisterHotKey(None, POLISH_HOTKEY_ID);
        let _ = UnregisterHotKey(None, TRANSLATE_HOTKEY_ID);
    }
    let _ = register_hotkey();
}

/// M9-6：划词应用白名单判定——options 白名单为空 = 所有应用放行；
/// 非空时仅允许列表中的 exe 文件名（大小写不敏感，取进程 exe 的 file_name）。
fn selection_whitelist_allows(exe_path: Option<&str>) -> bool {
    let whitelist = shurufa_options::load().general.selection_app_whitelist;
    if whitelist.is_empty() {
        return true;
    }
    let Some(name) = exe_path
        .map(std::path::Path::new)
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
    else {
        return false;
    };
    whitelist.iter().any(|item| item.eq_ignore_ascii_case(name))
}

/// 划词润色入口（Ctrl+Shift+R）：抓选区 → 面板预填进 Editing；无有效选区时
/// 面板走 Failed 样式提示"未选中有效文本"。请求与粘贴都复用面板状态机。
pub fn polish_selection() {
    crate::log_line("划词润色：收到热键");
    let target = unsafe { GetForegroundWindow() };
    let exe = foreground_app_name(target);
    if !selection_whitelist_allows(exe.as_deref()) {
        crate::log_line(&format!(
            "划词润色：白名单未命中（{}），跳过",
            exe.unwrap_or_default()
        ));
        return;
    }
    let grabbed = grab_selected_text();
    show_selection_mode(target, grabbed, PanelMode::Polish);
}

/// 划词翻译入口（Ctrl+Shift+T）：抓选区 → 面板预填进 Editing，AI 翻译；
/// 无有效选区时面板走 Failed 样式提示。回车覆盖选区（微信/搜狗划词翻译同类）。
pub fn translate_selection() {
    crate::log_line("划词翻译：收到热键");
    let target = unsafe { GetForegroundWindow() };
    let exe = foreground_app_name(target);
    if !selection_whitelist_allows(exe.as_deref()) {
        crate::log_line(&format!(
            "划词翻译：白名单未命中（{}），跳过",
            exe.unwrap_or_default()
        ));
        return;
    }
    let grabbed = grab_selected_text();
    show_selection_mode(target, grabbed, PanelMode::Translate);
}

/// 面板态选区入口（润色/翻译共用）：selected 为 None/空/超长 →
/// Failed("未选中有效文本")，否则预填选区文本并直接落 Editing，
/// 标题按模式渲染（"划词润色" / "划词翻译"）。
fn show_selection_mode(target: HWND, selected: Option<String>, mode: PanelMode) {
    let Some(hwnd) = ensure_window() else {
        crate::log_line("AI 面板窗口创建失败");
        return;
    };
    let (_, _, shadow) = palette();
    let skin = Skin::current();
    skin::apply_appearance(hwnd, &skin);
    let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);
    let width = scale(BASE_WIDTH, dpi);
    let min_height = scale(
        BASE_PADDING * 2
            + BASE_TEMPLATE_ROW
            + BASE_PROMPT_ROW
            + BASE_HINT_HEIGHT
            + BASE_PREVIEW_MIN,
        dpi,
    );

    let too_long = selected
        .as_ref()
        .map(|s| s.chars().count() > 2_000)
        .unwrap_or(false);
    let valid = selected
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
        && !too_long;
    let label = match mode {
        PanelMode::Polish => "划词润色",
        PanelMode::Translate => "划词翻译",
        PanelMode::Write => "AI 帮写",
    };
    let status = if valid {
        crate::log_line(&format!(
            "{label}面板弹出，选区 {} 字符",
            selected.as_ref().map(|s| s.chars().count()).unwrap_or(0)
        ));
        Status::Editing
    } else {
        crate::log_line(&format!("{label}：未选中有效文本"));
        Status::Failed {
            reason: "未选中有效文本".into(),
        }
    };
    EDITING.with_borrow_mut(|e| {
        e.query = if valid {
            selected.unwrap_or_default()
        } else {
            String::new()
        };
    });
    PANEL.with_borrow_mut(|slot| {
        *slot = Some(PanelState {
            hwnd,
            target,
            dpi,
            status,
            mode,
            template: 0,
            context: None,
        });
    });

    let anchor = caret_or_cursor_pos(target);
    let (mut x, mut y) = (anchor.x, anchor.y + scale(6, dpi));
    unsafe {
        x = x.min(GetSystemMetrics(SM_CXSCREEN) - width - 8).max(0);
        y = y.min(GetSystemMetrics(SM_CYSCREEN) - min_height - 8).max(0);
        let _ = MoveWindow(hwnd, x, y, width, min_height, true);
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        SHADOW.with_borrow_mut(|shell| shell.sync(hwnd, x, y, width, min_height, &shadow));
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, true);
        // M9-4：划词面板同样跟随输入锚点
        let _ = SetTimer(Some(hwnd), AI_FOLLOW_TIMER_ID, 200, None);
    }
}

// 划词润色面板仍使用同一套 Agnes chat 通道；选区经由 user 消息原文进入，
// 系统提示仍走所选模板（默认"正式"）。这里不再有单独的 rewrite 提示。

/// 抓前台选中的纯文本：保存剪贴板 → Ctrl+C → 等 150ms 读回文本 → 立刻恢复
/// 原剪贴板文本（恢复原内容失败时历史库里还能找到，静默掉）。
fn grab_selected_text() -> Option<String> {
    // 保存现有剪贴板文本（图片/文件场景下不恢复，反正是临时顶替）
    let prev_text = read_clipboard_text();

    unsafe {
        send_ctrl_c();
    }
    std::thread::sleep(Duration::from_millis(150));
    let grabbed = read_clipboard_text();
    // 恢复：只恢复文本；与抓回内容相同时说明本来就没新复制，跳过
    if let Some(text) = prev_text {
        if !text.is_empty() && Some(&text) != grabbed.as_ref() {
            let _ = crate::paste::set_clipboard_text(&text);
        }
    }
    grabbed.filter(|s| !s.is_empty())
}

unsafe fn send_ctrl_c() {
    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                },
            },
        }
    }
    let inputs = [
        key(VK_SHIFT, true),
        key(VK_CONTROL, false),
        key(VIRTUAL_KEY(0x43), false),
        key(VIRTUAL_KEY(0x43), true),
        key(VK_CONTROL, true),
    ];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

fn read_clipboard_text() -> Option<String> {
    // 与 listener::read_open_clipboard 同款读取；此处独立实现避免循环依赖。
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    unsafe {
        OpenClipboard(None).ok()?;
        let result = (|| {
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
            let hglobal = HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                return None;
            }
            let size = GlobalSize(hglobal);
            let wide = std::slice::from_raw_parts(ptr as *const u16, size / 2);
            let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
            let text = String::from_utf16_lossy(&wide[..len]);
            let _ = GlobalUnlock(hglobal);
            Some(text)
        })();
        let _ = CloseClipboard();
        result
    }
}

/// 预热面板窗口（run 模式启动时调用）：后台服务一启动就创建隐藏窗口，
/// 设置中心「AI 帮写」入口可随时投递 WM_AI_EXTERNAL_SHOW 唤起。
pub fn warm_up() {
    if let Some(hwnd) = ensure_window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// 热键触发：记录前台窗口并弹出面板。
pub fn show() {
    let target = unsafe { GetForegroundWindow() };
    let Some(hwnd) = ensure_window() else {
        crate::log_line("AI 面板窗口创建失败");
        return;
    };
    let (_, _, shadow) = palette();
    let skin = Skin::current();
    skin::apply_appearance(hwnd, &skin);
    let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);
    let width = scale(BASE_WIDTH, dpi);
    let min_height = scale(
        BASE_PADDING * 2
            + BASE_TEMPLATE_ROW
            + BASE_PROMPT_ROW
            + BASE_HINT_HEIGHT
            + BASE_PREVIEW_MIN,
        dpi,
    );

    EDITING.with_borrow_mut(|e| e.query.clear());
    let status = if std::env::var_os("AGNES_API_KEY").is_some() {
        crate::log_line("AI 帮写面板弹出，等待输入提示");
        Status::Editing
    } else {
        crate::log_line("AI 帮写：缺少 AGNES_API_KEY 环境变量");
        Status::Misconfigured
    };
    // M9-5：识别 Word/WPS 光标场景（标题显示「AI 光标助手」）
    let context = foreground_app_name(target);
    if context.as_deref().is_some_and(is_office_cursor_app) {
        crate::log_line("AI 光标助手：检测到 Word/WPS，提交后草稿将粘贴到光标处");
    }
    PANEL.with_borrow_mut(|slot| {
        *slot = Some(PanelState {
            hwnd,
            target,
            dpi,
            status,
            mode: PanelMode::Write,
            template: 0,
            context,
        });
    });

    let anchor = caret_or_cursor_pos(target);
    let (mut x, mut y) = (anchor.x, anchor.y + scale(6, dpi));
    unsafe {
        x = x.min(GetSystemMetrics(SM_CXSCREEN) - width - 8).max(0);
        y = y.min(GetSystemMetrics(SM_CYSCREEN) - min_height - 8).max(0);
        let _ = MoveWindow(hwnd, x, y, width, min_height, true);
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        // 阴影壳：主窗下一层的半透明黑圆角壳
        SHADOW.with_borrow_mut(|shell| shell.sync(hwnd, x, y, width, min_height, &shadow));
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, true);
        // M9-4：面板可见期间 200ms 轮询输入锚点
        let _ = SetTimer(Some(hwnd), AI_FOLLOW_TIMER_ID, 200, None);
    }
}

/// M9-4：输入锚点跟随——保持面板宽高，仅按目标窗口当前光标/插入点重定位
/// （配合 AI_FOLLOW_TIMER_ID 定时器；目标窗口消失时回退光标位置）。
fn follow_anchor() {
    let Some((hwnd, target, dpi)) =
        PANEL.with_borrow(|slot| slot.as_ref().map(|s| (s.hwnd, s.target, s.dpi)))
    else {
        return;
    };
    let anchor = caret_or_cursor_pos(target);
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return;
    }
    let (mut x, mut y) = (anchor.x, anchor.y + scale(6, dpi));
    unsafe {
        x = x.min(GetSystemMetrics(SM_CXSCREEN) - width - 8).max(0);
        y = y.min(GetSystemMetrics(SM_CYSCREEN) - height - 8).max(0);
        let _ = MoveWindow(hwnd, x, y, width, height, true);
        let (_, _, shadow) = palette();
        SHADOW.with_borrow_mut(|shell| shell.sync(hwnd, x, y, width, height, &shadow));
    }
}

fn hide() {
    // 见 panel.rs：隐藏持焦点的窗口会同步派发 WM_KILLFOCUS → 重入 hide 双重借用。
    let hwnd = PANEL.with_borrow_mut(|slot| slot.take().map(|s| s.hwnd));
    SHADOW.with_borrow_mut(|shell| shell.hide());
    if let Some(hwnd) = hwnd {
        unsafe {
            // M9-4：停止锚点跟随定时器
            let _ = KillTimer(Some(hwnd), AI_FOLLOW_TIMER_ID);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// 提交当前 prompt：发到 Agnes（SSE 流式），UI 主线程立刻进入 Pending。
fn start_request(query: String) {
    let Some(api_key) = std::env::var_os("AGNES_API_KEY").and_then(|v| v.into_string().ok()) else {
        PANEL.with_borrow_mut(|slot| {
            if let Some(state) = slot.as_mut() {
                state.status = Status::Misconfigured;
                unsafe {
                    let _ = InvalidateRect(Some(state.hwnd), None, true);
                }
            }
        });
        return;
    };
    let Some((hwnd, template, mode)) =
        PANEL.with_borrow(|slot| slot.as_ref().map(|s| (s.hwnd, s.template, s.mode)))
    else {
        return;
    };
    // 划词翻译用固定翻译提示；其它模式走所选模板（默认"正式"）。
    let system_prompt = if mode == PanelMode::Translate {
        SYSTEM_PROMPT_TRANSLATE
    } else {
        TEMPLATES
            .get(template)
            .map(|t| t.1)
            .unwrap_or(SYSTEM_PROMPT)
    };
    PANEL.with_borrow_mut(|slot| {
        if let Some(state) = slot.as_mut() {
            state.status = Status::Pending {
                started: Instant::now(),
                partial: String::new(),
            };
            unsafe {
                let _ = InvalidateRect(Some(state.hwnd), None, true);
            }
        }
    });
    let api_key = api_key.trim().to_owned();
    crate::log_line(&format!(
        "AI 流式请求开始：模板={}，提示词 {} 字符",
        TEMPLATES.get(template).map(|t| t.0).unwrap_or("正式"),
        query.chars().count()
    ));
    // HWND 不是 Send；跨线程时转成 isize，到对端再包回 HWND。
    let hwnd_raw = hwnd.0 as isize;
    std::thread::spawn(move || {
        let hwnd = HWND(hwnd_raw as *mut _);
        // 流式增量：worker 端累全文，按 ~40 字节步长把 Box<(全文, is_final)>
        // 通过 WM_AI_CHUNK 发回 UI 线程；UI 线程不重拼，直接替换渲染。
        let mut last_pushed = 0usize;
        let on_chunk = |acc: &str, is_final: bool| {
            if !is_final && acc.len() - last_pushed < CHUNK_MIN_BYTES {
                return;
            }
            last_pushed = acc.len();
            let boxed: Box<(String, bool)> = Box::new((acc.to_owned(), is_final));
            unsafe {
                let _ = PostMessageW(
                    Some(hwnd),
                    WM_AI_CHUNK,
                    WPARAM(0),
                    LPARAM(Box::into_raw(boxed) as isize),
                );
            }
        };
        let result = call_agnes_stream(&api_key, &query, system_prompt, on_chunk);
        let boxed: Box<(String, Result<String, String>)> = Box::new((query, result));
        unsafe {
            let _ = PostMessageW(
                Some(hwnd),
                WM_AI_DONE,
                WPARAM(0),
                LPARAM(Box::into_raw(boxed) as isize),
            );
        }
    });
}

/// 一次性调用 Agnes（非流式）。划词润色用它：选区原文送进、回写时一次性
/// 覆盖；主面板帮写改用 `call_agnes_stream`，两端并行演化不影响划词链路。
pub(crate) fn call_agnes(
    api_key: &str,
    user_prompt: &str,
    system_prompt: &str,
) -> Result<String, String> {
    let body = build_chat_body(user_prompt, system_prompt, false);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build();
    let resp = agent
        .post("https://apihub.agnes-ai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_bytes(&body)
        .map_err(map_ureq_err)?;
    let text = resp
        .into_string()
        .map_err(|e| format!("读取响应失败: {e}"))?;
    extract_chat_content(&text)
}

/// 流式调用 Agnes（SSE）。已累积全文通过 `on_chunk(accumulated, is_final)`
/// 回吐给调用方（步长由调用方控制）；返回完整拼接后的草稿。
/// 整体 45s 上限：`start.elapsed()` 超时即中止；若已有部分内容则把内容带上，
/// 由 UI 决定降级成"（流中断，已截断）"预览而不是整块 Failed。
fn call_agnes_stream<F>(
    api_key: &str,
    user_prompt: &str,
    system_prompt: &str,
    mut on_chunk: F,
) -> Result<String, String>
where
    F: FnMut(&str, bool),
{
    use std::io::BufRead;
    let body = build_chat_body(user_prompt, system_prompt, true);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build();
    let start = Instant::now();
    let resp = agent
        .post("https://apihub.agnes-ai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream")
        .send_bytes(&body)
        .map_err(map_ureq_err)?;
    let mut reader = std::io::BufReader::new(resp.into_reader());
    let mut acc = String::new();
    let mut first_chunk_at: Option<Duration> = None;
    let mut finish = "done";
    let mut raw = Vec::<u8>::new();
    loop {
        if start.elapsed() > Duration::from_secs(REQUEST_TIMEOUT_SECS) {
            finish = "timeout";
            break;
        }
        raw.clear();
        match reader.read_until(b'\n', &mut raw) {
            Ok(0) => break, // EOF：服务端正常收尾
            Ok(_) => {}
            Err(e) => {
                finish = "io-error";
                crate::log_line(&format!("AI 流读取失败：{e}"));
                break;
            }
        }
        // 一次 read_until 不一定停在 UTF-8 边界；不完整字符留给下一行一起判。
        let line = String::from_utf8_lossy(&raw);
        match parse_sse_line(line.trim_end(), &mut acc) {
            SseEvent::Skip => {}
            SseEvent::Done => {
                finish = "done";
                break;
            }
            SseEvent::Delta => {
                if first_chunk_at.is_none() {
                    first_chunk_at = Some(start.elapsed());
                }
                on_chunk(&acc, false);
            }
        }
    }
    let elapsed = start.elapsed();
    crate::log_line(&format!(
        "AI 流式结束：原因={finish}，首包 {}ms，总耗时 {}ms，草稿 {} 字符",
        first_chunk_at.map(|d| d.as_millis() as u64).unwrap_or(0),
        elapsed.as_millis(),
        acc.chars().count()
    ));
    if acc.trim().is_empty() {
        return Err(match finish {
            "timeout" => "请求超时（45s 无响应）".into(),
            _ => "Agnes 流式返回为空".into(),
        });
    }
    if finish == "timeout" {
        acc.push_str("（流中断，已截断）");
    }
    on_chunk(&acc, true);
    Ok(acc)
}

fn build_chat_body(user_prompt: &str, system_prompt: &str, stream: bool) -> Vec<u8> {
    #[derive(serde::Serialize)]
    struct Req<'a> {
        model: &'a str,
        messages: Vec<Msg<'a>>,
        temperature: f32,
        stream: bool,
    }
    #[derive(serde::Serialize)]
    struct Msg<'a> {
        role: &'a str,
        content: &'a str,
    }
    let req = Req {
        model: "agnes-2.5-flash",
        messages: vec![
            Msg {
                role: "system",
                content: system_prompt,
            },
            Msg {
                role: "user",
                content: user_prompt,
            },
        ],
        temperature: 0.5,
        stream,
    };
    serde_json::to_vec(&req).expect("请求序列化不应失败")
}

fn map_ureq_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            format!("HTTP {code}: {}", crate::single_line_preview(&body, 120))
        }
        ureq::Error::Transport(t) => format!("网络错误: {t}"),
    }
}

fn extract_chat_content(text: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Resp {
        choices: Option<Vec<Choice>>,
        error: Option<ErrObj>,
    }
    #[derive(serde::Deserialize)]
    struct Choice {
        message: Option<ChoiceMsg>,
    }
    #[derive(serde::Deserialize)]
    struct ChoiceMsg {
        content: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct ErrObj {
        message: Option<String>,
    }
    let parsed: Resp = serde_json::from_str(text).map_err(|e| {
        format!(
            "解析响应失败: {e}; 片段: {}",
            crate::single_line_preview(text, 100)
        )
    })?;
    if let Some(err) = parsed.error {
        return Err(format!(
            "Agnes 错误: {}",
            err.message.unwrap_or_else(|| "未知".into())
        ));
    }
    parsed
        .choices
        .and_then(|mut c| c.pop())
        .and_then(|c| c.message)
        .and_then(|m| m.content)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Agnes 返回为空".to_owned())
}

/// SSE 行解析结果；纯函数，便于脱离网络做单元测试。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SseEvent {
    /// 空行 / 非 data 行 / [DONE] 之外的段落分隔：跳过
    Skip,
    /// 本行 JSON 携带 choices[0].delta.content，已追加进 acc
    Delta,
    /// data: [DONE]，流结束
    Done,
}

/// 解析一行 SSE。入参应是去掉换行后的整行文本；若上一次的 read_until
/// 落在多字节 UTF-8 中间，`String::from_utf8_lossy` 会补 U+FFFD，JSON
/// 解析会失败 → 归入 Skip 静默跳过，等下一行把字符读全再入 acc。
pub(crate) fn parse_sse_line(line: &str, acc: &mut String) -> SseEvent {
    let line = line.trim_end();
    if line.is_empty() {
        return SseEvent::Skip;
    }
    let Some(payload) = line.strip_prefix("data:") else {
        return SseEvent::Skip;
    };
    let payload = payload.trim_start();
    if payload == "[DONE]" {
        return SseEvent::Done;
    }
    // 每行是一个 JSON：choices[0].delta.content 是增量；解析失败静默跳过
    let Some(delta) = extract_stream_delta(payload) else {
        return SseEvent::Skip;
    };
    acc.push_str(&delta);
    SseEvent::Delta
}

/// 解析 SSE `data: {...}` 行，返回本行携带的增量文本（若有）。
fn extract_stream_delta(payload: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Chunk {
        choices: Option<Vec<ChunkChoice>>,
    }
    #[derive(serde::Deserialize)]
    struct ChunkChoice {
        delta: Option<ChunkDelta>,
    }
    #[derive(serde::Deserialize)]
    struct ChunkDelta {
        content: Option<String>,
    }
    let chunk: Chunk = serde_json::from_str(payload).ok()?;
    chunk
        .choices?
        .into_iter()
        .next()?
        .delta?
        .content
        .filter(|s| !s.is_empty())
}

/// 粘贴预览草稿到目标窗口：写剪贴板 → 回前台 → 模拟 Ctrl+V。
fn commit_draft() {
    let Some((draft, target)) = PANEL.with_borrow(|slot| {
        slot.as_ref().and_then(|s| match &s.status {
            Status::Preview { draft, .. } => Some((draft.clone(), s.target)),
            _ => None,
        })
    }) else {
        return;
    };
    hide();
    crate::log_line(&format!("AI 草稿粘贴（{} 字符）", draft.chars().count()));
    if crate::paste::set_clipboard_text(&draft).is_ok() && !target.is_invalid() {
        unsafe {
            let _ = SetForegroundWindow(target);
            std::thread::sleep(Duration::from_millis(80));
            send_ctrl_v();
        }
    }
    // 重置为新一轮输入状态
    EDITING.with_borrow_mut(|e| e.query.clear());
}

unsafe fn send_ctrl_v() {
    send_ctrl_v_impl()
}

/// 由 speech 模块调用（同一 crate 内部）。
pub(crate) unsafe fn send_ctrl_v_external() {
    send_ctrl_v_impl()
}

fn send_ctrl_v_impl() {
    unsafe { send_ctrl_v_impl_inner() }
}

unsafe fn send_ctrl_v_impl_inner() {
    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                },
            },
        }
    }
    // 先抬 Shift：呼出用的 Ctrl+Shift+W 可能尚未松开，避免"Ctrl+Shift+V"无格式粘贴。
    let inputs = [
        key(VK_SHIFT, true),
        key(VK_CONTROL, false),
        key(VIRTUAL_KEY(0x56), false),
        key(VIRTUAL_KEY(0x56), true),
        key(VK_CONTROL, true),
    ];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

/// M9-5：Word/WPS 光标助手判定——exe 文件名白名单。
fn is_office_cursor_app(exe_path: &str) -> bool {
    let name = std::path::Path::new(exe_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(
        name.as_str(),
        "WINWORD.EXE" | "WPS.EXE" | "WPSOFFICE.EXE" | "ET.EXE" | "WPP.EXE"
    )
}

/// 取前台窗口所属进程的 exe 全路径（用于光标场景识别）。
fn foreground_app_name(target: HWND) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        if target.is_invalid() {
            return None;
        }
        let pid = GetWindowThreadProcessId(target, None);
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut size = 1024u32;
        let mut buf = vec![0u16; size as usize];
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        buf.truncate(size as usize);
        Some(String::from_utf16_lossy(&buf))
    }
}

fn caret_or_cursor_pos(target: HWND) -> POINT {
    unsafe {
        if !target.is_invalid() {
            let thread = GetWindowThreadProcessId(target, None);
            let mut info = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if GetGUIThreadInfo(thread, &mut info).is_ok()
                && !info.hwndCaret.is_invalid()
                && info.rcCaret.bottom > info.rcCaret.top
            {
                let mut p = POINT {
                    x: info.rcCaret.left,
                    y: info.rcCaret.bottom,
                };
                let _ = ClientToScreen(info.hwndCaret, &mut p);
                return p;
            }
        }
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        p
    }
}

fn scale(base: i32, dpi: u32) -> i32 {
    (base * dpi as i32 + 48) / 96
}

/// 模板 chips 的几何：返回第 idx 个 chip 的矩形（客户区坐标）。
/// 与 paint() 中渲染的坐标保持一致；改布局时只需同步修改这一处。
fn template_chip_rect(idx: usize, dpi: u32) -> RECT {
    let padding = scale(BASE_PADDING, dpi);
    let row_h = scale(BASE_TEMPLATE_ROW, dpi);
    let gap = scale(6, dpi);
    let chip_w = scale(64, dpi);
    let x = padding + (chip_w + gap) * idx as i32;
    RECT {
        left: x,
        top: padding,
        right: x + chip_w,
        bottom: padding + row_h,
    }
}

/// 判断客户区坐标命中了哪个模板 chip。
fn hit_template_chip(pt: POINT) -> Option<usize> {
    let dpi = PANEL.with_borrow(|slot| slot.as_ref().map(|s| s.dpi))?;
    for (idx, _) in TEMPLATES.iter().enumerate() {
        let r = template_chip_rect(idx, dpi);
        if pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom {
            return Some(idx);
        }
    }
    None
}

fn ensure_window() -> Option<HWND> {
    if let Some(hwnd) = PANEL.with_borrow(|s| s.as_ref().map(|s| s.hwnd)) {
        return Some(hwnd);
    }
    thread_local! {
        static CACHED_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
        static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
    }
    if let Some(hwnd) = CACHED_HWND.with_borrow(|h| *h) {
        return Some(hwnd);
    }
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
        let class_name = w!("ShurufaAiPanel");
        CLASS_REGISTERED.with_borrow_mut(|registered| {
            if !*registered {
                let class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: class_name,
                    hbrBackground: HBRUSH::default(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    ..Default::default()
                };
                RegisterClassW(&class);
                *registered = true;
            }
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("AI 帮写"),
            WS_POPUP,
            0,
            0,
            BASE_WIDTH,
            BASE_PADDING * 2 + BASE_TEMPLATE_ROW + BASE_PROMPT_ROW + BASE_PREVIEW_MIN,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .ok()?;
        skin::apply_appearance(hwnd, &Skin::current());
        CACHED_HWND.with_borrow_mut(|h| *h = Some(hwnd));
        PANEL_HWND.with_borrow_mut(|h| *h = Some(hwnd));
        Some(hwnd)
    }
}

/// 主题变化后由主题监听窗口（panel.rs）调用：重设外观并重绘可见面板。
pub fn on_theme_changed() {
    let hwnd = PANEL_HWND.with_borrow(|h| *h);
    if let Some(hwnd) = hwnd {
        let skin = Skin::current();
        unsafe {
            skin::apply_appearance(hwnd, &skin);
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint(hdc, &ps.rcPaint);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            on_key(hwnd, VIRTUAL_KEY(wparam.0 as u16));
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == AI_FOLLOW_TIMER_ID {
                follow_anchor();
            } else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_CHAR => {
            on_char(hwnd, wparam.0 as u32);
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            hide();
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            // 模板 chips 命中检测：LPARAM 低位是 (x,y) 客户区坐标（有符号）。
            let pt = POINT {
                x: (lparam.0 & 0xffff) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            };
            if let Some(idx) = hit_template_chip(pt) {
                PANEL.with_borrow_mut(|slot| {
                    if let Some(state) = slot.as_mut() {
                        if state.template != idx {
                            state.template = idx;
                            crate::log_line(&format!("AI 模板切换：{}", TEMPLATES[idx].0));
                            unsafe {
                                let _ = InvalidateRect(Some(state.hwnd), None, true);
                            }
                        }
                    }
                });
            }
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            if skin::is_immersive_color_change(lparam) {
                let skin = Skin::refresh_on_setting_change();
                unsafe {
                    skin::apply_appearance(hwnd, &skin);
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
            }
            LRESULT(0)
        }
        x if x == WM_AI_EXTERNAL_SHOW => {
            show();
            LRESULT(0)
        }
        x if x == WM_AI_CHUNK => {
            // 流式增量：worker 把已累积全文打成 Box<(String, is_final)> 发回来，
            // UI 线程直接替换 Pending.partial；Repaint 只在收到 chunk 时触发。
            let ptr = lparam.0 as *mut (String, bool);
            if !ptr.is_null() {
                let (accumulated, _is_final) = unsafe { *Box::from_raw(ptr) };
                PANEL.with_borrow_mut(|slot| {
                    if let Some(state) = slot.as_mut() {
                        if let Status::Pending { partial, .. } = &mut state.status {
                            *partial = accumulated;
                            unsafe {
                                let _ = InvalidateRect(Some(state.hwnd), None, true);
                            }
                        }
                    }
                });
            }
            LRESULT(0)
        }
        x if x == WM_AI_DONE => {
            // 来自 worker：Box<(prompt, Result<draft, reason>)>
            let ptr = lparam.0 as *mut (String, Result<String, String>);
            if !ptr.is_null() {
                let boxed = unsafe { Box::from_raw(ptr) };
                let (prompt, result) = *boxed;
                PANEL.with_borrow_mut(|slot| {
                    if let Some(state) = slot.as_mut() {
                        state.status = match result {
                            Ok(draft) => Status::Preview { prompt, draft },
                            Err(reason) => Status::Failed { reason },
                        };
                        unsafe {
                            let _ = InvalidateRect(Some(state.hwnd), None, true);
                        }
                    }
                });
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn on_key(_hwnd: HWND, vk: VIRTUAL_KEY) {
    enum Do {
        Close,
        Send,
        PasteDraft,
        BackToEdit,
    }
    let action = PANEL.with_borrow(|slot| {
        let state = slot.as_ref()?;
        match (vk, &state.status) {
            (VK_ESCAPE, _) => Some(Do::Close),
            (VK_RETURN, Status::Editing) => {
                let q = EDITING.with_borrow(|e| e.query.trim().to_owned());
                if q.is_empty() {
                    None
                } else {
                    Some(Do::Send)
                }
            }
            (VK_RETURN, Status::Preview { .. }) => Some(Do::PasteDraft),
            (VK_BACK, Status::Editing) => {
                EDITING.with_borrow_mut(|e| {
                    e.query.pop();
                });
                unsafe {
                    let _ = InvalidateRect(Some(state.hwnd), None, true);
                }
                None
            }
            (VIRTUAL_KEY(0x52), Status::Failed { .. }) => Some(Do::BackToEdit), // 'R' 重新输入
            // 'E' 回到编辑态：Preview 里不满意可基于 prompt 改词再发
            (VIRTUAL_KEY(0x45), Status::Preview { prompt, .. }) => {
                let prompt = prompt.clone();
                EDITING.with_borrow_mut(|e| e.query = prompt);
                Some(Do::BackToEdit)
            }
            _ => None,
        }
    });
    match action {
        Some(Do::Close) => hide(),
        Some(Do::Send) => {
            let q = EDITING.with_borrow(|e| e.query.trim().to_owned());
            start_request(q);
        }
        Some(Do::PasteDraft) => commit_draft(),
        Some(Do::BackToEdit) => {
            PANEL.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut() {
                    state.status = Status::Editing;
                    unsafe {
                        let _ = InvalidateRect(Some(state.hwnd), None, true);
                    }
                }
            });
        }
        None => {}
    }
}

fn on_char(_hwnd: HWND, code: u32) {
    let Some(character) = char::from_u32(code) else {
        return;
    };
    let is_editing =
        PANEL.with_borrow(|slot| matches!(slot.as_ref().map(|s| &s.status), Some(Status::Editing)));
    if !is_editing {
        return;
    }
    if character.is_control() {
        return;
    }
    let changed = EDITING.with_borrow_mut(|e| {
        e.query.push(character);
        true
    });
    if changed {
        let hwnd = PANEL.with_borrow(|s| s.as_ref().map(|s| s.hwnd));
        if let Some(hwnd) = hwnd {
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
        }
    }
}

unsafe fn make_font(height: i32, weight: i32) -> HFONT {
    CreateFontW(
        -height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        w!("Microsoft YaHei UI"),
    )
}

fn draw_line(hdc: HDC, text: &str, x: i32, y: i32, w: i32, h: i32) {
    // 空串会让 DrawTextW 触发 AV；与 panel.rs 一致，直接跳过。
    if text.is_empty() {
        return;
    }
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    unsafe {
        DrawTextW(
            hdc,
            &mut utf16,
            &mut rect,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
}

fn draw_wrapped(hdc: HDC, text: &str, x: i32, y: i32, w: i32, h: i32) {
    if text.is_empty() {
        return;
    }
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    unsafe {
        DrawTextW(
            hdc,
            &mut utf16,
            &mut rect,
            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
}

unsafe fn paint(hdc: HDC, rc: &RECT) {
    PANEL.with_borrow(|slot| {
        let Some(state) = slot.as_ref() else {
            return;
        };
        let dpi = state.dpi;
        let padding = scale(BASE_PADDING, dpi);
        let template_h = scale(BASE_TEMPLATE_ROW, dpi);
        let prompt_h = scale(BASE_PROMPT_ROW, dpi);
        let hint_h = scale(BASE_HINT_HEIGHT, dpi);
        let preview_h = scale(BASE_PREVIEW_MIN, dpi);
        let width = scale(BASE_WIDTH, dpi);
        let (colors, metrics, _) = palette();
        let fs = metrics.font_scale;
        let sf = |base: i32| ((scale(base, dpi) as f32) * fs).round().max(8.0) as i32;

        let bg = CreateSolidBrush(COLORREF(colors.bg));
        FillRect(hdc, rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkMode(hdc, TRANSPARENT);

        let font = make_font(sf(BASE_FONT), FW_NORMAL.0 as i32);
        let bold = make_font(sf(BASE_FONT), FW_BOLD.0 as i32);
        let small = make_font(sf(BASE_SMALL_FONT), FW_NORMAL.0 as i32);

        // 顶部第 0 行：标题（由模式决定："AI 帮写" / "划词润色" / "划词翻译"）
        SelectObject(hdc, HGDIOBJ(bold.0));
        SetTextColor(hdc, COLORREF(colors.accent));
        let title = match state.mode {
            PanelMode::Write => state
                .context
                .as_deref()
                .filter(|p| is_office_cursor_app(p))
                .map(|_| "AI 光标助手 · Word/WPS")
                .unwrap_or("AI 帮写"),
            PanelMode::Polish => "划词润色",
            PanelMode::Translate => "划词翻译",
        };
        draw_line(
            hdc,
            title,
            padding,
            padding,
            width - padding * 2,
            scale(14, dpi),
        );

        // 顶部第 1 行：4 个模板 chips（active 用皮肤高亮色，inactive 走 dim）
        let chips_top = padding + scale(18, dpi);
        let chip_gap = scale(6, dpi);
        let chip_w = scale(64, dpi);
        let chip_h = template_h;
        let active_brush = CreateSolidBrush(COLORREF(colors.prompt_hl));
        let inactive_brush = CreateSolidBrush(COLORREF(colors.prompt_bg));
        SelectObject(hdc, HGDIOBJ(small.0));
        for (idx, (label, _)) in TEMPLATES.iter().enumerate() {
            let r = RECT {
                left: padding + (chip_w + chip_gap) * idx as i32,
                top: chips_top,
                right: padding + (chip_w + chip_gap) * idx as i32 + chip_w,
                bottom: chips_top + chip_h,
            };
            let is_active = idx == state.template;
            let brush = if is_active { active_brush } else { inactive_brush };
            FillRect(hdc, &r, brush);
            SetTextColor(
                hdc,
                COLORREF(if is_active { colors.bg } else { colors.dim }),
            );
            // 文字水平居中：DT_CENTER 只允许单行，这里 chip 是固定高度单行。
            let mut utf16: Vec<u16> = label.encode_utf16().collect();
            let mut r2 = r;
            DrawTextW(
                hdc,
                &mut utf16,
                &mut r2,
                DT_CENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
        }
        let _ = DeleteObject(HGDIOBJ(active_brush.0));
        let _ = DeleteObject(HGDIOBJ(inactive_brush.0));

        // 模板行底 + padding 后是 prompt 行
        let prompt_top = chips_top + chip_h + scale(4, dpi);
        let prompt_rect = RECT {
            left: padding,
            top: prompt_top,
            right: width - padding,
            bottom: prompt_top + prompt_h,
        };
        let prompt_brush = CreateSolidBrush(COLORREF(colors.prompt_bg));
        FillRect(hdc, &prompt_rect, prompt_brush);
        let _ = DeleteObject(HGDIOBJ(prompt_brush.0));
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        match &state.status {
            Status::Editing => {
                SetTextColor(hdc, COLORREF(colors.text));
                let query = EDITING.with_borrow(|e| e.query.clone());
                let display = if query.is_empty() {
                    SetTextColor(hdc, COLORREF(colors.dim));
                    "输入想让 AI 写的内容（如：会议纪要开头 200 字）…".to_owned()
                } else {
                    format!("❯ {query}")
                };
                draw_line(
                    hdc,
                    &display,
                    padding + scale(6, dpi),
                    prompt_top + scale(4, dpi),
                    width - padding * 2 - scale(12, dpi),
                    prompt_h - scale(8, dpi),
                );
            }
            Status::Pending { started, .. } => {
                SetTextColor(hdc, COLORREF(colors.accent));
                let elapsed = started.elapsed().as_secs();
                let query = EDITING.with_borrow(|e| e.query.clone());
                draw_line(
                    hdc,
                    &format!("思考中…（{elapsed}s） {query}"),
                    padding + scale(6, dpi),
                    prompt_top + scale(4, dpi),
                    width - padding * 2 - scale(12, dpi),
                    prompt_h - scale(8, dpi),
                );
            }
            Status::Preview { prompt, .. } => {
                SetTextColor(hdc, COLORREF(colors.dim));
                draw_line(
                    hdc,
                    &format!("❯ {prompt}"),
                    padding + scale(6, dpi),
                    prompt_top + scale(4, dpi),
                    width - padding * 2 - scale(12, dpi),
                    prompt_h - scale(8, dpi),
                );
            }
            Status::Failed { .. } | Status::Misconfigured => {
                SetTextColor(hdc, COLORREF(colors.dim));
                let query = EDITING.with_borrow(|e| e.query.clone());
                draw_line(
                    hdc,
                    &if query.is_empty() {
                        "AI 帮写".to_owned()
                    } else {
                        format!("❯ {query}")
                    },
                    padding + scale(6, dpi),
                    prompt_top + scale(4, dpi),
                    width - padding * 2 - scale(12, dpi),
                    prompt_h - scale(8, dpi),
                );
            }
        }

        // 中部：状态区（预览 / 错误 / 提示）
        let mid_top = prompt_rect.bottom + padding;
        let mid_bottom = mid_top + preview_h;
        match &state.status {
            Status::Editing => {
                SelectObject(hdc, HGDIOBJ(small.0));
                SetTextColor(hdc, COLORREF(colors.dim));
                draw_line(
                    hdc,
                    "回车提交 · Esc 关闭 · 草稿成功后再回车即粘贴",
                    padding,
                    mid_top,
                    width - padding * 2,
                    hint_h,
                );
            }
            Status::Pending { partial, .. } => {
                if partial.is_empty() {
                    SelectObject(hdc, HGDIOBJ(small.0));
                    SetTextColor(hdc, COLORREF(colors.dim));
                    draw_line(
                        hdc,
                        "正在向 Agnes 请求草稿，请稍候；Esc 取消",
                        padding,
                        mid_top,
                        width - padding * 2,
                        hint_h,
                    );
                } else {
                    // SSE 流式片段先行渲染，等最终 WM_AI_DONE 落定 Preview。
                    SelectObject(hdc, HGDIOBJ(bold.0));
                    SetTextColor(hdc, COLORREF(colors.prompt_hl));
                    draw_line(hdc, "草稿（生成中…）", padding, mid_top, width - padding * 2, hint_h);
                    SelectObject(hdc, HGDIOBJ(font.0));
                    SetTextColor(hdc, COLORREF(colors.text));
                    let preview = crate::single_line_preview(partial, PREVIEW_MAX_CHARS);
                    draw_wrapped(
                        hdc,
                        &preview,
                        padding,
                        mid_top + hint_h,
                        width - padding * 2,
                        mid_bottom - mid_top - hint_h,
                    );
                }
            }
            Status::Preview { draft, .. } => {
                SelectObject(hdc, HGDIOBJ(bold.0));
                SetTextColor(hdc, COLORREF(colors.prompt_hl));
                draw_line(hdc, "草稿", padding, mid_top, width - padding * 2, hint_h);
                SelectObject(hdc, HGDIOBJ(font.0));
                SetTextColor(hdc, COLORREF(colors.text));
                let preview = crate::single_line_preview(draft, PREVIEW_MAX_CHARS);
                draw_wrapped(
                    hdc,
                    &preview,
                    padding,
                    mid_top + hint_h,
                    width - padding * 2,
                    mid_bottom - mid_top - hint_h,
                );
            }
            Status::Failed { reason } => {
                SelectObject(hdc, HGDIOBJ(bold.0));
                SetTextColor(hdc, COLORREF(colors.error));
                draw_line(
                    hdc,
                    "请求失败",
                    padding,
                    mid_top,
                    width - padding * 2,
                    hint_h,
                );
                SelectObject(hdc, HGDIOBJ(small.0));
                draw_wrapped(
                    hdc,
                    reason,
                    padding,
                    mid_top + hint_h,
                    width - padding * 2,
                    mid_bottom - mid_top - hint_h,
                );
            }
            Status::Misconfigured => {
                SelectObject(hdc, HGDIOBJ(bold.0));
                SetTextColor(hdc, COLORREF(colors.error));
                draw_line(
                    hdc,
                    "未配置 AGNES_API_KEY",
                    padding,
                    mid_top,
                    width - padding * 2,
                    hint_h,
                );
                SelectObject(hdc, HGDIOBJ(small.0));
                SetTextColor(hdc, COLORREF(colors.dim));
                draw_wrapped(
                    hdc,
                    "在“系统属性 → 环境变量”中添加用户级 AGNES_API_KEY，重启本进程后生效；key 不会被写入任何日志或配置文件。",
                    padding,
                    mid_top + hint_h,
                    width - padding * 2,
                    mid_bottom - mid_top - hint_h,
                );
            }
        }

        // 底部：快捷键提示
        let footer_top = mid_bottom + padding;
        SelectObject(hdc, HGDIOBJ(small.0));
        SetTextColor(hdc, COLORREF(colors.dim));
        let footer = match &state.status {
            Status::Editing => "回车 提交 · Esc 关闭",
            Status::Pending { .. } => "Esc 取消",
            Status::Preview { .. } => "回车 粘贴 · E 改提示重试 · Esc 关闭",
            Status::Failed { .. } => "R 重新输入 · Esc 关闭",
            Status::Misconfigured => "Esc 关闭",
        };
        draw_line(
            hdc,
            footer,
            padding,
            footer_top,
            width - padding * 2,
            hint_h,
        );

        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteObject(HGDIOBJ(bold.0));
        let _ = DeleteObject(HGDIOBJ(small.0));
    });
}

#[cfg(test)]
mod tests {
    //! AI 面板纯逻辑层（make_font/paint 不进入测试）。
    use super::{parse_sse_line, SseEvent};

    #[test]
    fn 环境变量名符合约定() {
        // 用户文档与代码统一使用同一个名字；这个名字写进 Misconfigured 提示。
        let declared = "AGNES_API_KEY";
        assert_eq!(declared.len(), 13);
    }

    #[test]
    fn sse_delta_累积文本() {
        let mut acc = String::new();
        let ev = parse_sse_line(
            r#"data: {"choices":[{"delta":{"content":"你好"}}]}"#,
            &mut acc,
        );
        assert_eq!(ev, SseEvent::Delta);
        assert_eq!(acc, "你好");
        let ev = parse_sse_line(
            r#"data: {"choices":[{"delta":{"content":"，世界"}}]}"#,
            &mut acc,
        );
        assert_eq!(ev, SseEvent::Delta);
        assert_eq!(acc, "你好，世界");
    }

    #[test]
    fn sse_done_行收尾() {
        let mut acc = "已有".to_owned();
        let ev = parse_sse_line("data: [DONE]", &mut acc);
        assert_eq!(ev, SseEvent::Done);
        assert_eq!(acc, "已有");
    }

    #[test]
    fn sse_空行与非data行_静默跳过() {
        let mut acc = String::new();
        assert_eq!(parse_sse_line("", &mut acc), SseEvent::Skip);
        assert_eq!(parse_sse_line(": comment", &mut acc), SseEvent::Skip);
        assert_eq!(parse_sse_line("event: message", &mut acc), SseEvent::Skip);
        assert!(acc.is_empty());
    }

    #[test]
    fn sse_坏json_静默跳过() {
        let mut acc = String::new();
        // 不合法 JSON：不能让整个流挂掉，跳过这段
        assert_eq!(parse_sse_line("data: {not-json", &mut acc), SseEvent::Skip);
        // 合法 JSON 但 choices 为空
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[]}"#, &mut acc),
            SseEvent::Skip
        );
        assert!(acc.is_empty());
    }

    #[test]
    fn sse_缺delta_静默跳过() {
        let mut acc = String::new();
        // role-only 心跳帧（无 content）：跳过
        assert_eq!(
            parse_sse_line(
                r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#,
                &mut acc
            ),
            SseEvent::Skip
        );
        assert!(acc.is_empty());
    }

    #[test]
    fn sse_utf8_跨行分割时经由lossy降级到lossy字符() {
        // 真实读循环走 read_until(b'\n') + from_utf8_lossy：半字节行被补 �，
        // parse_sse_line 应把它视为 Skip（JSON 不合法）而不是 panic。
        let mut acc = String::new();
        // “你”字的完整三字节是 e4 bd a0；这里手动只取前两字节模拟截断
        let prefix = br#"data: {"choices":[{"delta":{"content":""#;
        let mut raw: Vec<u8> = prefix.to_vec();
        raw.extend_from_slice(&[0xe4, 0xbd]); // 截掉最后一字节
        let line = String::from_utf8_lossy(&raw).into_owned();
        let ev = parse_sse_line(line.trim_end(), &mut acc);
        // 含 U+FFFD 的替换字符 → JSON parse 失败 → Skip，且 acc 未被污染
        assert_eq!(ev, SseEvent::Skip);
        assert!(acc.is_empty());
    }
}
