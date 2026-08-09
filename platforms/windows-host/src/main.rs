#![windows_subsystem = "windows"]

//! shurufa-host：桌面常驻进程。
//!
//! `run` 子命令启动剪贴板监听并写入历史库；其余子命令面向历史库的
//! 查询与管理，供验收与后续 UI 面板复用。

mod dict_update;
mod listener;
mod panel;
mod paste;
mod supervis;
mod sync;
#[cfg(debug_assertions)]
mod tsf_probe;

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "run" => {
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
        "supervise" => {
            // 登录自启动进入 supervisor；独占控制台时立即隐藏，避免用户看到黑窗口。
            hide_own_console();
            supervis::supervise()
        }
        "status" => supervis::cmd_status(),
        "stop" => supervis::cmd_stop(),
        "list" => {
            let n = parse_arg(&args, 1).unwrap_or(20);
            print_entries(&open_store().list(n, 0).unwrap_or_default());
        }
        "search" => {
            let Some(query) = args.get(1) else {
                eprintln!("用法：shurufa-host search <关键词>");
                std::process::exit(2);
            };
            print_entries(&open_store().search(query, 50).unwrap_or_default());
        }
        "pin" | "unpin" => {
            let Some(id) = parse_arg(&args, 1) else {
                eprintln!("用法：shurufa-host {cmd} <id>");
                std::process::exit(2);
            };
            let ok = open_store()
                .set_pinned(id as i64, cmd == "pin")
                .unwrap_or(false);
            println!("{}", if ok { "已更新" } else { "条目不存在" });
        }
        "delete" => {
            let Some(id) = parse_arg(&args, 1) else {
                eprintln!("用法：shurufa-host delete <id>");
                std::process::exit(2);
            };
            let ok = open_store().delete(id as i64).unwrap_or(false);
            println!("{}", if ok { "已删除" } else { "条目不存在" });
        }
        "copy" => {
            let Some(id) = parse_arg(&args, 1) else {
                eprintln!("用法：shurufa-host copy <id>");
                std::process::exit(2);
            };
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
        "install-autostart" => match install_autostart() {
            Ok(cmd) => println!("已写入开机自启（HKCU Run）：{cmd}"),
            Err(e) => {
                eprintln!("写入自启失败：{e}");
                std::process::exit(1);
            }
        },
        "uninstall-autostart" => match uninstall_autostart() {
            Ok(()) => println!("已移除开机自启"),
            Err(e) => {
                eprintln!("移除自启失败：{e}");
                std::process::exit(1);
            }
        },
        "clear" => {
            let n = open_store().clear_unpinned().unwrap_or(0);
            println!("已清空 {n} 条未置顶记录");
        }
        "pair" => {
            let Some(addr) = args.get(1) else {
                eprintln!("用法：shurufa-host pair <对方IP[:端口]>");
                std::process::exit(2);
            };
            sync::cli_pair(addr);
        }
        "devices" => sync::cli_devices(),
        "unpair" => {
            let Some(fp) = args.get(1) else {
                eprintln!("用法：shurufa-host unpair <指纹前缀>");
                std::process::exit(2);
            };
            sync::cli_unpair(fp);
        }
        "relay" => {
            let Some(value) = args.get(1) else {
                eprintln!("用法：shurufa-host relay <中继主机:端口|off>");
                std::process::exit(2);
            };
            sync::cli_relay(value);
        }
        "dict-update" => {
            let Some(url) = args.get(1) else {
                eprintln!("用法：shurufa-host dict-update <HTTPS 词库清单地址>");
                std::process::exit(2);
            };
            dict_update::cli_update(url);
        }
        "dict-rollback" => dict_update::cli_rollback(),
        "dict-current" => dict_update::cli_current(),
        "retention" => {
            let n = open_store()
                .apply_retention(&RetentionPolicy::default())
                .unwrap_or(0);
            println!("清理 {n} 条过期记录");
        }
        #[cfg(debug_assertions)]
        "test-set-image" => {
            let width = parse_arg(&args, 1).unwrap_or(41);
            let height = parse_arg(&args, 2).unwrap_or(29);
            if listener::request_test_image(width, height) {
                println!("已请求常驻进程写入测试图片剪贴板：{width}x{height}");
            } else {
                eprintln!("常驻进程未接受测试图片请求");
                std::process::exit(1);
            }
        }
        #[cfg(debug_assertions)]
        "test-inspect-image" => match listener::inspect_test_image() {
            Some((width, height)) => println!("图片={width}x{height}"),
            None => {
                eprintln!("常驻进程无法读取当前位图剪贴板");
                std::process::exit(1);
            }
        },
        #[cfg(debug_assertions)]
        "tsf-native-probe" => match tsf_probe::run() {
            Ok(text) => println!("原生编辑控件 TSF 验收通过：{text}"),
            Err(error) => exit_with_error(&format!("原生编辑控件 TSF 验收失败：{error}")),
        },
        _ => {
            println!(
                "用法：shurufa-host <子命令>\n\
                 \x20 run             启动剪贴板监听（常驻 worker）\n\
                 \x20 supervise       常驻监管：看护 worker，崩溃自动重启\n\
                 \x20 status          查看监管与运行状态\n\
                 \x20 stop            停止 supervisor\n\
                 \x20 list [N]        最近 N 条历史（默认 20）\n\
                 \x20 search <关键词>  搜索文本与文件名\n\
                 \x20 pin/unpin <id>  置顶/取消置顶\n\
                 \x20 copy <id>       把条目写回剪贴板\n\
                 \x20 delete <id>     删除单条\n\
                 \x20 clear           清空未置顶记录\n\
                 \x20 retention       立即执行留存清理
                 \x20 relay <地址|off> 配置或关闭自托管同步中继
                 \x20 dict-update <HTTPS地址> 更新自托管云词库
                 \x20 dict-rollback   回滚到上次更新前的词库
                 \x20 dict-current    打印当前词库版本"
            );
        }
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

fn parse_arg(args: &[String], idx: usize) -> Option<u32> {
    args.get(idx)?.parse().ok()
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
fn single_line_preview(text: &str, max_chars: usize) -> String {
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
