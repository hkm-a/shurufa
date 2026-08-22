//! 同步服务：监听、发现、重连与消息收发的主循环。
//!
//! 拓扑为全互联：每对已配对设备之间维持一条 TLS 连接（任一方发起，
//! 指纹去重）。剪贴板文本经 broadcast 通道推给所有活跃连接；
//! 收到的条目经 mpsc 交给宿主（桌面端入历史库，安卓端同理）。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Notify};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::protocol::{
    read_msg, read_msg_with_format, write_msg, write_msg_with_format, FrameFormat, Message,
};
use crate::relay::{accept_via_relay, connect_via_relay};
use crate::{tls, DeviceIdentity, Peer, PeerStore};

/// 同步文本上限：与桌面剪贴板采集策略一致
const MAX_CLIP_TEXT: usize = 64 * 1024;
/// 同步图片上限（PNG 字节）：留在协议帧上限内。
///
/// 平台层必须使用此值约束转码结果，避免图片在发送端被静默丢弃。
pub const MAX_CLIP_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// 单文件同步上限（字节）：与图片共用协议帧预算。
pub const MAX_CLIP_FILE_BYTES: usize = 8 * 1024 * 1024;
/// 文件 v3 单文件上限：64 MB。两端都遵守；超出按 too_large 拒绝。
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// 文件 v3 接收侧自动接受的体量阈值（<5MB）+ MIME 白名单。
pub const FILE_AUTO_ACCEPT_MAX: u64 = 5 * 1024 * 1024;

/// 出站广播容量。
///
/// 必须能装下最大文件 v3 的全部 Chunk（64MB / 64KB = 1024 块），否则生产者
/// 快速灌入时接收侧必然 `Lagged`，丢块后按 `chunk_out_of_order` 中止——实测
/// 超过 4MB 的文件传输必败。128 是给文本/图片/控制消息与调度抖动的余量。
const BROADCAST_CAPACITY: usize = 1024 + 128;

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
    /// 可选自托管中继地址；直连不可用时才经中继建立 TLS 流。
    pub relay_addr: Option<String>,
}

impl SyncConfig {
    pub fn new(config_dir: PathBuf, device_name: String) -> Self {
        SyncConfig {
            relay_addr: load_relay_addr(&config_dir),
            config_dir,
            device_name,
            port: 48632,
            enable_mdns: true,
            reconnect_secs: 10,
        }
    }
}

/// 读取自托管中继地址。空文件、无效内容或不存在均视为未启用，避免配置
/// 损坏阻止输入法或宿主的同步服务启动。
pub fn load_relay_addr(config_dir: &Path) -> Option<String> {
    fs::read_to_string(config_dir.join("relay.addr"))
        .ok()
        .and_then(|value| normalize_relay_addr(&value))
}

/// 原子保存自托管中继地址；传入 `None` 会关闭中继并移除配置文件。
pub fn save_relay_addr(config_dir: &Path, relay_addr: Option<&str>) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|e| format!("创建同步配置目录失败: {e}"))?;
    let path = config_dir.join("relay.addr");
    let relay_addr = match relay_addr {
        Some(value) => normalize_relay_addr(value).ok_or("中继地址必须为主机名或 IP 加端口")?,
        None => match fs::remove_file(&path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("移除中继配置失败: {e}")),
        },
    };
    let temp = config_dir.join("relay.addr.tmp");
    fs::write(&temp, format!("{relay_addr}\n")).map_err(|e| format!("写入中继配置失败: {e}"))?;
    fs::rename(&temp, &path).map_err(|e| format!("替换中继配置失败: {e}"))
}

fn normalize_relay_addr(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("off")
        || value.chars().any(char::is_whitespace)
    {
        return None;
    }
    let (host, port) = value.rsplit_once(':')?;
    if host.is_empty() || port.parse::<u16>().ok().filter(|port| *port > 0).is_none() {
        return None;
    }
    Some(value.to_string())
}

/// 交给宿主的入站事件。
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Clip {
        from_name: String,
        text: String,
        /// 对端声明的消息发送时刻（epoch 毫秒）；旧版本端未提供时为 None。
        sent_at_ms: Option<i64>,
        /// 对端分配的消息 id（msg-id-v1 特性）；用于回声/重放去重。
        msg_id: Option<String>,
    },
    /// 图片（PNG 字节）
    Image { from_name: String, png: Vec<u8> },
    /// 文件（文件名、MIME 类型与原始字节）。
    File {
        from_name: String,
        name: String,
        mime_type: String,
        data: Vec<u8>,
    },
    /// 跨设备剪贴板搜索的响应结果（来自某一台对端）。
    SearchResults {
        from_name: String,
        /// 与本端 SearchRequest 关联的请求 id；对端未回传时为 None。
        req_id: Option<String>,
        /// 命中摘要（至多 8 条，仅文本预览）。
        hits: Vec<crate::protocol::SearchHit>,
    },
    /// 收到对端文件 Offer（v3 file-v1 协商后）。宿主应提示用户；
    /// 自动接收条件（小文件 + 白名单 MIME）由宿主判定后调用
    /// `respond_file_offer(msg_id, true)`。
    FileOffer {
        from_name: String,
        msg_id: String,
        name: String,
        size: u64,
        mime: String,
        sha256: String,
        chunk_bytes: u32,
    },
    /// 文件传输进度（对端→本端）：已收字节数。
    FileProgress { msg_id: String, received_bytes: u64 },
    /// 本端发起的文件传输被对端确认/拒绝/出错后的终态。
    FileTransferDone {
        msg_id: String,
        name: String,
        ok: bool,
        /// Ok 时为接收方报回的字节数；Err 时为人类可读原因。
        detail: Result<u64, String>,
    },
}

/// 出站广播内容：文本或图片，经 broadcast 通道推给所有活跃连接。
/// 每条消息天然带一个 uuid v4 `msg_id` 与 `sent_at_ms`，供两端做
/// 近期复读回波抑制（旧版本端不带 msg_id 时职责落在写入端签名比对）。
#[derive(Debug, Clone)]
enum Outbound {
    Text {
        text: String,
        msg_id: String,
        sent_at_ms: i64,
    },
    Image(Vec<u8>),
    File {
        name: String,
        mime_type: String,
        data: Vec<u8>,
    },
    /// 跨设备剪贴板搜索请求；由所有协商了 search-v1 的连接各自写出。
    SearchRequest {
        query: String,
        req_id: String,
    },
    /// 文件 v3 控制/数据面消息；仅在协商了 file-v1 的连接上写出。
    FileWire(Box<Message>),
}

/// 生成一条出站消息 id（uuid v4，无连字符小写）。
fn new_msg_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // 基于时间戳 + 进程内自增 + 随机源构造 128bit，避免引入 uuid 依赖。
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let rand: u64 = rand_part();
    let a = (now as u64) ^ rand.rotate_left(21);
    let b = ((now >> 64) as u64) ^ seq.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ rand;
    format!("{a:016x}{b:016x}")
}

/// 取一个 64bit 随机数：优先 `getrandom` 不可用则回退到地址熵 + 时间。
fn rand_part() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    // RandomState 每次进程启动随机播种；用来快速取一个无法预测的 64bit。
    let state = RandomState::new();
    let hasher = state.build_hasher();
    use std::hash::Hasher;
    let mut h = hasher;
    h.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0),
    );
    h.finish()
}

/// 配对确认提示：宿主展示 `code` 并让用户比对两端一致后放行。
pub struct PairPrompt {
    pub peer_name: String,
    pub code: String,
}

pub type ConfirmFn = Arc<dyn Fn(PairPrompt) -> bool + Send + Sync>;

/// 搜索回调类型：输入查询串，返回命中摘要列表（由本端历史库提供）。
pub type SearchHandler = Arc<dyn Fn(&str) -> Vec<crate::protocol::SearchHit> + Send + Sync>;

/// 入站文件确认回调：返回 true=接受 false=拒绝。宿主在 UI 线程同步
/// 弹系统对话框/托盘气泡；回调在 tokio 运行时外被调用。
pub type FileConfirmFn = Arc<dyn Fn(FileOfferPrompt) -> bool + Send + Sync>;

