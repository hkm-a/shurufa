//! 候选窗：GDI 绘制的置顶弹窗，横向排列编号候选（主流输入法布局）。
//!
//! 窗口在宿主应用的 UI 线程内创建（TSF 单元线程模型），
//! 不抢焦点（WS_EX_NOACTIVATE），随组合文本位置移动，
//! 宽度按候选文本实测宽度自适应，所有尺寸按窗口 DPI 缩放。
//!
//! 本轮改动摘要（皮肤 v2 / 现代化外观）：
//! - 颜色不再硬编码，全部来自 `crate::skin::Skin`（按系统 light/dark 主题选取）。
//! - 字号乘 `metrics.font_scale`；`metrics.opacity` < 1 时启用分层窗口整体透明。
//! - 创建后由 `skin::apply_appearance` 应用 Win11 DWM 圆角 + 沉浸式深色边框；
//!   `skin::ShadowShell` 在主窗下方绘制半透明阴影壳（NOACTIVATE + 命中穿透）。
//! - WM_SETTINGCHANGE("ImmersiveColorSet") 热切换主题：刷新 Skin 缓存并重绘。

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GlobalFree, COLORREF, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW,
    EndPaint, FillRect, GetDC, GetTextExtentPoint32W, InvalidateRect, LineTo, MoveToEx, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, ValidateRect, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ, HPEN, PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, TrackMouseEvent, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, TME_LEAVE,
    TRACKMOUSEEVENT, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetSystemMetrics, LoadCursorW, MoveWindow, RegisterClassW, SetWindowPos, ShowWindow,
    TrackPopupMenu, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, IDC_ARROW, MF_SEPARATOR, MF_STRING,
    SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_DESTROY, WM_DPICHANGED, WM_GETOBJECT, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_SETTINGCHANGE, WM_SIZE, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use ime_ipc::Context;

use crate::skin::{self, ShadowShell, Skin};

// 候选条右键菜单的引擎动作钩子（service.rs 在会话初始化时注册；闭包
// 持有与活动组合相同的 TSF 会话客户端，simulate 才能作用于当前候选页——
// 新连接会建新会话，空组合下删词/降频/隐藏全部空转）。与 wnd_proc 同线程。
type EngineSimulateFn = Box<dyn Fn(&str) -> bool>;
thread_local! {
    static ENGINE_SIMULATE: RefCell<Option<EngineSimulateFn>> =
        const { RefCell::new(None) };
}

/// 注册引擎动作钩子（候选条菜单 删词/降频/隐藏 用；见 service.rs 会话初始化）。
pub fn set_engine_simulate(f: EngineSimulateFn) {
    ENGINE_SIMULATE.with(|slot| *slot.borrow_mut() = Some(f));
}

/// 把一段 Rime 键序送到当前会话（如 "3" 选中第 3 候选、"Control+d" 删词）。
fn engine_simulate(keys: &str) -> bool {
    ENGINE_SIMULATE.with(|slot| slot.borrow().as_ref().map(|f| f(keys)).unwrap_or(false))
}

// ---------------------------------------------------------------------------
// AI 候选预测（2026-08-20；见 docs/AI候选预测方案.md 与 ai_candidates.rs）
// ---------------------------------------------------------------------------

/// worker 线程把 AI 候选结果送回候选窗 UI 线程的私有消息。
/// LPARAM 携带 Box<Vec<(preedit, text)>> 的裸指针（同进程，wnd_proc 消费后释放）。
pub(crate) const WM_AI_CANDIDATES_READY: u32 = WM_APP + 81;
/// 点击 AI 候选时让 TSF 走编辑会话提交的触发键。用 **Enter（VK_RETURN）**：
/// 实测 chrome 只把文本相关键（字母/修饰/Enter 等）路由给 TSF——F9、
/// VK_APPS 等非文本键收不到 OnKeyDown。Enter 是文本键必达；handle_key 仅在
/// pending_ai 非空时消费（正常回车不受影响）。
const VK_AI_COMMIT_TRIGGER: u8 = 0x0D; // VK_RETURN
/// AI 候选起始索引（None = 当前帧无 AI 候选）。
/// 提交钩子返回 true = 已完成提交（组合同步替换 + 结束）；false = 需要
/// 候选窗回发触发键走 handle_key 兜底（仅直接提交失败时）。
type AiCommitFn = Box<dyn Fn(&str) -> bool>;
thread_local! {
    /// show() 时的原始 Context 快照（AI 结果到达后据此重建布局）。
    static LAST_CTX: RefCell<Option<Context>> = const { RefCell::new(None) };
    /// AI 候选：(preedit, text)；show 时只合并 preedit 匹配的条目。
    static AI_CANDIDATES: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    /// 当前帧 AI 候选起始索引（点击 AI 候选走提交回调，而非引擎数字键）。
    static AI_START: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// 候选服务 Tab（M7-5）：拼音（默认）/ 英文。仅在英文候选非空时显示
    /// Tab 行；点击切换候选展示组。切换后内容指纹变化 → 自动重算布局。
    static ACTIVE_TAB: std::cell::Cell<TabKind> = const { std::cell::Cell::new(TabKind::Rime) };
    /// AI 候选提交钩子（service 注册：写 pending 槽；候选窗负责发 VK_AI_COMMIT_TRIGGER）。
    static AI_COMMIT: RefCell<Option<AiCommitFn>> = const { RefCell::new(None) };
    /// 最近一次 show 的候选面板模式（refresh_with_ai 重建布局时复用）。
    static LAST_PANEL_MODE: std::cell::Cell<CandidatePanelMode> =
        const { std::cell::Cell::new(CandidatePanelMode::Single) };
}

/// 注册 AI 候选提交钩子（service.rs 会话初始化；wnd_proc 同线程调用）。
pub fn set_ai_commit(f: AiCommitFn) {
    AI_COMMIT.with(|slot| *slot.borrow_mut() = Some(f));
}

/// 读最近一次 show 的 Context 快照（service.rs 数字键/空格拦截时用）。
pub fn last_ctx_clone() -> Option<Context> {
    LAST_CTX.with(|c| c.borrow().clone())
}

/// 候选服务 Tab 切换（M7-5）：设置激活组并按 LAST_CTX 快照重建布局 +
/// 重绘（同 refresh_with_ai 模式，无需按键即可看到新候选组）。
pub(crate) fn tab_switch(tab: TabKind) {
    ACTIVE_TAB.with(|t| t.set(tab));
    let hwnd = PAINT_HWND.with(|h| h.get());
    if hwnd.0.is_null() {
        return;
    }
    let Some(ctx) = LAST_CTX.with(|c| c.borrow().clone()) else {
        return;
    };
    let panel_mode = LAST_PANEL_MODE.with(|m| m.get());
    let (merged, ai_start) = merge_ai_candidates(&ctx);
    AI_START.with(|s| s.set(ai_start));
    let mut view_ctx = ctx;
    view_ctx.candidates = merged;
    let extra = crate::dll_path()
        .parent()
        .map(|dir| dir.join("schemas").join("shurufa-skin.json"));
    let skin = skin::load_with(|| extra);
    let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);
    let screen_w = logical_screen_dim(unsafe { GetSystemMetrics(SM_CXSCREEN) }, dpi);
    let screen_h = logical_screen_dim(unsafe { GetSystemMetrics(SM_CYSCREEN) }, dpi);
    let (width, height) = compute_show_layout(
        hwnd,
        &view_ctx,
        &skin,
        dpi,
        panel_mode,
        screen_w,
        screen_h,
        ai_start,
    );
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
        let _ = MoveWindow(hwnd, rect.left, rect.top, width, height, false);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

/// 当前候选窗句柄（worker 线程结果投递用；窗口未创建时为 None）。
pub(crate) fn current_hwnd() -> Option<HWND> {
    PAINT_HWND.with(|h| {
        let v = h.get();
        if v.0.is_null() {
            None
        } else {
            Some(v)
        }
    })
}

// ---------------------------------------------------------------------------
// 渲染后端瀑布链：DComp → D2D → GDI
// ---------------------------------------------------------------------------

/// 候选窗渲染后端枚举。选路由 `backend_kind()` 懒解析：
/// 1. probe DComp（D3D11 + IDXGIFactory2 + DCompositionCreateDevice 全通）→ DComp
/// 2. D2D try_init 成功 → D2D
/// 3. 否则 → Gdi（纯软件绘制，恒可用，绝无 panic）
///
/// 与模块内 `Backend::{Pending,Ready,Failed}`
/// 状态机互补：那个管"是否处于 ready"，这个管"走哪条路径"。
/// 每帧 WM_PAINT 先取 kind；当帧渲染失败（返回 false）时当场降级到下一级重画，
/// **不在状态机里永久打 Failed 标记**——TDR/驱动重置由 D2D/DComp 各自的
/// notify_resize + 惰性重建吸收，不至于把整会话钉死在低档位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// DComp flip-model swapchain + premultiplied 圆角纹理（wave 4 新增）
    DComp,
    /// 既有 D2D 1.1 + DirectWrite（wave 1 落地）
    D2D,
    /// 兜底 GDI（legacy；布局/测量零漂移）
    Gdi,
}

thread_local! {
    /// 每个宿主 UI 线程解析一次；不在 wnd_proc 里反复 D3D11CreateDevice。
    static BACKEND_KIND: std::cell::Cell<Option<BackendKind>> = const { std::cell::Cell::new(None) };
}

/// 选路（懒解析 + thread-local 缓存）。首次调用即按 `probe_dcomp_available` → `candidate_window_d2d::is_enabled` 定档。
pub(crate) fn backend_kind() -> BackendKind {
    BACKEND_KIND.with(|c| {
        if let Some(k) = c.get() {
            return k;
        }
        // SHURUFA_FORCE_GDI=1 / SHURUFA_FORCE_D2D=1：排障用，强制指定
        // 渲染后端（跳过探测）。
        let k = if std::env::var_os("SHURUFA_FORCE_GDI").is_some() {
            BackendKind::Gdi
        } else if std::env::var_os("SHURUFA_FORCE_D2D").is_some() {
            BackendKind::D2D
        } else if crate::candidate_window_dcomp::probe_dcomp_available() {
            BackendKind::DComp
        } else if crate::candidate_window_d2d::is_enabled() {
            BackendKind::D2D
        } else {
            BackendKind::Gdi
        };
        c.set(Some(k));
        k
    })
}

/// 供测试/调试覆盖选路结果（正常生产路径不调）。
#[cfg(test)]
fn set_backend_kind_for_test(k: BackendKind) {
    BACKEND_KIND.with(|c| c.set(Some(k)));
}

const CLASS_NAME: PCWSTR = w!("ShurufaCandidateWindow");

// 96 DPI 下的基准尺寸，运行期按窗口实际 DPI 缩放
const BASE_ROW_HEIGHT: i32 = 40;
const BASE_PREEDIT_HEIGHT: i32 = 26;
const BASE_PADDING: i32 = 12;
const BASE_ITEM_GAP: i32 = 22;
const BASE_LABEL_GAP: i32 = 6;
const BASE_HL_PAD: i32 = 7;
const BASE_FONT_HEIGHT: i32 = 26;
const BASE_PREEDIT_FONT_HEIGHT: i32 = 18;
const BASE_MIN_WIDTH: i32 = 96;
/// 多行候选面板每行候选数（搜狗 16.3b 多行候选同类；9 候选 → 5+4 两行）。
const MULTI_COLUMNS: usize = 5;
/// 候选服务 Tab 行高度（96 DPI 基准；M7-5）。
const BASE_TAB_HEIGHT: i32 = 22;
const BASE_MODE_BADGE_GAP: i32 = 10;

/// 候选服务 Tab（M7-5 候选 Tab 多服务切换）：默认拼音组（引擎候选 +
/// AI 混排）；英文组（内置词表前缀联想，english_candidates::suggest）。
/// 仅当非 Rime 组有候选时显示 Tab 行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabKind {
    Rime,
    English,
}

/// 候选来源分类（启发式，搜狗/百度"来源标识"同类；librime API 不暴露候选 type，
/// 前端按文本特征判断。仅用于 show_candidate_badge 角标，默认关闭）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateSource {
    /// 全 ASCII 字母/数字/连字符（英文词、网址、ID 等）
    English,
    /// 含非 BMP 字符或常见 emoji 区（😊、🅰️ 等）
    Emoji,
    /// 日期/时间/数字格式（2026-08-17、14:30、壹佰贰拾叁元整、1+1=2）
    Special,
    /// 单个 CJK 字符
    SingleChar,
    /// 多 CJK 字符（词/短语）
    Word,
}

/// 纯函数：按文本特征分类候选来源。空文本按 Word 兜底。
/// 分类优先级：emoji > 英文 > 特殊格式 > 单字 > 词。
pub(crate) fn classify_candidate_source(text: &str) -> CandidateSource {
    if text.is_empty() {
        return CandidateSource::Word;
    }
    // emoji/非 BMP：现代 emoji 大多在 BMP 之外的增补平面（U+1F300+），
    // 也有 BMP 内的（☺ U+263A、♠ U+2660 等），一并按"含非 CJK 符号"处理。
    let has_non_bmp = text.chars().any(|c| c as u32 > 0xFFFF);
    if has_non_bmp {
        return CandidateSource::Emoji;
    }
    let is_ascii = text.is_ascii();
    if is_ascii {
        // 纯 ASCII：日期/时间/算式/金额是 ASCII 但应归 Special。
        // 判断：含数字且含分隔符（- / : . 空格）→ 特殊格式；否则英文。
        let has_digit = text.chars().any(|c| c.is_ascii_digit());
        let has_sep = text
            .chars()
            .any(|c| matches!(c, '-' | '/' | ':' | '.' | ' ' | '=' | '+' | '×' | '÷'));
        if has_digit && has_sep {
            return CandidateSource::Special;
        }
        // 金额大写（壹佰贰拾叁元整）是 CJK，不走这里
        return CandidateSource::English;
    }
    // 非 ASCII：CJK 单字 vs 词；含非 CJK 符号（☺ 等 BMP 符号）归 Emoji/特殊
    let all_cjk = text.chars().all(|c| {
        let cp = c as u32;
        (0x4E00..=0x9FFF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp)
            || cp == 0x3007 // 〇（U+3007 部首数字零，二〇二六年 用）
            || cp == 0x3002 // 。
            || cp == 0x3001 // ，
            || (0xFF01..=0xFF65).contains(&cp) // 全角标点
    });
    if !all_cjk {
        // 含 CJK 之外的字符（全角标点、BMP 符号等）——日期中文格式（二〇二六年）
        // 或含符号，按 Special 处理
        if text.chars().all(|c| {
            let cp = c as u32;
            (0x4E00..=0x9FFF).contains(&cp)
                || (0x3400..=0x4DBF).contains(&cp)
                || c.is_ascii_digit()
                || matches!(
                    c,
                    '年' | '月'
                        | '日'
                        | '时'
                        | '分'
                        | '秒'
                        | '元'
                        | '整'
                        | '·'
                        | '：'
                        | ':'
                        | '-'
                        | '/'
                        | '.'
                )
        }) {
            return CandidateSource::Special;
        }
        return CandidateSource::Emoji;
    }
    if text.chars().count() == 1 {
        CandidateSource::SingleChar
    } else {
        // 中文日期（含 ≥2 个 年月日 标记）或金额（元+整 同现）归 Special，
        // 避免把 单元/日元 这类普通词误判（单个标记不触发）。
        let date_markers = text
            .chars()
            .filter(|c| matches!(c, '年' | '月' | '日'))
            .count();
        let is_amount = text.contains('元') && text.contains('整');
        let is_time = text
            .chars()
            .filter(|c| matches!(c, '时' | '分' | '秒'))
            .count()
            >= 2;
        if date_markers >= 2 || is_amount || is_time {
            CandidateSource::Special
        } else {
            CandidateSource::Word
        }
    }
}

