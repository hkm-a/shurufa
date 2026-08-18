//! 轻量状态提示（toast）：模式切换 / 操作反馈等一次性提示，短暂显示后自动消失。
//!
//! 设计（成熟输入法方向 show_notifications，2026-08-18，微信/搜狗模式提示同类）：
//! - 独立小窗（WS_POPUP + TOPMOST | NOACTIVATE | TOOLWINDOW），不抢焦点、不进任务栏；
//! - 点击穿透（WM_NCHITTEST → HTTRANSPARENT），悬浮在光标附近也不挡鼠标；
//! - 显示 2 秒后自动隐藏（WM_TIMER），重复 show 从新计时；
//! - 外观沿用皮肤：背景 + 文字色 + DWM 圆角（`skin::apply_appearance`），与候选窗一致；
//! - 位置：优先在输入锚点（候选窗同款锚点）上方；无锚点时落在主屏底部居中；
//! - 线程本地状态，只在 TSF UI 线程调用（与候选窗同一线程模型）。

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetDC,
    GetTextExtentPoint32W, ReleaseDC, SelectObject, SetBkMode, SetTextColor, DT_CENTER,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ, PAINTSTRUCT,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, KillTimer, LoadCursorW,
    MoveWindow, RegisterClassW, SetTimer, ShowWindow, CS_HREDRAW, CS_VREDRAW, HTTRANSPARENT,
    IDC_ARROW, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, WM_DESTROY, WM_NCHITTEST,
    WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::candidate_window::{font_height, logical_screen_dim, scale};
use crate::skin::{self, Skin};

const TOAST_CLASS: PCWSTR = w!("ShurufaToast");
/// WM_TIMER 事件 id（wparam 比对用；usize 与 WPARAM 对齐）。
const TOAST_TIMER_ID: usize = 1;
/// 提示持续时长：2 秒（微信/搜狗同量级；太短读不完，太长挡视线）。
const TOAST_DURATION_MS: u32 = 2000;
/// toast 字号（基准 px，随 DPI 缩放；与候选窗副标同档）。
const BASE_FONT_HEIGHT: i32 = 18;
/// 文本两侧留白（基准 px，随 DPI 缩放）。
const BASE_PADDING: i32 = 12;
/// 最小宽度（基准 px），极短文本（如"全角"）也不至于窄成一条线。
const BASE_MIN_WIDTH: i32 = 64;
/// 无锚点时的底部边距（基准 px）。
const BASE_BOTTOM_MARGIN: i32 = 48;

thread_local! {
    static CLASS_REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TOAST_HWND: std::cell::Cell<HWND> = const {
        std::cell::Cell::new(HWND(std::ptr::null_mut()))
    };
    /// 最近一次 show 的文本（WM_PAINT 用；窗口显示期间文本不变）。
    static TOAST_TEXT: RefCell<String> = const { RefCell::new(String::new()) };
    /// 最近一次 show 应用的皮肤（WM_PAINT 用；None = 尚未 show 过，回退 Skin::current）。
    static TOAST_SKIN: RefCell<Option<Skin>> = const { RefCell::new(None) };
}

/// 显示一条 toast 提示。`anchor` 为输入锚点（组合文本左下角，与候选窗同源）；
/// None 时落在主屏底部居中。同窗口复用：新文本重测宽度并重新计时。
pub fn show(text: &str, anchor: Option<POINT>) {
    let Some(hwnd) = ensure_window() else {
        return;
    };
    unsafe {
        // 每次弹出重读皮肤（与候选窗一致：文件/主题改动即时生效）
        let extra = crate::dll_path()
            .parent()
            .map(|dir| dir.join("schemas").join("shurufa-skin.json"));
        let skin = skin::load_with(|| extra);
        skin::apply_appearance(hwnd, &skin);
        TOAST_SKIN.with(|s| *s.borrow_mut() = Some(skin));
        TOAST_TEXT.with(|t| *t.borrow_mut() = text.to_string());

        // 部分宿主（DPI 虚拟化）对弹窗返回 96 兜底值，取系统 DPI 兜底
        let dpi = GetDpiForWindow(hwnd).max(GetDpiForSystem()).max(96);
        let screen_w = logical_screen_dim(GetSystemMetrics(SM_CXSCREEN), dpi);
        let screen_h = logical_screen_dim(GetSystemMetrics(SM_CYSCREEN), dpi);
        let font_h = font_height(BASE_FONT_HEIGHT, dpi, skin.metrics.font_scale);
        let padding = scale(BASE_PADDING, dpi);

        // 用与绘制一致的字体实测文本宽度，确定窗口尺寸
        let hdc = GetDC(Some(hwnd));
        let font = make_font(font_h);
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        if !utf16.is_empty() {
            let _ = GetTextExtentPoint32W(hdc, &utf16, &mut size);
        }
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = ReleaseDC(Some(hwnd), hdc);

        let width = (size.cx + padding * 2).max(scale(BASE_MIN_WIDTH, dpi));
        let height = font_h + padding * 2;

        let (x, y) = toast_position(anchor, width, height, screen_w, screen_h);
        let _ = MoveWindow(hwnd, x, y, width, height, true);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        // 重复 show：重置计时器（同窗口复用，不叠加隐藏）
        let _ = KillTimer(Some(hwnd), TOAST_TIMER_ID);
        let _ = SetTimer(Some(hwnd), TOAST_TIMER_ID, TOAST_DURATION_MS, None);
        crate::debug_log(&format!("toast: {text} @({x},{y}) {width}x{height}"));
    }
}

