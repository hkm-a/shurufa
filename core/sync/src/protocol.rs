//! 线路协议：长度前缀（u32 小端）+ JSON 消息体。
//!
//! 剪贴板同步的消息量小、频率低，JSON 的可调试性优先于紧凑性。

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// 单条消息上限：文本仅数十 KB，图片同步的 base64 PNG 可达数 MB，留足余量
const MAX_FRAME: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFormat {
    LengthPrefixed,
    RawJson,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    /// 连接建立后双方首发：自报名称、指纹和可直连的监听端口。
    ///
    /// `features`/`protocol_version` 均为可选（serde default）：老版本端
    /// 序列化时不写这两个字段，新版本端读到缺省当空列表/版本 1 处理。
    /// 新版本端据此决定是否在 Clip* 中携带 msg_id，避免老版本端反序列化
    /// 失败。
    Hello {
        name: String,
        fingerprint: String,
        listen_port: u16,
        /// 该端声明支持的特性（如 "msg-id-v1"、"lww-v1"）。
        #[serde(default)]
        features: Vec<String>,
        /// 线协议版本：1 = 初始版，2 = 携带 msg_id + LWW。
        #[serde(default = "default_protocol_version")]
        protocol_version: u32,
    },
    /// 配对流程中，用户确认八位码一致后发送
    PairConfirm,
    /// 配对流程中，用户拒绝或超时
    PairReject,
    /// 文本剪贴板条目
    ClipText {
        text: String,
        sent_at_ms: i64,
        /// 消息指纹（发送端的 UUIDv4），跨端重传去重用。
        /// 老版本不发送：接收端 None 时退化为 (text, sent_at_ms) 推断。
        #[serde(default)]
        msg_id: Option<String>,
        /// 原始写入者设备指纹；远端回环时以此识别"是我自己的回声"。
        #[serde(default)]
        origin_device_fp: Option<String>,
    },
    /// 图片剪贴板条目（data 为 base64 编码的 PNG，跨平台通用）
    ClipImage {
        data: String,
        sent_at_ms: i64,
        #[serde(default)]
        msg_id: Option<String>,
        #[serde(default)]
        origin_device_fp: Option<String>,
    },
    /// 文件剪贴板条目（data 为 base64 编码的文件字节）。
    ClipFile {
        name: String,
        mime_type: String,
        data: String,
        sent_at_ms: i64,
        #[serde(default)]
        msg_id: Option<String>,
        #[serde(default)]
        origin_device_fp: Option<String>,
    },
    /// 保活
    Ping,
    /// 跨设备剪贴板历史搜索请求（特性 "search-v1" 协商后启用）。
    SearchRequest {
        query: String,
        /// 关联响应的请求 id（发送端生成的短随机串）。
        #[serde(default)]
        req_id: Option<String>,
    },
    /// 搜索响应：命中条目摘要（不含图片/文件字节，只给文本预览）。
    SearchResponse {
        #[serde(default)]
        req_id: Option<String>,
        /// 命中预览列表（至多 8 条）。
        #[serde(default)]
        hits: Vec<SearchHit>,
    },
}

/// 搜索命中摘要：仅文本预览，避免把图片/文件字节挤进查询响应。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct SearchHit {
    pub text: String,
    pub source_app: String,
    pub updated_at: i64,
}

fn default_protocol_version() -> u32 {
    1
}

/// 本端声明的特性列表；接收端将其存入 PeerCapabilities。
pub const FEATURE_MSG_ID_V1: &str = "msg-id-v1";
pub const FEATURE_LWW_V1: &str = "lww-v1";
/// "search-v1"：跨设备剪贴板历史搜索（SearchRequest/SearchResponse）。
pub const FEATURE_SEARCH_V1: &str = "search-v1";

/// 当前协议版本：v1 = Hello+Ping+Clip*；v2 = Hello 带 features 协商，
/// ClipText 带 msg_id/origin_device_fp，可用于跨端回声抑制。
pub const PROTOCOL_VERSION: u32 = 2;

