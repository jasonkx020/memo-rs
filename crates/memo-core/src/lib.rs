//! 分布式安全备忘录核心：配置、加密、审计、LWW 存储与服务。

pub mod audit;
pub mod backup;
pub mod config;
pub mod crypto;
pub mod cycle;
pub mod disk;
pub mod enc_store;
pub mod export;
pub mod hosted;
pub mod identity_keys;
pub mod license;
pub mod peer_acl;
pub mod person;
pub mod service;
pub mod store;
pub mod task;
pub mod verifier;

pub use config::{
    load_settings, needs_restart, save_settings, Config, NodeRole, ThemePreference,
};
pub use peer_acl::{PeerAclEntry, PeerAclStore};
pub use cycle::{CycleConfig, CycleStore};
pub use disk::{format_bytes, DiskSpace};
pub use identity_keys::{IdentityKeys, IdentityMeta, SCHEMA_VERSION};
pub use person::{Gender, Person, PersonView};
pub use service::{HistoryEvent, HistorySnapshot, MemoService, MemoView, TRASH_RETENTION_DAYS};
pub use store::{ConflictNotice, MemoItem, MemoLifecycle, MemoVisibility};
pub use hosted::BackupMetaItem;
pub use task::{TaskItem, TaskKind, TaskStatus, TaskView};
