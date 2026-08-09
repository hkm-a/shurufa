//! shurufa-algo：librime 独立算法服务进程。
//!
//! 架构 M6 前置的一部分：把 librime 引擎从每个 TSF 宿主进程内移出到本进程，
//! 通过命名管道 `\\.\pipe\shurufa-algo` 服务按键与上下文。这样用户词库
//! （leveldb LOCK）只在**这一个**进程加载，多宿主进程不再互相抢锁。
//!
//! 为验证演进正确性，同时提供 `--once` 单次模式：从 stdin 读一串键序列
//! （如 `nihao`），服务一次后打印候选并退出，便于命令行回归验证引擎可用。

// GUI 子系统构建（见 build.rs）：由 supervisor/TSF 以子进程方式拉起时不挂
// 控制台窗口，不会出现在任务栏；`--once` 命令行模式下输出走 stdout 不受影响。
#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::process::exit;

use ime_ipc::pipe::{PipeServer, PIPE_NAME};

fn log(msg: &str) {
    eprintln!("[algo] {msg}");
}

/// 共享数据目录：优先取 `SHURUFA_SCHEMAS` 环境变量，否则沿 exe 上级找 schemas，
/// 最后回落到 `%APPDATA%\shurufa\schemas`。
fn shared_data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SHURUFA_SCHEMAS") {
        return PathBuf::from(p);
    }
    for dir in std::env::current_exe().ok().into_iter().flat_map(|exe| {
        let mut d = exe;
        let mut out = Vec::new();
        while let Some(p) = d.parent() {
            out.push(p.to_path_buf());
            d = p.to_path_buf();
        }
        out
    }) {
        let candidate = dir.join("schemas");
        if candidate.join("default.yaml").exists() {
            return candidate;
        }
    }
    user_config_root().join("schemas")
}

fn user_config_root() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
}

/// 初始化引擎（含词典部署）。
fn init_engine() -> ime_bridge::Engine {
    let shared = shared_data_dir();
    let user = user_config_root().join("rime");
    log(&format!("初始化引擎：shared={}", shared.display()));
    match ime_bridge::Engine::init(&shared, &user) {
        Ok(e) => {
            log("引擎就绪");
            e
        }
        Err(e) => {
            log(&format!("引擎初始化失败：{e}"));
            exit(2);
        }
    }
}

/// 进入常驻服务循环：反复新建管道实例、接受连接、服务请求。
fn run_service() -> ! {
    let engine = init_engine();
    let engine: &'static ime_bridge::Engine = Box::leak(Box::new(engine));
    log(&format!("监听 {} …", PIPE_NAME));
    loop {
        let server = match PipeServer::create() {
            Ok(s) => s,
            Err(e) => {
                log(&format!("创建管道失败：{e}；退出"));
                exit(3);
            }
        };
        if let Err(e) = server.accept() {
            log(&format!("接受连接失败：{e}；继续"));
            server.reset();
            continue;
        }
        log("收到连接，服务会话…");
        // 每个 TSF 宿主都会长期持有一个连接。接受后立即回到循环创建下一个
        // 管道实例，连接处理在独立线程中进行，避免首个宿主阻塞全部后续宿主。
        std::thread::spawn(move || {
            ime_ipc::server::serve_connection(&server, move || {
                engine
                    .create_session()
                    .map_err(|e| format!("创建会话失败：{e}"))
            });
            log("会话结束");
        });
    }
}

/// `--once` 模式：喂一串键序列打印候选（供自检/被 supervisor 拉起时冒烟）。
fn run_once(keys: &str) -> ! {
    let root = user_config_root();
    let shared = shared_data_dir();
    log(&format!("--once 模式，键序列：{keys}"));
    let engine = match ime_bridge::Engine::init(&shared, &root.join("rime")) {
        Ok(e) => e,
        Err(e) => {
            log(&format!("引擎初始化失败：{e}"));
            exit(2);
        }
    };
    let session = match engine.create_session() {
        Ok(s) => s,
        Err(e) => {
            log(&format!("创建会话失败：{e}"));
            exit(2);
        }
    };
    if !session.simulate(keys) {
        log("键序列未被引擎接受");
        exit(4);
    }
    let ctx = session.context();
    println!("preedit: {}", ctx.preedit);
    for (i, c) in ctx.candidates.iter().enumerate() {
        println!("  {}: {} {}", i, c.text, c.comment);
    }
    exit(0);
}

/// 算法服务单实例锁：同进程内保持持有直至退出，避免两个算法服务抢
/// 用户词库锁。supervisor 用探测方式判断是否已有算法服务在跑。
const ALGO_MUTEX: &str = r"Global\shurufa-algo";

fn hold_algo_lock() {
    use windows::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    let name = windows::core::HSTRING::from(ALGO_MUTEX);
    static HELD: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    unsafe {
        let handle = match CreateMutexW(None, true, &name) {
            Ok(h) => h,
            Err(e) => {
                log(&format!("创建算法服务单实例锁失败：{e}"));
                exit(0);
            }
        };
        let r = WaitForSingleObject(handle, 0);
        if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
            // 本进程成为唯一算法服务；把句柄存入 static 并永久持有，
            // 进程退出时由系统回收（HANDLE 是 Copy，mem::forget 无效）。
            let _ = HELD.set(());
            let _ = &handle; // 保持引用，防止被优化掉
            return;
        }
        let _ = CloseHandle(handle);
    }
    log("已有算法服务在运行，本实例退出");
    exit(0);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--once") => {
            // 命令行回归模式：若系统没分控制台（我们从 GUI 子系统启动），
            // 临时 attach 到父进程控制台让 stdout/stderr 可见。
            attach_console_to_parent();
            let keys = args.get(1).map(String::as_str).unwrap_or("nihao");
            run_once(keys);
        }
        _ => {
            // 常驻模式：先抢单实例锁，抢不到直接退出（已有算法服务在跑）
            hold_algo_lock();
            run_service();
        }
    }
}

/// windows_subsystem="windows" 构建下 stdout/stderr 默认无宿主；我们只在
/// `--once` 调用时把句柄接到调用方控制台（cmd/pwsh），其余常驻分支不接。
#[cfg(windows)]
fn attach_console_to_parent() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        // 父进程不是控制台时返回 Err；静默忽略，因为此时没人会看 stdout。
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console_to_parent() {}