/// 对端特性闭包：判断 hello 协商出的特性是否包含某项。
pub fn peer_supports<'a>(features: &'a [String], name: &str) -> bool {
    features.iter().any(|f| f == name)
}

/// 本端默认宣称的特性（协商 hello 时发送）。
pub fn local_features() -> Vec<String> {
    vec![
        FEATURE_MSG_ID_V1.to_string(),
        FEATURE_LWW_V1.to_string(),
        FEATURE_SEARCH_V1.to_string(),
    ]
}

pub async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, msg: &Message) -> Result<(), String> {
    let body = serde_json::to_vec(msg).expect("消息序列化不应失败");
    if body.len() as u32 > MAX_FRAME {
        return Err("消息超过帧上限".into());
    }
    w.write_all(&(body.len() as u32).to_le_bytes())
        .await
        .map_err(|e| format!("写长度失败: {e}"))?;
    w.write_all(&body)
        .await
        .map_err(|e| format!("写消息失败: {e}"))?;
    w.flush().await.map_err(|e| format!("刷新失败: {e}"))?;
    Ok(())
}

pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> Result<Message, String> {
    read_msg_with_format(r).await.map(|(message, _)| message)
}

pub async fn write_msg_with_format<W: AsyncWrite + Unpin>(
    w: &mut W,
    msg: &Message,
    format: FrameFormat,
) -> Result<(), String> {
    let body = serde_json::to_vec(msg).expect("消息序列化不应失败");
    if body.len() as u32 > MAX_FRAME {
        return Err("消息超过帧上限".into());
    }
    if format == FrameFormat::LengthPrefixed {
        w.write_all(&(body.len() as u32).to_le_bytes())
            .await
            .map_err(|e| format!("写长度失败: {e}"))?;
    }
    w.write_all(&body)
        .await
        .map_err(|e| format!("写消息失败: {e}"))?;
    w.flush().await.map_err(|e| format!("刷新失败: {e}"))
}

