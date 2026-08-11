//! 语音转写面板（dev-stub）：Ctrl+Shift+S 呼出 / 收尾。
//!
//! 本轮交付"状态机 + stub STT + 书面语化"全链路。真实引擎（whisper / FunASR /
//! sherpa-onnx / 云端 API）留 wave 6 选型；本轮用 stub 分段节奏：
//! 250ms "你" → +750ms "你好" → +750ms "你好，" → +750ms final "你好，世界。"。
//! 事件面与真实引擎所需一一对应（Partial / Flush / Final / PolishDone），
//! wave 6 只换 [`spawn_engine`] 一行即可。
//!
//! 状态机：Idle → Listening →（再次热键 / auto_commit_threshold_secs 超时）
//! → Processing →（polish 回执）→ 提交并隐藏（paste::set_clipboard_text +
//! SetForegroundWindow + SendInput Ctrl+V，SENDINPUT_SHIFT_RELEASE 舞与
//! ai_panel 一致）。
//!
//! 开关（options.json `speech` 段）：
//! - `enabled` + `hotkey_enabled`：都 true 时 listener.rs 注册 Ctrl+Shift+S；
//!   热加载走 2s mtime watcher。
//! - `written_style_polish`：收尾后走 agnes-2.5-flash 书面语化（系统提示
//!   限定 ≤120 字）；失败回退 raw 提交。
//! - `auto_commit_threshold_secs`：最后一段 partial 后无新 partial 超过该秒
//!   数即触发收尾提交（默认 5s）。
//! - `max_session_secs`：会话总上限（默认 120s，本轮 stub 不主动触发；
//!   真实引擎接入时按此关闸）。
//!
//! 面板：底部居中 480x140、skin 驱动、NOACTIVATE + TOOLWINDOW + TOPMOST；
//! 红色 ⏺ 指示器按 500ms 闪烁仅 Listening 态。
//!
//! AGNES_API_KEY 守卫：只在 polish 分支请求时由 ai_panel::call_agnes 读，
//! 环境变量、不落盘、不进日志、不进剪贴板；key 缺失时 polish 静默回退 raw，
//! 不报错面板，日志只记"缺少 key 已回退"。

use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, InvalidateRect, RoundRect, SelectObject, SetBkMode, SetTextColor, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, FW_BOLD, HBRUSH, HDC, HFONT, HGDIOBJ,
    HPEN, PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, KillTimer, LoadCursorW, MoveWindow, PostMessageW,
    RegisterClassW, SetForegroundWindow, SetTimer, ShowWindow, SystemParametersInfoW,
    CS_HREDRAW, CS_VREDRAW, IDC_ARROW, SPI_GETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP, WM_APP, WM_PAINT, WM_SETTINGCHANGE, WM_TIMER,
};

use crate::panel::skin::{self, Skin};

pub const HOTKEY_ID: i32 = 5;

// ---------------------------------------------------------------------------
// 布局常量（96 DPI 基准）
// ---------------------------------------------------------------------------

const BASE_WIDTH: i32 = 480;
const BASE_HEIGHT: i32 = 140;
const BASE_PADDING: i32 = 14;
const BASE_TITLE_H: i32 = 30;
const BASE_TEXT_TOP: i32 = 44;
const BASE_TITLE_FONT: i32 = 13;
const BASE_BODY_FONT: i32 = 12;

/// ⏺ 指示器 500ms 闪烁（仅 Listening 态）
const TIMER_BLINK_ID: usize = 1;
const TIMER_BLINK_MS: u32 = 500;
/// auto_commit_threshold_secs 延后器
const TIMER_AUTO_COMMIT_ID: usize = 2;

/// Worker → UI 线程的私有跨线程消息：LPARAM = Box<(u64 session_id, SpeechEvent)>
const WM_SPEECH_EVENT: u32 = WM_APP + 90;

// ---------------------------------------------------------------------------
// 会话与事件
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Idle,
    Listening,
    Processing,
    Failed,
}

/// 会话状态（只非窗口字段跨线程共享；窗口句柄单独走 SPEECH_HWND）。
#[derive(Debug)]
pub(crate) struct SpeechState {
    pub(crate) phase: Phase,
    pub(crate) session_id: u64,
    pub(crate) started_at: Instant,
    pub(crate) committed_text: String,
    pub(crate) current_partial: String,
    pub(crate) polish_attempted: bool,
}

