//! shurufa-host：桌面常驻进程。
//!
//! `run` 子命令启动剪贴板监听并写入历史库；其余子命令面向历史库的
//! 查询与管理，供验收与后续 UI 面板复用。

mod listener;
mod panel;
mod paste;

use clipboard_store::{ClipEntry, ClipKind, ClipboardStore, RetentionPolicy};
use std::path::PathBuf;

/// 文件日志：常驻进程以最小化/无窗口方式运行，控制台输出不可见，
/// 排障信息统一落 %TEMP%\shurufa-host.log，失败静默。
pub fn log_line(msg: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("shurufa-host.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = f.write_all(format!("[{ts}] {msg}
").as_bytes());
    }
}

fn db_path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
        .join("clipboard.db")
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
            std::panic::set_hook(Box::new(|info| {
                log_line(&format!("PANIC：{info}"));
            }));
            hide_own_console();
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
        "retention" => {
            let n = open_store()
                .apply_retention(&RetentionPolicy::default())
                .unwrap_or(0);
            println!("清理 {n} 条过期记录");
        }
        _ => {
            println!(
                "用法：shurufa-host <子命令>\n\
                 \x20 run             启动剪贴板监听（常驻）\n\
                 \x20 list [N]        最近 N 条历史（默认 20）\n\
                 \x20 search <关键词>  搜索文本与文件名\n\
                 \x20 pin/unpin <id>  置顶/取消置顶\n\
                 \x20 copy <id>       把条目写回剪贴板\n\
                 \x20 delete <id>     删除单条\n\
                 \x20 clear           清空未置顶记录\n\
                 \x20 retention       立即执行留存清理"
            );
        }
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "shurufa-host";

/// 开机自启：当前用户 Run 键指向本 exe 的 run 子命令。
/// 登录时控制台会闪现一瞬，随即被 hide_own_console 隐藏。
fn install_autostart() -> Result<String, Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let cmd = format!("\"{}\" run", exe.display());
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
        if GetConsoleProcessList(&mut pids) == 1 {
            let hwnd = GetConsoleWindow();
            if !hwnd.is_invalid() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
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
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
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
