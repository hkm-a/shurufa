//! 跨设备剪贴板同步核心。
//!
//! 安全模型：每台设备持自签名证书，SHA-256 指纹即设备身份；
//! 配对时两端各自推导八位确认码由用户比对（KDE Connect 模式），
//! 确认后互相钉扎指纹；此后所有连接走 TLS 1.3 双向证书认证，
//! 握手后校验对端指纹必须在已配对列表中。
//!
//! 传输为 TCP + rustls（架构文档原定 QUIC，MVP 取更小集成面；
//! 单条有序流足以承载剪贴板推送，传输层可后换）。

mod identity;
mod peers;
mod protocol;
mod relay;
mod service;
mod tls;
pub mod wan;

pub use identity::DeviceIdentity;
pub use peers::{Peer, PeerStore};
pub use protocol::{Message, SearchHit};
pub use relay::{accept_via_relay, connect_via_relay, run_relay};
pub use service::{
    load_relay_addr, save_relay_addr, ConfirmFn, FileConfirmFn, FileOfferPrompt, FileSendState,
    Incoming, PairPrompt, SearchHandler, SendErr, SyncConfig, SyncService, FILE_AUTO_ACCEPT_MAX,
    MAX_CLIP_FILE_BYTES, MAX_CLIP_IMAGE_BYTES, MAX_FILE_BYTES,
};
pub use wan::{Wan, WanProfile};

use sha2::{Digest, Sha256};

/// 证书 DER 的 SHA-256 十六进制指纹（64 字符小写）。
pub fn fingerprint_hex(cert_der: &[u8]) -> String {
    let hash = Sha256::digest(cert_der);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// 配对确认码：对两端指纹排序拼接后取哈希，两端计算结果必然一致；
/// 八位数字供人眼比对，防中间人（6 位仅百万级空间，攻击者在配对窗口内
/// 可暴力枚举；8 位将空间扩至一亿，结合人工比对的单次性质足够抵御）。
pub fn pairing_code(fp_a: &str, fp_b: &str) -> String {
    let (lo, hi) = if fp_a <= fp_b {
        (fp_a, fp_b)
    } else {
        (fp_b, fp_a)
    };
    let hash = Sha256::digest(format!("shurufa-pair:{lo}:{hi}").as_bytes());
    let n = u64::from_be_bytes([
        hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7],
    ]) % 100_000_000;
    format!("{n:08}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 配对码两端一致且八位() {
        let a = "aa".repeat(32);
        let b = "bb".repeat(32);
        assert_eq!(pairing_code(&a, &b), pairing_code(&b, &a));
        assert_eq!(pairing_code(&a, &b).len(), 8);
        assert_ne!(pairing_code(&a, &b), pairing_code(&a, &a));
    }
}