/// 入站文件 Offer 提示；接收方宿主据此弹「接收/拒绝」。
///
/// `transfer_id` 由 sync-core 在 Offer 到达时单调生成（自增 AtomicU64），
/// 用于把「用户稍后在通知上点击接受/拒绝」对应回同一条 Offer——例如
/// Android 的 BroadcastReceiver 把 transfer_id 回传给 JNI 侧以放行
/// 阻塞中的决策通道。
#[derive(Debug, Clone, PartialEq)]
pub struct FileOfferPrompt {
    /// 接收侧单调递增的传输 id；仅在当前进程内有效。
    pub transfer_id: u64,
    /// 来源设备 SHA-256 指纹（64 字符小写十六进制）。前端可截 8 位展示。
    pub peer_fp: String,
    pub from_name: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
}

/// 接收侧 Offer 决策用的单调 id：进程内 AtomicU64，从 1 起自增，
/// 仅供把「同一进程内的某次 Offer 决策请求」与后来的用户响应一一对应；
/// 与协议层 msg_id 正交，不参与线上数据。
static FILE_OFFER_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_file_offer_id() -> u64 {
    FILE_OFFER_SEQ.fetch_add(1, AtomicOrdering::Relaxed)
}

struct Shared {
    identity: Arc<DeviceIdentity>,
    peers: Arc<PeerStore>,
    device_name: String,
    /// 活跃连接的对端指纹（去重：同一对设备只保留一条连接）
    connected: Mutex<HashSet<String>>,
    /// mDNS 发现的 指纹 → 地址 缓存
    addr_cache: Mutex<HashMap<String, SocketAddr>>,
    /// 本机同步监听端口；配对时传给对端保存为可重连地址。
    local_port: u16,
    /// 连接失败退避：指纹 → (连续失败次数, 最早可重试时刻)
    backoff: Mutex<HashMap<String, (u32, Instant)>>,
    outgoing: broadcast::Sender<Outbound>,
    incoming: mpsc::Sender<Incoming>,
    /// 入站配对请求的确认回调（None 表示拒绝一切配对请求）
    accept_confirm: Option<ConfirmFn>,
    relay_addr: Option<String>,
    /// 配对成功后唤醒重连循环，立即建立数据连接
    reconnect_now: Notify,
    /// 近期本端发出的 (msg_id, sent_at_ms)，用于回声剔除（跨端 LWW 辅助）。
    recent_out: Mutex<Vec<(String, i64)>>,
    /// 本端剪贴板历史搜索回调；启动后由宿主注入（None 时返回空结果）。
    search_handler: Mutex<Option<SearchHandler>>,
    /// 入站 Offer 弹出回调（宿主注入）；None 时按白名单+体量自动收。
    file_confirm: Mutex<Option<FileConfirmFn>>,
    /// 已送达宿主但尚未拿到决策的 Offer（transfer_id → prompt）。
    /// 用户异步响应（通知按钮/对话框）经此找回同一笔决策。
    pending_offers: Mutex<HashMap<u64, FileOfferPrompt>>,
    /// 传输中 msg_id → 状态；宿主轮询/测试断言用。
    transfers: Mutex<HashMap<String, FileSendState>>,
    /// 出站 Offer 发出时刻，用于计算 30s 无响应超时。
    offer_sent_at: Mutex<HashMap<String, Instant>>,
    /// 出站 Streaming 阶段最近一次收到 FileProgress（或发送 Chunk）时刻，
    /// 60s 无动静判定对端停滞。
    stream_progress_at: Mutex<HashMap<String, Instant>>,
    /// 入站文件接收中（msg_id → 状态）。
    file_recv: Mutex<HashMap<String, FileRecvState>>,
    /// 测试用替身目标目录；正常为 None → Downloads\shurufa（或平台对应目录）。
    file_recv_dir_override: Mutex<Option<PathBuf>>,
    log: Box<dyn Fn(&str) + Send + Sync>,
}

/// 入站文件传输的内部状态（按连接隔离、由 duplex 管理）。
#[derive(Clone)]
pub struct FileRecvState {
    pub from_name: String,
    pub name: String,
    pub size: u64,
    /// 保留给后续 UI 按 MIME 分组（图片/文本/归档等），当前未读但保留。
    #[allow(dead_code)]
    pub mime: String,
    pub sha256: String,
    /// 累积已写字节。
    pub received: u64,
    /// 临时落地目录；落地完成后 .part 会被原子改名。
    pub part_path: PathBuf,
    /// 最近一次收到 FileChunk 时刻（测试与超时用）。
    pub last_chunk_at: Instant,
}

impl Shared {
    fn log(&self, msg: &str) {
        (self.log)(msg);
    }

    /// 该指纹当前是否允许发起重连（未在退避期内）。
    fn allow_retry(&self, fp: &str) -> bool {
        let map = self
            .backoff
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match map.get(fp) {
            Some((_, until)) => *until <= Instant::now(),
            None => true,
        }
    }

    /// 记录一次连接失败，按指数策略延后下一次重试。
    fn mark_failure(&self, fp: &str) {
        let mut map = self
            .backoff
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (tries, _) = map.get(fp).copied().unwrap_or((0, Instant::now()));
        let tries = tries + 1;
        // 10s → 20s → 40s → … ，封顶 300s
        let factor = 2u64.saturating_pow(tries.saturating_sub(1));
        let secs = BACKOFF_BASE
            .as_secs()
            .saturating_mul(factor)
            .min(BACKOFF_CAP.as_secs());
        map.insert(
            fp.to_string(),
            (tries, Instant::now() + Duration::from_secs(secs)),
        );
    }

    /// 连接成功后清除该指纹的退避记录。
    fn clear_backoff(&self, fp: &str) {
        self.backoff
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(fp);
    }

    /// 把 msg_id 对应的传输切到指定状态；保留既有终态不动。
    fn set_transfer(&self, msg_id: &str, next: FileSendState) {
        let mut map = self
            .transfers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(
            map.get(msg_id),
            Some(FileSendState::Acked { .. })
                | Some(FileSendState::Declined { .. })
                | Some(FileSendState::Failed { .. })
        ) {
            return;
        }
        map.insert(msg_id.to_string(), next);
    }

    /// 当前传输状态副本（测试 / 宿主 UI 轮询用）。
    pub fn transfer_state(&self, msg_id: &str) -> Option<FileSendState> {
        self.transfers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(msg_id)
            .cloned()
    }

    /// 把 msg_id 转入 StreamDone；供 send_file 在推送完 Done 后调用。
    fn mark_stream_done(&self, msg_id: &str) {
        self.set_transfer(
            msg_id,
            FileSendState::StreamDone {
                msg_id: msg_id.to_string(),
            },
        );
    }

    /// 启动后台任务：扫描 transfers，把卡 OfferSent >30s / Streaming 停滞 >60s 的标 Failed。
    fn start_transfer_watchdog(self: &Arc<Self>) {
        let shared = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            loop {
                tick.tick().await;
                let now = Instant::now();
                let mut to_fail: Vec<(String, &'static str)> = Vec::new();
                {
                    let mut send_map = shared
                        .offer_sent_at
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    let mut prog_map = shared
                        .stream_progress_at
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    let transfers = shared.transfers.lock().unwrap_or_else(|p| p.into_inner());
                    for (msg_id, sent_at) in send_map.iter() {
                        if let Some(FileSendState::OfferSent { .. }) = transfers.get(msg_id) {
                            if now.duration_since(*sent_at) > Duration::from_secs(30) {
                                to_fail.push((msg_id.clone(), "对方未响应"));
                            }
                        }
                    }
                    for (msg_id, last) in prog_map.iter() {
                        if let Some(FileSendState::Streaming { .. }) = transfers.get(msg_id) {
                            if now.duration_since(*last) > Duration::from_secs(60) {
                                to_fail.push((msg_id.clone(), "传输进度停滞超时"));
                            }
                        }
                    }
                    send_map.retain(|mid, _| {
                        !to_fail.iter().any(|(o_mid, _)| o_mid == mid)
                            && !matches!(
                                transfers.get(mid),
                                Some(FileSendState::Acked { .. })
                                    | Some(FileSendState::Declined { .. })
                                    | Some(FileSendState::Failed { .. })
                            )
                    });
                    prog_map.retain(|mid, _| {
                        !to_fail.iter().any(|(o_mid, _)| o_mid == mid)
                            && matches!(transfers.get(mid), Some(FileSendState::Streaming { .. }))
                    });
                }
                for (msg_id, reason) in to_fail {
                    shared.set_transfer(
                        &msg_id,
                        FileSendState::Failed {
                            msg_id: msg_id.clone(),
                            error: reason.into(),
                        },
                    );
                    let _ = shared
                        .incoming
                        .send(Incoming::FileTransferDone {
                            msg_id: msg_id.clone(),
                            name: String::new(),
                            ok: false,
                            detail: Err(reason.into()),
                        })
                        .await;
                }
            }
        });
    }
}

