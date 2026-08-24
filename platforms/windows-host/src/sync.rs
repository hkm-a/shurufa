//! 桌面端同步接入：常驻线程运行 SyncService，接收条目入历史库，
//! 本机剪贴板文本经全局句柄广播给已配对设备。
//!
//! 入站配对确认用 MessageBox（守护进程无控制台）；`pair` 子命令
//! 在独立进程内用控制台交互发起配对，写入共享 peers.json 后由
//! 守护进程的重连循环自动接管连接。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use shurufa_options::{SyncActivityKind, SyncDirection};
use sync_core::{ConfirmFn, Incoming, PairPrompt, SyncConfig, SyncService, MAX_CLIP_IMAGE_BYTES};

/// 守护进程内广播的内容：文本或图片。
enum Broadcast {
    Text(String),
    Image(Vec<u8>),
    File {
        name: String,
        mime_type: String,
        data: Vec<u8>,
    },
    /// v3 文件：整路径交由 SyncService 内部分块、跟踪 ACK/Progress。
    FileV3(std::path::PathBuf),
    /// 配置/短语/皮肤同步（config-sync-v1）。
    Config {
        kind: String,
        name: String,
        data: String,
    },
}

/// 守护进程内广播出口；`run` 模式启动后可用
static CLIP_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<Broadcast>> = OnceLock::new();

/// 常驻同步服务实例（Clone 句柄），供配置增量 watcher 判断是否有在线对端。
static SYNC_SERVICE: OnceLock<SyncService> = OnceLock::new();

/// M10：发送中文件台账 msg_id → 原路径。FileTransferDone 失败时取回路径
/// 生成 SendFile 重试载荷（补 M8-1b 缺口：msg_id→原路径映射）。
static PENDING_FILE_SENDS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

fn pending_file_sends() -> &'static Mutex<HashMap<String, PathBuf>> {
    PENDING_FILE_SENDS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn sync_config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SHURUFA_SYNC_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
        .join("sync")
}

/// 守护进程监听端口，可经 SHURUFA_SYNC_PORT 覆盖（默认 48632）。
pub fn sync_port() -> u16 {
    std::env::var("SHURUFA_SYNC_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48632)
}

pub fn device_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows 设备".into())
}

/// 监听器捕获到本机文本时调用；服务未启动或无连接时静默。
pub fn broadcast_text(text: &str) {
    if let Some(tx) = CLIP_TX.get() {
        let _ = tx.send(Broadcast::Text(text.to_string()));
    }
}

/// 监听器捕获到本机图片时调用；BMP 转 PNG 后推送已配对设备。
pub fn broadcast_image(bmp: &[u8]) {
    if let Some(tx) = CLIP_TX.get() {
        if let Some((png, resized)) = bmp_to_sync_png(bmp) {
            if resized {
                crate::log_line("图片超过手机同步帧上限，已生成缩小副本发送");
            }
            let _ = tx.send(Broadcast::Image(png));
            crate::log_line("已将图片加入同步发送队列");
        } else {
            crate::log_line("图片编码为 PNG 失败，未加入同步发送队列");
        }
    } else {
        crate::log_line("同步发送队列尚未初始化，图片未发送");
    }
}

/// 监听器捕获到本机文件时调用；单文件在同步上限内才会发送。
pub fn broadcast_file(path: &std::path::Path) {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let Ok(data) = std::fs::read(path) else {
        return;
    };
    let mime_type = mime_type_for(path);
    if let Some(tx) = CLIP_TX.get() {
        let _ = tx.send(Broadcast::File {
            name: name.to_string(),
            mime_type,
            data,
        });
    }
}

/// 以 v3 文件路径方式向所有 file-v1 对端广播一个本机文件。
///
/// 这是同步 v3 的对外入口；面板层调它即可。同步线程内由 `Broadcast::FileV3`
/// 匹配 `SyncService::send_file_path` 驱动 Offer/Chunk/Done/Ack 状态机。
/// 对端不支持 file-v1 时该调用会自然走 v2 `ClipFile` 单帧广播，由
/// `SyncService::send_file_path` 接管。
///
/// 当前宿主 UI（panel.rs / ai_panel.rs）尚无专门的「以文件形式转发」右
/// 键菜单 hook，本函数仅暴露能力；需要接入 UI 时由对应面板调用层补一行。
pub fn send_file_to_all(path: &std::path::Path) {
    let path_buf = path.to_path_buf();
    if let Some(tx) = CLIP_TX.get() {
        let _ = tx.send(Broadcast::FileV3(path_buf));
    }
}

/// 把一份配置/短语/皮肤文本广播给所有已配对设备（config-sync-v1）。
pub fn broadcast_config(kind: &str, name: &str, data: &str) {
    if let Some(tx) = CLIP_TX.get() {
        let _ = tx.send(Broadcast::Config {
            kind: kind.to_string(),
            name: name.to_string(),
            data: data.to_string(),
        });
    }
}

/// 重试载荷目录：`app_dir()/sync-retry/`（每活动一个 JSON，键=retry_id）。
fn retry_payload_dir() -> PathBuf {
    shurufa_options::app_dir().join("sync-retry")
}

