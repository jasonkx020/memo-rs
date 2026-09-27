use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::audit::{AuditLog, EventData, EventType};
use crate::backup;
use crate::config::Config;
use crate::crypto::{self, derive_key, keys_equal, resolve_salt};
use crate::export::export_txt;
use crate::person::{Person, PersonStore, PersonView};
use crate::store::{Broadcaster, ConflictNotice, MemoItem, MemoStore};
use crate::task::{TaskItem, TaskStatus, TaskStore, TaskView};
use crate::verifier;

#[derive(Debug, Clone)]
pub struct MemoView {
    pub id: String,
    pub title: String,
    pub content: String,
    pub deleted: bool,
    pub version: u64,
    pub node_id: String,
}

#[derive(Debug, Clone)]
pub struct HistorySnapshot {
    pub title: String,
    pub content: String,
    pub version: u64,
    pub node_id: String,
    pub deleted: bool,
}

#[derive(Debug, Clone)]
pub struct HistoryEvent {
    pub seq: u64,
    pub time: String,
    pub event_type: String,
    /// 操作人员 id（可能为空）
    pub actor_person_id: String,
    /// 操作人员姓名快照
    pub actor_name: String,
    /// Author node of this version (after.node_id).
    pub author_node: String,
    /// Node that recorded this audit entry.
    pub record_node: String,
    /// Sync source peer; empty for local ops.
    pub source: String,
    pub before: Option<HistorySnapshot>,
    pub after: Option<HistorySnapshot>,
}

pub struct MemoService {
    cfg: Config,
    key: Zeroizing<Vec<u8>>,
    store: Arc<MemoStore>,
    person_store: Arc<PersonStore>,
    task_store: Arc<TaskStore>,
    audit: Arc<AuditLog>,
    current_person_id: Mutex<Option<String>>,
    subs: Mutex<Vec<Box<dyn Fn() + Send + Sync>>>,
}