pub struct SyncService {
    shared: Arc<Shared>,
    local_addr: SocketAddr,
    /// mDNS 守护线程句柄；持有以维持广播
    _mdns: Option<mdns_sd::ServiceDaemon>,
}

/// 出站文件传输状态机（供 send_file 驱动、duplex 辅助推进）。
#[derive(Debug, Clone, PartialEq)]
pub enum FileSendState {
    /// Offer 已发出，等待 Accept/Decline（或 30s 超时）。
    OfferSent { msg_id: String },
    /// 收到 Accept，开始流式发送 Chunk。
    Streaming { msg_id: String, sent_bytes: u64 },
    /// 全部 Chunk 已发并送 FileDone；等待 FileAck。
    StreamDone { msg_id: String },
    /// 对端 FileAck 已收到，传输结束（detail 为接收方报回字节数或错误）。
    Acked { msg_id: String, received: u64 },
    /// 对端明确拒绝。
    Declined { msg_id: String, reason: String },
    /// 本地错或对端无响应。
    Failed { msg_id: String, error: String },
}

/// 接收侧默认落地目录占位：测试经 `set_file_recv_dir_override` 覆盖，
/// 生产端走系统 Downloads/shurufa（见 incoming_file_part_path）。
#[allow(dead_code)]
pub fn file_received_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("received")
}

impl Clone for SyncService {
    fn clone(&self) -> Self {
        SyncService {
            shared: self.shared.clone(),
            local_addr: self.local_addr,
            _mdns: None,
        }
    }
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
        let (outgoing, _) = broadcast::channel(BROADCAST_CAPACITY);

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
            local_port: local_addr.port(),
            backoff: Mutex::new(HashMap::new()),
            outgoing,
            incoming,
            accept_confirm,
            relay_addr: config.relay_addr.clone(),
            reconnect_now: Notify::new(),
            recent_out: Mutex::new(Vec::new()),
            // 搜索留给宿主在历史库就绪后再注入，避免阻塞服务启动。
            search_handler: Mutex::new(None),
            file_confirm: Mutex::new(None),
            pending_offers: Mutex::new(HashMap::new()),
            transfers: Mutex::new(HashMap::new()),
            offer_sent_at: Mutex::new(HashMap::new()),
            stream_progress_at: Mutex::new(HashMap::new()),
            file_recv: Mutex::new(HashMap::new()),
            file_recv_dir_override: Mutex::new(None),
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
                    if let Err(e) =
                        handle_inbound(shared.clone(), acceptor, stream, Some(addr)).await
                    {
                        shared.log(&format!("入站连接 {addr} 结束：{e}"));
                    }
                });
            }
        });

        // 中继入站循环：注册后阻塞等待配对设备连接；中继仅转发随后的
        // TLS 字节流，仍由现有 handle_inbound 做证书指纹校验。
        if let Some(relay_addr) = shared.relay_addr.clone() {
            let relay_shared = shared.clone();
            let relay_acceptor = TlsAcceptor::from(tls::server_config(&identity)?);
            tokio::spawn(async move {
                loop {
                    match accept_via_relay(&relay_addr, &relay_shared.identity.fingerprint).await {
                        Ok(stream) => {
                            if let Err(e) = handle_inbound(
                                relay_shared.clone(),
                                relay_acceptor.clone(),
                                stream,
                                None,
                            )
                            .await
                            {
                                relay_shared.log(&format!("中继入站连接结束：{e}"));
                            }
                        }
                        Err(e) => {
                            relay_shared.log(&format!("中继注册失败：{e}"));
                            tokio::time::sleep(Duration::from_secs(3)).await;
                        }
                    }
                }
            });
        }

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

        shared.start_transfer_watchdog();

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

    /// 供压测工具把本端 fp 写入对端 `Peer.last_addr`（不依赖 mDNS）。
    /// 仅在测试 / tools 中使用；生产路径仍走 mDNS / pair_with。
    pub fn seed_peer_addr(&self, fingerprint: &str, addr: &str) {
        let _ = self.shared.peers.upsert(Peer {
            name: fingerprint.chars().take(8).collect(),
            fingerprint: fingerprint.to_string(),
            last_addr: Some(addr.to_string()),
            last_seen_ms: None,
        });
    }

    pub fn peers(&self) -> Vec<Peer> {
        self.shared.peers.list()
    }

    /// 当前活跃出/入站连接数（压测脚本用来确认三端全互联）。
    pub fn connected_count(&self) -> usize {
        self.shared
            .connected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn remove_peer(&self, fp_prefix: &str) -> Result<bool, String> {
        self.shared.peers.remove(fp_prefix)
    }

    pub fn connected_fingerprints(&self) -> Vec<String> {
        self.shared
            .connected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
        let _ = self.shared.outgoing.send(Outbound::Text {
            text: text.to_string(),
            msg_id: new_msg_id(),
            sent_at_ms: now_ms(),
        });
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

    /// 向所有已连接且协商了 search-v1 的对端广播搜索请求。
    pub fn send_search_request(&self, query: &str, req_id: String) {
        let query = query.trim();
        // 空查询或对超长查询直接忽略，避免无效请求占用对端历史库查询资源。
        if query.is_empty() || query.len() > 200 {
            return;
        }
        // 无活跃连接时发送失败属正常，静默
        let _ = self.shared.outgoing.send(Outbound::SearchRequest {
            query: query.to_string(),
            req_id,
        });
    }

    /// 注入本端剪贴板历史搜索回调；可在服务启动后随时调用。
    pub fn set_search_handler(&self, handler: SearchHandler) {
        *self
            .shared
            .search_handler
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(handler);
    }

    /// 以文件形式发送给所有已连接且协商了 file-v1 的对端（无 v3 对端时
    /// 自动退化为 v2 的 `ClipFile` 广播）。整个流程在单条 tokio task 中
    /// 走完，不占额外线程；进度/终态通过 `Incoming::FileProgress` /
    /// `Incoming::FileTransferDone` 通知宿主。
    ///
    /// 返回的 msg_id 用于关联后续进度事件。
    ///
    /// 与既有 `send_file(name, mime, data)` 共存：后者是 v2 ClipFile 广播
    /// 入口（`ClipboardSyncService.broadcast_file` 使用），不作改动。
    pub fn send_file_path(&self, path: &Path) -> Result<String, SendErr> {
        send_file_impl(self.shared.clone(), path.to_path_buf())
    }

    /// 当前出站/入站传输的状态副本（宿主 UI 轮询用）。
    pub fn transfer_state(&self, msg_id: &str) -> Option<FileSendState> {
        self.shared.transfer_state(msg_id)
    }

    /// 在接收侧注入 Offer 确认回调：宿主弹系统对话框 / Toast，返回
    /// true=接受 false=拒绝。传 None 时回到默认策略
    /// （体量 + MIME 白名单自动收），传 Some 时由宿主决策。
    pub fn set_file_confirm_handler(&self, handler: Option<FileConfirmFn>) {
        *self
            .shared
            .file_confirm
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = handler;
    }

    /// 当前已弹给宿主但尚未收到最终决策的 Offer 列表；宿主
    /// （例如 Android 侧的 BroadcastReceiver）用 transfer_id 反查
    /// 用户点击对应的是哪一笔。
    pub fn file_pending_offers(&self) -> Vec<(u64, FileOfferPrompt)> {
        self.shared
            .pending_offers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(id, p)| (*id, p.clone()))
            .collect()
    }

    /// 测试用：将入站文件落盘目录 override 到指定路径；生产代码无需调用。
    #[doc(hidden)]
    pub fn set_file_recv_dir_override(&self, dir: Option<PathBuf>) {
        *self
            .shared
            .file_recv_dir_override
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = dir;
    }
}

