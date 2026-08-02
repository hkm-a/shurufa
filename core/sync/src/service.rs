//! 同步服务：监听、发现、重连与消息收发的主循环。
//!
//! 拓扑为全互联：每对已配对设备之间维持一条 TLS 连接（任一方发起，
//! 指纹去重）。剪贴板文本经 broadcast 通道推给所有活跃连接；
//! 收到的条目经 mpsc 交给宿主（桌面端入历史库，安卓端同理）。

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Notify};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::protocol::{read_msg, write_msg, Message};
use crate::{tls, DeviceIdentity, Peer, PeerStore};

/// 同步文本上限：与桌面剪贴板采集策略一致
const MAX_CLIP_TEXT: usize = 64 * 1024;
/// 同步图片上限（PNG 字节）：留在协议帧上限内。
///
/// 平台层必须使用此值约束转码结果，避免图片在发送端被静默丢弃。
pub const MAX_CLIP_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// 单文件同步上限（字节）：与图片共用协议帧预算。
pub const MAX_CLIP_FILE_BYTES: usize = 8 * 1024 * 1024;

/// TCP 连接到达超时（秒）：避免对黑洞/不可达地址无限挂起。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 数据通道读空闲超时（秒）。
///
/// 对端正常时两侧每 30 秒互发一次 Ping，故活跃连接上收到的消息间隔
/// 不会超过约 30 秒。超过该窗口仍无任何数据即判定连接失效（拔线、
/// 对端崩溃但 TCP 未 RST 等），主动断开以让重连循环接管。取值显著
/// 大于 Ping 周期以容忍瞬时延迟抖动。
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// 连接失败初试退避（秒），指数增长封顶。
const BACKOFF_BASE: Duration = Duration::from_secs(10);
const BACKOFF_CAP: Duration = Duration::from_secs(300);

/// 连接失败或断线后介于两次 TCP 心跳之间的保活间隔（秒）。
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct SyncConfig {
    /// 身份与配对表所在目录
    pub config_dir: PathBuf,
    /// 本机展示名（如「书房台式机」「手机」）
    pub device_name: String,
    /// 监听端口；0 表示系统分配
    pub port: u16,
    /// 是否启用 mDNS 广播与发现（测试关闭）
    pub enable_mdns: bool,
    /// 重连扫描间隔（秒）
    pub reconnect_secs: u64,
}

impl SyncConfig {
    pub fn new(config_dir: PathBuf, device_name: String) -> Self {
        SyncConfig {
            config_dir,
            device_name,
            port: 48632,
            enable_mdns: true,
            reconnect_secs: 10,
        }
    }
}

/// 交给宿主的入站事件。
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Clip {
        from_name: String,
        text: String,
    },
    /// 图片（PNG 字节）
    Image {
        from_name: String,
        png: Vec<u8>,
    },
    /// 文件（文件名、MIME 类型与原始字节）。
    File {
        from_name: String,
        name: String,
        mime_type: String,
        data: Vec<u8>,
    },
}

/// 出站广播内容：文本或图片，经 broadcast 通道推给所有活跃连接。
#[derive(Debug, Clone)]
enum Outbound {
    Text(String),
    Image(Vec<u8>),
    File {
        name: String,
        mime_type: String,
        data: Vec<u8>,
    },
}

/// 配对确认提示：宿主展示 `code` 并让用户比对两端一致后放行。
pub struct PairPrompt {
    pub peer_name: String,
    pub code: String,
}

pub type ConfirmFn = Arc<dyn Fn(PairPrompt) -> bool + Send + Sync>;

struct Shared {
    identity: Arc<DeviceIdentity>,
    peers: Arc<PeerStore>,
    device_name: String,
    /// 活跃连接的对端指纹（去重：同一对设备只保留一条连接）
    connected: Mutex<HashSet<String>>,
    /// mDNS 发现的 指纹 → 地址 缓存
    addr_cache: Mutex<HashMap<String, SocketAddr>>,
    /// 连接失败退避：指纹 → (连续失败次数, 最早可重试时刻)
    backoff: Mutex<HashMap<String, (u32, Instant)>>,
    outgoing: broadcast::Sender<Outbound>,
    incoming: mpsc::Sender<Incoming>,
    /// 入站配对请求的确认回调（None 表示拒绝一切配对请求）
    accept_confirm: Option<ConfirmFn>,
    /// 配对成功后唤醒重连循环，立即建立数据连接
    reconnect_now: Notify,
    log: Box<dyn Fn(&str) + Send + Sync>,
}