impl MemoService {
    pub fn new(
        cfg: Config,
        key: Zeroizing<Vec<u8>>,
        store: Arc<MemoStore>,
        person_store: Arc<PersonStore>,
        task_store: Arc<TaskStore>,
        audit: Arc<AuditLog>,
    ) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            key,
            store,
            person_store,
            task_store,
            audit,
            current_person_id: Mutex::new(None),
            subs: Mutex::new(Vec::new()),
        })
    }

    pub fn store(&self) -> Arc<MemoStore> {
        self.store.clone()
    }

    pub fn person_store(&self) -> Arc<PersonStore> {
        self.person_store.clone()
    }

    pub fn task_store(&self) -> Arc<TaskStore> {
        self.task_store.clone()
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn set_broadcaster(&self, bc: Arc<dyn Broadcaster>) {
        self.store.set_broadcaster(bc.clone());
        self.person_store.set_broadcaster(bc.clone());
        self.task_store.set_broadcaster(bc);
    }

    pub fn subscribe(&self, f: Box<dyn Fn() + Send + Sync>) {
        self.subs.lock().push(f);
    }

    fn fire(&self) {
        for f in self.subs.lock().iter() {
            f();
        }
    }

    pub fn current_person_id(&self) -> Option<String> {
        self.current_person_id.lock().clone()
    }

    pub fn set_current_person(&self, id: Option<String>) {
        *self.current_person_id.lock() = id;
    }

    /// 当前会话操作人 (id, name)；未登录则空串。
    fn current_actor(&self) -> (String, String) {
        match self.current_person_id() {
            Some(id) => {
                let name = self.person_name(&id);
                (id, name)
            }
            None => (String::new(), String::new()),
        }
    }

    pub fn add(&self, title: &str, content: &str) -> anyhow::Result<String> {
        let ct = crypto::encrypt_string(&self.key, content)?;
        let id = Uuid::new_v4().to_string().replace('-', "");
        let (aid, aname) = self.current_actor();
        self.store.put(&id, title, &ct, &aid, &aname)?;
        self.fire();
        Ok(id)
    }

    pub fn list(&self) -> Vec<MemoView> {
        self.store
            .get_visible()
            .into_iter()
            .map(|it| {
                let plain = crypto::decrypt_string(&self.key, &it.content)
                    .unwrap_or_else(|_| "[解密失败]".into());
                MemoView {
                    id: it.id,
                    title: it.title,
                    content: plain,
                    deleted: it.deleted,
                    version: it.version,
                    node_id: it.node_id,
                }
            })
            .collect()
    }

    pub fn edit(&self, id: &str, title: &str, content: &str) -> anyhow::Result<()> {
        if self.store.get(id).is_none() {
            anyhow::bail!("备忘不存在");
        }
        let ct = crypto::encrypt_string(&self.key, content)?;
        let (aid, aname) = self.current_actor();
        self.store.put(id, title, &ct, &aid, &aname)?;
        self.fire();
        Ok(())
    }

    pub fn upsert_imported(&self, view: &MemoView) -> anyhow::Result<()> {
        let ct = crypto::encrypt_string(&self.key, &view.content)?;
        let (aid, aname) = self.current_actor();
        self.store.put(&view.id, &view.title, &ct, &aid, &aname)?;
        self.fire();
        Ok(())
    }

    pub fn verify_password(&self, password: &str) -> anyhow::Result<()> {
        self.verify_master_password(password)
    }

    pub fn delete(&self, id: &str, password: &str) -> anyhow::Result<()> {
        self.verify_master_password(password)?;
        let (aid, aname) = self.current_actor();
        self.store.delete(id, &aid, &aname)?;
        self.fire();
        Ok(())
    }

    fn verify_master_password(&self, password: &str) -> anyhow::Result<()> {
        if password.is_empty() {
            anyhow::bail!("请输入主密码");
        }
        let salt = resolve_salt(&self.cfg)?;
        let derived = derive_key(password.as_bytes(), &salt, &self.cfg.argon2)?;
        if !keys_equal(&derived, &self.key) {
            anyhow::bail!("主密码错误");
        }
        Ok(())
    }

    pub fn verify_audit(&self) -> (bool, String) {
        match self.audit.verify_file() {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e),
        }
    }

    pub fn export_txt(&self, path: PathBuf, ids: Option<Vec<String>>) -> anyhow::Result<()> {
        let all = self.list();
        let items: Vec<_> = match ids {
            None => all,
            Some(ids) => {
                let set: std::collections::HashSet<_> = ids.into_iter().collect();
                all.into_iter().filter(|i| set.contains(&i.id)).collect()
            }
        };
        export_txt(&path, &self.cfg.node_id, &items)
    }

    pub fn export_backup(&self, path: &Path, password: &str) -> anyhow::Result<()> {
        backup::export_encrypted(
            path,
            &self.cfg,
            password,
            &self.list(),
            &self.list_persons(),
            &self.list_tasks(),
        )
    }

    pub fn import_backup(&self, path: &Path, password: &str) -> anyhow::Result<usize> {
        let bundle = backup::import_encrypted(path, password)?;
        let mut n = 0usize;
        for it in &bundle.memos {
            self.upsert_imported(it)?;
            n += 1;
        }
        for p in &bundle.persons {
            let placeholder = format!("reset-{}", &p.id[..8.min(p.id.len())]);
            let (aid, aname) = self.current_actor();
            let _ = self.person_store.put_local(
                &p.id,
                &p.name,
                Some(&placeholder),
                p.disabled,
                None,
                &aid,
                &aname,
            );
            n += 1;
        }
        for t in &bundle.tasks {
            self.upsert_task(t)?;
            n += 1;
        }
        self.fire();
        Ok(n)
    }

    pub fn take_conflicts(&self) -> Vec<ConflictNotice> {
        let mut out = self.store.take_conflicts();
        out.extend(self.person_store.take_conflicts());
        out.extend(self.task_store.take_conflicts());
        out
    }

    pub fn history_for(&self, entity: &str, id: &str) -> anyhow::Result<Vec<HistoryEvent>> {
        let entries = self.audit.read_all()?;
        let mut out = Vec::new();
        for e in entries {
            let ed: EventData = match serde_json::from_value(e.data.clone()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if ed.entity != entity || ed.memo_id != id {
                continue;
            }
            let et = match ed.event_type {
                EventType::Create => "CREATE",
                EventType::Modify => "MODIFY",
                EventType::Delete => "DELETE",
            };
            let before = ed
                .before
                .as_ref()
                .and_then(|v| self.snapshot_for_entity(entity, v).ok());
            let after = ed
                .after
                .as_ref()
                .and_then(|v| self.snapshot_for_entity(entity, v).ok());
            let author_node = after
                .as_ref()
                .map(|s| s.node_id.clone())
                .unwrap_or_else(|| ed.node_id.clone());
            let actor_person_id = ed.actor_person_id.unwrap_or_default();
            let actor_name = ed.actor_name.unwrap_or_default();
            out.push(HistoryEvent {
                seq: e.seq,
                time: e.time,
                event_type: et.into(),
                actor_person_id,
                actor_name,
                author_node,
                record_node: ed.node_id,
                source: ed.source,
                before,
                after,
            });
        }
        Ok(out)
    }

    fn snapshot_for_entity(
        &self,
        entity: &str,
        v: &serde_json::Value,
    ) -> anyhow::Result<HistorySnapshot> {
        match entity {
            "memo" => self.snapshot_from_memo(v),
            "task" => self.snapshot_from_task(v),
            "person" => self.snapshot_from_person(v),
            _ => anyhow::bail!("未知实体类型: {entity}"),
        }
    }

    fn snapshot_from_memo(&self, v: &serde_json::Value) -> anyhow::Result<HistorySnapshot> {
        let item: MemoItem = serde_json::from_value(v.clone())?;
        let content = if item.content.is_empty() {
            String::new()
        } else {
            crypto::decrypt_string(&self.key, &item.content)
                .unwrap_or_else(|_| "[无法解密]".into())
        };
        Ok(HistorySnapshot {
            title: item.title,
            content,
            version: item.version,
            node_id: item.node_id,
            deleted: item.deleted,
        })
    }

    fn snapshot_from_task(&self, v: &serde_json::Value) -> anyhow::Result<HistorySnapshot> {
        let item: TaskItem = serde_json::from_value(v.clone())?;
        let plan = if item.plan.is_empty() {
            String::new()
        } else {
            crypto::decrypt_string(&self.key, &item.plan)
                .unwrap_or_else(|_| "[无法解密]".into())
        };
        let assignee = if item.assignee_id.is_empty() {
            "未指定".to_string()
        } else {
            self.person_name(&item.assignee_id)
        };
        let (end_d, end_h) = item.resolved_end();
        let content = format!(
            "开始: {} {:.1} 时\n结束: {} {:.1} 时\n工时: {:.1}\n负责人: {}\n状态: {}\n计划:\n{}",
            item.date,
            item.start_hour,
            end_d,
            end_h,
            item.hours,
            assignee,
            item.status.label(),
            plan
        );
        Ok(HistorySnapshot {
            title: item.title,
            content,
            version: item.version,
            node_id: item.node_id,
            deleted: item.deleted,
        })
    }

    fn snapshot_from_person(&self, v: &serde_json::Value) -> anyhow::Result<HistorySnapshot> {
        let item: Person = serde_json::from_value(v.clone())?;
        let content = format!(
            "禁用: {}\n已删除: {}",
            if item.disabled { "是" } else { "否" },
            if item.deleted { "是" } else { "否" }
        );
        Ok(HistorySnapshot {
            title: item.name,
            content,
            version: item.version,
            node_id: item.node_id,
            deleted: item.deleted,
        })
    }

    pub fn notify_updated(&self) {
        self.fire();
    }

    pub fn display_name_for(&self, node_id: &str) -> String {
        if node_id == self.cfg.node_id {
            let dn = self.cfg.node_display_name.trim();
            if !dn.is_empty() {
                return format!("{dn} ({node_id})");
            }
        }
        node_id.to_string()
    }

    pub fn disk_space(&self) -> crate::disk::DiskSpace {
        crate::disk::probe_data_dir(Path::new(&self.cfg.data_dir))
    }

    pub fn list_persons(&self) -> Vec<PersonView> {
        let mut v: Vec<_> = self
            .person_store
            .get_visible()
            .iter()
            .map(PersonStore::to_view)
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn list_active_persons(&self) -> Vec<PersonView> {
        self.list_persons()
            .into_iter()
            .filter(|p| !p.disabled)
            .collect()
    }

    pub fn add_person(&self, name: &str, password: &str) -> anyhow::Result<String> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("姓名不能为空");
        }
        if password.is_empty() {
            anyhow::bail!("人员密码不能为空");
        }
        let id = Uuid::new_v4().to_string().replace('-', "");
        let (aid, aname) = self.current_actor();
        self.person_store
            .put_local(&id, name, Some(password), false, None, &aid, &aname)?;
        self.fire();
        Ok(id)
    }

    pub fn update_person(&self, id: &str, name: &str, disabled: bool) -> anyhow::Result<()> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("姓名不能为空");
        }
        let prev = self
            .person_store
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("人员不存在"))?;
        let (aid, aname) = self.current_actor();
        self.person_store.put_local(
            id,
            name,
            None,
            disabled,
            Some((prev.salt_hex, prev.password_verifier)),
            &aid,
            &aname,
        )?;
        self.fire();
        Ok(())
    }

    pub fn set_person_password(&self, id: &str, new_password: &str) -> anyhow::Result<()> {
        if new_password.is_empty() {
            anyhow::bail!("人员密码不能为空");
        }
        let (aid, aname) = self.current_actor();
        self.person_store
            .set_password(id, new_password, &aid, &aname)?;
        self.fire();
        Ok(())
    }

    pub fn disable_person(&self, id: &str, disabled: bool) -> anyhow::Result<()> {
        let prev = self
            .person_store
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("人员不存在"))?;
        let (aid, aname) = self.current_actor();
        self.person_store.put_local(
            id,
            &prev.name,
            None,
            disabled,
            Some((prev.salt_hex, prev.password_verifier)),
            &aid,
            &aname,
        )?;
        self.fire();
        Ok(())
    }

    pub fn delete_person(&self, id: &str, master_password: &str) -> anyhow::Result<()> {
        self.verify_master_password(master_password)?;
        let (aid, aname) = self.current_actor();
        self.person_store.delete(id, &aid, &aname)?;
        if self.current_person_id() == Some(id.to_string()) {
            self.set_current_person(None);
        }
        self.fire();
        Ok(())
    }

    pub fn verify_person_password(&self, id: &str, password: &str) -> anyhow::Result<()> {
        let p = self
            .person_store
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("人员不存在"))?;
        if p.deleted || p.disabled {
            anyhow::bail!("该人员已禁用");
        }
        self.person_store.verify_password(&p, password)?;
        self.set_current_person(Some(id.to_string()));
        Ok(())
    }

    pub fn person_name(&self, id: &str) -> String {
        self.person_store
            .get(id)
            .map(|p| p.name)
            .unwrap_or_else(|| id.to_string())
    }

    pub fn merge_person(&self, remote: Person, source: &str) -> anyhow::Result<bool> {
        let ok = self.person_store.merge(remote, source)?;
        if ok {
            self.fire();
        }
        Ok(ok)
    }

    fn decrypt_task(&self, it: TaskItem) -> TaskView {
        let plan = if it.plan.is_empty() {
            String::new()
        } else {
            crypto::decrypt_string(&self.key, &it.plan)
                .unwrap_or_else(|_| "[解密失败]".into())
        };
        let (end_date, end_hour) = it.resolved_end();
        TaskView {
            id: it.id,
            title: it.title,
            plan,
            date: it.date,
            start_hour: it.start_hour,
            hours: it.hours,
            end_date,
            end_hour,
            assignee_id: it.assignee_id,
            status: it.status,
            deleted: it.deleted,
            version: it.version,
            node_id: it.node_id,
        }
    }

    pub fn list_tasks(&self) -> Vec<TaskView> {
        self.task_store
            .get_visible()
            .into_iter()
            .map(|it| self.decrypt_task(it))
            .collect()
    }

    pub fn list_tasks_on(&self, date: &str) -> Vec<TaskView> {
        let mut v: Vec<_> = self
            .list_tasks()
            .into_iter()
            .filter(|t| t.spans_date(date))
            .collect();
        v.sort_by(|a, b| {
            a.date
                .cmp(&b.date)
                .then_with(|| {
                    a.start_hour
                        .partial_cmp(&b.start_hour)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.title.cmp(&b.title))
        });
        v
    }

    pub fn list_tasks_in_range(&self, start_date: &str, end_date: &str) -> Vec<TaskView> {
        let mut v: Vec<_> = self
            .list_tasks()
            .into_iter()
            .filter(|t| t.date.as_str() <= end_date && t.end_date.as_str() >= start_date)
            .collect();
        v.sort_by(|a, b| {
            a.date
                .cmp(&b.date)
                .then_with(|| {
                    a.start_hour
                        .partial_cmp(&b.start_hour)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.title.cmp(&b.title))
        });
        v
    }

    pub fn get_task(&self, id: &str) -> Option<TaskView> {
        self.task_store.get(id).map(|it| self.decrypt_task(it))
    }

    pub fn add_task(
        &self,
        title: &str,
        plan: &str,
        date: &str,
        start_hour: f32,
        end_date: &str,
        end_hour: f32,
        assignee_id: &str,
        status: TaskStatus,
    ) -> anyhow::Result<String> {
        if title.trim().is_empty() {
            anyhow::bail!("任务标题不能为空");
        }
        if date.len() != 10 || end_date.len() != 10 {
            anyhow::bail!("日期格式应为 YYYY-MM-DD");
        }
        let ct = crypto::encrypt_string(&self.key, plan)?;
        let id = Uuid::new_v4().to_string().replace('-', "");
        let (aid, aname) = self.current_actor();
        self.task_store.put(
            &id,
            title.trim(),
            &ct,
            date,
            start_hour,
            end_date,
            end_hour,
            assignee_id,
            status,
            &aid,
            &aname,
        )?;
        self.fire();
        Ok(id)
    }

    pub fn update_task(
        &self,
        id: &str,
        title: &str,
        plan: &str,
        date: &str,
        start_hour: f32,
        end_date: &str,
        end_hour: f32,
        assignee_id: &str,
        status: TaskStatus,
    ) -> anyhow::Result<()> {
        if self.task_store.get(id).is_none() {
            anyhow::bail!("任务不存在");
        }
        if title.trim().is_empty() {
            anyhow::bail!("任务标题不能为空");
        }
        if date.len() != 10 || end_date.len() != 10 {
            anyhow::bail!("日期格式应为 YYYY-MM-DD");
        }
        let ct = crypto::encrypt_string(&self.key, plan)?;
        let (aid, aname) = self.current_actor();
        self.task_store.put(
            id,
            title.trim(),
            &ct,
            date,
            start_hour,
            end_date,
            end_hour,
            assignee_id,
            status,
            &aid,
            &aname,
        )?;
        self.fire();
        Ok(())
    }

    pub fn upsert_task(&self, view: &TaskView) -> anyhow::Result<()> {
        let ct = crypto::encrypt_string(&self.key, &view.plan)?;
        let (aid, aname) = self.current_actor();
        self.task_store.put(
            &view.id,
            &view.title,
            &ct,
            &view.date,
            view.start_hour,
            &view.end_date,
            view.end_hour,
            &view.assignee_id,
            view.status,
            &aid,
            &aname,
        )?;
        Ok(())
    }

    pub fn delete_task(&self, id: &str, password: &str) -> anyhow::Result<()> {
        self.verify_master_password(password)?;
        let (aid, aname) = self.current_actor();
        self.task_store.delete(id, &aid, &aname)?;
        self.fire();
        Ok(())
    }

    pub fn merge_task(&self, remote: TaskItem, source: &str) -> anyhow::Result<bool> {
        let ok = self.task_store.merge(remote, source)?;
        if ok {
            self.fire();
        }
        Ok(ok)
    }
}

/// Unlock vault with master password (rebuild stores + verifier).
pub fn unlock(cfg: Config, password: &[u8]) -> anyhow::Result<(Arc<MemoService>, Arc<AuditLog>)> {
    crate::config::ensure_data_dir(&cfg)?;
    let data_dir = PathBuf::from(&cfg.data_dir);
    let salt = resolve_salt(&cfg)?;
    let key = derive_key(password, &salt, &cfg.argon2)?;

    if verifier::exists(&data_dir) {
        verifier::verify(&data_dir, &key)?;
    }

    let keys = crate::audit::KeyPair::load_or_create(&data_dir)?;
    let audit_path = data_dir.join("audit.jsonl");
    let audit = Arc::new(AuditLog::open(audit_path, keys)?);
    let store = Arc::new(MemoStore::new(cfg.node_id.clone(), audit.clone()));
    store.rebuild_from_audit()?;

    let person_store = Arc::new(PersonStore::open(
        cfg.node_id.clone(),
        &data_dir,
        &key,
        cfg.argon2.clone(),
        audit.clone(),
    )?);
    let task_store = Arc::new(TaskStore::open(
        cfg.node_id.clone(),
        &data_dir,
        &key,
        audit.clone(),
    )?);

    let svc = MemoService::new(
        cfg,
        key.clone(),
        store,
        person_store,
        task_store,
        audit.clone(),
    );

    if !verifier::exists(&data_dir) {
        verifier::write(&data_dir, &key)?;
    }

    Ok((svc, audit))
}