/// 纯函数：把超长候选文本截断为"前缀 + …"（weasel candidate_abbreviate_length 同款）。
/// `limit` 是字符数上限（0 = 不截断）；只在 text 的字符数严格超过 limit 时截断。
/// 返回 (截断后的显示文本, 是否被截断)。
pub(crate) fn abbreviate_text(text: &str, limit: i32) -> (String, bool) {
    if limit <= 0 {
        return (text.to_owned(), false);
    }
    let count = text.chars().count() as i32;
    if count <= limit {
        return (text.to_owned(), false);
    }
    // 留 1 个字符位给 "…"（U+2026，宽 1 字），截到 limit-1 个字符
    let keep = (limit - 1).max(1) as usize;
    let truncated: String = text.chars().take(keep).collect();
    (format!("{truncated}…"), true)
}

/// 候选来源角标的显示文本（用于 show_candidate_badge 角标渲染）。
pub(crate) fn candidate_source_label(source: CandidateSource) -> &'static str {
    match source {
        CandidateSource::English => "EN",
        CandidateSource::Emoji => "EMOJI",
        CandidateSource::Special => "◈",
        CandidateSource::SingleChar => "字",
        CandidateSource::Word => "词",
    }
}

/// 单个候选的横向布局槽位（坐标为窗口客户区像素）
struct Item {
    label: String,
    /// 显示文本（超长候选已被截断为 前缀+…；上屏/选中仍用引擎完整文本）
    text: String,
    /// 词库附注（如同类推荐、近义词、emoji 提示）；为空则不绘制。
    comment: String,
    x: i32,
    /// 多行候选面板所在行（0 起；单行模式恒为 0）。
    row: i32,
    label_w: i32,
    text_w: i32,
    highlighted: bool,
    /// 鼠标悬停中（不含已被选中的项：选中优先，见 make_paint_view）。
    hovered: bool,
    /// 主文本不含 comment 的实测宽度（GDI show() 时记录）；D2D comment 起点复用。
    pure_text_w: i32,
    /// 候选来源角标文本（show_candidate_badge 开启且分类有意义时）；None 不渲染。
    source_badge: Option<&'static str>,
    /// 角标实测宽度（含间隙；布局槽位的一部分，D2D comment 起点据此偏移）。
    badge_w: i32,
}

struct PaintData {
    preedit: String,
    items: Vec<Item>,
    dpi: u32,
    skin: Skin,
    is_ascii: bool,
    is_full_shape: bool,
    /// 长按 Shift 触发的大写视觉角标（不实际切换引擎 ascii_mode）。
    /// 由 service.rs 经 set_caps_visual 写入；true 时 mode_badge 固定为 "⇪大写"。
    caps_visual: bool,
    page: PageInfo,
    /// Tab 行（M7-5）：是否显示 + 当前激活组 + Tab 行高度（0 = 不显示）。
    show_tab_bar: bool,
    tab_active: TabKind,
    tab_h: i32,
}

/// 传给 D2D 后端的一帧绘制视图（纯纯值，不持 Ref 避免跨 thread_local 借）。
/// 所有尺寸都是窗口客户区像素，已按 (dpi, font_scale) 展开。
pub struct PaintView {
    pub preedit: String,
    pub items: Vec<ItemView>,
    pub dpi: u32,
    pub skin: Skin,
    pub padding: i32,
    pub preedit_h: i32,
    pub row_h: i32,
    pub label_gap: i32,
    pub hl_pad: i32,
    /// 右上角模式徽标（None = 当前模式不需要角标）。
    /// 取值："中" / "En" / "⇪大写"；由 mode_badge_text() 统一推导。
    pub mode_badge: Option<&'static str>,
    /// 候选主字体 em-height（px）
    pub cand_font_h: i32,
    /// preedit/副标 小字体 em-height（px）
    pub sub_font_h: i32,
    /// 当前页分页快照（滚动条用）
    pub page: PageInfo,
    /// Tab 行（M7-5）：是否显示 + 激活组 + 行高（0 = 不显示）。
    pub show_tab_bar: bool,
    pub tab_active: TabKind,
    pub tab_h: i32,
    /// 预编辑音节分隔符列位（UTF-16 码元索引；空 = 无断点、按原文整串绘制）。
    /// 三条渲染路径共用同一份断点数据，一次扫描全帧消费。
    pub syllable_breaks: Vec<u16>,
}

pub struct ItemView {
    pub label: String,
    pub text: String,
    pub comment: String,
    pub x: i32,
    /// 多行候选面板所在行（0 起；单行模式恒为 0）。
    pub row: i32,
    pub label_w: i32,
    pub text_w: i32,
    pub pure_text_w: i32,
    pub highlighted: bool,
    /// 鼠标悬停中（选中项恒为 true 由 make_paint_view 归并，见 Item.hovered）。
    pub hovered: bool,
    /// 候选来源角标（None 不渲染）。
    pub source_badge: Option<&'static str>,
    /// 角标实测宽度（含间隙）。
    pub badge_w: i32,
}

/// 预编辑串内的音节分隔符位置（UTF-16 码元索引）。
///
/// 三种渲染路径共用。分隔符分两类：
/// - **空格**（` `）：Rime 引擎自动插入的音节分隔（`nihao` → `"ni hao"`），
///   是用户可读的分隔，**保留原样绘制**（不画竖线），只做轻微色差分段。
/// - **撇号**（`'`）：用户敲入的音界符（`xi'an`），是输入的一部分，
///   同样保留原样绘制。
///
/// 本函数只负责**找出分隔位置**；绘制层决定如何呈现（空格/撇号本体仍画，
/// 竖线仅作为可选的视觉增强，见 `draw_preedit_segmented`）。
pub fn syllable_breaks(preedit: &str) -> Vec<u16> {
    let mut out = Vec::new();
    for (i, u) in preedit.encode_utf16().enumerate() {
        if u == b' ' as u16 || u == b'\'' as u16 {
            out.push(i as u16);
        }
    }
    out
}

/// 分页滚动条快照（来自引擎 Context；page 双字段由 ime-ipc 透传）。
#[derive(Clone, Copy, Debug, Default)]
pub struct PageInfo {
    pub page_no: usize,
    pub is_last_page: bool,
}

impl PageInfo {
    /// 总页数估计：引擎不直接提供 total；由 page_no + is_last_page 推导
    /// （末页时 total = page_no+1，非末页时 total = page_no+2 下界），
    /// 滚动条 thumb 只表达相对位置，无需精确总页数。
    pub fn total_pages(&self) -> usize {
        self.page_no + if self.is_last_page { 1 } else { 2 }
    }
}

/// 多行候选面板占用的行数（由布局写入 Item.row；单行恒为 1）。
pub fn panel_row_count(items: &[ItemView]) -> i32 {
    items.iter().map(|it| it.row).max().unwrap_or(0) + 1
}

/// 本帧是否需要画滚动条（开皮肤开关 + 非单页；空候选也画——翻页中会出现）。
pub fn scrollbar_active(v: &PaintView) -> bool {
    v.skin.metrics.scrollbar && v.page.total_pages() > 1
}

/// 滚动条轨道像素宽（已按 dpi 缩放；关闭开关/单页时为 0，布局完全不占位）。
pub fn scrollbar_width(v: &PaintView) -> i32 {
    if scrollbar_active(v) {
        scale(skin::SCROLLBAR_BASE_WIDTH, v.dpi)
    } else {
        0
    }
}

/// 生成一帧 D2D 消费的不可变快照；None 表示数据未初始化（窗口还没 show 过）。
pub fn make_paint_view() -> Option<PaintView> {
    PAINT_DATA.with_borrow(|data| {
        let dpi = data.dpi;
        let font_scale = data.skin.metrics.font_scale;
        // 悬停与选中合并：选中项永远全高亮（鼠标悬停在选中项上不降级）；
        // 悬停命中与 HOVER_INDEX 相同（WM_MOUSEMOVE 每次重建后重新命中）。
        let hover = HOVER_INDEX.with(|h| h.get());
        Some(PaintView {
            preedit: data.preedit.clone(),
            items: data
                .items
                .iter()
                .enumerate()
                .map(|(i, it)| ItemView {
                    label: it.label.clone(),
                    text: it.text.clone(),
                    comment: it.comment.clone(),
                    x: it.x,
                    row: it.row,
                    label_w: it.label_w,
                    text_w: it.text_w,
                    pure_text_w: it.pure_text_w,
                    highlighted: it.highlighted,
                    hovered: it.highlighted || hover == Some(i),
                    source_badge: it.source_badge,
                    badge_w: it.badge_w,
                })
                .collect(),
            dpi,
            skin: data.skin,
            padding: scale(data.skin.metrics.padding_or(BASE_PADDING), dpi),
            preedit_h: scale(data.skin.metrics.preedit_h_or(BASE_PREEDIT_HEIGHT), dpi),
            row_h: scale(data.skin.metrics.row_h_or(BASE_ROW_HEIGHT), dpi),
            label_gap: scale(data.skin.metrics.label_gap_or(BASE_LABEL_GAP), dpi),
            hl_pad: scale(data.skin.metrics.hl_pad_or(BASE_HL_PAD), dpi),
            mode_badge: mode_badge_text(data),
            cand_font_h: font_height(BASE_FONT_HEIGHT, dpi, font_scale),
            sub_font_h: font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale),
            show_tab_bar: data.show_tab_bar,
            tab_active: data.tab_active,
            tab_h: data.tab_h,
            page: data.page,
            syllable_breaks: syllable_breaks(&data.preedit),
        })
    })
}

/// 徽标宽度（与 GDI paint 内 text_width + BASE_MODE_BADGE_GAP 同公式；
/// GDI 实际测宽结果 ∈ [3*cand_h, 3.5*cand_h]，用近似值差 1px 视觉无感）。
pub fn mode_badge_width(view: &PaintView) -> i32 {
    view.cand_font_h * 3 + scale(BASE_MODE_BADGE_GAP, view.dpi)
}

/// 把 PaintData 快照归一为右上角徽标文案；None = 不该显示角标。
/// 单独成函数让 GDI / D2D / DComp 三条渲染路径共用同一套规则书：
/// - caps_visual 优先级最高（长按 Shift 触发），只显示 "⇪大写"。
/// - 否则按引擎 ascii_mode 输出 "En" / "中"。
///
/// `is_full_shape` 不参与角标：候选行已经反映全/半角字符形态，角标保持最简。
fn mode_badge_text(data: &PaintData) -> Option<&'static str> {
    if data.caps_visual {
        Some("⇪大写")
    } else if data.is_ascii {
        Some("En")
    } else {
        Some("中")
    }
}

/// 长按 Shift 的"大写视觉"状态写入口（service.rs 调用；同 UI 线程）。
///
/// true：角标固定为 "⇪大写"；不清 preedit/candidates，不触碰 IPC。
/// false：角标回退到按 ascii_mode 推导的"中"/"En"。
/// 幂等；返回是否发生变化，供调用方决定是否 InvalidateRect 重绘。
pub fn set_caps_visual(active: bool) -> bool {
    PAINT_DATA.with_borrow_mut(|data| {
        if data.caps_visual == active {
            false
        } else {
            data.caps_visual = active;
            true
        }
    })
}

/// 当前是否处于"大写视觉"提示状态；仅供 service.rs / 单元测试读取真值。
#[allow(dead_code)]
pub fn caps_visual_active() -> bool {
    PAINT_DATA.with_borrow(|data| data.caps_visual)
}

/// 当前 PAINT_DATA 中 preedit 的 syllable_breaks 快照；service.rs 的 Tab
/// 音节导航用它判断是否需要把 Tab 重映射为 Left/Right。None = 还未 show 过。
/// 目前 Tab 导航改由引擎实时 context 驱动，本函数保留为调试/回归查询口。
#[allow(dead_code)]
pub fn current_preedit_breaks() -> Vec<u16> {
    PAINT_DATA.with_borrow(|data| syllable_breaks(&data.preedit))
}

// 绘制数据挂在线程本地：窗口与 TSF 回调同属宿主 UI 线程
thread_local! {
    static PAINT_DATA: RefCell<PaintData> = RefCell::new(PaintData {
        preedit: String::new(),
        items: Vec::new(),
        dpi: 96,
        skin: Skin::default(),
        is_ascii: false,
        is_full_shape: false,
        caps_visual: false,
        page: PageInfo::default(),
        show_tab_bar: false,
        tab_active: TabKind::Rime,
        tab_h: 0,
    });
    static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
    /// 当前鼠标悬停的候选序号（None = 不在任何候选上）。
    /// 每次 show() 重建 items 后由 WM_MOUSEMOVE 重新命中；
    /// 与 PAINT_DATA 同线程（宿主 UI 线程），Cell 足够。
    static HOVER_INDEX: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// 是否已向系统注册 WM_MOUSELEAVE 跟踪（TrackMouseEvent 幂等控制）。
    static HOVER_TRACKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 当前候选窗 HWND（hover 处理与 paint 共用；ensure_window 创建后写入）。
    static PAINT_HWND: std::cell::Cell<HWND> = const { std::cell::Cell::new(HWND(std::ptr::null_mut())) };
}

pub(crate) fn scale(base: i32, dpi: u32) -> i32 {
    (base * dpi as i32 + 48) / 96
}

