//! TSF 前端侧 IPC 客户端：连接 shurufa-algo 服务，把按键/上下文请求转发过去。
//!
//! 架构 M6 前置：引擎在独立算法服务进程里，本 DLL 只是轻量客户端。每个宿主
//! 进程持有一个会话（一个连接）。连接断开（服务崩溃/未启动）时自动重连，
//! 恢复后重建会话。

use ime_ipc::pipe::PipeClient;
use ime_ipc::{decode_response, encode_request, Request, Response};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// 单个 TSF 宿主的 IPC 客户端。`connect` 是惰性的：首次使用时才建连。
///
/// 熔断：一次失败/拒绝后，2 秒内不再尝试重连，避免每次按键都触发
/// 20×50ms=1s 自旋拉起已经死亡的算法服务（曾造成输入延迟陡涨）。
pub struct ImeClient {
    pipe: Option<PipeClient>,
    /// 上次连接尝试失败时刻（用于熔断冷却）
    last_failure: Option<std::time::Instant>,
    /// 连续失败计数（达到阈值后扩大冷却窗口）
    consecutive_failures: u32,
}

const CIRCUIT_BREAKER_COOLDOWN_MS: u64 = 2_000;
/// 连续失败次数超过该阈值后，冷却窗口翻倍至 4s
const CIRCUIT_BREAKER_BACKOFF_THRESHOLD: u32 = 3;
const CIRCUIT_BREAKER_COOLDOWN_LONG_MS: u64 = 4_000;
/// 单次 IPC 读响应超时（ms）：服务端卡死时客户端必须超时降级，
/// 绝不阻塞宿主 UI 线程（曾致"应用无响应 + 其他输入法失效"）。
/// 500ms 远低于可感知延迟，且覆盖正常引擎响应（亚毫秒级）。
const IPC_READ_TIMEOUT_MS: u64 = 500;

