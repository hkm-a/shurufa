//! 跨设备剪贴板同步核心。
//!
//! 安全模型：每台设备持自签名证书，SHA-256 指纹即设备身份；
//! 配对时两端各自推导六位确认码由用户比对（KDE Connect 模式），
//! 确认后互相钉扎指纹；此后所有连接走 TLS 1.3 双向证书认证，
//! 握手后校验对端指纹必须在已配对列表中。
//!
//! 传输为 TCP + rustls（架构文档原定 QUIC，MVP 取更小集成面；
//! 单条有序流足以承载剪贴板推送，传输层可后换）。

mod identity;
mod peers;
mod protocol;
mod service;
mod tls;

pub use identity::DeviceIdentity;
pub use peers::{Peer, PeerStore};
pub use protocol::Message;
pub use service::{
    ConfirmFn, Incoming, PairPrompt, SyncConfig, SyncService, MAX_CLIP_FILE_BYTES,
    MAX_CLIP_IMAGE_BYTES,
};

use sha2::{Digest, Sha256};

/// 证书 DER 的 SHA-256 十六进制指纹（64 字符小写）。
pub fn fingerprint_hex(cert_der: &[u8]) -> String {
    let hash = Sha256::digest(cert_der);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// 配对确认码：对两端指纹排序拼接后取哈希，两端计算结果必然一致；
/// 六位数字供人眼比对，防中间人。
pub fn pairing_code(fp_a: &str, fp_b: &str) -> String {
    let (lo, hi) = if fp_a <= fp_b {
        (fp_a, fp_b)
    } else {
        (fp_b, fp_a)
    };
    let hash = Sha256::digest(format!("shurufa-pair:{lo}:{hi}").as_bytes());
    let n = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) % 1_000_000;
    format!("{n:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 配对码两端一致且六位() {
        let a = "aa".repeat(32);
        let b = "bb".repeat(32);
        assert_eq!(pairing_code(&a, &b), pairing_code(&b, &a));
        assert_eq!(pairing_code(&a, &b).len(), 6);
        assert_ne!(pairing_code(&a, &b), pairing_code(&a, &a));
    }
}