impl Shared {
    fn log(&self, msg: &str) {
        (self.log)(msg);
    }

    /// 该指纹当前是否允许发起重连（未在退避期内）。
    fn allow_retry(&self, fp: &str) -> bool {
        let map = self.backoff.lock().expect("退避表锁不可恢复");
        match map.get(fp) {
            Some((_, until)) => *until <= Instant::now(),
            None => true,
        }
    }

    /// 记录一次连接失败，按指数策略延后下一次重试。
    fn mark_failure(&self, fp: &str) {
        let mut map = self.backoff.lock().expect("退避表锁不可恢复");
        let (tries, _) = map.get(fp).copied().unwrap_or((0, Instant::now()));
        let tries = tries + 1;
        // 10s → 20s → 40s → … ，封顶 300s
        let factor = 2u64.saturating_pow(tries.saturating_sub(1) as u32);
        let secs = BACKOFF_BASE
            .as_secs()
            .saturating_mul(factor)
            .min(BACKOFF_CAP.as_secs());
        map.insert(fp.to_string(), (tries, Instant::now() + Duration::from_secs(secs)));
    }

    /// 连接成功后清除该指纹的退避记录。
    fn clear_backoff(&self, fp: &str) {
        self.backoff
            .lock()
            .expect("退避表锁不可恢复")
            .remove(fp);
    }
}

pub struct SyncService {
    shared: Arc<Shared>,
    local_addr: SocketAddr,
    /// mDNS 守护线程句柄；持有以维持广播
    _mdns: Option<mdns_sd::ServiceDaemon>,
}

impl SyncService {
    /// 启动监听/重连/发现任务；在 tokio 运行时内调用。
    /// `accept_confirm` 处理入站配对请求（阻塞回调，内部自动转
    /// spawn_blocking）；`log` 接管服务日志输出。
    pub async fn start(
        config: SyncConfig,
        incoming: mpsc::Sender<Incoming>,
        accept_confirm: Option<ConfirmFn>,
        log: Box<dyn Fn(&str) + Send + Sync>,
    ) -> Result<SyncService, String> {
        let identity = Arc::new(DeviceIdentity::load_or_create(
            &config.config_dir,
            &config.device_name,
        )?);
        let peers = Arc::new(PeerStore::open(&config.config_dir)?);
        let (outgoing, _) = broadcast::channel(64);

        let listener = TcpListener::bind(("0.0.0.0", config.port))
            .await
            .map_err(|e| format!("绑定端口 {} 失败: {e}", config.port))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| format!("取监听地址失败: {e}"))?;

        let shared = Arc::new(Shared {
            identity: identity.clone(),
            peers,
            device_name: config.device_name.clone(),
            connected: Mutex::new(HashSet::new()),
            addr_cache: Mutex::new(HashMap::new()),
            backoff: Mutex::new(HashMap::new()),
            outgoing,
            incoming,
            accept_confirm,
            reconnect_now: Notify::new(),
            log,
        });

        let mdns = if config.enable_mdns {
            match start_mdns(&shared, local_addr.port()) {
                Ok(d) => Some(d),
                Err(e) => {
                    shared.log(&format!("mDNS 启动失败（退化为直连）：{e}"));
                    None
                }
            }
        } else {
            None
        };

