//! 全屏区域选择会话。
//!
//! 实现沿用 Flameshot 的会话边界：先冻结底图，再由选择层产出不可变区域。
//! 本模块不直接写历史库；确认后的图片仍只经系统剪贴板交给监听器入库与同步。

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect,
    Rectangle, SelectObject, SetBkMode, SetTextColor, StretchDIBits, TextOutW, BITMAPINFO,
    BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, MSG,
    SW_SHOW, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::capture::{self, CaptureRect};

struct RegionState {
    bounds: CaptureRect,
    original_bgra: Vec<u8>,
    dimmed_bgra: Vec<u8>,
    start: Option<(i32, i32)>,
    current: Option<(i32, i32)>,
    selected: Option<CaptureRect>,
    drag: Option<SelectionDrag>,
    drag_origin: Option<(i32, i32)>,
    drag_initial: Option<CaptureRect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionDrag {
    Draw,
    Move,
    Resize {
        left: bool,
        top: bool,
        right: bool,
        bottom: bool,
    },
}

thread_local! {
    static STATE: RefCell<Option<RegionState>> = const { RefCell::new(None) };
}

/// 用户主动取消截图时的错误标识；调用方以此与真实失败区分。
pub const CANCELLED: &str = "已取消区域截图";

/// 启动区域选择并把确认结果写入既有剪贴板链路。
pub fn select_region_to_clipboard() -> Result<(i32, i32), String> {
    let bounds = capture::virtual_screen_rect()?;
    let screenshot = capture::capture_bmp(bounds)?;
    let image = image::load_from_memory_with_format(&screenshot, image::ImageFormat::Bmp)
        .map_err(|e| format!("读取选区底图失败：{e}"))?
        .to_rgba8();
    if image.width() != bounds.width as u32 || image.height() != bounds.height as u32 {
        return Err("选区底图尺寸与虚拟桌面不一致".to_owned());
    }
    let mut original_bgra = image.into_raw();
    for pixel in original_bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let mut dimmed_bgra = original_bgra.clone();
    for pixel in dimmed_bgra.chunks_exact_mut(4) {
        pixel[0] = (pixel[0] as u16 * 45 / 100) as u8;
        pixel[1] = (pixel[1] as u16 * 45 / 100) as u8;
        pixel[2] = (pixel[2] as u16 * 45 / 100) as u8;
    }
    STATE.with_borrow_mut(|slot| {
        *slot = Some(RegionState {
            bounds,
            original_bgra,
            dimmed_bgra,
            start: None,
            current: None,
            selected: None,
            drag: None,
            drag_origin: None,
            drag_initial: None,
        });
    });
    let selected_result = unsafe { create_and_run_window() };
    // 选区层展示的是冻结底图；导出必须从同一底图裁剪，避免关闭选区后
    // 屏幕变化或动画帧改变，导致用户看到的区域与实际导出内容不一致。
    let frozen_bmp = match selected_result.as_ref() {
        Ok(Some(selected)) => STATE.with_borrow(|slot| {
            slot.as_ref()
                .ok_or_else(|| "选区底图状态丢失".to_owned())
                .and_then(|state| frozen_selection_bmp(state, *selected))
                .map(Some)
        }),
        _ => Ok(None),
    };
    STATE.with_borrow_mut(|slot| *slot = None);
    let selected = selected_result?;
    let selected = selected.ok_or_else(|| CANCELLED.to_owned())?;
    let bmp = frozen_bmp?.ok_or_else(|| "选区底图状态丢失".to_owned())?;
    let Some(output) = crate::editor::edit_bmp(&bmp)? else {
        return Err(CANCELLED.to_owned());
    };
    let (edited_bmp, pin_after_copy) = match output {
        crate::editor::EditorOutput::Copy(bmp) => (bmp, false),
        crate::editor::EditorOutput::Pin(bmp) => (bmp, true),
        crate::editor::EditorOutput::Save(bmp) => {
            let path = save_bmp_to_pictures(&bmp)?;
            crate::log_line(&format!("截图已保存：{}", path.display()));
            (bmp, false)
        }
        crate::editor::EditorOutput::LongScroll => {
            let report = crate::scroll::capture_region_manually_to_clipboard(selected)?;
            return Ok((report.width as i32, report.height as i32));
        }
    };
    crate::paste::set_clipboard_new_image(&edited_bmp)
        .map_err(|e| format!("写入标注截图剪贴板失败：{e}"))?;
    if pin_after_copy {
        // 贴图窗口有自己的消息循环；必须独立运行，不能阻塞监听线程处理
        // 刚刚写入的剪贴板更新，否则历史与同步会被推迟到贴图关闭后。
        crate::pin::show_bmp_async(edited_bmp.clone());
    }
    Ok((selected.width, selected.height))
}

fn save_bmp_to_pictures(bmp: &[u8]) -> Result<std::path::PathBuf, String> {
    let image = image::load_from_memory_with_format(bmp, image::ImageFormat::Bmp)
        .map_err(|error| format!("读取待保存截图失败：{error}"))?;
    let pictures = std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .map(|home| home.join("Pictures"))
        .ok_or_else(|| "无法定位用户图片目录".to_owned())?;
    let directory = pictures.join("Shurufa");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("创建截图保存目录失败：{error}"))?;
    let milliseconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("读取系统时间失败：{error}"))?
        .as_millis();
    let path = directory.join(screenshot_file_name(milliseconds));
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|error| format!("保存截图失败：{error}"))?;
    Ok(path)
}

