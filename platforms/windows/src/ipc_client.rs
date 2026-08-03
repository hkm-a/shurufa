//! TSF 前端侧 IPC 客户端：连接 shurufa-algo 服务，把按键/上下文请求转发过去。
//!
//! 架构 M6 前置：引擎在独立算法服务进程里，本 DLL 只是轻量客户端。每个宿主
//! 进程持有一个会话（一个连接）。连接断开（服务崩溃/未启动）时自动重连，
//! 恢复后重建会话。

use ime_ipc::pipe::PipeClient;
use ime_ipc::{decode_response, encode_request, Request, Response};

/// 单个 TSF 宿主的 IPC 客户端。`connect` 是惰性的：首次使用时才建连。
pub struct ImeClient {
    pipe: Option<PipeClient>,
}

#[allow(dead_code)]
impl ImeClient {
    pub fn new() -> Self {
        ImeClient { pipe: None }
    }

    /// 确保已连接（若服务尚未就绪则尝试连接）。
    fn ensure(&mut self) -> Option<&PipeClient> {
        if self.pipe.is_none() {
            match PipeClient::connect() {
                Ok(c) => self.pipe = Some(c),
                Err(e) => {
                    crate::debug_log(&format!("IPC 连接算法服务失败：{e}"));
                    return None;
                }
            }
        }
        self.pipe.as_ref()
    }

    /// 发送请求并取回应答；连接失效时自动重连一次。
    fn roundtrip(&mut self, req: &Request) -> Option<Response> {
        if self.ensure().is_none() {
            return None;
        }
        let data = encode_request(req).ok()?;
        if !self.try_send(&data) {
            // 连接断开：丢弃并重连一次
            self.pipe = None;
            if self.ensure().is_none() || !self.try_send(&data) {
                return None;
            }
        }
        let frame = self.pipe.as_ref()?.read_frame().ok()?;
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
}
