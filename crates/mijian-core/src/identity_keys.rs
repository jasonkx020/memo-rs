//! 身份密钥对：密钥对 = 人的身份；姓名仅为本节点别名。
//! 本机 `secret.key` 以登录密码（Argon2id + AES-GCM）加密存放。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

use crate::config::Argon2Params;
use crate::crypto;

pub const SCHEMA_VERSION: u32 = 2;
pub const MEMOKEY_MAGIC: &str = "MEMOKEY_V2";
pub const MEMOSECRET_MAGIC: &str = "MEMOSECRET_V1";
pub const MIN_LOCAL_PASSWORD_LEN: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityMeta {
    pub fingerprint: String,
    pub alias: String,
    #[serde(default)]
    pub exported_once: bool,
    #[serde(default)]
    pub created_at: String,
    /// 本机私钥是否已用登录密码加密（真源以 secret.key 格式为准）
    #[serde(default)]
    pub has_password: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoSecretFile {
    magic: String,
    salt_hex: String,
    argon2: Argon2Params,
    nonce_b64: String,
    ciphertext_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoKeyFile {
    magic: String,
    fingerprint: String,
    public_hex: String,
    /// 明文 hex 或 AES 包装后的 base64（见 encrypted）
    secret_hex: String,
    #[serde(default)]
    encrypted: bool,
    alias: String,
    /// Ed25519 签名（hex）；覆盖除自身外的稳定载荷
    sig_hex: String,
}

impl MemoKeyFile {
    fn sign_payload(&self) -> Vec<u8> {
        let enc = if self.encrypted { "1" } else { "0" };
        format!(
            "{}|{}|{}|{}|{}|{}",
            self.magic,
            self.fingerprint,
            self.public_hex,
            self.secret_hex,
            enc,
            self.alias
        )
        .into_bytes()
    }
}

#[derive(Clone)]
pub struct IdentityKeys {
    pub signing: SigningKey,
    pub fingerprint: String,
    pub alias: String,
    pub exported_once: bool,
}

/// 校验登录密码：非空、足够长、两次一致。
pub fn validate_local_password(password: &str, confirm: &str) -> anyhow::Result<()> {
    let p = password.trim();
    if p.is_empty() {
        anyhow::bail!("请设置登录密码");
    }
    if p.len() < MIN_LOCAL_PASSWORD_LEN {
        anyhow::bail!("登录密码至少 {MIN_LOCAL_PASSWORD_LEN} 位");
    }
    if p != confirm.trim() {
        anyhow::bail!("两次输入的登录密码不一致");
    }
    Ok(())
}

impl IdentityKeys {
    pub fn fingerprint_of(vk: &VerifyingKey) -> String {
        let dig = Sha256::digest(vk.as_bytes());
        hex::encode(&dig[..16])
    }

    pub fn short_fp(fp: &str) -> String {
        let s = fp.trim();
        if s.len() <= 12 {
            s.to_string()
        } else {
            format!("{}…{}", &s[..6], &s[s.len().saturating_sub(4)..])
        }
    }

    /// 内容加密密钥：SHA256("memo-content-v1" || secret)
    pub fn content_key(&self) -> Zeroizing<Vec<u8>> {
        let mut h = Sha256::new();
        h.update(b"memo-content-v1");
        h.update(self.signing.to_bytes());
        Zeroizing::new(h.finalize().to_vec())
    }

    pub fn generate(alias: &str) -> Self {
        let signing = SigningKey::generate(&mut OsRng);
        let fp = Self::fingerprint_of(&signing.verifying_key());
        Self {
            signing,
            fingerprint: fp,
            alias: alias.trim().to_string(),
            exported_once: false,
        }
    }

    pub fn from_secret(secret: [u8; 32], alias: &str) -> Self {
        let signing = SigningKey::from_bytes(&secret);
        let fp = Self::fingerprint_of(&signing.verifying_key());
        Self {
            signing,
            fingerprint: fp,
            alias: alias.trim().to_string(),
            exported_once: false,
        }
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.signing.verifying_key().as_bytes())
    }

    pub fn secret_hex(&self) -> String {
        hex::encode(self.signing.to_bytes())
    }

    pub fn identities_root(data_dir: &Path) -> PathBuf {
        data_dir.join("identities")
    }

    pub fn identity_dir(data_dir: &Path, fingerprint: &str) -> PathBuf {
        Self::identities_root(data_dir).join(fingerprint)
    }

    /// 每个身份独立的加密数据目录（备忘/人员/任务等）。
    pub fn vault_dir(data_dir: &Path, fingerprint: &str) -> PathBuf {
        data_dir.join("vaults").join(fingerprint)
    }

    pub fn list(data_dir: &Path) -> anyhow::Result<Vec<IdentityMeta>> {
        let root = Self::identities_root(data_dir);
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for ent in fs::read_dir(&root)? {
            let ent = ent?;
            if !ent.file_type()?.is_dir() {
                continue;
            }
            let meta_path = ent.path().join("meta.json");
            if !meta_path.exists() {
                continue;
            }
            let raw = fs::read_to_string(&meta_path)?;
            if let Ok(mut m) = serde_json::from_str::<IdentityMeta>(&raw) {
                // 以磁盘格式为准校正标志
                m.has_password = !Self::needs_password_setup(data_dir, &m.fingerprint);
                out.push(m);
            }
        }
        out.sort_by(|a, b| a.alias.cmp(&b.alias));
        Ok(out)
    }

    /// 本机私钥仍为明文 hex，须强制设登录密码。
    pub fn needs_password_setup(data_dir: &Path, fingerprint: &str) -> bool {
        let path = Self::identity_dir(data_dir, fingerprint).join("secret.key");
        let Ok(raw) = fs::read_to_string(&path) else {
            return false;
        };
        is_plaintext_secret(raw.trim())
    }

    /// 身份目录尚无 secret.key（新生成/刚导入未落盘）。
    pub fn secret_missing(data_dir: &Path, fingerprint: &str) -> bool {
        !Self::identity_dir(data_dir, fingerprint)
            .join("secret.key")
            .exists()
    }

    /// 用登录密码加载身份。加密文件验密解密；明文遗留仅用于迁移（随后须 seal）。
    pub fn load(data_dir: &Path, fingerprint: &str, password: &str) -> anyhow::Result<Self> {
        let dir = Self::identity_dir(data_dir, fingerprint);
        let secret_path = dir.join("secret.key");
        let meta_path = dir.join("meta.json");
        if !secret_path.exists() || !meta_path.exists() {
            anyhow::bail!("身份不存在: {fingerprint}");
        }
        let raw = fs::read_to_string(&secret_path)?;
        let trimmed = raw.trim();
        let secret_bytes = if is_plaintext_secret(trimmed) {
            // 迁移路径：允许无密读出，调用方须立即 seal
            let bytes = hex::decode(trimmed)?;
            if bytes.len() != 32 {
                anyhow::bail!("身份私钥长度错误");
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        } else {
            let file: MemoSecretFile = serde_json::from_str(trimmed)
                .map_err(|_| anyhow::anyhow!("本机密钥格式无效，请重新导入身份"))?;
            if file.magic != MEMOSECRET_MAGIC {
                anyhow::bail!("本机密钥格式无效（需要 MEMOSECRET_V1）");
            }
            if password.trim().is_empty() {
                anyhow::bail!("请输入登录密码");
            }
            decrypt_local_secret(&file, password.trim())?
        };
        let signing = SigningKey::from_bytes(&secret_bytes);
        let fp = Self::fingerprint_of(&signing.verifying_key());
        if fp != fingerprint {
            anyhow::bail!("指纹与私钥不匹配");
        }
        let meta: IdentityMeta = serde_json::from_str(&fs::read_to_string(&meta_path)?)?;
        Ok(Self {
            signing,
            fingerprint: fp,
            alias: meta.alias,
            exported_once: meta.exported_once,
        })
    }

    /// 用登录密码加密写入本机私钥，并更新 meta.has_password。
    pub fn seal_local_password(&self, data_dir: &Path, password: &str) -> anyhow::Result<()> {
        let p = password.trim();
        if p.len() < MIN_LOCAL_PASSWORD_LEN {
            anyhow::bail!("登录密码至少 {MIN_LOCAL_PASSWORD_LEN} 位");
        }
        let dir = Self::identity_dir(data_dir, &self.fingerprint);
        fs::create_dir_all(&dir)?;
        let file = encrypt_local_secret(&self.signing.to_bytes(), p)?;
        let secret_path = dir.join("secret.key");
        fs::write(&secret_path, serde_json::to_string_pretty(&file)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&secret_path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&secret_path, perms)?;
        }
        fs::write(dir.join("public.key"), self.public_hex())?;
        let mut meta = read_or_default_meta(data_dir, self)?;
        meta.has_password = true;
        meta.alias = self.alias.clone();
        meta.exported_once = self.exported_once;
        meta.fingerprint = self.fingerprint.clone();
        if meta.created_at.is_empty() {
            meta.created_at = chrono::Local::now().to_rfc3339();
        }
        fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
        Ok(())
    }

    /// 仅更新 meta / public.key，不改写密文私钥。
    pub fn save_meta(&self, data_dir: &Path) -> anyhow::Result<()> {
        let dir = Self::identity_dir(data_dir, &self.fingerprint);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("public.key"), self.public_hex())?;
        let mut meta = read_or_default_meta(data_dir, self)?;
        meta.alias = self.alias.clone();
        meta.exported_once = self.exported_once;
        meta.fingerprint = self.fingerprint.clone();
        meta.has_password = !Self::needs_password_setup(data_dir, &self.fingerprint)
            && !Self::secret_missing(data_dir, &self.fingerprint);
        if meta.created_at.is_empty() {
            meta.created_at = chrono::Local::now().to_rfc3339();
        }
        fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
        Ok(())
    }

    pub fn set_alias(&mut self, data_dir: &Path, alias: &str) -> anyhow::Result<()> {
        self.alias = alias.trim().to_string();
        self.save_meta(data_dir)
    }

    pub fn mark_exported(&mut self, data_dir: &Path) -> anyhow::Result<()> {
        self.exported_once = true;
        self.save_meta(data_dir)
    }

    /// 确保本机私钥已用登录密码密封（新身份或明文迁移）。
    pub fn ensure_sealed(&self, data_dir: &Path, local_password: &str) -> anyhow::Result<()> {
        if Self::secret_missing(data_dir, &self.fingerprint)
            || Self::needs_password_setup(data_dir, &self.fingerprint)
        {
            self.seal_local_password(data_dir, local_password)?;
        } else {
            self.save_meta(data_dir)?;
        }
        Ok(())
    }

    /// 用旧登录密码解密后，以新密码重新密封本机私钥。
    pub fn change_local_password(
        data_dir: &Path,
        fingerprint: &str,
        old_password: &str,
        new_password: &str,
    ) -> anyhow::Result<()> {
        let id = Self::load(data_dir, fingerprint, old_password)?;
        if Self::needs_password_setup(data_dir, fingerprint) {
            anyhow::bail!("请先完成登录密码设置");
        }
        // 旧密码错误时 load 已失败；新密码规则由调用方或此处校验
        if new_password.trim().len() < MIN_LOCAL_PASSWORD_LEN {
            anyhow::bail!("登录密码至少 {MIN_LOCAL_PASSWORD_LEN} 位");
        }
        if old_password.trim() == new_password.trim() {
            anyhow::bail!("新密码不能与当前密码相同");
        }
        id.seal_local_password(data_dir, new_password.trim())?;
        Ok(())
    }

    /// `.memokey` 是否为加密包（只读探测，不验签）。
    pub fn peek_encrypted(path: &Path) -> Option<bool> {
        let raw = fs::read_to_string(path).ok()?;
        let file: MemoKeyFile = serde_json::from_str(&raw).ok()?;
        if file.magic != MEMOKEY_MAGIC {
            return None;
        }
        Some(file.encrypted)
    }

    /// 导出 `.memokey`：明文私钥 + Ed25519 签名验真（无导出口令；`passphrase` 忽略）。
    pub fn export_file(&self, path: &Path, _passphrase: &str) -> anyhow::Result<()> {
        let alias = self.alias.trim();
        if alias.is_empty() {
            anyhow::bail!("导出前须设置显示名称");
        }
        let mut file = MemoKeyFile {
            magic: MEMOKEY_MAGIC.into(),
            fingerprint: self.fingerprint.clone(),
            public_hex: self.public_hex(),
            secret_hex: self.secret_hex(),
            encrypted: false,
            alias: alias.to_string(),
            sig_hex: String::new(),
        };
        let sig = self.signing.sign(&file.sign_payload());
        file.sig_hex = hex::encode(sig.to_bytes());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(&file)?)?;
        Ok(())
    }

    /// 导入 `.memokey`：验签 + 公私钥一致；别名取自包内。
    pub fn import_file(path: &Path, passphrase: &str) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)?;
        let file: MemoKeyFile = serde_json::from_str(&raw)?;
        if file.magic != MEMOKEY_MAGIC {
            anyhow::bail!("不是有效的 .memokey 文件（需要 MEMOKEY_V2）");
        }
        if file.alias.trim().is_empty() {
            anyhow::bail!("密钥包缺少显示名称");
        }
        if file.sig_hex.trim().is_empty() {
            anyhow::bail!("密钥包缺少完整性签名");
        }

        let secret_bytes = if file.encrypted {
            if passphrase.is_empty() {
                anyhow::bail!("该密钥包为旧版加密格式，需要口令");
            }
            unwrap_secret(passphrase.as_bytes(), &file.secret_hex)?
        } else {
            let b = hex::decode(file.secret_hex.trim())?;
            if b.len() != 32 {
                anyhow::bail!("私钥长度错误");
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        };

        let id = Self::from_secret(secret_bytes, file.alias.trim());
        if id.fingerprint != file.fingerprint {
            anyhow::bail!("密钥包指纹与私钥不一致（可能被篡改）");
        }
        if id.public_hex() != file.public_hex {
            anyhow::bail!("密钥包公钥与私钥不一致（可能被篡改）");
        }

        let sig_bytes = hex::decode(file.sig_hex.trim())
            .map_err(|_| anyhow::anyhow!("密钥包签名格式无效"))?;
        let sig = Signature::from_slice(&sig_bytes)
            .map_err(|_| anyhow::anyhow!("密钥包签名格式无效"))?;
        id.signing
            .verifying_key()
            .verify(&file.sign_payload(), &sig)
            .map_err(|_| anyhow::anyhow!("密钥包已损坏或被篡改"))?;

        Ok(id)
    }

    /// 只读预览包内别名（不验签也可用于选文件后展示；完整校验仍走 import_file）。
    pub fn peek_alias(path: &Path) -> Option<String> {
        let raw = fs::read_to_string(path).ok()?;
        let file: MemoKeyFile = serde_json::from_str(&raw).ok()?;
        let a = file.alias.trim();
        if a.is_empty() {
            None
        } else {
            Some(a.to_string())
        }
    }
}