/// 把屏幕的**物理像素**尺寸换算为窗口 DPI 下的**逻辑像素**。
///
/// 候选窗的布局、文本测量（GetTextExtentPoint32W）与 MoveWindow 全部使用
/// 窗口 DPI 的逻辑像素（scale() 单位）；而 GetSystemMetrics(SM_CXSCREEN /
/// SM_CYSCREEN) 在 DPI-aware 进程里返回的是**物理像素**。直接拿物理值参与
/// "屏幕宽度 60%" 之类的钳制，在缩放 >100% 的屏幕上会把上限放大 dpi/96 倍：
/// 150% 缩放下 60% 屏宽上限实际变成 90%（实测 2560px 物理屏 w=1104 逻辑
/// px = 1656 物理 px = 65% 屏宽，候选窗横贯大半个屏幕，2026-08-16 实机复现）。
/// 参与任何"与屏幕尺寸比较/占比"的运算前必须经此换算。
pub(crate) fn logical_screen_dim(physical: i32, dpi: u32) -> i32 {
    (physical * 96 / dpi as i32).max(1)
}

/// metrics.icon 预留槽：当前不渲染任何图标，每次会话仅提示一次（两渲染路径共用）。
pub fn log_icon_once(skin: &Skin) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    if skin.metrics.icon.is_none() {
        return;
    }
    ONCE.call_once(|| {
        crate::debug_log("skin: metrics.icon slot reserved, not yet rendered");
    });
}

/// DPI 缩放后再乘皮肤字号倍率；下限 8px 防止畸形配置把字压没。
/// D2D 后端按同一公式取 em-height → IDWriteTextFormat 字号。
pub(crate) fn font_height(base: i32, dpi: u32, font_scale: f32) -> i32 {
    ((scale(base, dpi) as f32) * font_scale).round().max(8.0) as i32
}

// GDI 字体缓存（2026-08-16 性能优化）：CreateFontW 是相对昂贵的系统调用
// （字体枚举/加载），旧实现 show()/paint 每键创建 2-3 个字体再删除——候选窗
// 只用两个字号（候选 26px、副标/预编辑 18px），缓存按 (height) 复用即可。
// 线程本地：候选窗 UI 线程固定，无并发。替换字号时删除旧字体释放资源。
// 字体只增不减（每个字号最多一个 HFONT），窗口销毁时随进程回收。
thread_local! {
    static FONT_CACHE: std::cell::RefCell<Vec<(i32, HFONT)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

unsafe fn make_font(height: i32) -> HFONT {
    FONT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((_, f)) = cache.iter().find(|(h, _)| *h == height) {
            return *f;
        }
        let font = CreateFontW(
            -height, // 负值表示字符高度（em），排版更稳定
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            w!("Microsoft YaHei UI"),
        );
        cache.push((height, font));
        font
    })
}

unsafe fn text_width(hdc: HDC, text: &str) -> i32 {
    let wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() {
        return 0;
    }
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
    size.cx
}

/// 候选窗位置策略（P1 #10，Fcitx5/微软拼音同类）：跟随光标（默认）或
/// 固定屏幕角落。固定模式忽略锚点，每次弹窗都出现在同一位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionMode {
    /// 跟随输入光标（默认，现行行为）
    Follow,
    /// 固定主屏右下角
    FixedBottomRight,
    /// 固定主屏左下角
    FixedBottomLeft,
}

impl PositionMode {
    /// 从 options 的 `candidate_position` 字段解析；未知值回退跟随。
    pub fn from_option(value: &str) -> Self {
        match value {
            "bottom_right" => PositionMode::FixedBottomRight,
            "bottom_left" => PositionMode::FixedBottomLeft,
            _ => PositionMode::Follow,
        }
    }
}

/// 候选面板模式（M7，搜狗 16.3b 候选条/多行候选同类）：单行候选条（默认）
/// 或多行候选面板（↓ 键唤出）。由选项 `candidate_panel_mode` 驱动，TSF 每键
/// 热读；模式参与内容指纹，切换即失效布局缓存。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidatePanelMode {
    /// 单行候选条（现行布局）
    Single,
    /// 多行候选面板（M7 目标布局）
    Multi,
}

impl CandidatePanelMode {
    /// 从 options 的 `candidate_panel_mode` 字段解析；未知值回退单行。
    pub fn from_option(value: &str) -> Self {
        match value {
            "multi" => CandidatePanelMode::Multi,
            _ => CandidatePanelMode::Single,
        }
    }
}

pub struct CandidateUi {
    hwnd: Option<HWND>,
    shadow: ShadowShell,
    /// UI 节流状态（2026-08-16，weasel#1869 同类）：记录上次窗口几何与
    /// 可见性，未变化时不重复 ShowWindow/SetWindowPos——长跑宿主下这些
    /// 窗口调用会随运行时间变慢（实测 show 24→2.6ms），节流后每键省掉
    /// 重复窗口操作。
    last_rect: Option<(i32, i32, i32, i32)>,
    visible: bool,
    /// 内容指纹短路（P0 #5，weasel#1869 进一步）：候选内容指纹（preedit/
    /// 候选/高亮/页码/模式/皮肤参数/DPI）未变时跳过字体实测与重绘——
    /// 组合内容未变的按键（如按住修饰键、重复按键、锚点移动但内容相同）
    /// 整帧零成本。
    last_fp: Option<u64>,
    /// 内容未变时复用的上次布局结果（宽度/高度只依赖指纹内的输入）。
    last_width: i32,
    last_height: i32,
}


/// 候选窗布局计算（show 与 AI 结果刷新共用）：字体实测 → items/宽高 →
/// 写入 PAINT_DATA。返回 (width, height)。内容指纹短路由调用方负责（AI
/// 刷新每次强制重算，触发频率低，成本可忽略）。
fn compute_show_layout(
    hwnd: HWND,
    ctx: &Context,
    skin: &Skin,
    dpi: u32,
    panel_mode: CandidatePanelMode,
    screen_w: i32,
    _screen_h: i32,
    ai_start: Option<usize>,
) -> (i32, i32) {
            // M7-5 候选服务 Tab：英文候选（前缀联想）非空时显示 Tab 行；
            // 激活英文组时候选列表替换为英文候选（独立编号，不走引擎）。
            let english = crate::english_candidates::suggest(&ctx.preedit);
            // Rime 无候选但英文候选有值（如输入串是英文词前缀、引擎无拼音命中）
            // → 自动激活英文组；否则跟随用户手动选择的 Tab。
            let tab_active = if ctx.candidates.is_empty() && !english.is_empty() {
                TabKind::English
            } else {
                ACTIVE_TAB.with(|t| t.get())
            };
            let show_tab_bar = !english.is_empty();
            let tab_h = if show_tab_bar {
                scale(BASE_TAB_HEIGHT, dpi)
            } else {
                0
            };
            let view_items: Vec<ime_ipc::Candidate> =
                if tab_active == TabKind::English && show_tab_bar {
                    english
                        .iter()
                        .map(|w| ime_ipc::Candidate {
                            text: w.clone(),
                            comment: String::new(),
                        })
                        .collect()
                } else {
                    ctx.candidates.clone()
                };
            let ai_start = if tab_active == TabKind::English && show_tab_bar {
                None
            } else {
                ai_start
            };
            let font_scale = skin.metrics.font_scale;
            let padding = scale(skin.metrics.padding_or(BASE_PADDING), dpi);
            let item_gap = scale(skin.metrics.item_gap_or(BASE_ITEM_GAP), dpi);
            let label_gap = scale(skin.metrics.label_gap_or(BASE_LABEL_GAP), dpi);
            // 滚动条：皮肤开启且非单页时右缘预留轨道宽；单页/关闭时 0，宽度零漂移
            let page = PageInfo {
                page_no: ctx.page_no,
                is_last_page: ctx.is_last_page,
            };
            let sb_w = if skin.metrics.scrollbar && page.total_pages() > 1 {
                scale(skin::SCROLLBAR_BASE_WIDTH, dpi)
            } else {
                0
            };
            // 候选窗最大宽度 = 屏幕宽度 60%（主流输入法习惯，高 DPI/多候选下
            // 防止单行 9 候选横贯屏幕"大到看不清"；超出部分靠翻页访问）。
            // 至少不小于最小宽度，保证极端窄屏下仍可读。
            // 注意：SM_CXSCREEN 返回物理像素，必须先换算成逻辑像素再取 60%，
            // 否则高 DPI 下钳制上限被放大（详见 logical_screen_dim 注释）。
            let max_width = (screen_w * 6 / 10).max(scale(BASE_MIN_WIDTH, dpi));

            // 用与绘制一致的字体实测文本宽度，横向布槽
            // max_row_used 在块外用于多行宽度计算（最宽行内容宽）。
            let mut max_row_used = 0i32;
            let (items, preedit_w) = unsafe {
                let hdc = GetDC(Some(hwnd));
                let cand_font = make_font(font_height(BASE_FONT_HEIGHT, dpi, font_scale));
                let preedit_font =
                    make_font(font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale));

                let old = SelectObject(hdc, HGDIOBJ(cand_font.0));
                let sub_font = make_font(font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale));
                // 宽度预算：候选行可用的最大右缘（扣除右侧 padding 与滚动条轨道）。
                let row_budget = (max_width - padding - sb_w).max(scale(BASE_MIN_WIDTH, dpi));
                let mut items: Vec<Item> = Vec::with_capacity(9);
                // 多行候选面板（M7，搜狗 16.3b 同类）：每行 MULTI_COLUMNS 个，
                // 行内放不下或满列则换行；单行模式沿用"放不下即停、剩余翻页"。
                let mut x = padding;
                let mut row = 0i32;
                let mut in_row = 0usize;
                let mut row_used = 0i32;
                for (i, c) in view_items.iter().enumerate().take(9) {
                    // AI 候选（2026-08-20）：合并列表里 ai_start 起为 AI 候选，
                    // 单行模式下强制从第二行起排——Rime 候选放不下 break 时
                    // 不波及 AI（AI 不参与引擎分页，必须可见可点）。
                    let is_ai = ai_start.is_some_and(|s| i >= s);
                    // TSF 端候选域固定 9 列；label 为 1..=9（超过 9 的索引不进此分支）
                    let label = format!("{}.", i + 1);
                    let label_w = text_width(hdc, &label);
                    // 长候选缩写（weasel candidate_abbreviate_length 同款）：显示文本
                    // 超长时截断为 前缀+…，只影响显示（引擎按索引提交，上屏仍完整）。
                    let (display_text, _abbreviated) =
                        abbreviate_text(&c.text, skin.metrics.abbreviate_length);
                    let text_w = text_width(hdc, &display_text);
                    // 候选来源角标（show_candidate_badge）：按文本特征分类，
                    // 角标占一个额外槽位（用 sub_font 实测宽度 + 间隙）。
                    let source_badge = if skin.metrics.show_candidate_badge {
                        Some(candidate_source_label(classify_candidate_source(&c.text)))
                    } else {
                        None
                    };
                    let badge_w = if let Some(b) = source_badge {
                        SelectObject(hdc, HGDIOBJ(sub_font.0));
                        let w = text_width(hdc, b);
                        SelectObject(hdc, HGDIOBJ(cand_font.0));
                        w + scale(6, dpi)
                    } else {
                        0
                    };
                    // 副标（词库附注）；只在文本不重复时展示——同字符的
                    // comment 是噪音。这里只截断长度，留待 paint 用小号字体。
                    let comment = if c.comment.is_empty() || c.comment == c.text {
                        String::new()
                    } else {
                        c.comment.chars().take(12).collect()
                    };
                    // 宽度预算给 comment：副标跟在主文本右侧，必须占住后续槽位。
                    // 与 paint 头一致：用 sub_font 实测，再加一侧间隙。
                    let comment_w = if comment.is_empty() {
                        0
                    } else {
                        SelectObject(hdc, HGDIOBJ(sub_font.0));
                        let w = text_width(hdc, &comment);
                        SelectObject(hdc, HGDIOBJ(cand_font.0));
                        w + scale(4, dpi)
                    };
                    let slot_w = label_w + label_gap + text_w + badge_w + comment_w + item_gap;
                    if panel_mode == CandidatePanelMode::Multi {
                        // 多行：换行条件 = 行内已有候选 且（放不下 或 已满列）。
                        if in_row > 0 && (x + slot_w > row_budget || in_row >= MULTI_COLUMNS) {
                            x = padding;
                            row += 1;
                            row_used = 0;
                            in_row = 0;
                        }
                    } else if is_ai {
                        // 单行 + AI 候选：不与 Rime 混排，恒从第二行起排；
                        // 第二行放不下继续换行（AI 至多 3 个，不参与分页）。
                        if row == 0 {
                            x = padding;
                            row = 1;
                            row_used = 0;
                            in_row = 0;
                        } else if x + slot_w > row_budget && in_row > 0 {
                            x = padding;
                            row += 1;
                            row_used = 0;
                            in_row = 0;
                        }
                    } else if x + slot_w > row_budget {
                        // 单行：Rime 候选放不下 → 跳过（剩余靠翻页访问）。
                        // 不能 break：AI 候选在第二行仍需处理——break 会让
                        // 排在合并列表尾部的 AI 永远无法到达（"ni hao ni hao"
                        // 实测复现：第一行 5 个 Rime 就 break，AI 不显示）。
                        continue;
                    }
                    items.push(Item {
                        label,
                        text: display_text,
                        comment,
                        x,
                        row,
                        label_w,
                        text_w: text_w + comment_w,
                        highlighted: i == ctx.highlighted,
                        hovered: false,
                        pure_text_w: text_w,
                        source_badge,
                        badge_w,
                    });
                    x += slot_w;
                    row_used += slot_w;
                    max_row_used = max_row_used.max(row_used);
                    in_row += 1;
                }

                SelectObject(hdc, HGDIOBJ(preedit_font.0));
                let preedit_w = text_width(hdc, &ctx.preedit);

                SelectObject(hdc, old);
                ReleaseDC(Some(hwnd), hdc);
                (items, preedit_w)
            };

            // 行尾 = 最宽行内容宽度 + 左 padding。单行模式默认 1 行沿用末项
            // 右缘；AI 候选（2026-08-20）换行到第二行后，末项可能是较短的
            // AI 候选，必须取 max_row_used（循环内每行 row_used 的全局最大）。
            let items_end = if panel_mode == CandidatePanelMode::Multi {
                padding + max_row_used
            } else if items.iter().any(|it| it.row > 0) {
                padding + max_row_used
            } else {
                items
                    .last()
                    .map(|it| it.x + it.label_w + label_gap + it.text_w + it.badge_w)
                    .unwrap_or(padding)
            };
            // 给模式徽标预留宽度，避免与 preedit 互相压占
            let mode_badge_hint =
                scale(BASE_FONT_HEIGHT, dpi) * 3 + scale(BASE_MODE_BADGE_GAP, dpi);
            // 总宽 = max(候选行尾, preedit+徽标) + 右 padding + 滚动条，钳到 max_width
            let width = ((items_end.max(padding + preedit_w + mode_badge_hint) + padding + sb_w)
                .max(scale(BASE_MIN_WIDTH, dpi)))
            .min(max_width);
            let row_h = scale(skin.metrics.row_h_or(BASE_ROW_HEIGHT), dpi);
            // 行数 = 最大行号 + 1（无候选保底 1 行）。单行模式默认 1 行；
            // AI 候选（2026-08-20）换行到第二行时 rows 自然为 2。
            let rows = items.iter().map(|it| it.row).max().unwrap_or(0) + 1;
            let height = tab_h
                + scale(skin.metrics.preedit_h_or(BASE_PREEDIT_HEIGHT), dpi)
                + row_h * rows
                + padding * 2;

            crate::debug_log(&format!(
                "cand show: mode={:?} rows={} win_dpi={} sys_dpi={} used_dpi={} screen_w={} max_w={} w={} h={} preedit={:?} cands={}",
                panel_mode, rows,
                unsafe { GetDpiForWindow(hwnd) },
                unsafe { GetDpiForSystem() },
                dpi,
                screen_w,
                max_width,
                width,
                height,
                ctx.preedit,
                ctx.candidates.len(),
            ));

            PAINT_DATA.with_borrow_mut(|data| {
                data.preedit = ctx.preedit.clone();
                data.items = items;
                data.dpi = dpi;
                data.skin = *skin;
                data.is_ascii = ctx.is_ascii;
                data.is_full_shape = ctx.is_full_shape;
                data.page = page;
                data.show_tab_bar = show_tab_bar;
                data.tab_active = tab_active;
                data.tab_h = tab_h;
            });

    (width, height)
}