        // 接受循环
        let accept_shared = shared.clone();
        let acceptor = TlsAcceptor::from(tls::server_config(&identity)?);
        tokio::spawn(async move {
            loop {
                let Ok((stream, addr)) = listener.accept().await else {
                    break;
                };
                let shared = accept_shared.clone();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_inbound(shared.clone(), acceptor, stream, addr).await {
                        shared.log(&format!("入站连接 {addr} 结束：{e}"));
                    }
                });
            }
        });

        // 重连循环
        let reconnect_shared = shared.clone();
        let connector = TlsConnector::from(tls::client_config(&identity)?);
        let interval = Duration::from_secs(config.reconnect_secs.max(1));
        tokio::spawn(async move {
            loop {
                connect_missing_peers(&reconnect_shared, &connector).await;
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = reconnect_shared.reconnect_now.notified() => {}
                }
            }
        });

        Ok(SyncService {
            shared,
            local_addr,
            _mdns: mdns,
        })
    }

    pub fn local_port(&self) -> u16 {
        self.local_addr.port()
    }

    pub fn fingerprint(&self) -> String {
        self.shared.identity.fingerprint.clone()
    }

    pub fn peers(&self) -> Vec<Peer> {
        self.shared.peers.list()
    }

    pub fn remove_peer(&self, fp_prefix: &str) -> Result<bool, String> {
        self.shared.peers.remove(fp_prefix)
    }

    pub fn connected_fingerprints(&self) -> Vec<String> {
        self.shared
            .connected
            .lock()
            .expect("连接表锁不可恢复")
            .iter()
            .cloned()
            .collect()
    }

    /// 广播一条本机产生的剪贴板文本。
    pub fn send_clip(&self, text: &str) {
        if text.is_empty() || text.len() > MAX_CLIP_TEXT {
            return;
        }
        // 无活跃连接时发送失败属正常，静默
        let _ = self.shared.outgoing.send(Outbound::Text(text.to_string()));
    }

    /// 广播一张本机产生的剪贴板图片（PNG 字节）。
    pub fn send_image(&self, png: &[u8]) {
        if png.is_empty() || png.len() > MAX_CLIP_IMAGE_BYTES {
            return;
        }
        let _ = self.shared.outgoing.send(Outbound::Image(png.to_vec()));
    }

    /// 广播一个本机复制的文件。文件名与 MIME 类型仅用于接收端落盘和上屏。
    pub fn send_file(&self, name: &str, mime_type: &str, data: &[u8]) {
        if name.is_empty()
            || name.len() > 255
            || mime_type.is_empty()
            || mime_type.len() > 255
            || data.is_empty()
            || data.len() > MAX_CLIP_FILE_BYTES
        {
            return;
        }
        let _ = self.shared.outgoing.send(Outbound::File {
            name: name.to_string(),
            mime_type: mime_type.to_string(),
            data: data.to_vec(),
        });
    }

    /// 主动向指定地址发起配对（发起端流程）。
    pub async fn pair_with(&self, addr: &str, confirm: ConfirmFn) -> Result<Peer, String> {
        let connector = TlsConnector::from(tls::client_config(&self.shared.identity)?);
        let peer = pair_initiate(&self.shared, &connector, addr, confirm).await?;
        self.shared.reconnect_now.notify_one();
        Ok(peer)
    }
}

/// mDNS：广播自身并缓存同类设备地址。
fn start_mdns(shared: &Arc<Shared>, port: u16) -> Result<mdns_sd::ServiceDaemon, String> {
    use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
    const SERVICE_TYPE: &str = "_shurufa._tcp.local.";

    let daemon = ServiceDaemon::new().map_err(|e| format!("mDNS 守护创建失败: {e}"))?;
    let instance = format!(
        "{}-{}",
        sanitize_instance(&shared.device_name),
        shared.identity.short_fp()
    );
    let props = [
        ("fp", shared.identity.fingerprint.as_str()),
        ("name", shared.device_name.as_str()),
    ];
    let info = ServiceInfo::new(
        SERVICE_TYPE,
        &instance,
        &format!("{instance}.local."),
        "",
        port,
        &props[..],
    )
    .map_err(|e| format!("mDNS 服务信息错误: {e}"))?
    .enable_addr_auto();
    daemon
        .register(info)
        .map_err(|e| format!("mDNS 注册失败: {e}"))?;

    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| format!("mDNS 浏览失败: {e}"))?;
    let browse_shared = shared.clone();
    std::thread::spawn(move || {
        while let Ok(event) = receiver.recv() {
            if let ServiceEvent::ServiceResolved(info) = event {
                let Some(fp) = info.get_property_val_str("fp") else {
                    continue;
                };
                if fp == browse_shared.identity.fingerprint {
                    continue;
                }
                if let Some(ip) = info.get_addresses().iter().next() {
                    let addr = SocketAddr::new(ip.to_ip_addr(), info.get_port());
                    browse_shared
                        .addr_cache
                        .lock()
                        .expect("地址缓存锁不可恢复")
                        .insert(fp.to_string(), addr);
                    browse_shared.reconnect_now.notify_one();
                }
            }
        }
    });
    Ok(daemon)
}

