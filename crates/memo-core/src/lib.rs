//! 分布式安全备忘录核心：配置、加密、审计、LWW 存储与服务。

pub mod audit;
pub mod backup;
pub mod config;
pub mod crypto;
pub mod cycle;
pub mod disk;
pub mod due;
pub mod enc_store;
pub mod export;
pub mod hosted;
pub mod identity_keys;
pub mod license;
pub mod male_health;
pub mod peer_acl;
pub mod person;
pub mod service;
pub mod store;
pub mod verifier;

pub use config::{
    load_settings, needs_restart, save_settings, Config, NodeRole, ThemePreference,
};
pub use peer_acl::{PeerAclEntry, PeerAclStore};
pub use cycle::{
    apply_mark_stats, analyze_period_marks, CycleConfig, CycleStore, OvulationPreset, PeriodEpisode,
    PeriodMarkStats,
};
pub use disk::{format_bytes, DiskSpace};
pub use due::{
    covers_calendar_day, days_until_due, deadline_date, display_due, due_date_part,
    due_is_due_today, due_time_hm, event_end_date, format_due, is_due_soon, is_due_today,
    is_overdue, normalize_end_date, overdue_days,
    parse_due_local, remind_at,
};
pub use identity_keys::{IdentityKeys, IdentityMeta, SCHEMA_VERSION};
pub use male_health::{MaleHealthConfig, MaleHealthStore};
pub use person::{Gender, Person, PersonView};
pub use service::{HistoryEvent, HistorySnapshot, MemoService, MemoView, TRASH_RETENTION_DAYS};
pub use store::{
    ConflictNotice, MemoCategory, MemoItem, MemoLifecycle, MemoPriority, MemoVisibility,
};
pub use hosted::BackupMetaItem;
