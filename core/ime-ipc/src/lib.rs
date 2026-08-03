//! librime 独立算法服务与 TSF 客户端之间的 IPC。
//!
//! 目标（架构 M6 前置）：把 librime 引擎从每个 TSF 宿主进程内移出，放进单独
//! 的算法服务进程（shurufa-algo）。TSF 客户端经命名管道向其转发按键与读取
//! 上下文。好处：
//!  - 引擎与用户词库（leveldb LOCK）只在**一个**进程里加载，消除多宿主进程
//!    抢锁导致的“造词/调频只在抢到锁的进程生效”。
//!  - 候选窗/状态由服务侧集中管理，宿主反复进出不重建引擎。

pub mod pipe;
pub mod server;

use serde::{Deserialize, Serialize};

/// 命名管道单条消息的最大总字节数（含四字节长度前缀）。
pub const MAX_FRAME_BYTES: usize = 65_536;

/// 客户端 → 服务：一次操作请求。
#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    /// 建一个新输入会话，返回会话号。
    CreateSession,
    /// 销毁一个会话，释放资源。
    DestroySession,
    /// 把单个键（X11 keysym + 修饰掩码）喂给引擎；返回处理结果与最新上下文。
    ProcessKey { keysym: i32, mask: i32 },
    /// 读取当前会话的上屏文本（非阻塞，无则空）。
    Commit,
    /// 读取当前会话的输入上下文快照。
    Context,
    /// 模拟一段键序列（如 "nihao"、"{Escape}"）。
    Simulate(String),
    /// 读取布尔开关（ascii_mode / simplification …）。
    GetOption(String),
    /// 设置布尔开关。
    SetOption { name: String, value: bool },
    /// 切换中英文（ascii_mode）并返回切换后状态。
    ToggleAscii,
}

/// 候选条目（与 ime_bridge::Candidate 对应，仅用于序列化）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Candidate {
    pub text: String,
    pub comment: String,
}

/// 上下文快照（与 ime_bridge::Context 对应）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Context {
    pub preedit: String,
    pub candidates: Vec<Candidate>,
    pub highlighted: usize,
}

/// 服务 → 客户端：一次操作的应答。
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    /// CreateSession 结果；`None` 表示失败。
    Session(Option<u64>),
    /// DestroySession 结果永远成功。
    Ok,
    /// ProcessKey 结果。
    ProcessKey {
        eaten: bool,
        commit: Option<String>,
        context: Context,
    },
    /// Commit 结果。
    Commit(Option<String>),
    /// Context 结果。
    Context(Context),
    /// Simulate 是否被接受。
    Simulate(bool),
    /// GetOption 结果。
    Option(bool),
    /// ToggleAscii 结果（切换后是否为英文直输）。
    Ascii(bool),
    /// 底层错误（管道/反序列化失败等）。
    Error(String),
    /// 会话不存在（宿主中途退出重建导致）。
    NoSession,
}

/// 请求在管道上的线格式：`[u32 长度][JSON]`。
pub fn encode_request(req: &Request) -> Result<Vec<u8>, String> {
    let body = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    if body.len() > MAX_FRAME_BYTES - 4 {
        return Err("请求过大".into());
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// 应答在管道上的线格式：`[u32 长度][JSON]`。
pub fn encode_response(resp: &Response) -> Result<Vec<u8>, String> {
    let body = serde_json::to_vec(resp).map_err(|e| e.to_string())?;
    if body.len() > MAX_FRAME_BYTES - 4 {
        return Err("应答过大".into());
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// 从缓冲区解析一帧：返回 `(帧数据, 剩余)`。数据不足返回 `None`。
pub fn decode_frame(buf: &[u8]) -> Option<(Vec<u8>, &[u8])> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return None;
    }
    Some((buf[4..4 + len].to_vec(), &buf[4 + len..]))
}

pub fn decode_request(data: &[u8]) -> Result<Request, String> {
    serde_json::from_slice(data).map_err(|e| e.to_string())
}

pub fn decode_response(data: &[u8]) -> Result<Response, String> {
    serde_json::from_slice(data).map_err(|e| e.to_string())
}

/// 把 ime_bridge 上下文（含引擎侧生命周期内的 C 字符串）复制为可序列化的 DTO。
pub fn context_from_bridge(ctx: &ime_bridge::Context) -> Context {
    Context {
        preedit: ctx.preedit.clone(),
        candidates: ctx
            .candidates
            .iter()
            .map(|c| Candidate {
                text: c.text.clone(),
                comment: c.comment.clone(),
            })
            .collect(),
        highlighted: ctx.highlighted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let req = Request::ProcessKey {
            keysym: 0x1,
            mask: 0x2,
        };
        let bytes = encode_request(&req).unwrap();
        let (frame, rest) = decode_frame(&bytes).unwrap();
        assert!(rest.is_empty());
        assert_eq!(decode_request(&frame).unwrap().header(), "ProcessKey");
    }

    #[test]
    fn response_roundtrip() {
        let resp = Response::ProcessKey {
            eaten: true,
            commit: Some("你好".into()),
            context: Context {
                preedit: "ni".into(),
                candidates: vec![Candidate {
                    text: "你".into(),
                    comment: "".into(),
                }],
                highlighted: 0,
            },
        };
        let bytes = encode_response(&resp).unwrap();
        let (frame, rest) = decode_frame(&bytes).unwrap();
        assert!(rest.is_empty());
        let got = decode_response(&frame).unwrap();
        match got {
            Response::ProcessKey {
                eaten,
                commit,
                context,
            } => {
                assert!(eaten);
                assert_eq!(commit.as_deref(), Some("你好"));
                assert_eq!(context.preedit, "ni");
                assert_eq!(context.candidates[0].text, "你");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn partial_frame_waits() {
        let bytes = encode_request(&Request::Context).unwrap();
        // 只给前 2 字节，应判定数据不足
        assert!(decode_frame(&bytes[..2]).is_none());
        // 给满 4 字节长度但无 body
        assert!(decode_frame(&bytes[..4]).is_none());
    }

    #[test]
    fn rejects_messages_larger_than_pipe_capacity() {
        let result = encode_request(&Request::Simulate("x".repeat(MAX_FRAME_BYTES)));
        assert!(matches!(result, Err(ref message) if message == "请求过大"));
    }

    impl Request {
        fn header(&self) -> &'static str {
            match self {
                Request::CreateSession => "CreateSession",
                Request::DestroySession => "DestroySession",
                Request::ProcessKey { .. } => "ProcessKey",
                Request::Commit => "Commit",
                Request::Context => "Context",
                Request::Simulate(_) => "Simulate",
                Request::GetOption(_) => "GetOption",
                Request::SetOption { .. } => "SetOption",
                Request::ToggleAscii => "ToggleAscii",
            }
        }
    }
}