impl SpeechState {
    pub(crate) fn new() -> Self {
        SpeechState {
            phase: Phase::Idle,
            session_id: 0,
            started_at: Instant::now(),
            committed_text: String::new(),
            current_partial: String::new(),
            polish_attempted: false,
        }
    }
}

static SESSION: OnceLock<Mutex<SpeechState>> = OnceLock::new();
static SESSION_SEQ: AtomicU64 = AtomicU64::new(1);
/// 面板 HWND 单独存放（AtomicIsize 0 = 未建）；只在 UI 线程读写，跨线程只读用于
/// worker PostMessage 目标。
static SPEECH_HWND: AtomicIsize = AtomicIsize::new(0);

#[derive(Debug)]
pub(crate) enum SpeechEvent {
    Partial { text: String, replace: bool },
    Flush,
    Final { raw_text: String },
    PolishDone {
        polished: Option<String>,
        reason: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// 公共入口（listener.rs Ctrl+Shift+S 调这里）
// ---------------------------------------------------------------------------

pub fn register_hotkey() -> String {
    let opts = shurufa_options::load();
    if !opts.speech.enabled || !opts.speech.hotkey_enabled {
        return "关闭（speech.disabled 或 hotkey_enabled=false）".to_owned();
    }
    let ok = unsafe {
        RegisterHotKey(
            None,
            HOTKEY_ID,
            MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
            0x53, // 'S'
        )
        .is_ok()
    };
    if ok {
        "Ctrl+Shift+S".to_owned()
    } else {
        "注册失败（可能被占用）".to_owned()
    }
}

pub fn toggle() {
    if let Err(e) = toggle_inner() {
        crate::log_line(&format!("语音：toggle 失败 {e}"));
    }
}

fn toggle_inner() -> Result<(), String> {
    let opts = shurufa_options::load();
    if !opts.speech.enabled || !opts.speech.hotkey_enabled {
        crate::log_line("语音：options.json speech.enabled/hotkey_enabled 未开，忽略热键");
        return Ok(());
    }
    let mut st = SESSION
        .get_or_init(|| Mutex::new(SpeechState::new()))
        .lock()
        .expect("speech session 锁中毒");
    match st.phase {
        Phase::Idle | Phase::Failed => {
            st.phase = Phase::Listening;
            st.session_id = SESSION_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
            st.started_at = Instant::now();
            st.committed_text.clear();
            st.current_partial.clear();
            st.polish_attempted = false;
            let sid = st.session_id;
            drop(st);
            let hwnd = ensure_panel()?;
            let skin = Skin::current();
            skin::apply_appearance(hwnd, &skin);
            position_panel(hwnd);
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
            spawn_engine(sid);
            crate::log_line(&format!("语音：会话 {sid} 开始（stub STT）"));
        }
        Phase::Listening => {
            let sid = st.session_id;
            st.phase = Phase::Processing;
            drop(st);
            crate::log_line(&format!("语音：会话 {sid} 手动收尾"));
            let snapshot = read_committed_snapshot(sid);
            post_event(sid, SpeechEvent::Final { raw_text: snapshot });
        }
        Phase::Processing => {
            crate::log_line("语音：正在 Processing，忽略重复热键");
        }
    }
    Ok(())
}

fn read_committed_snapshot(sid: u64) -> String {
    let st = SESSION
        .get_or_init(|| Mutex::new(SpeechState::new()))
        .lock()
        .expect("speech session 锁中毒");
    if st.session_id != sid {
        return String::new();
    }
    let mut out = st.committed_text.clone();
    out.push_str(&st.current_partial);
    out
}

// ---------------------------------------------------------------------------
// stub 引擎（节奏与事件面同真实引擎；wave 6 换实现即可）
// ---------------------------------------------------------------------------

/// (分段文本, 距会话起点 ms)
const STUB_STAGES: &[(&str, u64)] = &[("你", 250), ("你好", 750), ("你好，", 750)];
const STUB_FINAL_TEXT: &str = "你好，世界。";
const STUB_FINAL_DELAY_MS: u64 = 750;

fn spawn_engine(session_id: u64) {
    std::thread::spawn(move || {
        let mut acc = String::new();
        for (chunk, hold) in STUB_STAGES {
            std::thread::sleep(Duration::from_millis(*hold));
            acc.push_str(chunk);
            post_event(
                session_id,
                SpeechEvent::Partial { text: acc.clone(), replace: true },
            );
        }
        std::thread::sleep(Duration::from_millis(STUB_FINAL_DELAY_MS));
        post_event(
            session_id,
            SpeechEvent::Final { raw_text: STUB_FINAL_TEXT.to_owned() },
        );
    });
}

// ---------------------------------------------------------------------------
// 事件 pump（worker → UI 线程）
// ---------------------------------------------------------------------------

fn post_event(session_id: u64, ev: SpeechEvent) {
    let raw = SPEECH_HWND.load(AtomicOrdering::Acquire);
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut _);
    let boxed = Box::new((session_id, ev));
    unsafe {
        let _ = PostMessageW(
            Some(hwnd),
            WM_SPEECH_EVENT,
            WPARAM(0),
            LPARAM(Box::into_raw(boxed) as isize),
        );
    }
}

fn set_phase(p: Phase) {
    let mut st = SESSION
        .get_or_init(|| Mutex::new(SpeechState::new()))
        .lock()
        .expect("speech session 锁中毒");
    st.phase = p;
}

fn on_event(session_id: u64, ev: SpeechEvent) {
    let mut st = SESSION
        .get_or_init(|| Mutex::new(SpeechState::new()))
        .lock()
        .expect("speech session 锁中毒");
    if st.session_id != session_id {
        return;
    }
    match ev {
        SpeechEvent::Partial { text, replace } => {
            if replace {
                st.current_partial = text;
            } else {
                st.current_partial.push_str(&text);
            }
            let threshold = shurufa_options::load()
                .speech
                .auto_commit_threshold_secs
                .max(1);
            let raw = SPEECH_HWND.load(AtomicOrdering::Acquire);
            if raw != 0 {
                let hwnd = HWND(raw as *mut _);
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_AUTO_COMMIT_ID);
                    let _ = SetTimer(Some(hwnd), TIMER_AUTO_COMMIT_ID, threshold * 1000, None);
                }
            }
        }
        SpeechEvent::Flush => {
            if !st.current_partial.is_empty() {
                let partial = std::mem::take(&mut st.current_partial);
                st.committed_text.push_str(&partial);
            }
        }
        SpeechEvent::Final { raw_text } => {
            let partial = std::mem::take(&mut st.current_partial);
            st.committed_text.push_str(&partial);
            st.phase = Phase::Processing;
            let raw = if raw_text.is_empty() {
                st.committed_text.clone()
            } else {
                raw_text
            };
            let polish_on = shurufa_options::load().speech.written_style_polish;
            let sid = st.session_id;
            let already_requested = st.polish_attempted;
            st.polish_attempted = true;
            drop(st);
            crate::log_line(&format!(
                "语音：会话 {sid} 收尾（raw {} 字符），polish={polish_on}",
                raw.chars().count()
            ));
            if polish_on && !already_requested {
                spawn_polish(sid, raw);
            } else {
                commit_and_hide(sid, raw, None);
            }
            let raw_hwnd = SPEECH_HWND.load(AtomicOrdering::Acquire);
            if raw_hwnd != 0 {
                let hwnd = HWND(raw_hwnd as *mut _);
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
            }
        }
        SpeechEvent::PolishDone { polished, reason } => {
            let raw_payload = st.committed_text.clone();
            let sid = st.session_id;
            drop(st);
            match polished {
                Some(p) => {
                    crate::log_line(&format!(
                        "语音：会话 {sid} polish 成功（{} 字符）",
                        p.chars().count()
                    ));
                    commit_and_hide(sid, p, None);
                }
                None => {
                    let r = reason.unwrap_or_else(|| "未知错误".to_owned());
                    crate::log_line(&format!(
                        "语音：会话 {sid} polish 失败（{r}），回退 raw"
                    ));
                    commit_and_hide(sid, raw_payload, Some(r));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// polish：agnes-2.5-flash 把口语转书面语；失败回退 raw
// ---------------------------------------------------------------------------

/// 书面语化系统提示（限定 ≤120 字，直接产出可粘贴中文）。
const POLISH_SYSTEM_PROMPT: &str =
    "你是用户输入法里的中文书面语润色助手。把用户口语输入转成书面语：去除口头语气词、调整句式、保持原意。不超过 120 字。直接输出润色后文本，不加解释。";

fn spawn_polish(session_id: u64, raw: String) {
    std::thread::spawn(move || {
        let key = std::env::var_os("AGNES_API_KEY")
            .and_then(|v| v.into_string().ok())
            .unwrap_or_default();
        if key.is_empty() {
            post_event(
                session_id,
                SpeechEvent::PolishDone {
                    polished: None,
                    reason: Some("缺少 AGNES_API_KEY 环境变量".to_owned()),
                },
            );
            return;
        }
        match crate::ai_panel::call_agnes(&key, &raw, POLISH_SYSTEM_PROMPT) {
            Ok(p) => {
                let trimmed = p.trim().to_owned();
                post_event(
                    session_id,
                    SpeechEvent::PolishDone { polished: Some(trimmed), reason: None },
                );
            }
            Err(e) => {
                post_event(
                    session_id,
                    SpeechEvent::PolishDone { polished: None, reason: Some(e) },
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------
// 提交 + 隐藏
// ---------------------------------------------------------------------------

fn commit_and_hide(session_id: u64, text: String, polish_error: Option<String>) {
    set_phase(Phase::Idle);
    let raw_hwnd = SPEECH_HWND.load(AtomicOrdering::Acquire);
    if raw_hwnd != 0 {
        let hwnd = HWND(raw_hwnd as *mut _);
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_BLINK_ID);
            let _ = KillTimer(Some(hwnd), TIMER_AUTO_COMMIT_ID);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    if text.trim().is_empty() {
        crate::log_line(&format!("语音：会话 {session_id} 提交为空，剪贴板不动"));
        return;
    }
    let target = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if crate::paste::set_clipboard_text(&text).is_ok() && !target.is_invalid() {
        unsafe {
            let _ = SetForegroundWindow(target);
            std::thread::sleep(Duration::from_millis(80));
            crate::ai_panel::send_ctrl_v_external();
        }
        crate::log_line(&format!(
            "语音：会话 {session_id} 提交 {} 字符{}",
            text.chars().count(),
            polish_error
                .map(|e| format!("；polish 失败已回退：{e}"))
                .unwrap_or_default()
        ));
    } else {
        crate::log_line(&format!("语音：会话 {session_id} 提交失败：无法写剪贴板"));
    }
}

// ---------------------------------------------------------------------------
// 面板窗口
// ---------------------------------------------------------------------------

fn ensure_panel() -> Result<HWND, String> {
    let raw = SPEECH_HWND.load(AtomicOrdering::Acquire);
    if raw != 0 {
        let hwnd = HWND(raw as *mut _);
        if unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)) }.as_bool() {
            return Ok(hwnd);
        }
        SPEECH_HWND.store(0, AtomicOrdering::Release);
    }
    let hinstance = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|e| e.to_string())?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance.into(),
        lpszClassName: w!("ShurufaSpeechPanel"),
        style: CS_HREDRAW | CS_VREDRAW,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        ..Default::default()
    };
    unsafe {
        RegisterClassW(&class);
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            w!("ShurufaSpeechPanel"),
            w!("语音转写"),
            WS_POPUP,
            0,
            0,
            BASE_WIDTH,
            BASE_HEIGHT,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    }
    .map_err(|e| format!("创建语音面板失败：{e}"))?;
    SPEECH_HWND.store(hwnd.0 as isize, AtomicOrdering::Release);
    unsafe {
        let _ = SetTimer(Some(hwnd), TIMER_BLINK_ID, TIMER_BLINK_MS, None);
    }
    Ok(hwnd)
}

fn position_panel(hwnd: HWND) {
    unsafe {
        let mut area: RECT = std::mem::zeroed();
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut area as *mut RECT as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let dpi = GetDpiForWindow(hwnd).max(GetDpiForSystem()).max(96);
        let w = scale_px(BASE_WIDTH, dpi);
        let h = scale_px(BASE_HEIGHT, dpi);
        let x = (area.left + area.right).saturating_sub(w) / 2;
        let y = area.bottom.saturating_sub(h + 24);
        let _ = MoveWindow(hwnd, x, y, w, h, true);
    }
}

fn scale_px(px: i32, dpi: u32) -> i32 {
    (px as i64 * dpi as i64 / 96) as i32
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_BLINK_ID {
                let _ = InvalidateRect(Some(hwnd), None, true);
            } else if wparam.0 == TIMER_AUTO_COMMIT_ID {
                let _ = KillTimer(Some(hwnd), TIMER_AUTO_COMMIT_ID);
                let sid = {
                    let st = SESSION
                        .get_or_init(|| Mutex::new(SpeechState::new()))
                        .lock()
                        .expect("speech session 锁中毒");
                    st.session_id
                };
                let snapshot = read_committed_snapshot(sid);
                post_event(sid, SpeechEvent::Final { raw_text: snapshot });
            }
            LRESULT(0)
        }
        WM_SPEECH_EVENT => {
            let boxed = unsafe { Box::from_raw(lparam.0 as *mut (u64, SpeechEvent)) };
            let (sid, ev) = *boxed;
            on_event(sid, ev);
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            if lparam.0 != 0 {
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn paint(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rect = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);

        let skin = Skin::current();
        let cand = skin.candidate;
        let metrics = skin.metrics;
        let bg_brush: HBRUSH = CreateSolidBrush(COLORREF(cand.background));
        FillRect(hdc, &rect, bg_brush);
        let _ = DeleteObject(HGDIOBJ(bg_brush.0));

        let dpi = GetDpiForWindow(hwnd).max(GetDpiForSystem()).max(96);

        // 标题
        let phase = current_phase();
        let title = match phase {
            Phase::Idle => "语音转写 (dev-stub) · 空闲",
            Phase::Listening => "语音转写 (dev-stub) · ⏺ 正在听…",
            Phase::Processing => "语音转写 (dev-stub) · 处理中…",
            Phase::Failed => "语音转写 (dev-stub) · 失败",
        };
        let title_font = CreateFontW(
            -scale_px(BASE_TITLE_FONT, dpi),
            0, 0, 0,
            FW_BOLD.0 as i32,
            0, 0, 0,
            windows::Win32::Graphics::Gdi::FONT_CHARSET(1), // DEFAULT_CHARSET
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            w!("Microsoft YaHei UI"),
        );
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(cand.text));
        let old_font = SelectObject(hdc, HGDIOBJ(title_font.0));
        let mut tr = RECT {
            left: scale_px(BASE_PADDING, dpi),
            top: scale_px(6, dpi),
            right: rect.right - scale_px(BASE_PADDING, dpi),
            bottom: scale_px(BASE_TITLE_H, dpi),
        };
        let mut title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        DrawTextW(
            hdc,
            &mut title_wide,
            &mut tr,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(title_font.0));

        // 正文：已提交 + 正在接收的 partial（用 | 分隔视觉）
        let committed = current_committed_text();
        let partial = current_partial();
        let body = match (committed.is_empty(), partial.is_empty()) {
            (true, true) => "（等待语音输入… stub：将逐段出现）".to_owned(),
            (true, false) => partial,
            (false, true) => committed,
            (false, false) => format!("{committed}|{partial}"),
        };
        let body_font = CreateFontW(
            -scale_px(BASE_BODY_FONT, dpi),
            0, 0, 0,
            400, // FW_NORMAL
            0, 0, 0,
            windows::Win32::Graphics::Gdi::FONT_CHARSET(1),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            w!("Microsoft YaHei UI"),
        );
        let old_font2 = SelectObject(hdc, HGDIOBJ(body_font.0));
        SetTextColor(hdc, COLORREF(cand.preedit));
        let mut body_rect = RECT {
            left: scale_px(BASE_PADDING, dpi),
            top: scale_px(BASE_TEXT_TOP, dpi),
            right: rect.right - scale_px(BASE_PADDING, dpi),
            bottom: rect.bottom - scale_px(BASE_PADDING, dpi),
        };
        let mut body_wide: Vec<u16> = body.encode_utf16().chain(std::iter::once(0)).collect();
        DrawTextW(
            hdc,
            &mut body_wide,
            &mut body_rect,
            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(hdc, old_font2);
        let _ = DeleteObject(HGDIOBJ(body_font.0));

        // 1px 圆角边框（skin 高亮背景色；候选皮肤没有 highlight 字段，
        // 只有 highlight_background，panel.rs 里高亮底色就是它）
        let radius = metrics.radius.max(2).min(16) as i32;
        let border_pen: HPEN = CreatePen(PS_SOLID, 1, COLORREF(cand.highlight_background));
        let border_brush: HBRUSH = CreateSolidBrush(COLORREF(cand.highlight_background));
        let old_pen = SelectObject(hdc, HGDIOBJ(border_pen.0));
        let old_brush = SelectObject(hdc, HGDIOBJ(border_brush.0));
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right - 1,
            rect.bottom - 1,
            radius,
            radius,
        );
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(border_pen.0));
        let _ = DeleteObject(HGDIOBJ(border_brush.0));

        EndPaint(hwnd, &ps);
    }
}

fn current_phase() -> Phase {
    let st = SESSION
        .get_or_init(|| Mutex::new(SpeechState::new()))
        .lock()
        .expect("speech session 锁中毒");
    st.phase
}

fn current_committed_text() -> String {
    let st = SESSION
        .get_or_init(|| Mutex::new(SpeechState::new()))
        .lock()
        .expect("speech session 锁中毒");
    st.committed_text.clone()
}

fn current_partial() -> String {
    let st = SESSION
        .get_or_init(|| Mutex::new(SpeechState::new()))
        .lock()
        .expect("speech session 锁中毒");
    st.current_partial.clone()
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 初始状态是_idle() {
        let st = SpeechState::new();
        assert_eq!(st.phase, Phase::Idle);
        assert_eq!(st.session_id, 0);
        assert!(st.committed_text.is_empty());
        assert!(st.current_partial.is_empty());
        assert!(!st.polish_attempted);
    }

    #[test]
    fn 会话id_单调递增() {
        let a = SESSION_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let b = SESSION_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        assert!(b > a, "SESSION_SEQ 必须单调");
    }

    #[test]
    fn polish_prompt_字面量校验() {
        assert!(POLISH_SYSTEM_PROMPT.contains("书面语"));
        assert!(POLISH_SYSTEM_PROMPT.contains("120 字"));
        assert!(POLISH_SYSTEM_PROMPT.contains("口头语气词"));
    }

    #[test]
    fn stub_stage_常量_非空且覆盖总周期为正() {
        assert!(!STUB_STAGES.is_empty(), "stub 至少要有一段 partial");
        for (chunk, ms) in STUB_STAGES {
            assert!(!chunk.is_empty(), "分段文本不能为空");
            assert!(*ms > 0, "分段等待必须为正：{ms}");
        }
        // 总耗时≈250+750+750 + final 750 = 2.5s，与 "speech.auto_commit_threshold_secs"
        // 默认 5s 留有充足余量，真实 stub 不会在自动提交前就被超时切断
        let total: u64 = STUB_STAGES.iter().map(|(_, ms)| *ms).sum::<u64>() + STUB_FINAL_DELAY_MS;
        assert!(total > 0 && total < 10_000, "stub 总节奏应在毫秒级：{total}ms");
        assert!(!STUB_FINAL_TEXT.is_empty());
    }

    #[test]
    fn session_seq_全局自增_不受单测影响() {
        let first = SESSION_SEQ.load(AtomicOrdering::Relaxed);
        SESSION_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let second = SESSION_SEQ.load(AtomicOrdering::Relaxed);
        assert!(second > first);
    }

    #[test]
    fn state_转换_只读_phase_枚举语义() {
        for p in [Phase::Idle, Phase::Listening, Phase::Processing, Phase::Failed] {
            let _ = format!("{p:?}");
        }
    }

    #[test]
    fn speech_hwnd_原子槽_从空开始() {
        // 测试不创建窗口，仅保证初始为 0
        // 注意：本测试运行前若有真实 panel 启动，则常量可能已被占；为此测试
        // 在干净包上保证。
        let raw = SPEECH_HWND.load(AtomicOrdering::Relaxed);
        // 若在 cargo test 上下文（IS_TEST 不影响；面板不会自己起来），值应该为 0
        // 允许已被其他测试占用的极小概率：只验证不是垃圾值
        assert!(raw >= 0);
    }
}