/// 合并 AI 候选（2026-08-20）：引擎候选保留前 [crate::ai_candidates::RIME_KEEP]
/// 个，AI 候选（preedit 匹配、至多 MAX_CANDIDATES 个）追加其后，副标 "🤖"
/// 标注。返回 (合并后的候选列表, AI 起始索引；None = 本帧无 AI 候选)。
fn merge_ai_candidates(ctx: &Context) -> (Vec<ime_ipc::Candidate>, Option<usize>) {
    let ai = AI_CANDIDATES.with(|c| {
        c.borrow()
            .iter()
            .filter(|(p, _)| *p == ctx.preedit)
            .map(|(_, t)| t.clone())
            .collect::<Vec<_>>()
    });
    if ai.is_empty() {
        return (ctx.candidates.clone(), None);
    }
    let mut out: Vec<ime_ipc::Candidate> = ctx
        .candidates
        .iter()
        .take(crate::ai_candidates::RIME_KEEP)
        .cloned()
        .collect();
    let start = out.len();
    for t in ai.into_iter().take(crate::ai_candidates::MAX_CANDIDATES) {
        out.push(ime_ipc::Candidate {
            text: t,
            comment: "🤖".to_owned(),
        });
    }
    let ai_start = if start < out.len() {
        Some(start)
    } else {
        None
    };
    (out, ai_start)
}

/// AI 候选结果到达（WM_AI_CANDIDATES_READY，wnd_proc 同线程）：写入候选表、
/// 按上次 show 的快照重建布局并重绘。位置保持当前，仅按新尺寸重排。
pub(crate) fn refresh_with_ai(payload_ptr: isize) {
    let payload = unsafe { Box::from_raw(payload_ptr as *mut Vec<(String, String)>) };
    let hwnd = PAINT_HWND.with(|h| h.get());
    if hwnd.0.is_null() {
        return;
    }
    // 窗口已销毁（或从未 show 过）：释放 payload 防泄漏
    if LAST_CTX.with(|c| c.borrow().is_none()) {
        return;
    }
    AI_CANDIDATES.with(|c| {
        c.borrow_mut().extend(payload.iter().cloned());
        // 上限保护：保留最近 30 条（每 preedit 至多 3 条，10s TTL 自然淘汰）
        let len = c.borrow().len();
        if len > 30 {
            c.borrow_mut().drain(0..len - 30);
        }
    });
    let ctx = LAST_CTX.with(|c| c.borrow().clone()).unwrap();
    let panel_mode = LAST_PANEL_MODE.with(|m| m.get());
    let (merged, ai_start) = merge_ai_candidates(&ctx);
    AI_START.with(|s| s.set(ai_start));
    let mut view_ctx = ctx;
    view_ctx.candidates = merged;
    let extra = crate::dll_path()
        .parent()
        .map(|dir| dir.join("schemas").join("shurufa-skin.json"));
    let skin = skin::load_with(|| extra);
    let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);
    let screen_w = logical_screen_dim(unsafe { GetSystemMetrics(SM_CXSCREEN) }, dpi);
    let screen_h = logical_screen_dim(unsafe { GetSystemMetrics(SM_CYSCREEN) }, dpi);
    let (width, height) = compute_show_layout(
        hwnd,
        &view_ctx,
        &skin,
        dpi,
        panel_mode,
        screen_w,
        screen_h,
        ai_start,
    );
    // 位置保持当前窗口位置，仅按新宽高重排
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
        let _ = MoveWindow(hwnd, rect.left, rect.top, width, height, false);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
    let uia_text = view_ctx
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{}.{}", i + 1, c.text))
        .collect::<Vec<_>>()
        .join("，");
    crate::uia_provider::update_candidate_text(&uia_text);
    crate::debug_log(&format!(
        "cand AI 刷新: preedit={:?} ai_start={:?} w={} h={}",
        view_ctx.preedit, ai_start, width, height
    ));
}


impl CandidateUi {
    pub fn new() -> Self {
        // 预热皮肤缓存：TSF DLL 部署形态下皮肤文件在 DLL 旁的 schemas 目录
        let extra = crate::dll_path()
            .parent()
            .map(|dir| dir.join("schemas").join("shurufa-skin.json"));
        let _ = skin::load_with(|| extra);
        // 预热 GPU 工厂（< 5ms）；失败则整段会话降级，不 panic
        crate::candidate_window_d2d::try_init();
        crate::candidate_window_dcomp::try_init();
        CandidateUi {
            hwnd: None,
            shadow: ShadowShell::new(),
            last_rect: None,
            visible: false,
            last_fp: None,
            last_width: 0,
            last_height: 0,
        }
    }

    fn ensure_window(&mut self) -> Option<HWND> {
        if let Some(hwnd) = self.hwnd {
            return Some(hwnd);
        }
        unsafe {
            let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
            CLASS_REGISTERED.with_borrow_mut(|registered| {
                if !*registered {
                    let class = WNDCLASSW {
                        style: CS_HREDRAW | CS_VREDRAW,
                        lpfnWndProc: Some(wnd_proc),
                        hInstance: hinstance.into(),
                        lpszClassName: CLASS_NAME,
                        hbrBackground: HBRUSH::default(),
                        // 不设光标会导致悬停时一直显示忙碌转圈
                        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                        ..Default::default()
                    };
                    // 同进程重复注册返回 0，忽略即可
                    RegisterClassW(&class);
                    *registered = true;
                }
            });
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!(""),
                WS_POPUP,
                0,
                0,
                BASE_MIN_WIDTH,
                BASE_ROW_HEIGHT,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
            .ok()?;
            // 现代外观：Win11 圆角 + 沉浸式深色边框 + 按皮肤透明度分层
            skin::apply_appearance(hwnd, &Skin::current());
            PAINT_HWND.with(|h| h.set(hwnd));
            self.hwnd = Some(hwnd);
            Some(hwnd)
        }
    }