fn read_or_default_meta(data_dir: &Path, id: &IdentityKeys) -> anyhow::Result<IdentityMeta> {
    let meta_path = IdentityKeys::identity_dir(data_dir, &id.fingerprint).join("meta.json");
    if meta_path.exists() {
        let raw = fs::read_to_string(&meta_path)?;
        if let Ok(m) = serde_json::from_str::<IdentityMeta>(&raw) {
            return Ok(m);
        }
    }
    Ok(IdentityMeta {
        fingerprint: id.fingerprint.clone(),
        alias: id.alias.clone(),
        exported_once: id.exported_once,
        created_at: chrono::Local::now().to_rfc3339(),
        has_password: false,
    })
}

fn is_plaintext_secret(raw: &str) -> bool {
    if raw.is_empty() || raw.starts_with('{') {
        return false;
    }
    // 64 hex chars = 32-byte Ed25519 secret
    raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit())
}

fn encrypt_local_secret(secret: &[u8; 32], password: &str) -> anyhow::Result<MemoSecretFile> {
    let argon2 = Argon2Params::default();
    let mut salt = vec![0u8; argon2.salt_len as usize];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = crypto::derive_key(password.as_bytes(), &salt, &argon2)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, secret.as_slice())
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    Ok(MemoSecretFile {
        magic: MEMOSECRET_MAGIC.into(),
        salt_hex: hex::encode(&salt),
        argon2,
        nonce_b64: B64.encode(nonce_bytes),
        ciphertext_b64: B64.encode(ct),
    })
}