/// mDNS 实例名只保留安全字符，中文等替换为短横线。
fn sanitize_instance(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "shurufa".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 对每个已配对但未连接的设备发起一轮连接（带失败退避）。
async fn connect_missing_peers(shared: &Arc<Shared>, connector: &TlsConnector) {
    for peer in shared.peers.list() {
        let already = shared
            .connected
            .lock()
            .expect("连接表锁不可恢复")
            .contains(&peer.fingerprint);
        if already {
            continue;
        }
        if !shared.allow_retry(&peer.fingerprint) {
            continue;
        }
        let cached = shared
            .addr_cache
            .lock()
            .expect("地址缓存锁不可恢复")
            .get(&peer.fingerprint)
            .copied();
        let addr = match (cached, &peer.last_addr) {
            (Some(a), _) => a.to_string(),
            (None, Some(a)) => a.clone(),
            (None, None) => continue,
        };
        let shared = shared.clone();
        let connector = connector.clone();
        let fp = peer.fingerprint.clone();
        tokio::spawn(async move {
            if let Err(e) = connect_peer(shared.clone(), connector, &peer, &addr).await {
                shared.log(&format!("连接 {}（{addr}）失败：{e}", peer.name));
                shared.mark_failure(&fp);
            }
        });
    }
}

/// 出站数据连接：握手 → 指纹校验 → Hello 交换 → 收发循环。
async fn connect_peer(
    shared: Arc<Shared>,
    connector: TlsConnector,
    peer: &Peer,
    addr: &str,
) -> Result<(), String> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| format!("连接超时（{}s）: {addr}", CONNECT_TIMEOUT.as_secs()))?
        .map_err(|e| format!("TCP 连接失败: {e}"))?;
    let domain =
        rustls::pki_types::ServerName::try_from("shurufa-device").expect("固定域名不应失败");
    let tls = connector
        .connect(domain, stream)
        .await
        .map_err(|e| format!("TLS 握手失败: {e}"))?;

    let fp = peer_fingerprint(tls.get_ref().1)?;
    if fp != peer.fingerprint {
        return Err("对端指纹与配对记录不符".into());
    }
    let mut tls = tls;
    write_msg(
        &mut tls,
        &Message::Hello {
            name: shared.device_name.clone(),
            fingerprint: shared.identity.fingerprint.clone(),
        },
    )
    .await?;
    let Message::Hello { name, .. } = read_msg(&mut tls).await? else {
        return Err("对端未按协议发送 Hello".into());
    };
    let _ = shared.peers.update_addr(&fp, addr);
    shared.clear_backoff(&fp);
    duplex(shared, tls, fp, name).await
}

