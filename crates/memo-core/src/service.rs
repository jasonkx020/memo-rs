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
use crate::store::{Broadcaster, ConflictNotice, MemoItem, MemoStore};
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
    /// 写入该版本的作者节点（after.node_id）
    pub author_node: String,
    /// 记录本条审计的本机节点
    pub record_node: String,
    /// 同步来源 peer；本地操作为空
    pub source: String,
    pub before: Option<HistorySnapshot>,
    pub after: Option<HistorySnapshot>,
}

pub struct MemoService {
    cfg: Config,
    key: Zeroizing<Vec<u8>>,
    store: Arc<MemoStore>,
    audit: Arc<AuditLog>,
    subs: Mutex<Vec<Box<dyn Fn() + Send + Sync>>>,
}

impl MemoService {
    pub fn new(
        cfg: Config,
        key: Zeroizing<Vec<u8>>,
        store: Arc<MemoStore>,
        audit: Arc<AuditLog>,
    ) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            key,
            store,
            audit,
            subs: Mutex::new(Vec::new()),
        })
    }

    pub fn store(&self) -> Arc<MemoStore> {
        self.store.clone()
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn set_broadcaster(&self, bc: Arc<dyn Broadcaster>) {
        self.store.set_broadcaster(bc);
    }

    pub fn subscribe(&self, f: Box<dyn Fn() + Send + Sync>) {
        self.subs.lock().push(f);
    }

    fn fire(&self) {
        for f in self.subs.lock().iter() {
            f();
        }
    }

    pub fn add(&self, title: &str, content: &str) -> anyhow::Result<String> {
        let ct = crypto::encrypt_string(&self.key, content)?;
        let id = Uuid::new_v4().to_string().replace('-', "");
        self.store.put(&id, title, &ct)?;
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
        self.store.put(id, title, &ct)?;
        self.fire();
        Ok(())
    }

    /// 按 id 写入明文备忘（导入备份用；保留原 id）。
    pub fn upsert_imported(&self, view: &MemoView) -> anyhow::Result<()> {
        let ct = crypto::encrypt_string(&self.key, &view.content)?;
        self.store.put(&view.id, &view.title, &ct)?;
        self.fire();
        Ok(())
    }

    pub fn verify_password(&self, password: &str) -> anyhow::Result<()> {
        self.verify_master_password(password)
    }

    pub fn delete(&self, id: &str, password: &str) -> anyhow::Result<()> {
        self.verify_master_password(password)?;
        self.store.delete(id)?;
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
        backup::export_encrypted(path, &self.cfg, password, &self.list())
    }

    pub fn import_backup(&self, path: &Path, password: &str) -> anyhow::Result<usize> {
        let items = backup::import_encrypted(path, password)?;
        let n = items.len();
        for it in items {
            self.upsert_imported(&it)?;
        }
        Ok(n)
    }

    pub fn take_conflicts(&self) -> Vec<ConflictNotice> {
        self.store.take_conflicts()
    }

    /// 本机视角：该备忘的变更时间线（解密 before/after 正文）。
    pub fn history_for(&self, memo_id: &str) -> anyhow::Result<Vec<HistoryEvent>> {
        let entries = self.audit.read_all()?;
        let mut out = Vec::new();
        for e in entries {
            let ed: EventData = match serde_json::from_value(e.data.clone()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if ed.memo_id != memo_id {
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
                .and_then(|v| self.snapshot_from_value(v).ok());
            let after = ed
                .after
                .as_ref()
                .and_then(|v| self.snapshot_from_value(v).ok());
            let author_node = after
                .as_ref()
                .map(|s| s.node_id.clone())
                .unwrap_or_else(|| ed.node_id.clone());
            out.push(HistoryEvent {
                seq: e.seq,
                time: e.time,
                event_type: et.into(),
                author_node,
                record_node: ed.node_id,
                source: ed.source,
                before,
                after,
            });
        }
        Ok(out)
    }

    fn snapshot_from_value(&self, v: &serde_json::Value) -> anyhow::Result<HistorySnapshot> {
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
}

/// 用主密码装配 store（含审计重放与密码校验文件）。
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
    let svc = MemoService::new(cfg, key.clone(), store, audit.clone());

    // 首次或旧数据迁移：写入校验文件
    if !verifier::exists(&data_dir) {
        verifier::write(&data_dir, &key)?;
    }

    Ok((svc, audit))
}
