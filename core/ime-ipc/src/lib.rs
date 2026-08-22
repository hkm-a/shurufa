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
        cursor_pos: ctx.cursor_pos,
        page_no: ctx.page_no,
        page_size: ctx.page_size,
        is_last_page: ctx.is_last_page,
        // 状态位需要会话访问，此处没有 session，置默认；由 server.rs 填充
        is_ascii: false,
        is_full_shape: false,
    }
}

/// 超长组合防护判定（weasel#649 同类，2026-08-16）：组合输入串达到
/// `MAX_COMPOSITION_LEN` 码时，server.rs 会在喂下一键前清空组合，让超长串
/// 转纯字母直通，防止 librime translator 在超大音节图上爆炸（内存/CPU 暴涨
/// 导致按键卡死）。正常整句输入（"zhonghuarenmingongheguo" 21 码）远低于
/// 阈值，零影响。
pub const MAX_COMPOSITION_LEN: usize = 64;

/// 组合长度是否已达超长阈值（需在下一键前清空）。
pub fn is_overlong_composition(input_len: usize) -> bool {
    input_len >= MAX_COMPOSITION_LEN
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 超长组合防护：正常输入（21 码整句）不受影响；≥64 码才触发清空。
    #[test]
    fn overlong_composition_threshold() {
        // 正常整句输入远低于阈值
        assert!(!is_overlong_composition(0));
        assert!(!is_overlong_composition(21)); // "zhonghuarenmingongheguo"
        assert!(!is_overlong_composition(63));
        // 达到/超过阈值触发
        assert!(is_overlong_composition(64));
        assert!(is_overlong_composition(100));
        // 阈值本身可被消费方引用
        assert_eq!(MAX_COMPOSITION_LEN, 64);
    }

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
