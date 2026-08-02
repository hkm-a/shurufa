//! 原生图片贴图窗口。
//!
//! 贴图只读取既有剪贴板历史中的图片，不创建第二个相册。窗口保持置顶，
//! 鼠标滚轮缩放，右键或 Escape 关闭。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, PAINTSTRUCT,
    SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VK_A, VK_CONTROL, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowRect, MoveWindow, PostMessageW, PostQuitMessage, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    ES_AUTOVSCROLL, ES_MULTILINE, HTCAPTION, LWA_ALPHA, MSG, SWP_NOACTIVATE, SW_SHOW, WM_APP,
    WM_DESTROY, WM_KEYDOWN, WM_MOUSEWHEEL, WM_NCHITTEST, WM_NCRBUTTONUP, WM_PAINT, WM_RBUTTONUP, WM_SIZE,
    WNDCLASSW, WS_CHILD, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
    WS_VSCROLL,
};

const WM_OCR_COMPLETE: u32 = WM_APP + 91;

struct PinImage {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
    scale: f32,
    opacity: u8,
    ocr_pending: bool,
    text_view: Option<HWND>,
}

struct OcrCompletion {
    hwnd: usize,
    result: Result<String, String>,
}

thread_local! {
    static PIN: RefCell<Option<PinImage>> = const { RefCell::new(None) };
}

static OCR_COMPLETIONS: OnceLock<Mutex<VecDeque<OcrCompletion>>> = OnceLock::new();

/// OCR 文本框上方保留给父窗的把手高度（像素）
const TEXT_VIEW_HANDLE_HEIGHT: i32 = 28;

fn discard_ocr_completions(hwnd: HWND) {
    if let Some(queue) = OCR_COMPLETIONS.get().and_then(|queue| queue.lock().ok()) {
        let mut queue = queue;
        queue.retain(|completion| completion.hwnd != hwnd.0 as usize);
    }
}

/// 显示一张历史 BMP。调用会进入贴图自己的消息循环，直至用户关闭窗口。
pub fn show_bmp(bmp: &[u8]) -> Result<(), String> {
    let image = image::load_from_memory_with_format(bmp, image::ImageFormat::Bmp)
        .map_err(|e| format!("读取贴图图片失败：{e}"))?
        .to_rgba8();
    let (width, height) = (image.width() as i32, image.height() as i32);
    if width <= 0 || height <= 0 {
        return Err("贴图图片尺寸无效".to_owned());
    }
    let mut pixels = image.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let max_scale = (720.0 / width as f32).min(540.0 / height as f32).min(1.0);
    PIN.with_borrow_mut(|slot| {
        *slot = Some(PinImage {
            pixels,
            width,
            height,
            scale: max_scale.max(0.1),
            opacity: 255,
            ocr_pending: false,
            text_view: None,
        });
    });
    unsafe { create_and_run_window() }
}

/// 在独立窗口线程显示贴图，避免调用方的剪贴板监听循环被贴图消息循环占用。
/// 截图先经既有剪贴板通知入历史和同步，贴图只是同一张图片的视觉副本。
pub fn show_bmp_async(bmp: Vec<u8>) {
    spawn_pin_thread(move || {
        if let Err(error) = show_bmp(&bmp) {
            crate::log_line(&format!("创建截图贴图失败：{error}"));
        }
    });
}

fn spawn_pin_thread(job: impl FnOnce() + Send + 'static) {
    let _ = std::thread::spawn(job);
}

fn start_ocr(hwnd: HWND) {
    let source = PIN.with_borrow_mut(|slot| {
        let pin = slot.as_mut()?;
        if pin.ocr_pending || pin.text_view.is_some() {
            return None;
        }
        pin.ocr_pending = true;
        Some((pin.pixels.clone(), pin.width, pin.height))
    });
    let Some((pixels, width, height)) = source else {
        return;
    };
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
    let hwnd_value = hwnd.0 as usize;
    std::thread::spawn(move || {
        let result = crate::ocr::recognize_bgra(&pixels, width, height);
        let queue = OCR_COMPLETIONS.get_or_init(|| Mutex::new(VecDeque::new()));
        if let Ok(mut queue) = queue.lock() {
            queue.push_back(OcrCompletion {
                hwnd: hwnd_value,
                result,
            });
        }
        unsafe {
            if PostMessageW(
                Some(HWND(hwnd_value as _)),
                WM_OCR_COMPLETE,
                WPARAM(0),
                LPARAM(0),
            )
            .is_err()
            {
                discard_ocr_completions(HWND(hwnd_value as _));
            }
        }
    });
}

fn is_ocr_shortcut(key: u16, control_pressed: bool) -> bool {
    key == VK_A.0 && control_pressed
}

