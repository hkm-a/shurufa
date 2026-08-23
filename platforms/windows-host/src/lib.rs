//! shurufa-host 库：三个故障域二进制的共享实现（阶段4第4项拆分）。
//!
//! - `shurufa-clipd`（数据路径）：剪贴板监听入库、同步 daemon、supervisor。
//! - `shurufa-ui`（面板集合）：历史/AI/语音面板与全部热键，崩了不影响数据路径。
//! - `shurufa-ctl`（CLI）：历史库查询管理、配对、词库维护的一次性命令。

pub mod ai_panel;
pub mod asr;
pub mod audio_capture;
pub mod cand_host;
pub mod cand_uia;
pub mod dict_update;
pub mod listener;
pub mod panel;
pub mod paste;
pub mod speech;
pub mod supervis;
pub mod sync;
#[cfg(debug_assertions)]
pub mod tsf_probe;
pub mod update_check;

use clipboard_store::{ClipEntry, ClipKind, ClipboardStore, RetentionPolicy};
use std::path::PathBuf;

/// 初始化 tracing 文件日志：常驻进程以最小化/无窗口方式运行，控制台输出
/// 不可见，排障信息统一落 %TEMP%\shurufa-host.log（可用 SHURUFA_LOG_PATH 覆盖）。
/// 由常驻子命令在产生日志前调用。
pub fn init_logging() {
    let path = std::env::var_os("SHURUFA_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("shurufa-host.log"));
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        // subscriber 持有 Mutex<File>，文件句柄随全局 subscriber 存活。
        let writer = std::sync::Mutex::new(file);
        let _ = tracing_subscriber::fmt()
            .with_writer(writer)
            .with_ansi(false)
            .try_init();
    }
}

/// 日志入口：保持既有 `crate::log_line(...)` 调用点不变，实际写入交给 tracing。
pub fn log_line(msg: &str) {
    tracing::info!("{msg}");
}

/// shurufa 应用数据目录：%APPDATA%\shurufa（无 APPDATA 时回退临时目录）。
pub fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
}

pub fn db_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SHURUFA_DB_PATH") {
        return PathBuf::from(path);
    }
    app_data_dir().join("clipboard.db")
}

pub fn open_store() -> ClipboardStore {
    let path = db_path();
    match ClipboardStore::open(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("打开历史库失败 {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "shurufa-host";

/// 开机自启：当前用户 Run 键指向 clipd exe 的 supervise 子命令。
/// 由 supervisor 看护 worker（崩溃自动重启、status/stop 统一管理），
/// 而不是裸 run（裸 run 崩溃无人接管）。登录时控制台会闪现一瞬，
/// 随即被 hide_own_console 隐藏。
pub fn install_autostart() -> Result<String, Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let cmd = format!("\"{}\" supervise", exe.display());
    windows_registry::CURRENT_USER
        .create(RUN_KEY)?
        .set_string(RUN_VALUE, &cmd)?;
    Ok(cmd)
}

pub fn uninstall_autostart() -> Result<(), Box<dyn std::error::Error>> {
    windows_registry::CURRENT_USER
        .create(RUN_KEY)?
        .remove_value(RUN_VALUE)?;
    Ok(())
}

/// 隐藏本进程独占的控制台窗口（start/双击启动时系统新建的那个）。
/// 从已有终端手动运行时控制台由 shell 共享，不隐藏。
pub fn hide_own_console() {
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

pub fn print_entries(entries: &[ClipEntry]) {
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
pub fn single_line_preview(text: &str, max_chars: usize) -> String {
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

pub fn apply_retention_now() {
    let n = open_store()
        .apply_retention(&RetentionPolicy::default())
        .unwrap_or(0);
    println!("清理 {n} 条过期记录");
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
