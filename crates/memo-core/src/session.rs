//! 联网会话票：官方 Ed25519 签发，本机宽限内可离线开同步。
//! 设备客户端证书用于向官方鉴权服 mTLS 续期（不经手同步数据）。

use crate::license::LicenseInfo;
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// 会话票验签公钥（与 license 公钥独立，便于轮换）。
pub const ATTEST_PUBKEY_HEX: &str =
    "f528256b0bff5e36dc0e954f4d4662ee72dcbf4b95d5e93eb865713d29f6a5f1";

/// 正式版钉死的首次激活 URL（续期路径由客户端替换为 `/v1/renew`）。
pub const ATTEST_URL: &str = "https://license.memo-rs.example/v1/attest";

/// 官方 TLS 证书 SPKI SHA-256（hex）。为空且非 insecure 时拒绝联网激活。
/// 部署时用实际服证书 SPKI 替换；开发可用 `MEMO_ATTEST_INSECURE=1`。
pub const ATTEST_SPKI_SHA256_HEX: &str = "";

/// 票面短 TTL（服务端签发 expires_at 参考）；宽限看 grace_until。
pub const TICKET_TTL_DAYS: i64 = 7;
/// 距上次成功鉴权的离线宽限。
pub const GRACE_DAYS: i64 = 30;
/// 时钟回拨超过该阈值则废票。
pub const CLOCK_ROLLBACK_MAX: Duration = match Duration::try_days(1) {
    Some(d) => d,
    None => Duration::zero(),
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTicket {
    pub licensee: String,
    pub device_id: String,
    pub issued_at: String,
    pub expires_at: String,
    pub last_attest_at: String,
    pub grace_until: String,
    pub sig: String,
}

#[derive(Debug, Clone)]
pub enum SessionStatus {
    /// 无票或未激活
    Missing,
    /// 验签通过且在宽限内
    Active {
        licensee: String,
        grace_remaining_days: i64,
    },
    /// 需重新联网
    Expired(String),
    Invalid(String),
}

impl SessionStatus {
    pub fn allows_sync_now(&self) -> bool {
        matches!(self, SessionStatus::Active { .. })
    }
}

pub fn ticket_path(config_dir: &Path) -> PathBuf {
    config_dir.join("session.ticket")
}

pub fn device_cert_path(config_dir: &Path) -> PathBuf {
    config_dir.join("device.crt.pem")
}

pub fn device_key_path(config_dir: &Path) -> PathBuf {
    config_dir.join("device.key.pem")
}

/// Windows MachineGuid；失败则 hostname + config_dir。
pub fn device_id(config_dir: &Path) -> String {
    #[cfg(windows)]
    {
        if let Some(id) = machine_guid_windows() {
            return hash_device_id(&id);
        }
    }
    let host = hostname_fallback();
    hash_device_id(&format!("{host}|{}", config_dir.display()))
}

fn hash_device_id(raw: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"memo-device-v1");
    h.update(raw.as_bytes());
    hex::encode(h.finalize())
}

fn hostname_fallback() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown-host".into())
}

#[cfg(windows)]
fn machine_guid_windows() -> Option<String> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, KEY_READ,
    };

    const KEY: &[u16] = &[
        b'S' as u16, b'O' as u16, b'F' as u16, b'T' as u16, b'W' as u16, b'A' as u16, b'R' as u16,
        b'E' as u16, b'\\' as u16, b'M' as u16, b'i' as u16, b'c' as u16, b'r' as u16, b'o' as u16,
        b's' as u16, b'o' as u16, b'f' as u16, b't' as u16, b'\\' as u16, b'C' as u16, b'r' as u16,
        b'y' as u16, b'p' as u16, b't' as u16, b'o' as u16, b'g' as u16, b'r' as u16, b'a' as u16,
        b'p' as u16, b'h' as u16, b'y' as u16, b'\\' as u16, b'M' as u16, b'a' as u16, b'c' as u16,
        b'h' as u16, b'i' as u16, b'n' as u16, b'e' as u16, b'G' as u16, b'u' as u16, b'i' as u16,
        b'd' as u16, 0,
    ];
    const VAL: &[u16] = &[
        b'M' as u16, b'a' as u16, b'c' as u16, b'h' as u16, b'i' as u16, b'n' as u16, b'e' as u16,
        b'G' as u16, b'u' as u16, b'i' as u16, b'd' as u16, 0,
    ];

    unsafe {
        let mut hkey = 0;
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, KEY.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return None;
        }
        let mut typ = 0u32;
        let mut data = [0u16; 128];
        let mut data_bytes = (data.len() * 2) as u32;
        let rc = RegQueryValueExW(
            hkey,
            VAL.as_ptr(),
            std::ptr::null_mut(),
            &mut typ,
            data.as_mut_ptr() as *mut u8,
            &mut data_bytes,
        );
        RegCloseKey(hkey);
        if rc != 0 {
            return None;
        }
        let nchars = (data_bytes as usize / 2).saturating_sub(1);
        let s = std::ffi::OsString::from_wide(&data[..nchars]);
        Some(s.to_string_lossy().into_owned())
    }
}