#[allow(dead_code)]
impl ImeClient {
    /// 尝试拉起 shurufa-algo.exe（DLL 同目录优先，其次 PATH/APPDATA）。
    fn spawn_algo_if_needed() {
        let names = [
            Self::dll_dir().map(|d| d.join("shurufa-algo.exe")),
            Some(PathBuf::from("shurufa-algo.exe")),
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("shurufa").join("shurufa-algo.exe")),
        ];
        for candidate in names.iter().flatten() {
            if candidate.exists() {
                crate::debug_log(&format!("拉起算法服务：{}", candidate.display()));
                let _ = Command::new(candidate)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                return;
            }
        }
        crate::debug_log("未找到 shurufa-algo.exe，无法自拉起");
    }

    /// 当前 DLL 所在目录。
    fn dll_dir() -> Option<PathBuf> {
        crate::dll_path().parent().map(|p| p.to_path_buf())
    }
    pub fn new() -> Self {
        ImeClient {
            pipe: None,
            last_failure: None,
            consecutive_failures: 0,
        }
    }

    /// 是否处于熔断冷却期（避免每次按键都重新尝试连接死亡的服务）。
    fn circuit_open(&self) -> bool {
        match self.last_failure {
            Some(at) => {
                let cooldown_ms = if self.consecutive_failures >= CIRCUIT_BREAKER_BACKOFF_THRESHOLD
                {
                    CIRCUIT_BREAKER_COOLDOWN_LONG_MS
                } else {
                    CIRCUIT_BREAKER_COOLDOWN_MS
                };
                at.elapsed() < std::time::Duration::from_millis(cooldown_ms)
            }
            None => false,
        }
    }

    fn note_failure(&mut self) {
        self.last_failure = Some(std::time::Instant::now());
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    fn note_success(&mut self) {
        self.last_failure = None;
        self.consecutive_failures = 0;
    }

    /// 确保已连接；若服务未就绪则尝试拉起算法服务并**单次**连接。
    ///
    /// **绝不在 UI 线程长轮询**：历史版本这里做 20×50ms=1s 阻塞轮询，
    /// algo 不在时每次按键（尤其高频的 Shift）都卡 UI 线程 1s，造成
    /// "应用无响应 + 其他输入法失效"（2026-08-12 用户反馈）。
    /// 现在：spawn 一次 + 单次 connect；失败立即返回，由熔断冷却 + supervisor
    /// 看护（崩溃自动重启）兜底，下一按键自然重试。
    fn ensure(&mut self) -> Option<&PipeClient> {
        if self.pipe.is_some() {
            return self.pipe.as_ref();
        }
        if self.circuit_open() {
            return None;
        }
        // 首次连接失败时尝试拉起算法服务（supervisor 负责常驻；这里只兜底）
        Self::spawn_algo_if_needed();
        match PipeClient::connect() {
            Ok(c) => {
                self.pipe = Some(c);
                self.note_success();
                self.pipe.as_ref()
            }
            Err(e) => {
                crate::debug_log(&format!("IPC 连接算法服务失败：{e}"));
                self.note_failure();
                None
            }
        }
    }

    /// 发送请求并取回应答；连接失效时自动重连一次。
    ///
    /// **防 UI 阻塞**：读响应带超时（IPC_READ_TIMEOUT）。服务端若卡死/慢，
    /// 超时返回 None → 调用方走降级（按键直通），绝不让宿主 UI 线程无限
    /// 阻塞（曾造成"应用无响应 + 其他输入法失效"，见 2026-08-12）。
    ///
    /// **超时必须丢弃连接**：协议是「一请求一应答」且帧里没有请求 ID，
    /// 无法把应答与请求配对。若超时后继续用同一条管道，服务端迟到写入的
    /// 那一帧会被下一次请求读走——此后每一次按键拿到的都是**上一键**的
    /// 候选与上屏文本，且 Response 变体不匹配的请求（toggle_ascii/get_option）
    /// 全部静默返回 None，表现为「输入法突然整个乱掉」且直到宿主进程退出
    /// 都不会自愈。丢弃连接让下一次请求重连，是这里唯一安全的处置。
    fn roundtrip(&mut self, req: &Request) -> Option<Response> {
        self.ensure()?;
        let data = encode_request(req).ok()?;
        if !self.try_send(&data) {
            // 连接断开：丢弃并重连一次
            self.pipe = None;
            if self.ensure().is_none() || !self.try_send(&data) {
                return None;
            }
        }
        let read = self
            .pipe
            .as_ref()?
            .read_frame_timeout(std::time::Duration::from_millis(IPC_READ_TIMEOUT_MS));
        let frame = match read {
            Ok(f) => f,
            Err(_) => {
                // 超时或读失败：连接上可能残留一帧未读应答，必须整条丢弃。
                self.pipe = None;
                self.note_failure();
                return None;
            }
        };
        decode_response(&frame).ok()
    }

    /// 尝试发送一帧；失败返回 false（不持有借用）。
    fn try_send(&self, data: &[u8]) -> bool {
        match self.pipe.as_ref() {
            Some(p) => p.write_frame(data).is_ok(),
            None => false,
        }
    }

    /// 会话是否已建立（服务可连）。
    pub fn available(&mut self) -> bool {
        matches!(
            self.roundtrip(&Request::CreateSession),
            Some(Response::Session(Some(_)))
        )
    }

    /// 喂键；返回 (是否被吃掉, 上屏文本, 上下文)。
    pub fn process_key(
        &mut self,
        keysym: i32,
        mask: i32,
    ) -> Option<(bool, Option<String>, ime_ipc::Context)> {
        match self.roundtrip(&Request::ProcessKey { keysym, mask })? {
            Response::ProcessKey {
                eaten,
                commit,
                context,
            } => Some((eaten, commit, context)),
            _ => None,
        }
    }

    /// 读上下文快照。
    pub fn context(&mut self) -> Option<ime_ipc::Context> {
        match self.roundtrip(&Request::Context)? {
            Response::Context(c) => Some(c),
            _ => None,
        }
    }

    /// 模拟键序列（如 "{Escape}" 清组合）。
    pub fn simulate(&mut self, keys: &str) -> bool {
        matches!(
            self.roundtrip(&Request::Simulate(keys.to_string())),
            Some(Response::Simulate(true))
        )
    }

    /// 切换中英文，返回切换后是否英文直输。
    pub fn toggle_ascii(&mut self) -> Option<bool> {
        match self.roundtrip(&Request::ToggleAscii)? {
            Response::Ascii(b) => Some(b),
            _ => None,
        }
    }

    /// 读取布尔开关（如 "full_shape"、"ascii_punct"）；连接失败返回 None。
    pub fn get_option(&mut self, name: &str) -> Option<bool> {
        match self.roundtrip(&Request::GetOption(name.to_string()))? {
            Response::Option(b) => Some(b),
            _ => None,
        }
    }

    /// 设置布尔开关；成功返回 true。
    pub fn set_option(&mut self, name: &str, value: bool) -> bool {
        matches!(
            self.roundtrip(&Request::SetOption {
                name: name.to_string(),
                value,
            }),
            Some(Response::Ok)
        )
    }
}
