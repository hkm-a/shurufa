//! shurufa-ctl：一次性 CLI（阶段4第4项拆分）。
//!
//! 历史库查询管理、写回剪贴板、配对与词库维护。不常驻、无窗口依赖；
//! `copy` 的剪贴板写回经 clipd 的监听窗口（按类名跨进程 SendMessage）。

use clap::{Parser, Subcommand};
use clipboard_store::ClipboardStore;
use shurufa_host::{open_store, print_entries};
use update_core::{should_update, UpdateManifest};

/// 下载文件到本地，边下边在 stderr 显示进度百分比；返回 SHA256（小写 hex）。
fn download_with_progress(url: &str, out: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("下载失败：{e}"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(out).map_err(|e| format!("创建本地文件失败：{e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    let mut done: u64 = 0;
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        done += n as u64;
        if let Some(t) = total {
            if let Some(pct) = done.saturating_mul(100).checked_div(t) {
                eprint!("\r下载进度：{}%", pct.min(100));
            }
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| format!("写入文件失败：{e}"))?;
    }
    if total.is_some() {
        eprintln!();
    }
    file.flush().map_err(|e| format!("flush 失败：{e}"))?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

#[derive(Parser)]
#[command(name = "shurufa-ctl", about = "Shurufa CLI：历史库/配对/词库管理")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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
    /// 唤起 AI 帮写面板（shurufa-ui 常驻时）
    Ai { action: String },
    /// 置顶
    Pin { id: u32 },
    /// 取消置顶
    Unpin { id: u32 },
    /// 删除单条
    Delete { id: u32 },
    /// 把条目写回剪贴板
    Copy { id: u32 },
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
    /// 检查更新（拉取 update.json + 灰度判定）
    #[command(name = "check-update")]
    CheckUpdate {
        /// update.json 地址
        #[arg(long)]
        url: String,
        /// 渠道：stable / canary / beta
        #[arg(long, default_value = "stable")]
        channel: String,
        /// 机器标识（默认取主机名）
        #[arg(long)]
        machine_id: Option<String>,
        /// 当前版本（默认读 version.json，读不到用 0.0.0）
        #[arg(long)]
        current_version: Option<String>,
    },
    /// 下载安装包、校验 SHA256 并启动安装
    #[command(name = "update-apply")]
    UpdateApply {
        /// 安装包下载地址
        #[arg(long)]
        url: String,
        /// 期望 SHA256（小写 hex）；为空则跳过校验
        #[arg(long)]
        sha256: Option<String>,
        /// 下载到本地路径；默认 %TEMP%\shurufa-update\update.exe
        #[arg(long)]
        out: Option<String>,
    },
    /// 一键自动更新：灰度判断 + 下载 + 校验 + 启动安装
    Update {
        /// update.json 地址
        #[arg(long)]
        url: String,
        /// 渠道：stable / canary / beta
        #[arg(long, default_value = "stable")]
        channel: String,
        /// 机器标识（默认取主机名）
        #[arg(long)]
        machine_id: Option<String>,
        /// 当前版本（默认读 version.json）
        #[arg(long)]
        current_version: Option<String>,
        /// 只检查不安装
        #[arg(long)]
        check_only: bool,
        /// 静默安装（NSIS /S）
        #[arg(long)]
        silent: bool,
        /// 下载到本地路径（默认 %TEMP%\shurufa-update\update.exe）
        #[arg(long)]
        out: Option<String>,
    },
    /// 立即执行留存清理
    Retention,
    #[cfg(debug_assertions)]
    #[command(name = "tsf-native-probe")]
    TsfNativeProbe,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
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
            shurufa_host::sync::cli_remote_search(&query);
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
                                shurufa_host::ai_panel::WM_AI_EXTERNAL_SHOW,
                                WPARAM(0),
                                LPARAM(0),
                            )
                        };
                        println!("已唤起 AI 帮写面板");
                    }
                    Err(_) => {
                        eprintln!("AI 面板尚未创建（shurufa-ui 未运行？）");
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
            match shurufa_host::ai_panel::call_agnes(
                &key,
                &prompt,
                shurufa_host::ai_panel::SYSTEM_PROMPT,
            ) {
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
            let store: ClipboardStore = open_store();
            match store.get(id as i64) {
                Ok(Some(entry)) => {
                    match shurufa_host::paste::copy_entry_to_clipboard(&store, &entry) {
                        Ok(true) => println!("已写回剪贴板"),
                        Ok(false) => println!("条目数据缺失，无法写回"),
                        Err(e) => {
                            eprintln!("写回失败：{e}");
                            std::process::exit(1);
                        }
                    }
                }
                _ => println!("条目不存在"),
            }
        }
        Command::Clear => {
            let n = open_store().clear_unpinned().unwrap_or(0);
            println!("已清空 {n} 条未置顶记录");
        }
        Command::Pair { addr } => shurufa_host::sync::cli_pair(&addr),
        Command::PairUi { addr } => shurufa_host::sync::cli_pair_ui(&addr),
        Command::Devices => shurufa_host::sync::cli_devices(),
        Command::Unpair { fp } => shurufa_host::sync::cli_unpair(&fp),
        Command::Relay { value } => shurufa_host::sync::cli_relay(&value),
        Command::DictUpdate { url } => shurufa_host::dict_update::cli_update(&url),
        Command::Deploy => shurufa_host::dict_update::cli_deploy(),
        Command::DictRollback { revision } => {
            shurufa_host::dict_update::cli_rollback(revision.as_deref())
        }
        Command::DictCurrent => shurufa_host::dict_update::cli_current(),
        Command::DictHistory => shurufa_host::dict_update::cli_history(),
        Command::CheckUpdate {
            url,
            channel,
            machine_id,
            current_version,
        } => {
            let machine_id = machine_id.unwrap_or_else(|| {
                std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown-machine".to_owned())
            });
            let current_version = current_version.unwrap_or_else(|| {
                std::fs::read_to_string("version.json")
                    .ok()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from))
                    .unwrap_or_else(|| "0.0.0".to_owned())
            });
            println!("检查更新：channel={channel} machine={machine_id} current={current_version}");
            let body = match ureq::get(&url).call() {
                Ok(resp) => match resp.into_string() {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("读取 update.json 失败：{e}");
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("拉取 update.json 失败：{e}");
                    std::process::exit(1);
                }
            };
            let manifest = match UpdateManifest::from_json(&body) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("解析 update.json 失败：{e}");
                    std::process::exit(1);
                }
            };
            let Some(info) = manifest.channels.get(&channel) else {
                eprintln!("渠道不存在：{channel}");
                std::process::exit(1);
            };
            let update = should_update(&manifest, &channel, &machine_id, &current_version);
            println!("目标版本：{}", info.version);
            println!("灰度比例：{}%", info.rollout_percent);
            println!("是否更新：{}", if update { "是" } else { "否" });
            if update {
                println!("下载地址：{}", info.url);
                if !info.sha256.is_empty() {
                    println!("SHA256：{}", info.sha256);
                }
            }
            std::process::exit(if update { 0 } else { 2 });
        }
        Command::UpdateApply { url, sha256, out } => {
            let out = out.unwrap_or_else(|| {
                let dir = std::env::temp_dir().join("shurufa-update");
                std::fs::create_dir_all(&dir).expect("创建更新目录失败");
                dir.join("update.exe").to_string_lossy().to_string()
            });
            println!("下载：{url}");
            let actual = match download_with_progress(&url, &out) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            println!("已下载：{out}");
            println!("SHA256：{actual}");
            if let Some(expected) = sha256 {
                if !expected.eq_ignore_ascii_case(&actual) {
                    eprintln!("SHA256 不匹配：期望 {expected}");
                    std::process::exit(1);
                }
                println!("SHA256 校验通过");
            }
            println!("启动安装器…");
            let _ = std::process::Command::new(&out).spawn();
            std::process::exit(0);
        }
        Command::Update {
            url,
            channel,
            machine_id,
            current_version,
            check_only,
            silent,
            out,
        } => {
            let machine_id = machine_id.unwrap_or_else(|| {
                std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown-machine".to_owned())
            });
            let current_version = current_version.unwrap_or_else(|| {
                std::fs::read_to_string("version.json")
                    .ok()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from))
                    .unwrap_or_else(|| "0.0.0".to_owned())
            });
            println!("自动更新：channel={channel} machine={machine_id} current={current_version}");
            let body = match ureq::get(&url).call() {
                Ok(resp) => match resp.into_string() {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("读取 update.json 失败：{e}");
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("拉取 update.json 失败：{e}");
                    std::process::exit(1);
                }
            };
            let manifest = match UpdateManifest::from_json(&body) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("解析 update.json 失败：{e}");
                    std::process::exit(1);
                }
            };
            let Some(info) = manifest.channels.get(&channel) else {
                eprintln!("渠道不存在：{channel}");
                std::process::exit(1);
            };
            let update = should_update(&manifest, &channel, &machine_id, &current_version);
            println!("目标版本：{}", info.version);
            println!("灰度比例：{}%", info.rollout_percent);
            if !update {
                println!("当前无需更新");
                std::process::exit(2);
            }
            if check_only {
                println!("需要更新：{}", info.url);
                std::process::exit(0);
            }
            let out = out.unwrap_or_else(|| {
                let dir = std::env::temp_dir().join("shurufa-update");
                std::fs::create_dir_all(&dir).expect("创建更新目录失败");
                dir.join("update.exe").to_string_lossy().to_string()
            });
            println!("下载：{}", info.url);
            let actual = match download_with_progress(&info.url, &out) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            println!("已下载：{out}");
            println!("SHA256：{actual}");
            if !info.sha256.is_empty() && !info.sha256.eq_ignore_ascii_case(&actual) {
                eprintln!("SHA256 不匹配：期望 {}", info.sha256);
                std::process::exit(1);
            }
            if !info.sha256.is_empty() {
                println!("SHA256 校验通过");
            }
            let mut cmd = std::process::Command::new(&out);
            if silent {
                cmd.arg("/S");
            }
            println!("启动安装器{}…", if silent { "（静默）" } else { "" });
            let _ = cmd.spawn();
            std::process::exit(0);
        }
        Command::Retention => shurufa_host::apply_retention_now(),
        #[cfg(debug_assertions)]
        Command::TsfNativeProbe => match shurufa_host::tsf_probe::run() {
            Ok(text) => println!("原生编辑控件 TSF 验收通过：{text}"),
            Err(error) => {
                eprintln!("原生编辑控件 TSF 验收失败：{error}");
                std::process::exit(1);
            }
        },
    }
}
