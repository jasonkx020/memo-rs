//! 主机侧对端 ACL：按 node_id 控制私有备份与公开同步。

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::NodeRole;

const ACL_FILE: &str = "peer_acl.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerAclEntry {
    pub node_id: String,
    #[serde(default)]
    pub alias: String,
    #[serde(default)]
    pub key_fingerprint: String,
    #[serde(default)]
    pub peer_role: NodeRole,
    pub allow_private_backup: bool,
    pub allow_public_sync: bool,
    #[serde(default)]
    pub last_seen_addr: String,
    #[serde(default)]
    pub updated_at: String,
}

impl PeerAclEntry {
    pub fn defaults_for(role: NodeRole) -> (bool, bool) {
        match role {
            NodeRole::Master => (true, true),
            NodeRole::Slave => (true, false),
        }
    }

    pub fn new_default(
        node_id: impl Into<String>,
        peer_role: NodeRole,
        alias: impl Into<String>,
        key_fingerprint: impl Into<String>,
        addr: impl Into<String>,
    ) -> Self {
        let (allow_private_backup, allow_public_sync) = Self::defaults_for(peer_role);
        Self {
            node_id: node_id.into(),
            alias: alias.into(),
            key_fingerprint: key_fingerprint.into(),
            peer_role,
            allow_private_backup,
            allow_public_sync,
            last_seen_addr: addr.into(),
            updated_at: chrono::Local::now().to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PeerAclFile {
    entries: Vec<PeerAclEntry>,
}

pub struct PeerAclStore {
    path: PathBuf,
    inner: RwLock<PeerAclFile>,
}

impl PeerAclStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join(ACL_FILE);
        let inner = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            PeerAclFile::default()
        };
        Ok(Self {
            path,
            inner: RwLock::new(inner),
        })
    }

    fn persist(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = self.inner.read().clone();
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(&data)?)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<PeerAclEntry> {
        let mut v = self.inner.read().entries.clone();
        v.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        v
    }

    pub fn get(&self, node_id: &str) -> Option<PeerAclEntry> {
        self.inner
            .read()
            .entries
            .iter()
            .find(|e| e.node_id == node_id)
            .cloned()
    }

    /// 登记或刷新元数据；已有条目保留用户改过的权限开关。
    pub fn upsert_seen(
        &self,
        node_id: &str,
        peer_role: NodeRole,
        alias: &str,
        key_fingerprint: &str,
        addr: &str,
    ) -> anyhow::Result<PeerAclEntry> {
        let mut g = self.inner.write();
        if let Some(e) = g.entries.iter_mut().find(|e| e.node_id == node_id) {
            e.peer_role = peer_role;
            if !alias.is_empty() {
                e.alias = alias.to_string();
            }
            if !key_fingerprint.is_empty() {
                e.key_fingerprint = key_fingerprint.to_string();
            }
            if !addr.is_empty() {
                e.last_seen_addr = addr.to_string();
            }
            e.updated_at = chrono::Local::now().to_rfc3339();
            let out = e.clone();
            drop(g);
            let _ = self.persist();
            return Ok(out);
        }
        let entry = PeerAclEntry::new_default(node_id, peer_role, alias, key_fingerprint, addr);
        g.entries.push(entry.clone());
        drop(g);
        self.persist()?;
        Ok(entry)
    }

    pub fn set_flags(
        &self,
        node_id: &str,
        allow_private_backup: bool,
        allow_public_sync: bool,
    ) -> anyhow::Result<Option<PeerAclEntry>> {
        let mut g = self.inner.write();
        let Some(e) = g.entries.iter_mut().find(|e| e.node_id == node_id) else {
            return Ok(None);
        };
        e.allow_private_backup = allow_private_backup;
        e.allow_public_sync = allow_public_sync;
        e.updated_at = chrono::Local::now().to_rfc3339();
        let out = e.clone();
        drop(g);
        self.persist()?;
        Ok(Some(out))
    }

    pub fn allow_private_backup(&self, node_id: &str) -> bool {
        self.get(node_id)
            .map(|e| e.allow_private_backup)
            .unwrap_or(true)
    }

    pub fn allow_public_sync(&self, node_id: &str) -> bool {
        self.get(node_id)
            .map(|e| e.allow_public_sync)
            .unwrap_or(false)
    }
}