pub fn ticket_payload_bytes(t: &SessionTicket) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(t.licensee.as_bytes());
    h.update(b"|");
    h.update(t.device_id.as_bytes());
    h.update(b"|");
    h.update(t.issued_at.as_bytes());
    h.update(b"|");
    h.update(t.expires_at.as_bytes());
    h.update(b"|");
    h.update(t.last_attest_at.as_bytes());
    h.update(b"|");
    h.update(t.grace_until.as_bytes());
    h.finalize().to_vec()
}

pub fn sign_ticket(signing_key_hex: &str, mut ticket: SessionTicket) -> anyhow::Result<SessionTicket> {
    let bytes = hex::decode(signing_key_hex.trim())?;
    if bytes.len() != 32 {
        anyhow::bail!("会话票私钥须为 32 字节 hex");
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    let sk = SigningKey::from_bytes(&arr);
    ticket.sig = String::new();
    let msg = ticket_payload_bytes(&ticket);
    ticket.sig = hex::encode(sk.sign(&msg).to_bytes());
    Ok(ticket)
}

/// 构造并签名一张新票（供鉴权服使用）。
pub fn issue_ticket(
    signing_key_hex: &str,
    licensee: &str,
    device_id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<SessionTicket> {
    let issued = now.to_rfc3339();
    let expires = (now + Duration::try_days(TICKET_TTL_DAYS).unwrap()).to_rfc3339();
    let grace = (now + Duration::try_days(GRACE_DAYS).unwrap()).to_rfc3339();
    let ticket = SessionTicket {
        licensee: licensee.to_string(),
        device_id: device_id.to_string(),
        issued_at: issued.clone(),
        expires_at: expires,
        last_attest_at: issued,
        grace_until: grace,
        sig: String::new(),
    };
    sign_ticket(signing_key_hex, ticket)
}

pub fn verify_ticket_with_pubkey(ticket: &SessionTicket, pubkey_hex: &str) -> Result<(), String> {
    let pk_bytes = hex::decode(pubkey_hex).map_err(|_| "内置会话公钥无效".to_string())?;
    if pk_bytes.len() != 32 {
        return Err("内置会话公钥长度错误".into());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&pk_bytes);
    let vk = VerifyingKey::from_bytes(&arr).map_err(|_| "会话公钥格式错误".to_string())?;
    let sig_raw = hex::decode(ticket.sig.trim()).map_err(|_| "会话票签名无法解码".to_string())?;
    let sig = Signature::from_slice(&sig_raw).map_err(|_| "会话票签名格式错误".to_string())?;
    let msg = ticket_payload_bytes(ticket);
    vk.verify(&msg, &sig)
        .map_err(|_| "会话票签名无效".to_string())?;
    Ok(())
}

fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| format!("时间无法解析: {s}"))
}

/// 验签 + 宽限 + 时钟回拨；供本机门控与同步互验。
pub fn evaluate_ticket(ticket: &SessionTicket, pubkey_hex: &str, now: DateTime<Utc>) -> SessionStatus {
    if let Err(e) = verify_ticket_with_pubkey(ticket, pubkey_hex) {
        return SessionStatus::Invalid(e);
    }
    let last = match parse_rfc3339(&ticket.last_attest_at) {
        Ok(t) => t,
        Err(e) => return SessionStatus::Invalid(e),
    };
    let grace = match parse_rfc3339(&ticket.grace_until) {
        Ok(t) => t,
        Err(e) => return SessionStatus::Invalid(e),
    };
    if now + CLOCK_ROLLBACK_MAX < last {
        return SessionStatus::Expired("系统时间回拨，需重新联网激活".into());
    }
    if now > grace {
        return SessionStatus::Expired("会话宽限已过，需重新联网激活".into());
    }
    let remaining = (grace - now).num_days().max(0);
    SessionStatus::Active {
        licensee: ticket.licensee.clone(),
        grace_remaining_days: remaining,
    }
}

