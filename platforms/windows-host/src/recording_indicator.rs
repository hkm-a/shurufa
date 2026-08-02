//! 屏幕录制期间的轻量状态条。
//!
//! 常驻进程没有可见控制台，热键录制必须有明确反馈。状态条只反映录制状态，
//! 不参与视频存储、剪贴板或历史处理。

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, Ellipse, EndPaint, FillRect, SelectObject,
    SetBkMode, SetTextColor, TextOutW, HGDIOBJ, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassW, SetTimer, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, MSG, SW_SHOW,
    WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

const WIDTH: i32 = 252;
const HEIGHT: i32 = 42;
const TIMER_ID: usize = 0x5352;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);

/// 显示录制状态；重复调用只刷新既有状态条。
pub fn show() {
    ACTIVE.store(true, Ordering::Release);
    if RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let _ = std::thread::spawn(|| {
            if let Err(error) = run_window() {
                crate::log_line(&format!("录制状态条创建失败：{error}"));
            }
            RUNNING.store(false, Ordering::Release);
            // 关闭瞬间若又开始录制，立即重建状态条。
            if ACTIVE.load(Ordering::Acquire) {
                show();
            }
        });
    }
}

/// 隐藏录制状态；窗口在线程下一次定时检查时自行销毁。
pub fn hide() {
    ACTIVE.store(false, Ordering::Release);
}

fn run_window() -> Result<(), String> {
    unsafe {
        let instance =
            GetModuleHandleW(PCWSTR::null()).map_err(|error| format!("读取模块失败：{error}"))?;
        let class_name = w!("ShurufaRecordingIndicator");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
        // 不设光标会导致悬停时一直显示忙碌转圈
        hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
            None,
            windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
        )
        .unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name,
            w!("Shurufa 正在录制"),
            WS_POPUP,
            20,
            20,
            WIDTH,
            HEIGHT,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|error| format!("创建录制状态条失败：{error}"))?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetTimer(Some(hwnd), TIMER_ID, 150, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe fn paint(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut paint);
    let rect = RECT {
        left: 0,
        top: 0,
        right: WIDTH,
        bottom: HEIGHT,
    };
    let background = CreateSolidBrush(COLORREF(0x002b2b2b));
    let _ = FillRect(hdc, &rect, background);
    let _ = DeleteObject(HGDIOBJ(background.0));
    let red = CreateSolidBrush(COLORREF(0x003f38dc));
    let previous = SelectObject(hdc, HGDIOBJ(red.0));
    let _ = Ellipse(hdc, 14, 14, 26, 26);
    let _ = SelectObject(hdc, previous);
    let _ = DeleteObject(HGDIOBJ(red.0));
    let label: Vec<u16> = "正在录制屏幕 · 再按热键停止".encode_utf16().collect();
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(0x00f5f5f5));
    let _ = TextOutW(hdc, 38, 13, &label);
    let _ = EndPaint(hwnd, &paint);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_TIMER if _wparam.0 == TIMER_ID && !ACTIVE.load(Ordering::Acquire) => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, _wparam, _lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::{ACTIVE, HEIGHT, WIDTH};
    use std::sync::atomic::Ordering;

    #[test]
    fn 状态条具有紧凑固定尺寸() {
        assert!(WIDTH >= 220);
        assert!(HEIGHT <= 48);
    }

    #[test]
    fn 停止状态会清除可见标记() {
        ACTIVE.store(true, Ordering::Release);
        super::hide();
        assert!(!ACTIVE.load(Ordering::Acquire));
    }
}
