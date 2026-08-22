#![windows_subsystem = "windows"]

//! shurufa-host：桌面常驻进程。
//!
//! `run` 子命令启动剪贴板监听并写入历史库；其余子命令面向历史库的
//! 查询与管理，供验收与后续 UI 面板复用。

mod ai_panel;
mod asr;
mod audio_capture;
mod dict_update;
mod listener;
mod onscreen_kbd;
mod panel;
mod paste;
mod speech;
mod supervis;
mod sync;
#[cfg(debug_assertions)]
mod tsf_probe;

use clap::{Parser, Subcommand};
use clipboard_store::{ClipEntry, ClipKind, ClipboardStore, RetentionPolicy};
use std::path::PathBuf;

/// 文件日志：常驻进程以最小化/无窗口方式运行，控制台输出不可见，
/// 排障信息统一落 %TEMP%\shurufa-host.log，失败静默。
pub fn log_line(msg: &str) {
    use std::io::Write;
    let path = std::env::var_os("SHURUFA_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("shurufa-host.log"));
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = f.write_all(
            format!(
                "[{ts}] {msg}
"
            )
            .as_bytes(),
        );
    }
}

/// shurufa 应用数据目录：%APPDATA%\shurufa（无 APPDATA 时回退临时目录）。
pub fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
}

fn db_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SHURUFA_DB_PATH") {
        return PathBuf::from(path);
    }
    app_data_dir().join("clipboard.db")
}

