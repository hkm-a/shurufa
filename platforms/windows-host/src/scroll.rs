//! 长截图的逐帧重叠拼接。
//!
//! 状态与 ShareX 的滚动截图一致：每帧只在确认重叠后追加；无法确认时保留已经
//! 拼接的图片并返回“部分成功”，绝不把可能错位的图片标记为完整长图。

use std::cell::RefCell;

const MIN_OVERLAP_ROWS: u32 = 12;
const MAX_OUTPUT_ROWS: u32 = 32_000;
// 这是单通道平均误差。阈值必须足够严，避免平缓背景把滚动位移误判成整帧重叠。
const MAX_MEAN_ERROR: f64 = 8.0;
const MANUAL_SAMPLE_INTERVAL_MS: u32 = 240;
const MANUAL_MAX_FRAMES: u32 = 900;
const MANUAL_TOOLBAR_WIDTH: i32 = 320;
const MANUAL_TOOLBAR_HEIGHT: i32 = 44;
const MANUAL_TIMER_ID: usize = 0x534c;
const MANUAL_DONE_BUTTON: (i32, i32) = (204, 258);
const MANUAL_CANCEL_BUTTON: (i32, i32) = (263, 315);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StitchStatus {
    /// 已接受首帧或新增帧，仍需继续滚动。
    InProgress,
    /// 由调用方在正常到达底部后确认完成。
    Complete,
    /// 新帧无法与现有图像可靠对齐，保留此前结果。
    Partial,
    /// 首帧无效或尺寸不兼容，不能产生结果。
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StitchUpdate {
    pub status: StitchStatus,
    pub appended_rows: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollCaptureReport {
    pub status: StitchStatus,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
}

/// 只依赖像素帧的拼接器，滚轮、按键或窗口消息由上层驱动。
pub struct ScrollStitcher {
    output: Option<image::RgbaImage>,
    accepted_frames: u32,
}

impl ScrollStitcher {
    pub fn new() -> Self {
        Self {
            output: None,
            accepted_frames: 0,
        }
    }

    pub fn image(&self) -> Option<&image::RgbaImage> {
        self.output.as_ref()
    }

    /// 尝试追加一帧。仅在重叠误差足够小时写入输出。
    pub fn push(&mut self, frame: image::RgbaImage) -> StitchUpdate {
        if frame.width() == 0 || frame.height() == 0 {
            return StitchUpdate {
                status: StitchStatus::Failed,
                appended_rows: 0,
            };
        }
        let Some(previous) = self.output.as_ref() else {
            self.accepted_frames = 1;
            self.output = Some(frame);
            return StitchUpdate {
                status: StitchStatus::InProgress,
                appended_rows: 0,
            };
        };
        if frame.width() != previous.width() {
            return StitchUpdate {
                status: StitchStatus::Failed,
                appended_rows: 0,
            };
        }
        let Some(overlap) = find_overlap(previous, &frame) else {
            return StitchUpdate {
                status: if self.accepted_frames > 1 {
                    StitchStatus::Partial
                } else {
                    StitchStatus::Failed
                },
                appended_rows: 0,
            };
        };
        let appended_rows = frame.height() - overlap;
        let new_height = previous.height().saturating_add(appended_rows);
        if new_height > MAX_OUTPUT_ROWS {
            return StitchUpdate {
                status: StitchStatus::Partial,
                appended_rows: 0,
            };
        }
        let mut combined = image::RgbaImage::new(previous.width(), new_height);
        copy_rows(previous, &mut combined, 0, 0, previous.height());
        copy_rows(
            &frame,
            &mut combined,
            overlap,
            previous.height(),
            appended_rows,
        );
        self.output = Some(combined);
        self.accepted_frames += 1;
        StitchUpdate {
            status: StitchStatus::InProgress,
            appended_rows,
        }
    }

    /// 上层检测到滚动到底部时调用。只有至少一帧有效图片时可完成。
    pub fn finish(&self) -> StitchStatus {
        if self.output.is_some() {
            StitchStatus::Complete
        } else {
            StitchStatus::Failed
        }
    }
}

impl Default for ScrollStitcher {
    fn default() -> Self {
        Self::new()
    }
}

struct ManualScrollState {
    rect: crate::capture::CaptureRect,
    stitcher: ScrollStitcher,
    frames: u32,
    completed: bool,
    saw_partial: bool,
    error: Option<String>,
}

thread_local! {
    static MANUAL_STATE: RefCell<Option<ManualScrollState>> = const { RefCell::new(None) };
}

/// 在已选定的单一区域中持续采样。用户自行滚动内容，点击悬浮控制条的“完成”
/// 后才导出，因此不向前台窗口注入滚轮事件。
pub fn capture_region_manually_to_clipboard(
    rect: crate::capture::CaptureRect,
) -> Result<ScrollCaptureReport, String> {
    let bounds = crate::capture::virtual_screen_rect()?;
    let rect = rect
        .clamp_to(bounds)
        .ok_or_else(|| "长截图区域无效或已完全越界".to_owned())?;
    MANUAL_STATE.with_borrow_mut(|slot| {
        *slot = Some(ManualScrollState {
            rect,
            stitcher: ScrollStitcher::new(),
            frames: 0,
            completed: false,
            saw_partial: false,
            error: None,
        });
    });
    let result = unsafe { create_and_run_manual_window(rect, bounds) };
    let state = MANUAL_STATE.with_borrow_mut(|slot| slot.take());
    result?;
    let Some(state) = state else {
        return Err("长截图会话状态丢失".to_owned());
    };
    if let Some(error) = state.error {
        return Err(error);
    }
    if !state.completed {
        return Err("已取消长截图".to_owned());
    }
    let output = state
        .stitcher
        .image()
        .ok_or_else(|| "长截图未取得有效画面".to_owned())?;
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(output.clone())
        .write_to(&mut encoded, image::ImageFormat::Bmp)
        .map_err(|error| format!("编码滚动长截图失败：{error}"))?;
    crate::paste::set_clipboard_new_image(&encoded.into_inner())
        .map_err(|error| format!("写入滚动长截图剪贴板失败：{error}"))?;
    Ok(ScrollCaptureReport {
        status: if state.saw_partial {
            StitchStatus::Partial
        } else {
            state.stitcher.finish()
        },
        width: output.width(),
        height: output.height(),
        frames: state.frames,
    })
}

unsafe fn create_and_run_manual_window(
    rect: crate::capture::CaptureRect,
    bounds: crate::capture::CaptureRect,
) -> Result<(), String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassW, ShowWindow,
        TranslateMessage, CS_HREDRAW, CS_VREDRAW, MSG, SW_SHOW, WNDCLASSW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    let instance =
        GetModuleHandleW(PCWSTR::null()).map_err(|error| format!("读取模块失败：{error}"))?;
    let class_name = w!("ShurufaManualScrollCapture");
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(manual_wnd_proc),
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
    let (x, y) = manual_toolbar_origin(rect, bounds);
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class_name,
        w!("Shurufa 长截图"),
        WS_POPUP,
        x,
        y,
        MANUAL_TOOLBAR_WIDTH,
        MANUAL_TOOLBAR_HEIGHT,
        None,
        None,
        Some(instance.into()),
        None,
    )
    .map_err(|error| format!("创建长截图控制条失败：{error}"))?;
    let _ = ShowWindow(hwnd, SW_SHOW);
    if let Err(error) = capture_manual_frame() {
        MANUAL_STATE.with_borrow_mut(|slot| {
            if let Some(state) = slot.as_mut() {
                state.error = Some(error);
            }
        });
        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
    } else {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetTimer(
            Some(hwnd),
            MANUAL_TIMER_ID,
            MANUAL_SAMPLE_INTERVAL_MS,
            None,
        );
    }
    let mut message = MSG::default();
    while GetMessageW(&mut message, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    Ok(())
}

fn capture_manual_frame() -> Result<(), String> {
    let rect = MANUAL_STATE
        .with_borrow(|slot| slot.as_ref().map(|state| state.rect))
        .ok_or_else(|| "长截图会话状态丢失".to_owned())?;
    let bmp = crate::capture::capture_bmp(rect)?;
    let frame = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp)
        .map_err(|error| format!("读取长截图帧失败：{error}"))?
        .to_rgba8();
    MANUAL_STATE.with_borrow_mut(|slot| {
        let Some(state) = slot.as_mut() else {
            return Err("长截图会话状态丢失".to_owned());
        };
        if state.frames >= MANUAL_MAX_FRAMES {
            state.saw_partial = true;
            return Ok(());
        }
        state.frames += 1;
        match state.stitcher.push(frame).status {
            StitchStatus::Failed => {
                state.error = Some("长截图画面尺寸不兼容，已停止采样".to_owned());
            }
            StitchStatus::Partial => state.saw_partial = true,
            StitchStatus::InProgress | StitchStatus::Complete => {}
        }
        Ok(())
    })
}