pub async fn read_msg_with_format<R: AsyncRead + Unpin>(
    r: &mut R,
) -> Result<(Message, FrameFormat), String> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("读长度失败: {e}"))?;
    let len = u32::from_le_bytes(len_buf);
    if len <= MAX_FRAME {
        let mut body = vec![0u8; len as usize];
        r.read_exact(&mut body)
            .await
            .map_err(|e| format!("读消息失败: {e}"))?;
        return serde_json::from_slice(&body)
            .map(|message| (message, FrameFormat::LengthPrefixed))
            .map_err(|e| format!("消息格式错误: {e}"));
    }
    if len_buf[0] != b'{' {
        return Err(format!("对端帧过大: {len}"));
    }
    let mut body = len_buf.to_vec();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut escaped = false;
    let mut scanned = 0usize;
    loop {
        for &byte in &body[scanned..] {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
            } else if byte == b'"' {
                quoted = true;
            } else if byte == b'{' {
                depth += 1;
            } else if byte == b'}' {
                depth -= 1;
            }
            if depth == 0 && !quoted {
                return serde_json::from_slice(&body)
                    .map(|message| (message, FrameFormat::RawJson))
                    .map_err(|e| format!("旧协议消息格式错误: {e}"));
            }
        }
        scanned = body.len();
        if body.len() >= MAX_FRAME as usize {
            return Err("旧协议消息超过帧上限".into());
        }
        let mut next = [0u8; 1];
        r.read_exact(&mut next)
            .await
            .map_err(|e| format!("读旧协议消息失败: {e}"))?;
        body.push(next[0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn 消息读写往返() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let msg = Message::ClipText {
            text: "你好，同步".into(),
            sent_at_ms: 1234567890,
            msg_id: Some("test-msg-id".into()),
            origin_device_fp: Some("fp-a".repeat(8)),
        };
        write_msg(&mut a, &msg).await.unwrap();
        let got = read_msg(&mut b).await.unwrap();
        assert_eq!(got, msg);
    }

    #[tokio::test]
    async fn 原始_json帧可被识别并保持格式() {
        let (mut writer, mut reader) = tokio::io::duplex(4096);
        let msg = Message::Hello {
            name: "安卓".into(),
            fingerprint: "a".repeat(64),
            listen_port: 48632,
            features: vec![],
            protocol_version: 1,
        };
        let expected = serde_json::to_vec(&msg).unwrap();
        let write = tokio::spawn(async move {
            writer.write_all(&expected).await.unwrap();
        });
        let (actual, format) = read_msg_with_format(&mut reader).await.unwrap();
        write.await.unwrap();
        assert_eq!(actual, msg);
        assert_eq!(format, FrameFormat::RawJson);
    }

    #[tokio::test]
    async fn 旧版本_hello_缺新字段仍可解析() {
        // 模拟老版本端发来的 Hello（没有 features/protocol_version）
        let legacy = serde_json::json!({
            "type": "hello",
            "name": "老版本",
            "fingerprint": "b".repeat(64),
            "listen_port": 48632,
        });
        let parsed: Message = serde_json::from_value(legacy).unwrap();
        match parsed {
            Message::Hello {
                features,
                protocol_version,
                ..
            } => {
                assert!(features.is_empty());
                assert_eq!(protocol_version, 1);
            }
            other => panic!("应解析为 Hello，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn 带_msg_id_的_clip_text_序列化后仍能反解() {
        let msg = Message::ClipText {
            text: "x".into(),
            sent_at_ms: 1,
            msg_id: Some("m".into()),
            origin_device_fp: None,
        };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let parsed: Message = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, msg);
    }

    #[tokio::test]
    async fn 旧版消息不含搜索变体仍可解析() {
        // 老版本端只发 ClipText/Ping，新端解析不应受新增变体影响
        let legacy = serde_json::json!({"type": "ping"});
        let parsed: Message = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed, Message::Ping);
        let legacy_clip = serde_json::json!({
            "type": "clip_text",
            "text": "旧端文本",
            "sent_at_ms": 42,
        });
        let parsed: Message = serde_json::from_value(legacy_clip).unwrap();
        match parsed {
            Message::ClipText { text, msg_id, .. } => {
                assert_eq!(text, "旧端文本");
                assert_eq!(msg_id, None);
            }
            other => panic!("应解析为 ClipText，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn 搜索请求与响应序列化往返() {
        let request = Message::SearchRequest {
            query: "邮件".into(),
            req_id: Some("req-1".into()),
        };
        let bytes = serde_json::to_vec(&request).unwrap();
        let parsed: Message = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, request);

        // req_id 缺省（旧端或本端未生成 id）也能解析
        let no_id: Message =
            serde_json::from_value(serde_json::json!({"type": "search_request", "query": "q"}))
                .unwrap();
        assert_eq!(
            no_id,
            Message::SearchRequest {
                query: "q".into(),
                req_id: None,
            }
        );

        let response = Message::SearchResponse {
            req_id: Some("req-1".into()),
            hits: vec![
                SearchHit {
                    text: "主题：周报邮件".into(),
                    source_app: "Mail".into(),
                    updated_at: 1234567890,
                },
                SearchHit::default(),
            ],
        };
        let bytes = serde_json::to_vec(&response).unwrap();
        let parsed: Message = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, response);

        // hit 字段缺省时走默认，保证与未来扩展字段兼容
        let sparse: SearchHit =
            serde_json::from_value(serde_json::json!({"text": "仅文本"})).unwrap();
        assert_eq!(sparse.text, "仅文本");
        assert_eq!(sparse.source_app, "");
        assert_eq!(sparse.updated_at, 0);
    }
}
