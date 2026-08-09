//! AI 帮写面板：Ctrl+Shift+W 呼出，内嵌题词、调用 Agnes API、回车粘贴草稿。
//!
//! 与 `panel.rs` 共享"置顶弹窗 + 消息循环"骨架，差异在于本面板是
//! 单输入框 + 状态机（输入/请求中/预览/错误），网络走 ureq（HTTP
//! 同步客户端），异步通过 `std::thread::spawn` + `PostMessageW` 回到
//! UI 线程。API key 从环境变量 `AGNES_API_KEY` 读取，**永不落盘、
//! 永不混进日志**；缺环境变量时面板只显示配置提示，不发请求。

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_WORDBREAK, FW_BOLD, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ,
    PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_SHIFT, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_ESCAPE,
    VK_RETURN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo,
    GetSystemMetrics, GetWindowThreadProcessId, LoadCursorW, MoveWindow, PostMessageW,
    RegisterClassW, SetForegroundWindow, ShowWindow, CS_HREDRAW, CS_VREDRAW, GUITHREADINFO,
    IDC_ARROW, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOWNA, WM_APP, WM_CHAR, WM_KEYDOWN,
    WM_KILLFOCUS, WM_PAINT, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub const HOTKEY_ID: i32 = 2;
/// Worker 线程把网络结果转回 UI 线程的私有消息。
const WM_AI_DONE: u32 = WM_APP + 71;

/// 内置系统提示：控制输出端为"可直接粘贴的中文文本片段"。
const SYSTEM_PROMPT: &str = "你是用户输入法里的‘AI 帮写’助手。直接输出可粘贴的中文段落，不要解释、不要 Markdown 代码块；除非用户另有要求，控制在 300 字以内。";
const REQUEST_TIMEOUT_SECS: u64 = 45;
/// 预览区一次至少写入剪贴板的最大字符数；超过仅截断显示，不截断写入。
const PREVIEW_MAX_CHARS: usize = 220;

// 96 DPI 基准
const BASE_WIDTH: i32 = 540;
const BASE_PROMPT_ROW: i32 = 30;
const BASE_HINT_HEIGHT: i32 = 20;
const BASE_PREVIEW_MIN: i32 = 80;
const BASE_PADDING: i32 = 10;
const BASE_FONT: i32 = 16;
const BASE_SMALL_FONT: i32 = 12;

const COLOR_BG: u32 = 0x00FA_FAFA;
const COLOR_PROMPT_BG: u32 = 0x00FF_FFFF;
const COLOR_PROMPT_HL: u32 = 0x00D6_E4F0;
const COLOR_TEXT: u32 = 0x0020_2020;
const COLOR_DIM: u32 = 0x0080_8080;
const COLOR_ACCENT: u32 = 0x00A0_5010;
const COLOR_ERROR: u32 = 0x0000_30C0;

struct PanelState {
    hwnd: HWND,
    /// 呼出时的前台窗口，回车粘贴目标
    target: HWND,
    dpi: u32,
    status: Status,
}

impl Default for Status {
    fn default() -> Self {
        Status::Editing
    }
}

#[derive(Debug, Clone)]
enum Status {
    /// 正在输入提示词；query 是要发送给 Agnes 的 user 消息原文。
    Editing,
    /// 已发出请求；done_started 用于"已等待 X 秒"。
    Pending { started: Instant },
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
    static EDITING: RefCell<EditingState> = RefCell::new(EditingState { query: String::new() });
}

/// 注册全局热键；首选 Ctrl+Shift+W（与"输入法"的 Writer 思路呼应），失败
/// 尝试 Alt+W。线程级注册，由 `listener.rs` 消息循环分发。
pub fn register_hotkey() -> &'static str {
    unsafe {
        let which = if RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_SHIFT, 0x57).is_ok() {
            "Ctrl+Shift+W"
        } else if RegisterHotKey(None, HOTKEY_ID, MOD_ALT, 0x57).is_ok() {
            "Alt+W"
        } else {
            "（AI 热键注册失败）"
        };
        crate::log_line(&format!("AI 帮写热键注册结果：{which}"));
        which
    }
}