fn open_store() -> ClipboardStore {
    let path = db_path();
    match ClipboardStore::open(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("打开历史库失败 {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}

#[derive(Parser)]
#[command(name = "shurufa-host", about = "Shurufa 桌面常驻进程")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 启动剪贴板监听（常驻 worker）
    Run,
    /// 常驻监管：看护 worker，崩溃自动重启
    Supervise,
    /// 查看监管与运行状态
    Status,
    /// 停止 supervisor
    Stop,
    /// 最近 N 条历史（默认 20）
    List { n: Option<u32> },
    /// 搜索文本与文件名
    Search { query: String },
    /// search 同义别名（供脚本用）
    #[command(name = "clip-search")]
    ClipSearch { query: String },
    /// 跨设备搜索（8 秒聚合）
    #[command(name = "clip-remote-search")]
    ClipRemoteSearch { query: String },
    /// Agnes 一次性帮写（不弹面板）
    Chat { prompt: String },
    /// 唤起 AI 帮写面板（后台服务常驻时）
    Ai { action: String },
    /// 置顶
    Pin { id: u32 },
    /// 取消置顶
    Unpin { id: u32 },
    /// 删除单条
    Delete { id: u32 },
    /// 把条目写回剪贴板
    Copy { id: u32 },
    /// 写入开机自启（HKCU Run）
    InstallAutostart,
    /// 移除开机自启
    UninstallAutostart,
    /// 清空未置顶记录
    Clear,
    /// 发起配对（控制台确认码交互）
    Pair { addr: String },
    /// 设置中心配对向导发起端（文件确认）
    #[command(name = "pair-ui")]
    PairUi { addr: String },
    /// 列出已配对设备
    Devices,
    /// 取消配对
    Unpair { fp: String },
    /// 配置或关闭自托管同步中继
    Relay { value: String },
    /// 更新自托管云词库
    #[command(name = "dict-update")]
    DictUpdate { url: String },
    /// 重新部署：重建二进制词典（方案/词库改动后）
    Deploy,
    /// 回滚词库（默认上一代）
    #[command(name = "dict-rollback")]
    DictRollback {
        /// 回滚到指定版本或内置
        #[arg(long)]
        revision: Option<String>,
    },
    /// 列出本地可回滚的历史版本
    #[command(name = "dict-history")]
    DictHistory,
    /// 打印当前词库版本
    #[command(name = "dict-current")]
    DictCurrent,
    /// 立即执行留存清理
    Retention,
    #[cfg(debug_assertions)]
    #[command(name = "test-set-image")]
    TestSetImage {
        width: Option<u32>,
        height: Option<u32>,
    },
    #[cfg(debug_assertions)]
    #[command(name = "test-inspect-image")]
    TestInspectImage,
    #[cfg(debug_assertions)]
    #[command(name = "tsf-native-probe")]
    TsfNativeProbe,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Run => {
            // 崩溃必须留痕：面板/监听均为回调驱动，控制台通常不可见
            hide_own_console();
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
            sync::start_daemon();
            // 高分屏下面板按真实 DPI 布局渲染，而非被系统位图拉伸
            unsafe {
                use windows::Win32::UI::HiDpi::{
                    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                };
                let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            }
            let store = open_store();
            println!("剪贴板监听已启动，历史库：{}", db_path().display());
            log_line(&format!("守护进程启动，历史库：{}", db_path().display()));
            if let Err(e) = listener::run(store) {
                eprintln!("监听进程异常退出：{e}");
                std::process::exit(1);
            }
        }
        Command::Supervise => {
            // 登录自启动进入 supervisor；独占控制台时立即隐藏，避免用户看到黑窗口。
            hide_own_console();
            supervis::supervise()
        }
        Command::Status => supervis::cmd_status(),
        Command::Stop => supervis::cmd_stop(),
        Command::List { n } => {
            let n = n.unwrap_or(20);
            print_entries(&open_store().list(n, 0).unwrap_or_default());
        }
        Command::Search { query } => {
            print_entries(&open_store().search(&query, 50).unwrap_or_default());
        }
        Command::ClipSearch { query } => {
            print_entries(&open_store().search(&query, 50).unwrap_or_default());
        }
        Command::ClipRemoteSearch { query } => {
            sync::cli_remote_search(&query);
        }
        Command::Ai { action } => match action.as_str() {
            "show" => {
                use windows::Win32::Foundation::{LPARAM, WPARAM};
                use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
                let class = windows::core::w!("ShurufaAiPanel");
                match unsafe { FindWindowW(class, None) } {
                    Ok(hwnd) => {
                        let _ = unsafe {
                            PostMessageW(
                                Some(hwnd),
                                ai_panel::WM_AI_EXTERNAL_SHOW,
                                WPARAM(0),
                                LPARAM(0),
                            )
                        };
                        println!("已唤起 AI 帮写面板");
                    }
                    Err(_) => {
                        eprintln!("AI 面板尚未创建（后台服务未运行？先执行 start-service）");
                        std::process::exit(1);
                    }
                }
            }
            other => {
                eprintln!("未知动作：{other}（仅支持 show）");
                std::process::exit(2);
            }
        },
        Command::Chat { prompt } => {
            let key = std::env::var("AGNES_API_KEY")
                .unwrap_or_default()
                .trim()
                .to_owned();
            if key.is_empty() {
                eprintln!("缺少 AGNES_API_KEY（系统环境变量）。key 不落盘、不入日志。");
                std::process::exit(1);
            }
            match ai_panel::call_agnes(&key, &prompt, ai_panel::SYSTEM_PROMPT) {
                Ok(draft) => println!("{draft}"),
                Err(e) => {
                    eprintln!("请求失败：{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Pin { id } => {
            let ok = open_store().set_pinned(id as i64, true).unwrap_or(false);
            println!("{}", if ok { "已更新" } else { "条目不存在" });
        }
        Command::Unpin { id } => {
            let ok = open_store().set_pinned(id as i64, false).unwrap_or(false);
            println!("{}", if ok { "已更新" } else { "条目不存在" });
        }
        Command::Delete { id } => {
            let ok = open_store().delete(id as i64).unwrap_or(false);
            println!("{}", if ok { "已删除" } else { "条目不存在" });
        }
        Command::Copy { id } => {
            let store = open_store();
            match store.get(id as i64) {
                Ok(Some(entry)) => match paste::copy_entry_to_clipboard(&store, &entry) {
                    Ok(true) => println!("已写回剪贴板"),
                    Ok(false) => println!("条目数据缺失，无法写回"),
                    Err(e) => {
                        eprintln!("写回失败：{e}");
                        std::process::exit(1);
                    }
                },
                _ => println!("条目不存在"),
            }
        }
        Command::InstallAutostart => match install_autostart() {
            Ok(cmd) => println!("已写入开机自启（HKCU Run）：{cmd}"),
            Err(e) => {
                eprintln!("写入自启失败：{e}");
                std::process::exit(1);
            }
        },
        Command::UninstallAutostart => match uninstall_autostart() {
            Ok(()) => println!("已移除开机自启"),
            Err(e) => {
                eprintln!("移除自启失败：{e}");
                std::process::exit(1);
            }
        },
        Command::Clear => {
            let n = open_store().clear_unpinned().unwrap_or(0);
            println!("已清空 {n} 条未置顶记录");
        }
        Command::Pair { addr } => sync::cli_pair(&addr),
        Command::PairUi { addr } => sync::cli_pair_ui(&addr),
        Command::Devices => sync::cli_devices(),
        Command::Unpair { fp } => sync::cli_unpair(&fp),
        Command::Relay { value } => sync::cli_relay(&value),
        Command::DictUpdate { url } => dict_update::cli_update(&url),
        Command::Deploy => dict_update::cli_deploy(),
        Command::DictRollback { revision } => dict_update::cli_rollback(revision.as_deref()),
        Command::DictCurrent => dict_update::cli_current(),
        Command::DictHistory => dict_update::cli_history(),
        Command::Retention => {
            let n = open_store()
                .apply_retention(&RetentionPolicy::default())
                .unwrap_or(0);
            println!("清理 {n} 条过期记录");
        }
        #[cfg(debug_assertions)]
        Command::TestSetImage { width, height } => {
            let width = width.unwrap_or(41);
            let height = height.unwrap_or(29);
            if listener::request_test_image(width, height) {
                println!("已请求常驻进程写入测试图片剪贴板：{width}x{height}");
            } else {
                eprintln!("常驻进程未接受测试图片请求");
                std::process::exit(1);
            }
        }
        #[cfg(debug_assertions)]
        Command::TestInspectImage => match listener::inspect_test_image() {
            Some((width, height)) => println!("图片={width}x{height}"),
            None => {
                eprintln!("常驻进程无法读取当前位图剪贴板");
                std::process::exit(1);
            }
        },
        #[cfg(debug_assertions)]
        Command::TsfNativeProbe => match tsf_probe::run() {
            Ok(text) => println!("原生编辑控件 TSF 验收通过：{text}"),
            Err(error) => exit_with_error(&format!("原生编辑控件 TSF 验收失败：{error}")),
        },
    }
}

#[cfg(debug_assertions)]
fn exit_with_error(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "shurufa-host";

/// 开机自启：当前用户 Run 键指向本 exe 的 supervise 子命令。
/// 由 supervisor 看护 worker（崩溃自动重启、status/stop 统一管理），
/// 而不是裸 run（裸 run 崩溃无人接管）。登录时控制台会闪现一瞬，
/// 随即被 hide_own_console 隐藏。
fn install_autostart() -> Result<String, Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let cmd = format!("\"{}\" supervise", exe.display());
    windows_registry::CURRENT_USER
        .create(RUN_KEY)?
        .set_string(RUN_VALUE, &cmd)?;
    Ok(cmd)
}

fn uninstall_autostart() -> Result<(), Box<dyn std::error::Error>> {
    windows_registry::CURRENT_USER
        .create(RUN_KEY)?
        .remove_value(RUN_VALUE)?;
    Ok(())
}

/// 隐藏本进程独占的控制台窗口（start/双击启动时系统新建的那个）。
/// 从已有终端手动运行时控制台由 shell 共享，不隐藏。
fn hide_own_console() {
    use windows::Win32::System::Console::{GetConsoleProcessList, GetConsoleWindow};
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    unsafe {
        let mut pids = [0u32; 2];
        if should_hide_console(GetConsoleProcessList(&mut pids)) {
            let hwnd = GetConsoleWindow();
            if !hwnd.is_invalid() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
}

fn should_hide_console(process_count: u32) -> bool {
    process_count == 1
}

fn print_entries(entries: &[ClipEntry]) {
    if entries.is_empty() {
        println!("（无记录）");
        return;
    }
    for e in entries {
        let kind = match e.kind {
            ClipKind::Text => "文本",
            ClipKind::Image => "图片",
            ClipKind::Files => "文件",
        };
        let pin = if e.pinned { "★" } else { " " };
        let preview = match e.kind {
            ClipKind::Image => format!("<{} 字节>", e.data_size),
            _ => single_line_preview(&e.text, 48),
        };
        println!(
            "{pin}{:>5}  [{kind}] {:<10} {:>8}  {preview}",
            e.id,
            e.source_app,
            age(e.updated_at)
        );
    }
}

/// 压成单行并按字符数截断，避免多行内容打乱列表。
pub(crate) fn single_line_preview(text: &str, max_chars: usize) -> String {
    let mut line: String = text
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .take(max_chars)
        .collect();
    if text.chars().count() > max_chars {
        line.push('…');
    }
    line
}

fn age(ts_millis: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let secs = ((now - ts_millis) / 1000).max(0);
    match secs {
        0..=59 => format!("{secs}秒前"),
        60..=3599 => format!("{}分钟前", secs / 60),
        3600..=86399 => format!("{}小时前", secs / 3600),
        _ => format!("{}天前", secs / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::should_hide_console;

    #[test]
    fn 只有进程独占控制台时才隐藏() {
        assert!(should_hide_console(1));
        assert!(!should_hide_console(0));
        assert!(!should_hide_console(2));
    }
}