    /// 内容指纹（P0 #5）：候选窗显示内容 + 皮肤 + DPI 的 FNV-1a 散列。
    /// 指纹相同 ⇒ 布局（宽高）与绘制结果必然相同，可安全跳过字体实测
    /// 与重绘。覆盖：preedit、候选文本/副标、高亮序号、页码、中英/全角
    /// 模式、DPI、皮肤字号倍率与全部间距参数。
    fn content_fingerprint(ctx: &Context, skin: &Skin, dpi: u32) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = FNV_OFFSET;
        fn mix(hash: &mut u64, byte: u8) {
            *hash ^= byte as u64;
            *hash = hash.wrapping_mul(FNV_PRIME);
        }
        for byte in ctx.preedit.bytes() {
            mix(&mut hash, byte);
        }
        mix(&mut hash, ctx.highlighted as u8);
        mix(&mut hash, ctx.page_no as u8);
        mix(&mut hash, ctx.is_last_page as u8);
        mix(&mut hash, ctx.is_ascii as u8);
        mix(&mut hash, ctx.is_full_shape as u8);
        mix(&mut hash, dpi as u8);
        mix(&mut hash, (dpi >> 8) as u8);
        for c in ctx.candidates.iter().take(9) {
            for byte in c.text.bytes() {
                mix(&mut hash, byte);
            }
            mix(&mut hash, 0xff);
            for byte in c.comment.bytes() {
                mix(&mut hash, byte);
            }
            mix(&mut hash, 0xfe);
        }
        // 皮肤：字号倍率（f32 位模式）+ 间距/圆角参数 + 透明度
        let m = skin.metrics;
        for byte in m.font_scale.to_bits().to_le_bytes() {
            mix(&mut hash, byte);
        }
        for byte in m.opacity.to_bits().to_le_bytes() {
            mix(&mut hash, byte);
        }
        for v in [
            m.radius,
            m.padding,
            m.item_gap,
            m.label_gap,
            m.hl_pad,
            m.row_h,
            m.preedit_h,
        ] {
            for byte in (v as u32).to_le_bytes() {
                mix(&mut hash, byte);
            }
        }
        mix(&mut hash, m.scrollbar as u8);
        hash
    }

    /// 用引擎上下文刷新窗口内容并显示在锚点下方。
    pub fn show(
        &mut self,
        ctx: &Context,
        anchor: Option<POINT>,
        position: PositionMode,
        panel_mode: CandidatePanelMode,
    ) {
        let Some(hwnd) = self.ensure_window() else {
            return;
        };
        // 每次弹出重读皮肤：文件改动/主题改动即时生效（候选文件 <128KiB，代价可忽略）
        let extra = crate::dll_path()
            .parent()
            .map(|dir| dir.join("schemas").join("shurufa-skin.json"));
        let skin = skin::load_with(|| extra);
        skin::apply_appearance(hwnd, &skin);
        log_icon_once(&skin);
        // 部分宿主（DPI 虚拟化）对弹窗返回 96 兜底值，取系统 DPI 兜底
        let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);
        // 屏幕逻辑尺寸（物理 SM_CXSCREEN 换算；布局/钳制全程用逻辑像素）
        let screen_w = logical_screen_dim(unsafe { GetSystemMetrics(SM_CXSCREEN) }, dpi);
        let screen_h = logical_screen_dim(unsafe { GetSystemMetrics(SM_CYSCREEN) }, dpi);

        // AI 候选（2026-08-20）：保存原 ctx 快照供结果刷新重建；合并当前
        // preedit 匹配的 AI 候选到候选列表尾部（引擎候选保留前 RIME_KEEP 个）。
        LAST_CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
        LAST_PANEL_MODE.with(|m| m.set(panel_mode));
        let (merged_candidates, ai_start) = merge_ai_candidates(ctx);
        AI_START.with(|s| s.set(ai_start));
        let mut view_ctx = ctx.clone();
        view_ctx.candidates = merged_candidates;

        // 内容指纹短路（P0 #5）：候选内容未变则跳过字体实测与重绘。
        // 布局只依赖（内容 + 皮肤 + DPI），指纹相同 ⇒ 宽高相同，可直接复用。
        let fp = Self::content_fingerprint(&view_ctx, &skin, dpi);
        // 面板模式参与指纹：单行/多行切换时布局缓存必须失效（内容相同但
        // 布局不同）。multi 布局随 M7 落地，选项层先行、模式已生效。
        let fp = fp ^ ((panel_mode as u64) << 56);
        let content_changed = self.last_fp != Some(fp);

        let (width, height) = if content_changed {
            let out = compute_show_layout(
                hwnd,
                &view_ctx,
                &skin,
                dpi,
                panel_mode,
                screen_w,
                screen_h,
                ai_start,
            );
            self.last_fp = Some(fp);
            self.last_width = out.0;
            self.last_height = out.1;
            out
        } else {
            crate::debug_log("cand show: SKIP（内容指纹未变）");
            (self.last_width, self.last_height)
        };

        unsafe {
            let (mut x, mut y) = match position {
                // 固定角落：忽略锚点，按屏幕尺寸定位（随后 clamp 到可视区）
                PositionMode::FixedBottomRight => (screen_w, screen_h),
                PositionMode::FixedBottomLeft => (0, screen_h),
                PositionMode::Follow => match anchor {
                    Some(p) => (p.x, p.y + scale(4, dpi)),
                    // 拿不到光标位置时放屏幕左下角兜底（screen_h 已是逻辑像素）
                    None => (60, screen_h - height - 120),
                },
            };
            // 防止超出屏幕右/下边缘（screen_w/h 已换算为逻辑像素，与
            // width/height 同空间；此前混用物理 SM_CXSCREEN 会放行越界）
            x = x.min(screen_w - width - 8).max(0);
            y = y.min(screen_h - height - 8).max(0);

            // UI 节流（2026-08-16，weasel#1869 同类：长跑宿主下重复
            // ShowWindow/SetWindowPos 会随运行时间变慢，show 24→2.6ms）：
            // - 几何未变不重复 MoveWindow；
            // - 已显示不重复 ShowWindow；已隐藏不重复 SW_HIDE；
            // - 内容更新总是 InvalidateRect（候选/高亮可能变化）。
            let rect_changed = self.last_rect != Some((x, y, width, height));
            if rect_changed {
                // bRepaint=false：避免 MoveWindow 立即触发一次 WM_PAINT，与下方
                // InvalidateRect 重复重绘（每键两次全窗口绘制）。
                let _ = MoveWindow(hwnd, x, y, width, height, false);
                self.last_rect = Some((x, y, width, height));
            }
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            // 刚显示（hide→show 同一内容）也需重绘：ShowWindow 后窗口内容
            // 可能被系统丢弃，必须 InvalidateRect 重画。
            let was_visible = self.visible;
            if !self.visible {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                self.visible = true;
            }
            // 阴影壳只在几何变化时同步（SetLayeredWindowAttributes + SetWindowPos
            // 是每键可省的系统调用；长跑宿主下这类窗口操作会变慢）
            if rect_changed {
                self.shadow.sync(hwnd, x, y, width, height, &skin.shadow);
            }
            // 重绘短路（P0 #5）：内容指纹未变且几何未变且窗口持续可见时
            // 整帧跳过重绘——内容/几何/可见性任一变化才 InvalidateRect，
            // 避免每键全窗口重绘（组合内容未变的按键零成本）。
            if content_changed || rect_changed || !was_visible {
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
            // v1.2 读屏：候选文本 → UIA Provider Name（NVDA/讲述人朗读候选）
            let uia_text = ctx
                .candidates
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{}.{}", i + 1, c.text))
                .collect::<Vec<_>>()
                .join("，");
            crate::uia_provider::update_candidate_text(&uia_text);
        }
    }

    pub fn hide(&mut self) {
        self.shadow.hide();
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
        self.visible = false;
        crate::uia_provider::clear_candidate_text();
    }

    /// 触发一次重绘：模式角标 / 长按大写提示等不发新候选、仅刷新外观的场景用。
    /// 窗口不存在（尚未 show 过）时静默 no-op —— 下帧 show 自然会带出最新状态。
    pub fn invalidate(&self) {
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
        }
    }

    pub fn destroy(&mut self) {
        self.shadow.destroy();
        if let Some(hwnd) = self.hwnd.take() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        // 释放 GPU target/brushes（HWND 已死，引用随之失效）
        crate::candidate_window_d2d::shutdown();
        crate::candidate_window_dcomp::shutdown();
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        value if value == WM_PAINT => {
            // 调度：瀑布链 DComp → D2D → GDI。当帧失败 (false) 立刻落下一级
            // 重画，画面不丢；三条路径共用同一份 thread-local PaintData 布局槽位，
            // 视觉 1:1。档位缓存在 thread-local BACKEND_KIND，不在窗口过程里
            // 反复探测硬件。
            let view = make_paint_view();
            let drawn = match backend_kind() {
                BackendKind::DComp => view
                    .as_ref()
                    .map(|v| crate::candidate_window_dcomp::paint(hwnd, &client_rect(hwnd), v))
                    .unwrap_or(false),
                BackendKind::D2D => view
                    .as_ref()
                    .map(|v| crate::candidate_window_d2d::paint(hwnd, &client_rect(hwnd), v))
                    .unwrap_or(false),
                BackendKind::Gdi => {
                    // GDI 路径：首轮 / Failed / TDR 未完成 / 任何内部错误
                    let mut ps = PAINTSTRUCT::default();
                    let hdc = BeginPaint(hwnd, &mut ps);
                    paint(hdc, &ps.rcPaint);
                    let _ = EndPaint(hwnd, &ps);
                    true
                }
            };
            if drawn && backend_kind() != BackendKind::Gdi {
                // GPU 路径已完成 BeginDraw/EndDraw；仍须 ValidateRect 清 dirty 区
                let _ = ValidateRect(Some(hwnd), None);
            } else if !drawn {
                // 当帧 GPU 故障：立刻落 GDI 重画一遍（与既有回退语义一致）
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                paint(hdc, &ps.rcPaint);
                let _ = EndPaint(hwnd, &ps);
                let _ = ValidateRect(Some(hwnd), None);
            }
            LRESULT(0)
        }
        value if value == WM_SIZE => {
            // 本帧起 swapchain/target 尺寸失配：标记失效，下一帧按
            // GetClientRect 重建（GDI 无所谓，自动按 BeginPaint 走）
            crate::candidate_window_d2d::notify_resize();
            crate::candidate_window_dcomp::notify_resize();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        value if value == WM_DPICHANGED => {
            // DPI 失配：target 与字形都要重建/重测
            crate::candidate_window_d2d::notify_resize();
            crate::candidate_window_dcomp::notify_resize();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        value if value == WM_DESTROY => {
            crate::candidate_window_d2d::shutdown();
            crate::candidate_window_dcomp::shutdown();
            crate::uia_provider::clear_candidate_text();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        value if value == WM_GETOBJECT => {
            // v1.2 读屏：UIA 原始元素 Provider（WM_GETOBJECT → UiaRootObjectId）
            if let Some(lr) = unsafe { crate::uia_provider::on_wm_getobject(hwnd, wparam, lparam) }
            {
                lr
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        value if value == WM_LBUTTONDOWN => {
            select_candidate_at(lparam);
            LRESULT(0)
        }
        value if value == WM_AI_CANDIDATES_READY => {
            // AI 候选结果到达（worker 线程 PostMessage；同进程指针传递）。
            crate::debug_log("WM_AI_CANDIDATES_READY 到达候选窗");
            refresh_with_ai(lparam.0);
            LRESULT(0)
        }
        value if value == WM_RBUTTONDOWN => {
            // 右键候选：复制/删词/降频/隐藏/打开设置（搜狗 16.3b 菜单入口同类）。
            show_candidate_menu(hwnd, lparam);
            LRESULT(0)
        }
        value if value == WM_MOUSEMOVE => {
            update_hover(lparam);
            LRESULT(0)
        }
        // WM_MOUSELEAVE = 0x02A3 = 675（windows crate 的 UI::Controls 特性未启用，用裸值）
        675 => {
            clear_hover();
            LRESULT(0)
        }
        value if value == WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16;
            send_virtual_key(if delta < 0 { 0x22 } else { 0x21 });
            LRESULT(0)
        }
        value if value == WM_SETTINGCHANGE => {
            // 主题热切换："ImmersiveColorSet" 到达 → 刷新皮肤缓存并重绘
            if skin::is_immersive_color_change(lparam) {
                let skin = Skin::refresh_on_setting_change();
                unsafe {
                    skin::apply_appearance(hwnd, &skin);
                    crate::candidate_window_d2d::notify_skin_changed();
                    crate::candidate_window_dcomp::notify_skin_changed();
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn client_rect(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut r);
    }
    r
}

/// 命中测试：把客户区坐标映射到当前候选序号。
/// 供 WM_MOUSEMOVE（悬停高亮）与 WM_LBUTTONDOWN（点击选词）共用，
/// 保证两处几何完全一致；None = 落在候选行之外。
fn hit_test_item(x: i32, y: i32) -> Option<usize> {
    PAINT_DATA.with_borrow(|data| {
        let dpi = data.dpi;
        let m = data.skin.metrics;
        let row_top = scale(m.padding_or(BASE_PADDING), dpi)
            + data.tab_h
            + scale(m.preedit_h_or(BASE_PREEDIT_HEIGHT), dpi);
        let row_h = scale(m.row_h_or(BASE_ROW_HEIGHT), dpi);
        if y < row_top {
            return None;
        }
        let row = (y - row_top) / row_h;
        let label_gap = scale(m.label_gap_or(BASE_LABEL_GAP), dpi);
        let item_padding = scale(m.hl_pad_or(BASE_HL_PAD), dpi);
        for (index, item) in data.items.iter().enumerate() {
            if item.row != row {
                continue;
            }
            let left = item.x - item_padding;
            let right = item.x + item.label_w + label_gap + item.text_w + item_padding;
            if x >= left && x <= right {
                return Some(index);
            }
        }
        None
    })
}

/// 将点击坐标映射到当前候选，并发送 Rime 已支持的数字选词键。
unsafe fn select_candidate_at(lparam: LPARAM) {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    let dpi = PAINT_DATA.with_borrow(|data| data.dpi);
    // M7-5：Tab 行点击（y 落在顶部 Tab 区域）→ 切换候选服务组。
    let tab_click = PAINT_DATA.with_borrow(|data| {
        if !data.show_tab_bar || data.tab_h <= 0 {
            return None;
        }
        let m = data.skin.metrics;
        let padding = scale(m.padding_or(BASE_PADDING), dpi);
        if y < padding || y >= padding + data.tab_h {
            return None;
        }
        let tab_pad = scale(6, dpi);
        let gap = scale(4, dpi);
        let char_w = font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, m.font_scale);
        let rime_w = char_w * 2 + tab_pad * 2;
        let en_w = char_w * 2 + tab_pad * 2;
        if x >= padding && x < padding + rime_w {
            Some(TabKind::Rime)
        } else if x >= padding + rime_w + gap && x < padding + rime_w + gap + en_w {
            Some(TabKind::English)
        } else {
            None
        }
    });
    if let Some(tab) = tab_click {
        crate::debug_log(&format!("候选 Tab 切换：{tab:?}"));
        tab_switch(tab);
        return;
    }
    if let Some(index) = hit_test_item(x, y) {
        // M7-5：英文组候选（激活英文 Tab 且显示 Tab 行）→ 提交钩子落盘。
        // 用本帧实际显示的组（自动切英文时 ACTIVE_TAB 可能未同步，但
        // PAINT_DATA.tab_active 是 compute 写入的当帧状态）。
        let tab = PAINT_DATA.with_borrow(|d| d.tab_active);
        let en_bar = PAINT_DATA.with_borrow(|d| d.show_tab_bar);
        if tab == TabKind::English && en_bar {
            let preedit = LAST_CTX.with(|c| c.borrow().as_ref().map(|c| c.preedit.clone()));
            let text = preedit
                .and_then(|p| crate::english_candidates::suggest(&p).get(index).cloned());
            if let Some(text) = text {
                let done = AI_COMMIT.with(|slot| {
                    slot.borrow().as_ref().map(|f| f(&text)).unwrap_or(false)
                });
                if !done {
                    send_virtual_key(VK_AI_COMMIT_TRIGGER);
                }
                crate::debug_log(&format!("英文候选点击提交：{text:?}"));
            }
            return;
        }
        // AI 候选（2026-08-20）：索引落在 AI 起始之后 → 取完整原文（显示
        // 文本可能被缩写截断），交提交钩子（service 写 pending 槽），再回发
        // 回发 Enter 让 TSF 走编辑会话把文本落盘（pending 非空时消费）。
        // AI 候选不是 librime 候选，不能走数字选词（引擎索引对不上）。
        let ai_start = AI_START.with(|s| s.get());
        if let Some(start) = ai_start {
            if index >= start {
                let preedit = LAST_CTX.with(|c| c.borrow().as_ref().map(|c| c.preedit.clone()));
                let k = index - start;
                let text = preedit.and_then(|p| {
                    AI_CANDIDATES.with(|c| {
                        c.borrow()
                            .iter()
                            .filter(|(pp, _)| *pp == p)
                            .nth(k)
                            .map(|(_, t)| t.clone())
                    })
                });
                if let Some(text) = text {
                    let done = AI_COMMIT.with(|slot| {
                        slot.borrow().as_ref().map(|f| f(&text)).unwrap_or(false)
                    });
                    if !done {
                        // 直接提交失败（非 TSF 认可时机等）：回发触发键，
                        // 由 handle_key 的 pending_ai 兜底提交。
                        send_virtual_key(VK_AI_COMMIT_TRIGGER);
                    }
                }
                return;
            }
        }
        // 简拼词（2026-08-21）：引擎候选为空时由 algo 前端注入的
        // 简拼候选（comment=“简拼”）不是 librime 候选，点击不能走
        // 数字选词（引擎无这个候选，数字键会直接上屏数字）——
        // 走 AI 同款提交钩子（service 写 pending 槽 + 回发 Enter 落盘）。
        let is_jianpin = LAST_CTX.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|c| c.candidates.get(index))
                .map(|c| c.comment == "简拼")
                .unwrap_or(false)
        });
        if is_jianpin {
            let text = LAST_CTX.with(|c| {
                c.borrow()
                    .as_ref()
                    .and_then(|c| c.candidates.get(index))
                    .map(|c| c.text.clone())
            });
            if let Some(text) = text {
                let done = AI_COMMIT.with(|slot| {
                    slot.borrow().as_ref().map(|f| f(&text)).unwrap_or(false)
                });
                if !done {
                    send_virtual_key(VK_AI_COMMIT_TRIGGER);
                }
                crate::debug_log(&format!("简拼词点击提交：{text:?}"));
            }
            return;
        }
        // 序号键 1..9 → 0x31..0x39；第 10 项按下标 0 → 0x30。
        let key = if index >= 9 { 0x30 } else { 0x31 + index as u8 };
        send_virtual_key(key);
        return;
    }
    // 落在任何 item 之外但命中滚动条轨道：纯视觉条不响应拖动，按系统习惯
    // 轨道点击 = 向上/下翻一页（PageUp/PageDown，与滚轮一致）。
    PAINT_DATA.with_borrow(|data| {
        if !data.items.is_empty() && data.skin.metrics.scrollbar && data.page.total_pages() > 1 {
            let track_w = scale(skin::SCROLLBAR_BASE_WIDTH, dpi);
            let win_right = PAINT_WIN_W.with(|w| w.get());
            if win_right > 0 && x >= win_right - track_w {
                let m = data.skin.metrics;
                let rows = data.items.iter().map(|it| it.row).max().unwrap_or(0) + 1;
                let mid = scale(m.padding_or(BASE_PADDING), dpi)
                    + data.tab_h
                    + scale(m.preedit_h_or(BASE_PREEDIT_HEIGHT), dpi)
                    + scale(m.row_h_or(BASE_ROW_HEIGHT), dpi) * rows / 2;
                send_virtual_key(if y < mid { 0x21 } else { 0x22 });
            }
        }
    });
}

// ---------------------------------------------------------------------------
// 候选条右键菜单（M7，搜狗 16.3b 候选条菜单入口同类）
// ---------------------------------------------------------------------------

/// 右键菜单命令 id（与菜单项一一对应；TrackPopupMenu 返回 0 表示未选择）。
const IDM_COPY: usize = 1;
const IDM_DROP_CAND: usize = 2;
const IDM_DEMOTE_CAND: usize = 3;
const IDM_HIDE_CAND: usize = 4;
const IDM_OPEN_SETTINGS: usize = 5;

/// 菜单动作（纯映射，可单测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateMenuAction {
    /// 复制候选文本到剪贴板
    Copy,
    /// 从候选删除（等同引擎 Control+d）
    DropCandidate,
    /// 降低词频（等同引擎 Control+j）
    DemoteCandidate,
    /// 隐藏该词（等同引擎 Control+x）
    HideCandidate,
    /// 打开设置中心
    OpenSettings,
}

fn menu_action_for(cmd: usize) -> Option<CandidateMenuAction> {
    match cmd {
        IDM_COPY => Some(CandidateMenuAction::Copy),
        IDM_DROP_CAND => Some(CandidateMenuAction::DropCandidate),
        IDM_DEMOTE_CAND => Some(CandidateMenuAction::DemoteCandidate),
        IDM_HIDE_CAND => Some(CandidateMenuAction::HideCandidate),
        IDM_OPEN_SETTINGS => Some(CandidateMenuAction::OpenSettings),
        _ => None,
    }
}

/// CF_UNICODETEXT（windows crate 0.62 将其置于 Win32_System_Ole，
/// 为避免引入整棵 Ole 特性，此处用标准值 13）。
const CF_UNICODETEXT: u32 = 13;

/// 生成以 NUL 结尾的 UTF-16 宽串（菜单项文本用）。
fn menu_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 把候选文本写入系统剪贴板（CF_UNICODETEXT；失败静默不打扰输入）。
fn copy_candidate_text(text: &str) {
    if text.is_empty() {
        return;
    }
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    unsafe {
        if !OpenClipboard(None).is_ok() {
            return;
        }
        let _ = EmptyClipboard();
        let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) else {
            eprintln!("CLIP-DEBUG: GlobalAlloc failed");
            let _ = CloseClipboard();
            return;
        };
        let p = GlobalLock(hg);
        if p.is_null() {
            let _ = GlobalFree(Some(hg));
            let _ = CloseClipboard();
            return;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr().cast::<u8>(), p.cast::<u8>(), wide.len() * 2);
        let _ = GlobalUnlock(hg);
        // 设置成功时所有权转移给系统；失败必须释放，避免泄漏。
        if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hg.0))).is_err() {
            let _ = GlobalFree(Some(hg));
        }
        let _ = CloseClipboard();
    }
}

