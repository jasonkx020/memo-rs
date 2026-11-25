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

/// 记录所在空间：普通栏 vs 私密空间（与局域网共享正交）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoSpace {
    #[default]
    Normal,
    Private,
}

impl MemoSpace {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "普通",
            Self::Private => "私密空间",
        }
    }
}

/// 是否局域网明文同步（仅普通空间可选；私密空间强制 Off）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LanShare {
    #[default]
    Off,
    On,
}

impl LanShare {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "不共享",
            Self::On => "局域网共享",
        }
    }

    pub fn is_on(self) -> bool {
        matches!(self, Self::On)
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
        "永久".into()
    }

    pub fn expires_at(&self) -> Option<&str> {
        None
    }

    #[allow(dead_code)]
    pub fn from_days(_days: i64) -> Self {
        Self::Permanent
    }

    pub fn is_expired(&self) -> bool {
        // 产品已取消生命周期过期；保留方法以兼容旧调用，恒为未过期。
        let _ = self;
        false
    }

    pub fn is_permanent(&self) -> bool {
        true
    }

    /// 旧 ExpiresAt 一律视为永久。
    pub fn normalize(self) -> Self {
        Self::Permanent
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoCategory {
    #[default]
    General,
    Todo,
    Credentials,
    Work,
    /// 旧「机关事务」分类（反序列化兼容；写入时折为任务 +「工作」标签）。
    Office,
    Life,
    Finance,
    /// 统一性别私密（经期关怀 + 体检健康）；新写入使用此变体。
    GenderPrivate,
    /// 旧版女性私密（反序列化兼容）。
    WomenPrivate,
    /// 旧版男性私密（反序列化兼容）。
    MalePrivate,
    Emergency,
    Inspiration,
}

impl MemoCategory {
    /// 界面可选分类（任务 / 备忘）；旧细类与 GenderPrivate 仅反序列化兼容。
    pub const ALL: &'static [MemoCategory] = &[Self::Todo, Self::Credentials];

    pub fn label(self) -> &'static str {
        match self.bucket() {
            Self::Credentials => "备忘",
            _ => "任务",
        }
    }

    /// 旧细分类对应的建议标签（界面两类之外的语义）。
    pub fn legacy_tag(self) -> Option<&'static str> {
        match self.canonical() {
            Self::Work | Self::Office => Some("工作"),
            Self::Life => Some("生活"),
            Self::Finance => Some("财务"),
            Self::Emergency => Some("应急"),
            Self::Inspiration => Some("灵感"),
            Self::General => Some("随手记"),
            Self::Todo | Self::Credentials | Self::GenderPrivate | Self::WomenPrivate
            | Self::MalePrivate => None,
        }
    }

    /// 旧性别私密分类（写入时折入私密空间 + 任务/备忘）。
    pub fn is_gender_private(self) -> bool {
        matches!(
            self,
            Self::GenderPrivate | Self::WomenPrivate | Self::MalePrivate
        )
    }

    /// 已下线分类归并（机关事务 → 工作；女/男私密 → 性别私密）。
    pub fn canonical(self) -> Self {
        match self {
            Self::Office => Self::Work,
            Self::WomenPrivate | Self::MalePrivate => Self::GenderPrivate,
            other => other,
        }
    }

    /// 界面两类桶：任务 / 备忘（旧 GenderPrivate 按待办桶展示，路由看 space）。
    pub fn bucket(self) -> Self {
        match self.canonical() {
            Self::Credentials => Self::Credentials,
            _ => Self::Todo,
        }
    }

    /// 写入前归入任务/备忘（无 due 上下文时 GenderPrivate → Todo）。
    pub fn for_store(self) -> Self {
        self.bucket()
    }

    /// 按到期日把旧 GenderPrivate 折成任务（有日期）或备忘（无日期）。
    pub fn for_store_with_due(self, due: &str) -> Self {
        if self.is_gender_private() {
            if due.trim().is_empty() {
                Self::Credentials
            } else {
                Self::Todo
            }
        } else {
            self.bucket()
        }
    }

    /// 旧细类语义写入标签（若尚无），并返回任务/备忘分类。
    pub fn fold_into_tags(self, tags: &mut Vec<String>) -> Self {
        if let Some(tag) = self.legacy_tag() {
            if !tags.iter().any(|t| t == tag) {
                tags.push(tag.to_string());
            }
        }
        self.for_store()
    }
}

