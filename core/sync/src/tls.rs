//! TLS 配置：双向证书认证，证书本身不走 CA 链校验。
//!
//! 验证器只做签名与格式层面的接受，**身份判定在握手完成后**由调用方
//! 读取对端证书指纹并比对配对表（钉扎）。配对阶段的中间人风险由
//! 六位确认码的人工比对覆盖。

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};

use crate::DeviceIdentity;

#[derive(Debug)]
struct PinAfterHandshake {
    provider: Arc<CryptoProvider>,
}

impl PinAfterHandshake {
    fn new() -> Self {
        PinAfterHandshake {
            provider: Arc::new(ring::default_provider()),
        }
    }

    fn schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

impl ServerCertVerifier for PinAfterHandshake {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes()
    }
}

impl ClientCertVerifier for PinAfterHandshake {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes()
    }
}

/// 进程级默认加密后端只装一次；依赖图里若同时出现 ring 与
/// aws-lc-rs，rustls 无法自动抉择会直接 panic。
fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = ring::default_provider().install_default();
    });
}

fn cert_key(identity: &DeviceIdentity) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    let cert = CertificateDer::from(identity.cert_der.clone());
    let key = PrivateKeyDer::Pkcs8(identity.key_der.clone().into());
    (vec![cert], key)
}

pub fn server_config(identity: &DeviceIdentity) -> Result<Arc<ServerConfig>, String> {
    ensure_crypto_provider();
    let (certs, key) = cert_key(identity);
    let config = ServerConfig::builder()
        .with_client_cert_verifier(Arc::new(PinAfterHandshake::new()))
        .with_single_cert(certs, key)
        .map_err(|e| format!("服务端 TLS 配置失败: {e}"))?;
    Ok(Arc::new(config))
}

pub fn client_config(identity: &DeviceIdentity) -> Result<Arc<ClientConfig>, String> {
    ensure_crypto_provider();
    let (certs, key) = cert_key(identity);
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinAfterHandshake::new()))
        .with_client_auth_cert(certs, key)
        .map_err(|e| format!("客户端 TLS 配置失败: {e}"))?;
    Ok(Arc::new(config))
}