/// 为可重试失败保存载荷，返回 retry_id；载荷超限等不可重试场景返回 None。
fn save_retry_payload(kind: &str, value: &serde_json::Value) -> Option<String> {
    let id = format!(
        "r{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let dir = retry_payload_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let payload = serde_json::json!({ "kind": kind, "value": value });
    let bytes = serde_json::to_vec_pretty(&payload).ok()?;
    std::fs::write(dir.join(format!("{id}.json")), bytes).ok()?;
    Some(id)
}

/// 记录一条跨设备同步活动（M8-1：同步状态可视化/来源标签；M8-1b：失败重试句柄）。
/// 失败静默：活动记录本身不影响同步主流程。
fn record_sync_activity(
    direction: SyncDirection,
    kind: SyncActivityKind,
    preview: String,
    peer: Option<String>,
    ok: bool,
    detail: Option<String>,
    retry_id: Option<String>,
) {
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let _ = shurufa_options::sync_activity::record(shurufa_options::SyncActivityEntry {
        id: 0,
        direction,
        kind,
        preview,
        peer,
        status: if ok {
            "ok".to_owned()
        } else {
            "failed".to_owned()
        },
        detail,
        retry_id,
        ts_ms,
    });
}

/// M8-1b：失败重试执行器。设置中心写入 `app_dir()/sync-retry-request.json`
/// （{"id": <活动 id>}）后，本线程 2s 轮询处理：找回原活动与载荷 →
/// 重放（文本/图片/文件写剪贴板）→ 记录新活动 → 清理请求与载荷。
fn spawn_sync_retry_watcher() {
    std::thread::Builder::new()
        .name("sync-retry".into())
        .spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let request = shurufa_options::app_dir().join("sync-retry-request.json");
            let Ok(raw) = std::fs::read_to_string(&request) else {
                continue;
            };
            // 无论成败都先清除请求，避免同一文件被永久轮询
            let _ = std::fs::remove_file(&request);
            let Ok(req) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            let Some(id) = req.get("id").and_then(serde_json::Value::as_u64) else {
                continue;
            };
            execute_sync_retry(id);
        })
        .ok();
}

/// P2 配置同步请求 watcher：`shurufa-ctl sync-config` 只写请求文件，
/// 由常驻同步守护进程读取后用自己的活跃连接广播，避免临时实例
/// 与守护进程同指纹去重导致连接被对端拒绝。
fn spawn_sync_config_watcher() {
    std::thread::Builder::new()
        .name("sync-config".into())
        .spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let request = shurufa_options::app_dir().join("sync-config-request.json");
            let Ok(raw) = std::fs::read_to_string(&request) else {
                continue;
            };
            let _ = std::fs::remove_file(&request);
            let Ok(req) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            let Some(kind) = req.get("kind").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(path) = req.get("path").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let dir = crate::app_data_dir();
            let data = match sync_core::config_sync::prepare_send(
                &dir,
                kind,
                std::path::Path::new(path),
            ) {
                Ok(Some(data)) => data,
                Ok(None) => {
                    crate::log_line(&format!(
                        "配置同步：{kind} 内容自上次同步后未变化，跳过增量发送"
                    ));
                    continue;
                }
                Err(e) => {
                    crate::log_line(&format!("配置同步：读取 {} 失败，已跳过（{e}）", path));
                    continue;
                }
            };
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("config.txt")
                .to_owned();
            crate::log_line(&format!(
                "配置同步：收到请求，广播 {kind}/{name}（{} 字符）",
                data.chars().count()
            ));
            broadcast_config(kind, &name, &data);
            let has_peer = SYNC_SERVICE
                .get()
                .map(|service| service.connected_count() > 0)
                .unwrap_or(false);
            if has_peer {
                if let Err(e) = sync_core::config_sync::mark_sent(&dir, kind, &data) {
                    crate::log_line(&format!("配置同步：记录发送状态失败：{e}"));
                }
            } else {
                crate::log_line("配置同步：当前无在线对端，暂不记录增量状态，下次可重发");
            }
        })
        .ok();
}

pub fn cli_sync_config(kind: &str, path: &str) {
    if !matches!(kind, "custom_phrase" | "skin" | "options") {
        eprintln!("kind 必须是 custom_phrase / skin / options");
        std::process::exit(1);
    }
    if !std::path::Path::new(path).exists() {
        eprintln!("文件不存在：{path}");
        std::process::exit(1);
    }
    let request = shurufa_options::app_dir().join("sync-config-request.json");
    let body = serde_json::json!({ "kind": kind, "path": path });
    if std::fs::write(&request, serde_json::to_vec(&body).unwrap()).is_err() {
        eprintln!("写入同步请求失败");
        std::process::exit(1);
    }
    println!("已提交配置同步请求：{kind} <- {path}（由后台同步服务广播）");
}

