//! 加密备份 / 导入（`.memobak`）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::config::{Argon2Params, Config};
use crate::crypto::{self, derive_key, resolve_salt};
use crate::service::MemoView;

const FORMAT: &str = "memo-bak-v1";

#[derive(Debug, Serialize, Deserialize)]
struct BackupFile {
    format: String,
    salt_hex: String,
    argon2: Argon2Params,
    /// AES-GCM(base64) 包裹的 JSON 明文
    ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupPayload {
    node_id: String,
    exported_at: String,
    memos: Vec<BackupMemo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupMemo {
    id: String,
    title: String,
    content: String,
    version: u64,
    node_id: String,
}

/// 用给定密码加密导出全部可见备忘。
pub fn export_encrypted(
    path: &Path,
    cfg: &Config,
    password: &str,
    items: &[MemoView],
) -> anyhow::Result<()> {
    if password.is_empty() {
        anyhow::bail!("备份密码不能为空");
    }
    if items.is_empty() {
        anyhow::bail!("没有可备份的备忘");
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let salt = resolve_salt(cfg)?;
    let key = derive_key(password.as_bytes(), &salt, &cfg.argon2)?;
    let payload = BackupPayload {
        node_id: cfg.node_id.clone(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        memos: items
            .iter()
            .map(|m| BackupMemo {
                id: m.id.clone(),
                title: m.title.clone(),
                content: m.content.clone(),
                version: m.version,
                node_id: m.node_id.clone(),
            })
            .collect(),
    };
    let plain = serde_json::to_string(&payload)?;
    let ciphertext = crypto::encrypt_string(&key, &plain)?;
    let file = BackupFile {
        format: FORMAT.into(),
        salt_hex: cfg.salt_hex.clone(),
        argon2: cfg.argon2.clone(),
        ciphertext,
    };
    fs::write(path, serde_json::to_string_pretty(&file)?)?;
    Ok(())
}

/// 解密备份，返回明文备忘列表（调用方负责写入 store）。
pub fn import_encrypted(path: &Path, password: &str) -> anyhow::Result<Vec<MemoView>> {
    if password.is_empty() {
        anyhow::bail!("备份密码不能为空");
    }
    let raw = fs::read_to_string(path)?;
    let file: BackupFile = serde_json::from_str(&raw)?;
    if file.format != FORMAT {
        anyhow::bail!("不支持的备份格式: {}", file.format);
    }
    let salt = hex::decode(file.salt_hex.trim())?;
    if salt.len() != file.argon2.salt_len as usize {
        anyhow::bail!("备份盐长度异常");
    }
    let key = derive_key(password.as_bytes(), &salt, &file.argon2)?;
    let plain = crypto::decrypt_string(&key, &file.ciphertext)
        .map_err(|_| anyhow::anyhow!("备份密码错误或文件损坏"))?;
    let payload: BackupPayload = serde_json::from_str(&plain)?;
    Ok(payload
        .memos
        .into_iter()
        .map(|m| MemoView {
            id: m.id,
            title: m.title,
            content: m.content,
            deleted: false,
            version: m.version,
            node_id: m.node_id,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Argon2Params, Config};

    fn test_cfg(salt_hex: &str) -> Config {
        Config {
            node_id: "n1".into(),
            node_display_name: String::new(),
            data_dir: ".".into(),
            listen_port: 7000,
            peers: vec![],
            salt_hex: salt_hex.into(),
            lan_discovery: false,
            argon2: Argon2Params {
                memory: 8192,
                iterations: 1,
                parallelism: 1,
                salt_len: 16,
                key_len: 32,
            },
        }
    }

    #[test]
    fn backup_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let salt = hex::encode([1u8; 16]);
        let cfg = test_cfg(&salt);
        let path = dir.path().join("t.memobak");
        let items = vec![MemoView {
            id: "1".into(),
            title: "标题".into(),
            content: "正文".into(),
            deleted: false,
            version: 3,
            node_id: "n1".into(),
        }];
        export_encrypted(&path, &cfg, "pw123", &items).unwrap();
        let got = import_encrypted(&path, "pw123").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "标题");
        assert_eq!(got[0].content, "正文");
    }

    #[test]
    fn wrong_password_fails() {
        let dir = tempfile::tempdir().unwrap();
        let salt = hex::encode([2u8; 16]);
        let cfg = test_cfg(&salt);
        let path = dir.path().join("t.memobak");
        let items = vec![MemoView {
            id: "1".into(),
            title: "t".into(),
            content: "c".into(),
            deleted: false,
            version: 1,
            node_id: "n1".into(),
        }];
        export_encrypted(&path, &cfg, "right", &items).unwrap();
        assert!(import_encrypted(&path, "wrong").is_err());
    }
}
