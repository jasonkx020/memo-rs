//! 备忘附件：本地文件 + 备份/托管打包。

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::memo_doc;

/// 单文件明文上限（备份/托管打包前）。
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedAttachment {
    pub file_id: String,
    pub name: String,
    pub data_b64: String,
}

pub fn memo_dir(data_dir: &Path, memo_id: &str) -> PathBuf {
    data_dir.join("attachments").join(memo_id)
}

pub fn path_of(data_dir: &Path, memo_id: &str, file_id: &str) -> PathBuf {
    memo_dir(data_dir, memo_id).join(file_id)
}

/// 写入图片；`ext` 如 `png` / `jpg`。返回 `file_id`（含扩展名）。
pub fn add_image(
    data_dir: &Path,
    memo_id: &str,
    bytes: &[u8],
    ext: &str,
) -> anyhow::Result<String> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        anyhow::bail!("图片超过 {} MB 上限", MAX_FILE_BYTES / (1024 * 1024));
    }
    let ext = ext
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    let ext = match ext.as_str() {
        "jpg" | "jpeg" => "jpg",
        "png" => "png",
        "gif" => "gif",
        "webp" => "webp",
        "" => "png",
        other => other,
    };
    let id = format!(
        "{}.{}",
        Uuid::new_v4().to_string().replace('-', ""),
        ext
    );
    let dir = memo_dir(data_dir, memo_id);
    fs::create_dir_all(&dir)?;
    fs::write(path_of(data_dir, memo_id, &id), bytes)?;
    Ok(id)
}

pub fn delete_memo_dir(data_dir: &Path, memo_id: &str) {
    let dir = memo_dir(data_dir, memo_id);
    let _ = fs::remove_dir_all(dir);
}

/// 按正文引用打包附件（缺文件则跳过）。
pub fn pack_for_content(
    data_dir: &Path,
    memo_id: &str,
    content: &str,
) -> anyhow::Result<Vec<PackedAttachment>> {
    let mut out = Vec::new();
    for file_id in memo_doc::collect_file_ids(content) {
        let path = path_of(data_dir, memo_id, &file_id);
        if !path.is_file() {
            continue;
        }
        let meta = fs::metadata(&path)?;
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let bytes = fs::read(&path)?;
        out.push(PackedAttachment {
            file_id: file_id.clone(),
            name: file_id.clone(),
            data_b64: B64.encode(&bytes),
        });
    }
    Ok(out)
}

/// 还原附件到本地目录。
pub fn restore_packed(
    data_dir: &Path,
    memo_id: &str,
    items: &[PackedAttachment],
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let dir = memo_dir(data_dir, memo_id);
    fs::create_dir_all(&dir)?;
    for it in items {
        if it.file_id.trim().is_empty() {
            continue;
        }
        let Ok(bytes) = B64.decode(it.data_b64.trim()) else {
            continue;
        };
        if bytes.len() as u64 > MAX_FILE_BYTES {
            continue;
        }
        fs::write(path_of(data_dir, memo_id, &it.file_id), bytes)?;
    }
    Ok(())
}
