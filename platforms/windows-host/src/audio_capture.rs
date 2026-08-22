//! 语音输入麦克风采集（cpal，16kHz / 16bit / 单声道 PCM）。
//!
//! v1.2 云端转写试点的录音侧：AudioCapture::start() 打开默认输入设备，
//! 回调模式下持续累积 PCM；stop() 停止并组装 WAV。纯函数部分
//! （WAV 头、PCM→WAV）有单测覆盖；cpal 实际硬件行为留实机验证。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// 采集参数：16kHz 单声道 16bit（约 32KB/s，3 分钟约 5.8MB，可接受）。
pub const SAMPLE_RATE: u32 = 16_000;
pub const CHANNELS: u16 = 1;
pub const BITS_PER_SAMPLE: u16 = 16;
/// 每个缓冲时长（毫秒）
#[cfg(test)]
pub const BUFFER_MS: u32 = 500;
/// 缓冲字节数 = 16000 * 1 * 2 * 0.5 = 16000
#[cfg(test)]
pub const BUFFER_BYTES: usize = (SAMPLE_RATE as usize)
    * (CHANNELS as usize)
    * (BITS_PER_SAMPLE as usize / 8)
    * (BUFFER_MS as usize)
    / 1000;

/// 回调共享上下文。
struct CaptureCtx {
    pcm: Mutex<Vec<u8>>,
    stop: AtomicBool,
}

/// 进行中的采集。stop() 消费自身，返回完整 WAV 字节。
pub struct AudioCapture {
    _stream: cpal::Stream,
    ctx: Arc<CaptureCtx>,
}

impl AudioCapture {
    /// 打开默认麦克风并开始录音。失败返回可展示的错误信息。
    pub fn start() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or("未找到默认输入设备")?;
        let supported = device
            .supported_input_configs()
            .map_err(|e| e.to_string())?;
        let range = supported
            .filter(|c| {
                c.channels() == CHANNELS
                    && c.min_sample_rate().0 <= SAMPLE_RATE
                    && SAMPLE_RATE <= c.max_sample_rate().0
            })
            .find(|c| {
                matches!(
                    c.sample_format(),
                    cpal::SampleFormat::I16 | cpal::SampleFormat::F32
                )
            })
            .ok_or("未找到 16k 单声道输入配置")?;
        let config = range.with_sample_rate(cpal::SampleRate(SAMPLE_RATE));
        let sample_format = config.sample_format();

        let ctx = Arc::new(CaptureCtx {
            pcm: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        });

        let err_fn = {
            let ctx = ctx.clone();
            move |e| {
                if !ctx.stop.load(Ordering::Relaxed) {
                    eprintln!("录音流错误：{e}");
                }
            }
        };

        let stream = match sample_format {
            cpal::SampleFormat::I16 => {
                let ctx = ctx.clone();
                device
                    .build_input_stream(
                        &config.into(),
                        move |data: &[i16], _| {
                            if !ctx.stop.load(Ordering::Relaxed) {
                                let mut pcm = ctx.pcm.lock().unwrap_or_else(|p| p.into_inner());
                                for &sample in data {
                                    pcm.extend_from_slice(&sample.to_le_bytes());
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| format!("打开麦克风失败：{e}"))?
            }
            cpal::SampleFormat::F32 => {
                let ctx = ctx.clone();
                device
                    .build_input_stream(
                        &config.into(),
                        move |data: &[f32], _| {
                            if !ctx.stop.load(Ordering::Relaxed) {
                                let mut pcm = ctx.pcm.lock().unwrap_or_else(|p| p.into_inner());
                                for &sample in data {
                                    let v = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
                                    pcm.extend_from_slice(&v.to_le_bytes());
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| format!("打开麦克风失败：{e}"))?
            }
            _ => return Err("不支持的采样格式".into()),
        };

        stream.play().map_err(|e| format!("启动录音失败：{e}"))?;
        Ok(Self {
            _stream: stream,
            ctx,
        })
    }

    /// 停止采集并返回完整 WAV 文件字节（含头）。
    pub fn stop(self) -> Vec<u8> {
        self.ctx.stop.store(true, Ordering::Relaxed);
        drop(self._stream);
        let pcm = self.ctx.pcm.lock().map(|p| p.clone()).unwrap_or_default();
        pcm_to_wav(&pcm)
    }
}

/// PCM（16bit LE 单声道）→ 完整 WAV 字节（用 hound 写头，替代手写 RIFF 头）。
pub fn pcm_to_wav(pcm: &[u8]) -> Vec<u8> {
    use std::io::Cursor;

    let spec = hound::WavSpec {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: BITS_PER_SAMPLE,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("创建 WAV writer 失败");
        for chunk in pcm.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            writer.write_sample(sample).expect("写入 WAV 采样失败");
        }
        writer.finalize().expect("finalize WAV 失败");
    }
    cursor.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm转wav_头部与数据完整() {
        let pcm = vec![0u8; 16000];
        let wav = pcm_to_wav(&pcm);
        assert_eq!(wav.len(), 44 + pcm.len());
        assert_eq!(
            u32::from_le_bytes(wav[4..8].try_into().unwrap()),
            36 + 16000
        );
        assert_eq!(&wav[44..], pcm.as_slice());
    }

    #[test]
    fn 缓冲参数合理() {
        assert_eq!(BUFFER_BYTES, 16000);
    }
}
