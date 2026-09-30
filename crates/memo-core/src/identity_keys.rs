//! 身份密钥对：密钥对 = 人的身份；姓名仅为本节点别名。

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

pub const SCHEMA_VERSION: u32 = 2;
pub const MEMOKEY_MAGIC: &str = "MEMOKEY_V2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityMeta {
    pub fingerprint: String,
    pub alias: String,
    #[serde(default)]
    pub exported_once: bool,
    #[serde(default)]
    pub created_at: String,
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
            if let Ok(m) = serde_json::from_str::<IdentityMeta>(&raw) {
                out.push(m);
            }
        }
        out.sort_by(|a, b| a.alias.cmp(&b.alias));
        Ok(out)
    }

    pub fn load(data_dir: &Path, fingerprint: &str) -> anyhow::Result<Self> {
        let dir = Self::identity_dir(data_dir, fingerprint);
        let secret_path = dir.join("secret.key");
        let meta_path = dir.join("meta.json");
        if !secret_path.exists() || !meta_path.exists() {
            anyhow::bail!("身份不存在: {fingerprint}");
        }
        let hex_str = fs::read_to_string(&secret_path)?;
        let bytes = hex::decode(hex_str.trim())?;
        if bytes.len() != 32 {
            anyhow::bail!("身份私钥长度错误");
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        let signing = SigningKey::from_bytes(&arr);
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

    pub fn save(&self, data_dir: &Path) -> anyhow::Result<()> {
        let dir = Self::identity_dir(data_dir, &self.fingerprint);
        fs::create_dir_all(&dir)?;
        let secret_path = dir.join("secret.key");
        fs::write(&secret_path, self.secret_hex())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&secret_path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&secret_path, perms)?;
        }
        fs::write(
            dir.join("public.key"),
            self.public_hex(),
        )?;
        let meta = IdentityMeta {
            fingerprint: self.fingerprint.clone(),
            alias: self.alias.clone(),
            exported_once: self.exported_once,
            created_at: chrono::Local::now().to_rfc3339(),
        };
        fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
        Ok(())
    }

    pub fn set_alias(&mut self, data_dir: &Path, alias: &str) -> anyhow::Result<()> {
        self.alias = alias.trim().to_string();
        self.save(data_dir)
    }

    pub fn mark_exported(&mut self, data_dir: &Path) -> anyhow::Result<()> {
        self.exported_once = true;
        self.save(data_dir)
    }

    /// 导出 `.memokey`；`passphrase` 非空则 AES 包装私钥；含别名与完整性签名。
    pub fn export_file(&self, path: &Path, passphrase: &str) -> anyhow::Result<()> {
        let alias = self.alias.trim();
        if alias.is_empty() {
            anyhow::bail!("导出前须设置显示名称");
        }
        let (secret_hex, encrypted) = if passphrase.is_empty() {
            (self.secret_hex(), false)
        } else {
            let wrap = wrap_secret(passphrase.as_bytes(), &self.signing.to_bytes())?;
            (wrap, true)
        };
        let mut file = MemoKeyFile {
            magic: MEMOKEY_MAGIC.into(),
            fingerprint: self.fingerprint.clone(),
            public_hex: self.public_hex(),
            secret_hex,
            encrypted,
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
                anyhow::bail!("该密钥包需要保险口令");
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
        .map_err(|_| anyhow::anyhow!("保险口令错误或密钥包损坏"))?;
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
