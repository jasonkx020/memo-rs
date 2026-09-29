//! 分布式安全备忘录核心：配置、加密、审计、LWW 存储与服务。

pub mod audit;
pub mod backup;
pub mod config;
pub mod crypto;
pub mod disk;
pub mod enc_store;
pub mod export;
pub mod hosted;
pub mod identity_keys;
pub mod license;
pub mod person;
pub mod service;
pub mod store;
pub mod task;
pub mod verifier;

pub use config::{load_settings, needs_restart, save_settings, Config};
pub use disk::{format_bytes, DiskSpace};
pub use identity_keys::{IdentityKeys, IdentityMeta, SCHEMA_VERSION};
pub use person::{Person, PersonView};
pub use service::{HistoryEvent, HistorySnapshot, MemoService, MemoView};
pub use store::{ConflictNotice, MemoItem, MemoVisibility};
pub use task::{TaskItem, TaskStatus, TaskView};
