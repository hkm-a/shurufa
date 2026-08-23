//! 命名管道传输：服务端（shurufa-algo 内）与客户端（TSF 内）的收发原语。
//!
//! 用 Windows 消息模式管道：每次 WriteFile 是一条消息，每次 ReadFile 收一条
//! 完整消息，天然一帧一帧对齐。协议层仍保留长度前缀（[ime_ipc::encode_*]），
//! 便于将来切换为字节流模式或跨进程复用。

use std::io;

use std::ffi::c_void;

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, LocalFree, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PeekNamedPipe,
    SetNamedPipeHandleState, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};

pub const PIPE_NAME: &str = r"\\.\pipe\shurufa-algo";
/// 候选窗事件管道（TSF → shurufa-ui，阶段 6 候选窗迁出宿主进程）。
pub const CAND_PIPE_NAME: &str = r"\\.\pipe\shurufa-cand";

/// 每帧最大字节（候选最多 8 页 × 10 条，文本 UTF-8 足够）。
const MAX_FRAME: u32 = ime_ipc::MAX_FRAME_BYTES as u32;

type IoResult<T> = Result<T, io::Error>;

fn win_err(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0)
}

/// 服务端：新建一个命名管道实例并进入监听态。
pub struct PipeServer {
    handle: HANDLE,
}

unsafe impl Send for PipeServer {}

/// 生成的 SDDL，仅允许 SYSTEM、Administrators 与"创建者所有者"（即当前用户）访问。
/// 不再走默认 DACL，避免同机其他用户/沙盒进程注入按键或窃取 preedit。
/// 额外带 `S:(ML;;NW;;;ME)`：把管道标记为 Medium 完整性。若宿主链因故被提权
/// （如安装器以管理员启动服务），High 进程创建的管道默认隐式 High 标签会拒绝
/// 普通（Medium）应用连接 → 输入法整体失效（2026-08-14 实机复现：安装后
/// 普通应用 IPC 全拒，err=5）。显式 Medium 标签让普通应用在 DACL 允许时始终可达。
const PIPE_SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)S:(ML;;NW;;;ME)";

impl PipeServer {
    /// 新建管道实例并允许客户端连接（FILE_FLAG_FIRST_PIPE_INSTANCE 仅首个实例）。
    pub fn create() -> IoResult<Self> {
        Self::create_named(PIPE_NAME)
    }

