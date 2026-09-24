use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use zeroize::{Zeroize, Zeroizing};

use crate::config::{Argon2Params, Config};

/// Argon2id 派生 32 字节密钥；调用方负责 Zeroize。
pub fn derive_key(password: &[u8], salt: &[u8], p: &Argon2Params) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    let argon = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(p.memory, p.iterations, p.parallelism, Some(p.key_len as usize))
            .map_err(|e| anyhow::anyhow!("argon2 params: {e}"))?,
    );
    let mut out = vec![0u8; p.key_len as usize];
    argon
        .hash_password_into(password, salt, &mut out)
        .map_err(|e| anyhow::anyhow!("argon2: {e}"))?;
    Ok(Zeroizing::new(out))
}

pub fn resolve_salt(cfg: &Config) -> anyhow::Result<Vec<u8>> {
    let salt = hex::decode(cfg.salt_hex.trim())?;
    if salt.len() != cfg.argon2.salt_len as usize {
        anyhow::bail!(
            "salt 长度不匹配: got {} want {}",
            salt.len(),
            cfg.argon2.salt_len
        );
    }
    Ok(salt)
}

pub fn encrypt_string(key: &[u8], plaintext: &str) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    let mut out = nonce_bytes.to_vec();
    out.append(&mut ct);
    Ok(B64.encode(out))
}

pub fn decrypt_string(key: &[u8], ciphertext_b64: &str) -> anyhow::Result<String> {
    let blob = B64.decode(ciphertext_b64)?;
    if blob.len() < 12 {
        anyhow::bail!("密文过短");
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ct)
        .map_err(|_| anyhow::anyhow!("解密失败（密码错误或数据被篡改）"))?;
    Ok(String::from_utf8(plain)?)
}

pub fn keys_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // 常量时间比较
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn zeroize_bytes(b: &mut [u8]) {
    b.zeroize();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Argon2Params;

    fn fast_params() -> Argon2Params {
        Argon2Params {
            memory: 8192,
            iterations: 1,
            parallelism: 1,
            salt_len: 16,
            key_len: 32,
        }
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let salt = [9u8; 16];
        let key = derive_key(b"secret", &salt, &fast_params()).unwrap();
        let ct = encrypt_string(&key, "你好 memo").unwrap();
        let pt = decrypt_string(&key, &ct).unwrap();
        assert_eq!(pt, "你好 memo");
    }

    #[test]
    fn wrong_key_fails() {
        let salt = [9u8; 16];
        let key = derive_key(b"secret", &salt, &fast_params()).unwrap();
        let bad = derive_key(b"other", &salt, &fast_params()).unwrap();
        let ct = encrypt_string(&key, "data").unwrap();
        assert!(decrypt_string(&bad, &ct).is_err());
    }

    #[test]
    fn keys_equal_length_mismatch() {
        assert!(!keys_equal(b"ab", b"a"));
        assert!(keys_equal(b"ab", b"ab"));
        assert!(!keys_equal(b"ab", b"ac"));
    }
}