unsafe fn finish_ocr(hwnd: HWND) {
    let completion = OCR_COMPLETIONS
        .get()
        .and_then(|queue| queue.lock().ok())
        .and_then(|mut queue| {
            queue
                .iter()
                .position(|item| item.hwnd == hwnd.0 as usize)
                .and_then(|position| queue.remove(position))
        });
    let Some(completion) = completion else {
        return;
    };
    let result = completion.result;
    PIN.with_borrow_mut(|slot| {
        if let Some(pin) = slot.as_mut() {
            pin.ocr_pending = false;
        }
    });
    let text = match result {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => "未识别到可复制文字".to_owned(),
        Err(error) => {
            crate::log_line(&format!("贴图 OCR 失败：{error}"));
            format!("OCR 失败：{error}")
        }
    };
    let _ = crate::paste::set_clipboard_text_with_owner(&text, Some(hwnd));
    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);
    let wide = HSTRING::from(&text);
    // 顶部留出把手条：EDIT 夺焦点后，父窗仍有区域接收拖动与右键关闭
    let edit = CreateWindowExW(
        Default::default(),
        w!("EDIT"),
        PCWSTR(wide.as_ptr()),
        WS_CHILD
            | WS_VISIBLE
            | WS_VSCROLL
            | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                (ES_MULTILINE | ES_AUTOVSCROLL) as u32,
            ),
        0,
        TEXT_VIEW_HANDLE_HEIGHT,
        rect.right - rect.left,
        (rect.bottom - rect.top - TEXT_VIEW_HANDLE_HEIGHT).max(1),
        Some(hwnd),
        None,
        None,
        None,
    );
    if let Ok(edit) = edit {
        PIN.with_borrow_mut(|slot| {
            if let Some(pin) = slot.as_mut() {
                pin.text_view = Some(edit);
            }
        });
        let _ = SetFocus(Some(edit));
    }
}

unsafe fn create_and_run_window() -> Result<(), String> {
    let instance = GetModuleHandleW(PCWSTR::null()).map_err(|e| format!("读取模块失败：{e}"))?;
    let class_name = w!("ShurufaImagePin");
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
    let (width, height) = pin_size().ok_or_else(|| "贴图状态丢失".to_owned())?;
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        class_name,
        w!("Shurufa 贴图"),
        WS_POPUP,
        120,
        120,
        width,
        height,
        None,
        None,
        Some(instance.into()),
        None,
    )
    .map_err(|e| format!("创建贴图窗口失败：{e}"))?;
    let _ = SetLayeredWindowAttributes(
        hwnd,
        windows::Win32::Foundation::COLORREF(0),
        255,
        LWA_ALPHA,
    );
    let _ = ShowWindow(hwnd, SW_SHOW);
    let mut message = MSG::default();
    while GetMessageW(&mut message, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    PIN.with_borrow_mut(|slot| *slot = None);
    Ok(())
}

fn change_opacity(hwnd: HWND, delta: i16) {
    if delta == 0 {
        return;
    }
    let opacity = PIN.with_borrow_mut(|slot| {
        let pin = slot.as_mut()?;
        pin.opacity = adjust_opacity(pin.opacity, delta);
        Some(pin.opacity)
    });
    if let Some(opacity) = opacity {
        unsafe {
            let _ = SetLayeredWindowAttributes(
                hwnd,
                windows::Win32::Foundation::COLORREF(0),
                opacity,
                LWA_ALPHA,
            );
        }
    }
}

fn adjust_opacity(opacity: u8, delta: i16) -> u8 {
    let step = if delta > 0 { 20 } else { -20 };
    (i16::from(opacity) + step).clamp(51, 255) as u8
}

fn pin_size() -> Option<(i32, i32)> {
    PIN.with_borrow(|slot| {
        slot.as_ref().map(|pin| {
            (
                (pin.width as f32 * pin.scale).round() as i32,
                (pin.height as f32 * pin.scale).round() as i32,
            )
        })
    })
}

