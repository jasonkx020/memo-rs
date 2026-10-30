//! 加密备份 / 导入（`.memobak`）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::attachments::{self, PackedAttachment};
use crate::config::{Argon2Params, Config};
use crate::crypto::{self, derive_key, resolve_salt};
use crate::person::PersonView;
use crate::service::MemoView;

const FORMAT_V1: &str = "memo-bak-v1";
const FORMAT_V2: &str = "memo-bak-v2";

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
    /// 旧版备份可能含 tasks；导入时忽略
    #[serde(default)]
    tasks: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupMemo {
    id: String,
    title: String,
    content: String,
    version: u64,
    node_id: String,
    #[serde(default)]
    attachments: Vec<PackedAttachment>,
    #[serde(default)]
    category: crate::store::MemoCategory,
    #[serde(default)]
    due_date: String,
    #[serde(default)]
    end_date: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    priority: crate::store::MemoPriority,
    #[serde(default)]
    remind_before_days: u32,
    #[serde(default)]
    remind_seen_for: String,
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
    #[serde(default)]
    gender: crate::person::Gender,
    #[serde(default)]
    birth_ymd: String,
    #[serde(default)]
    birth_hour: Option<u8>,
}

pub struct BackupBundle {
    pub memos: Vec<MemoView>,
    pub persons: Vec<PersonView>,
    /// 与 memos 一一对应的附件包（可能为空）
    pub memo_attachments: Vec<Vec<PackedAttachment>>,
}

/// 用给定密码加密导出全部可见备忘（及人员元数据）；携带插图等附件。
pub fn export_encrypted(
    path: &Path,
    cfg: &Config,
    password: &str,
    items: &[MemoView],
    persons: &[PersonView],
) -> anyhow::Result<()> {
    if password.is_empty() {
        anyhow::bail!("备份密码不能为空");
    }
    if items.is_empty() && persons.is_empty() {
        anyhow::bail!("没有可备份的数据");
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let data_dir = Path::new(&cfg.data_dir);
    let salt = resolve_salt(cfg)?;
    let key = derive_key(password.as_bytes(), &salt, &cfg.argon2)?;
    let payload = BackupPayload {
        node_id: cfg.node_id.clone(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        memos: items
            .iter()
            .map(|m| {
                let attachments =
                    attachments::pack_for_content(data_dir, &m.id, &m.content).unwrap_or_default();
                BackupMemo {
                    id: m.id.clone(),
                    title: m.title.clone(),
                    content: m.content.clone(),
                    version: m.version,
                    node_id: m.node_id.clone(),
                    attachments,
                    category: m.category.canonical(),
                    due_date: m.due_date.clone(),
                    end_date: m.end_date.clone(),
                    done: m.done,
                    tags: m.tags.clone(),
                    priority: m.priority,
                    remind_before_days: m.remind_before_days,
                    remind_seen_for: m.remind_seen_for.clone(),
                }
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
                gender: p.gender,
                birth_ymd: p.birth_ymd.clone(),
                birth_hour: p.birth_hour,
            })
            .collect(),
        tasks: Vec::new(),
    };
    let plain = serde_json::to_string(&payload)?;
    let ciphertext = crypto::encrypt_string(&key, &plain)?;
    let file = BackupFile {
        format: FORMAT_V2.into(),
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
    if file.format != FORMAT_V1 && file.format != FORMAT_V2 {
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
    let _ = payload.tasks; // 忽略旧版任务
    let mut memo_attachments = Vec::with_capacity(payload.memos.len());
    let memos = payload
        .memos
        .into_iter()
        .map(|m| {
            memo_attachments.push(m.attachments);
            MemoView {
                id: m.id,
                title: m.title,
                content: m.content,
                deleted: false,
                version: m.version,
                node_id: m.node_id,
                visibility: crate::store::MemoVisibility::Private,
                owner_fp: String::new(),
                modified_at: String::new(),
                lifecycle: crate::store::MemoLifecycle::Permanent,
                deleted_at: String::new(),
                category: m.category.canonical(),
                due_date: m.due_date,
                end_date: m.end_date,
                remind_before_days: m.remind_before_days,
                remind_seen_for: m.remind_seen_for,
                done: m.done,
                tags: m.tags,
                priority: m.priority,
            }
        })
        .collect();
    Ok(BackupBundle {
        memos,
        persons: payload
            .persons
            .into_iter()
            .map(|p| PersonView {
                id: p.id,
                name: p.name,
                disabled: p.disabled,
                version: p.version,
                node_id: p.node_id,
                gender: p.gender,
                birth_ymd: p.birth_ymd,
                birth_hour: p.birth_hour,
            })
            .collect(),
        memo_attachments,
    })
}
