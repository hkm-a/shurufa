//! 线路协议：长度前缀（u32 小端）+ JSON 消息体。
//!
//! 剪贴板同步的消息量小、频率低，JSON 的可调试性优先于紧凑性。

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// 单条消息上限：文本仅数十 KB，图片同步的 base64 PNG 可达数 MB，留足余量
const MAX_FRAME: u32 = 16 * 1024 * 1024;

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
    /// 文件传输 v3：发送方宣告文件元数据（file-v1 特性协商后启用）。
    /// 所有字段带默认值：v2 端序列化时可能缺字段，解析不应失败。
    FileOffer {
        /// 传输 id（发送端 uuidv4 风格 32 hex），贯穿整个传输生命周期。
        #[serde(default)]
        msg_id: String,
        /// 文件名（不含路径分隔符，由发送端写死 file_name）。
        #[serde(default)]
        name: String,
        /// 文件总字节数。
        #[serde(default)]
        size: u64,
        /// MIME 类型（未知可为 application/octet-stream）。
        #[serde(default)]
        mime: String,
        /// 整体内容 sha256 十六进制小写（64 字符）。
        #[serde(default)]
        sha256: String,
        /// 每块最大字节（约定 64 KiB；最后一块可更小）。
        #[serde(default)]
        chunk_bytes: u32,
    },
    /// 接收方同意接收。
    FileAccept {
        #[serde(default)]
        msg_id: String,
    },
    /// 接收方拒绝（too_large / user_declined / timeout 等）。
    FileDecline {
        #[serde(default)]
        msg_id: String,
        #[serde(default)]
        reason: String,
    },
    /// 流式数据块：data 为 base64；last 表示最后一块。
    FileChunk {
        #[serde(default)]
        msg_id: String,
        /// 本块在文件中的偏移（自 0 起，块按序到达）。
        #[serde(default)]
        offset: u64,
        #[serde(default)]
        data: String,
        #[serde(default)]
        last: bool,
    },
    /// 发送方发完全部块后的收尾（含校验值）。
    FileDone {
        #[serde(default)]
        msg_id: String,
        #[serde(default)]
        sha256: String,
    },
    /// 接收方落盘校验后的最终应答。
    FileAck {
        #[serde(default)]
        msg_id: String,
        #[serde(default)]
        received_bytes: u64,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    /// 接收方周期性回报已收字节数（供发送端渲染进度）。
    FileProgress {
        #[serde(default)]
        msg_id: String,
        #[serde(default)]
        received_bytes: u64,
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
/// "file-v1"：文件同步 v3（FileOffer/Accept/Chunk/Done/Ack/Progress）。
pub const FEATURE_FILE_V1: &str = "file-v1";

/// 当前协议版本：v1 = Hello+Ping+Clip*；v2 = Hello 带 features 协商，
/// ClipText 带 msg_id/origin_device_fp，可用于跨端回声抑制；
/// v3 = 分块文件传输（Offer/Accept/Chunk/Done/Ack/Progress）。
pub const PROTOCOL_VERSION: u32 = 3;

/// 对端特性闭包：判断 hello 协商出的特性是否包含某项。
pub fn peer_supports(features: &[String], name: &str) -> bool {
    features.iter().any(|f| f == name)
}

/// 本端默认宣称的特性（协商 hello 时发送）。
pub fn local_features() -> Vec<String> {
    vec![
        FEATURE_MSG_ID_V1.to_string(),
        FEATURE_LWW_V1.to_string(),
        FEATURE_SEARCH_V1.to_string(),
        FEATURE_FILE_V1.to_string(),
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

/// 仅接受长度前缀帧。曾有一条兼容裸 JSON 流的回退路径（对 TLS 流逐字节
/// read_exact 扫描大括号配平，最坏循环 1600 万次）——长度前缀帧自 v0.5.x
/// 起已是唯一线格式，两端同仓同发，回退路径已删（架构审视报告 §7.1）。
pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> Result<Message, String> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("读长度失败: {e}"))?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME {
        return Err(format!("对端帧过大: {len}"));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)
        .await
        .map_err(|e| format!("读消息失败: {e}"))?;
    serde_json::from_slice(&body).map_err(|e| format!("消息格式错误: {e}"))
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
    async fn 裸json流被拒绝() {
        // 旧版裸 JSON 帧的首字节是 '{'，按小端 u32 解码必然超出帧上限——
        // 回退路径已删，必须直接报错而不是逐字节扫描兼容。
        let (mut writer, mut reader) = tokio::io::duplex(4096);
        let mut raw = br#"{"type":"hello","name":"legacy""#.to_vec();
        raw.push(b'}');
        writer.write_all(&raw).await.unwrap();
        let err = read_msg(&mut reader).await.unwrap_err();
        assert!(err.contains("对端帧过大"), "实际错误：{err}");
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

    #[tokio::test]
    async fn 文件变体序列化往返且v3缺省字段可解析() {
        let offer = Message::FileOffer {
            msg_id: "m1".into(),
            name: "报告.pdf".into(),
            size: 1024,
            mime: "application/pdf".into(),
            sha256: "ab".repeat(32),
            chunk_bytes: 64 * 1024,
        };
        let bytes = serde_json::to_vec(&offer).unwrap();
        let parsed: Message = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, offer);

        // v3 接收端应能容忍缺省字段（v2 端不会发这些变体，但若字段缺失仍可解析）
        let sparse: Message = serde_json::from_value(serde_json::json!({
            "type": "file_offer",
            "msg_id": "m2",
            "name": "x.bin",
        }))
        .unwrap();
        match sparse {
            Message::FileOffer {
                msg_id,
                name,
                size,
                mime,
                sha256,
                chunk_bytes,
            } => {
                assert_eq!(msg_id, "m2");
                assert_eq!(name, "x.bin");
                assert_eq!(size, 0);
                assert_eq!(mime, "");
                assert_eq!(sha256, "");
                assert_eq!(chunk_bytes, 0);
            }
            other => panic!("应解析为 FileOffer，实际 {other:?}"),
        }

        let ack = Message::FileAck {
            msg_id: "m1".into(),
            received_bytes: 1024,
            ok: true,
            error: None,
        };
        let bytes = serde_json::to_vec(&ack).unwrap();
        let parsed: Message = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, ack);

        // error 缺省（v3 对端省略）应为 None
        let sparse_ack: Message = serde_json::from_value(serde_json::json!({
            "type": "file_ack",
            "msg_id": "m",
            "received_bytes": 10,
            "ok": false,
        }))
        .unwrap();
        match sparse_ack {
            Message::FileAck { ok, error, .. } => {
                assert!(!ok);
                assert_eq!(error, None);
            }
            other => panic!("应解析为 FileAck，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn v2_老端不含_file_v1_特性() {
        // v2 local_features 只到 search-v1；v3 追加 file-v1 后，老端解析不会受影响。
        let features = local_features();
        assert!(features.iter().any(|f| f == FEATURE_FILE_V1));
        assert_eq!(PROTOCOL_VERSION, 3);
    }
}