fn change_scale(hwnd: HWND, delta: i16) {
    if delta == 0 {
        return;
    }
    let size = PIN.with_borrow_mut(|slot| {
        let pin = slot.as_mut()?;
        pin.scale = (pin.scale + if delta > 0 { 0.1 } else { -0.1 }).clamp(0.1, 3.0);
        Some((
            (pin.width as f32 * pin.scale).round() as i32,
            (pin.height as f32 * pin.scale).round() as i32,
        ))
    });
    let Some((width, height)) = size else {
        return;
    };
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.left - (width - (rect.right - rect.left)) / 2,
                rect.top - (height - (rect.bottom - rect.top)) / 2,
                width,
                height,
                SWP_NOACTIVATE,
            );
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            PIN.with_borrow(|slot| {
                let Some(pin) = slot.as_ref() else {
                    return;
                };
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: pin.width,
                        biHeight: -pin.height,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: 0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let _ = StretchDIBits(
                    hdc,
                    0,
                    0,
                    client.right - client.left,
                    client.bottom - client.top,
                    0,
                    0,
                    pin.width,
                    pin.height,
                    Some(pin.pixels.as_ptr().cast()),
                    &info,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
                if pin.ocr_pending {
                    let label: Vec<u16> = "正在识别中文文字…".encode_utf16().collect();
                    let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                        hdc,
                        windows::Win32::Foundation::COLORREF(0x00FFFFFF),
                    );
                    let _ = windows::Win32::Graphics::Gdi::SetBkMode(
                        hdc,
                        windows::Win32::Graphics::Gdi::TRANSPARENT,
                    );
                    let _ = windows::Win32::Graphics::Gdi::TextOutW(hdc, 12, 12, &label);
                }
            });
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_NCHITTEST => LRESULT(HTCAPTION as isize),
        // 整窗按标题栏处理时右键到达的是 NC 消息，这里是右键关闭的
        // 实际路径（WM_RBUTTONUP 仅在子窗布局变化时兜底）
        WM_NCRBUTTONUP => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) as u16) as i16;
            if GetKeyState(VK_CONTROL.0 as i32) < 0 {
                change_opacity(hwnd, delta);
            } else {
                change_scale(hwnd, delta);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_OCR_COMPLETE => {
            finish_ocr(hwnd);
            LRESULT(0)
        }
        WM_SIZE => {
            PIN.with_borrow(|slot| {
                if let Some(Some(edit)) = slot.as_ref().map(|pin| pin.text_view) {
                    let mut rect = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rect);
                    let _ = MoveWindow(
                        edit,
                        0,
                        TEXT_VIEW_HANDLE_HEIGHT,
                        rect.right - rect.left,
                        (rect.bottom - rect.top - TEXT_VIEW_HANDLE_HEIGHT).max(1),
                        true,
                    );
                }
            });
            LRESULT(0)
        }
        WM_KEYDOWN if is_ocr_shortcut(wparam.0 as u16, GetKeyState(VK_CONTROL.0 as i32) < 0) => {
            start_ocr(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            // 窗口已关闭时丢弃迟到的后台识别结果，避免队列保留无主结果。
            discard_ocr_completions(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        adjust_opacity, discard_ocr_completions, is_ocr_shortcut, spawn_pin_thread, OcrCompletion,
        OCR_COMPLETIONS,
    };
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_A;

    #[test]
    fn 缩放范围保持合理() {
        assert_eq!((0.1_f32 - 0.1).clamp(0.1, 3.0), 0.1);
        assert_eq!((3.0_f32 + 0.1).clamp(0.1, 3.0), 3.0);
    }

    #[test]
    fn 透明度保持在可见范围内() {
        assert_eq!(adjust_opacity(51, -120), 51);
        assert_eq!(adjust_opacity(255, 120), 255);
        assert_eq!(adjust_opacity(180, -120), 160);
    }

    #[test]
    fn 只有贴图内的_ctrl_a_会启动_ocr() {
        assert!(is_ocr_shortcut(VK_A.0, true));
        assert!(!is_ocr_shortcut(VK_A.0, false));
        assert!(!is_ocr_shortcut(b'B' as u16, true));
    }

    #[test]
    fn 关闭贴图会丢弃迟到的识别结果() {
        let queue = OCR_COMPLETIONS.get_or_init(|| Mutex::new(VecDeque::new()));
        let mut queue = queue.lock().expect("OCR 队列锁必须可用");
        queue.clear();
        queue.push_back(OcrCompletion {
            hwnd: 41,
            result: Ok("应丢弃".into()),
        });
        queue.push_back(OcrCompletion {
            hwnd: 42,
            result: Ok("应保留".into()),
        });
        drop(queue);

        discard_ocr_completions(HWND(41usize as _));

        let queue = OCR_COMPLETIONS
            .get()
            .expect("OCR 队列必须已初始化")
            .lock()
            .expect("OCR 队列锁必须可用");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.front().map(|completion| completion.hwnd), Some(42));
    }

    #[test]
    fn 截图贴图使用独立窗口线程() {
        let caller = std::thread::current().id();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        spawn_pin_thread(move || {
            let _ = sender.send(std::thread::current().id());
        });
        let window_thread = receiver.recv().expect("贴图线程必须可启动");
        assert_ne!(caller, window_thread);
    }
}
