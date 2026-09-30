use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::audit::{AuditLog, EventData, EventType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoVisibility {
    #[default]
    Private,
    Public,
}

impl MemoVisibility {
    pub fn label(self) -> &'static str {
        match self {
            MemoVisibility::Private => "私密",
            MemoVisibility::Public => "公开",
        }
    }
}

/// 备忘生命周期：永久或到期时间（RFC3339）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoLifecycle {
    Permanent,
    ExpiresAt { at: String },
}

impl Default for MemoLifecycle {
    fn default() -> Self {
        Self::Permanent
    }
}

impl MemoLifecycle {
    pub fn label(&self) -> String {
        match self {
            Self::Permanent => "永久".into(),
            Self::ExpiresAt { at } => format!("到期 {at}"),
        }
    }

    pub fn expires_at(&self) -> Option<&str> {
        match self {
            Self::Permanent => None,
            Self::ExpiresAt { at } => Some(at.as_str()),
        }
    }

    pub fn from_days(days: i64) -> Self {
        let at = (chrono::Local::now() + chrono::Duration::try_days(days).unwrap_or_default())
            .to_rfc3339();
        Self::ExpiresAt { at }
    }

    pub fn is_expired(&self) -> bool {
        match self {
            Self::Permanent => false,
            Self::ExpiresAt { at } => chrono::DateTime::parse_from_rfc3339(at)
                .map(|t| t < chrono::Local::now())
                .unwrap_or(false),
        }
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoItem {
    pub id: String,
    pub title: String,
    pub content: String,
    pub deleted: bool,
    pub version: u64,
    pub node_id: String,
    /// 最后修改人员 id（随 LWW 同步）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub modified_by_person_id: String,
    /// 最后修改人员姓名快照
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub modified_by_name: String,
    #[serde(default)]
    pub visibility: MemoVisibility,
    /// 所属身份公钥指纹
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner_fp: String,
    /// 内容修改时间（RFC3339）；用于增量备份比对
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub modified_at: String,
    #[serde(default)]
    pub lifecycle: MemoLifecycle,
    /// 软删时间（RFC3339）；回收站宽限期据此计算
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub deleted_at: String,
}

fn actor_opts(person_id: &str, name: &str) -> (Option<String>, Option<String>) {
    let pid = if person_id.is_empty() {
        None
    } else {
        Some(person_id.to_string())
    };
    let pname = if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    };
    (pid, pname)
}

fn actor_from_memo(item: &MemoItem) -> (Option<String>, Option<String>) {
    actor_opts(&item.modified_by_person_id, &item.modified_by_name)
}

/// 同步时本机版本获胜、远端被拒的提示。
#[derive(Debug, Clone)]
pub struct ConflictNotice {
    pub memo_id: String,
    pub title: String,
    pub local_version: u64,
    pub remote_version: u64,
    pub remote_node: String,
    pub source: String,
}

pub trait Broadcaster: Send + Sync {
    fn broadcast_memo(&self, item: &MemoItem);
    fn broadcast_person(&self, item: &crate::person::Person);
    fn broadcast_task(&self, item: &crate::task::TaskItem);
    fn broadcast_hosted(&self, blob: &crate::hosted::HostedBlob) {
        let _ = blob;
    }
    /// 从主机抹除指定私有备份
    fn broadcast_hosted_purge(&self, owner_fp: &str, memo_id: &str, version: u64) {
        let _ = (owner_fp, memo_id, version);
    }
}

pub struct MemoStore {
    items: RwLock<HashMap<String, MemoItem>>,
    clock: RwLock<u64>,
    node_id: String,
    audit: Arc<AuditLog>,
    broadcaster: RwLock<Option<Arc<dyn Broadcaster>>>,
    conflicts: RwLock<Vec<ConflictNotice>>,
}

impl MemoStore {
    pub fn new(node_id: String, audit: Arc<AuditLog>) -> Self {
        Self {
            items: RwLock::new(HashMap::new()),
            clock: RwLock::new(0),
            node_id,
            audit,
            broadcaster: RwLock::new(None),
            conflicts: RwLock::new(Vec::new()),
        }
    }

    pub fn set_broadcaster(&self, bc: Arc<dyn Broadcaster>) {
        *self.broadcaster.write() = Some(bc);
    }

    pub fn broadcaster_opt(&self) -> Option<Arc<dyn Broadcaster>> {
        self.broadcaster.read().clone()
    }

    pub fn take_conflicts(&self) -> Vec<ConflictNotice> {
        std::mem::take(&mut *self.conflicts.write())
    }

    fn tick(&self, hint: u64) -> u64 {
        let mut c = self.clock.write();
        if hint > *c {
            *c = hint;
        }
        *c += 1;
        *c
    }

