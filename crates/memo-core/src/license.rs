//! 可选授权文件（`license.json`）。无文件时按社区版运行，不阻断功能。

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// 演示用验签公钥（32 字节 hex）。正式售卖时替换为发行方公钥。
const LICENSE_PUBKEY_HEX: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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

fn payload_bytes(lic: &LicenseInfo) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(lic.licensee.as_bytes());
    h.update(b"|");
    h.update(lic.edition.as_bytes());
    h.update(b"|");
    h.update(lic.expires.as_deref().unwrap_or("").as_bytes());
    h.finalize().to_vec()
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
        Some(lic) => verify(&lic),
    }
}

fn verify(lic: &LicenseInfo) -> LicenseStatus {
    let Ok(pk_bytes) = hex::decode(LICENSE_PUBKEY_HEX) else {
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
        LicenseStatus::Community => "社区版（未激活）".into(),
        LicenseStatus::Licensed { licensee, edition } => {
            format!("{edition} · {licensee}")
        }
        LicenseStatus::Invalid(e) => format!("授权无效: {e}"),
    }
}