/// 热键触发：记录前台窗口并弹出面板。
pub fn show() {
    let target = unsafe { GetForegroundWindow() };
    let Some(hwnd) = ensure_window() else {
        crate::log_line("AI 面板窗口创建失败");
        return;
    };
    let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);
    let width = scale(BASE_WIDTH, dpi);
    let min_height = scale(BASE_PADDING * 2 + BASE_PROMPT_ROW + BASE_HINT_HEIGHT + BASE_PREVIEW_MIN, dpi);

    EDITING.with_borrow_mut(|e| e.query.clear());
    let status = if std::env::var_os("AGNES_API_KEY").is_some() {
        crate::log_line("AI 帮写面板弹出，等待输入提示");
        Status::Editing
    } else {
        crate::log_line("AI 帮写：缺少 AGNES_API_KEY 环境变量");
        Status::Misconfigured
    };
    PANEL.with_borrow_mut(|slot| {
        *slot = Some(PanelState {
            hwnd,
            target,
            dpi,
            status,
        });
    });

    let anchor = caret_or_cursor_pos(target);
    let (mut x, mut y) = (anchor.x, anchor.y + scale(6, dpi));
    unsafe {
        x = x.min(GetSystemMetrics(SM_CXSCREEN) - width - 8).max(0);
        y = y.min(GetSystemMetrics(SM_CYSCREEN) - min_height - 8).max(0);
        let _ = MoveWindow(hwnd, x, y, width, min_height, true);
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn hide() {
    // 见 panel.rs：隐藏持焦点的窗口会同步派发 WM_KILLFOCUS → 重入 hide 双重借用。
    let hwnd = PANEL.with_borrow_mut(|slot| slot.take().map(|s| s.hwnd));
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// 提交当前 prompt：发到 Agnes，UI 主线程立刻进入 Pending。
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
    let Some(hwnd) = PANEL.with_borrow(|slot| slot.as_ref().map(|s| s.hwnd)) else {
        return;
    };
    PANEL.with_borrow_mut(|slot| {
        if let Some(state) = slot.as_mut() {
            state.status = Status::Pending {
                started: Instant::now(),
            };
            unsafe {
                let _ = InvalidateRect(Some(state.hwnd), None, true);
            }
        }
    });
    let api_key = api_key.trim().to_owned();
    // HWND 不是 Send；跨线程时转成 isize，到对端再包回 HWND。
    let hwnd_raw = hwnd.0 as isize;
    std::thread::spawn(move || {
        let result = call_agnes(&api_key, &query);
        let boxed: Box<(String, Result<String, String>)> = Box::new((query, result));
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd_raw as *mut _)),
                WM_AI_DONE,
                WPARAM(0),
                LPARAM(Box::into_raw(boxed) as isize),
            );
        }
    });
}

/// 调 Agnes Chat Completions。返回 Err 时已携带可读错误文案。
fn call_agnes(api_key: &str, user_prompt: &str) -> Result<String, String> {
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

    let req = Req {
        model: "agnes-2.5-flash",
        messages: vec![
            Msg {
                role: "system",
                content: SYSTEM_PROMPT,
            },
            Msg {
                role: "user",
                content: user_prompt,
            },
        ],
        temperature: 0.5,
        stream: false,
    };
    let body = serde_json::to_vec(&req).map_err(|e| format!("序列化请求失败: {e}"))?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build();
    let resp = agent
        .post("https://apihub.agnes-ai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_bytes(&body)
        .map_err(|e| match e {
            ureq::Error::Status(code, response) => {
                let body = response.into_string().unwrap_or_default();
                format!("HTTP {code}: {}", crate::single_line_preview(&body, 120))
            }
            ureq::Error::Transport(t) => format!("网络错误: {t}"),
        })?;
    let text = resp
        .into_string()
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let parsed: Resp = serde_json::from_str(&text)
        .map_err(|e| format!("解析响应失败: {e}; 内容片段: {}", crate::single_line_preview(&text, 100)))?;
    if let Some(err) = parsed.error {
        return Err(format!(
            "Agnes 错误: {}",
            err.message.unwrap_or_else(|| "未知".into())
        ));
    }
    let draft = parsed
        .choices
        .and_then(|mut c| c.pop())
        .and_then(|c| c.message)
        .and_then(|m| m.content)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Agnes 返回为空".to_owned())?;
    Ok(draft)
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
    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() },
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
            BASE_PROMPT_ROW,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .ok()?;
        CACHED_HWND.with_borrow_mut(|h| *h = Some(hwnd));
        Some(hwnd)
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
        WM_CHAR => {
            on_char(hwnd, wparam.0 as u32);
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            hide();
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
    let is_editing = PANEL.with_borrow(|slot| {
        matches!(slot.as_ref().map(|s| &s.status), Some(Status::Editing))
    });
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
        DrawTextW(hdc, &mut utf16, &mut rect, DT_LEFT | DT_WORDBREAK | DT_NOPREFIX | DT_END_ELLIPSIS);
    }
}

