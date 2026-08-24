//! TSF 侧宿主 toast 客户端：连到 `\\.\pipe\shurufa-toast`，收到 HostToast 后
//! 经隐藏窗口消息投递到 TSF UI 线程，调用 `crate::toast::show` 弹出轻量提示。
//!
//! 断线自动每 2 秒重连；只负责“宿主事件 → 用户可见 toast”的最后一跳。

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ime_ipc::{decode_host_toast, encode_toast_hello, HostToast, ToastHello};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, PostMessageW, RegisterClassW, WM_APP, WNDCLASSW,
};
use windows_ipc::pipe::{PipeClient, TOAST_PIPE_NAME};

const WM_APP_TOAST: u32 = WM_APP + 91;
const TOAST_CLASS: PCWSTR = w!("ShurufaToastPipeHost");
static TOAST_HWND: AtomicIsize = AtomicIsize::new(0);

/// PipeClient 只有 Send 没有 Sync；客户端读写由同一 reader 线程顺序执行。
struct SyncPipeClient(PipeClient);
unsafe impl Sync for SyncPipeClient {}

impl std::ops::Deref for SyncPipeClient {
    type Target = PipeClient;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 启动 toast 管道客户端（进程内只启动一次）。
pub fn start() {
    use std::sync::Once;
    static START: Once = Once::new();
    START.call_once(|| unsafe {
        let Some(hwnd) = create_hidden_window() else {
            return;
        };
        TOAST_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        std::thread::spawn(client_loop);
    });
}

unsafe fn create_hidden_window() -> Option<HWND> {
    let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance.into(),
        lpszClassName: TOAST_CLASS,
        ..Default::default()
    };
    RegisterClassW(&class);
    CreateWindowExW(
        windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
        TOAST_CLASS,
        w!(""),
        windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(0),
        0,
        0,
        0,
        0,
        None,
        None,
        Some(hinstance.into()),
        None,
    )
    .ok()
}

fn client_loop() {
    loop {
        let client = match PipeClient::connect_named(TOAST_PIPE_NAME) {
            Ok(c) => c,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        let shared = Arc::new(SyncPipeClient(client));
        let hello = ToastHello {
            pid: std::process::id(),
        };
        if encode_toast_hello(&hello)
            .ok()
            .and_then(|f| shared.write_frame(&f).ok())
            .is_none()
        {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        loop {
            match shared.read_frame_timeout(Duration::from_millis(200)) {
                Ok(frame) => {
                    if let Ok(toast) = decode_host_toast(&frame) {
                        post_to_ui(toast);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(_) => break,
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn post_to_ui(toast: HostToast) {
    let hwnd = TOAST_HWND.load(Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    let boxed = Box::into_raw(Box::new(toast));
    unsafe {
        if PostMessageW(
            Some(HWND(hwnd as *mut _)),
            WM_APP_TOAST,
            WPARAM(0),
            LPARAM(boxed as isize),
        )
        .is_err()
        {
            drop(Box::from_raw(boxed));
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_APP_TOAST {
        let ptr = lparam.0 as *mut HostToast;
        if !ptr.is_null() {
            let toast = Box::from_raw(ptr);
            crate::toast::show(&toast.text, None);
        }
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