/// 入站连接：已配对 → 数据通道；未配对 → 配对流程（需确认回调）。
async fn handle_inbound(
    shared: Arc<Shared>,
    acceptor: TlsAcceptor,
    stream: TcpStream,
    addr: SocketAddr,
) -> Result<(), String> {
    let tls = acceptor
        .accept(stream)
        .await
        .map_err(|e| format!("TLS 接受失败: {e}"))?;
    let fp = peer_fingerprint(tls.get_ref().1)?;

    let mut tls = tls;
    let Message::Hello { name, fingerprint } = read_msg(&mut tls).await? else {
        return Err("对端未按协议发送 Hello".into());
    };
    if fingerprint != fp {
        return Err("Hello 指纹与证书不符".into());
    }
    write_msg(
        &mut tls,
        &Message::Hello {
            name: shared.device_name.clone(),
            fingerprint: shared.identity.fingerprint.clone(),
        },
    )
    .await?;

    if shared.peers.contains(&fp) {
        let _ = shared.peers.update_addr(&fp, &addr.to_string());
        return duplex(shared, tls, fp, name).await;
    }

    // 未配对：走配对确认
    let Some(confirm) = shared.accept_confirm.clone() else {
        write_msg(&mut tls, &Message::PairReject).await.ok();
        return Err(format!("拒绝未配对设备 {name}（未开启配对确认）"));
    };
    let code = crate::pairing_code(&shared.identity.fingerprint, &fp);
    let prompt = PairPrompt {
        peer_name: name.clone(),
        code,
    };
    let accepted = tokio::task::spawn_blocking(move || confirm(prompt))
        .await
        .map_err(|e| format!("确认回调失败: {e}"))?;
    if !accepted {
        write_msg(&mut tls, &Message::PairReject).await.ok();
        return Err(format!("用户拒绝与 {name} 配对"));
    }
    write_msg(&mut tls, &Message::PairConfirm).await?;
    match read_msg(&mut tls).await? {
        Message::PairConfirm => {}
        _ => return Err("对端未确认配对".into()),
    }
    shared.peers.upsert(Peer {
        name: name.clone(),
        fingerprint: fp.clone(),
        last_addr: Some(addr.to_string()),
    })?;
    shared.log(&format!("已与 {name} 配对"));
    duplex(shared, tls, fp, name).await
}

/// 发起端配对：连接 → Hello → 确认码 → PairConfirm 交换 → 入表。
async fn pair_initiate(
    shared: &Arc<Shared>,
    connector: &TlsConnector,
    addr: &str,
    confirm: ConfirmFn,
) -> Result<Peer, String> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| format!("连接超时（{}s）: {addr}", CONNECT_TIMEOUT.as_secs()))?
        .map_err(|e| format!("TCP 连接失败: {e}"))?;
    let domain =
        rustls::pki_types::ServerName::try_from("shurufa-device").expect("固定域名不应失败");
    let mut tls = connector
        .connect(domain, stream)
        .await
        .map_err(|e| format!("TLS 握手失败: {e}"))?;
    let fp = peer_fingerprint(tls.get_ref().1)?;

    write_msg(
        &mut tls,
        &Message::Hello {
            name: shared.device_name.clone(),
            fingerprint: shared.identity.fingerprint.clone(),
        },
    )
    .await?;
    let Message::Hello { name, fingerprint } = read_msg(&mut tls).await? else {
        return Err("对端未按协议发送 Hello".into());
    };
    if fingerprint != fp {
        return Err("Hello 指纹与证书不符".into());
    }

    let code = crate::pairing_code(&shared.identity.fingerprint, &fp);
    let prompt = PairPrompt {
        peer_name: name.clone(),
        code,
    };
    let accepted = tokio::task::spawn_blocking(move || confirm(prompt))
        .await
        .map_err(|e| format!("确认回调失败: {e}"))?;
    if !accepted {
        write_msg(&mut tls, &Message::PairReject).await.ok();
        return Err("本端取消配对".into());
    }
    write_msg(&mut tls, &Message::PairConfirm).await?;
    match read_msg(&mut tls).await? {
        Message::PairConfirm => {}
        Message::PairReject => return Err("对端拒绝配对".into()),
        _ => return Err("对端未确认配对".into()),
    }

    let peer = Peer {
        name,
        fingerprint: fp,
        last_addr: Some(addr.to_string()),
    };
    shared.peers.upsert(peer.clone())?;
    shared.log(&format!("已与 {} 配对", peer.name));
    Ok(peer)
}

/// 从 TLS 连接取对端证书指纹。
fn peer_fingerprint(conn: &rustls::CommonState) -> Result<String, String> {
    let certs = conn.peer_certificates().ok_or("对端未提供证书")?;
    let first = certs.first().ok_or("对端证书为空")?;
    Ok(crate::fingerprint_hex(first.as_ref()))
}