fn screenshot_file_name(milliseconds: u128) -> String {
    format!("截图-{milliseconds}.png")
}

fn frozen_selection_bmp(state: &RegionState, selected: CaptureRect) -> Result<Vec<u8>, String> {
    let offset_x = selected.x - state.bounds.x;
    let offset_y = selected.y - state.bounds.y;
    if offset_x < 0
        || offset_y < 0
        || offset_x + selected.width > state.bounds.width
        || offset_y + selected.height > state.bounds.height
    {
        return Err("选区超出冻结底图范围".to_owned());
    }
    let source_width = state.bounds.width as usize;
    let mut rgba = Vec::with_capacity(selected.width as usize * selected.height as usize * 4);
    for y in offset_y as usize..(offset_y + selected.height) as usize {
        for x in offset_x as usize..(offset_x + selected.width) as usize {
            let pixel = &state.original_bgra[(y * source_width + x) * 4..][..4];
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    let image = image::RgbaImage::from_raw(selected.width as u32, selected.height as u32, rgba)
        .ok_or_else(|| "构造冻结选区图片失败".to_owned())?;
    let mut output = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut output, image::ImageFormat::Bmp)
        .map_err(|error| format!("编码冻结选区图片失败：{error}"))?;
    Ok(output.into_inner())
}

unsafe fn create_and_run_window() -> Result<Option<CaptureRect>, String> {
    let instance = GetModuleHandleW(PCWSTR::null()).map_err(|e| format!("读取模块失败：{e}"))?;
    let class_name = w!("ShurufaRegionSelector");
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW | windows::Win32::UI::WindowsAndMessaging::CS_DBLCLKS,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance.into(),
        lpszClassName: class_name,
        // 不设光标会导致悬停时一直显示忙碌转圈
        hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
            None,
            windows::Win32::UI::WindowsAndMessaging::IDC_CROSS,
        )
        .unwrap_or_default(),
        ..Default::default()
    };
    RegisterClassW(&class);
    let bounds = STATE
        .with_borrow(|slot| slot.as_ref().map(|state| state.bounds))
        .ok_or_else(|| "选区状态丢失".to_owned())?;
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class_name,
        w!("Shurufa 区域截图"),
        WS_POPUP,
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
        None,
        None,
        Some(instance.into()),
        None,
    )
    .map_err(|e| format!("创建区域选择窗口失败：{e}"))?;
    let _ = ShowWindow(hwnd, SW_SHOW);
    let mut message = MSG::default();
    while GetMessageW(&mut message, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    Ok(STATE.with_borrow(|slot| slot.as_ref().and_then(|state| state.selected)))
}

fn point_from_lparam(lparam: LPARAM) -> (i32, i32) {
    let raw = lparam.0 as u32;
    (
        (raw as u16 as i16) as i32,
        ((raw >> 16) as u16 as i16) as i32,
    )
}