/// 纯函数：toast 窗口位置。有锚点时放在锚点上方（gap 6px），
/// 无锚点落在主屏底部居中；两种都 clamp 到屏幕内（至少露 8px 边距）。
fn toast_position(
    anchor: Option<POINT>,
    width: i32,
    height: i32,
    screen_w: i32,
    screen_h: i32,
) -> (i32, i32) {
    match anchor {
        Some(p) => {
            let x = p.x.min(screen_w - width - 8).max(0);
            let y = (p.y - height - 6).min(screen_h - height - 8).max(0);
            (x, y)
        }
        None => {
            let x = ((screen_w - width) / 2).max(0);
            let y = (screen_h - height - BASE_BOTTOM_MARGIN).max(0);
            (x, y)
        }
    }
}

/// 立即隐藏（重复调用幂等；计时器未到也会隐藏）。
pub fn hide() {
    let hwnd = TOAST_HWND.with(|h| h.get());
    if hwnd.0.is_null() {
        return;
    }
    unsafe {
        let _ = KillTimer(Some(hwnd), TOAST_TIMER_ID);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// 销毁窗口（Deactivate / 进程收尾时调用；幂等）。
pub fn destroy() {
    let hwnd = TOAST_HWND.with(|h| h.replace(HWND(std::ptr::null_mut())));
    if !hwnd.0.is_null() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn ensure_window() -> Option<HWND> {
    let existing = TOAST_HWND.with(|h| h.get());
    if !existing.0.is_null() {
        return Some(existing);
    }
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
        CLASS_REGISTERED.with(|registered| {
            if !registered.get() {
                let class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: TOAST_CLASS,
                    hbrBackground: HBRUSH::default(),
                    // 不设光标会导致悬停时一直显示忙碌转圈
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    ..Default::default()
                };
                // 同进程重复注册返回 0，忽略即可
                RegisterClassW(&class);
                registered.set(true);
            }
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            TOAST_CLASS,
            w!(""),
            WS_POPUP,
            0,
            0,
            scale(BASE_MIN_WIDTH, 96),
            scale(30, 96),
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .ok()?;
        TOAST_HWND.with(|h| h.set(hwnd));
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
            let mut rc = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);
            let skin = TOAST_SKIN.with(|s| s.borrow().unwrap_or_else(Skin::current));
            let text = TOAST_TEXT.with(|t| t.borrow().clone());
            paint(hdc, &rc, &skin, &text);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TOAST_TIMER_ID => {
            let _ = KillTimer(Some(hwnd), TOAST_TIMER_ID);
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        // 点击穿透：toast 不拦截任何鼠标事件（悬浮在光标附近也不挡操作）
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_DESTROY => DefWindowProcW(hwnd, msg, wparam, lparam),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint(hdc: HDC, rc: &RECT, skin: &Skin, text: &str) {
    // 皮肤背景铺底（DWM 圆角由 apply_appearance 在 show 时设定）
    let bg = CreateSolidBrush(COLORREF(skin.candidate.background));
    FillRect(hdc, rc, bg);
    let _ = DeleteObject(HGDIOBJ(bg.0));
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, COLORREF(skin.candidate.text));

    let dpi = TOAST_HWND.with(|h| unsafe { GetDpiForWindow(h.get()) }.max(96));
    let font = make_font(font_height(BASE_FONT_HEIGHT, dpi, skin.metrics.font_scale));
    let old = SelectObject(hdc, HGDIOBJ(font.0));
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    if !utf16.is_empty() {
        let mut text_rc = *rc;
        let _ = DrawTextW(
            hdc,
            &mut utf16,
            &mut text_rc,
            DT_CENTER | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER,
        );
    }
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(font.0));
}

/// 创建 toast 字体（负高度 = 字符高度 em，排版稳定；用完即删，toast 低频无需缓存）。
unsafe fn make_font(height: i32) -> HFONT {
    CreateFontW(
        -height,
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_without_anchor_bottom_center() {
        // 800x600 屏幕、200x40 窗口 → 底部居中 (300, 512)
        assert_eq!(
            toast_position(None, 200, 40, 800, 600),
            ((800 - 200) / 2, 600 - 40 - BASE_BOTTOM_MARGIN)
        );
    }

    #[test]
    fn position_with_anchor_above_caret() {
        // 锚点 (300, 500) → 窗口放在锚点上方 6px
        let (x, y) = toast_position(Some(POINT { x: 300, y: 500 }), 200, 40, 1920, 1080);
        assert_eq!(x, 300);
        assert_eq!(y, 500 - 40 - 6);
    }

    #[test]
    fn position_anchor_clamped_to_screen() {
        // 锚点贴右缘/顶部 → clamp 到可视区内
        let (x, y) = toast_position(Some(POINT { x: 1900, y: 30 }), 200, 40, 1920, 1080);
        assert_eq!(x, 1920 - 200 - 8);
        assert_eq!(y, 0);
    }

    #[test]
    fn position_narrow_screen_keeps_margin() {
        // 极窄屏：窗口比屏幕还宽时 clamp 到 0，不越界
        let (x, y) = toast_position(None, 500, 40, 300, 200);
        assert_eq!(x, 0);
        assert_eq!(y, (200 - 40 - BASE_BOTTOM_MARGIN).max(0));
    }
}
