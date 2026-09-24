//! 分布式安全备忘录核心：配置、加密、审计、LWW 存储与服务。

pub mod audit;
pub mod backup;
pub mod config;
pub mod crypto;
pub mod export;
pub mod license;
pub mod service;
pub mod store;
pub mod verifier;

pub use config::{load_settings, save_settings, needs_restart, Config};
pub use service::{HistoryEvent, HistorySnapshot, MemoService, MemoView};
pub use store::{ConflictNotice, MemoItem};
