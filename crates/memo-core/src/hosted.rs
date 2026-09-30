//! 异地托管：按身份公钥指纹存放密文，托管方不可解。

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::crypto;
use crate::store::MemoLifecycle;

const HOSTED_FILE: &str = "hosted_index.json";
const HOSTED_MEMO_FORMAT: &str = "memo-hosted-v1";

/// 私有托管密文内层：标题 + 正文（整体再经身份密钥加密）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedMemoPayload {
    pub format: String,
    pub title: String,
    pub content: String,
}

impl HostedMemoPayload {
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            format: HOSTED_MEMO_FORMAT.into(),
            title: title.into(),
            content: content.into(),
        }
    }

    pub fn encode_ciphertext(key: &[u8], title: &str, plain_content: &str) -> anyhow::Result<String> {
        let json = serde_json::to_string(&Self::new(title, plain_content))?;
        crypto::encrypt_string(key, &json)
    }

    /// 解析托管密文 → (标题, 写入 store 用的 content 密文)
    /// - v1：密文内为 JSON{title,content}，content 为明文，需再加密入库
    /// - 旧版：密文即为 store 侧 content 密文；标题取 title_hint 或正文首行
    pub fn decode_for_restore(
        key: &[u8],
        blob_ciphertext: &str,
        title_hint: &str,
    ) -> anyhow::Result<(String, String)> {
        let plain = crypto::decrypt_string(key, blob_ciphertext)?;
        if let Ok(payload) = serde_json::from_str::<Self>(&plain) {
            if payload.format == HOSTED_MEMO_FORMAT {
                let content_ct = crypto::encrypt_string(key, &payload.content)?;
                let title = if payload.title.trim().is_empty() {
                    title_from_plain(&payload.content)
                } else {
                    payload.title
                };
                return Ok((title, content_ct));
            }
        }
        // 旧版：blob.ciphertext 与 MemoItem.content 同形（已是正文密文）
        let title = {
            let hint = title_hint.trim();
            if !hint.is_empty() && !is_restore_placeholder(hint) {
                hint.to_string()
            } else {
                title_from_plain(&plain)
            }
        };
        Ok((title, blob_ciphertext.to_string()))
    }
}

/// 历史恢复占位标题：`恢复` + memo_id 前缀
pub fn is_restore_placeholder(title: &str) -> bool {
    let t = title.trim();
    t.starts_with("恢复 ") && t.chars().count() <= 20
}

