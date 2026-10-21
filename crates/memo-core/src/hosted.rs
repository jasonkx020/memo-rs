//! 异地托管：按身份公钥指纹存放密文，托管方不可解。

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::attachments::PackedAttachment;
use crate::crypto;
use crate::store::{MemoCategory, MemoItem, MemoLifecycle, MemoPriority};

const HOSTED_FILE: &str = "hosted_index.json";
const HOSTED_MEMO_FORMAT_V1: &str = "memo-hosted-v1";
const HOSTED_MEMO_FORMAT_V2: &str = "memo-hosted-v2";
const HOSTED_MEMO_FORMAT_V3: &str = "memo-hosted-v3";

/// 导航元数据（v3 起写入托管密文；决定恢复后任务/备忘/私密页签）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostedNavMeta {
    #[serde(default)]
    pub category: MemoCategory,
    #[serde(default)]
    pub due_date: String,
    #[serde(default)]
    pub end_date: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub priority: MemoPriority,
    #[serde(default)]
    pub remind_before_days: u32,
    #[serde(default)]
    pub remind_seen_for: String,
}

impl Default for HostedNavMeta {
    fn default() -> Self {
        Self {
            category: MemoCategory::General,
            due_date: String::new(),
            end_date: String::new(),
            done: false,
            tags: vec![],
            priority: MemoPriority::Normal,
            remind_before_days: 0,
            remind_seen_for: String::new(),
        }
    }
}

impl HostedNavMeta {
    pub fn from_item(item: &MemoItem) -> Self {
        Self {
            category: item.category.canonical(),
            due_date: item.due_date.clone(),
            end_date: item.end_date.clone(),
            done: item.done,
            tags: item.tags.clone(),
            priority: item.priority,
            remind_before_days: item.remind_before_days,
            remind_seen_for: item.remind_seen_for.clone(),
        }
    }

    pub fn from_existing(item: &MemoItem) -> Self {
        Self::from_item(item)
    }
}

/// `decode_for_restore` 结果。
#[derive(Debug, Clone)]
pub struct RestoredHostedMemo {
    pub title: String,
    /// store 侧 content 密文
    pub content_ct: String,
    pub attachments: Vec<PackedAttachment>,
    /// v3 有完整导航元数据；v1/v2/旧版为 `None`
    pub nav: Option<HostedNavMeta>,
}

/// 私有托管密文内层：标题 + 正文 + 附件 +（v3）导航元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedMemoPayload {
    pub format: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<PackedAttachment>,
    #[serde(default)]
    pub category: MemoCategory,
    #[serde(default)]
    pub due_date: String,
    #[serde(default)]
    pub end_date: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub priority: MemoPriority,
    #[serde(default)]
    pub remind_before_days: u32,
    #[serde(default)]
    pub remind_seen_for: String,
}

impl HostedMemoPayload {
    pub fn from_item(
        item: &MemoItem,
        plain_content: impl Into<String>,
        attachments: Vec<PackedAttachment>,
    ) -> Self {
        let nav = HostedNavMeta::from_item(item);
        Self {
            format: HOSTED_MEMO_FORMAT_V3.into(),
            title: item.title.clone(),
            content: plain_content.into(),
            attachments,
            category: nav.category,
            due_date: nav.due_date,
            end_date: nav.end_date,
            done: nav.done,
            tags: nav.tags,
            priority: nav.priority,
            remind_before_days: nav.remind_before_days,
            remind_seen_for: nav.remind_seen_for,
        }
    }

    fn nav_meta(&self) -> HostedNavMeta {
        HostedNavMeta {
            category: self.category.canonical(),
            due_date: self.due_date.clone(),
            end_date: self.end_date.clone(),
            done: self.done,
            tags: self.tags.clone(),
            priority: self.priority,
            remind_before_days: self.remind_before_days,
            remind_seen_for: self.remind_seen_for.clone(),
        }
    }

    pub fn encode_ciphertext(
        key: &[u8],
        item: &MemoItem,
        plain_content: &str,
        attachments: Vec<PackedAttachment>,
    ) -> anyhow::Result<String> {
        let json = serde_json::to_string(&Self::from_item(item, plain_content, attachments))?;
        crypto::encrypt_string(key, &json)
    }

