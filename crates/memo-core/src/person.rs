//! 任务负责人等展示用人员目录（无日常密码；身份由密钥对决定）。

use parking_lot::RwLock;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::audit::{AuditLog, EventData, EventType};
use crate::config::Argon2Params;
use crate::enc_store;
use crate::store::{Broadcaster, ConflictNotice};

const PERSON_FILE: &str = "persons.json.enc";

/// 人员性别（生理期提醒等能力仅面向女性）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Gender {
    #[default]
    Unknown,
    Male,
    Female,
}

impl Gender {
    pub fn label(self) -> &'static str {
        match self {
            Gender::Unknown => "未设置",
            Gender::Male => "男",
            Gender::Female => "女",
        }
    }

    /// 发现广播 / 节点标签用短文案。
    pub fn tag(self) -> &'static str {
        match self {
            Gender::Unknown => "",
            Gender::Male => "男",
            Gender::Female => "女",
        }
    }

    pub fn from_announce(s: &str) -> Self {
        match s.trim() {
            "male" | "男" => Gender::Male,
            "female" | "女" => Gender::Female,
            _ => Gender::Unknown,
        }
    }

    pub fn announce_code(self) -> &'static str {
        match self {
            Gender::Male => "male",
            Gender::Female => "female",
            Gender::Unknown => "",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub name: String,
    /// 遗留字段，恒为空（不再校验人员密码）
    #[serde(default)]
    pub password_verifier: String,
    #[serde(default)]
    pub salt_hex: String,
    pub disabled: bool,
    pub deleted: bool,
    pub version: u64,
    pub node_id: String,
    #[serde(default)]
    pub gender: Gender,
}

#[derive(Debug, Clone)]
pub struct PersonView {
    pub id: String,
    pub name: String,
    pub disabled: bool,
    pub version: u64,
    pub node_id: String,
    pub gender: Gender,
}

pub struct PersonStore {
    items: RwLock<HashMap<String, Person>>,
    clock: RwLock<u64>,
    node_id: String,
    data_dir: PathBuf,
    key: parking_lot::Mutex<Vec<u8>>,
    #[allow(dead_code)]
    argon2: Argon2Params,
    audit: Arc<AuditLog>,
    broadcaster: RwLock<Option<Arc<dyn Broadcaster>>>,
    conflicts: RwLock<Vec<ConflictNotice>>,
}

impl PersonStore {
    pub fn open(
        node_id: String,
        data_dir: &Path,
        key: &[u8],
        argon2: Argon2Params,
        audit: Arc<AuditLog>,
    ) -> anyhow::Result<Self> {
        let loaded = enc_store::load_vec::<Person>(data_dir, PERSON_FILE, key)?;
        let mut items = HashMap::new();
        let mut clock = 0u64;
        for p in loaded {
            if p.version > clock {
                clock = p.version;
            }
            items.insert(p.id.clone(), p);
        }
        Ok(Self {
            items: RwLock::new(items),
            clock: RwLock::new(clock),
            node_id,
            data_dir: data_dir.to_path_buf(),
            key: parking_lot::Mutex::new(key.to_vec()),
            argon2,
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
        enc_store::save_vec(&self.data_dir, PERSON_FILE, &key, &items)
    }

    fn audit_write(
        &self,
        ev: EventType,
        id: &str,
        before: Option<&Person>,
        after: Option<&Person>,
        source: &str,
        actor_person_id: Option<String>,
        actor_name: Option<String>,
    ) -> anyhow::Result<()> {
        let data = EventData {
            event_type: ev,
            entity: "person".into(),
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

    pub fn put_local(
        &self,
        id: &str,
        name: &str,
        _password: Option<&str>,
        disabled: bool,
        gender: Gender,
        keep_verifier: Option<(String, String)>,
        actor_person_id: &str,
        actor_name: &str,
    ) -> anyhow::Result<Person> {
        let mut items = self.items.write();
        let prev = items.get(id).cloned();
        let ver = self.tick(0);
        let (salt_hex, password_verifier) = if let Some((s, v)) = keep_verifier {
            (s, v)
        } else if let Some(p) = &prev {
            (p.salt_hex.clone(), p.password_verifier.clone())
        } else {
            let mut salt = vec![0u8; 16];
            rand::thread_rng().fill_bytes(&mut salt);
            (hex::encode(salt), String::new())
        };
        let item = Person {
            id: id.to_string(),
            name: name.to_string(),
            password_verifier,
            salt_hex,
            disabled,
            deleted: false,
            version: ver,
            node_id: self.node_id.clone(),
            gender,
        };
        items.insert(id.to_string(), item.clone());
        drop(items);
        let ev = if prev.is_some() {
            EventType::Modify
        } else {
            EventType::Create
        };
        let aid = if actor_person_id.is_empty() {
            None
        } else {
            Some(actor_person_id.to_string())
        };
        let aname = if actor_name.is_empty() {
            None
        } else {
            Some(actor_name.to_string())
        };
        self.audit_write(ev, id, prev.as_ref(), Some(&item), "", aid, aname)?;
        self.persist()?;
        if let Some(bc) = self.broadcaster.read().clone() {
            bc.broadcast_person(&item);
        }
        Ok(item)
    }

    pub fn delete(
        &self,
        id: &str,
        actor_person_id: &str,
        actor_name: &str,
    ) -> anyhow::Result<Person> {
        let mut items = self.items.write();
        let prev = items
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("人员不存在: {id}"))?;
        let ver = self.tick(0);
        let mut item = prev.clone();
        item.deleted = true;
        item.version = ver;
        item.node_id = self.node_id.clone();
        items.insert(id.to_string(), item.clone());
        drop(items);
        let aid = if actor_person_id.is_empty() {
            None
        } else {
            Some(actor_person_id.to_string())
        };
        let aname = if actor_name.is_empty() {
            None
        } else {
            Some(actor_name.to_string())
        };
        self.audit_write(EventType::Delete, id, Some(&prev), Some(&item), "", aid, aname)?;
        self.persist()?;
        if let Some(bc) = self.broadcaster.read().clone() {
            bc.broadcast_person(&item);
        }
        Ok(item)
    }

    pub fn merge(&self, remote: Person, source: &str) -> anyhow::Result<bool> {
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
                    title: l.name.clone(),
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
        self.audit_write(ev, &remote.id, local.as_ref(), Some(&remote), source, None, None)?;
        self.persist()?;
        Ok(true)
    }

    pub fn get(&self, id: &str) -> Option<Person> {
        self.items.read().get(id).cloned()
    }

    pub fn get_visible(&self) -> Vec<Person> {
        self.items
            .read()
            .values()
            .filter(|i| !i.deleted)
            .cloned()
            .collect()
    }

    pub fn all(&self) -> Vec<Person> {
        self.items.read().values().cloned().collect()
    }

    pub fn to_view(p: &Person) -> PersonView {
        PersonView {
            id: p.id.clone(),
            name: p.name.clone(),
            disabled: p.disabled,
            version: p.version,
            node_id: p.node_id.clone(),
            gender: p.gender,
        }
    }
}
