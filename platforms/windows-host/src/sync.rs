//! 桌面端同步接入：常驻线程运行 SyncService，接收条目入历史库，
//! 本机剪贴板文本经全局句柄广播给已配对设备。
//!
//! 入站配对确认用 MessageBox（守护进程无控制台）；`pair` 子命令
//! 在独立进程内用控制台交互发起配对，写入共享 peers.json 后由
//! 守护进程的重连循环自动接管连接。

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

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
}

/// 守护进程内广播出口；`run` 模式启动后可用
static CLIP_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<Broadcast>> = OnceLock::new();

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

#[cfg(test)]
mod tests {
    use super::next_sync_dimensions;

    #[test]
    fn 同步缩放按比例递减且不会归零() {
        assert_eq!(next_sync_dimensions(2560, 1600), Some((1920, 1200)));
        assert_eq!(next_sync_dimensions(2, 2), Some((1, 1)));
        assert_eq!(next_sync_dimensions(1, 2), None);
    }
}

/// 在独立线程启动同步服务（run 模式调用一次）。
pub fn start_daemon() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Broadcast>();
    if CLIP_TX.set(tx).is_err() {
        return;
    }
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
                                    Ok(msg_id) => crate::log_line(&format!(
                                        "已发送 {} → 等待对端接收（msg {}）",
                                        path.file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("?"),
                                        &msg_id[..8]
                                    )),
                                    Err(e) => crate::log_line(&format!(
                                        "文件发送失败 {}：{e}",
                                        path.display()
                                    )),
                                }
                            }
                        },
                        Some(incoming) = in_rx.recv() => {
                            // 入库/落盘/图片转码/写系统剪贴板均为阻塞或 CPU 密集
                            // 操作，移到 spawn_blocking，避免占用 tokio worker 线程。
                            tokio::task::spawn_blocking(move || match incoming {
                                Incoming::Clip { from_name, text, .. } => {
                                    let store = crate::open_store();
                                    match store.insert_text(&text, &format!("同步·{from_name}")) {
                                        Ok(_) => {
                                            if crate::listener::write_remote_text(text.clone()) {
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
                                        Err(e) => crate::log_line(&format!("同步条目入库失败：{e}")),
                                    }
                                }
                                Incoming::Image { from_name, png } => match png_to_bmp(&png) {
                                    Some(bmp) => {
                                        let store = crate::open_store();
                                        match store.insert_image(&bmp, &format!("同步·{from_name}")) {
                                            Ok(_) => {
                                                if crate::listener::write_remote_image(bmp.clone()) {
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
                                            Err(e) => crate::log_line(&format!("同步图片入库失败：{e}")),
                                        }
                                    }
                                    None => crate::log_line("收到图片解码失败"),
                                },
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
                                        crate::log_line(&format!(
                                            "对方已接收 {name}（{}，{} B）",
                                            &msg_id[..8.min(msg_id.len())],
                                            bytes
                                        ));
                                    } else {
                                        let reason = detail.err().unwrap_or_else(|| "未知".into());
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
                                                    if crate::listener::write_remote_files(
                                                        path.to_string_lossy().into_owned(),
                                                    ) {
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
        while let Ok(Some(incoming)) = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            in_rx.recv(),
        )
        .await
        {
            if let Incoming::SearchResults { from_name, req_id: rid, hits } = incoming {
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
