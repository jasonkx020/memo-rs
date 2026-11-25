//! 主密码校验文件：防止空库任意密码“解锁”。

use std::fs;
use std::path::Path;

use crate::crypto;

const MAGIC: &str = "MEMO_VERIFIER_V1";

pub fn path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("verifier.dat")
}

pub fn exists(data_dir: &Path) -> bool {
    path(data_dir).exists()
}

/// 首次设密或迁移：写入用当前密钥加密的魔串。
pub fn write(data_dir: &Path, key: &[u8]) -> anyhow::Result<()> {
    fs::create_dir_all(data_dir)?;
    let ct = crypto::encrypt_string(key, MAGIC)?;
    fs::write(path(data_dir), ct)?;
    Ok(())
}

/// 校验派生密钥是否正确。
pub fn verify(data_dir: &Path, key: &[u8]) -> anyhow::Result<()> {
    let p = path(data_dir);
    if !p.exists() {
        anyhow::bail!("缺少密码校验文件");
    }
    let ct = fs::read_to_string(p)?;
    let plain = crypto::decrypt_string(key, ct.trim())
        .map_err(|_| anyhow::anyhow!("主密码错误"))?;
    if plain != MAGIC {
        anyhow::bail!("主密码错误");
    }
    Ok(())
}
