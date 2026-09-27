//! 加密备份 / 导入（`.memobak`）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::config::{Argon2Params, Config};
use crate::crypto::{self, derive_key, resolve_salt};
use crate::person::PersonView;
use crate::service::MemoView;
use crate::task::{end_from_duration, TaskStatus, TaskView};

const FORMAT: &str = "memo-bak-v1";

#[derive(Debug, Serialize, Deserialize)]
struct BackupFile {
    format: String,
    salt_hex: String,
    argon2: Argon2Params,
    /// AES-GCM(base64) 包裹的 JSON 明文
    ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupPayload {
    node_id: String,
    exported_at: String,
    memos: Vec<BackupMemo>,
    #[serde(default)]
    persons: Vec<BackupPerson>,
    #[serde(default)]
    tasks: Vec<BackupTask>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupMemo {
    id: String,
    title: String,
    content: String,
    version: u64,
    node_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupPerson {
    id: String,
    name: String,
    disabled: bool,
    version: u64,
    node_id: String,
    /// 备份不含人员密码哈希；导入后需重新设密
    #[serde(default)]
    needs_password_reset: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupTask {
    id: String,
    title: String,
    plan: String,
    date: String,
    #[serde(default = "default_start")]
    start_hour: f32,
    hours: f32,
    #[serde(default)]
    end_date: String,
    #[serde(default)]
    end_hour: Option<f32>,
    assignee_id: String,
    #[serde(default)]
    status: TaskStatus,
    version: u64,
    node_id: String,
}

fn default_start() -> f32 {
    9.0
}

pub struct BackupBundle {
    pub memos: Vec<MemoView>,
    pub persons: Vec<PersonView>,
    pub tasks: Vec<TaskView>,
}

/// 用给定密码加密导出全部可见备忘（及人员元数据、任务）。
pub fn export_encrypted(
    path: &Path,
    cfg: &Config,
    password: &str,
    items: &[MemoView],
    persons: &[PersonView],
    tasks: &[TaskView],
) -> anyhow::Result<()> {
    if password.is_empty() {
        anyhow::bail!("备份密码不能为空");
    }
    if items.is_empty() && persons.is_empty() && tasks.is_empty() {
        anyhow::bail!("没有可备份的数据");
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let salt = resolve_salt(cfg)?;
    let key = derive_key(password.as_bytes(), &salt, &cfg.argon2)?;
    let payload = BackupPayload {
        node_id: cfg.node_id.clone(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        memos: items
            .iter()
            .map(|m| BackupMemo {
                id: m.id.clone(),
                title: m.title.clone(),
                content: m.content.clone(),
                version: m.version,
                node_id: m.node_id.clone(),
            })
            .collect(),
        persons: persons
            .iter()
            .map(|p| BackupPerson {
                id: p.id.clone(),
                name: p.name.clone(),
                disabled: p.disabled,
                version: p.version,
                node_id: p.node_id.clone(),
                needs_password_reset: true,
            })
            .collect(),
        tasks: tasks
            .iter()
            .map(|t| BackupTask {
                id: t.id.clone(),
                title: t.title.clone(),
                plan: t.plan.clone(),
                date: t.date.clone(),
                start_hour: t.start_hour,
                hours: t.hours,
                end_date: t.end_date.clone(),
                end_hour: Some(t.end_hour),
                assignee_id: t.assignee_id.clone(),
                status: t.status,
                version: t.version,
                node_id: t.node_id.clone(),
            })
            .collect(),
    };
    let plain = serde_json::to_string(&payload)?;
    let ciphertext = crypto::encrypt_string(&key, &plain)?;
    let file = BackupFile {
        format: FORMAT.into(),
        salt_hex: cfg.salt_hex.clone(),
        argon2: cfg.argon2.clone(),
        ciphertext,
    };
    fs::write(path, serde_json::to_string_pretty(&file)?)?;
    Ok(())
}

/// 解密备份。
pub fn import_encrypted(path: &Path, password: &str) -> anyhow::Result<BackupBundle> {
    if password.is_empty() {
        anyhow::bail!("备份密码不能为空");
    }
    let raw = fs::read_to_string(path)?;
    let file: BackupFile = serde_json::from_str(&raw)?;
    if file.format != FORMAT {
        anyhow::bail!("不支持的备份格式: {}", file.format);
    }
    let salt = hex::decode(file.salt_hex.trim())?;
    if salt.len() != file.argon2.salt_len as usize {
        anyhow::bail!("备份盐长度异常");
    }
    let key = derive_key(password.as_bytes(), &salt, &file.argon2)?;
    let plain = crypto::decrypt_string(&key, &file.ciphertext)
        .map_err(|_| anyhow::anyhow!("备份密码错误或文件损坏"))?;
    let payload: BackupPayload = serde_json::from_str(&plain)?;
    Ok(BackupBundle {
        memos: payload
            .memos
            .into_iter()
            .map(|m| MemoView {
                id: m.id,
                title: m.title,
                content: m.content,
                deleted: false,
                version: m.version,
                node_id: m.node_id,
            })
            .collect(),
        persons: payload
            .persons
            .into_iter()
            .map(|p| PersonView {
                id: p.id,
                name: p.name,
                disabled: p.disabled,
                version: p.version,
                node_id: p.node_id,
            })
            .collect(),
        tasks: payload
            .tasks
            .into_iter()
            .map(|t| {
                let (end_date, end_hour) = if !t.end_date.is_empty() {
                    if let Some(h) = t.end_hour {
                        (t.end_date, h)
                    } else {
                        end_from_duration(&t.date, t.start_hour, t.hours)
                    }
                } else {
                    end_from_duration(&t.date, t.start_hour, t.hours)
                };
                TaskView {
                    id: t.id,
                    title: t.title,
                    plan: t.plan,
                    date: t.date,
                    start_hour: t.start_hour,
                    hours: t.hours,
                    end_date,
                    end_hour,
                    assignee_id: t.assignee_id,
                    status: t.status,
                    deleted: false,
                    version: t.version,
                    node_id: t.node_id,
                }
            })
            .collect(),
    })
}