fn selected_rect(
    bounds: CaptureRect,
    start: (i32, i32),
    current: (i32, i32),
) -> Option<CaptureRect> {
    let left = start.0.min(current.0);
    let top = start.1.min(current.1);
    let width = (start.0 - current.0).unsigned_abs() as i32;
    let height = (start.1 - current.1).unsigned_abs() as i32;
    CaptureRect {
        x: bounds.x + left,
        y: bounds.y + top,
        width,
        height,
    }
    .clamp_to(bounds)
    .filter(|rect| rect.width >= 4 && rect.height >= 4)
}

fn selection_in_client(state: &RegionState) -> Option<(i32, i32, i32, i32)> {
    state
        .selected
        .or_else(|| selected_rect(state.bounds, state.start?, state.current?))
        .map(|rect| {
            (
                rect.x - state.bounds.x,
                rect.y - state.bounds.y,
                rect.width,
                rect.height,
            )
        })
}

fn selection_drag_at(rect: (i32, i32, i32, i32), point: (i32, i32)) -> SelectionDrag {
    const HANDLE: i32 = 8;
    let (left, top, width, height) = rect;
    let right = left + width;
    let bottom = top + height;
    let in_horizontal = (left - HANDLE..=right + HANDLE).contains(&point.0);
    let in_vertical = (top - HANDLE..=bottom + HANDLE).contains(&point.1);
    let is_left = (left - HANDLE..=left + HANDLE).contains(&point.0);
    let is_right = (right - HANDLE..=right + HANDLE).contains(&point.0);
    let is_top = (top - HANDLE..=top + HANDLE).contains(&point.1);
    let is_bottom = (bottom - HANDLE..=bottom + HANDLE).contains(&point.1);
    if in_horizontal && in_vertical && (is_left || is_right || is_top || is_bottom) {
        SelectionDrag::Resize {
            left: is_left,
            top: is_top,
            right: is_right,
            bottom: is_bottom,
        }
    } else if (left..right).contains(&point.0) && (top..bottom).contains(&point.1) {
        SelectionDrag::Move
    } else {
        SelectionDrag::Draw
    }
}

fn move_selection(bounds: CaptureRect, rect: CaptureRect, delta: (i32, i32)) -> CaptureRect {
    let max_x = bounds.x + bounds.width - rect.width;
    let max_y = bounds.y + bounds.height - rect.height;
    CaptureRect {
        x: (rect.x + delta.0).clamp(bounds.x, max_x),
        y: (rect.y + delta.1).clamp(bounds.y, max_y),
        ..rect
    }
}

