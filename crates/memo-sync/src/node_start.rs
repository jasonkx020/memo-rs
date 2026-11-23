//! 桌面 / 无 UI 共用的节点解锁与 SyncEngine 启动。

use crate::{EngineBroadcaster, SyncEngine};
use memo_core::config::{Config, NodeRole};
use memo_core::identity_keys::IdentityKeys;
use memo_core::service::MemoService;
use std::path::PathBuf;
use std::sync::Arc;

/// 启动选项：区分终端用户会话与纯存储节点。
#[derive(Debug, Clone)]
pub struct NodeStartOpts {
    /// 桌面用户会话：确保 Person；存储节点应关闭。
    pub ensure_session_person: bool,
    /// 覆盖 cfg 中的角色（如 no-ui 默认 Master）；None 用 cfg.node_role。
    pub role_override: Option<NodeRole>,
    pub lan_discovery: Option<bool>,
    pub node_visible: Option<bool>,
    pub accept_foreign_backup: Option<bool>,
}

impl Default for NodeStartOpts {
    fn default() -> Self {
        Self {
            ensure_session_person: true,
            role_override: None,
            lan_discovery: None,
            node_visible: None,
            accept_foreign_backup: None,
        }
    }
}

impl NodeStartOpts {
    pub fn desktop_user() -> Self {
        Self {
            ensure_session_person: true,
            ..Default::default()
        }
    }

    pub fn storage_node(role: NodeRole) -> Self {
        Self {
            ensure_session_person: false,
            role_override: Some(role),
            ..Default::default()
        }
    }
}

pub struct StartedNode {
    pub svc: Arc<MemoService>,
    pub engine: Arc<SyncEngine>,
}

/// 解锁身份并启动 SyncEngine（调用方须已 `runtime.enter()`）。
pub fn start_node(
    cfg: Config,
    identity: &IdentityKeys,
    local_password: &str,
    opts: NodeStartOpts,
) -> anyhow::Result<StartedNode> {
    let (svc, _audit) =
        memo_core::service::unlock_with_identity(cfg.clone(), identity, local_password)?;
    let _ = svc.restore_from_hosted();
    let _ = svc.purge_expired_trash();
    if opts.ensure_session_person {
        svc.ensure_session_person()?;
    }

    let role = opts.role_override.unwrap_or(cfg.node_role);
    let lan_discovery = opts.lan_discovery.unwrap_or(cfg.lan_discovery);
    let node_visible = opts.node_visible.unwrap_or(cfg.node_visible);
    let accept_foreign_backup = opts
        .accept_foreign_backup
        .unwrap_or(cfg.accept_foreign_backup);

    let engine = SyncEngine::new_with_identity(
        cfg.node_id.clone(),
        cfg.listen_port,
        cfg.peers.clone(),
        svc.store(),
        svc.person_store(),
        cfg.cluster_salt_hex.clone(),
        lan_discovery,
        role,
        node_visible,
        accept_foreign_backup,
        identity.fingerprint.clone(),
        identity.alias.clone(),
        PathBuf::from(&cfg.data_dir),
    );
    engine.set_service(&svc);
    engine.set_backup_targets(cfg.backup_targets.clone());
    svc.set_broadcaster(Arc::new(EngineBroadcaster::new(engine.clone())));
    engine.start();

    Ok(StartedNode { svc, engine })
}
