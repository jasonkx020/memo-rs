//! 授权文件（`license.json`）。
//! 社区版 / 无效授权：仅本机存储，禁止局域网同步。
//! 商业版：验签通过后允许发现与同步。

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// 发行方 Ed25519 验签公钥（32 字节 hex）。私钥仅用于 `tools/license-sign`，勿入库。
pub const LICENSE_PUBKEY_HEX: &str =
    "fc2ba849cff4e28c3fa4b930cf009b2840964e3ea3165594aa404415318dc81f";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseInfo {
    pub licensee: String,
    #[serde(default)]
    pub edition: String,
    #[serde(default)]
    pub expires: Option<String>,
    pub sig: String,
}

#[derive(Debug, Clone)]
pub enum LicenseStatus {
    Community,
    Licensed { licensee: String, edition: String },
    Invalid(String),
}

impl LicenseStatus {
    /// 社区版与无效授权均不允许局域网同步。
    pub fn allows_lan_sync(&self) -> bool {
        matches!(self, LicenseStatus::Licensed { .. })
    }
}

pub fn payload_bytes(lic: &LicenseInfo) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(lic.licensee.as_bytes());
    h.update(b"|");
    h.update(lic.edition.as_bytes());
    h.update(b"|");
    h.update(lic.expires.as_deref().unwrap_or("").as_bytes());
    h.finalize().to_vec()
}

/// 用私钥对授权字段签名，返回 hex 签名（供签发工具使用）。
pub fn sign_license(
    signing_key_hex: &str,
    licensee: &str,
    edition: &str,
    expires: Option<&str>,
) -> anyhow::Result<LicenseInfo> {
    let bytes = hex::decode(signing_key_hex.trim())?;
    if bytes.len() != 32 {
        anyhow::bail!("私钥须为 32 字节 hex");
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    let sk = SigningKey::from_bytes(&arr);
    let mut lic = LicenseInfo {
        licensee: licensee.to_string(),
        edition: edition.to_string(),
        expires: expires.map(|s| s.to_string()),
        sig: String::new(),
    };
    let msg = payload_bytes(&lic);
    let sig = sk.sign(&msg);
    lic.sig = hex::encode(sig.to_bytes());
    Ok(lic)
}

pub fn load_status(config_dir: &Path) -> LicenseStatus {
    let path = config_dir.join("license.json");
    if !path.exists() {
        return LicenseStatus::Community;
    }
    match fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<LicenseInfo>(&s).ok())
    {
        None => LicenseStatus::Invalid("授权文件无法解析".into()),
        Some(lic) => verify_with_pubkey(&lic, LICENSE_PUBKEY_HEX),
    }
}

pub fn verify_with_pubkey(lic: &LicenseInfo, pubkey_hex: &str) -> LicenseStatus {
    let Ok(pk_bytes) = hex::decode(pubkey_hex) else {
        return LicenseStatus::Invalid("内置公钥无效".into());
    };
    if pk_bytes.len() != 32 {
        return LicenseStatus::Invalid("内置公钥长度错误".into());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&pk_bytes);
    let Ok(vk) = VerifyingKey::from_bytes(&arr) else {
        return LicenseStatus::Invalid("公钥格式错误".into());
    };
    let Ok(sig_raw) = hex::decode(lic.sig.trim()) else {
        return LicenseStatus::Invalid("签名无法解码".into());
    };
    let Ok(sig) = Signature::from_slice(&sig_raw) else {
        return LicenseStatus::Invalid("签名格式错误".into());
    };
    let msg = payload_bytes(lic);
    if vk.verify(&msg, &sig).is_err() {
        return LicenseStatus::Invalid("授权签名无效".into());
    }
    if let Some(exp) = &lic.expires {
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(exp) {
            if t < chrono::Utc::now() {
                return LicenseStatus::Invalid("授权已过期".into());
            }
        }
    }
    LicenseStatus::Licensed {
        licensee: lic.licensee.clone(),
        edition: if lic.edition.is_empty() {
            "商业版".into()
        } else {
            lic.edition.clone()
        },
    }
}

pub fn status_label(s: &LicenseStatus) -> String {
    match s {
        LicenseStatus::Community => "社区版（仅本机，未激活）".into(),
        LicenseStatus::Licensed { licensee, edition } => {
            format!("{edition} · {licensee}")
        }
        LicenseStatus::Invalid(e) => format!("授权无效: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn sign_and_verify_ok() {
        let sk = SigningKey::generate(&mut OsRng);
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        let sk_hex = hex::encode(sk.to_bytes());
        let lic = sign_license(&sk_hex, "Acme", "商业版", None).unwrap();
        match verify_with_pubkey(&lic, &pk_hex) {
            LicenseStatus::Licensed { licensee, edition } => {
                assert_eq!(licensee, "Acme");
                assert_eq!(edition, "商业版");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn bad_sig_rejected() {
        let sk = SigningKey::generate(&mut OsRng);
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        let sk_hex = hex::encode(sk.to_bytes());
        let mut lic = sign_license(&sk_hex, "Acme", "商业版", None).unwrap();
        lic.sig = "00".repeat(64);
        assert!(matches!(
            verify_with_pubkey(&lic, &pk_hex),
            LicenseStatus::Invalid(_)
        ));
    }

    #[test]
    fn expired_rejected() {
        let sk = SigningKey::generate(&mut OsRng);
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        let sk_hex = hex::encode(sk.to_bytes());
        let lic = sign_license(
            &sk_hex,
            "Acme",
            "商业版",
            Some("2020-01-01T00:00:00Z"),
        )
        .unwrap();
        assert!(matches!(
            verify_with_pubkey(&lic, &pk_hex),
            LicenseStatus::Invalid(_)
        ));
    }

    #[test]
    fn missing_file_is_community() {
        let dir = temp_dir();
        assert!(matches!(
            load_status(dir.path()),
            LicenseStatus::Community
        ));
        assert!(!LicenseStatus::Community.allows_lan_sync());
    }

    #[test]
    fn builtin_pubkey_roundtrip() {
        let sk_hex = std::env::var("MEMO_LICENSE_PRIVATE_HEX").unwrap_or_else(|_| {
            // 与发行公钥配对的本地开发私钥（gitignored）；CI 无此文件则跳过
            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("license-private.hex");
            std::fs::read_to_string(p)
                .unwrap_or_default()
                .trim()
                .to_string()
        });
        if sk_hex.len() != 64 {
            return;
        }
        let lic = sign_license(&sk_hex, "TestCo", "商业版", None).unwrap();
        assert!(matches!(
            verify_with_pubkey(&lic, LICENSE_PUBKEY_HEX),
            LicenseStatus::Licensed { .. }
        ));
        assert!(verify_with_pubkey(&lic, LICENSE_PUBKEY_HEX).allows_lan_sync());
    }
}
