//! 语音输入麦克风采集（waveIn，16kHz / 16bit / 单声道 PCM）。
//!
//! v1.2 云端转写试点的录音侧：AudioCapture::start() 打开默认输入设备，
//! 回调模式下持续累积 PCM；stop() 停止并组装 WAV。纯函数部分
//! （WAV 头、PCM→WAV）有单测覆盖；waveIn 实际硬件行为留实机验证。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use windows::Win32::Media::Audio::{
    waveInAddBuffer, waveInClose, waveInOpen, waveInPrepareHeader, waveInReset, waveInStart,
    waveInStop, waveInUnprepareHeader, HWAVEIN, MIDI_WAVE_OPEN_TYPE, WAVEFORMATEX, WAVEHDR,
};

/// 采集参数：16kHz 单声道 16bit（约 32KB/s，3 分钟约 5.8MB，可接受）。
pub const SAMPLE_RATE: u32 = 16_000;
pub const CHANNELS: u16 = 1;
pub const BITS_PER_SAMPLE: u16 = 16;
/// 每个 WAVEHDR 缓冲时长（毫秒）
pub const BUFFER_MS: u32 = 500;
/// 缓冲字节数 = 16000 * 1 * 2 * 0.5 = 16000
pub const BUFFER_BYTES: usize = (SAMPLE_RATE as usize)
    * (CHANNELS as usize)
    * (BITS_PER_SAMPLE as usize / 8)
    * (BUFFER_MS as usize)
    / 1000;
/// 同时挂入的缓冲数
const BUFFER_COUNT: usize = 4;

/// WIM_DATA 消息（有数据到达）
const WIM_DATA: u32 = 0x03BF;

/// 回调共享上下文（泄漏到静态区，采集结束时回收）。
struct CaptureCtx {
    pcm: Mutex<Vec<u8>>,
    stop: AtomicBool,
    frames: AtomicU64,
}

unsafe extern "system" fn wave_callback(
    hwi: HWAVEIN,
    u_msg: u32,
    instance: usize,
    param1: usize,
    _param2: usize,
) {
    if u_msg != WIM_DATA {
        return;
    }
    let ctx = &*(instance as *const CaptureCtx);
    let hdr = param1 as *mut WAVEHDR;
    if !ctx.stop.load(Ordering::Relaxed) {
        let len = (*hdr).dwBytesRecorded as usize;
        if len > 0 {
            let data = std::slice::from_raw_parts((*hdr).lpData.0 as *const u8, len);
            if let Ok(mut pcm) = ctx.pcm.lock() {
                pcm.extend_from_slice(data);
                ctx.frames.fetch_add(1, Ordering::Relaxed);
            }
        }
        (*hdr).dwBytesRecorded = 0;
        let _ = unsafe { waveInAddBuffer(hwi, hdr, std::mem::size_of::<WAVEHDR>() as u32) };
    }
}

/// 一个挂载缓冲：WAVEHDR 与数据区同盒，保证地址稳定。
struct WavBuffer {
    header: WAVEHDR,
    data: Vec<u8>,
}

/// 进行中的采集。stop() 消费自身，返回完整 WAV 字节。
pub struct AudioCapture {
    hwi: HWAVEIN,
    ctx: &'static mut CaptureCtx,
    /// Box 保证 waveIn 驱动的 WAVEHDR/数据地址不随 Vec 扩容而移动。
    #[allow(clippy::vec_box)]
    buffers: Vec<Box<WavBuffer>>,
}

