//! Shurufa Tauri 控制中心后端。
//!
//! 前端只负责界面与交互；中继配置、后台宿主和词库更新仍沿用既有 Rust 模块与
//! 同目录可执行文件，避免为桌面 UI 再建立平行的业务实现。

#![cfg(windows)]
#![windows_subsystem = "windows"]

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Serialize)]
struct DashboardState {
    relay: String,
    service_status: String,
    data_directory: String,
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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            dashboard_state,
            save_relay,
            start_service,
            stop_service,
            update_dictionary,
            open_system_settings,
            open_data_directory
        ])
        .run(tauri::generate_context!())
        .expect("启动 Shurufa 控制中心失败");
}

#[cfg(test)]
mod tests {
    use super::sync_dir;

    #[test]
    fn 默认同步目录位于应用数据目录下() {
        assert!(sync_dir().ends_with("shurufa\\sync"));
    }
}