fn execute_sync_retry(id: u64) {
    let act = shurufa_options::sync_activity::load();
    let Some(orig) = act.entries.iter().find(|e| e.id == id).cloned() else {
        crate::log_line(&format!("重试：活动 {id} 不存在"));
        return;
    };
    let Some(retry_id) = orig.retry_id.clone() else {
        crate::log_line(&format!("重试：活动 {id} 无可重试载荷"));
        return;
    };
    let payload_path = retry_payload_dir().join(format!("{retry_id}.json"));
    let Ok(raw) = std::fs::read_to_string(&payload_path) else {
        crate::log_line(&format!("重试：载荷 {retry_id} 缺失"));
        return;
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&raw) else {
        crate::log_line(&format!("重试：载荷 {retry_id} 解析失败"));
        return;
    };
    let kind = payload
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let value = payload.get("value");
    let result: Result<(), String> = (|| match kind {
        "clip_text" => {
            let text = value
                .and_then(|v| v.get("text"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "载荷缺 text".to_owned())?;
            if crate::listener::write_remote_text(text.to_owned()) {
                Ok(())
            } else {
                Err("写入系统剪贴板失败".to_owned())
            }
        }
        "clip_image" => {
            let b64 = value
                .and_then(|v| v.get("png_b64"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "载荷缺 png_b64".to_owned())?;
            let png = {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|e| format!("PNG 解码失败：{e}"))?
            };
            let bmp = png_to_bmp(&png).ok_or_else(|| "PNG 转 BMP 失败".to_owned())?;
            if crate::listener::write_remote_image(bmp) {
                Ok(())
            } else {
                Err("写入系统剪贴板失败".to_owned())
            }
        }
        "clip_files" => {
            let path = value
                .and_then(|v| v.get("paths"))
                .and_then(serde_json::Value::as_array)
                .and_then(|a| a.first())
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "载荷缺 paths".to_owned())?;
            if crate::listener::write_remote_files(path.to_owned()) {
                Ok(())
            } else {
                Err("写入系统剪贴板失败".to_owned())
            }
        }
        "send_file" => {
            // M10：发送失败重试——取回原路径走既有广播通道重发
            let path = value
                .and_then(|v| v.get("path"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "载荷缺 path".to_owned())?;
            let path_buf = std::path::PathBuf::from(path);
            if !path_buf.is_file() {
                return Err("原文件已不存在，无法重发".to_owned());
            }
            match CLIP_TX.get() {
                Some(tx) => {
                    let _ = tx.send(Broadcast::FileV3(path_buf));
                    Ok(())
                }
                None => Err("同步服务未运行".to_owned()),
            }
        }
        other => Err(format!("未知重试类型：{other}")),
    })();
    let (ok, detail) = match result {
        Ok(()) => (true, Some("重试成功".to_owned())),
        Err(e) => (false, Some(format!("重试失败：{e}"))),
    };
    record_sync_activity(
        SyncDirection::In,
        orig.kind,
        format!("{}（重试）", orig.preview),
        orig.peer.clone(),
        ok,
        detail,
        None,
    );
    let _ = std::fs::remove_file(&payload_path);
    crate::log_line(&format!(
        "重试活动 {id}：{}",
        if ok { "成功" } else { "失败" }
    ));
}

fn mime_type_for(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn received_file_path(name: &str) -> Option<PathBuf> {
    let name = std::path::Path::new(name).file_name()?.to_str()?;
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    let dir = sync_config_dir().parent()?.join("received");
    std::fs::create_dir_all(&dir).ok()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some(dir.join(format!("{stamp}_{name}")))
}

/// BMP 转成可通过手机协议帧的 PNG。
///
/// 桌面历史保留完整图片；只有同步副本会按 3/4 比例逐轮缩小，直到落入
/// 协议上限，避免高分辨率图片无法进入历史或被静默丢弃。
fn bmp_to_sync_png(bmp: &[u8]) -> Option<(Vec<u8>, bool)> {
    let mut image = image::load_from_memory_with_format(bmp, image::ImageFormat::Bmp).ok()?;
    let mut resized = false;
    loop {
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).ok()?;
        let png = out.into_inner();
        if png.len() <= MAX_CLIP_IMAGE_BYTES {
            return Some((png, resized));
        }
        let (next_width, next_height) = next_sync_dimensions(image.width(), image.height())?;
        image = image.resize(
            next_width,
            next_height,
            image::imageops::FilterType::Triangle,
        );
        resized = true;
    }
}

fn next_sync_dimensions(width: u32, height: u32) -> Option<(u32, u32)> {
    if width <= 1 || height <= 1 {
        return None;
    }
    Some(((width * 3 / 4).max(1), (height * 3 / 4).max(1)))
}

/// PNG 字节转自包含 BMP（供 clipboard-store 存储，与本机采集格式一致）。
fn png_to_bmp(png: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Bmp).ok()?;
    Some(out.into_inner())
}