    /// 指定管道名新建实例（algo 管道与候选窗管道共用同一传输实现与 SDDL）。
    pub fn create_named(name: &str) -> IoResult<Self> {
        unsafe {
            let mut sd: *mut c_void = std::ptr::null_mut();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(PIPE_SDDL),
                SDDL_REVISION_1,
                &mut sd as *mut _ as *mut _,
                None,
            )
            .map_err(win_err)?;
            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: false.into(),
            };
            // 警告：不要在此叠加 PIPE_REJECT_REMOTE_CLIENTS —— 实测它在 Win11
            // 26200 上让 CreateNamedPipeW 直接返回 ERROR_INVALID_PARAMETER(87)，
            // 见 examples/probe-pipe.rs。SDDL 仍限定 SY/BA/OW 三类 SID，远端
            // 客户端（匿名/网络 SID）受 ACL 拦截；远程桌面重定向是典型的常见远
            // 端访问路径，足以挡住。
            let handle = CreateNamedPipeW(
                &HSTRING::from(name),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0),
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                MAX_FRAME,
                MAX_FRAME,
                0,
                Some(&sa),
            );
            // SECURITY_DESCRIPTOR 由 ConvertString* 用 LocalAlloc 分配，使用后必须 LocalFree
            let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(sd)));
            if handle.is_invalid() {
                return Err(io::Error::last_os_error());
            }
            Ok(PipeServer { handle })
        }
    }

    /// 阻塞等待一个客户端连接。
    pub fn accept(&self) -> IoResult<()> {
        unsafe {
            let hr = ConnectNamedPipe(self.handle, None);
            match hr {
                Ok(()) => Ok(()),
                Err(e) => {
                    // 客户端在服务端 ConnectNamedPipe 之前已连接：视为成功
                    if e.code().0 == windows::Win32::Foundation::ERROR_PIPE_CONNECTED.0 as i32 {
                        return Ok(());
                    }
                    Err(win_err(e))
                }
            }
        }
    }

    /// 读一帧（消息模式一次 ReadFile 收一条完整消息）。
    pub fn read_frame(&self) -> IoResult<Vec<u8>> {
        let mut buf = vec![0u8; MAX_FRAME as usize];
        let mut read = 0u32;
        unsafe {
            ReadFile(self.handle, Some(&mut buf), Some(&mut read), None).map_err(win_err)?;
        }
        message_body(&buf[..read as usize])
    }

    /// 写一帧。
    pub fn write_frame(&self, data: &[u8]) -> IoResult<()> {
        validate_encoded_frame(data)?;
        unsafe { WriteFile(self.handle, Some(data), None, None).map_err(win_err) }
    }

    /// 断开当前连接。
    pub fn reset(&self) {
        unsafe {
            let _ = FlushFileBuffers(self.handle);
            let _ = DisconnectNamedPipe(self.handle);
        }
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// 客户端：打开现存命名管道。
pub struct PipeClient {
    handle: HANDLE,
}

unsafe impl Send for PipeClient {}

impl PipeClient {
    pub fn connect() -> IoResult<Self> {
        Self::connect_named(PIPE_NAME)
    }

    /// 指定管道名连接（候选窗事件管道客户端走这里）。
    pub fn connect_named(name: &str) -> IoResult<Self> {
        unsafe {
            let handle = CreateFileW(
                &HSTRING::from(name),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
            .map_err(win_err)?;
            if handle.is_invalid() {
                return Err(io::Error::last_os_error());
            }
            // 客户端的读端也要消息模式，配合服务端的消息写。
            SetNamedPipeHandleState(handle, Some(&PIPE_READMODE_MESSAGE), None, None).ok();
            Ok(PipeClient { handle })
        }
    }

    pub fn write_frame(&self, data: &[u8]) -> IoResult<()> {
        validate_encoded_frame(data)?;
        unsafe { WriteFile(self.handle, Some(data), None, None).map_err(win_err) }
    }

    pub fn read_frame(&self) -> IoResult<Vec<u8>> {
        let mut buf = vec![0u8; MAX_FRAME as usize];
        let mut read = 0u32;
        unsafe {
            ReadFile(self.handle, Some(&mut buf), Some(&mut read), None).map_err(win_err)?;
        }
        message_body(&buf[..read as usize])
    }

    /// 带超时读一帧：先用 PeekNamedPipe 非阻塞轮询确认有数据，再 ReadFile。
    /// `timeout` 总预算；超时返回 `timed_out()`（调用方应放弃本次请求并降级，
    /// 绝不让 UI 线程无限阻塞 —— 服务端卡死时客户端必须能退出）。
    pub fn read_frame_timeout(&self, timeout: std::time::Duration) -> IoResult<Vec<u8>> {
        use std::time::Instant;
        let start = Instant::now();
        loop {
            let mut available: u32 = 0;
            let mut total: u32 = 0;
            let ok = unsafe {
                PeekNamedPipe(
                    self.handle,
                    None,
                    0,
                    None,
                    Some(&mut available),
                    Some(&mut total),
                )
            };
            if ok.is_err() {
                return Err(io::Error::last_os_error());
            }
            if available > 0 {
                return self.read_frame();
            }
            if start.elapsed() >= timeout {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "pipe read timed out",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

impl Drop for PipeClient {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// 从一条完整消息中取出 JSON 体，并拒绝伪造或截断的长度前缀。
fn message_body(message: &[u8]) -> IoResult<Vec<u8>> {
    validate_encoded_frame(message)?;
    Ok(message[4..].to_vec())
}

/// 命名管道使用消息模式，因此每次写入都必须是一个完整且自描述的帧。
fn validate_encoded_frame(frame: &[u8]) -> IoResult<()> {
    if frame.len() < 4 || frame.len() > MAX_FRAME as usize {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "IPC 帧长度非法"));
    }
    let declared = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if declared != frame.len() - 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC 帧长度前缀与消息体不一致",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{message_body, validate_encoded_frame};

    #[test]
    fn accepts_complete_frame() {
        assert_eq!(message_body(&[2, 0, 0, 0, b'{', b'}']).unwrap(), b"{}");
    }

    #[test]
    fn rejects_truncated_or_mismatched_frame() {
        assert!(validate_encoded_frame(&[1, 0, 0, 0]).is_err());
        assert!(validate_encoded_frame(&[3, 0, 0, 0, b'{', b'}']).is_err());
    }
}
