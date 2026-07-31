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
    /// 连接建立后双方首发：自报名称与指纹
    Hello { name: String, fingerprint: String },
    /// 配对流程中，用户确认六位码一致后发送
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
        };
        write_msg(&mut a, &msg).await.unwrap();
        let got = read_msg(&mut b).await.unwrap();
        assert_eq!(got, msg);
    }
}
