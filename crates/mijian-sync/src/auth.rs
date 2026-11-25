//! 同步层 HMAC 握手 + 加密 UDP 发现密钥。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

const PSK_DOMAIN: &[u8] = b"memo-rs-sync-v1";
const DISCO_DOMAIN: &[u8] = b"memo-rs-disco-v1";
const HELLO_DOMAIN: &[u8] = b"hello-v1";

/// 由完整 salt 派生集群 PSK。
pub fn derive_psk(salt: &[u8]) -> [u8; 32] {
    derive_psk_with_session(salt, None)
}

/// salt + 可选会话材料（票签名派生），提高无票私改客户端互通成本。
pub fn derive_psk_with_session(salt: &[u8], session_material: Option<&[u8; 32]>) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(PSK_DOMAIN);
    h.update(salt);
    if let Some(m) = session_material {
        h.update(b"|session|");
        h.update(m);
    }
    let dig = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

/// 发现用 AES-256-GCM 密钥。
pub fn derive_disco_key(salt: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(DISCO_DOMAIN);
    h.update(salt);
    let dig = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

pub fn random_nonce_hex() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

/// mac = HMAC-SHA256(psk, "hello-v1" || node_id || nonce)
pub fn hello_mac(psk: &[u8; 32], node_id: &str, nonce_hex: &str) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(psk).expect("HMAC accepts 32-byte key");
    mac.update(HELLO_DOMAIN);
    mac.update(node_id.as_bytes());
    mac.update(nonce_hex.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_hello_mac(psk: &[u8; 32], node_id: &str, nonce_hex: &str, mac_hex: &str) -> bool {
    let expected = hello_mac(psk, node_id, nonce_hex);
    if expected.len() != mac_hex.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected.bytes().zip(mac_hex.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoPlain {
    pub node_id: String,
    pub tcp_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoPacket {
    pub v: u8,
    pub blob: String,
}

/// 加密发现载荷 → 线上 v2 包。
pub fn seal_disco(key: &[u8; 32], plain: &DiscoPlain) -> anyhow::Result<DiscoPacket> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let json = serde_json::to_vec(plain)?;
    let mut ct = cipher
        .encrypt(nonce, json.as_ref())
        .map_err(|e| anyhow::anyhow!("disco encrypt: {e}"))?;
    let mut out = nonce_bytes.to_vec();
    out.append(&mut ct);
    Ok(DiscoPacket {
        v: 2,
        blob: B64.encode(out),
    })
}

/// 解密发现包；密钥错误或损坏则失败。
pub fn open_disco(key: &[u8; 32], pkt: &DiscoPacket) -> anyhow::Result<DiscoPlain> {
    if pkt.v != 2 {
        anyhow::bail!("unsupported disco version");
    }
    let blob = B64.decode(pkt.blob.trim())?;
    if blob.len() < 12 {
        anyhow::bail!("disco blob too short");
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ct)
        .map_err(|_| anyhow::anyhow!("disco decrypt failed"))?;
    Ok(serde_json::from_slice(&plain)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psk_with_session_differs() {
        let salt = [3u8; 16];
        let mat = [9u8; 32];
        assert_ne!(
            derive_psk_with_session(&salt, Some(&mat)),
            derive_psk(&salt)
        );
        assert_eq!(
            derive_psk_with_session(&salt, Some(&mat)),
            derive_psk_with_session(&salt, Some(&mat))
        );
    }

    #[test]
    fn psk_depends_on_full_salt() {
        let a = derive_psk(&[1u8; 16]);
        let b = derive_psk(&[2u8; 16]);
        assert_ne!(a, b);
        assert_eq!(derive_psk(&[1u8; 16]), a);
    }

    #[test]
    fn hello_mac_roundtrip() {
        let psk = derive_psk(b"0123456789abcdef");
        let nonce = random_nonce_hex();
        let mac = hello_mac(&psk, "node-1", &nonce);
        assert!(verify_hello_mac(&psk, "node-1", &nonce, &mac));
        assert!(!verify_hello_mac(&psk, "node-2", &nonce, &mac));
        assert!(!verify_hello_mac(&psk, "node-1", &nonce, "00"));
    }

    #[test]
    fn disco_roundtrip() {
        let key = derive_disco_key(&[9u8; 16]);
        let plain = DiscoPlain {
            node_id: "node-a".into(),
            tcp_port: 7000,
        };
        let pkt = seal_disco(&key, &plain).unwrap();
        assert_eq!(pkt.v, 2);
        let got = open_disco(&key, &pkt).unwrap();
        assert_eq!(got.node_id, "node-a");
        assert_eq!(got.tcp_port, 7000);
    }

    #[test]
    fn disco_wrong_key_fails() {
        let key = derive_disco_key(&[1u8; 16]);
        let bad = derive_disco_key(&[2u8; 16]);
        let pkt = seal_disco(
            &key,
            &DiscoPlain {
                node_id: "n".into(),
                tcp_port: 1,
            },
        )
        .unwrap();
        assert!(open_disco(&bad, &pkt).is_err());
    }
}