/// 文件发送侧错误：仅包含同步词法/大小/打开失败等启动即知的问题。
/// 传输过程中出现的对端拒绝、超时、sha256 不一致等以 `Incoming::FileTransferDone`
/// 上报。
#[derive(Debug, Clone, PartialEq)]
pub enum SendErr {
    /// 路径无法读为普通文件（不存在、不是文件等）。
    Io(String),
    /// 文件超过 64 MB 上限。
    TooLarge(u64),
    /// 无法从路径提取合法文件名。
    BadName,
}

impl std::fmt::Display for SendErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendErr::Io(e) => write!(f, "读取文件失败: {e}"),
            SendErr::TooLarge(n) => write!(f, "文件超过 64MB 上限: {n}"),
            SendErr::BadName => write!(f, "无法从路径取文件名"),
        }
    }
}

impl std::error::Error for SendErr {}

/// 构建文件传输任务并返回 msg_id。
///
/// 流程：读盘 + sha256 + 分块 base64 → 广播 Offer → 30s 内等 Accept/Decline
/// → 按序推送 Chunk → 发 Done → 60s 静默超时由 watchdog 判定。
/// 全程只用既有 outgoing broadcast；接收端对 Offer 的应答会以 duplex 收
/// 到的 FileAccept/Decline/Ack 形式回流到 Shared::transfers（由 duplex
/// 更新状态并 emit Incoming::FileTransferDone）。
fn send_file_impl(shared: Arc<Shared>, path: PathBuf) -> Result<String, SendErr> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .ok_or(SendErr::BadName)?
        .to_string();
    let mut file = std::fs::File::open(&path).map_err(|e| SendErr::Io(e.to_string()))?;
    let size = file
        .metadata()
        .map_err(|e| SendErr::Io(e.to_string()))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(SendErr::TooLarge(size));
    }

    let msg_id = new_msg_id();
    let chunk_bytes: u32 = 64 * 1024;
    let name_for_log = name.clone();
    let shared_for_task = shared.clone();
    let msg_id_for_task = msg_id.clone();
    tokio::spawn(async move {
        let ret = tokio::task::spawn_blocking(move || {
            // 一次性读完 + sha256 + 分块 base64；64MB 内可接受，
            // 且与 `ClipFile` 既有路径一致，避免在大文件前引入流式编码复杂度。
            let mut hasher = Sha256::new();
            let mut buf = Vec::with_capacity(size as usize);
            file.read_to_end(&mut buf)
                .map_err(|e| format!("读文件失败: {e}"))?;
            hasher.update(&buf);
            let sha256 = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let chunks: Vec<String> = buf
                .chunks(chunk_bytes as usize)
                .map(|c| base64::engine::general_purpose::STANDARD.encode(c))
                .collect();
            Ok::<_, String>((buf.len() as u64, sha256, chunks))
        })
        .await;
        let (actual_size, sha256, chunks) = match ret {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                shared_for_task.log(&format!("文件 {name_for_log} 读取失败：{e}"));
                return;
            }
            Err(e) => {
                shared_for_task.log(&format!("文件 {name_for_log} 读取任务失败：{e}"));
                return;
            }
        };
        if actual_size != size {
            // 元数据与实读不一致（文件被截短/替换）仍以实读为准。
            shared_for_task.log(&format!(
                "文件 {name_for_log} 大小从 {size} 变为 {actual_size}"
            ));
        }

        // 记 OfferSent + offer_sent_at，让 watchdog 30s 后判定「对方未响应」。
        shared_for_task.set_transfer(
            &msg_id_for_task,
            FileSendState::OfferSent {
                msg_id: msg_id_for_task.clone(),
            },
        );
        shared_for_task
            .offer_sent_at
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(msg_id_for_task.clone(), Instant::now());

        let offer = Message::FileOffer {
            msg_id: msg_id_for_task.clone(),
            name: name_for_log.clone(),
            size: actual_size,
            mime: "application/octet-stream".into(),
            sha256: sha256.clone(),
            chunk_bytes,
        };
        if shared_for_task
            .outgoing
            .send(Outbound::FileWire(Box::new(offer)))
            .is_err()
        {
            shared_for_task.log(&format!("无对端可接收文件 {name_for_log}，已忽略"));
            return;
        }

        // 等 Accept / Decline：最多 30s（与 watchdog 相同节奏，提前 5s 粒度轮询）。
        let accept_deadline = Instant::now() + Duration::from_secs(30);
        'wait_accept: loop {
            let state = shared_for_task.transfer_state(&msg_id_for_task);
            match state {
                Some(FileSendState::Declined { reason, .. }) => {
                    let _ = shared_for_task
                        .incoming
                        .send(Incoming::FileTransferDone {
                            msg_id: msg_id_for_task.clone(),
                            name: name_for_log.clone(),
                            ok: false,
                            detail: Err(format!("对方拒绝：{reason}")),
                        })
                        .await;
                    return;
                }
                Some(FileSendState::Streaming { .. }) => break 'wait_accept,
                Some(FileSendState::OfferSent { .. }) => {
                    if Instant::now() > accept_deadline {
                        // watchdog 会标 Failed 并发事件，这里直接退出。
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                _ => {
                    // Failed/其它已由 watchdog 处理
                    return;
                }
            }
        }

        // 按序推 Chunk；每发一块登记流式进度时间戳，供 watchdog 检测停滞。
        // 节流：每 4 块让出一次调度（≈2MB/s 量级），既避免把对端压垮，
        // 也给接收端 FileProgress 与人工破坏/调试窗口留下缝隙。
        let total = chunks.len();
        for (idx, data) in chunks.into_iter().enumerate() {
            let last = idx + 1 == total;
            let chunk = Message::FileChunk {
                msg_id: msg_id_for_task.clone(),
                offset: (idx as u64) * (chunk_bytes as u64),
                data,
                last,
            };
            if shared_for_task
                .outgoing
                .send(Outbound::FileWire(Box::new(chunk)))
                .is_err()
            {
                shared_for_task.log(&format!(
                    "发送块 {idx}/{total} 时广播通道已闭，中止 {name_for_log}"
                ));
                return;
            }
            if idx % 4 == 3 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            let sent_bytes: u64 = ((idx + 1) as u64) * (chunk_bytes as u64).min(actual_size);
            shared_for_task.set_transfer(
                &msg_id_for_task,
                FileSendState::Streaming {
                    msg_id: msg_id_for_task.clone(),
                    sent_bytes,
                },
            );
            shared_for_task
                .stream_progress_at
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(msg_id_for_task.clone(), Instant::now());
        }
        let done = Message::FileDone {
            msg_id: msg_id_for_task.clone(),
            sha256,
        };
        let _ = shared_for_task
            .outgoing
            .send(Outbound::FileWire(Box::new(done)));
        shared_for_task.mark_stream_done(&msg_id_for_task);
    });
    Ok(msg_id)
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
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
        // 双端重连会同时发生。按稳定指纹只让较小的一端发起，避免两条
        // 交叉连接各自占住本地去重位后又互相断开。
        if shared.identity.fingerprint > peer.fingerprint {
            continue;
        }
        let already = shared
            .connected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&peer.fingerprint)
            .copied();
        let direct_addr = match (cached, &peer.last_addr) {
            (Some(a), _) => Some(a.to_string()),
            (None, Some(a)) => Some(a.clone()),
            (None, None) => None,
        };
        let relay_addr = shared.relay_addr.clone();
        if direct_addr.is_none() && relay_addr.is_none() {
            continue;
        }
        let shared = shared.clone();
        let connector = connector.clone();
        let fp = peer.fingerprint.clone();
        tokio::spawn(async move {
            let result = match direct_addr {
                Some(addr) => match connect_peer(shared.clone(), connector.clone(), &peer, &addr)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(direct_error) => match relay_addr.as_deref() {
                        Some(relay) => {
                            shared.log(&format!(
                                "直连 {}（{addr}）失败，改经中继 {relay}：{direct_error}",
                                peer.name
                            ));
                            connect_peer_via_relay(shared.clone(), connector, &peer, relay).await
                        }
                        None => Err(direct_error),
                    },
                },
                None => {
                    connect_peer_via_relay(
                        shared.clone(),
                        connector,
                        &peer,
                        relay_addr.as_deref().expect("已确认存在中继地址"),
                    )
                    .await
                }
            };
            if let Err(e) = result {
                shared.log(&format!("连接 {} 失败：{e}", peer.name));
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
    connect_peer_stream(shared, connector, peer, stream, Some(addr)).await
}

/// 经中继建立出站数据流；TLS 与后续消息处理与直连完全相同。
async fn connect_peer_via_relay(
    shared: Arc<Shared>,
    connector: TlsConnector,
    peer: &Peer,
    relay_addr: &str,
) -> Result<(), String> {
    let stream = tokio::time::timeout(
        CONNECT_TIMEOUT,
        connect_via_relay(relay_addr, &shared.identity.fingerprint, &peer.fingerprint),
    )
    .await
    .map_err(|_| {
        format!(
            "中继连接超时（{}s）：{relay_addr}",
            CONNECT_TIMEOUT.as_secs()
        )
    })??;
    connect_peer_stream(shared, connector, peer, stream, None).await
}

/// 对任意已建立的 TCP 流执行既有 TLS、身份校验与同步消息循环。
async fn connect_peer_stream(
    shared: Arc<Shared>,
    connector: TlsConnector,
    peer: &Peer,
    stream: TcpStream,
    direct_addr: Option<&str>,
) -> Result<(), String> {
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
            listen_port: shared.local_port,
            features: crate::protocol::local_features(),
            protocol_version: crate::protocol::PROTOCOL_VERSION,
        },
    )
    .await?;
    let (Message::Hello { name, features, .. }, format) = read_msg_with_format(&mut tls).await?
    else {
        return Err("对端未按协议发送 Hello".into());
    };
    if let Some(addr) = direct_addr {
        let _ = shared.peers.update_addr(&fp, addr);
    }
    shared.clear_backoff(&fp);
    duplex(shared, tls, fp, name, format, features).await
}

