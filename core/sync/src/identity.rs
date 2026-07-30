//! 设备身份：自签名证书 + 私钥，持久化于同步配置目录。
//!
//! 证书仅作身份载体（指纹钉扎），不依赖 CA 链；文件为原始 DER，
//! 首次运行生成，此后直接加载（无需还原 rcgen 结构）。

use std::fs;
use std::path::{Path, PathBuf};

use rcgen::{CertificateParams, KeyPair};

pub struct DeviceIdentity {
    pub device_name: String,
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint: String,
}

impl DeviceIdentity {
    /// 加载或创建身份。`device_name` 仅用于展示，不参与身份判定。
    pub fn load_or_create(dir: &Path, device_name: &str) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| format!("创建同步配置目录失败: {e}"))?;
        let cert_path = dir.join("identity.cert.der");
        let key_path = dir.join("identity.key.der");

        let (cert_der, key_der) = if cert_path.exists() && key_path.exists() {
            (
                fs::read(&cert_path).map_err(|e| format!("读取证书失败: {e}"))?,
                fs::read(&key_path).map_err(|e| format!("读取私钥失败: {e}"))?,
            )
        } else {
            let (cert, key) = generate()?;
            write_private(&key_path, &key)?;
            fs::write(&cert_path, &cert).map_err(|e| format!("写入证书失败: {e}"))?;
            (cert, key)
        };

        let fingerprint = crate::fingerprint_hex(&cert_der);
        Ok(DeviceIdentity {
            device_name: device_name.to_string(),
            cert_der,
            key_der,
            fingerprint,
        })
    }

    /// 指纹短形式（前 12 位），用于展示与 mDNS 实例名。
    pub fn short_fp(&self) -> &str {
        &self.fingerprint[..12]
    }
}

fn generate() -> Result<(Vec<u8>, Vec<u8>), String> {
    let key = KeyPair::generate().map_err(|e| format!("生成密钥失败: {e}"))?;
    let params = CertificateParams::new(vec!["shurufa-device".to_string()])
        .map_err(|e| format!("证书参数错误: {e}"))?;
    let cert = params
        .self_signed(&key)
        .map_err(|e| format!("自签证书失败: {e}"))?;
    Ok((cert.der().to_vec(), key.serialize_der()))
}

/// 私钥落盘。Windows 用户目录默认即本人可读；类 Unix 平台收紧到 0600。
fn write_private(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| format!("写入私钥失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 身份生成与重载一致() {
        let dir = tempfile::tempdir().unwrap();
        let a = DeviceIdentity::load_or_create(dir.path(), "测试机").unwrap();
        let b = DeviceIdentity::load_or_create(dir.path(), "测试机").unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint.len(), 64);
        assert_eq!(a.short_fp().len(), 12);
    }
}