    pub fn put(
        &self,
        id: &str,
        title: &str,
        content: &str,
        actor_person_id: &str,
        actor_name: &str,
        visibility: MemoVisibility,
        owner_fp: &str,
        lifecycle: MemoLifecycle,
    ) -> anyhow::Result<MemoItem> {
        let mut items = self.items.write();
        let prev = items.get(id).cloned();
        let ver = self.tick(0);
        let item = MemoItem {
            id: id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            deleted: false,
            version: ver,
            node_id: self.node_id.clone(),
            modified_by_person_id: actor_person_id.to_string(),
            modified_by_name: actor_name.to_string(),
            visibility,
            owner_fp: owner_fp.to_string(),
            modified_at: chrono::Local::now().to_rfc3339(),
            lifecycle,
            deleted_at: String::new(),
        };
        items.insert(id.to_string(), item.clone());
        drop(items);

        let ev = if prev.is_some() {
            EventType::Modify
        } else {
            EventType::Create
        };
        let (aid, aname) = actor_opts(actor_person_id, actor_name);
        let data = EventData {
            event_type: ev,
            entity: "memo".into(),
            memo_id: id.to_string(),
            before: prev.map(|p| serde_json::to_value(p).unwrap()),
            after: Some(serde_json::to_value(&item).unwrap()),
            node_id: self.node_id.clone(),
            source: String::new(),
            actor_person_id: aid,
            actor_name: aname,
        };
        self.audit
            .append_value(serde_json::to_value(data)?)?;
        if let Some(bc) = self.broadcaster.read().clone() {
            bc.broadcast_memo(&item);
        }
        Ok(item)
    }

    pub fn delete(
        &self,
        id: &str,
        actor_person_id: &str,
        actor_name: &str,
    ) -> anyhow::Result<MemoItem> {
        let mut items = self.items.write();
        let prev = items
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("备忘不存在: {id}"))?;
        let ver = self.tick(0);
        let mut item = prev.clone();
        item.deleted = true;
        item.deleted_at = chrono::Local::now().to_rfc3339();
        item.version = ver;
        item.node_id = self.node_id.clone();
        item.modified_by_person_id = actor_person_id.to_string();
        item.modified_by_name = actor_name.to_string();
        item.modified_at = chrono::Local::now().to_rfc3339();
        items.insert(id.to_string(), item.clone());
        drop(items);

        let (aid, aname) = actor_opts(actor_person_id, actor_name);
        let data = EventData {
            event_type: EventType::Delete,
            entity: "memo".into(),
            memo_id: id.to_string(),
            before: Some(serde_json::to_value(&prev)?),
            after: Some(serde_json::to_value(&item)?),
            node_id: self.node_id.clone(),
            source: String::new(),
            actor_person_id: aid,
            actor_name: aname,
        };
        self.audit
            .append_value(serde_json::to_value(data)?)?;
        if let Some(bc) = self.broadcaster.read().clone() {
            bc.broadcast_memo(&item);
        }
        Ok(item)
    }

    /// 从回收站恢复
    pub fn undelete(
        &self,
        id: &str,
        actor_person_id: &str,
        actor_name: &str,
    ) -> anyhow::Result<MemoItem> {
        let mut items = self.items.write();
        let prev = items
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("备忘不存在: {id}"))?;
        if !prev.deleted {
            anyhow::bail!("备忘未删除");
        }
        let ver = self.tick(0);
        let mut item = prev.clone();
        item.deleted = false;
        item.deleted_at.clear();
        item.version = ver;
        item.node_id = self.node_id.clone();
        item.modified_by_person_id = actor_person_id.to_string();
        item.modified_by_name = actor_name.to_string();
        item.modified_at = chrono::Local::now().to_rfc3339();
        items.insert(id.to_string(), item.clone());
        drop(items);