/// 入站连接：已配对 → 数据通道；未配对 → 配对流程（需确认回调）。
async fn handle_inbound(
    shared: Arc<Shared>,
    acceptor: TlsAcceptor,
    stream: TcpStream,
    direct_addr: Option<SocketAddr>,
) -> Result<(), String> {
    let tls = acceptor
        .accept(stream)
        .await
        .map_err(|e| format!("TLS 接受失败: {e}"))?;
    let fp = peer_fingerprint(tls.get_ref().1)?;

    let mut tls = tls;
    let (
        Message::Hello {
            name,
            fingerprint,
            listen_port,
            features,
            ..
        },
        format,
    ) = read_msg_with_format(&mut tls).await?
    else {
        return Err("对端未按协议发送 Hello".into());
    };
    if fingerprint != fp {
        return Err("Hello 指纹与证书不符".into());
    }
    write_msg_with_format(
        &mut tls,
        &Message::Hello {
            name: shared.device_name.clone(),
            fingerprint: shared.identity.fingerprint.clone(),
            listen_port: shared.local_port,
            features: crate::protocol::local_features(),
            protocol_version: crate::protocol::PROTOCOL_VERSION,
        },
        format,
    )
    .await?;

    let direct_addr = direct_addr.map(|addr| SocketAddr::new(addr.ip(), listen_port).to_string());
    if shared.peers.contains(&fp) {
        if let Some(addr) = direct_addr.as_deref() {
            let _ = shared.peers.update_addr(&fp, addr);
        }
        return duplex(shared, tls, fp, name, format, features).await;
    }

    // 未配对：走配对确认
    let Some(confirm) = shared.accept_confirm.clone() else {
        write_msg_with_format(&mut tls, &Message::PairReject, format)
            .await
            .ok();
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
        write_msg_with_format(&mut tls, &Message::PairReject, format)
            .await
            .ok();
        return Err(format!("用户拒绝与 {name} 配对"));
    }
    write_msg_with_format(&mut tls, &Message::PairConfirm, format).await?;
    match read_msg_with_format(&mut tls).await?.0 {
        Message::PairConfirm => {}
        _ => return Err("对端未确认配对".into()),
    }
    shared.peers.upsert(Peer {
        name: name.clone(),
        fingerprint: fp.clone(),
        last_addr: direct_addr,
        last_seen_ms: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        ),
    })?;
    shared.log(&format!("已与 {name} 配对"));
    duplex(shared, tls, fp, name, format, features).await
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
            listen_port: shared.local_port,
            features: crate::protocol::local_features(),
            protocol_version: crate::protocol::PROTOCOL_VERSION,
        },
    )
    .await?;
    let Message::Hello {
        name, fingerprint, ..
    } = read_msg(&mut tls).await?
    else {
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
        last_seen_ms: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        ),
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
    tls: S,
    fp: String,
    peer_name: String,
    format: FrameFormat,
    peer_features: Vec<String>,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    {
        let mut connected = shared
            .connected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !connected.insert(fp.clone()) {
            // 双向同时建连的竞态：保留已有连接
            return Ok(());
        }
    }
    let peer_has_msg_id =
        crate::protocol::peer_supports(&peer_features, crate::protocol::FEATURE_MSG_ID_V1);
    let peer_has_search =
        crate::protocol::peer_supports(&peer_features, crate::protocol::FEATURE_SEARCH_V1);
    let peer_has_file_v1 =
        crate::protocol::peer_supports(&peer_features, crate::protocol::FEATURE_FILE_V1);
    shared.log(&format!(
        "已连接 {peer_name}（协议 {}：{} msg_id）",
        if peer_has_msg_id { "v2" } else { "v1" },
        if peer_has_msg_id { "启用" } else { "关闭" },
    ));

    // S6：读侧放进独立任务，写侧继续在 duplex 主循环处理。
    //
    // 此前同一个 tokio::select! 同时 poll 读帧和 Ping tick，Ping 一旦到点会
    // 取消正在进行的 read_msg_with_format，已消费的半帧数据从 TLS 流中丢失，
    // 下一轮从帧体中间解析长度 → “对端帧过大”断连。拆分后读任务不会被写侧
    // Ping 取消，大帧可以完整读完；读任务需要回写的应答经 reply_tx 交给本循环。
    let (read_half, write_half) = tokio::io::split(tls);
    let (reply_tx, mut reply_rx) = mpsc::channel::<Message>(64);

    let mut read_task = tokio::spawn(duplex_read_loop(
        shared.clone(),
        read_half,
        reply_tx,
        fp.clone(),
        peer_name.clone(),
        peer_has_search,
    ));

    let mut write_half = write_half;
    let mut rx = shared.outgoing.subscribe();
    let mut ping = tokio::time::interval(HEARTBEAT_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let result = loop {
        tokio::select! {
            out = rx.recv() => match out {
                Ok(Outbound::Text {
                    text,
                    msg_id,
                    sent_at_ms,
                }) => {
                    // 记账最近发出的 (msg_id, sent_at_ms) 用于回声剔除；窗口短而
                    // 浅，跟随广播吞吐，64 条已覆盖 32 字/秒级聊天强度。
                    {
                        let mut recent = shared.recent_out.lock().unwrap_or_else(|p| p.into_inner());
                        if recent.len() >= 64 {
                            recent.remove(0);
                        }
                        recent.push((msg_id.clone(), sent_at_ms));
                    }
                    let msg = Message::ClipText {
                        text,
                        sent_at_ms,
                        msg_id: if peer_has_msg_id { Some(msg_id) } else { None },
                        origin_device_fp: if peer_has_msg_id {
                            Some(shared.identity.fingerprint.clone())
                        } else {
                            None
                        },
                    };
                    if let Err(e) = write_msg_with_format(&mut write_half, &msg, format).await {
                        break Err(e);
                    }
                }
                Ok(Outbound::Image(png)) => {
                    let data = base64::engine::general_purpose::STANDARD.encode(&png);
                    let msg = Message::ClipImage {
                        data,
                        sent_at_ms: now_ms(),
                        msg_id: None,
                        origin_device_fp: None,
                    };
                    if let Err(e) = write_msg_with_format(&mut write_half, &msg, format).await {
                        break Err(e);
                    }
                }
                Ok(Outbound::File { name, mime_type, data }) => {
                    let msg = Message::ClipFile {
                        name,
                        mime_type,
                        data: base64::engine::general_purpose::STANDARD.encode(&data),
                        sent_at_ms: now_ms(),
                        msg_id: None,
                        origin_device_fp: None,
                    };
                    if let Err(e) = write_msg_with_format(&mut write_half, &msg, format).await {
                        break Err(e);
                    }
                }
                Ok(Outbound::SearchRequest { query, req_id }) => {
                    // 仅向协商了 search-v1 的对端发送，避免老端反序列化失败断连。
                    if !peer_has_search {
                        shared.log("对端不支持 search-v1，跳过搜索");
                        continue;
                    }
                    let msg = Message::SearchRequest {
                        query,
                        req_id: Some(req_id),
                    };
                    if let Err(e) = write_msg_with_format(&mut write_half, &msg, format).await {
                        break Err(e);
                    }
                }
                Ok(Outbound::FileWire(msg)) => {
                    // 文件 v3 控制/数据面消息：file-v1 协商后才允许上路，
                    // 保证 v2 对端不会因收到未知变体而断开。
                    if !peer_has_file_v1 {
                        continue;
                    }
                    if let Err(e) = write_msg_with_format(&mut write_half, &msg, format).await {
                        break Err(e);
                    }
                }
                // 落后于广播积压时跳过旧消息继续
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break Ok(()),
            },
            reply = reply_rx.recv() => match reply {
                Some(msg) => {
                    if let Err(e) = write_msg_with_format(&mut write_half, &msg, format).await {
                        break Err(e);
                    }
                }
                // 读任务结束后 reply_tx 会关闭；真正的结束原因由 read_task 分支上报。
                None => continue,
            },
            _ = ping.tick() => {
                if let Err(e) = write_msg_with_format(&mut write_half, &Message::Ping, format).await {
                    break Err(e);
                }
            }
            r = &mut read_task => {
                break match r {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(e),
                    Err(e) => Err(format!("读任务失败: {e}")),
                };
            }
        }
    };

    // 主循环退出（对端断开/写失败/广播关闭）时，确保读任务也停止，
    // 否则它会继续持有 ReadHalf 在后台空转。
    read_task.abort();

    shared
        .connected
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&fp);
    shared.log(&format!("连接 {peer_name} 已断开"));
    result
}