/// 在独立线程启动同步服务（run 模式调用一次）。
pub fn start_daemon() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Broadcast>();
    if CLIP_TX.set(tx).is_err() {
        return;
    }
    spawn_sync_retry_watcher();
    spawn_sync_config_watcher();
    std::thread::Builder::new()
        .name("sync".into())
        .spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    crate::log_line(&format!("同步运行时创建失败：{e}"));
                    return;
                }
            };
            rt.block_on(async move {
                let (in_tx, mut in_rx) = tokio::sync::mpsc::channel::<Incoming>(64);
                let mut config = SyncConfig::new(sync_config_dir(), device_name());
                config.port = sync_port();
                let confirm: ConfirmFn = Arc::new(confirm_by_messagebox);
                let service = match SyncService::start(
                    config,
                    in_tx,
                    Some(confirm),
                    Box::new(|m| crate::log_line(&format!("同步：{m}"))),
                )
                .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        crate::log_line(&format!("同步服务启动失败：{e}"));
                        return;
                    }
                };
                crate::log_line(&format!(
                    "同步服务就绪，端口 {}，本机指纹 {}",
                    service.local_port(),
                    &service.fingerprint()[..12]
                ));
                let _ = SYNC_SERVICE.set(service.clone());
                // 对端发来的跨设备搜索请求：走本机历史库的 LIKE 搜索，
                // 结果以 SearchHit 列表回传（仅文本条目，图片/文件不回传内容）。
                service.set_search_handler(Arc::new(|query: &str| {
                    let store = crate::open_store();
                    store
                        .search(query, 8)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|hit| sync_core::SearchHit {
                            text: hit.text.clone(),
                            source_app: hit.source_app.clone(),
                            updated_at: hit.updated_at,
                        })
                        .collect()
                }));

                loop {
                    tokio::select! {
                        Some(b) = rx.recv() => match b {
                            Broadcast::Text(t) => service.send_clip(&t),
                            Broadcast::Image(png) => service.send_image(&png),
                            Broadcast::File { name, mime_type, data } => {
                                service.send_file(&name, &mime_type, &data)
                            }
                            Broadcast::FileV3(path) => {
                                match service.send_file_path(&path) {
                                    Ok(msg_id) => {
                                        // M10：记台账供失败重试取回原路径
                                        pending_file_sends()
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner())
                                            .insert(msg_id.clone(), path.clone());
                                        crate::log_line(&format!(
                                            "已发送 {} → 等待对端接收（msg {}）",
                                            path.file_name()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or("?"),
                                            &msg_id[..8]
                                        ));
                                    }
                                    Err(e) => crate::log_line(&format!(
                                        "文件发送失败 {}：{e}",
                                        path.display()
                                    )),
                                }
                            }
                            Broadcast::Config { kind, name, data } => {
                                service.send_config(&kind, &name, &data);
                            }
                        },
                        Some(incoming) = in_rx.recv() => {
                            // 入库/落盘/图片转码/写系统剪贴板均为阻塞或 CPU 密集
                            // 操作，移到 spawn_blocking，避免占用 tokio worker 线程。
                            tokio::task::spawn_blocking(move || match incoming {
                                Incoming::Clip { from_name, text, .. } => {
                                    let store = crate::open_store();
                                    let preview: String = text.chars().take(60).collect();
                                    match store.insert_text(&text, &format!("同步·{from_name}")) {
                                        Ok(_) => {
                                            let ok = crate::listener::write_remote_text(text.clone());
                                            // M8-1b：剪贴板写失败可一键重试；超大文本不落载荷
                                            let retry = if ok || text.chars().count() > 512 * 1024 {
                                                None
                                            } else {
                                                save_retry_payload(
                                                    "clip_text",
                                                    &serde_json::json!({ "text": text }),
                                                )
                                            };
                                            record_sync_activity(
                                                SyncDirection::In,
                                                SyncActivityKind::Text,
                                                preview,
                                                Some(from_name.clone()),
                                                ok,
                                                if ok {
                                                    None
                                                } else {
                                                    Some("写入系统剪贴板失败".to_owned())
                                                },
                                                retry,
                                            );
                                            if ok {
                                                crate::log_line(&format!(
                                                "收到 {from_name} 的剪贴板（{} 字符），已写入系统剪贴板",
                                                text.chars().count()
                                                ));
                                            } else {
                                                crate::log_line(&format!(
                                                    "收到 {from_name} 的文本，但写入系统剪贴板失败"
                                                ));
                                            }
                                        }
                                        Err(e) => {
                                            record_sync_activity(
                                                SyncDirection::In,
                                                SyncActivityKind::Text,
                                                preview,
                                                Some(from_name),
                                                false,
                                                Some(format!("同步条目入库失败：{e}")),
                                                None,
                                            );
                                            crate::log_line(&format!("同步条目入库失败：{e}"));
                                        }
                                    }
                                }
                                Incoming::Image { from_name, png } => match png_to_bmp(&png) {
                                    Some(bmp) => {
                                        let store = crate::open_store();
                                        let preview = format!("图片 {} 字节", png.len());
                                        match store.insert_image(&bmp, &format!("同步·{from_name}")) {
                                            Ok(_) => {
                                                let ok = crate::listener::write_remote_image(bmp.clone());
                                                // M8-1b：图片剪贴板写失败可重试（存 PNG 载荷）
                                                let retry = if ok || png.len() > 1024 * 1024 {
                                                    None
                                                } else {
                                                    let b64 = {
                                                        use base64::Engine;
                                                        base64::engine::general_purpose::STANDARD
                                                            .encode(&png)
                                                    };
                                                    save_retry_payload(
                                                        "clip_image",
                                                        &serde_json::json!({ "png_b64": b64 }),
                                                    )
                                                };
                                                record_sync_activity(
                                                    SyncDirection::In,
                                                    SyncActivityKind::Image,
                                                    preview,
                                                    Some(from_name.clone()),
                                                    ok,
                                                    if ok {
                                                        None
                                                    } else {
                                                        Some("写入系统剪贴板失败".to_owned())
                                                    },
                                                    retry,
                                                );
                                                if ok {
                                                    crate::log_line(&format!(
                                                    "收到 {from_name} 的图片（{} 字节 PNG），已写入系统剪贴板",
                                                    png.len()
                                                    ));
                                                } else {
                                                    crate::log_line(&format!(
                                                        "收到 {from_name} 的图片，但写入系统剪贴板失败"
                                                    ));
                                                }
                                            }
                                            Err(e) => {
                                                record_sync_activity(
                                                    SyncDirection::In,
                                                    SyncActivityKind::Image,
                                                    preview,
                                                    Some(from_name),
                                                    false,
                                                    Some(format!("同步图片入库失败：{e}")),
                                                    None,
                                                );
                                                crate::log_line(&format!("同步图片入库失败：{e}"));
                                            }
                                        }
                                    }
                                    None => crate::log_line("收到图片解码失败"),
                                },
                                Incoming::ConfigFile { from_name, kind, name, data } => {
                                    if !shurufa_options::load().config_sync_enabled {
                                        crate::log_line(&format!(
                                            "收到 {from_name} 的配置 {kind}/{name}，但“接收配置同步”已关闭，忽略"
                                        ));
                                        return;
                                    }
                                    let dir = crate::app_data_dir();
                                    let preview = format!("{kind}/{name}（{} 字符）", data.chars().count());
                                    if sync_core::config_sync::config_path(&dir, &kind).is_none() {
                                        crate::log_line(&format!(
                                            "收到未知配置类型 {kind}（来自 {from_name}），忽略"
                                        ));
                                        return;
                                    }
                                    match sync_core::config_sync::apply_incoming(&dir, &kind, &name, &data) {
                                        Ok(outcome) => {
                                            let path = sync_core::config_sync::config_path(&dir, &kind)
                                                .expect("已校验 kind");
                                            let detail = match outcome.status {
                                                sync_core::config_sync::ApplyStatus::Noop =>
                                                    format!("内容未变化，跳过 {}", path.display()),
                                                sync_core::config_sync::ApplyStatus::AppliedRemote =>
                                                    format!("已写入 {}", path.display()),
                                                sync_core::config_sync::ApplyStatus::Merged => {
                                                    match &outcome.backup {
                                                        Some(bp) => format!(
                                                            "两端均修改，已自动合并写入 {}（旧文件备份 {}）",
                                                            path.display(),
                                                            bp.display()
                                                        ),
                                                        None => format!("两端均修改，已自动合并写入 {}", path.display()),
                                                    }
                                                }
                                                sync_core::config_sync::ApplyStatus::KeptLocal =>
                                                    format!("本地已修改，保留本地 {}（远端未变化）", path.display()),
                                            };
                                            record_sync_activity(
                                                SyncDirection::In,
                                                SyncActivityKind::Config,
                                                preview,
                                                Some(from_name.clone()),
                                                true,
                                                Some(detail.clone()),
                                                None,
                                            );
                                            crate::log_line(&format!(
                                                "收到 {from_name} 的配置 {kind}/{name}，{detail}"
                                            ));
                                        }
                                        Err(e) => {
                                            record_sync_activity(
                                                SyncDirection::In,
                                                SyncActivityKind::Config,
                                                preview,
                                                Some(from_name.clone()),
                                                false,
                                                Some(format!("写入失败：{e}")),
                                                None,
                                            );
                                            crate::log_line(&format!(
                                                "写入 {from_name} 的配置 {kind}/{name} 失败：{e}"
                                            ));
                                        }
                                    }
                                }
                                Incoming::SearchResults { from_name, req_id, hits } => {
                                    crate::log_line(&format!(
                                        "跨设备搜索 {}：{from_name} 返回 {} 条命中",
                                        req_id.as_deref().unwrap_or("<无 id>"),
                                        hits.len()
                                    ));
                                    for hit in hits.iter().take(8) {
                                        crate::log_line(&format!(
                                            "· [{} @{}] {}",
                                            hit.source_app,
                                            hit.updated_at,
                                            crate::single_line_preview(&hit.text, 60)
                                        ));
                                    }
                                }
                                Incoming::FileOffer { from_name, name, size, mime, msg_id, .. } => {
                                    // 入站文件 Offer：在宿主层面只打日志；后续
                                    // 若要弹「接收/拒绝」对话框应在此调用
                                    // service.set_file_confirm_handler(...) 注入。
                                    crate::log_line(&format!(
                                        "收到文件 Offer：来自 {from_name} 的 {name}（{size} B, {mime}, msg {})",
                                        &msg_id[..8.min(msg_id.len())]
                                    ));
                                }
                                Incoming::FileProgress { msg_id, received_bytes } => {
                                    crate::log_line(&format!(
                                        "文件传输进度 msg={} 已收 {} B",
                                        &msg_id[..8.min(msg_id.len())],
                                        received_bytes
                                    ));
                                }
                                Incoming::FileTransferDone { msg_id, name, ok, detail } => {
                                    if ok {
                                        let bytes = detail.unwrap_or(0);
                                        // M10：发送成功清理台账
                                        pending_file_sends()
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner())
                                            .remove(&msg_id);
                                        record_sync_activity(
                                            SyncDirection::Out,
                                            SyncActivityKind::File,
                                            name.clone(),
                                            None,
                                            true,
                                            Some(format!("对方已接收（{bytes} B）")),
                                            None,
                                        );
                                        crate::log_line(&format!(
                                            "对方已接收 {name}（{}，{} B）",
                                            &msg_id[..8.min(msg_id.len())],
                                            bytes
                                        ));
                                    } else {
                                        let reason = detail.err().unwrap_or_else(|| "未知".into());
                                        // M10：取台账原路径生成 SendFile 重试载荷（一键重发）
                                        let retry = pending_file_sends()
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner())
                                            .remove(&msg_id)
                                            .and_then(|path| {
                                                save_retry_payload(
                                                    "send_file",
                                                    &serde_json::json!({ "path": path }),
                                                )
                                            });
                                        record_sync_activity(
                                            SyncDirection::Out,
                                            SyncActivityKind::File,
                                            name.clone(),
                                            None,
                                            false,
                                            Some(format!("文件发送失败：{reason}")),
                                            retry,
                                        );
                                        crate::log_line(&format!(
                                            "文件发送失败 {name}：{reason}"
                                        ));
                                    }
                                }
                                Incoming::File { from_name, name, data, .. } => {
                                    let Some(path) = received_file_path(&name) else {
                                        crate::log_line("收到文件名无效");
                                        return;
                                    };
                                    match std::fs::write(&path, data) {
                                        Ok(()) => {
                                            let store = crate::open_store();
                                            let paths = vec![path.to_string_lossy().into_owned()];
                                            match store.insert_files(&paths, &format!("同步·{from_name}")) {
                                                Ok(_) => {
                                                    let ok = crate::listener::write_remote_files(
                                                        path.to_string_lossy().into_owned(),
                                                    );
                                                    // M8-1b：文件剪贴板写失败可重试（按落盘路径重放）
                                                    let retry = if ok {
                                                        None
                                                    } else {
                                                        save_retry_payload(
                                                            "clip_files",
                                                            &serde_json::json!({
                                                                "paths": [path.to_string_lossy()],
                                                            }),
                                                        )
                                                    };
                                                    record_sync_activity(
                                                        SyncDirection::In,
                                                        SyncActivityKind::File,
                                                        name.clone(),
                                                        Some(from_name.clone()),
                                                        ok,
                                                        if ok {
                                                            None
                                                        } else {
                                                            Some("写入系统剪贴板失败".to_owned())
                                                        },
                                                        retry,
                                                    );
                                                    if ok {
                                                        crate::log_line(&format!(
                                                        "收到 {from_name} 的文件：{name}，已写入系统剪贴板"
                                                        ));
                                                    } else {
                                                        crate::log_line(&format!(
                                                            "收到 {from_name} 的文件，但写入系统剪贴板失败"
                                                        ));
                                                    }
                                                }
                                                Err(e) => crate::log_line(&format!("同步文件入库失败：{e}")),
                                            }
                                        }
                                        Err(e) => crate::log_line(&format!("收到文件落盘失败：{e}")),
                                    }
                                }
                            })
                            .await
                            .ok(); // 处理任务 panic 时忽略该条，不影响后续
                        }
                                                else => break,
                    }
                }
            });
        })
        .expect("同步线程创建失败");
}

