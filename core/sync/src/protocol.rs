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
    Hello {
        name: String,
        fingerprint: String,
        listen_port: u16,
    },
    /// 配对流程中，用户确认八位码一致后发送
    PairConfirm,
    /// 配对流程中，用户拒绝或超时
    PairReject,
    /// 文本剪贴板条目
    ClipText { text: String, sent_at_ms: i64 },
    /// 图片剪贴板条目（data 为 base64 编码的 PNG，跨平台通用）
    ClipImage { data: String, sent_at_ms: i64 },
    /// 文件剪贴板条目（data 为 base64 编码的文件字节）。
    ClipFile {
        name: String,
        mime_type: String,
        data: String,
        sent_at_ms: i64,
    },
    /// 保活
    Ping,
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
}
