//! 异地托管：按身份公钥指纹存放密文，托管方不可解。

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const HOSTED_FILE: &str = "hosted_index.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedBlob {
    pub owner_fp: String,
    pub memo_id: String,
    pub version: u64,
    pub ciphertext: String,
    pub title_hint: String,
    #[serde(default)]
    pub visibility: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub source_node: String,
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
    pub fn upsert_blob(&self, blob: HostedBlob) -> anyhow::Result<bool> {
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

    pub fn all_blobs(&self) -> Vec<HostedBlob> {
        self.index.read().blobs.clone()
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
