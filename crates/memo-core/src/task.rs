//! 日程任务（独立于备忘录）。

use chrono::{Duration, NaiveDate, NaiveDateTime, Timelike};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::audit::{AuditLog, EventData, EventType};
use crate::enc_store;
use crate::store::{Broadcaster, ConflictNotice};

const TASK_FILE: &str = "tasks.json.enc";

fn default_start_hour() -> f32 {
    9.0
}

fn parse_ymd(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn datetime_at(date: &str, hour: f32) -> Option<NaiveDateTime> {
    let d = parse_ymd(date)?;
    let base = d.and_hms_opt(0, 0, 0)?;
    let secs = (hour.max(0.0) * 3600.0).round() as i64;
    Some(base + Duration::try_seconds(secs).unwrap_or_default())
}

/// 由开始/结束日期时刻计算工时（小时），至少 0.25。
pub fn duration_hours(
    start_date: &str,
    start_hour: f32,
    end_date: &str,
    end_hour: f32,
) -> anyhow::Result<f32> {
    let start = datetime_at(start_date, start_hour)
        .ok_or_else(|| anyhow::anyhow!("开始日期无效"))?;
    let end = datetime_at(end_date, end_hour)
        .ok_or_else(|| anyhow::anyhow!("结束日期无效"))?;
    let secs = (end - start).num_seconds();
    if secs <= 0 {
        anyhow::bail!("结束时间必须晚于开始时间");
    }
    Ok((secs as f32 / 3600.0).max(0.25))
}

/// 开始时刻 + 工时 → 结束日期与时刻。
pub fn end_from_duration(start_date: &str, start_hour: f32, hours: f32) -> (String, f32) {
    let Some(start) = datetime_at(start_date, start_hour) else {
        return (start_date.to_string(), start_hour + hours.max(0.25));
    };
    let end = start
        + Duration::try_seconds((hours.max(0.25) * 3600.0).round() as i64).unwrap_or_default();
    let end_date = end.date().format("%Y-%m-%d").to_string();
    let end_hour = end.time().num_seconds_from_midnight() as f32 / 3600.0;
    (end_date, end_hour)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    NotStarted,
    InProgress,
    Paused,
    Blocked,
    Cancelled,
    Done,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 6] = [
        TaskStatus::NotStarted,
        TaskStatus::InProgress,
        TaskStatus::Paused,
        TaskStatus::Blocked,
        TaskStatus::Cancelled,
        TaskStatus::Done,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::NotStarted => "未开始",
            TaskStatus::InProgress => "进行中",
            TaskStatus::Paused => "暂停",
            TaskStatus::Blocked => "阻塞",
            TaskStatus::Cancelled => "已取消",
            TaskStatus::Done => "已结束",
        }
    }

    /// Short label for narrow calendar cells.
    pub fn short_label(self) -> &'static str {
        match self {
            TaskStatus::NotStarted => "未",
            TaskStatus::InProgress => "进",
            TaskStatus::Paused => "停",
            TaskStatus::Blocked => "阻",
            TaskStatus::Cancelled => "消",
            TaskStatus::Done => "完",
        }
    }

    pub fn is_closed(self) -> bool {
        matches!(self, TaskStatus::Cancelled | TaskStatus::Done)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: String,
    pub title: String,
    /// 工作计划正文（密文，由 Service 加解密）
    pub plan: String,
    /// 开始日期 YYYY-MM-DD
    pub date: String,
    #[serde(default = "default_start_hour")]
    pub start_hour: f32,
    /// 工时（小时）；与结束日期时刻保持一致
    pub hours: f32,
    /// 结束日期 YYYY-MM-DD；空则兼容旧数据（按 date + start + hours 推导）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub end_date: String,
    /// 结束时刻（小数小时）；仅当 end_date 非空时有效
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_hour: Option<f32>,
    pub assignee_id: String,
    #[serde(default)]
    pub status: TaskStatus,
    pub deleted: bool,
    pub version: u64,
    pub node_id: String,
    /// 最后修改人员 id（随 LWW 同步）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub modified_by_person_id: String,
    /// 最后修改人员姓名快照
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub modified_by_name: String,
}

impl TaskItem {
    pub fn resolved_end(&self) -> (String, f32) {
        if !self.end_date.is_empty() {
            if let Some(h) = self.end_hour {
                return (self.end_date.clone(), h);
            }
        }
        end_from_duration(&self.date, self.start_hour, self.hours)
    }

