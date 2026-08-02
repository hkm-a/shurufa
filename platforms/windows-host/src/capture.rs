//! Shurufa 原生静态截图。
//!
//! 此模块只负责把屏幕像素编码为自包含 BMP。写入剪贴板、历史入库和
//! 跨设备同步仍由既有链路负责，避免形成第二套图片数据流。

use std::ffi::c_void;
use std::io::Cursor;
use std::sync::mpsc;
use std::time::Duration;

use windows_capture::capture::{Context as CaptureContext, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, CAPTUREBLT, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, GetWindowRect, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// 屏幕坐标系中的截图矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

struct CapturedFrame {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

/// `windows-capture` 的单帧适配器。它把 Windows Graphics Capture 的首帧
/// 转交给同步通道后立刻停止；持续帧则由后续录制模块复用同一接口处理。
struct SingleFrameCapture {
    sender: mpsc::SyncSender<Result<CapturedFrame, String>>,
}

impl GraphicsCaptureApiHandler for SingleFrameCapture {
    type Flags = mpsc::SyncSender<Result<CapturedFrame, String>>;
    type Error = String;

    fn new(ctx: CaptureContext<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { sender: ctx.flags })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let width = frame.width();
        let height = frame.height();
        let mut padding_free = Vec::new();
        let bgra = match frame.buffer() {
            Ok(buffer) => buffer.as_nopadding_buffer(&mut padding_free).to_vec(),
            Err(e) => {
                let _ = self.sender.send(Err(format!("读取图形捕获帧失败：{e}")));
                capture_control.stop();
                return Ok(());
            }
        };
        let _ = self.sender.send(Ok(CapturedFrame {
            width,
            height,
            bgra,
        }));
        capture_control.stop();
        Ok(())
    }
}

impl CaptureRect {
    /// 将区域裁剪到可见虚拟桌面；完全越界或非正尺寸时返回空。
    pub fn clamp_to(self, bounds: Self) -> Option<Self> {
        let right = self.x.checked_add(self.width)?;
        let bottom = self.y.checked_add(self.height)?;
        let bounds_right = bounds.x.checked_add(bounds.width)?;
        let bounds_bottom = bounds.y.checked_add(bounds.height)?;
        let x = self.x.max(bounds.x);
        let y = self.y.max(bounds.y);
        let right = right.min(bounds_right);
        let bottom = bottom.min(bounds_bottom);
        (right > x && bottom > y).then_some(Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

/// 读取包含全部显示器的虚拟桌面坐标。
pub fn virtual_screen_rect() -> Result<CaptureRect, String> {
    unsafe {
        let rect = CaptureRect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        };
        (rect.width > 0 && rect.height > 0)
            .then_some(rect)
            .ok_or_else(|| "无法读取显示器尺寸".to_owned())
    }
}

/// 读取当前前台窗口的外框，用于快捷键触发的窗口截图。
pub fn foreground_window_rect() -> Result<CaptureRect, String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return Err("当前没有可截图的前台窗口".to_owned());
        }
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).map_err(|e| format!("读取前台窗口位置失败：{e}"))?;
        CaptureRect {
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        }
        .clamp_to(virtual_screen_rect()?)
        .ok_or_else(|| "前台窗口不在可截图区域内".to_owned())
    }
}

/// 捕获一块可见屏幕区域，返回自包含 BMP。
pub fn capture_bmp(rect: CaptureRect) -> Result<Vec<u8>, String> {
    let rect = rect
        .clamp_to(virtual_screen_rect()?)
        .ok_or_else(|| "截图区域无效或已完全越界".to_owned())?;
    let pixel_count = (rect.width as usize)
        .checked_mul(rect.height as usize)
        .ok_or_else(|| "截图尺寸过大".to_owned())?;
    let byte_len = pixel_count
        .checked_mul(4)
        .ok_or_else(|| "截图尺寸过大".to_owned())?;

    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("无法获取屏幕设备上下文".to_owned());
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.is_invalid() {
            let _ = ReleaseDC(None, screen);
            return Err("无法创建截图内存上下文".to_owned());
        }

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: rect.width,
                // 负高度表示从上到下排列，省去读取后的垂直翻转。
                biHeight: -rect.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|e| format!("无法创建截图位图：{e}"));

        let result = (|| -> Result<Vec<u8>, String> {
            let bitmap = bitmap?;
            if bits.is_null() {
                return Err("截图位图没有可读像素缓冲区".to_owned());
            }
            let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
            if previous.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                return Err("无法选择截图位图".to_owned());
            }
            let copied = BitBlt(
                memory,
                0,
                0,
                rect.width,
                rect.height,
                Some(screen),
                rect.x,
                rect.y,
                SRCCOPY | CAPTUREBLT,
            );
            let _ = SelectObject(memory, previous);
            if let Err(e) = copied {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                return Err(format!("读取屏幕像素失败：{e}"));
            }

            // BI_RGB 的 32 位 DIB 在小端平台中为 BGRA，image 需要 RGBA。
            let mut rgba = std::slice::from_raw_parts(bits as *const u8, byte_len).to_vec();
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let image = image::RgbaImage::from_raw(rect.width as u32, rect.height as u32, rgba)
                .ok_or_else(|| "截图像素尺寸不匹配".to_owned())?;
            let mut output = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut output, image::ImageFormat::Bmp)
                .map_err(|e| format!("编码截图失败：{e}"))?;
            Ok(output.into_inner())
        })();
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(None, screen);
        result
    }
}