/// 入站配对确认：置顶 MessageBox 展示确认码。
fn confirm_by_messagebox(prompt: PairPrompt) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
    };
    let text = format!(
        "设备「{}」请求配对同步剪贴板。\n\n确认码：{}\n\n对方屏幕显示相同确认码时点“是”。",
        prompt.peer_name, prompt.code
    );
    let result = unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from("Shurufa 设备配对"),
            MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };
    result == IDYES
}

/// `pair` 子命令：控制台交互发起配对。
pub fn cli_pair(addr: &str) {
    let addr = if addr.contains(':') {
        addr.to_string()
    } else {
        format!("{addr}:48632")
    };
    let rt = tokio::runtime::Runtime::new().expect("创建运行时失败");
    let result = rt.block_on(async {
        let (in_tx, _in_rx) = tokio::sync::mpsc::channel(4);
        let mut config = SyncConfig::new(sync_config_dir(), device_name());
        // 临时实例：不监听固定端口、不广播，仅作发起端
        config.port = 0;
        config.enable_mdns = false;
        let service = SyncService::start(config, in_tx, None, Box::new(|_| {})).await?;
        let confirm: ConfirmFn = Arc::new(|prompt: PairPrompt| {
            println!(
                "\n设备「{}」\n确认码：{}\n对方屏幕显示相同码则输入 y 回车：",
                prompt.peer_name, prompt.code
            );
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).ok();
            matches!(line.trim(), "y" | "Y" | "yes" | "是")
        });
        service.pair_with(&addr, confirm).await
    });
    match result {
        Ok(peer) => println!("已与「{}」配对。守护进程将在数秒内自动连接。", peer.name),
        Err(e) => {
            eprintln!("配对失败：{e}");
            std::process::exit(1);
        }
    }
}