/// 读侧独立任务：只从 TLS 读半部读帧，不回写 TLS。
///
/// 需要回写的应答（FileAccept/Decline、FileProgress、FileAck、SearchResponse）
/// 统一经 `reply_tx` 交给 duplex 主循环写，避免读任务与写侧共享同一 `&mut tls`
/// 或让 select! 取消半截读帧。
async fn duplex_read_loop<S>(
    shared: Arc<Shared>,
    mut read_half: tokio::io::ReadHalf<S>,
    reply_tx: mpsc::Sender<Message>,
    fp: String,
    peer_name: String,
    peer_has_search: bool,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    loop {
        let incoming =
            tokio::time::timeout(READ_IDLE_TIMEOUT, read_msg_with_format(&mut read_half)).await;
        match incoming {
            Ok(Ok((
                Message::ClipText {
                    text,
                    sent_at_ms,
                    msg_id,
                    origin_device_fp,
                },
                _,
            ))) => {
                // 回声丢弃：本端指纹即对端 `origin_device_fp` 时，说明是
                // 本端先前发出、经对端转发回来的副本，不再上屏也不入库。
                if origin_device_fp.as_deref() == Some(shared.identity.fingerprint.as_str()) {
                    continue;
                }
                // 与旧端互通：旧端仍会 echo 本端消息但 `origin_device_fp`
                // 为 None。用 (msg_id, sent_at_ms) 与本端最后几条出站消息的
                // 组合做短窗口复读剔除，近似 LWW（last-writer-wins）。
                if let Some(mid) = msg_id.as_deref() {
                    if shared
                        .recent_out
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .iter()
                        .any(|(o_mid, o_sent)| {
                            o_mid == mid && (*o_sent - sent_at_ms).abs() <= 5_000
                        })
                    {
                        continue;
                    }
                }
                if text.len() <= MAX_CLIP_TEXT {
                    let _ = shared
                        .incoming
                        .send(Incoming::Clip {
                            from_name: peer_name.clone(),
                            text,
                            sent_at_ms: Some(sent_at_ms),
                            msg_id,
                        })
                        .await;
                }
            }
            Ok(Ok((Message::ClipImage { data, .. }, _))) => {
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
            Ok(Ok((
                Message::ClipFile {
                    name,
                    mime_type,
                    data,
                    ..
                },
                _,
            ))) => match base64::engine::general_purpose::STANDARD.decode(&data) {
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
            },
            Ok(Ok((
                Message::FileOffer {
                    msg_id,
                    name,
                    size,
                    mime,
                    sha256,
                    chunk_bytes,
                },
                _,
            ))) => {
                // 64MB 上限先测：超出立即拒。
                if size > MAX_FILE_BYTES {
                    let reply = Message::FileDecline {
                        msg_id: msg_id.clone(),
                        reason: "too_large".into(),
                    };
                    reply_tx
                        .send(reply)
                        .await
                        .map_err(|e| format!("回写通道关闭: {e}"))?;
                    continue;
                }
                // 让宿主无论决策如何都能看到 Offer 事件（UI 渲染/日志）。
                let _ = shared
                    .incoming
                    .send(Incoming::FileOffer {
                        from_name: peer_name.clone(),
                        msg_id: msg_id.clone(),
                        name: name.clone(),
                        size,
                        mime: mime.clone(),
                        sha256: sha256.clone(),
                        chunk_bytes,
                    })
                    .await;
                let accepted = {
                    let cb = shared
                        .file_confirm
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    match cb {
                        Some(f) => {
                            // 生成单调 transfer_id 并入表，便于宿主把“用户
                            // 稍后点击的通知按钮”回指到本条 Offer。
                            let transfer_id = next_file_offer_id();
                            let prompt = FileOfferPrompt {
                                transfer_id,
                                peer_fp: fp.clone(),
                                from_name: peer_name.clone(),
                                name: name.clone(),
                                size,
                                mime: mime.clone(),
                            };
                            shared
                                .pending_offers
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .insert(transfer_id, prompt.clone());
                            // 宿主回调可能阻塞（等待用户点击通知上的
                            // 接受/拒绝），放进 spawn_blocking，避免
                            // 阻塞 duplex 主循环。
                            let decision = {
                                let prompt_for_cb = prompt.clone();
                                tokio::task::spawn_blocking(move || f(prompt_for_cb))
                                    .await
                                    .unwrap_or(false)
                            };
                            shared
                                .pending_offers
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .remove(&transfer_id);
                            decision
                        }
                        None => size < FILE_AUTO_ACCEPT_MAX && mime_is_whitelisted(&mime),
                    }
                };
                if !accepted {
                    let reply = Message::FileDecline {
                        msg_id: msg_id.clone(),
                        reason: "user_declined".into(),
                    };
                    reply_tx
                        .send(reply)
                        .await
                        .map_err(|e| format!("回写通道关闭: {e}"))?;
                    continue;
                }
                // 同意：准备 .part 落盘路径并 touch（让测试/宿主立刻可见）。
                let part_path = incoming_file_part_path(&shared, &msg_id, &name);
                if part_path.is_none() {
                    let reply = Message::FileDecline {
                        msg_id: msg_id.clone(),
                        reason: "io_error".into(),
                    };
                    reply_tx
                        .send(reply)
                        .await
                        .map_err(|e| format!("回写通道关闭: {e}"))?;
                    continue;
                }
                let part_path = part_path.unwrap();
                if let Some(parent) = part_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                // touch 空 .part：首个 Chunk 追加之前界面/测试即可观察。
                let _ = fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(false)
                    .open(&part_path);
                {
                    let mut recv = shared.file_recv.lock().unwrap_or_else(|p| p.into_inner());
                    recv.insert(
                        msg_id.clone(),
                        FileRecvState {
                            from_name: peer_name.clone(),
                            name: name.clone(),
                            size,
                            mime: mime.clone(),
                            sha256: sha256.clone(),
                            received: 0,
                            part_path: part_path.clone(),
                            last_chunk_at: Instant::now(),
                        },
                    );
                }
                let reply = Message::FileAccept { msg_id };
                reply_tx
                    .send(reply)
                    .await
                    .map_err(|e| format!("回写通道关闭: {e}"))?;
            }
            Ok(Ok((Message::FileAccept { msg_id }, _))) => {
                // 出站方向：对端同意，切 Streaming 并唤醒等待循环。
                shared.set_transfer(
                    &msg_id,
                    FileSendState::Streaming {
                        msg_id: msg_id.clone(),
                        sent_bytes: 0,
                    },
                );
                shared
                    .stream_progress_at
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(msg_id, Instant::now());
            }
            Ok(Ok((Message::FileDecline { msg_id, reason }, _))) => {
                shared.set_transfer(
                    &msg_id,
                    FileSendState::Declined {
                        msg_id: msg_id.clone(),
                        reason,
                    },
                );
            }
            Ok(Ok((
                Message::FileChunk {
                    msg_id,
                    offset,
                    data,
                    last,
                },
                _,
            ))) => {
                let _ = last; // 是否最后一块由 FileDone 落地，块本身无需标记
                let bytes = match base64::engine::general_purpose::STANDARD.decode(&data) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                enum ChunkStep {
                    Continue,
                    Progress(u64),
                    Reject(&'static str),
                }
                let step = {
                    let mut recv = shared.file_recv.lock().unwrap_or_else(|p| p.into_inner());
                    match recv.get_mut(&msg_id) {
                        Some(st) => {
                            if offset != st.received {
                                recv.remove(&msg_id);
                                ChunkStep::Reject("chunk_out_of_order")
                            } else {
                                let mut opened = std::fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(&st.part_path);
                                match opened.as_mut().map(|f| {
                                    use std::io::Write;
                                    f.write_all(&bytes)
                                }) {
                                    Ok(Ok(())) => {
                                        st.received += bytes.len() as u64;
                                        st.last_chunk_at = Instant::now();
                                        // 节流：每 16 块（≈1MB @ 64KB）或刚好收满时回 progress
                                        if st.received % (64 * 1024 * 16) < 64 * 1024
                                            || st.received >= st.size
                                        {
                                            ChunkStep::Progress(st.received)
                                        } else {
                                            ChunkStep::Continue
                                        }
                                    }
                                    _ => {
                                        recv.remove(&msg_id);
                                        ChunkStep::Reject("io_error")
                                    }
                                }
                            }
                        }
                        None => ChunkStep::Continue,
                    }
                };
                match step {
                    ChunkStep::Continue => {}
                    ChunkStep::Progress(received) => {
                        let progress = Message::FileProgress {
                            msg_id: msg_id.clone(),
                            received_bytes: received,
                        };
                        reply_tx
                            .send(progress)
                            .await
                            .map_err(|e| format!("回写通道关闭: {e}"))?;
                    }
                    ChunkStep::Reject(reason) => {
                        let reply = Message::FileDecline {
                            msg_id: msg_id.clone(),
                            reason: reason.into(),
                        };
                        let _ = reply_tx.send(reply).await;
                        continue;
                    }
                }
            }
            Ok(Ok((Message::FileDone { msg_id, sha256 }, _))) => {
                let (path, expected, from_name, name, size) = {
                    let recv = shared.file_recv.lock().unwrap_or_else(|p| p.into_inner());
                    match recv.get(&msg_id) {
                        Some(st) => (
                            st.part_path.clone(),
                            st.sha256.clone(),
                            st.from_name.clone(),
                            st.name.clone(),
                            st.size,
                        ),
                        None => continue,
                    }
                };
                if sha256 != expected {
                    let _ = fs::remove_file(&path);
                    let reply = Message::FileAck {
                        msg_id: msg_id.clone(),
                        received_bytes: 0,
                        ok: false,
                        error: Some("sha256_mismatch".into()),
                    };
                    let _ = reply_tx.send(reply).await;
                    shared
                        .file_recv
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&msg_id);
                    continue;
                }
                // 校验 .part 实际 sha256
                let actual = sha256_of_file(&path);
                let ok = matches!(actual.as_deref(), Some(h) if h == expected);
                if !ok {
                    let _ = fs::remove_file(&path);
                    let reply = Message::FileAck {
                        msg_id: msg_id.clone(),
                        received_bytes: 0,
                        ok: false,
                        error: Some("sha256_mismatch".into()),
                    };
                    let _ = reply_tx.send(reply).await;
                    shared
                        .file_recv
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&msg_id);
                    continue;
                }
                // 原子改名 .part → 最终文件名
                let final_path = unique_landing_path(&path, &name);
                let rename_ok = fs::rename(&path, &final_path).is_ok();
                if !rename_ok {
                    let _ = fs::remove_file(&path);
                    let reply = Message::FileAck {
                        msg_id: msg_id.clone(),
                        received_bytes: 0,
                        ok: false,
                        error: Some("io_error".into()),
                    };
                    let _ = reply_tx.send(reply).await;
                    shared
                        .file_recv
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&msg_id);
                    continue;
                }
                let reply = Message::FileAck {
                    msg_id: msg_id.clone(),
                    received_bytes: size,
                    ok: true,
                    error: None,
                };
                let _ = reply_tx.send(reply).await;
                shared
                    .file_recv
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&msg_id);
                let _ = shared
                    .incoming
                    .send(Incoming::FileTransferDone {
                        msg_id: msg_id.clone(),
                        name: name.clone(),
                        ok: true,
                        detail: Ok(size),
                    })
                    .await;
                shared.log(&format!("已保存 {name}（来自 {from_name}）"));
            }
            Ok(Ok((
                Message::FileAck {
                    msg_id,
                    received_bytes,
                    ok,
                    error,
                },
                _,
            ))) => {
                let err_detail = error.unwrap_or_else(|| "unknown".into());
                let next = if ok {
                    FileSendState::Acked {
                        msg_id: msg_id.clone(),
                        received: received_bytes,
                    }
                } else {
                    FileSendState::Failed {
                        msg_id: msg_id.clone(),
                        error: err_detail.clone(),
                    }
                };
                shared.set_transfer(&msg_id, next);
                let _ = shared
                    .incoming
                    .send(Incoming::FileTransferDone {
                        msg_id: msg_id.clone(),
                        name: String::new(),
                        ok,
                        detail: if ok {
                            Ok(received_bytes)
                        } else {
                            Err(err_detail)
                        },
                    })
                    .await;
            }
            Ok(Ok((
                Message::FileProgress {
                    msg_id,
                    received_bytes,
                },
                _,
            ))) => {
                // picker：先更新状态，再通知宿主。
                if let Some(FileSendState::Streaming { .. }) = shared.transfer_state(&msg_id) {
                    shared.set_transfer(
                        &msg_id,
                        FileSendState::Streaming {
                            msg_id: msg_id.clone(),
                            sent_bytes: received_bytes,
                        },
                    );
                    shared
                        .stream_progress_at
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(msg_id.clone(), Instant::now());
                }
                let _ = shared
                    .incoming
                    .send(Incoming::FileProgress {
                        msg_id,
                        received_bytes,
                    })
                    .await;
            }
            Ok(Ok((Message::Ping, _))) => {}
            Ok(Ok((Message::SearchRequest { query, req_id }, _))) => {
                // 仅在对端协商了 search-v1 时响应：老端不认识该消息，
                // 不会发送；这里多一层判定是防止对端特性表与实际行为不一致。
                if peer_has_search {
                    let handler = shared
                        .search_handler
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .clone();
                    // 未注入回调时返回空列表而不是出错：宿主可能尚未就绪。
                    let hits = handler
                        .map(|h| h(&query))
                        .unwrap_or_default()
                        .into_iter()
                        .take(8)
                        .map(|mut hit| {
                            // 预览截断到 200 字符，避免超长条目撑爆查询响应帧预算。
                            hit.text = hit.text.chars().take(200).collect();
                            hit
                        })
                        .collect();
                    let reply = Message::SearchResponse { req_id, hits };
                    reply_tx
                        .send(reply)
                        .await
                        .map_err(|e| format!("回写通道关闭: {e}"))?;
                }
            }
            Ok(Ok((Message::SearchResponse { req_id, hits }, _))) => {
                // 命中数截断到 8 做防御：不信任对端自觉守约。
                let hits = hits.into_iter().take(8).collect();
                let _ = shared
                    .incoming
                    .send(Incoming::SearchResults {
                        from_name: peer_name.clone(),
                        req_id,
                        hits,
                    })
                    .await;
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(format!(
                    "对端 {peer_name} {} 秒无响应，判定连接失效",
                    READ_IDLE_TIMEOUT.as_secs()
                ))
            }
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 接收侧白名单：满足 (size < 5MB 且 mime 属于列表) + 无用户回调时自动接受。
fn mime_is_whitelisted(mime: &str) -> bool {
    mime.starts_with("image/")
        || mime.starts_with("text/")
        || mime == "application/pdf"
        || mime == "application/zip"
}

/// 入站 .part 落盘路径：测试 override 优先，否则 Downloads/shurufa。
fn incoming_file_part_path(shared: &Shared, msg_id: &str, name: &str) -> Option<PathBuf> {
    let base = shared
        .file_recv_dir_override
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .or_else(default_file_landing_dir)?;
    let safe_name = Path::new(name).file_name()?.to_str()?;
    if safe_name.is_empty() || safe_name == "." || safe_name == ".." {
        return None;
    }
    Some(base.join(format!("{msg_id}.part")))
}

/// 默认文件落地目录：优先系统 Downloads，其次家目录；测试通过 override 覆盖。
fn default_file_landing_dir() -> Option<PathBuf> {
    // 读取 USERPROFILE（Windows）或 HOME，附加 Downloads\shurufa
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))?;
    Some(base.join("Downloads").join("shurufa"))
}