    pub fn spans_date(&self, ymd: &str) -> bool {
        let (end_d, _) = self.resolved_end();
        ymd >= self.date.as_str() && ymd <= end_d.as_str()
    }
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

fn actor_from_task(item: &TaskItem) -> (Option<String>, Option<String>) {
    actor_opts(&item.modified_by_person_id, &item.modified_by_name)
}

#[derive(Debug, Clone)]
pub struct TaskView {
    pub id: String,
    pub title: String,
    pub plan: String,
    pub date: String,
    pub start_hour: f32,
    pub hours: f32,
    pub end_date: String,
    pub end_hour: f32,
    pub assignee_id: String,
    pub status: TaskStatus,
    pub deleted: bool,
    pub version: u64,
    pub node_id: String,
}

impl TaskView {
    pub fn spans_date(&self, ymd: &str) -> bool {
        ymd >= self.date.as_str() && ymd <= self.end_date.as_str()
    }
}

pub struct TaskStore {
    items: RwLock<HashMap<String, TaskItem>>,
    clock: RwLock<u64>,
    node_id: String,
    data_dir: PathBuf,
    key: parking_lot::Mutex<Vec<u8>>,
    audit: Arc<AuditLog>,
    broadcaster: RwLock<Option<Arc<dyn Broadcaster>>>,
    conflicts: RwLock<Vec<ConflictNotice>>,
}

impl TaskStore {
    pub fn open(
        node_id: String,
        data_dir: &Path,
        key: &[u8],
        audit: Arc<AuditLog>,
    ) -> anyhow::Result<Self> {
        let loaded = enc_store::load_vec::<TaskItem>(data_dir, TASK_FILE, key)?;
        let mut items = HashMap::new();
        let mut clock = 0u64;
        for t in loaded {
            if t.version > clock {
                clock = t.version;
            }
            items.insert(t.id.clone(), t);
        }
        Ok(Self {
            items: RwLock::new(items),
            clock: RwLock::new(clock),
            node_id,
            data_dir: data_dir.to_path_buf(),
            key: parking_lot::Mutex::new(key.to_vec()),
            audit,
            broadcaster: RwLock::new(None),
            conflicts: RwLock::new(Vec::new()),
        })
    }

    pub fn set_broadcaster(&self, bc: Arc<dyn Broadcaster>) {
        *self.broadcaster.write() = Some(bc);
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

    fn persist(&self) -> anyhow::Result<()> {
        let items: Vec<_> = self.items.read().values().cloned().collect();
        let key = self.key.lock();
        enc_store::save_vec(&self.data_dir, TASK_FILE, &key, &items)
    }

    fn audit_write(
        &self,
        ev: EventType,
        id: &str,
        before: Option<&TaskItem>,
        after: Option<&TaskItem>,
        source: &str,
        actor_person_id: Option<String>,
        actor_name: Option<String>,
    ) -> anyhow::Result<()> {
        let data = EventData {
            event_type: ev,
            entity: "task".into(),
            memo_id: id.to_string(),
            before: before.map(|p| serde_json::to_value(p).unwrap()),
            after: after.map(|p| serde_json::to_value(p).unwrap()),
            node_id: self.node_id.clone(),
            source: source.to_string(),
            actor_person_id,
            actor_name,
        };
        self.audit.append_value(serde_json::to_value(data)?)
    }

    pub fn put(
        &self,
        id: &str,
        title: &str,
        plan_cipher: &str,
        date: &str,
        start_hour: f32,
        end_date: &str,
        end_hour: f32,
        assignee_id: &str,
        status: TaskStatus,
        actor_person_id: &str,
        actor_name: &str,
    ) -> anyhow::Result<TaskItem> {
        let hours = duration_hours(date, start_hour, end_date, end_hour)?;
        let mut items = self.items.write();
        let prev = items.get(id).cloned();
        let ver = self.tick(0);
        let item = TaskItem {
            id: id.to_string(),
            title: title.to_string(),
            plan: plan_cipher.to_string(),
            date: date.to_string(),
            start_hour,
            hours,
            end_date: end_date.to_string(),
            end_hour: Some(end_hour),
            assignee_id: assignee_id.to_string(),
            status,
            deleted: false,
            version: ver,
            node_id: self.node_id.clone(),
            modified_by_person_id: actor_person_id.to_string(),
            modified_by_name: actor_name.to_string(),
        };
        items.insert(id.to_string(), item.clone());
        drop(items);
        let ev = if prev.is_some() {
            EventType::Modify
        } else {
            EventType::Create
        };
        let (aid, aname) = actor_opts(actor_person_id, actor_name);
        self.audit_write(ev, id, prev.as_ref(), Some(&item), "", aid, aname)?;
        self.persist()?;
        if let Some(bc) = self.broadcaster.read().clone() {
            bc.broadcast_task(&item);
        }
        Ok(item)
    }

    pub fn delete(
        &self,
        id: &str,
        actor_person_id: &str,
        actor_name: &str,
    ) -> anyhow::Result<TaskItem> {
        let mut items = self.items.write();
        let prev = items
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {id}"))?;
        let ver = self.tick(0);
        let mut item = prev.clone();
        item.deleted = true;
        item.version = ver;
        item.node_id = self.node_id.clone();
        item.modified_by_person_id = actor_person_id.to_string();
        item.modified_by_name = actor_name.to_string();
        items.insert(id.to_string(), item.clone());
        drop(items);
        let (aid, aname) = actor_opts(actor_person_id, actor_name);
        self.audit_write(EventType::Delete, id, Some(&prev), Some(&item), "", aid, aname)?;
        self.persist()?;
        if let Some(bc) = self.broadcaster.read().clone() {
            bc.broadcast_task(&item);
        }
        Ok(item)
    }

    pub fn merge(&self, remote: TaskItem, source: &str) -> anyhow::Result<bool> {
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
        let (aid, aname) = actor_from_task(&remote);
        self.audit_write(ev, &remote.id, local.as_ref(), Some(&remote), source, aid, aname)?;
        self.persist()?;
        Ok(true)
    }

    pub fn get(&self, id: &str) -> Option<TaskItem> {
        self.items.read().get(id).cloned()
    }

    pub fn get_visible(&self) -> Vec<TaskItem> {
        self.items
            .read()
            .values()
            .filter(|i| !i.deleted)
            .cloned()
            .collect()
    }

    pub fn all(&self) -> Vec<TaskItem> {
        self.items.read().values().cloned().collect()
    }
}