fn manual_toolbar_origin(
    rect: crate::capture::CaptureRect,
    bounds: crate::capture::CaptureRect,
) -> (i32, i32) {
    let max_x = (bounds.x + bounds.width - MANUAL_TOOLBAR_WIDTH).max(bounds.x);
    let x = rect.x.clamp(bounds.x, max_x);
    let below = rect.y + rect.height + 8;
    let max_y = (bounds.y + bounds.height - MANUAL_TOOLBAR_HEIGHT).max(bounds.y);
    let y = if below <= max_y {
        below
    } else {
        (rect.y - MANUAL_TOOLBAR_HEIGHT - 8).clamp(bounds.y, max_y)
    };
    (x, y)
}

/// 返回 `Some(true)` 表示完成，`Some(false)` 表示取消；控制条空白区域不结束会话。
fn manual_toolbar_action(x: i32, y: i32) -> Option<bool> {
    if !(5..39).contains(&y) {
        return None;
    }
    if (MANUAL_DONE_BUTTON.0..MANUAL_DONE_BUTTON.1).contains(&x) {
        Some(true)
    } else if (MANUAL_CANCEL_BUTTON.0..MANUAL_CANCEL_BUTTON.1).contains(&x) {
        Some(false)
    } else {
        None
    }
}

unsafe fn paint_manual_toolbar(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, SetBkMode, SetTextColor,
        TextOutW, HGDIOBJ, PAINTSTRUCT, TRANSPARENT,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

    let mut paint = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut paint);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let background = CreateSolidBrush(COLORREF(0x002b2b2b));
    let _ = FillRect(hdc, &client, background);
    let _ = DeleteObject(HGDIOBJ(background.0));
    let done = RECT {
        left: MANUAL_DONE_BUTTON.0,
        top: 5,
        right: MANUAL_DONE_BUTTON.1,
        bottom: 39,
    };
    let cancel = RECT {
        left: MANUAL_CANCEL_BUTTON.0,
        top: 5,
        right: MANUAL_CANCEL_BUTTON.1,
        bottom: 39,
    };
    for (rect, color, label) in [
        (done, COLORREF(0x00c77628), "完成"),
        (cancel, COLORREF(0x00414141), "取消"),
    ] {
        let brush = CreateSolidBrush(color);
        let _ = FillRect(hdc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
        let text: Vec<u16> = label.encode_utf16().collect();
        let _ = SetTextColor(hdc, COLORREF(0x00f5f5f5));
        let _ = SetBkMode(hdc, TRANSPARENT);
        let _ = TextOutW(hdc, rect.left + 8, rect.top + 10, &text);
    }
    let message: Vec<u16> = "滚动内容后点完成".encode_utf16().collect();
    let _ = SetTextColor(hdc, COLORREF(0x00ffffff));
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = TextOutW(hdc, 10, 14, &message);
    let _ = EndPaint(hwnd, &paint);
}

