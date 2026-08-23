//! 后台更新检查：由 shurufa-ui 定时触发，命中灰度后写状态文件。
//!
//! 不弹窗、不自动安装；控制中心可读取 `update-available.json` 展示。
//! 清单地址通过环境变量 `SHURUFA_UPDATE_MANIFEST` 提供，未配置则跳过。

use std::path::PathBuf;
use std::process::Command;

use crate::log_line;

fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
}

fn read_channel() -> String {
    let path = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
        .join("channel.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("channel").and_then(|c| c.as_str()).map(String::from))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "stable".to_owned())
}

/// 用系统托盘气球通知提示有更新（不需要常驻托盘图标，10s 后自动删除）。
fn notify_update_available(detail: &str) {
    use windows::core::w;
        use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NOTIFYICONDATAW, NOTIFY_ICON_DATA_FLAGS, NOTIFY_ICON_INFOTIP_FLAGS, NIF_INFO,
        NIM_ADD, NIM_DELETE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, LoadIconW, IDI_APPLICATION};

    unsafe {
        let Ok(hwnd) = FindWindowW(w!("ShurufaUiHost"), None) else {
            return;
        };
        let icon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
        let title: Vec<u16> = "FOX输入法更新".encode_utf16().chain(std::iter::once(0)).collect();
        let msg: Vec<u16> = detail.encode_utf16().chain(std::iter::once(0)).collect();
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_INFO | NOTIFY_ICON_DATA_FLAGS(0);
        nid.hIcon = icon;
        nid.dwInfoFlags = NOTIFY_ICON_INFOTIP_FLAGS(1); // NIIF_INFO
        let ti = title.as_ptr();
        let mi = msg.as_ptr();
        std::ptr::copy_nonoverlapping(ti, nid.szInfoTitle.as_mut_ptr(), title.len().min(nid.szInfoTitle.len()));
        std::ptr::copy_nonoverlapping(mi, nid.szInfo.as_mut_ptr(), msg.len().min(nid.szInfo.len()));
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        // 10s 后移除临时托盘图标（NOTIFYICONDATAW 不是 Send，这里只传 usize）
        let raw_hwnd = hwnd.0 as usize;
        let uid = nid.uID;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            {
                use windows::Win32::Foundation::HWND;
                let mut del: NOTIFYICONDATAW = std::mem::zeroed();
                del.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                del.hWnd = HWND(raw_hwnd as *mut _);
                del.uID = uid;
                let _ = Shell_NotifyIconW(NIM_DELETE, &del);
            }
        });
    }
}

pub fn run_once() {
    let Some(manifest) = std::env::var_os("SHURUFA_UPDATE_MANIFEST") else {
        return;
    };
    let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) else {
        return;
    };
    let ctl = exe_dir.join("shurufa-ctl.exe");
    if !ctl.exists() {
        log_line("update_check: 未找到 shurufa-ctl.exe，跳过");
        return;
    }
    let channel = read_channel();
    log_line(&format!(
        "update_check: 检查更新 manifest={} channel={channel}",
        manifest.to_string_lossy()
    ));
    let output = match Command::new(&ctl)
        .args(["update", "--url"])
        .arg(&manifest)
        .args(["--channel", &channel, "--check-only"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log_line(&format!("update_check: 执行失败：{e}"));
            return;
        }
    };
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() {
        let dir = app_data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let state = serde_json::json!({
            "available": true,
            "checked_at": chrono_now_rfc3339(),
            "channel": channel,
            "detail": text,
        });
        let _ = std::fs::write(dir.join("update-available.json"), serde_json::to_string_pretty(&state).unwrap_or_default());
        log_line(&format!("update_check: 发现更新\n{text}"));
        notify_update_available(&text);
    } else if output.status.code() == Some(2) {
        let _ = std::fs::remove_file(app_data_dir().join("update-available.json"));
        log_line("update_check: 当前无需更新");
    } else {
        log_line(&format!(
            "update_check: 检查失败 exit={:?} {} {}",
            output.status.code(),
            text,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
}

/// 无 chrono 依赖的 RFC3339 近似时间（UTC，秒级）。
fn chrono_now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 简单可读：epoch 秒（控制中心可再格式化）
    format!("{secs}")
}
