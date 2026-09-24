//! 同步层 HMAC 握手（证明对端知道完整 salt，而非可嗅探的 salt_fp）。

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

const PSK_DOMAIN: &[u8] = b"memo-rs-sync-v1";
const HELLO_DOMAIN: &[u8] = b"hello-v1";

/// 由完整 salt 派生集群 PSK。
pub fn derive_psk(salt: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(PSK_DOMAIN);
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
    let mut mac =
        HmacSha256::new_from_slice(psk).expect("HMAC accepts 32-byte key");
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
