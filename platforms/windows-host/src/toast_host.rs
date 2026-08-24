//! 宿主 → TSF toast 管道服务端。
//!
//! TSF DLL 在各自宿主进程作为客户端连到 `\\.\pipe\shurufa-toast`，连接后先发
//! `ToastHello{pid}`；本模块按进程 PID 保存连接。`send_toast` 取当前前台进程
//! PID，把 `HostToast` 写到对应 TSF 实例，由 TSF 在 UI 线程弹出轻量提示。

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use ime_ipc::{decode_toast_hello, encode_host_toast, HostToast};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows_ipc::pipe::{PipeServer, TOAST_PIPE_NAME};

use crate::log_line;

/// PipeServer 只有 Send 没有 Sync；写端由连接线程独占，包一层显式 Sync。
struct SyncPipe(PipeServer);
unsafe impl Sync for SyncPipe {}

impl Deref for SyncPipe {
    type Target = PipeServer;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// pid → 该 TSF 实例的命令发送端（主调用线程 send_toast 投递）。
static CONNS: OnceLock<Mutex<HashMap<u32, Sender<HostToast>>>> = OnceLock::new();

fn conns() -> &'static Mutex<HashMap<u32, Sender<HostToast>>> {
    CONNS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 启动 toast 管道服务（异步 accept；返回 false 表示创建首个实例失败）。
pub fn start() -> bool {
    let handle = match PipeServer::create_named(TOAST_PIPE_NAME) {
        Ok(h) => h,
        Err(e) => {
            log_line(&format!("toast_host: 创建管道失败：{e}"));
            return false;
        }
    };
    // 首个实例成功后进入 accept 循环；后续实例在循环内创建。
    std::thread::spawn(move || accept_loop_after(handle));
    true
}

fn accept_loop_after(first: PipeServer) {
    if let Err(e) = first.accept() {
        log_line(&format!("toast_host: 接受连接失败：{e}"));
    } else {
        spawn_connection(first);
    }
    accept_loop();
}

fn accept_loop() {
    loop {
        let server = match PipeServer::create_named(TOAST_PIPE_NAME) {
            Ok(s) => s,
            Err(e) => {
                log_line(&format!("toast_host: 创建管道失败：{e}"));
                return;
            }
        };
        if let Err(e) = server.accept() {
            log_line(&format!("toast_host: 接受连接失败：{e}"));
            continue;
        }
        spawn_connection(server);
    }
}

fn spawn_connection(server: PipeServer) {
    let conn = Arc::new(SyncPipe(server));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || serve_connection(conn, tx, rx));
}

fn serve_connection(conn: Arc<SyncPipe>, tx: Sender<HostToast>, rx: Receiver<HostToast>) {
    // 第一条必须是 TSF 注册帧 ToastHello。
    let frame = match conn.read_frame() {
        Ok(f) => f,
        Err(_) => return,
    };
    let Ok(hello) = decode_toast_hello(&frame) else {
        return;
    };
    conns().lock().unwrap().insert(hello.pid, tx.clone());
    log_line(&format!("toast_host: TSF 进程 {} 已连接", hello.pid));

    loop {
        match conn.peek_available() {
            Ok(true) => {
                // 客户端可能发心跳/断开；这里只消费，不处理。
                let _ = conn.read_frame();
            }
            Ok(false) => match rx.try_recv() {
                Ok(toast) => {
                    let Ok(frame) = encode_host_toast(&toast) else {
                        continue;
                    };
                    if conn.write_frame(&frame).is_err() {
                        break;
                    }
                }
                Err(TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(TryRecvError::Disconnected) => break,
            },
            Err(_) => break,
        }
    }

    conns().lock().unwrap().remove(&hello.pid);
    log_line(&format!("toast_host: TSF 进程 {} 已断开", hello.pid));
}

/// 给当前前台进程的 TSF 实例发送一条 toast；无连接时返回 false。
pub fn send_toast(text: &str) -> bool {
    let pid = foreground_pid();
    if pid == 0 {
        return false;
    }
    let Some(tx) = conns().lock().unwrap().get(&pid).cloned() else {
        return false;
    };
    tx.send(HostToast {
        text: text.to_owned(),
    })
    .is_ok()
}

fn foreground_pid() -> u32 {
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return 0;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid
    }
}
