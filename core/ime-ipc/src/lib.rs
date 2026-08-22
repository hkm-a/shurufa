//! 跨平台 IPC 协议与 DTO：librime 算法服务与各前端（TSF/Android）之间的
//! 请求/应答、上下文快照与帧编解码。
//!
//! 阶段 4 拆分后，本 crate 不包含任何平台传输实现；Windows 命名管道
//! `pipe` / 算法服务接入 `server` 位于 `platforms/windows-ipc`。

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
    /// 组合光标在 preedit 中的位置（UTF-16 码元数）。
    pub cursor_pos: usize,
    /// 当前候选页页码（从 0 开始）。
    pub page_no: usize,
    /// 每页候选条数上限。
    pub page_size: usize,
    /// 是否为候选最后一页。
    pub is_last_page: bool,
    /// 引擎当前是否英文直输（由服务侧填充；默认 false 兼容旧 JSON）。
    #[serde(default)]
    pub is_ascii: bool,
    /// 引擎当前是否全角（由服务侧填充）。
    #[serde(default)]
    pub is_full_shape: bool,
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
    use bytes::Bytes;
    use tokio_util::codec::{Encoder, LengthDelimitedCodec};

    let body = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    if body.len() > MAX_FRAME_BYTES - 4 {
        return Err("请求过大".into());
    }
    let mut out = bytes::BytesMut::with_capacity(4 + body.len());
    LengthDelimitedCodec::builder()
        .max_frame_length(MAX_FRAME_BYTES)
        .new_codec()
        .encode(Bytes::copy_from_slice(&body), &mut out)
        .map_err(|e| e.to_string())?;
    Ok(out.to_vec())
}

/// 应答在管道上的线格式：`[u32 长度][JSON]`。
pub fn encode_response(resp: &Response) -> Result<Vec<u8>, String> {
    use bytes::Bytes;
    use tokio_util::codec::{Encoder, LengthDelimitedCodec};

    let body = serde_json::to_vec(resp).map_err(|e| e.to_string())?;
    if body.len() > MAX_FRAME_BYTES - 4 {
        return Err("应答过大".into());
    }
    let mut out = bytes::BytesMut::with_capacity(4 + body.len());
    LengthDelimitedCodec::builder()
        .max_frame_length(MAX_FRAME_BYTES)
        .new_codec()
        .encode(Bytes::copy_from_slice(&body), &mut out)
        .map_err(|e| e.to_string())?;
    Ok(out.to_vec())
}

/// 从缓冲区解析一帧：返回 `(帧数据, 剩余)`。数据不足返回 `None`。
pub fn decode_frame(buf: &[u8]) -> Option<(Vec<u8>, &[u8])> {
    use tokio_util::codec::{Decoder, LengthDelimitedCodec};

    let mut src = bytes::BytesMut::from(buf);
    let mut codec = LengthDelimitedCodec::builder()
        .max_frame_length(MAX_FRAME_BYTES)
        .new_codec();
    match codec.decode(&mut src) {
        Ok(Some(frame)) => {
            let used = buf.len() - src.len();
            Some((frame.to_vec(), &buf[used..]))
        }
        _ => None,
    }
}

pub fn decode_request(data: &[u8]) -> Result<Request, String> {
    serde_json::from_slice(data).map_err(|e| e.to_string())
}

pub fn decode_response(data: &[u8]) -> Result<Response, String> {
    serde_json::from_slice(data).map_err(|e| e.to_string())
}

/// 超长组合防护判定（weasel#649 同类，2026-08-16）：实现已下沉到
/// `core/ime-policy`，此处保留 re-export 避免破坏现有调用方。
pub use ime_policy::{is_overlong_composition, MAX_COMPOSITION_LEN};

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
                cursor_pos: 2,
                page_no: 0,
                page_size: 9,
                is_last_page: false,
                ..Context::default()
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