fn decrypt_local_secret(file: &MemoSecretFile, password: &str) -> anyhow::Result<[u8; 32]> {
    let salt = hex::decode(file.salt_hex.trim())?;
    if salt.len() != file.argon2.salt_len as usize {
        anyhow::bail!("本机密钥 salt 长度错误");
    }
    let key = crypto::derive_key(password.as_bytes(), &salt, &file.argon2)?;
    let nonce_bytes = B64.decode(file.nonce_b64.trim())?;
    if nonce_bytes.len() != 12 {
        anyhow::bail!("本机密钥 nonce 长度错误");
    }
    let ct = B64.decode(file.ciphertext_b64.trim())?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ct.as_ref())
        .map_err(|_| anyhow::anyhow!("登录密码错误"))?;
    if plain.len() != 32 {
        anyhow::bail!("私钥长度错误");
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&plain);
    Ok(arr)
}

/// 旧版加密包格式（新导出已不用；保留供兼容测试）。
#[allow(dead_code)]
fn wrap_secret(passphrase: &[u8], secret: &[u8; 32]) -> anyhow::Result<String> {
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = sha_key(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, secret.as_slice())
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    let mut out = salt.to_vec();
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(B64.encode(out))
}

fn unwrap_secret(passphrase: &[u8], wrapped_b64: &str) -> anyhow::Result<[u8; 32]> {
    let blob = B64.decode(wrapped_b64)?;
    if blob.len() < 16 + 12 + 16 {
        anyhow::bail!("密钥包格式错误");
    }
    let (salt, rest) = blob.split_at(16);
    let (nonce_bytes, ct) = rest.split_at(12);
    let key = sha_key(passphrase, salt);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ct)
        .map_err(|_| anyhow::anyhow!("旧版包口令错误或密钥包损坏"))?;
    if plain.len() != 32 {
        anyhow::bail!("私钥长度错误");
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&plain);
    Ok(arr)
}

