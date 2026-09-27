//! 人员账号（主密码开库后，用人员密码切换身份）。

use parking_lot::RwLock;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::audit::{AuditLog, EventData, EventType};
use crate::config::Argon2Params;
use crate::crypto::{self, derive_key};
use crate::enc_store;
use crate::store::{Broadcaster, ConflictNotice};

const PERSON_FILE: &str = "persons.json.enc";
const PERSON_MAGIC: &str = "MEMO_PERSON_V1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub name: String,
    /// 人员密码校验密文（派生密钥加密的魔串）
    pub password_verifier: String,
    /// 人员独立盐（hex）
    pub salt_hex: String,
    pub disabled: bool,
    pub deleted: bool,
    pub version: u64,
    pub node_id: String,
}

#[derive(Debug, Clone)]
pub struct PersonView {
    pub id: String,
    pub name: String,
    pub disabled: bool,
    pub version: u64,
    pub node_id: String,
}

pub struct PersonStore {
    items: RwLock<HashMap<String, Person>>,
    clock: RwLock<u64>,
    node_id: String,
    data_dir: PathBuf,
    key: parking_lot::Mutex<Vec<u8>>,
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

    fn make_verifier(&self, password: &str, salt_hex: &str) -> anyhow::Result<String> {
        let salt = hex::decode(salt_hex.trim())?;
        let derived = derive_key(password.as_bytes(), &salt, &self.argon2)?;
        crypto::encrypt_string(&derived, PERSON_MAGIC)
    }

    pub fn verify_password(&self, person: &Person, password: &str) -> anyhow::Result<()> {
        if password.is_empty() {
            anyhow::bail!("请输入人员密码");
        }
        let salt = hex::decode(person.salt_hex.trim())?;
        let derived = derive_key(password.as_bytes(), &salt, &self.argon2)?;
        let plain = crypto::decrypt_string(&derived, &person.password_verifier)
            .map_err(|_| anyhow::anyhow!("人员密码错误"))?;
        if plain != PERSON_MAGIC {
            anyhow::bail!("人员密码错误");
        }
        Ok(())
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
        password: Option<&str>,
        disabled: bool,
        keep_verifier: Option<(String, String)>,
        actor_person_id: &str,
        actor_name: &str,
    ) -> anyhow::Result<Person> {
        let mut items = self.items.write();
        let prev = items.get(id).cloned();
        let ver = self.tick(0);
        let (salt_hex, password_verifier) = if let Some(pw) = password {
            let mut salt = vec![0u8; self.argon2.salt_len as usize];
            rand::thread_rng().fill_bytes(&mut salt);
            let salt_hex = hex::encode(&salt);
            let verifier = self.make_verifier(pw, &salt_hex)?;
            (salt_hex, verifier)
        } else if let Some((s, v)) = keep_verifier {
            (s, v)
        } else if let Some(p) = &prev {
            (p.salt_hex.clone(), p.password_verifier.clone())
        } else {
            anyhow::bail!("新建人员必须设置密码");
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

    pub fn set_password(
        &self,
        id: &str,
        new_password: &str,
        actor_person_id: &str,
        actor_name: &str,
    ) -> anyhow::Result<Person> {
        let prev = self
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("人员不存在"))?;
        self.put_local(
            id,
            &prev.name,
            Some(new_password),
            prev.disabled,
            None,
            actor_person_id,
            actor_name,
        )
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
        // 人员实体无 modified_by；同步合并审计不填操作人
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
        }
    }
}
