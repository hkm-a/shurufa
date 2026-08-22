#![windows_subsystem = "windows"]

//! shurufa-clipd：数据路径守护进程（阶段4第4项拆分，无 UI）。
//!
//! `run` 启动剪贴板监听入库与同步 daemon；`supervise` 由 supervisor
//! 看护 worker、算法服务与 shurufa-ui。面板与热键在 shurufa-ui 进程里。

use clap::{Parser, Subcommand};
use shurufa_host::{db_path, init_logging, log_line, open_store, supervis};

#[derive(Parser)]
#[command(
    name = "shurufa-clipd",
    about = "Shurufa 数据路径守护进程（剪贴板监听/同步/监管）"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 启动剪贴板监听（常驻 worker）
    Run,
    /// 常驻监管：看护 worker、算法服务与 shurufa-ui，崩溃自动重启
    Supervise,
    /// 查看监管与运行状态
    Status,
    /// 停止 supervisor
    Stop,
    /// 写入开机自启（HKCU Run）
    InstallAutostart,
    /// 移除开机自启
    UninstallAutostart,
    #[cfg(debug_assertions)]
    #[command(name = "test-set-image")]
    TestSetImage {
        width: Option<u32>,
        height: Option<u32>,
    },
    #[cfg(debug_assertions)]
    #[command(name = "test-inspect-image")]
    TestInspectImage,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Run => {
            // 崩溃必须留痕：监听为回调驱动，控制台通常不可见
            init_logging();
            shurufa_host::hide_own_console();
            // 单实例强制：同一时刻只允许一个 worker。已有实例（如未走
            // supervisor 的手动 run）在跑时直接退出，避免抢端口/热键冲突。
            let worker_mutex = supervis::worker_mutex_name();
            match supervis::acquire_singleton(&worker_mutex) {
                Ok(None) => {
                    eprintln!("已有剪贴板监听实例在运行（可用 status 查看，或 stop 后再启）。");
                    std::process::exit(1);
                }
                Ok(Some(h)) => {
                    // 保持持有直至进程退出
                    std::mem::forget(h);
                }
                Err(e) => {
                    eprintln!("创建 worker 单实例锁失败：{e}");
                    std::process::exit(1);
                }
            }
            shurufa_host::sync::start_daemon();
            let store = open_store();
            println!("剪贴板监听已启动，历史库：{}", db_path().display());
            log_line(&format!("守护进程启动，历史库：{}", db_path().display()));
            if let Err(e) = shurufa_host::listener::run(store) {
                eprintln!("监听进程异常退出：{e}");
                std::process::exit(1);
            }
        }
        Command::Supervise => {
            // 登录自启动进入 supervisor；独占控制台时立即隐藏，避免用户看到黑窗口。
            init_logging();
            shurufa_host::hide_own_console();
            supervis::supervise()
        }
        Command::Status => supervis::cmd_status(),
        Command::Stop => supervis::cmd_stop(),
        Command::InstallAutostart => match shurufa_host::install_autostart() {
            Ok(cmd) => println!("已写入开机自启（HKCU Run）：{cmd}"),
            Err(e) => {
                eprintln!("写入自启失败：{e}");
                std::process::exit(1);
            }
        },
        Command::UninstallAutostart => match shurufa_host::uninstall_autostart() {
            Ok(()) => println!("已移除开机自启"),
            Err(e) => {
                eprintln!("移除自启失败：{e}");
                std::process::exit(1);
            }
        },
        #[cfg(debug_assertions)]
        Command::TestSetImage { width, height } => {
            let width = width.unwrap_or(41);
            let height = height.unwrap_or(29);
            if shurufa_host::listener::request_test_image(width, height) {
                println!("已请求常驻进程写入测试图片剪贴板：{width}x{height}");
            } else {
                eprintln!("常驻进程未接受测试图片请求");
                std::process::exit(1);
            }
        }
        #[cfg(debug_assertions)]
        Command::TestInspectImage => match shurufa_host::listener::inspect_test_image() {
            Some((width, height)) => println!("图片={width}x{height}"),
            None => {
                eprintln!("常驻进程无法读取当前位图剪贴板");
                std::process::exit(1);
            }
        },
    }
}
