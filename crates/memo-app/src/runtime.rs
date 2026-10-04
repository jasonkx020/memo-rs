//! 解锁后的运行时句柄。

use memo_core::config::Config;
use memo_core::service::MemoService;
use memo_sync::SyncEngine;
use std::sync::Arc;

pub struct AppRuntime {
    pub svc: Arc<MemoService>,
    pub engine: Arc<SyncEngine>,
    pub cfg: Config,
    /// 保持 tokio 运行时存活（同步引擎依赖它）
    pub _rt: tokio::runtime::Runtime,
}