    /// 解析托管密文。
    /// - v3：含导航元数据
    /// - v1/v2：仅正文；`nav = None`
    /// - 旧版：密文即为 store 侧 content 密文
    pub fn decode_for_restore(
        key: &[u8],
        blob_ciphertext: &str,
        title_hint: &str,
    ) -> anyhow::Result<RestoredHostedMemo> {
        let plain = crypto::decrypt_string(key, blob_ciphertext)?;
        if let Ok(payload) = serde_json::from_str::<Self>(&plain) {
            if payload.format == HOSTED_MEMO_FORMAT_V1
                || payload.format == HOSTED_MEMO_FORMAT_V2
                || payload.format == HOSTED_MEMO_FORMAT_V3
            {
                let content_ct = crypto::encrypt_string(key, &payload.content)?;
                let title = if payload.title.trim().is_empty() {
                    title_from_plain(&payload.content)
                } else {
                    payload.title.clone()
                };
                let nav = if payload.format == HOSTED_MEMO_FORMAT_V3 {
                    Some(payload.nav_meta())
                } else {
                    None
                };
                return Ok(RestoredHostedMemo {
                    title,
                    content_ct,
                    attachments: payload.attachments,
                    nav,
                });
            }
        }
        // 兼容仅有 format/title/content 的旧 JSON
        #[derive(Deserialize)]
        struct LegacyPayload {
            format: String,
            title: String,
            content: String,
        }
        if let Ok(payload) = serde_json::from_str::<LegacyPayload>(&plain) {
            if payload.format == HOSTED_MEMO_FORMAT_V1 || payload.format == HOSTED_MEMO_FORMAT_V2 {
                let content_ct = crypto::encrypt_string(key, &payload.content)?;
                let title = if payload.title.trim().is_empty() {
                    title_from_plain(&payload.content)
                } else {
                    payload.title
                };
                return Ok(RestoredHostedMemo {
                    title,
                    content_ct,
                    attachments: Vec::new(),
                    nav: None,
                });
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
        Ok(RestoredHostedMemo {
            title,
            content_ct: blob_ciphertext.to_string(),
            attachments: Vec::new(),
            nav: None,
        })
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

    /// 按 backed_up_at 最旧优先删除托管备份以腾挪空间（最多一批 8 条）。
    pub fn purge_for_space(&self, need_bytes_hint: u64) -> anyhow::Result<usize> {
        let _ = need_bytes_hint;
        let mut idx = self.index.write();
        if idx.blobs.is_empty() {
            return Ok(0);
        }
        let mut candidates: Vec<(usize, String)> = idx
            .blobs
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let key = if b.backed_up_at.is_empty() {
                    b.updated_at.clone()
                } else {
                    b.backed_up_at.clone()
                };
                (i, key)
            })
            .collect();
        candidates.sort_by(|a, b| a.1.cmp(&b.1));
        let remove_n = candidates.len().min(8);
        let mut remove_idx: Vec<usize> =
            candidates.into_iter().take(remove_n).map(|(i, _)| i).collect();
        remove_idx.sort_unstable_by(|a, b| b.cmp(a));
        let mut n = 0usize;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto;
    use crate::store::{MemoCategory, MemoItem, MemoLifecycle, MemoPriority, MemoVisibility};

    fn test_key() -> Vec<u8> {
        vec![0u8; 32]
    }

    fn sample_item(
        title: &str,
        category: MemoCategory,
        due_date: &str,
    ) -> MemoItem {
        MemoItem {
            id: "m1".into(),
            title: title.into(),
            content: String::new(),
            deleted: false,
            version: 1,
            node_id: "n1".into(),
            modified_by_person_id: String::new(),
            modified_by_name: String::new(),
            visibility: MemoVisibility::Private,
            owner_fp: "fp".into(),
            modified_at: String::new(),
            lifecycle: MemoLifecycle::Permanent,
            deleted_at: String::new(),
            category,
            due_date: due_date.into(),
            end_date: String::new(),
            remind_before_days: 1,
            remind_seen_for: String::new(),
            done: false,
            tags: vec!["t".into()],
            priority: MemoPriority::High,
        }
    }

    fn assert_nav_roundtrip(category: MemoCategory, due: &str) {
        let key = test_key();
        let item = sample_item("标题", category, due);
        let ct = HostedMemoPayload::encode_ciphertext(&key, &item, "正文内容", vec![]).unwrap();
        let restored = HostedMemoPayload::decode_for_restore(&key, &ct, "").unwrap();
        let nav = restored.nav.expect("v3 should carry nav");
        assert_eq!(nav.category, category.canonical());
        assert_eq!(nav.due_date, due);
        assert_eq!(nav.priority, MemoPriority::High);
        assert_eq!(nav.tags, vec!["t".to_string()]);
        assert_eq!(nav.remind_before_days, 1);
        assert_eq!(restored.title, "标题");
        let plain = crypto::decrypt_string(&key, &restored.content_ct).unwrap();
        assert_eq!(plain, "正文内容");
    }

    #[test]
    fn v3_roundtrip_gender_private() {
        assert_nav_roundtrip(MemoCategory::GenderPrivate, "");
    }

    #[test]
    fn v3_roundtrip_todo_with_due() {
        assert_nav_roundtrip(MemoCategory::Todo, "2026-10-21 09:00");
    }

    #[test]
    fn v3_roundtrip_credentials() {
        assert_nav_roundtrip(MemoCategory::Credentials, "");
    }

    #[test]
    fn v2_decode_has_no_nav() {
        let key = test_key();
        let payload = serde_json::json!({
            "format": HOSTED_MEMO_FORMAT_V2,
            "title": "旧包",
            "content": "旧正文",
            "attachments": []
        });
        let ct = crypto::encrypt_string(&key, &payload.to_string()).unwrap();
        let restored = HostedMemoPayload::decode_for_restore(&key, &ct, "").unwrap();
        assert!(restored.nav.is_none());
        assert_eq!(restored.title, "旧包");
    }
}