impl AudioCapture {
    /// 打开默认麦克风并开始录音。失败返回可展示的错误信息。
    pub fn start() -> Result<Self, String> {
        let ctx: &'static mut CaptureCtx = Box::leak(Box::new(CaptureCtx {
            pcm: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
            frames: AtomicU64::new(0),
        }));
        let fmt = WAVEFORMATEX {
            wFormatTag: 1, // WAVE_FORMAT_PCM
            nChannels: CHANNELS,
            nSamplesPerSec: SAMPLE_RATE,
            nAvgBytesPerSec: SAMPLE_RATE * (BITS_PER_SAMPLE as u32 / 8),
            nBlockAlign: CHANNELS * (BITS_PER_SAMPLE / 8),
            wBitsPerSample: BITS_PER_SAMPLE,
            cbSize: 0,
        };
        let mut hwi = HWAVEIN::default();
        let rc = unsafe {
            waveInOpen(
                Some(&mut hwi),
                windows::Win32::Media::Audio::WAVE_MAPPER,
                &fmt,
                Some(wave_callback as *const () as usize),
                Some(ctx as *const CaptureCtx as usize),
                MIDI_WAVE_OPEN_TYPE(196608u32),
            )
        };
        if rc != 0 {
            drop(unsafe { Box::from_raw(ctx as *mut CaptureCtx) });
            return Err(format!(
                "打开麦克风失败（MMSYSERR {rc}）：请检查录音设备与权限"
            ));
        }
        let mut buffers = Vec::with_capacity(BUFFER_COUNT);
        for _ in 0..BUFFER_COUNT {
            let mut data = vec![0u8; BUFFER_BYTES];
            let header = WAVEHDR {
                lpData: windows::core::PSTR(data.as_mut_ptr()),
                dwBufferLength: data.len() as u32,
                dwBytesRecorded: 0,
                dwUser: 0,
                dwFlags: 0,
                dwLoops: 0,
                lpNext: std::ptr::null_mut(),
                reserved: 0,
            };
            let mut buf = Box::new(WavBuffer { header, data });
            let rc = unsafe {
                waveInPrepareHeader(hwi, &mut buf.header, std::mem::size_of::<WAVEHDR>() as u32)
            };
            if rc != 0 {
                drop(unsafe { Box::from_raw(ctx as *mut CaptureCtx) });
                let _ = unsafe { waveInClose(hwi) };
                return Err(format!("准备录音缓冲失败（MMSYSERR {rc}）"));
            }
            let rc = unsafe {
                waveInAddBuffer(hwi, &mut buf.header, std::mem::size_of::<WAVEHDR>() as u32)
            };
            if rc != 0 {
                drop(unsafe { Box::from_raw(ctx as *mut CaptureCtx) });
                let _ = unsafe { waveInClose(hwi) };
                return Err(format!("挂载录音缓冲失败（MMSYSERR {rc}）"));
            }
            buffers.push(buf);
        }
        let rc = unsafe { waveInStart(hwi) };
        if rc != 0 {
            drop(unsafe { Box::from_raw(ctx as *mut CaptureCtx) });
            let _ = unsafe { waveInClose(hwi) };
            return Err(format!("启动录音失败（MMSYSERR {rc}）"));
        }
        Ok(Self { hwi, ctx, buffers })
    }

    /// 停止采集并返回完整 WAV 文件字节（含 44 字节头）。
    pub fn stop(mut self) -> Vec<u8> {
        self.ctx.stop.store(true, Ordering::Relaxed);
        unsafe {
            let _ = waveInStop(self.hwi);
            let _ = waveInReset(self.hwi);
        }
        for mut buf in self.buffers.drain(..) {
            debug_assert!(buf.header.dwBytesRecorded as usize <= buf.data.len());
            unsafe {
                let _ = waveInUnprepareHeader(
                    self.hwi,
                    &mut buf.header,
                    std::mem::size_of::<WAVEHDR>() as u32,
                );
            }
        }
        unsafe {
            let _ = waveInClose(self.hwi);
        }
        let pcm = self.ctx.pcm.lock().map(|p| p.clone()).unwrap_or_default();
        drop(unsafe { Box::from_raw(self.ctx as *mut CaptureCtx) });
        pcm_to_wav(&pcm)
    }
}

/// 组装 44 字节 WAV(PCM) 文件头（小端）。
pub fn build_wav_header(data_len: usize) -> [u8; 44] {
    let byte_rate = SAMPLE_RATE * (BITS_PER_SAMPLE as u32 / 8) * (CHANNELS as u32);
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&((36 + data_len) as u32).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&CHANNELS.to_le_bytes());
    h[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&block_align.to_le_bytes());
    h[34..36].copy_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&(data_len as u32).to_le_bytes());
    h
}

/// PCM（16bit LE 单声道）→ 完整 WAV 字节。
pub fn pcm_to_wav(pcm: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(&build_wav_header(pcm.len()));
    out.extend_from_slice(pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav头_符合_riff规范() {
        let h = build_wav_header(32000);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(&h[8..12], b"WAVE");
        assert_eq!(&h[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes(h[16..20].try_into().unwrap()), 16);
        assert_eq!(u16::from_le_bytes(h[20..22].try_into().unwrap()), 1, "PCM");
        assert_eq!(
            u16::from_le_bytes(h[22..24].try_into().unwrap()),
            1,
            "单声道"
        );
        assert_eq!(u32::from_le_bytes(h[24..28].try_into().unwrap()), 16000);
        assert_eq!(
            u32::from_le_bytes(h[40..44].try_into().unwrap()),
            32000,
            "data 长度"
        );
    }

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