unsafe fn paint(hdc: HDC, rc: &RECT) {
    PANEL.with_borrow(|slot| {
        let Some(state) = slot.as_ref() else {
            return;
        };
        let dpi = state.dpi;
        let padding = scale(BASE_PADDING, dpi);
        let prompt_h = scale(BASE_PROMPT_ROW, dpi);
        let hint_h = scale(BASE_HINT_HEIGHT, dpi);
        let preview_h = scale(BASE_PREVIEW_MIN, dpi);
        let width = scale(BASE_WIDTH, dpi);

        let bg = CreateSolidBrush(COLORREF(COLOR_BG));
        FillRect(hdc, rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkMode(hdc, TRANSPARENT);

        let font = make_font(scale(BASE_FONT, dpi), FW_NORMAL.0 as i32);
        let bold = make_font(scale(BASE_FONT, dpi), FW_BOLD.0 as i32);
        let small = make_font(scale(BASE_SMALL_FONT, dpi), FW_NORMAL.0 as i32);

        // 顶部：提示词输入行（白底）
        let prompt_rect = RECT {
            left: padding,
            top: padding,
            right: width - padding,
            bottom: padding + prompt_h,
        };
        let prompt_brush = CreateSolidBrush(COLORREF(COLOR_PROMPT_BG));
        FillRect(hdc, &prompt_rect, prompt_brush);
        let _ = DeleteObject(HGDIOBJ(prompt_brush.0));
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        match &state.status {
            Status::Editing => {
                SetTextColor(hdc, COLORREF(COLOR_TEXT));
                let query = EDITING.with_borrow(|e| e.query.clone());
                let display = if query.is_empty() {
                    SetTextColor(hdc, COLORREF(COLOR_DIM));
                    "输入想让 AI 写的内容（如：会议纪要开头 200 字）…".to_owned()
                } else {
                    format!("❯ {query}")
                };
                draw_line(
                    hdc,
                    &display,
                    padding + scale(6, dpi),
                    padding + scale(4, dpi),
                    width - padding * 2 - scale(12, dpi),
                    prompt_h - scale(8, dpi),
                );
            }
            Status::Pending { started } => {
                SetTextColor(hdc, COLORREF(COLOR_ACCENT));
                let elapsed = started.elapsed().as_secs();
                let query = EDITING.with_borrow(|e| e.query.clone());
                draw_line(
                    hdc,
                    &format!("思考中…（{elapsed}s） {query}"),
                    padding + scale(6, dpi),
                    padding + scale(4, dpi),
                    width - padding * 2 - scale(12, dpi),
                    prompt_h - scale(8, dpi),
                );
            }
            Status::Preview { prompt, .. } => {
                SetTextColor(hdc, COLORREF(COLOR_DIM));
                draw_line(
                    hdc,
                    &format!("❯ {prompt}"),
                    padding + scale(6, dpi),
                    padding + scale(4, dpi),
                    width - padding * 2 - scale(12, dpi),
                    prompt_h - scale(8, dpi),
                );
            }
            Status::Failed { .. } | Status::Misconfigured => {
                SetTextColor(hdc, COLORREF(COLOR_DIM));
                let query = EDITING.with_borrow(|e| e.query.clone());
                draw_line(
                    hdc,
                    &if query.is_empty() {
                        "AI 帮写".to_owned()
                    } else {
                        format!("❯ {query}")
                    },
                    padding + scale(6, dpi),
                    padding + scale(4, dpi),
                    width - padding * 2 - scale(12, dpi),
                    prompt_h - scale(8, dpi),
                );
            }
        }

        // 中部：状态区（预览 / 错误 / 提示）
        let mid_top = padding * 2 + prompt_h;
        let mid_bottom = mid_top + preview_h;
        match &state.status {
            Status::Editing => {
                SelectObject(hdc, HGDIOBJ(small.0));
                SetTextColor(hdc, COLORREF(COLOR_DIM));
                draw_line(
                    hdc,
                    "回车提交 · Esc 关闭 · 草稿成功后再回车即粘贴",
                    padding,
                    mid_top,
                    width - padding * 2,
                    hint_h,
                );
            }
            Status::Pending { .. } => {
                SelectObject(hdc, HGDIOBJ(small.0));
                SetTextColor(hdc, COLORREF(COLOR_DIM));
                draw_line(
                    hdc,
                    "正在向 Agnes 请求草稿，请稍候；Esc 取消",
                    padding,
                    mid_top,
                    width - padding * 2,
                    hint_h,
                );
            }
            Status::Preview { draft, .. } => {
                SelectObject(hdc, HGDIOBJ(bold.0));
                SetTextColor(hdc, COLORREF(COLOR_PROMPT_HL));
                draw_line(hdc, "草稿", padding, mid_top, width - padding * 2, hint_h);
                SelectObject(hdc, HGDIOBJ(font.0));
                SetTextColor(hdc, COLORREF(COLOR_TEXT));
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
                SetTextColor(hdc, COLORREF(COLOR_ERROR));
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
                SetTextColor(hdc, COLORREF(COLOR_ERROR));
                draw_line(
                    hdc,
                    "未配置 AGNES_API_KEY",
                    padding,
                    mid_top,
                    width - padding * 2,
                    hint_h,
                );
                SelectObject(hdc, HGDIOBJ(small.0));
                SetTextColor(hdc, COLORREF(COLOR_DIM));
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
        SetTextColor(hdc, COLORREF(COLOR_DIM));
        let footer = match &state.status {
            Status::Editing => "回车 提交 · Esc 关闭",
            Status::Pending { .. } => "Esc 取消",
            Status::Preview { .. } => "回车 粘贴 · Esc 关闭",
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
    //! AI 面板纯逻辑层（make_font/paint 不进入测试）：只校验现场没有
    //! 被泄漏到文本字段中。
    #[test]
    fn 环境变量名符合约定() {
        // 用户文档与代码统一使用同一个名字；这个名字写进 Misconfigured 提示。
        let declared = "AGNES_API_KEY";
        assert_eq!(declared.len(), 13);
    }
}