pub fn load_ticket(config_dir: &Path) -> Option<SessionTicket> {
    let path = ticket_path(config_dir);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn save_ticket(config_dir: &Path, ticket: &SessionTicket) -> anyhow::Result<()> {
    fs::create_dir_all(config_dir)?;
    let dest = ticket_path(config_dir);
    let tmp = dest.with_extension("ticket.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(ticket)?)?;
    fs::rename(&tmp, &dest)?;
    Ok(())
}

pub fn save_device_creds(config_dir: &Path, cert_pem: &str, key_pem: &str) -> anyhow::Result<()> {
    fs::create_dir_all(config_dir)?;
    fs::write(device_cert_path(config_dir), cert_pem)?;
    fs::write(device_key_path(config_dir), key_pem)?;
    Ok(())
}

pub fn load_device_creds(config_dir: &Path) -> Option<(String, String)> {
    let cert = fs::read_to_string(device_cert_path(config_dir)).ok()?;
    let key = fs::read_to_string(device_key_path(config_dir)).ok()?;
    if cert.trim().is_empty() || key.trim().is_empty() {
        return None;
    }
    Some((cert, key))
}

pub fn load_status(config_dir: &Path) -> SessionStatus {
    match load_ticket(config_dir) {
        None => SessionStatus::Missing,
        Some(t) => evaluate_ticket(&t, ATTEST_PUBKEY_HEX, Utc::now()),
    }
}

/// 同步门控：商业授权 ∧ 有效会话票。
pub fn allows_sync(config_dir: &Path, license_ok: bool) -> bool {
    license_ok && load_status(config_dir).allows_sync_now()
}

/// 供 PSK 派生的会话材料（同 licensee 各设备一致，便于互通）。
pub fn session_key_material(ticket: &SessionTicket) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"memo-session-psk-v1");
    h.update(ticket.licensee.as_bytes());
    let dig = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

/// 对端票互验：签名有效且未过宽限。
pub fn verify_peer_ticket(ticket: &SessionTicket) -> bool {
    matches!(
        evaluate_ticket(ticket, ATTEST_PUBKEY_HEX, Utc::now()),
        SessionStatus::Active { .. }
    )
}

pub fn status_label(s: &SessionStatus) -> String {
    match s {
        SessionStatus::Missing => "未激活（需联网激活一次）".into(),
        SessionStatus::Active {
            licensee,
            grace_remaining_days,
        } => format!("已激活 · {licensee} · 宽限剩余 {grace_remaining_days} 天"),
        SessionStatus::Expired(e) => format!("需续期: {e}"),
        SessionStatus::Invalid(e) => format!("会话无效: {e}"),
    }
}

/// 解析 license.json 原文为 LicenseInfo（激活请求用）。
pub fn read_license_file(config_dir: &Path) -> anyhow::Result<LicenseInfo> {
    let path = config_dir.join("license.json");
    let raw = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("无法读取 license.json: {e}"))?;
    Ok(serde_json::from_str(&raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn issue_and_evaluate_ok() {
        let sk = SigningKey::generate(&mut OsRng);
        let sk_hex = hex::encode(sk.to_bytes());
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        let now = Utc::now();
        let t = issue_ticket(&sk_hex, "Acme", "dev-1", now).unwrap();
        match evaluate_ticket(&t, &pk_hex, now) {
            SessionStatus::Active { licensee, .. } => assert_eq!(licensee, "Acme"),
            o => panic!("{o:?}"),
        }
        assert!(matches!(
            evaluate_ticket(&t, &pk_hex, now + Duration::try_days(GRACE_DAYS + 1).unwrap()),
            SessionStatus::Expired(_)
        ));
    }

    #[test]
    fn rollback_detected() {
        let sk = SigningKey::generate(&mut OsRng);
        let sk_hex = hex::encode(sk.to_bytes());
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        let now = Utc::now();
        let t = issue_ticket(&sk_hex, "Acme", "dev-1", now).unwrap();
        let past = now - Duration::try_days(2).unwrap();
        assert!(matches!(
            evaluate_ticket(&t, &pk_hex, past),
            SessionStatus::Expired(_)
        ));
    }

    #[test]
    fn bad_sig_invalid() {
        let sk = SigningKey::generate(&mut OsRng);
        let sk_hex = hex::encode(sk.to_bytes());
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        let mut t = issue_ticket(&sk_hex, "Acme", "dev-1", Utc::now()).unwrap();
        t.sig = "00".repeat(64);
        assert!(matches!(
            evaluate_ticket(&t, &pk_hex, Utc::now()),
            SessionStatus::Invalid(_)
        ));
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let sk = SigningKey::generate(&mut OsRng);
        let sk_hex = hex::encode(sk.to_bytes());
        let t = issue_ticket(&sk_hex, "Acme", "dev-1", Utc::now()).unwrap();
        save_ticket(dir.path(), &t).unwrap();
        let got = load_ticket(dir.path()).unwrap();
        assert_eq!(got, t);
    }
}
