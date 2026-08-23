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