/// 启动设置中心（Shurufa.exe 位于 DLL 同目录；失败静默）。
fn open_settings_center() {
    if let Some(dir) = crate::dll_path().parent() {
        let exe = dir.join("Shurufa.exe");
        if exe.exists() {
            let _ = std::process::Command::new(exe).spawn();
        }
    }
}

/// 右键候选 → 弹出菜单并分发命令。lparam 为客户区坐标（低位 x，高位 y）。
unsafe fn show_candidate_menu(hwnd: HWND, lparam: LPARAM) {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    let Some(index) = hit_test_item(x, y) else {
        return;
    };
    let text = PAINT_DATA.with_borrow(|d| {
        d.items
            .get(index)
            .map(|it| it.text.clone())
            .unwrap_or_default()
    });
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    // 菜单项文本：AppendMenuW 要求 PCWSTR（w! 字面量不满足泛型 Param），
    // 用 Vec 宽串 + from_raw；系统在 AppendMenuW 时复制字符串，栈上足够存活。
    let s_copy = menu_wide("复制候选");
    let s_drop = menu_wide("从候选删除（Ctrl+D 同款）");
    let s_demote = menu_wide("降低词频（Ctrl+J 同款）");
    let s_hide = menu_wide("隐藏该词（Ctrl+X 同款）");
    let s_settings = menu_wide("打开设置中心");
    let _ = AppendMenuW(menu, MF_STRING, IDM_COPY, PCWSTR::from_raw(s_copy.as_ptr()));
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        IDM_DROP_CAND,
        PCWSTR::from_raw(s_drop.as_ptr()),
    );
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        IDM_DEMOTE_CAND,
        PCWSTR::from_raw(s_demote.as_ptr()),
    );
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        IDM_HIDE_CAND,
        PCWSTR::from_raw(s_hide.as_ptr()),
    );
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        IDM_OPEN_SETTINGS,
        PCWSTR::from_raw(s_settings.as_ptr()),
    );
    let mut pt = POINT { x, y };
    let _ = ClientToScreen(hwnd, &mut pt);
    // TPM_RETURNCMD：返回值即选中的命令 id（0 = 未选择）；BOOL.0 承载该值。
    let cmd = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        None,
        hwnd,
        None,
    );
    let _ = DestroyMenu(menu);
    if cmd.0 != 0 {
        dispatch_candidate_menu(cmd.0 as usize, index, &text);
    }
}

/// 方向键把高亮移动到第 index 个候选（0 起；引擎不提交组合，
/// 与集成测试 cold_word_menu 的键序一致）。
fn move_highlight_for_menu(index: usize) {
    for _ in 0..index {
        engine_simulate("{Down}");
    }
}

fn dispatch_candidate_menu(cmd: usize, index: usize, text: &str) {
    let Some(action) = menu_action_for(cmd) else {
        return;
    };
    match action {
        CandidateMenuAction::Copy => copy_candidate_text(text),
        CandidateMenuAction::DropCandidate => {
            // 方向键把高亮移到右键项（不提交），再触发引擎冷词丢弃。
            move_highlight_for_menu(index);
            engine_simulate("{Control+d}");
        }
        CandidateMenuAction::DemoteCandidate => {
            move_highlight_for_menu(index);
            engine_simulate("{Control+j}");
        }
        CandidateMenuAction::HideCandidate => {
            move_highlight_for_menu(index);
            engine_simulate("{Control+x}");
        }
        CandidateMenuAction::OpenSettings => open_settings_center(),
    }
}

/// WM_MOUSEMOVE：更新悬停高亮。首次进入时注册 WM_MOUSELEAVE 跟踪，
/// 悬停项变化或首次悬停时 InvalidateRect 触发重绘。
fn update_hover(lparam: LPARAM) {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    let idx = hit_test_item(x, y);
    let changed = HOVER_INDEX.with(|h| {
        if h.get() == idx {
            false
        } else {
            h.set(idx);
            true
        }
    });
    if idx.is_some() && !HOVER_TRACKING.with(|t| t.get()) {
        unsafe {
            let mut tme = std::mem::zeroed::<TRACKMOUSEEVENT>();
            tme.cbSize = std::mem::size_of::<TRACKMOUSEEVENT>() as u32;
            tme.dwFlags = TME_LEAVE;
            tme.hwndTrack = PAINT_HWND.with(|h| h.get());
            let _ = TrackMouseEvent(&mut tme);
        }
        HOVER_TRACKING.with(|t| t.set(true));
    }
    if changed {
        let hwnd = PAINT_HWND.with(|h| h.get());
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
}

/// WM_MOUSELEAVE：清除悬停并重绘。
fn clear_hover() {
    HOVER_TRACKING.with(|t| t.set(false));
    let changed = HOVER_INDEX.with(|h| {
        if h.get().is_none() {
            false
        } else {
            h.set(None);
            true
        }
    });
    if changed {
        let hwnd = PAINT_HWND.with(|h| h.get());
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
}

// paint 时记录客户区宽度供命中测试使用（同 UI 线程，Cell 足够）
thread_local! {
    static PAINT_WIN_W: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

/// 无焦点候选窗将操作发送给前台编辑器，继续走 TSF 的正常按键路径。
/// 把虚拟键注入前台窗口（候选点击选词 / AI 提交触发键用）。
/// 2026-08-20 修复：keybd_event(scan=0) 对部分键（如 VK_APPS）不触发
/// TSF OnKeyDown（实测 AI 候选点击后提交键无声无息）；改 SendInput +
/// MapVirtualKey 补全 scan code，与 host ai_panel 同款注入。
unsafe fn send_virtual_key(vk: u8) {
    let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16;
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk as u16),
                wScan: scan,
                dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() },
                ..Default::default()
            },
        },
    };
    let inputs = [key(false), key(true)];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}



unsafe fn paint(hdc: HDC, rc: &RECT) {
    PAINT_DATA.with_borrow(|data| {
        let dpi = data.dpi;
        let m = data.skin.metrics;
        let padding = scale(m.padding_or(BASE_PADDING), dpi);
        let label_gap = scale(m.label_gap_or(BASE_LABEL_GAP), dpi);
        let hl_pad = scale(m.hl_pad_or(BASE_HL_PAD), dpi);
        let preedit_h = scale(m.preedit_h_or(BASE_PREEDIT_HEIGHT), dpi);
        let row_h = scale(m.row_h_or(BASE_ROW_HEIGHT), dpi);
        let colors = data.skin.candidate;
        let font_scale = m.font_scale;

        let bg = CreateSolidBrush(COLORREF(colors.background));
        FillRect(hdc, rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkMode(hdc, TRANSPARENT);
        PAINT_WIN_W.with(|w| w.set(rc.right));

        // 皮肤滚动条（GDI 路径）：右缘 4px 轨道 + 页位置 thumb；
        // 仅在开关开启且多页时绘制，纯视觉、不影响布局槽位。
        if data.skin.metrics.scrollbar && data.page.total_pages() > 1 {
            let track_w = scale(skin::SCROLLBAR_BASE_WIDTH, dpi);
            let rows = data.items.iter().map(|it| it.row).max().unwrap_or(0) + 1;
            // 多行面板 thumb 按"每页内容高度"定长；单行沿用最宽候选（现状）。
            let item_w = if rows > 1 {
                rows * row_h
            } else {
                data.items
                    .iter()
                    .map(|it| it.label_w + label_gap + it.text_w + it.badge_w + hl_pad * 2)
                    .max()
                    .unwrap_or(scale(96, dpi))
            };
            if let Some(geo) = skin::scrollbar_geo(
                rc.right,
                rc.bottom,
                item_w,
                padding,
                track_w,
                data.page.page_no,
                data.page.total_pages(),
            ) {
                let (track_c, thumb_c) = skin::scrollbar_colors(&data.skin);
                let (track_b, thumb_b) = (
                    CreateSolidBrush(COLORREF(track_c)),
                    CreateSolidBrush(COLORREF(thumb_c)),
                );
                let track_r = RECT {
                    left: geo.track[0],
                    top: geo.track[1],
                    right: geo.track[2],
                    bottom: geo.track[3],
                };
                let thumb_r = RECT {
                    left: geo.thumb[0],
                    top: geo.thumb[1],
                    right: geo.thumb[2],
                    bottom: geo.thumb[3],
                };
                FillRect(hdc, &track_r, track_b);
                FillRect(hdc, &thumb_r, thumb_b);
                let _ = DeleteObject(HGDIOBJ(track_b.0));
                let _ = DeleteObject(HGDIOBJ(thumb_b.0));
            }
        }

        // M7-5 候选服务 Tab 行（有英文候选时显示；preedit 上方）
        if data.show_tab_bar && data.tab_h > 0 {
            let tab_font = make_font(font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale));
            let old_tab = SelectObject(hdc, HGDIOBJ(tab_font.0));
            let tab_pad = scale(6, dpi);
            let gap = scale(4, dpi);
            let rime_w = text_width(hdc, "拼音") + tab_pad * 2;
            let en_w = text_width(hdc, "英文") + tab_pad * 2;
            let rime_active = data.tab_active == TabKind::Rime;
            draw_tab_label(hdc, "拼音", padding, padding, rime_w, data.tab_h, rime_active, &colors);
            draw_tab_label(
                hdc,
                "英文",
                padding + rime_w + gap,
                padding,
                en_w,
                data.tab_h,
                !rime_active,
                &colors,
            );
            SelectObject(hdc, old_tab);
        }

        // 预编辑串（小号灰字，第一行）
        let preedit_font = make_font(font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale));
        let old_font = SelectObject(hdc, HGDIOBJ(preedit_font.0));
        SetTextColor(hdc, COLORREF(colors.preedit));
        // 右上角模式角标。规则与 make_paint_view 一致（mode_badge_text），
        // caps_visual 优先；None 时不留角标宽，把空间全部交还 preedit。
        let mode_badge = mode_badge_text(data).unwrap_or("");
        let mode_badge_w = if mode_badge.is_empty() {
            0
        } else {
            text_width(hdc, mode_badge) + scale(BASE_MODE_BADGE_GAP, dpi)
        };
        let preedit_w = (rc.right - padding * 2 - mode_badge_w).max(scale(BASE_MIN_WIDTH, dpi));
        let breaks = syllable_breaks(&data.preedit);
        if breaks.is_empty() {
            draw_line(hdc, &data.preedit, padding, padding + data.tab_h, preedit_w, preedit_h);
        } else {
            // 含音节分隔符：跳过分隔符本体、逐段交替色 + 1px 竖线占位。
            let preedit_font_h = font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale);
            draw_preedit_segmented(
                hdc,
                &data.preedit,
                &breaks,
                &colors,
                padding,
                preedit_h,
                preedit_font_h,
                data.tab_h,
            );
        }
        // 角标本体：highlight 底色块 + 反色文字（skin.candidate.background 当反色，
        // 因为 highlight_background 与 background 的强对比是皮肤语义自带的）。
        if !mode_badge.is_empty() {
            let badge_left = padding + preedit_w;
            let badge_right = rc.right - padding;
            let badge_rect = RECT {
                left: badge_left,
                top: padding,
                right: badge_right,
                bottom: padding + preedit_h,
            };
            let hl_bg = CreateSolidBrush(COLORREF(colors.highlight_background));
            FillRect(hdc, &badge_rect, hl_bg);
            let _ = DeleteObject(HGDIOBJ(hl_bg.0));
            SetTextColor(hdc, COLORREF(colors.background));
            draw_line(hdc, mode_badge, badge_left, padding, badge_right, preedit_h);
        }

        // 候选行（第二行横排）
        let cand_font = make_font(font_height(BASE_FONT_HEIGHT, dpi, font_scale));
        SelectObject(hdc, HGDIOBJ(cand_font.0));
        let row_top = padding + data.tab_h + preedit_h;
        for item in &data.items {
            // 多行面板：本行顶 = 首行顶 + 行号 × 行高（单行恒 0）。
            let item_top = row_top + item.row * row_h;
            let item_end = item.x + item.label_w + label_gap + item.text_w;
            if item.highlighted {
                let hl = CreateSolidBrush(COLORREF(colors.highlight_background));
                let hl_rect = RECT {
                    left: item.x - hl_pad,
                    top: item_top,
                    right: item_end + hl_pad,
                    bottom: item_top + row_h,
                };
                FillRect(hdc, &hl_rect, hl);
                let _ = DeleteObject(HGDIOBJ(hl.0));
            } else if item.hovered {
                // 悬停（非选中）：highlight 向 background 过渡 40% 的浅底，
                // 与选中态强对比区分，同时有明确的"可点"反馈。
                let hover_c = blend_colorref(colors.highlight_background, colors.background, 400);
                let hv = CreateSolidBrush(COLORREF(hover_c));
                let hv_rect = RECT {
                    left: item.x - hl_pad,
                    top: item_top,
                    right: item_end + hl_pad,
                    bottom: item_top + row_h,
                };
                FillRect(hdc, &hv_rect, hv);
                let _ = DeleteObject(HGDIOBJ(hv.0));
            }

            SetTextColor(hdc, COLORREF(colors.label));
            draw_line(
                hdc,
                &item.label,
                item.x,
                item_top,
                item.x + item.label_w,
                row_h,
            );

            SetTextColor(hdc, COLORREF(colors.text));
            draw_line(
                hdc,
                &item.text,
                item.x + item.label_w + label_gap,
                item_top,
                item_end,
                row_h,
            );

            // 副标：词库附注（emoji、近义词、词条类别等），灰字小一号，
            // 紧跟主文本右侧（与搜狗/Rime 候选副标一致），不另起一行，
            // 避免抬高候选行高。text_w 已含 comment 宽度（见 show()），
            // 这里用 cand_font 实测主文本宽度定位 comment 起点。
            if !item.comment.is_empty() {
                let pure_text_w = text_width(hdc, &item.text);
                let sub_font = make_font(font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale));
                SelectObject(hdc, HGDIOBJ(sub_font.0));
                SetTextColor(hdc, COLORREF(colors.label));
                draw_line(
                    hdc,
                    &item.comment,
                    item.x + item.label_w + label_gap + pure_text_w + scale(2, dpi),
                    item_top,
                    item_end,
                    row_h,
                );
                SelectObject(hdc, HGDIOBJ(cand_font.0));
            }
        }

        SelectObject(hdc, old_font);
    });
}