/// `pair-ui` 子命令：设置中心配对向导的发起端。连接对端拿到确认码后写
/// `pair-prompt.json`，轮询 `pair-confirm.json`（token 校验、60s 超时），
/// 完成/取消后写 `pair-result.json` 并清理 prompt。
pub fn cli_pair_ui(addr: &str) {
    let addr = if addr.contains(':') {
        addr.to_string()
    } else {
        format!("{addr}:48632")
    };
    let data = crate::app_data_dir();
    let prompt_path = data.join("pair-prompt.json");
    let confirm_path = data.join("pair-confirm.json");
    let result_path = data.join("pair-result.json");
    let _ = std::fs::remove_file(&prompt_path);
    let _ = std::fs::remove_file(&confirm_path);
    let _ = std::fs::remove_file(&result_path);
    let token = format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let rt = tokio::runtime::Runtime::new().expect("创建运行时失败");
    let outcome: Result<String, String> = rt.block_on(async {
        let (in_tx, _in_rx) = tokio::sync::mpsc::channel(4);
        let mut config = SyncConfig::new(sync_config_dir(), device_name());
        // 临时实例：不监听固定端口、不广播，仅作发起端
        config.port = 0;
        config.enable_mdns = false;
        let service = SyncService::start(config, in_tx, None, Box::new(|_| {}))
            .await
            .map_err(|e| e.to_string())?;
        let confirm: ConfirmFn = Arc::new({
            let prompt_path = prompt_path.clone();
            let confirm_path = confirm_path.clone();
            let token = token.clone();
            move |prompt: PairPrompt| {
                // 1) 展示确认码（设置中心轮询读取）
                let _ = std::fs::write(
                    &prompt_path,
                    serde_json::json!({
                        "token": token,
                        "peer_name": prompt.peer_name,
                        "code": prompt.code,
                        "ts_ms": std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0),
                    })
                    .to_string(),
                );
                // 2) 轮询确认文件（60s 超时，300ms 间隔）
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
                loop {
                    if let Ok(raw) = std::fs::read_to_string(&confirm_path) {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                            let matches = value.get("token").and_then(serde_json::Value::as_str)
                                == Some(token.as_str());
                            if matches {
                                let yes = value
                                    .get("yes")
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false);
                                let _ = std::fs::remove_file(&confirm_path);
                                return yes;
                            }
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            }
        });
        match service.pair_with(&addr, confirm).await {
            Ok(peer) => Ok(format!("已与「{}」配对", peer.name)),
            Err(e) => Err(format!("配对失败：{e}")),
        }
    });
    let _ = std::fs::remove_file(&prompt_path);
    let (ok, message) = match outcome {
        Ok(message) => (true, message),
        Err(message) => (false, message),
    };
    let _ = std::fs::write(
        &result_path,
        serde_json::json!({ "token": token, "ok": ok, "message": message }).to_string(),
    );
}