fn sha_key(passphrase: &[u8], salt: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"memo-wrap-v1");
    h.update(salt);
    h.update(passphrase);
    let dig = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&dig);
    out
}

/// 数据目录是否为新版 schema（含 identities 或 schema.json）。
pub fn is_v2_data_dir(data_dir: &Path) -> bool {
    let marker = data_dir.join("schema.json");
    if marker.exists() {
        if let Ok(raw) = fs::read_to_string(&marker) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v.get("version").and_then(|x| x.as_u64()) == Some(SCHEMA_VERSION as u64) {
                    return true;
                }
            }
        }
    }
    IdentityKeys::identities_root(data_dir).exists()
}

/// 旧版痕迹：verifier / 无 schema 的 audit+keys。
pub fn looks_like_legacy_data(data_dir: &Path) -> bool {
    if !data_dir.exists() {
        return false;
    }
    if is_v2_data_dir(data_dir) {
        return false;
    }
    data_dir.join("verifier.dat").exists()
        || data_dir.join("audit.jsonl").exists()
        || data_dir.join("keys").join("private.key").exists()
        || data_dir.join("persons.json.enc").exists()
}

pub fn write_schema_marker(data_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(data_dir)?;
    let v = serde_json::json!({ "version": SCHEMA_VERSION });
    fs::write(data_dir.join("schema.json"), serde_json::to_string_pretty(&v)?)?;
    Ok(())
}