fn resize_selection(
    bounds: CaptureRect,
    rect: CaptureRect,
    drag: SelectionDrag,
    delta: (i32, i32),
) -> Option<CaptureRect> {
    let SelectionDrag::Resize {
        left,
        top,
        right,
        bottom,
    } = drag
    else {
        return Some(rect);
    };
    let mut x1 = rect.x;
    let mut y1 = rect.y;
    let mut x2 = rect.x + rect.width;
    let mut y2 = rect.y + rect.height;
    if left {
        x1 = (x1 + delta.0).clamp(bounds.x, x2 - 4);
    }
    if right {
        x2 = (x2 + delta.0).clamp(x1 + 4, bounds.x + bounds.width);
    }
    if top {
        y1 = (y1 + delta.1).clamp(bounds.y, y2 - 4);
    }
    if bottom {
        y2 = (y2 + delta.1).clamp(y1 + 4, bounds.y + bounds.height);
    }
    Some(CaptureRect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

unsafe fn paint(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut paint);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    STATE.with_borrow(|slot| {
        let Some(state) = slot.as_ref() else { return };
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: state.bounds.width,
                biHeight: -state.bounds.height,
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
            client.right,
            client.bottom,
            0,
            0,
            state.bounds.width,
            state.bounds.height,
            Some(state.dimmed_bgra.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        if let Some((left, top, width, height)) = selection_in_client(state) {
            let _ = StretchDIBits(
                hdc,
                left,
                top,
                width,
                height,
                left,
                top,
                width,
                height,
                Some(state.original_bgra.as_ptr().cast()),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            let pen = CreatePen(PS_SOLID, 2, windows::Win32::Foundation::COLORREF(0x00D7FF));
            if !pen.is_invalid() {
                let old = SelectObject(hdc, HGDIOBJ(pen.0));
                let _ = Rectangle(hdc, left, top, left + width, top + height);
                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(HGDIOBJ(pen.0));
            }
            let handle = CreatePen(PS_SOLID, 6, windows::Win32::Foundation::COLORREF(0x00D7FF));
            if !handle.is_invalid() {
                let old = SelectObject(hdc, HGDIOBJ(handle.0));
                for (x, y) in [
                    (left, top),
                    (left + width, top),
                    (left, top + height),
                    (left + width, top + height),
                ] {
                    let _ = Rectangle(hdc, x - 2, y - 2, x + 2, y + 2);
                }
                let _ = SelectObject(hdc, old);
                let _ = DeleteObject(HGDIOBJ(handle.0));
            }
            let label_top = if top >= 28 { top - 24 } else { top + 6 };
            let label_rect = RECT {
                left,
                top: label_top,
                right: left + 170,
                bottom: label_top + 20,
            };
            let label_background =
                CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00303030));
            let _ = FillRect(hdc, &label_rect, label_background);
            let _ = DeleteObject(HGDIOBJ(label_background.0));
            let label: Vec<u16> = format!("{} × {}  Enter 确认", width, height)
                .encode_utf16()
                .collect();
            let _ = SetBkMode(hdc, TRANSPARENT);
            let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00F5F5F5));
            let _ = TextOutW(hdc, left + 6, label_top + 4, &label);
        }
    });
    let _ = EndPaint(hwnd, &paint);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let point = point_from_lparam(lparam);
            STATE.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut() {
                    let drag = state
                        .selected
                        .and_then(|_| selection_in_client(state))
                        .map(|rect| selection_drag_at(rect, point))
                        .unwrap_or(SelectionDrag::Draw);
                    state.drag = Some(drag);
                    state.drag_origin = Some(point);
                    state.drag_initial = state.selected;
                    if drag == SelectionDrag::Draw {
                        state.selected = None;
                        state.start = Some(point);
                        state.current = Some(point);
                    }
                }
            });
            let _ = SetCapture(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            STATE.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut() {
                    let point = point_from_lparam(lparam);
                    let Some(drag) = state.drag else { return };
                    match drag {
                        SelectionDrag::Draw => state.current = Some(point),
                        SelectionDrag::Move => {
                            if let (Some(origin), Some(initial)) =
                                (state.drag_origin, state.drag_initial)
                            {
                                state.selected = Some(move_selection(
                                    state.bounds,
                                    initial,
                                    (point.0 - origin.0, point.1 - origin.1),
                                ));
                            }
                        }
                        resize => {
                            if let (Some(origin), Some(initial)) =
                                (state.drag_origin, state.drag_initial)
                            {
                                state.selected = resize_selection(
                                    state.bounds,
                                    initial,
                                    resize,
                                    (point.0 - origin.0, point.1 - origin.1),
                                );
                            }
                        }
                    }
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let point = point_from_lparam(lparam);
            STATE.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut() {
                    if state.drag == Some(SelectionDrag::Draw) {
                        state.current = Some(point);
                        state.selected = state
                            .start
                            .and_then(|start| selected_rect(state.bounds, start, point));
                    }
                    state.drag = None;
                    state.drag_origin = None;
                    state.drag_initial = None;
                }
            });
            let _ = ReleaseCapture();
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK
            if STATE
                .with_borrow(|slot| slot.as_ref().and_then(|state| state.selected).is_some()) =>
        {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN
            if _wparam.0 as u16 == VK_RETURN.0
                && STATE.with_borrow(|slot| {
                    slot.as_ref().and_then(|state| state.selected).is_some()
                }) =>
        {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN if _wparam.0 as u16 == VK_ESCAPE.0 => {
            // 取消必须清空已画出的选区：主流程以 selected 是否存在判定
            // 用户意图，残留会让 Esc 后照常截图
            STATE.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut() {
                    state.selected = None;
                    state.start = None;
                    state.current = None;
                    state.drag = None;
                }
            });
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            // 选择器运行在监听线程的嵌套消息循环中；仅销毁窗口不会让
            // `GetMessageW` 返回，必须投递退出消息让结果继续写入剪贴板。
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, _wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> CaptureRect {
        CaptureRect {
            x: -100,
            y: 20,
            width: 300,
            height: 200,
        }
    }

    #[test]
    fn 反向拖拽会得到同一个区域() {
        assert_eq!(
            selected_rect(bounds(), (260, 160), (20, 40)),
            Some(CaptureRect {
                x: -80,
                y: 60,
                width: 240,
                height: 120
            })
        );
    }

    #[test]
    fn 小于最小尺寸的点击不会产生截图() {
        assert_eq!(selected_rect(bounds(), (20, 20), (23, 23)), None);
    }

    #[test]
    fn 导出选区使用冻结底图而非再次抓屏() {
        let bounds = CaptureRect {
            x: 10,
            y: 20,
            width: 3,
            height: 2,
        };
        let mut original_bgra = Vec::new();
        for value in 0u8..6 {
            original_bgra.extend_from_slice(&[value, value + 20, value + 40, 255]);
        }
        let state = RegionState {
            bounds,
            dimmed_bgra: original_bgra.clone(),
            original_bgra,
            start: None,
            current: None,
            selected: None,
            drag: None,
            drag_origin: None,
            drag_initial: None,
        };
        let bmp = frozen_selection_bmp(
            &state,
            CaptureRect {
                x: 11,
                y: 20,
                width: 2,
                height: 2,
            },
        )
        .expect("冻结底图选区必须可编码");
        let image = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp)
            .expect("冻结选区 BMP 必须可读取")
            .to_rgba8();
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.get_pixel(0, 0).0, [41, 21, 1, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [45, 25, 5, 255]);
    }

    #[test]
    fn 越过边界的拖拽会裁剪到虚拟桌面() {
        assert_eq!(
            selected_rect(bounds(), (-20, -10), (500, 300)),
            Some(CaptureRect {
                x: -100,
                y: 20,
                width: 300,
                height: 200
            })
        );
    }

    #[test]
    fn 已有选区可区分移动和四角缩放() {
        let rect = (20, 30, 100, 60);
        assert_eq!(selection_drag_at(rect, (60, 50)), SelectionDrag::Move);
        assert_eq!(
            selection_drag_at(rect, (20, 30)),
            SelectionDrag::Resize {
                left: true,
                top: true,
                right: false,
                bottom: false
            }
        );
        assert_eq!(
            selection_drag_at(rect, (120, 90)),
            SelectionDrag::Resize {
                left: false,
                top: false,
                right: true,
                bottom: true
            }
        );
    }

    #[test]
    fn 移动与缩放保持在虚拟桌面内() {
        let rect = CaptureRect {
            x: 100,
            y: 100,
            width: 80,
            height: 60,
        };
        assert_eq!(
            move_selection(bounds(), rect, (500, 500)),
            CaptureRect {
                x: 120,
                y: 160,
                width: 80,
                height: 60
            }
        );
        assert_eq!(
            resize_selection(
                bounds(),
                rect,
                SelectionDrag::Resize {
                    left: false,
                    top: false,
                    right: true,
                    bottom: true
                },
                (500, 500)
            ),
            Some(CaptureRect {
                x: 100,
                y: 100,
                width: 100,
                height: 120
            })
        );
    }

    #[test]
    fn 缩放越过对侧时保持最小选区而不丢失() {
        let rect = CaptureRect {
            x: 0,
            y: 40,
            width: 80,
            height: 60,
        };
        assert_eq!(
            resize_selection(
                bounds(),
                rect,
                SelectionDrag::Resize {
                    left: true,
                    top: false,
                    right: false,
                    bottom: false,
                },
                (500, 0),
            ),
            Some(CaptureRect {
                x: 76,
                y: 40,
                width: 4,
                height: 60,
            })
        );
    }

    #[test]
    fn 保存文件名使用唯一时间戳与_png_格式() {
        assert_eq!(screenshot_file_name(123), "截图-123.png");
    }
}