/// `devices` 子命令：列出本机身份与已配对设备。
pub fn cli_devices() {
    match sync_core::DeviceIdentity::load_or_create(&sync_config_dir(), &device_name()) {
        Ok(identity) => println!(
            "本机：{}（指纹 {}）",
            identity.device_name,
            identity.short_fp()
        ),
        Err(e) => println!("本机身份不可用：{e}"),
    }
    match sync_core::PeerStore::open(&sync_config_dir()) {
        Ok(store) => {
            let peers = store.list();
            if peers.is_empty() {
                println!("（尚无配对设备，使用 pair <对方IP> 配对）");
            }
            for p in peers {
                println!(
                    "  {} {}  最近地址 {}",
                    &p.fingerprint[..12],
                    p.name,
                    p.last_addr.as_deref().unwrap_or("未知")
                );
            }
        }
        Err(e) => println!("读取配对表失败：{e}"),
    }
}

/// `clip-remote-search` 子命令：连上已配对设备，广播 SearchRequest，
/// 聚合 8 秒内收到的所有 SearchResults 打印出来，随后退出。
///
/// 与常驻守护进程可同时运行：临时实例绑定随机端口（port=0）、关闭 mDNS。
pub fn cli_remote_search(query: &str) {
    let rt = tokio::runtime::Runtime::new().expect("创建运行时失败");
    rt.block_on(async {
        let (in_tx, mut in_rx) = tokio::sync::mpsc::channel(32);
        let mut config = SyncConfig::new(sync_config_dir(), device_name());
        config.port = 0; // 随机端口，避免与常驻 worker 冲突
        config.enable_mdns = false;
        config.reconnect_secs = 1;
        let service = match SyncService::start(config, in_tx, None, Box::new(|_| {})).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("同步服务启动失败：{e}");
                std::process::exit(1);
            }
        };
        let peers = service.peers();
        if peers.is_empty() {
            eprintln!("尚无已配对设备（先 pair）");
            std::process::exit(1);
        }
        println!("搜索关键词：{query}；等待对端响应（至多 8 秒）…");
        // 给重连循环一点时间连上在线的对端，再发请求
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let req_id = format!("cli-{}", std::process::id());
        service.send_search_request(query, req_id.clone());

        let mut hits_total = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while let Ok(Some(incoming)) =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), in_rx.recv()).await
        {
            if let Incoming::SearchResults {
                from_name,
                req_id: rid,
                hits,
            } = incoming
            {
                if rid.as_deref() != Some(req_id.as_str()) {
                    continue;
                }
                println!("设备 {from_name} 命中 {} 条：", hits.len());
                for hit in hits.iter().take(8) {
                    println!(
                        "  · [{}] {}",
                        hit.source_app,
                        crate::single_line_preview(&hit.text, 60)
                    );
                }
                hits_total += hits.len();
            }
        }
        if hits_total == 0 {
            println!("（无命中或对端未在 8 秒内响应）");
        }
    });
}