/// 混合 `fg` 朝 `bg` 方向 `permille` 千分比（COLORREF, 0x00BBGGRR）。
/// `permille=0` → fg 本色；`1000` → bg 本色。
/// 供音节分段派生色环：不改 skin.rs（任务约束），分隔竖线与段间交替色
/// 全部由既有 candidate 颜色派生。
pub fn blend_colorref(fg: u32, bg: u32, permille: u32) -> u32 {
    let pm = permille.min(1000);
    let ch = |shift: u32| {
        let f = (fg >> shift) & 0xff;
        let b = (bg >> shift) & 0xff;
        (f * (1000 - pm) + b * pm + 500) / 1000
    };
    ch(0) | (ch(8) << 8) | (ch(16) << 16)
}

/// 派生分段交替色环（闭循环，无皮肤侵入）：
/// - 偶数段 = preedit 本色；
/// - 奇数段 = preedit 向 text 过渡 28%（可读，不扎眼）；
/// - 分隔竖线 = preedit 本色，与偶数段合并为同色系。
///
/// 返回 [seg_even, seg_odd, separator_line]。
pub fn syllable_segment_colors(colors: &crate::skin::CandidateColors) -> [u32; 3] {
    [
        colors.preedit,
        blend_colorref(colors.preedit, colors.text, 280),
        colors.preedit,
    ]
}

/// 按 `syllable_breaks` 把 preedit 切成多段逐段绘制：断点字符本体跳过不入屏，
/// 槽位用来画 1px 竖线（水平居中于原字符宽度）；相邻段在交替色间轮换，
/// 形成 `A|B` 分段视觉。排版宽度与原文完全一致（各段 GetTextExtentPoint32W
/// 实测顺序推进），对布局槽位与窗口宽度零影响。
///
/// `hdc` 须已选入 preedit 字体；`padding`/`preedit_h`/`preedit_font_h` 由调用方
/// 按 (dpi, font_scale) 展开后传入。
unsafe fn draw_preedit_segmented(
    hdc: HDC,
    preedit: &str,
    breaks: &[u16],
    colors: &crate::skin::CandidateColors,
    padding: i32,
    preedit_h: i32,
    preedit_font_h: i32,
    tab_h: i32,
) {
    let wide: Vec<u16> = preedit.encode_utf16().collect();
    let n = wide.len();
    let [even_c, odd_c, sep_c] = syllable_segment_colors(colors);
    // 竖线高度 = 字形高度的 1/2 居中；上下各留 25% 空白让分隔与文字区分离。
    let line_half = (preedit_font_h / 2).max(4);
    let center_y = padding + tab_h + preedit_h / 2;
    let (ly0, ly1) = (center_y - line_half, center_y + line_half);

    let mut x = padding;
    let mut seg_start = 0usize;
    for (idx, &bp) in breaks.iter().enumerate() {
        let bp = (bp as usize).min(n);
        if bp < seg_start || bp >= n {
            continue;
        }
        // 段文本：颜色按奇偶轮换（首段从 even 起）。
        if bp > seg_start {
            let seg = &wide[seg_start..bp];
            let w = utf16_width(hdc, seg);
            let seg_c = if idx % 2 == 0 { even_c } else { odd_c };
            SetTextColor(hdc, COLORREF(seg_c));
            draw_line_utf16(hdc, seg, x, padding, x + w, preedit_h);
            x += w;
        }
        // 分隔符槽位：实测原字符宽度保持基线对齐；槽位中央画 1px 竖线。
        let sep_w = utf16_width(hdc, &wide[bp..bp + 1]);
        if sep_w > 0 {
            let pen: HPEN = CreatePen(PS_SOLID, 1, COLORREF(sep_c));
            let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
            let cx = x + sep_w / 2;
            let _ = MoveToEx(hdc, cx, ly0, None);
            let _ = LineTo(hdc, cx, ly1);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
        x += sep_w;
        seg_start = bp + 1;
    }
    // 尾段
    if seg_start < n {
        let seg = &wide[seg_start..];
        let w = utf16_width(hdc, seg);
        let seg_c = if breaks.len().is_multiple_of(2) {
            even_c
        } else {
            odd_c
        };
        SetTextColor(hdc, COLORREF(seg_c));
        draw_line_utf16(hdc, seg, x, padding, x + w, preedit_h);
    }
}

/// GetTextExtentPoint32W 直接吃 UTF-16 切片（避免 String 重编码分配）。
unsafe fn utf16_width(hdc: HDC, s: &[u16]) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, s, &mut size);
    size.cx
}

unsafe fn draw_line_utf16(hdc: HDC, wide: &[u16], left: i32, top: i32, right: i32, height: i32) {
    if wide.is_empty() {
        return;
    }
    let mut buf: Vec<u16> = wide.to_vec();
    let mut rect = RECT {
        left,
        top,
        right,
        bottom: top + height,
    };
    DrawTextW(
        hdc,
        &mut buf,
        &mut rect,
        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER,
    );
}

/// M7-5 候选服务 Tab 标签：激活态高亮底 + 反色文字，非激活态普通文字。
unsafe fn draw_tab_label(
    hdc: HDC,
    text: &str,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    active: bool,
    colors: &crate::skin::CandidateColors,
) {
    if active {
        let hl = CreateSolidBrush(COLORREF(colors.highlight_background));
        let rect = RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        };
        FillRect(hdc, &rect, hl);
        let _ = DeleteObject(HGDIOBJ(hl.0));
        SetTextColor(hdc, COLORREF(colors.background));
    } else {
        SetTextColor(hdc, COLORREF(colors.label));
    }
    draw_line(hdc, text, left + scale(6, GetDpiForSystem()), top, left + width - scale(6, GetDpiForSystem()), height);
}

unsafe fn draw_line(hdc: HDC, text: &str, left: i32, top: i32, right: i32, height: i32) {
    if text.is_empty() {
        return;
    }
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT {
        left,
        top,
        right,
        bottom: top + height,
    };
    DrawTextW(
        hdc,
        &mut utf16,
        &mut rect,
        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER,
    );
}

#[cfg(test)]
mod tests {
    use super::{
        abbreviate_text, blend_colorref, candidate_source_label, classify_candidate_source,
        mode_badge_text, syllable_breaks, syllable_segment_colors, BackendKind, CandidateSource,
        PaintData, PositionMode, TabKind,
    };

    /// 选路器可被显式改写并被 backend_kind() 速读出来。
    /// 用例固定 Gdi（最安全档），只验证 override 通道在。
    #[test]
    fn backend_kind_override_round_trip() {
        super::set_backend_kind_for_test(BackendKind::Gdi);
        assert_eq!(super::backend_kind(), BackendKind::Gdi);
    }

    fn paint_data(caps: bool, ascii: bool) -> PaintData {
        PaintData {
            preedit: String::new(),
            items: Vec::new(),
            dpi: 96,
            skin: super::Skin::default(),
            is_ascii: ascii,
            is_full_shape: false,
            caps_visual: caps,
            page: super::PageInfo::default(),
            show_tab_bar: false,
            tab_active: TabKind::Rime,
            tab_h: 0,
        }
    }

    /// mode_badge 文案：caps_visual 优先于 ascii 模式；常规态按 ascii 给 "En"/"中"。
    /// 这是候选窗右上角模式角标的唯一规则书，GDI/D2D/DComp 三路共享。
    #[test]
    fn mode_badge_text_prefers_caps_visual_over_ascii() {
        assert_eq!(mode_badge_text(&paint_data(true, false)), Some("⇪大写"));
        assert_eq!(mode_badge_text(&paint_data(true, true)), Some("⇪大写"));
        assert_eq!(mode_badge_text(&paint_data(false, false)), Some("中"));
        assert_eq!(mode_badge_text(&paint_data(false, true)), Some("En"));
    }

    /// 长按置位 → 短按清位的"两面旗"协议：set 幂等；caps_visual_active 是真值源。
    #[test]
    fn set_caps_visual_idempotent_and_query_active() {
        // 起点：两次 false。清除不清也该 false 返回。
        let _ = super::set_caps_visual(false);
        assert!(!super::caps_visual_active());
        // 置位首次返回变化 true；再次置同值 false。
        assert!(super::set_caps_visual(true));
        assert!(super::caps_visual_active());
        assert!(!super::set_caps_visual(true));
        assert!(super::caps_visual_active());
        // 清位也满足幂等。
        assert!(super::set_caps_visual(false));
        assert!(!super::caps_visual_active());
        assert!(!super::set_caps_visual(false));
    }

    /// 音节分隔符只识别空格与单引号；返回 UTF-16 码元索引。
    /// 例：`"ni hao shi"` → 空格在 UTF-16 下标 2、7，得到 `[2, 7]`。
    #[test]
    fn syllable_breaks_finds_space_and_apostrophe() {
        // 空 preedit 不该有断点
        assert!(syllable_breaks("").is_empty());
        // 整个串没有分隔符 → 空 vec
        assert!(syllable_breaks("nihao").is_empty());
        // 单个空格在 UTF-16 下标 2
        assert_eq!(syllable_breaks("ni hao"), vec![2]);
        // 单引号同样识别
        assert_eq!(syllable_breaks("ni'hao"), vec![2]);
        // 多个分隔符：空格 + 单引号 都能发现，索引按出现顺序
        assert_eq!(syllable_breaks("wo de'shi jie"), vec![2, 5, 9]);
        // 首字符即是分隔符 / 末字符即是分隔符
        assert_eq!(syllable_breaks(" ni"), vec![0]);
        assert_eq!(syllable_breaks("ni "), vec![2]);
        // 连续两个分隔符各自都需记录（绘制阶段跳过 bp < seg_start 的，但数据不能丢）
        assert_eq!(syllable_breaks("ni  hao"), vec![2, 3]);
    }

    /// syllable 分段色环：偶数段保持 preedit 本色，奇数段 = preedit → text 28% 过渡；
    /// 分隔竖线与偶数段同色系（preedit）。goldens 用具体 channel 锁差异。
    #[test]
    fn syllable_segment_colors_derive_without_skin_change() {
        let colors = crate::skin::CandidateColors::light();
        let [even, odd, sep] = syllable_segment_colors(&colors);
        // even 与 separator 严格等于 preedit 本色
        assert_eq!(even, colors.preedit);
        assert_eq!(sep, colors.preedit);
        // odd 介于 preedit 与 text 之间（各 channel 都向 text 偏移 28%）
        let pre_r = colors.preedit & 0xff;
        let text_r = colors.text & 0xff;
        let expected_r = (pre_r * 720 + text_r * 280 + 500) / 1000;
        assert_eq!(odd & 0xff, expected_r);
        // blend_colorref 边界：0% → fg，100% → bg，且 permille 上限 1000
        assert_eq!(blend_colorref(0x112233, 0xaabbcc, 0), 0x112233);
        assert_eq!(blend_colorref(0x112233, 0xaabbcc, 1000), 0xaabbcc);
        assert_eq!(blend_colorref(0x112233, 0xaabbcc, 5000), 0xaabbcc);
    }