pub fn wipe_data_dir(data_dir: &Path) -> anyhow::Result<()> {
    if data_dir.exists() {
        fs::remove_dir_all(data_dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_data_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "memo-id-test-{}-{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn seal_load_roundtrip_and_wrong_password() {
        let dir = tmp_data_dir("seal");
        let id = IdentityKeys::generate("测试");
        id.seal_local_password(&dir, "secret12").unwrap();
        assert!(!IdentityKeys::needs_password_setup(&dir, &id.fingerprint));
        let loaded = IdentityKeys::load(&dir, &id.fingerprint, "secret12").unwrap();
        assert_eq!(loaded.fingerprint, id.fingerprint);
        assert_eq!(loaded.alias, "测试");
        let err = IdentityKeys::load(&dir, &id.fingerprint, "wrong!!!")
            .err()
            .expect("wrong password should fail");
        assert!(err.to_string().contains("登录密码错误"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plaintext_needs_setup_then_seal() {
        let dir = tmp_data_dir("plain");
        let id = IdentityKeys::generate("旧户");
        let idir = IdentityKeys::identity_dir(&dir, &id.fingerprint);
        fs::create_dir_all(&idir).unwrap();
        fs::write(idir.join("secret.key"), id.secret_hex()).unwrap();
        fs::write(
            idir.join("meta.json"),
            serde_json::to_string(&IdentityMeta {
                fingerprint: id.fingerprint.clone(),
                alias: id.alias.clone(),
                exported_once: false,
                created_at: String::new(),
                has_password: false,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(IdentityKeys::needs_password_setup(&dir, &id.fingerprint));
        let loaded = IdentityKeys::load(&dir, &id.fingerprint, "").unwrap();
        loaded.seal_local_password(&dir, "migrate1").unwrap();
        assert!(!IdentityKeys::needs_password_setup(&dir, &id.fingerprint));
        IdentityKeys::load(&dir, &id.fingerprint, "migrate1").unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_local_password_rules() {
        assert!(validate_local_password("abc", "abc").is_err());
        assert!(validate_local_password("abcdef", "abcdefg").is_err());
        assert!(validate_local_password("abcdef", "abcdef").is_ok());
    }

    #[test]
    fn change_local_password_roundtrip() {
        let dir = tmp_data_dir("chpwd");
        let id = IdentityKeys::generate("改密");
        id.seal_local_password(&dir, "oldpass1").unwrap();
        IdentityKeys::change_local_password(&dir, &id.fingerprint, "oldpass1", "newpass2")
            .unwrap();
        assert!(IdentityKeys::load(&dir, &id.fingerprint, "oldpass1").is_err());
        IdentityKeys::load(&dir, &id.fingerprint, "newpass2").unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_file_plain_signed_and_tamper_fails() {
        let dir = tmp_data_dir("exp");
        let id = IdentityKeys::generate("导出");
        let path = dir.join("t.memokey");
        id.export_file(&path, "").unwrap();
        assert_eq!(IdentityKeys::peek_encrypted(&path), Some(false));
        let loaded = IdentityKeys::import_file(&path, "").unwrap();
        assert_eq!(loaded.fingerprint, id.fingerprint);
        // 篡改私钥后验签应失败
        let mut raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let bad = "00".repeat(32);
        raw["secret_hex"] = serde_json::Value::String(bad);
        fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();
        assert!(IdentityKeys::import_file(&path, "").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