        let (aid, aname) = actor_opts(actor_person_id, actor_name);
        let data = EventData {
            event_type: EventType::Modify,
            entity: "memo".into(),
            memo_id: id.to_string(),
            before: Some(serde_json::to_value(&prev)?),
            after: Some(serde_json::to_value(&item)?),
            node_id: self.node_id.clone(),
            source: "undelete".into(),
            actor_person_id: aid,
            actor_name: aname,
        };
        self.audit
            .append_value(serde_json::to_value(data)?)?;
        if let Some(bc) = self.broadcaster.read().clone() {
            bc.broadcast_memo(&item);
        }
        Ok(item)
    }

    /// 彻底清除：清空正文，保留已删 tombstone（更高 version）
    pub fn purge(
        &self,
        id: &str,
        actor_person_id: &str,
        actor_name: &str,
    ) -> anyhow::Result<MemoItem> {
        let mut items = self.items.write();
        let prev = items
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("备忘不存在: {id}"))?;
        let ver = self.tick(0);
        let mut item = prev.clone();
        item.deleted = true;
        if item.deleted_at.is_empty() {
            item.deleted_at = chrono::Local::now().to_rfc3339();
        }
        item.content.clear();
        item.version = ver;
        item.node_id = self.node_id.clone();
        item.modified_by_person_id = actor_person_id.to_string();
        item.modified_by_name = actor_name.to_string();
        item.modified_at = chrono::Local::now().to_rfc3339();
        items.insert(id.to_string(), item.clone());
        drop(items);

        let (aid, aname) = actor_opts(actor_person_id, actor_name);
        let data = EventData {
            event_type: EventType::Delete,
            entity: "memo".into(),
            memo_id: id.to_string(),
            before: Some(serde_json::to_value(&prev)?),
            after: Some(serde_json::to_value(&item)?),
            node_id: self.node_id.clone(),
            source: "purge".into(),
            actor_person_id: aid,
            actor_name: aname,
        };
        self.audit
            .append_value(serde_json::to_value(data)?)?;
        if let Some(bc) = self.broadcaster.read().clone() {
            bc.broadcast_memo(&item);
        }
        Ok(item)
    }

    pub fn get_deleted(&self) -> Vec<MemoItem> {
        self.items
            .read()
            .values()
            .filter(|i| i.deleted && !i.content.is_empty())
            .cloned()
            .collect()
    }

    /// LWW 合并；返回是否更新本地。
    pub fn merge(&self, remote: MemoItem, source: &str) -> anyhow::Result<bool> {
        let mut items = self.items.write();
        let local = items.get(&remote.id).cloned();
        let win = match &local {
            None => true,
            Some(l) if remote.version > l.version => true,
            Some(l) if remote.version == l.version && remote.node_id > l.node_id => true,
            _ => false,
        };
        if !win {
            let mut c = self.clock.write();
            if remote.version > *c {
                *c = remote.version;
            }
            if let Some(l) = &local {
                self.conflicts.write().push(ConflictNotice {
                    memo_id: remote.id.clone(),
                    title: l.title.clone(),
                    local_version: l.version,
                    remote_version: remote.version,
                    remote_node: remote.node_id.clone(),
                    source: source.to_string(),
                });
            }
            return Ok(false);
        }

        let ev = match &local {
            None if remote.deleted => EventType::Delete,
            None => EventType::Create,
            Some(l) if remote.deleted && !l.deleted => EventType::Delete,
            _ => EventType::Modify,
        };
        items.insert(remote.id.clone(), remote.clone());
        {
            let mut c = self.clock.write();
            if remote.version > *c {
                *c = remote.version;
            }
        }
        drop(items);

        let (aid, aname) = actor_from_memo(&remote);
        let data = EventData {
            event_type: ev,
            entity: "memo".into(),
            memo_id: remote.id.clone(),
            before: local.map(|l| serde_json::to_value(l).unwrap()),
            after: Some(serde_json::to_value(&remote).unwrap()),
            node_id: self.node_id.clone(),
            source: source.to_string(),
            actor_person_id: aid,
            actor_name: aname,
        };
        self.audit
            .append_value(serde_json::to_value(data)?)?;
        Ok(true)
    }

    /// 托管恢复专用：绕过 LWW，按备份内容覆盖本机条目（含同版本修正占位标题）。
    pub fn force_put_restored(&self, item: MemoItem) -> anyhow::Result<()> {
        let mut items = self.items.write();
        let before = items.get(&item.id).cloned();
        items.insert(item.id.clone(), item.clone());
        {
            let mut c = self.clock.write();
            if item.version > *c {
                *c = item.version;
            }
        }
        drop(items);
        let (aid, aname) = actor_from_memo(&item);
        let data = EventData {
            event_type: if before.is_some() {
                EventType::Modify
            } else {
                EventType::Create
            },
            entity: "memo".into(),
            memo_id: item.id.clone(),
            before: before.map(|l| serde_json::to_value(l).unwrap()),
            after: Some(serde_json::to_value(&item).unwrap()),
            node_id: self.node_id.clone(),
            source: "hosted-restore".into(),
            actor_person_id: aid,
            actor_name: aname,
        };
        self.audit.append_value(serde_json::to_value(data)?)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<MemoItem> {
        self.items.read().get(id).cloned()
    }

    pub fn get_visible(&self) -> Vec<MemoItem> {
        self.items
            .read()
            .values()
            .filter(|i| !i.deleted)
            .cloned()
            .collect()
    }

    pub fn all(&self) -> Vec<MemoItem> {
        self.items.read().values().cloned().collect()
    }

    pub fn rebuild_from_audit(&self) -> anyhow::Result<()> {
        let entries = self.audit.read_all()?;
        let mut items = HashMap::new();
        let mut clock = 0u64;
        for e in entries {
            let ed: EventData = serde_json::from_value(e.data)?;
            if ed.entity != "memo" {
                continue;
            }
            if let Some(after) = ed.after {
                let item: MemoItem = match serde_json::from_value(after) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if item.version > clock {
                    clock = item.version;
                }
                items.insert(item.id.clone(), item);
            }
        }
        *self.items.write() = items;
        *self.clock.write() = clock;
        Ok(())
    }
}
