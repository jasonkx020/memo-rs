//! 主密钥加密的 JSON 快照文件（人员 / 任务持久化）。

use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::crypto;

pub fn file_path(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join(name)
}

pub fn load_vec<T: DeserializeOwned>(
    data_dir: &Path,
    name: &str,
    key: &[u8],
) -> anyhow::Result<Vec<T>> {
    let p = file_path(data_dir, name);
    if !p.exists() {
        return Ok(Vec::new());
    }
    let ct = fs::read_to_string(&p)?;
    if ct.trim().is_empty() {
        return Ok(Vec::new());
    }
    let plain = crypto::decrypt_string(key, ct.trim())
        .map_err(|_| anyhow::anyhow!("解密 {name} 失败（主密码或文件损坏）"))?;
    if plain.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&plain)?)
}

pub fn save_vec<T: Serialize>(
    data_dir: &Path,
    name: &str,
    key: &[u8],
    items: &[T],
) -> anyhow::Result<()> {
    fs::create_dir_all(data_dir)?;
    let plain = serde_json::to_string(items)?;
    let ct = crypto::encrypt_string(key, &plain)?;
    fs::write(file_path(data_dir, name), ct)?;
    Ok(())
}