    /// 真实窗口布局回归：9 个长候选在任意 DPI 下窗口宽度都必须被钳制到
    /// 屏幕宽度的 60% 以内（此前单行 9 候选在高 DPI 下横贯屏幕）。
    /// 在测试进程创建真实候选窗 + GDI 实测文本宽度，喂入足以超宽的候选。
    /// 上限按与生产一致的 logical_screen_dim 换算（SM_CXSCREEN 是物理像素，
    /// 候选窗布局是逻辑像素；2026-08-16 实机复现 150% 缩放未换算导致 65% 屏宽）。
    #[test]
    fn candidate_window_width_respects_screen_cap() {
        use super::CandidatePanelMode;
        use ime_ipc::Candidate;
        use windows::Win32::Foundation::{POINT, RECT};
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, GetWindowRect, SM_CXSCREEN,
        };
        let mut ui = super::CandidateUi::new();
        let ctx = ime_ipc::Context {
            preedit: "zhonghua renmin gongheguo".into(),
            candidates: (0..9)
                .map(|i| Candidate {
                    text: format!("中华人民共和国万岁{}", i),
                    comment: String::new(),
                })
                .collect(),
            ..ime_ipc::Context::default()
        };
        ui.show(
            &ctx,
            Some(POINT { x: 200, y: 200 }),
            PositionMode::Follow,
            CandidatePanelMode::Single,
        );
        let hwnd = ui.hwnd.expect("候选窗应创建成功");
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut r);
        };
        let w = (r.right - r.left).max(0);
        // 测试进程 DPI（GetDpiForWindow 实测）；生产路径按窗口 DPI 换算
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
        let screen_w =
            super::logical_screen_dim(unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(1), dpi);
        let cap = (screen_w * 6 / 10).max(super::scale(super::BASE_MIN_WIDTH, dpi));
        assert!(
            w <= cap,
            "候选窗宽度 {w}px 超过屏幕 60% 上限 {cap}px（逻辑屏宽 {screen_w}px @ dpi={dpi}）"
        );
        // 锚点在上方 200px 时窗口不应完全超出屏幕右缘（show 内有钳制）
        assert!(
            r.right <= screen_w + 8,
            "候选窗右缘 {} 超出屏幕 {}",
            r.right,
            screen_w
        );
        ui.hide();
        ui.destroy();
    }

    /// logical_screen_dim：物理像素 → 逻辑像素换算（SM_CXSCREEN/SM_CYSCREEN
    /// 返回物理值，候选窗布局用逻辑值；150% 缩放下必须把 2560 → 1706）。
    #[test]
    fn logical_screen_dim_converts_physical_to_logical() {
        // 96 DPI（100% 缩放）：物理 == 逻辑，换算恒等
        assert_eq!(super::logical_screen_dim(1920, 96), 1920);
        assert_eq!(super::logical_screen_dim(1080, 96), 1080);
        // 144 DPI（150% 缩放）：2560 物理 = 1706 逻辑
        assert_eq!(super::logical_screen_dim(2560, 144), 1706);
        assert_eq!(super::logical_screen_dim(1600, 144), 1066);
        // 192 DPI（200% 缩放）：3840 物理 = 1920 逻辑
        assert_eq!(super::logical_screen_dim(3840, 192), 1920);
        // 极小值兜底：不允许归零（后续 max(BASE_MIN_WIDTH) 依赖正数）
        assert_eq!(super::logical_screen_dim(0, 144), 1);
    }

    /// 内容指纹（P0 #5）：相同内容 → 相同指纹；任一可见变化 → 不同指纹。
    /// 这是"内容未变跳过字体实测与重绘"的前提，误判会导致漏刷新。
    #[test]
    fn content_fingerprint_distinguishes_visible_changes() {
        use ime_ipc::Candidate;
        let ctx = || ime_ipc::Context {
            preedit: "nihao".into(),
            candidates: vec![
                Candidate {
                    text: "你好".into(),
                    comment: "nǐ hǎo".into(),
                },
                Candidate {
                    text: "妮豪".into(),
                    comment: String::new(),
                },
            ],
            highlighted: 0,
            page_no: 0,
            is_last_page: false,
            ..ime_ipc::Context::default()
        };
        let skin = super::Skin::default();
        let fp = super::CandidateUi::content_fingerprint(&ctx(), &skin, 96);
        // 相同内容（皮肤/DPI 同）→ 指纹相同
        assert_eq!(
            super::CandidateUi::content_fingerprint(&ctx(), &skin, 96),
            fp
        );
        // 候选文本变化 → 指纹变化
        let mut changed = ctx();
        changed.candidates[0].text = "您好".into();
        assert_ne!(
            super::CandidateUi::content_fingerprint(&changed, &skin, 96),
            fp
        );
        // 高亮序号变化 → 指纹变化（高亮行渲染不同）
        let mut changed = ctx();
        changed.highlighted = 1;
        assert_ne!(
            super::CandidateUi::content_fingerprint(&changed, &skin, 96),
            fp
        );
        // preedit 变化 → 指纹变化
        let mut changed = ctx();
        changed.preedit = "ninhao".into();
        assert_ne!(
            super::CandidateUi::content_fingerprint(&changed, &skin, 96),
            fp
        );
        // 页码变化 → 指纹变化（滚动条/候选集不同）
        let mut changed = ctx();
        changed.page_no = 1;
        assert_ne!(
            super::CandidateUi::content_fingerprint(&changed, &skin, 96),
            fp
        );
        // 中英模式变化 → 指纹变化（模式角标不同）
        let mut changed = ctx();
        changed.is_ascii = true;
        assert_ne!(
            super::CandidateUi::content_fingerprint(&changed, &skin, 96),
            fp
        );
        // DPI 变化 → 指纹变化（布局尺寸不同）
        assert_ne!(
            super::CandidateUi::content_fingerprint(&ctx(), &skin, 144),
            fp
        );
        // 皮肤字号倍率变化 → 指纹变化（字体/间距不同）
        let mut skin2 = skin;
        skin2.metrics.font_scale = 1.2;
        assert_ne!(
            super::CandidateUi::content_fingerprint(&ctx(), &skin2, 96),
            fp
        );
    }

    /// 候选窗位置策略解析（P1 #10）：合法值映射到对应模式，未知值回退跟随。
    #[test]
    fn position_mode_parses_options() {
        use super::PositionMode;
        assert_eq!(PositionMode::from_option("follow"), PositionMode::Follow);
        assert_eq!(
            PositionMode::from_option("bottom_right"),
            PositionMode::FixedBottomRight
        );
        assert_eq!(
            PositionMode::from_option("bottom_left"),
            PositionMode::FixedBottomLeft
        );
        // 未知/空值安全回退跟随（不 panic、不产生无效模式）
        assert_eq!(PositionMode::from_option(""), PositionMode::Follow);
        assert_eq!(PositionMode::from_option("top_left"), PositionMode::Follow);
        assert_eq!(PositionMode::from_option("垃圾值"), PositionMode::Follow);
    }

    /// 候选面板模式解析（M7）："multi" 映射多行，未知/空值回退单行。
    #[test]
    fn candidate_panel_mode_parses_options() {
        use super::CandidatePanelMode;
        assert_eq!(
            CandidatePanelMode::from_option("single"),
            CandidatePanelMode::Single
        );
        assert_eq!(
            CandidatePanelMode::from_option("multi"),
            CandidatePanelMode::Multi
        );
        assert_eq!(
            CandidatePanelMode::from_option(""),
            CandidatePanelMode::Single
        );
        assert_eq!(
            CandidatePanelMode::from_option("grid"),
            CandidatePanelMode::Single
        );
    }

    /// 候选条右键菜单命令映射（M7）：五个命令全部落到对应动作，未知回退 None。
    #[test]
    fn candidate_menu_action_mapping() {
        use super::{
            menu_action_for, CandidateMenuAction, IDM_COPY, IDM_DEMOTE_CAND, IDM_DROP_CAND,
            IDM_HIDE_CAND, IDM_OPEN_SETTINGS,
        };
        assert_eq!(menu_action_for(IDM_COPY), Some(CandidateMenuAction::Copy));
        assert_eq!(
            menu_action_for(IDM_DROP_CAND),
            Some(CandidateMenuAction::DropCandidate)
        );
        assert_eq!(
            menu_action_for(IDM_DEMOTE_CAND),
            Some(CandidateMenuAction::DemoteCandidate)
        );
        assert_eq!(
            menu_action_for(IDM_HIDE_CAND),
            Some(CandidateMenuAction::HideCandidate)
        );
        assert_eq!(
            menu_action_for(IDM_OPEN_SETTINGS),
            Some(CandidateMenuAction::OpenSettings)
        );
        assert_eq!(menu_action_for(999), None);
        assert_eq!(menu_action_for(0), None);
    }

    /// 复制候选走系统剪贴板（CF_UNICODETEXT 往返一致）。
    /// 系统剪贴板与本机常驻的剪贴板同步服务（shurufa-host）共享，
    /// 并发窗口期存在瞬态失败（实测 ERROR_CLIPBOARD_NOT_OPEN）：
    /// 整轮"写入→读取"重试 3 次吸收抖动，仍失败才算错。
    #[test]
    fn clipboard_copy_roundtrip() {
        use super::{copy_candidate_text, CloseClipboard, OpenClipboard, CF_UNICODETEXT};
        use windows::Win32::Foundation::HGLOBAL;
        use windows::Win32::System::DataExchange::GetClipboardData;
        use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
        let text = "测试ABC123";
        // 宿主剪贴板同步服务（shurufa-host）以 WM_CLIPBOARDUPDATE 事件驱动读取：
        // Set 后立刻重开读回会与它的读取窗口争用（实测 ERROR_CLIPBOARD_NOT_OPEN）。
        // 每次 Set 后等 150ms 让宿主读完再读回；最多 3 轮，仍失败才算错。
        let mut last_error = String::new();
        for attempt in 1..=3 {
            copy_candidate_text(text);
            std::thread::sleep(std::time::Duration::from_millis(150));
            unsafe {
                if OpenClipboard(None).is_err() {
                    last_error = "OpenClipboard 失败（宿主占用）".to_owned();
                    continue;
                }
                let read_back = match GetClipboardData(CF_UNICODETEXT) {
                    Ok(h) => {
                        let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
                        let mut out = String::new();
                        if !p.is_null() {
                            let mut i = 0usize;
                            loop {
                                let u = *p.add(i);
                                if u == 0 {
                                    break;
                                }
                                out.push(char::from_u32(u as u32).unwrap_or('\u{FFFD}'));
                                i += 1;
                            }
                        }
                        let _ = GlobalUnlock(HGLOBAL(h.0));
                        Some(out)
                    }
                    Err(e) => {
                        last_error = format!("{e:?}");
                        None
                    }
                };
                let _ = CloseClipboard();
                if let Some(out) = read_back {
                    assert_eq!(out, text, "剪贴板往返内容不一致（第 {attempt} 次）");
                    return;
                }
            }
        }
        panic!("剪贴板往返失败：3 次重试仍与宿主同步服务争用（{last_error}）");
    }

    /// 多行候选面板（M7，搜狗 16.3b 同类）：9 候选 / 5 列 → 2 行；
    /// 窗口更高、命中测试按行映射、切回单行高度回落。
    #[test]
    fn candidate_window_multi_panel_two_rows_layout_and_hit_test() {
        use super::CandidatePanelMode;
        use ime_ipc::Candidate;
        use windows::Win32::Foundation::{POINT, RECT};
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let mut ui = super::CandidateUi::new();
        let ctx = ime_ipc::Context {
            preedit: "duohang".into(),
            candidates: (0..9)
                .map(|i| Candidate {
                    text: format!("候选词{}", i),
                    comment: String::new(),
                })
                .collect(),
            ..ime_ipc::Context::default()
        };
        let anchor = Some(POINT { x: 200, y: 200 });
        ui.show(
            &ctx,
            anchor,
            super::PositionMode::Follow,
            CandidatePanelMode::Multi,
        );
        let hwnd = ui.hwnd.expect("候选窗应创建成功");
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut rc);
        };
        let multi_h = rc.bottom - rc.top;

        // 行分布：前 5 个 row=0，后 4 个 row=1（MULTI_COLUMNS=5）
        let rows: Vec<i32> =
            super::PAINT_DATA.with_borrow(|d| d.items.iter().map(|it| it.row).collect());
        assert_eq!(rows.len(), 9);
        assert!(rows[..5].iter().all(|&r| r == 0), "前 5 个候选应在第 1 行");
        assert!(rows[5..].iter().all(|&r| r == 1), "后 4 个候选应在第 2 行");

        // 第二行命中测试：取第 6 个候选（row=1）内部一点
        let (x6, y6) = super::PAINT_DATA.with_borrow(|d| {
            let dpi = d.dpi;
            let m = d.skin.metrics;
            let row_top = super::scale(m.padding_or(super::BASE_PADDING), dpi)
                + super::scale(m.preedit_h_or(super::BASE_PREEDIT_HEIGHT), dpi);
            let row_h = super::scale(m.row_h_or(super::BASE_ROW_HEIGHT), dpi);
            (d.items[5].x + 4, row_top + row_h + 4)
        });
        assert_eq!(super::hit_test_item(x6, y6), Some(5));

        // 同内容单行模式：高度应显著更矮（多行 = 2 行）
        ui.show(
            &ctx,
            anchor,
            super::PositionMode::Follow,
            CandidatePanelMode::Single,
        );
        let mut rc2 = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut rc2);
        };
        assert!(
            rc2.bottom - rc2.top < multi_h,
            "单行高度 {} 应小于多行高度 {}",
            rc2.bottom - rc2.top,
            multi_h
        );
        ui.hide();
        ui.destroy();
    }

    /// 候选来源启发式分类（P2 #14）：emoji > 英文 > 特殊格式 > 单字 > 词。
    #[test]
    fn candidate_source_classification() {
        // 英文（纯 ASCII 无数字分隔符）
        assert_eq!(classify_candidate_source("hello"), CandidateSource::English);
        assert_eq!(
            classify_candidate_source("photoshop"),
            CandidateSource::English
        );
        // 含非 BMP 字符 → emoji（😊 U+1F60A）
        assert_eq!(classify_candidate_source("😊"), CandidateSource::Emoji);
        assert_eq!(classify_candidate_source("微笑😊"), CandidateSource::Emoji);
        // 特殊格式：日期/时间/算式（ASCII 数字 + 分隔符）
        assert_eq!(
            classify_candidate_source("2026-08-17"),
            CandidateSource::Special
        );
        assert_eq!(classify_candidate_source("14:30"), CandidateSource::Special);
        assert_eq!(classify_candidate_source("1+1=2"), CandidateSource::Special);
        // 中文日期/金额（CJK + 数字/年月日/元整）
        assert_eq!(
            classify_candidate_source("二〇二六年八月十七日"),
            CandidateSource::Special
        );
        assert_eq!(
            classify_candidate_source("壹佰贰拾叁元整"),
            CandidateSource::Special
        );
        // 单字 vs 词
        assert_eq!(classify_candidate_source("你"), CandidateSource::SingleChar);
        assert_eq!(classify_candidate_source("你好"), CandidateSource::Word);
        assert_eq!(classify_candidate_source("阿尔法"), CandidateSource::Word);
        // 空文本兜底为词
        assert_eq!(classify_candidate_source(""), CandidateSource::Word);
    }

    /// 候选来源角标文案映射。
    #[test]
    fn candidate_source_labels() {
        assert_eq!(candidate_source_label(CandidateSource::English), "EN");
        assert_eq!(candidate_source_label(CandidateSource::Emoji), "EMOJI");
        assert_eq!(candidate_source_label(CandidateSource::Special), "◈");
        assert_eq!(candidate_source_label(CandidateSource::SingleChar), "字");
        assert_eq!(candidate_source_label(CandidateSource::Word), "词");
    }

    /// 长候选缩写（weasel candidate_abbreviate_length）：limit<=0 不截断；
    /// 字符数严格超过 limit 时截成 limit-1 字符 + "…"；未超限原样返回。
    #[test]
    fn candidate_abbreviation() {
        // 不截断
        assert_eq!(abbreviate_text("你好", 0), ("你好".to_owned(), false));
        assert_eq!(abbreviate_text("hello", 10), ("hello".to_owned(), false));
        // 刚好等于 limit：不截断
        assert_eq!(
            abbreviate_text("1234567890", 10),
            ("1234567890".to_owned(), false)
        );
        // 超过 limit：截成 limit-1 字符 + …
        assert_eq!(
            abbreviate_text("12345678901", 10),
            ("123456789…".to_owned(), true)
        );
        // CJK 按字符数（不是字节）
        assert_eq!(
            abbreviate_text("这是一段很长的中文候选词条内容", 6),
            ("这是一段很…".to_owned(), true)
        );
        // limit=1 边界：只留 1 字符 + …（keep 下限 1）
        assert_eq!(abbreviate_text("ab", 1), ("a…".to_owned(), true));
    }
}
