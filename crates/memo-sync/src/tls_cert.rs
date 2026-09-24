//! 每节点自签 TLS 证书（不做 WebPKI；集群认证靠 HMAC）。

use anyhow::Context;
use rustls::{Certificate, PrivateKey, ServerConfig};
use std::fs;
use std::path::Path;
use std::sync::Arc;

const CERT_FILE: &str = "tls.crt";
const KEY_FILE: &str = "tls.key";

/// 从 data_dir/keys 加载或生成自签 ECDSA P-256 证书。
pub fn load_or_create(data_dir: &Path) -> anyhow::Result<(Vec<Certificate>, PrivateKey)> {
    let dir = data_dir.join("keys");
    fs::create_dir_all(&dir)?;
    let cert_path = dir.join(CERT_FILE);
    let key_path = dir.join(KEY_FILE);

    if cert_path.exists() && key_path.exists() {
        return load_pem(&cert_path, &key_path);
    }

    let mut params = rcgen::CertificateParams::new(vec!["memo.local".into()]);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "DistributedMemo");
    params.alg = &rcgen::PKCS_ECDSA_P256_SHA256;
    let cert = rcgen::Certificate::from_params(params).context("rcgen")?;
    let cert_pem = cert.serialize_pem().context("serialize cert")?;
    let key_pem = cert.serialize_private_key_pem();
    fs::write(&cert_path, cert_pem)?;
    fs::write(&key_path, key_pem)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&key_path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&key_path, perms)?;
    }
    load_pem(&cert_path, &key_path)
}

fn load_pem(
    cert_path: &Path,
    key_path: &Path,
) -> anyhow::Result<(Vec<Certificate>, PrivateKey)> {
    let cert_pem = fs::read(cert_path)?;
    let key_pem = fs::read(key_path)?;
    let mut certs = Vec::new();
    for item in rustls_pemfile::certs(&mut cert_pem.as_slice())? {
        certs.push(Certificate(item));
    }
    if certs.is_empty() {
        anyhow::bail!("tls.crt 中无证书");
    }
    let mut keys = rustls_pemfile::pkcs8_private_keys(&mut key_pem.as_slice())?;
    let key = keys
        .pop()
        .map(PrivateKey)
        .ok_or_else(|| anyhow::anyhow!("tls.key 中无 PKCS8 私钥"))?;
    Ok((certs, key))
}

pub fn server_config(data_dir: &Path) -> anyhow::Result<ServerConfig> {
    let (certs, key) = load_or_create(data_dir)?;
    ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("ServerConfig")
}

pub fn client_config() -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth()
}

/// 跳过主机名/CA 校验（身份由 HMAC 保障）。
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_or_create_stable() {
        let dir = tempfile::tempdir().unwrap();
        let (c1, k1) = load_or_create(dir.path()).unwrap();
        let (c2, k2) = load_or_create(dir.path()).unwrap();
        assert_eq!(c1.len(), c2.len());
        assert_eq!(c1[0].0, c2[0].0);
        assert_eq!(k1.0, k2.0);
        assert!(dir.path().join("keys").join(CERT_FILE).exists());
    }
}