/// 以 Windows Graphics Capture 获取主显示器首帧。
///
/// 此路径是新功能的统一捕获后端：它与录制共用 Windows 的图形捕获管线，
/// 不会受 GDI 对 GPU 窗口和覆盖层的限制。坐标区域仍暂由旧 GDI 回退路径
/// 提供，直到选区编辑会话接入后统一在 Graphics Capture 帧上裁剪。
pub fn capture_primary_monitor_bmp() -> Result<Vec<u8>, String> {
    let monitor = Monitor::primary().map_err(|e| format!("读取主显示器失败：{e}"))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Exclude,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        sender,
    );
    SingleFrameCapture::start(settings).map_err(|e| format!("图形捕获启动失败：{e}"))?;
    let frame = receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "图形捕获在五秒内未返回首帧".to_owned())??;
    encode_bgra_bmp(frame.width, frame.height, &frame.bgra)
}

fn encode_bgra_bmp(width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "截图尺寸过大".to_owned())?;
    if width == 0 || height == 0 || bgra.len() != expected {
        return Err("图形捕获帧的尺寸或像素数据无效".to_owned());
    }
    let mut rgba = bgra.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let image = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "图形捕获帧无法转换为图片".to_owned())?;
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut output, image::ImageFormat::Bmp)
        .map_err(|e| format!("编码图形捕获帧失败：{e}"))?;
    Ok(output.into_inner())
}

/// 统一完成截图并交给既有剪贴板监听链路入历史与同步。
pub fn capture_to_clipboard(rect: CaptureRect) -> Result<(i32, i32), String> {
    let rect = rect
        .clamp_to(virtual_screen_rect()?)
        .ok_or_else(|| "截图区域无效或已完全越界".to_owned())?;
    let bmp = capture_bmp(rect)?;
    crate::paste::set_clipboard_new_image(&bmp).map_err(|e| format!("写入截图剪贴板失败：{e}"))?;
    Ok((rect.width, rect.height))
}

#[cfg(test)]
mod tests {
    use super::CaptureRect;

    #[test]
    fn 区域会裁剪到虚拟桌面() {
        let bounds = CaptureRect {
            x: -100,
            y: 0,
            width: 200,
            height: 100,
        };
        assert_eq!(
            CaptureRect {
                x: -140,
                y: -20,
                width: 80,
                height: 50,
            }
            .clamp_to(bounds),
            Some(CaptureRect {
                x: -100,
                y: 0,
                width: 40,
                height: 30,
            })
        );
    }

    #[test]
    fn 完全越界或非正区域会被拒绝() {
        let bounds = CaptureRect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        assert_eq!(
            CaptureRect {
                x: 100,
                y: 0,
                width: 1,
                height: 1,
            }
            .clamp_to(bounds),
            None
        );
        assert_eq!(
            CaptureRect {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            }
            .clamp_to(bounds),
            None
        );
    }

    #[test]
    fn 图形捕获帧会从_bgra_规范化为_bmp() {
        let bmp = super::encode_bgra_bmp(2, 1, &[3, 2, 1, 4, 30, 20, 10, 40]).unwrap();
        let image = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp)
            .unwrap()
            .to_rgba8();
        assert_eq!(image.as_raw(), &[1, 2, 3, 255, 10, 20, 30, 255]);
    }
}