unsafe extern "system" fn manual_wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    message: u32,
    _wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, DestroyWindow, KillTimer, PostQuitMessage, WM_DESTROY, WM_LBUTTONUP,
        WM_PAINT, WM_TIMER,
    };

    match message {
        WM_PAINT => {
            paint_manual_toolbar(hwnd);
            LRESULT(0)
        }
        WM_TIMER if _wparam.0 == MANUAL_TIMER_ID => {
            if let Err(error) = capture_manual_frame() {
                MANUAL_STATE.with_borrow_mut(|slot| {
                    if let Some(state) = slot.as_mut() {
                        state.error = Some(error);
                    }
                });
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = lparam.0 as u32 as u16 as i32;
            let y = ((lparam.0 as u32 >> 16) as u16 as i16) as i32;
            if let Some(completed) = manual_toolbar_action(x, y) {
                MANUAL_STATE.with_borrow_mut(|slot| {
                    if let Some(state) = slot.as_mut() {
                        state.completed = completed;
                    }
                });
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), MANUAL_TIMER_ID);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, _wparam, lparam),
    }
}

fn find_overlap(previous: &image::RgbaImage, next: &image::RgbaImage) -> Option<u32> {
    let max_overlap = previous.height().min(next.height());
    if max_overlap < MIN_OVERLAP_ROWS {
        return None;
    }
    // 忽略两侧 5%：网页滚动条、固定侧栏和鼠标悬停控件不应破坏正文对齐。
    let margin = (previous.width() / 20).min(32);
    let left = margin;
    let right = previous.width().saturating_sub(margin);
    if right <= left {
        return None;
    }
    (MIN_OVERLAP_ROWS..=max_overlap)
        .rev()
        .find(|&overlap| mean_error(previous, next, overlap, left, right) <= MAX_MEAN_ERROR)
}

fn mean_error(
    previous: &image::RgbaImage,
    next: &image::RgbaImage,
    overlap: u32,
    left: u32,
    right: u32,
) -> f64 {
    let start = previous.height() - overlap;
    let mut total = 0u64;
    let mut samples = 0u64;
    // 等距抽样可避免高分屏长图在每次滚轮后的匹配上占用过多 CPU。
    let x_step = ((right - left) / 96).max(1);
    let y_step = (overlap / 96).max(1);
    for relative_y in (0..overlap).step_by(y_step as usize) {
        for x in (left..right).step_by(x_step as usize) {
            let former = previous.get_pixel(x, start + relative_y).0;
            let latter = next.get_pixel(x, relative_y).0;
            total += u64::from(
                former[0]
                    .abs_diff(latter[0])
                    .max(former[1].abs_diff(latter[1]))
                    .max(former[2].abs_diff(latter[2])),
            );
            samples += 1;
        }
    }
    total as f64 / samples.max(1) as f64
}