fn config_backup_dir() -> PathBuf {
    crate::app_data_dir().join("sync-config-backups")
}

/// `sync-config-backups` 子命令：列出配置同步备份文件。
pub fn cli_sync_config_backups() {
    let dir = config_backup_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries.flatten().collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    if entries.is_empty() {
        println!("（暂无配置同步备份）");
        return;
    }
    for entry in entries {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    println!("{name}");
                }
            }
        }
    }
}

/// `sync-config-restore` 子命令：从备份文件名恢复对应配置。
/// 文件名格式：`<ts>_<kind>_<safe_name>`。
pub fn cli_sync_config_restore(file: &str) {
    let backup = config_backup_dir().join(file);
    if !backup.is_file() {
        eprintln!("备份文件不存在：{}", backup.display());
        std::process::exit(1);
    }
    let Some(kind) = sync_core::config_sync::kind_from_backup_name(file) else {
        eprintln!("备份文件名格式不正确：{file}");
        std::process::exit(1);
    };
    let dir = crate::app_data_dir();
    let target = match kind {
        "options" => dir.join("options.json"),
        "skin" => dir.join("shurufa-skin.json"),
        "custom_phrase" => dir.join("rime").join("custom_phrase.txt"),
        _ => {
            eprintln!("无法从备份文件名识别配置类型：{file}");
            std::process::exit(1);
        }
    };
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::copy(&backup, &target) {
        Ok(_) => println!("已从 {} 恢复到 {}", backup.display(), target.display()),
        Err(e) => {
            eprintln!("恢复失败：{e}");
            std::process::exit(1);
        }
    }
}

/// `relay` 子命令：持久化自托管中继地址；下次启动守护进程时生效。
pub fn cli_relay(value: &str) {
    let value = value.trim();
    let relay = if value.eq_ignore_ascii_case("off") {
        None
    } else {
        Some(value)
    };
    match sync_core::save_relay_addr(&sync_config_dir(), relay) {
        Ok(()) if relay.is_some() => {
            println!("已保存自托管中继：{value}。重启 shurufa-host 后生效。")
        }
        Ok(()) => println!("已关闭自托管中继。重启 shurufa-host 后生效。"),
        Err(e) => {
            eprintln!("保存中继配置失败：{e}");
            std::process::exit(1);
        }
    }
}

/// `unpair` 子命令。
pub fn cli_unpair(fp_prefix: &str) {
    match sync_core::PeerStore::open(&sync_config_dir()).and_then(|s| s.remove(fp_prefix)) {
        Ok(true) => println!("已解除配对"),
        Ok(false) => println!("未找到匹配设备（用 devices 查看指纹前缀）"),
        Err(e) => eprintln!("操作失败：{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 发送台账可登记取回并清理() {
        pending_file_sends()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let path = PathBuf::from("C:\\tmp\\report.pdf");
        let msg_id = "msg-abc-123".to_owned();
        {
            let mut ledger = pending_file_sends()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            ledger.insert(msg_id.clone(), path.clone());
            assert_eq!(ledger.get(&msg_id), Some(&path));
        }
        let taken = pending_file_sends()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&msg_id);
        assert_eq!(taken, Some(path));
        assert!(pending_file_sends()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
    }

    #[test]
    fn 同步缩放按比例递减且不会归零() {
        assert_eq!(next_sync_dimensions(2560, 1600), Some((1920, 1200)));
        assert_eq!(next_sync_dimensions(2, 2), Some((1, 1)));
        assert_eq!(next_sync_dimensions(1, 2), None);
    }
}
