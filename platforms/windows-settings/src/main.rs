//! Shurufa Tauri 控制中心后端。
//!
//! 前端只负责界面与交互；中继配置、后台宿主和词库更新仍沿用既有 Rust 模块与
//! 同目录可执行文件，避免为桌面 UI 再建立平行的业务实现。

#![cfg(windows)]
#![windows_subsystem = "windows"]

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use clipboard_store::{ClipEntry, ClipKind, ClipboardStore};
use serde::Serialize;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Serialize)]
struct DashboardState {
    relay: String,
    service_status: String,
    data_directory: String,
}

#[derive(Serialize)]
struct HistoryEntry {
    id: i64,
    kind: String,
    text: String,
    source_app: String,
    updated_at: i64,
    pinned: bool,
}

fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
}

fn sync_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("SHURUFA_SYNC_DIR") {
        PathBuf::from(path)
    } else {
        app_data_dir().join("sync")
    }
}

fn history_db_path() -> PathBuf {
    std::env::var_os("SHURUFA_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| app_data_dir().join("clipboard.db"))
}

fn open_history_store() -> Result<ClipboardStore, String> {
    ClipboardStore::open(&history_db_path()).map_err(|error| format!("打开剪贴板历史失败：{error}"))
}

fn history_entry(entry: ClipEntry) -> HistoryEntry {
    let kind = match entry.kind {
        ClipKind::Text => "文本",
        ClipKind::Image => "图片",
        ClipKind::Files => "文件",
    };
    let text = match entry.kind {
        ClipKind::Image => format!("图片（{} KB）", (entry.data_size.max(0) + 1023) / 1024),
        ClipKind::Files => entry.text.lines().next().unwrap_or("文件").to_owned(),
        ClipKind::Text => entry.text,
    };
    HistoryEntry {
        id: entry.id,
        kind: kind.to_owned(),
        text,
        source_app: entry.source_app,
        updated_at: entry.updated_at,
        pinned: entry.pinned,
    }
}

fn sibling_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn launch_host(args: &[&str]) -> Result<(), String> {
    Command::new(sibling_exe("shurufa-host.exe"))
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动后台宿主：{error}"))
}

fn read_service_status() -> String {
    let state = std::fs::read_to_string(app_data_dir().join("daemon.state")).unwrap_or_default();
    if state.contains("status=running") {
        "运行中".to_owned()
    } else if state.contains("status=restarting") {
        "正在恢复".to_owned()
    } else {
        "待启动".to_owned()
    }
}

#[tauri::command]
fn dashboard_state() -> DashboardState {
    DashboardState {
        relay: sync_core::load_relay_addr(&sync_dir()).unwrap_or_default(),
        service_status: read_service_status(),
        data_directory: app_data_dir().display().to_string(),
    }
}

#[tauri::command]
fn save_relay(relay: String) -> Result<(), String> {
    let relay = (!relay.trim().is_empty()).then_some(relay.trim());
    sync_core::save_relay_addr(&sync_dir(), relay)
        .map_err(|error| format!("保存中继设置失败：{error}"))
}

#[tauri::command]
fn start_service() -> Result<(), String> {
    launch_host(&["supervise"])
}

#[tauri::command]
fn stop_service() -> Result<(), String> {
    launch_host(&["stop"])
}

#[tauri::command]
fn update_dictionary() -> Result<(), String> {
    launch_host(&["dict-update", "rime-ice"])
}

#[tauri::command]
fn open_system_settings() -> Result<(), String> {
    Command::new("cmd.exe")
        .args(["/c", "start", "", "ms-settings:regionlanguage"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("打开系统设置失败：{error}"))
}

#[tauri::command]
fn open_data_directory() -> Result<(), String> {
    let directory = app_data_dir();
    std::fs::create_dir_all(&directory).map_err(|error| format!("创建数据目录失败：{error}"))?;
    Command::new("explorer.exe")
        .arg(directory)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("打开数据目录失败：{error}"))
}

#[tauri::command]
fn history_entries() -> Result<Vec<HistoryEntry>, String> {
    open_history_store()?
        .list(80, 0)
        .map(|entries| entries.into_iter().map(history_entry).collect())
        .map_err(|error| format!("读取剪贴板历史失败：{error}"))
}

#[tauri::command]
fn copy_history(id: i64) -> Result<(), String> {
    let id = id.to_string();
    launch_host(&["copy", &id])
}

#[tauri::command]
fn set_history_pinned(id: i64, pinned: bool) -> Result<(), String> {
    let updated = open_history_store()?
        .set_pinned(id, pinned)
        .map_err(|error| format!("更新置顶状态失败：{error}"))?;
    if updated {
        Ok(())
    } else {
        Err("该历史条目已不存在".to_owned())
    }
}

#[tauri::command]
fn delete_history(id: i64) -> Result<(), String> {
    let deleted = open_history_store()?
        .delete(id)
        .map_err(|error| format!("删除历史条目失败：{error}"))?;
    if deleted {
        Ok(())
    } else {
        Err("该历史条目已不存在".to_owned())
    }
}

#[tauri::command]
fn clear_unpinned_history() -> Result<usize, String> {
    open_history_store()?
        .clear_unpinned()
        .map_err(|error| format!("清空未置顶历史失败：{error}"))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            dashboard_state,
            save_relay,
            start_service,
            stop_service,
            update_dictionary,
            open_system_settings,
            open_data_directory,
            history_entries,
            copy_history,
            set_history_pinned,
            delete_history,
            clear_unpinned_history
        ])
        .run(tauri::generate_context!())
        .expect("启动 Shurufa 控制中心失败");
}

#[cfg(test)]
mod tests {
    use super::{history_entry, sync_dir};
    use clipboard_store::{ClipEntry, ClipKind};

    #[test]
    fn 默认同步目录位于应用数据目录下() {
        assert!(sync_dir().ends_with("shurufa\\sync"));
    }

    #[test]
    fn 图片历史显示数据大小而非空文本() {
        let entry = history_entry(ClipEntry {
            id: 1,
            kind: ClipKind::Image,
            text: String::new(),
            source_app: "pixpin.exe".to_owned(),
            created_at: 0,
            updated_at: 1,
            use_count: 1,
            pinned: false,
            data_size: 1025,
        });
        assert_eq!(entry.kind, "图片");
        assert_eq!(entry.text, "图片（2 KB）");
    }
}