fn title_from_plain(plain: &str) -> String {
    let line = plain.lines().next().unwrap_or("").trim();
    let t: String = line.chars().take(40).collect();
    if t.is_empty() {
        "未命名备忘".into()
    } else {
        t
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedBlob {
    pub owner_fp: String,
    pub memo_id: String,
    pub version: u64,
    pub ciphertext: String,
    pub title_hint: String,
    #[serde(default)]
    pub visibility: String,
    /// 内容侧修改时间（来自从机 memo.modified_at）
    #[serde(default)]
    pub content_modified_at: String,
    /// 主机完成写入本条备份的时间
    #[serde(default)]
    pub backed_up_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub source_node: String,
    #[serde(default)]
    pub lifecycle: MemoLifecycle,
    /// 软删 tombstone：保留密文供回收站恢复
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub deleted_at: String,
}

/// 主机返回给从机的备份元数据（无密文），用于增量判断。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMetaItem {
    pub memo_id: String,
    pub version: u64,
    #[serde(default)]
    pub content_modified_at: String,
    #[serde(default)]
    pub backed_up_at: String,
    /// 主机侧是否已保存明文标题提示（旧备份可能为 false）
    #[serde(default)]
    pub has_title_hint: bool,
    /// 主机侧已是软删 tombstone
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEscrowBlob {
    pub owner_fp: String,
    pub ciphertext: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub source_node: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HostedIndex {
    blobs: Vec<HostedBlob>,
    #[serde(default)]
    escrows: Vec<KeyEscrowBlob>,
}

pub struct HostedStore {
    dir: PathBuf,
    index: RwLock<HostedIndex>,
}

impl HostedStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let dir = data_dir.join("hosted");
        fs::create_dir_all(&dir)?;
        let path = dir.join(HOSTED_FILE);
        let index = if path.exists() {
            serde_json::from_str(&fs::read_to_string(&path)?)?
        } else {
            HostedIndex::default()
        };
        Ok(Self {
            dir,
            index: RwLock::new(index),
        })
    }

    fn persist(&self) -> anyhow::Result<()> {
        let path = self.dir.join(HOSTED_FILE);
        let idx = self.index.read();
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(&*idx)?)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// LWW：同 owner_fp+memo_id 取更高 version。
    pub fn upsert_blob(&self, mut blob: HostedBlob) -> anyhow::Result<bool> {
        if blob.backed_up_at.trim().is_empty() {
            blob.backed_up_at = chrono::Local::now().to_rfc3339();
        }
        if blob.updated_at.trim().is_empty() {
            blob.updated_at = blob.backed_up_at.clone();
        }
        let mut idx = self.index.write();
        if let Some(existing) = idx
            .blobs
            .iter_mut()
            .find(|b| b.owner_fp == blob.owner_fp && b.memo_id == blob.memo_id)
        {
            if blob.version > existing.version
                || (blob.version == existing.version && blob.source_node > existing.source_node)
            {
                *existing = blob;
                drop(idx);
                self.persist()?;
                return Ok(true);
            }
            return Ok(false);
        }
        idx.blobs.push(blob);
        drop(idx);
        self.persist()?;
        Ok(true)
    }

    pub fn blobs_for_owner(&self, owner_fp: &str) -> Vec<HostedBlob> {
        self.index
            .read()
            .blobs
            .iter()
            .filter(|b| b.owner_fp == owner_fp)
            .cloned()
            .collect()
    }

    pub fn blobs_for_ids(&self, owner_fp: &str, memo_ids: &[String]) -> Vec<HostedBlob> {
        if memo_ids.is_empty() {
            return Vec::new();
        }
        let want: HashSet<&str> = memo_ids.iter().map(|s| s.as_str()).collect();
        self.index
            .read()
            .blobs
            .iter()
            .filter(|b| b.owner_fp == owner_fp && want.contains(b.memo_id.as_str()))
            .cloned()
            .collect()
    }

    pub fn meta_for_owner(&self, owner_fp: &str) -> Vec<BackupMetaItem> {
        self.blobs_for_owner(owner_fp)
            .into_iter()
            .map(|b| BackupMetaItem {
                memo_id: b.memo_id,
                version: b.version,
                content_modified_at: if b.content_modified_at.is_empty() {
                    b.updated_at
                } else {
                    b.content_modified_at
                },
                backed_up_at: b.backed_up_at,
                has_title_hint: !b.title_hint.trim().is_empty(),
                deleted: b.deleted,
            })
            .collect()
    }

    pub fn all_blobs(&self) -> Vec<HostedBlob> {
        self.index.read().blobs.clone()
    }

    /// 按 owner+memo_id 抹除托管备份
    pub fn remove_blob(&self, owner_fp: &str, memo_id: &str) -> anyhow::Result<bool> {
        let mut idx = self.index.write();
        let before = idx.blobs.len();
        idx.blobs
            .retain(|b| !(b.owner_fp == owner_fp && b.memo_id == memo_id));
        let removed = before != idx.blobs.len();
        if removed {
            drop(idx);
            self.persist()?;
        }
        Ok(removed)
    }

    /// 删除已过期（非永久且到期）的托管备份；返回删除条数。
    pub fn purge_expired(&self) -> anyhow::Result<usize> {
        let mut idx = self.index.write();
        let before = idx.blobs.len();
        idx.blobs.retain(|b| !b.lifecycle.is_expired());
        let n = before - idx.blobs.len();
        if n > 0 {
            drop(idx);
            self.persist()?;
        }
        Ok(n)
    }

    /// 在仍不足时，按 backed_up_at 最旧优先删除已过期；若无过期可删，再删非永久中最旧的。
    pub fn purge_for_space(&self, need_bytes_hint: u64) -> anyhow::Result<usize> {
        let _ = need_bytes_hint;
        let mut n = self.purge_expired()?;
        if n > 0 {
            return Ok(n);
        }
        // 无过期项：删除最旧的非永久备份（最多一批 8 条）以腾挪
        let mut idx = self.index.write();
        let mut candidates: Vec<(usize, String)> = idx
            .blobs
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.lifecycle.is_permanent())
            .map(|(i, b)| {
                let key = if b.backed_up_at.is_empty() {
                    b.updated_at.clone()
                } else {
                    b.backed_up_at.clone()
                };
                (i, key)
            })
            .collect();
        if candidates.is_empty() {
            return Ok(0);
        }
        candidates.sort_by(|a, b| a.1.cmp(&b.1));
        let remove_n = candidates.len().min(8);
        let mut remove_idx: Vec<usize> = candidates.into_iter().take(remove_n).map(|(i, _)| i).collect();
        remove_idx.sort_unstable_by(|a, b| b.cmp(a));
        for i in remove_idx {
            idx.blobs.remove(i);
            n += 1;
        }
        drop(idx);
        if n > 0 {
            self.persist()?;
        }
        Ok(n)
    }

    pub fn upsert_escrow(&self, escrow: KeyEscrowBlob) -> anyhow::Result<()> {
        let mut idx = self.index.write();
        if let Some(e) = idx.escrows.iter_mut().find(|e| e.owner_fp == escrow.owner_fp) {
            *e = escrow;
        } else {
            idx.escrows.push(escrow);
        }
        drop(idx);
        self.persist()
    }

    pub fn escrow_for(&self, owner_fp: &str) -> Option<KeyEscrowBlob> {
        self.index
            .read()
            .escrows
            .iter()
            .find(|e| e.owner_fp == owner_fp)
            .cloned()
    }
}