/// 同目录内生成不冲突的最终文件名：name.ext → name (2).ext → name (3).ext。
fn unique_landing_path(part: &Path, name: &str) -> PathBuf {
    let dir = part.parent().map(Path::to_path_buf).unwrap_or_default();
    let name_path = Path::new(name);
    let stem = name_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = name_path.extension().and_then(|s| s.to_str()).unwrap_or("");
    for i in 0..=99 {
        let candidate = match i {
            0 => dir.join(name),
            n => {
                if ext.is_empty() {
                    dir.join(format!("{stem} ({n})"))
                } else {
                    dir.join(format!("{stem} ({n}).{ext}"))
                }
            }
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}

/// 计算文件 sha256；读失败返回 None。
fn sha256_of_file(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    hasher.update(&buf);
    Some(
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::serve_listener;

    async fn wait_connected(first: &SyncService, second: &SyncService) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !first.connected_fingerprints().is_empty()
                    && !second.connected_fingerprints().is_empty()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("两端应在中继上建立连接");
    }

    #[test]
    fn 中继配置会持久化并拒绝无效地址() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_relay_addr(dir.path()), None);
        save_relay_addr(dir.path(), Some("relay.example.com:48633")).unwrap();
        assert_eq!(
            load_relay_addr(dir.path()).as_deref(),
            Some("relay.example.com:48633")
        );
        assert!(save_relay_addr(dir.path(), Some("没有端口")).is_err());
        save_relay_addr(dir.path(), None).unwrap();
        assert_eq!(load_relay_addr(dir.path()), None);
    }

    #[tokio::test]
    async fn 已配对设备可仅经中继双向同步文本() {
        let relay_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = relay_listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let _ = serve_listener(relay_listener).await;
        });

        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let first_identity = DeviceIdentity::load_or_create(first_dir.path(), "甲设备").unwrap();
        let second_identity = DeviceIdentity::load_or_create(second_dir.path(), "乙设备").unwrap();
        PeerStore::open(first_dir.path())
            .unwrap()
            .upsert(Peer {
                name: "乙设备".into(),
                fingerprint: second_identity.fingerprint.clone(),
                last_addr: None,
                last_seen_ms: None,
            })
            .unwrap();
        PeerStore::open(second_dir.path())
            .unwrap()
            .upsert(Peer {
                name: "甲设备".into(),
                fingerprint: first_identity.fingerprint.clone(),
                last_addr: None,
                last_seen_ms: None,
            })
            .unwrap();

        let (first_tx, mut first_rx) = mpsc::channel(4);
        let (second_tx, mut second_rx) = mpsc::channel(4);
        let mut first_config = SyncConfig::new(first_dir.path().into(), "甲设备".into());
        first_config.port = 0;
        first_config.enable_mdns = false;
        first_config.reconnect_secs = 1;
        first_config.relay_addr = Some(relay_addr.clone());
        let mut second_config = SyncConfig::new(second_dir.path().into(), "乙设备".into());
        second_config.port = 0;
        second_config.enable_mdns = false;
        second_config.reconnect_secs = 1;
        second_config.relay_addr = Some(relay_addr);

        let first = SyncService::start(first_config, first_tx, None, Box::new(|_| {}))
            .await
            .unwrap();
        let second = SyncService::start(second_config, second_tx, None, Box::new(|_| {}))
            .await
            .unwrap();
        wait_connected(&first, &second).await;

        first.send_clip("仅经中继的同步文本");
        match tokio::time::timeout(Duration::from_secs(5), second_rx.recv())
            .await
            .unwrap()
        {
            Some(Incoming::Clip {
                from_name,
                text,
                sent_at_ms,
                msg_id,
            }) => {
                assert_eq!(from_name, "甲设备");
                assert_eq!(text, "仅经中继的同步文本");
                assert!(sent_at_ms.is_some());
                assert!(msg_id.as_deref().map(|s| s.len() >= 16).unwrap_or(false));
            }
            other => panic!("期待 Clip 消息，实际 {other:?}"),
        }
        second.send_clip("中继回传文本");
        match tokio::time::timeout(Duration::from_secs(5), first_rx.recv())
            .await
            .unwrap()
        {
            Some(Incoming::Clip {
                from_name,
                text,
                sent_at_ms,
                msg_id,
            }) => {
                assert_eq!(from_name, "乙设备");
                assert_eq!(text, "中继回传文本");
                assert!(sent_at_ms.is_some());
                assert!(msg_id.as_deref().map(|s| s.len() >= 16).unwrap_or(false));
            }
            other => panic!("期待 Clip 消息，实际 {other:?}"),
        }
        assert!(first.peers().iter().all(|peer| peer.last_addr.is_none()));
        assert!(second.peers().iter().all(|peer| peer.last_addr.is_none()));
    }

    /// 单例验证：file_confirm handler 返回 false → 发送侧终态为 Declined，
    /// 决策理由 = "user_declined"。集成行为由 tests/file_sync.rs 的
    /// `接收方拒绝时发送端进入_declined` 覆盖；这里只验证 set/clear 接口。
    #[test]
    fn 可注入与清空file_confirm_决策回调() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = SyncConfig::new(dir.path().into(), "测试机".into());
        cfg.port = 0;
        cfg.enable_mdns = false;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, _rx) = mpsc::channel(4);
        let svc = rt
            .block_on(SyncService::start(cfg, tx, None, Box::new(|_| {})))
            .expect("启动测试 SyncService");
        // 未注入时默认为 None，pending_offers 也应为空
        assert!(svc.file_pending_offers().is_empty());
        let decline: FileConfirmFn = Arc::new(|_p: FileOfferPrompt| false);
        svc.set_file_confirm_handler(Some(decline));
        // 清空后再设为 None，应回到 auto 策略（不再持有回调）。
        svc.set_file_confirm_handler(None);
        let guard = svc
            .shared
            .file_confirm
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        assert!(guard.is_none());
    }
}