fn copy_rows(
    source: &image::RgbaImage,
    destination: &mut image::RgbaImage,
    source_start: u32,
    destination_start: u32,
    rows: u32,
) {
    let row_bytes = source.width() as usize * 4;
    let source_offset = source_start as usize * row_bytes;
    let destination_offset = destination_start as usize * row_bytes;
    let byte_len = rows as usize * row_bytes;
    destination.as_mut()[destination_offset..destination_offset + byte_len]
        .copy_from_slice(&source.as_raw()[source_offset..source_offset + byte_len]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(start_row: u8, height: u32) -> image::RgbaImage {
        let mut image = image::RgbaImage::new(40, height);
        for y in 0..height {
            for x in 0..image.width() {
                let row = start_row.wrapping_add(y as u8);
                image.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        row.wrapping_mul(73),
                        (x as u8)
                            .wrapping_mul(41)
                            .wrapping_add(row.wrapping_mul(19)),
                        80u8.wrapping_add(row.wrapping_mul(29)),
                        255,
                    ]),
                );
            }
        }
        image
    }

    #[test]
    fn 重叠帧只会追加新行() {
        let mut stitcher = ScrollStitcher::new();
        assert_eq!(stitcher.push(frame(0, 24)).status, StitchStatus::InProgress);
        let update = stitcher.push(frame(12, 24));
        assert_eq!(
            update,
            StitchUpdate {
                status: StitchStatus::InProgress,
                appended_rows: 12
            }
        );
        assert_eq!(stitcher.image().unwrap().height(), 36);
        assert_eq!(stitcher.finish(), StitchStatus::Complete);
    }

    #[test]
    fn 侧边动态内容不会破坏正文重叠() {
        let first = frame(0, 24);
        let mut second = frame(12, 24);
        for y in 0..12 {
            second.put_pixel(0, y, image::Rgba([255, 0, 0, 255]));
            second.put_pixel(39, y, image::Rgba([255, 0, 0, 255]));
        }
        let mut stitcher = ScrollStitcher::new();
        stitcher.push(first.clone());
        assert_eq!(stitcher.push(second).appended_rows, 12);
        // 确认首帧未被拼接过程改写。
        assert_eq!(first.get_pixel(0, 0).0, [0, 0, 80, 255]);
    }

    #[test]
    fn 无法可靠匹配时返回部分成功而不追加() {
        let mut stitcher = ScrollStitcher::new();
        stitcher.push(frame(0, 24));
        stitcher.push(frame(12, 24));
        let before_height = stitcher.image().unwrap().height();
        let update = stitcher.push(frame(200, 24));
        assert_eq!(update.status, StitchStatus::Partial);
        assert_eq!(stitcher.image().unwrap().height(), before_height);
    }

    #[test]
    fn 宽度不一致直接失败() {
        let mut stitcher = ScrollStitcher::new();
        stitcher.push(frame(0, 24));
        let incompatible = image::RgbaImage::new(41, 24);
        assert_eq!(stitcher.push(incompatible).status, StitchStatus::Failed);
    }

    #[test]
    fn 手动长截图控制条优先放在选区下方并保持在虚拟桌面内() {
        let bounds = crate::capture::CaptureRect {
            x: -200,
            y: 0,
            width: 1600,
            height: 900,
        };
        assert_eq!(
            manual_toolbar_origin(
                crate::capture::CaptureRect {
                    x: 100,
                    y: 200,
                    width: 500,
                    height: 300,
                },
                bounds,
            ),
            (100, 508)
        );
        let (x, y) = manual_toolbar_origin(
            crate::capture::CaptureRect {
                x: 1300,
                y: 820,
                width: 200,
                height: 60,
            },
            bounds,
        );
        assert!((bounds.x..=bounds.x + bounds.width - MANUAL_TOOLBAR_WIDTH).contains(&x));
        assert!((bounds.y..=bounds.y + bounds.height - MANUAL_TOOLBAR_HEIGHT).contains(&y));
        assert!(y < 820, "底部空间不足时控制条应放在选区上方");
    }

    #[test]
    fn 手动长截图控制条只响应两个明确动作() {
        assert_eq!(manual_toolbar_action(210, 20), Some(true));
        assert_eq!(manual_toolbar_action(280, 20), Some(false));
        assert_eq!(manual_toolbar_action(100, 20), None);
        assert_eq!(manual_toolbar_action(210, 42), None);
    }
}
