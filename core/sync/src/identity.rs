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

/// 私钥落盘并收紧权限。
///
/// Unix 平台设为 0600（仅属主可读写）；Windows 平台为磁盘上的私钥文件
/// 套用「仅当前用户」的 DACL（阻断继承），防止第三方/低权限读取私钥
/// 冒名设备。Windows 收紧为 best-effort：任何失败仅记录日志，不影响
/// 身份可用（用户目录默认即本人可读，属纵深防御）。
fn write_private(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| format!("写入私钥失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(windows)]
    {
        restrict_private_key(path);
    }
    Ok(())
}

/// Windows：把私钥文件 ACL 收紧为仅当前用户，并打掉继承，防止被
/// 本地其他账户/低权限进程读取。best-effort，任何失败静默降级。
#[cfg(windows)]
fn restrict_private_key(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{LocalFree, GENERIC_ALL};
    use windows_sys::Win32::Security::Authorization::{
        SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, SE_FILE_OBJECT,
        SET_ACCESS, TRUSTEE_IS_NAME, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };

    // Trustee 以名称 "CURRENT_USER" 指代当前交互用户，避免手工映射 SID。
    let mut current_user: Vec<u16> = "CURRENT_USER".encode_utf16().chain(Some(0)).collect();
    let trustee = TRUSTEE_W {
        pMultipleTrustee: std::ptr::null_mut(),
        TrusteeForm: TRUSTEE_IS_NAME,
        TrusteeType: TRUSTEE_IS_USER,
        ptstrName: current_user.as_mut_ptr(),
        ..Default::default()
    };
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_ALL as u32,
        grfAccessMode: SET_ACCESS,
        grfInheritance: 0, // NO_INHERITANCE：不继承父目录 ACL
        Trustee: trustee,
        ..Default::default()
    };
    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    // ERROR_SUCCESS 才继续；失败时静默返回，ACL 维持默认。
    if unsafe { SetEntriesInAclW(1, &access, std::ptr::null(), &mut new_dacl) } != 0 {
        return;
    }
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let info: u32 = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    unsafe {
        let _ = SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            info,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            new_dacl,
            std::ptr::null::<ACL>(),
        );
        let _ = LocalFree(new_dacl as _);
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn 私钥acl仅限当前用户() {
        let dir = tempfile::tempdir().unwrap();
        DeviceIdentity::load_or_create(dir.path(), "测试机").unwrap();
        let key = dir.path().join("identity.key.der");
        let out = std::process::Command::new("icacls")
            .arg(&key)
            .output()
            .expect("运行 icacls 失败");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let user = std::env::var("USERNAME").unwrap_or_default();
        assert!(!user.is_empty(), "缺少 USERNAME");
        assert!(
            stdout.contains(&user),
            "ACL 应授予当前用户 {user}，实际: {stdout}"
        );
        assert!(
            !stdout.contains("Everyone"),
            "ACL 不应包含 Everyone，实际: {stdout}"
        );
        assert!(
            !stdout.contains(r"BUILTIN\Users"),
            "ACL 不应包含 Users 组，实际: {stdout}"
        );
    }

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