/// 数据通道收发循环：出站广播 + 入站转交 + 保活。
///
/// 读侧带空闲超时：正常连接每 30 秒必能收到对端 Ping，超过
/// `READ_IDLE_TIMEOUT` 无数据即视为连接失效并断开，避免僵尸连接
/// 长期占用 `connected` 集合、阻断重连。
async fn duplex<S>(
    shared: Arc<Shared>,
    mut tls: S,
    fp: String,
    peer_name: String,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    {
        let mut connected = shared.connected.lock().expect("连接表锁不可恢复");
        if !connected.insert(fp.clone()) {
            // 双向同时建连的竞态：保留已有连接
            return Ok(());
        }
    }
    shared.log(&format!("已连接 {peer_name}"));

    let mut rx = shared.outgoing.subscribe();
    let mut ping = tokio::time::interval(HEARTBEAT_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let result = loop {
        tokio::select! {
            incoming = tokio::time::timeout(READ_IDLE_TIMEOUT, read_msg(&mut tls)) => {
                match incoming {
                    Ok(Ok(Message::ClipText { text, .. })) => {
                        if text.len() <= MAX_CLIP_TEXT {
                            let _ = shared
                                .incoming
                                .send(Incoming::Clip {
                                    from_name: peer_name.clone(),
                                    text,
                                })
                                .await;
                        }
                    }
                    Ok(Ok(Message::ClipImage { data, .. })) => {
                        match base64::engine::general_purpose::STANDARD.decode(&data) {
                            Ok(png) if png.len() <= MAX_CLIP_IMAGE_BYTES => {
                                let _ = shared
                                    .incoming
                                    .send(Incoming::Image {
                                        from_name: peer_name.clone(),
                                        png,
                                    })
                                    .await;
                            }
                            _ => {}
                        }
                    }
                    Ok(Ok(Message::ClipFile { name, mime_type, data, .. })) => {
                        match base64::engine::general_purpose::STANDARD.decode(&data) {
                            Ok(data)
                                if !name.is_empty()
                                    && name.len() <= 255
                                    && !mime_type.is_empty()
                                    && mime_type.len() <= 255
                                    && data.len() <= MAX_CLIP_FILE_BYTES =>
                            {
                                let _ = shared
                                    .incoming
                                    .send(Incoming::File {
                                        from_name: peer_name.clone(),
                                        name,
                                        mime_type,
                                        data,
                                    })
                                    .await;
                            }
                            _ => {}
                        }
                    }
                    Ok(Ok(Message::Ping)) => {}
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => break Err(e),
                    Err(_) => break Err(format!(
                        "对端 {peer_name} {} 秒无响应，判定连接失效",
                        READ_IDLE_TIMEOUT.as_secs()
                    )),
                }
            }
            out = rx.recv() => match out {
                Ok(Outbound::Text(text)) => {
                    let msg = Message::ClipText {
                        text,
                        sent_at_ms: now_ms(),
                    };
                    if let Err(e) = write_msg(&mut tls, &msg).await {
                        break Err(e);
                    }
                }
                Ok(Outbound::Image(png)) => {
                    let data = base64::engine::general_purpose::STANDARD.encode(&png);
                    let msg = Message::ClipImage {
                        data,
                        sent_at_ms: now_ms(),
                    };
                    if let Err(e) = write_msg(&mut tls, &msg).await {
                        break Err(e);
                    }
                }
                Ok(Outbound::File { name, mime_type, data }) => {
                    let msg = Message::ClipFile {
                        name,
                        mime_type,
                        data: base64::engine::general_purpose::STANDARD.encode(&data),
                        sent_at_ms: now_ms(),
                    };
                    if let Err(e) = write_msg(&mut tls, &msg).await {
                        break Err(e);
                    }
                }
                // 落后于广播积压时跳过旧消息继续
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break Ok(()),
            },
            _ = ping.tick() => {
                if let Err(e) = write_msg(&mut tls, &Message::Ping).await {
                    break Err(e);
                }
            }
        }
    };

    shared
        .connected
        .lock()
        .expect("连接表锁不可恢复")
        .remove(&fp);
    shared.log(&format!("连接 {peer_name} 已断开"));
    result
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
