//! 基于 Windows Graphics Capture 的 MP4 录制。
//!
//! 使用 `windows-capture` 的硬件编码器，不退回到逐帧 BMP 或外部录屏软件。
//! 录制只在显式 CLI 调用时开始，音频默认关闭，避免后台采集。

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows_capture::capture::{Context as CaptureContext, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

pub struct RecordingReport {
    pub path: PathBuf,
    pub frames: u64,
    pub duration: Duration,
}

/// 已启动录制的停止控制。录制完成后只保留在图片目录的 MP4，不进入图片剪贴板历史。
pub struct RecordingStop {
    sender: mpsc::SyncSender<()>,
}

impl RecordingStop {
    pub fn stop(self) {
        let _ = self.sender.send(());
    }
}

struct RecordingFlags {
    path: PathBuf,
    width: u32,
    height: u32,
    report_sender: mpsc::SyncSender<Result<RecordingReport, String>>,
}

struct Recorder {
    encoder: Option<VideoEncoder>,
    started: Instant,
    frames: u64,
    flags: RecordingFlags,
}

impl Recorder {
    fn finish_recording(&mut self) {
        let Some(encoder) = self.encoder.take() else {
            return;
        };
        let result = encoder
            .finish()
            .map_err(|error| format!("完成 MP4 文件失败：{error}"))
            .map(|()| RecordingReport {
                path: self.flags.path.clone(),
                frames: self.frames,
                duration: self.started.elapsed(),
            });
        let _ = self.flags.report_sender.send(result);
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // 外部计时器停止捕获时，不一定恰好再收到一帧；析构处必须完成
        // 编码，保证静态桌面也能产生可播放的 MP4。
        self.finish_recording();
    }
}

impl GraphicsCaptureApiHandler for Recorder {
    type Flags = RecordingFlags;
    type Error = String;

    fn new(ctx: CaptureContext<Self::Flags>) -> Result<Self, Self::Error> {
        let video_settings = VideoSettingsBuilder::new(ctx.flags.width, ctx.flags.height)
            .frame_rate(30)
            .bitrate(10_000_000);
        let encoder = VideoEncoder::new(
            video_settings,
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            &ctx.flags.path,
        )
        .map_err(|error| format!("创建 MP4 编码器失败：{error}"))?;
        Ok(Self {
            encoder: Some(encoder),
            started: Instant::now(),
            frames: 0,
            flags: ctx.flags,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        self.encoder
            .as_mut()
            .ok_or_else(|| "录制编码器已结束".to_owned())?
            .send_frame(frame)
            .map_err(|error| format!("写入录制帧失败：{error}"))?;
        self.frames += 1;
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.finish_recording();
        Ok(())
    }
}

/// 录制主显示器至指定 MP4 文件。调用会阻塞到指定时长结束。
pub fn record_primary_monitor(path: &Path, duration: Duration) -> Result<RecordingReport, String> {
    if duration.is_zero() {
        return Err("录制时长必须大于零".to_owned());
    }
    let (stop_sender, stop_receiver) = mpsc::sync_channel(1);
    let _ = std::thread::spawn(move || {
        std::thread::sleep(duration);
        let _ = stop_sender.send(());
    });
    record_primary_monitor_until(path, stop_receiver, None)
}

/// 从桌面热键启动主显示器录制；接收器会在 MP4 完成或失败时返回最终结果。
pub fn start_default_recording() -> Result<
    (
        RecordingStop,
        PathBuf,
        mpsc::Receiver<Result<RecordingReport, String>>,
    ),
    String,
> {
    let path = default_recording_path()?;
    let (stop_sender, stop_receiver) = mpsc::sync_channel(1);
    let (started_sender, started_receiver) = mpsc::sync_channel(1);
    let (finished_sender, finished_receiver) = mpsc::sync_channel(1);
    let thread_path = path.clone();
    std::thread::spawn(move || {
        let result =
            record_primary_monitor_until(&thread_path, stop_receiver, Some(started_sender));
        let _ = finished_sender.send(result);
    });
    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "五秒内未能启动屏幕录制".to_owned())??;
    Ok((
        RecordingStop {
            sender: stop_sender,
        },
        path,
        finished_receiver,
    ))
}

fn default_recording_path() -> Result<PathBuf, String> {
    let pictures = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join("Pictures").join("Shurufa"))
        .ok_or_else(|| "无法定位用户图片目录".to_owned())?;
    let milliseconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("读取系统时间失败：{error}"))?
        .as_millis();
    Ok(pictures.join(format!("录屏-{milliseconds}.mp4")))
}

fn record_primary_monitor_until(
    path: &Path,
    stop_receiver: mpsc::Receiver<()>,
    started_sender: Option<mpsc::SyncSender<Result<(), String>>>,
) -> Result<RecordingReport, String> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        return Err("录制输出必须是 .mp4 文件".to_owned());
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建录制输出目录失败：{error}"))?;
    }
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let monitor = Monitor::primary().map_err(|error| format!("读取主显示器失败：{error}"))?;
    let width = monitor
        .width()
        .map_err(|error| format!("读取主显示器宽度失败：{error}"))?;
    let height = monitor
        .height()
        .map_err(|error| format!("读取主显示器高度失败：{error}"))?;
    let (report_sender, report_receiver) = mpsc::sync_channel(1);
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::WithCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Exclude,
        MinimumUpdateIntervalSettings::Custom(Duration::from_millis(33)),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        RecordingFlags {
            path,
            width,
            height,
            report_sender,
        },
    );
    let control =
        Recorder::start_free_threaded(settings).map_err(|error| format!("启动录制失败：{error}"));
    let control = match control {
        Ok(control) => control,
        Err(error) => {
            if let Some(sender) = started_sender {
                let _ = sender.send(Err(error.clone()));
            }
            return Err(error);
        }
    };
    if let Some(sender) = started_sender {
        let _ = sender.send(Ok(()));
    }
    // 时长或热键停止都通过控制通道收束；不能依赖下一帧到达，静态桌面
    // 不保证持续推帧。
    stop_receiver
        .recv()
        .map_err(|_| "录制停止控制已断开".to_owned())?;
    control
        .stop()
        .map_err(|error| format!("停止录制失败：{error}"))?;
    report_receiver
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "录制结束后未能取得 MP4 输出结果".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 录制输出扩展名大小写不敏感() {
        assert!(Path::new("capture.mp4")
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4")));
        assert!(Path::new("capture.MP4")
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4")));
        assert!(!Path::new("capture.png")
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4")));
    }

    #[test]
    fn 默认录制文件保存在_shurufa_图片目录() {
        let path = default_recording_path().expect("必须能构造默认录制路径");
        assert_eq!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("mp4")
        );
        assert_eq!(
            path.parent().and_then(|parent| parent.file_name()),
            Some("Shurufa".as_ref())
        );
        assert!(path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("录屏-")));
    }
}