/// 归一空间 / 共享 / 分类 / 兼容 visibility（写入与读迁移共用）。
pub fn normalize_memo_fields(
    category: MemoCategory,
    space: MemoSpace,
    lan_share: LanShare,
    due: &str,
    tags: &[String],
) -> (MemoCategory, MemoSpace, LanShare, MemoVisibility, Vec<String>) {
    let mut tags = tags.to_vec();
    let mut space = space;
    let mut lan = lan_share;

    if category.is_gender_private() {
        space = MemoSpace::Private;
        lan = LanShare::Off;
        if !tags.iter().any(|t| t == "私密") {
            tags.push("私密".into());
        }
        let category = category.for_store_with_due(due);
        return (category, space, lan, MemoVisibility::Private, tags);
    }

    let category = category.fold_into_tags(&mut tags);
    if space == MemoSpace::Private {
        lan = LanShare::Off;
    }
    let visibility = match lan {
        LanShare::On => MemoVisibility::Public,
        LanShare::Off => MemoVisibility::Private,
    };
    (category, space, lan, visibility, tags)
}

/// 旧数据：无 space/lan 字段或仍为 GenderPrivate 分类时，读路径补齐。
pub fn migrate_legacy_item(item: &mut MemoItem) {
    if item.category.is_gender_private() {
        let (cat, space, lan, vis, tags) = normalize_memo_fields(
            item.category,
            MemoSpace::Private,
            LanShare::Off,
            &item.due_date,
            &item.tags,
        );
        item.category = cat;
        item.space = space;
        item.lan_share = lan;
        item.visibility = vis;
        item.tags = tags;
        return;
    }
    // 旧 Public → 普通空间 + 局域网共享
    if item.visibility == MemoVisibility::Public {
        item.space = MemoSpace::Normal;
        item.lan_share = LanShare::On;
    }
    let mut tags = std::mem::take(&mut item.tags);
    item.category = item.category.fold_into_tags(&mut tags);
    item.tags = tags;
    // 与 lan 对齐 visibility
    item.visibility = match item.lan_share {
        LanShare::On => MemoVisibility::Public,
        LanShare::Off => MemoVisibility::Private,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoPriority {
    Low,
    #[default]
    Normal,
    High,
}

impl MemoPriority {
    pub const ALL: &'static [MemoPriority] = &[Self::Low, Self::Normal, Self::High];

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "低",
            Self::Normal => "普通",
            Self::High => "高",
        }
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
    /// 普通栏 / 私密空间（与 visibility/局域网共享拆分）。
    #[serde(default)]
    pub space: MemoSpace,
    /// 是否局域网明文同步；私密空间强制 Off。
    #[serde(default)]
    pub lan_share: LanShare,
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
    #[serde(default)]
    pub category: MemoCategory,
    /// 到期时间本地 `YYYY-MM-DD HH:MM`（兼容旧 `YYYY-MM-DD`）；空=无到期（永久）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub due_date: String,
    /// 跨天结束日；空=单日
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub end_date: String,
    /// 提前提醒天数；仅 due 非空时有效；0=到期当日该时刻
    #[serde(default)]
    pub remind_before_days: u32,
    /// 已弹出提醒所对应的 due_date 全文
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub remind_seen_for: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub priority: MemoPriority,
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
        space: MemoSpace,
        lan_share: LanShare,
        owner_fp: &str,
        lifecycle: MemoLifecycle,
        category: MemoCategory,
        due_date: &str,
        end_date: &str,
        done: bool,
        tags: &[String],
        priority: MemoPriority,
        remind_before_days: u32,
        remind_seen_for: &str,
    ) -> anyhow::Result<MemoItem> {
        let mut items = self.items.write();
        let prev = items.get(id).cloned();
        let ver = self.tick(0);
        let due = due_date.trim().to_string();
        let end = crate::due::normalize_end_date(&due, end_date)
            .map_err(anyhow::Error::msg)?;
        let (remind_before_days, remind_seen_for) = if due.is_empty() {
            (0u32, String::new())
        } else {
            let due_changed = prev
                .as_ref()
                .map(|p| p.due_date.trim() != due.as_str())
                .unwrap_or(true);
            let seen = if due_changed {
                // 到期变更后重新提醒；ack 时调用方传入 due 全文
                if remind_seen_for == due.as_str() {
                    remind_seen_for.to_string()
                } else {
                    String::new()
                }
            } else if remind_seen_for.is_empty() {
                prev.as_ref()
                    .map(|p| p.remind_seen_for.clone())
                    .unwrap_or_default()
            } else {
                remind_seen_for.to_string()
            };
            (remind_before_days.min(365), seen)
        };
        let (category, space, lan_share, visibility, tags) =
            normalize_memo_fields(category, space, lan_share, &due, tags);
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
            space,
            lan_share,
            owner_fp: owner_fp.to_string(),
            modified_at: chrono::Local::now().to_rfc3339(),
            lifecycle,
            deleted_at: String::new(),
            category,
            due_date: due,
            end_date: end,
            remind_before_days,
            remind_seen_for,
            done,
            tags,
            priority,
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
