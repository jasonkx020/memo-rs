use memo_core::config::NodeRole;
use memo_core::disk::{
    self, estimate_sync_need, probe_data_dir, DiskSpace, INCREMENTAL_MIN_FREE, SKIP_FULL_SYNC_FREE,
};
use memo_core::hosted::{BackupMetaItem, HostedBlob};
use memo_core::peer_acl::{PeerAclEntry, PeerAclStore};
use memo_core::person::{Gender, Person, PersonStore};
use memo_core::store::{Broadcaster, MemoItem, MemoStore, MemoVisibility};
use memo_core::MemoService;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;

/// 局域网 UDP 发现固定端口
pub const DISCOVERY_PORT: u16 = 17000;
/// 主机广播间隔
const DISCOVER_INTERVAL: Duration = Duration::from_secs(10);
/// 超过此时长无 Announce / 刷新则从发现表移除
const DISCOVER_TTL: Duration = Duration::from_secs(60);
/// UDP 在线 / 不稳定阈值（配合 10s 广播）
const ONLINE_SECS: u64 = 25;
const STALE_SECS: u64 = 45;
const FULL_SYNC_COOLDOWN: Duration = Duration::from_secs(120);
const MAX_CONCURRENT_DIALS: usize = 4;
const MAX_PEER_WARN: usize = 16;
const UDP_PULSE_MS: u64 = 800;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgType {
    Hello,
    MemoUpdate,
    PersonUpdate,
    /// 旧版对等节点可能仍发送；本端忽略
    TaskUpdate,
    SyncRequest,
    SyncResponse,
    SyncReject,
    PrivateBackupPush,
    AclOffer,
    BackupMetaRequest,
    BackupMetaResponse,
    BackupPullRequest,
    BackupPullResponse,
    PrivateBackupPurge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    #[serde(rename = "type")]
    pub msg_type: MsgType,
    pub from: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    pub since_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub items: Vec<MemoItem>,
    #[serde(default)]
    pub persons: Vec<Person>,
    /// 旧版可能含 tasks；导入时忽略
    #[serde(default)]
    pub tasks: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReject {
    pub reason: String,
    pub need_bytes: u64,
    pub free_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Announce {
    /// 协议版本：2 = 身份密钥架构
    v: u8,
    node_id: String,
    tcp_port: u16,
    salt_fp: String,
    #[serde(default)]
    key_fingerprint: String,
    #[serde(default)]
    alias: String,
    #[serde(default = "default_true_announce")]
    accept_backup: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    disk_free: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    disk_total: Option<u64>,
    /// 当前节点选用人员性别：male / female / 空
    #[serde(default)]
    gender: String,
    /// master | slave；仅主机应广播
    #[serde(default)]
    role: String,
}

fn default_true_announce() -> bool {
    true
}

/// UDP 成员状态（控制面）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerStatus {
    Online,
    Stale,
    Offline,
    Undiscovered,
}

impl PeerStatus {
    pub fn label(self) -> &'static str {
        match self {
            PeerStatus::Online => "在线",
            PeerStatus::Stale => "不稳定",
            PeerStatus::Offline => "离线",
            PeerStatus::Undiscovered => "未发现",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub node_id: String,
    pub addr: String,
    pub last_seen_secs: u64,
    pub status: PeerStatus,
    pub sync_ready: bool,
    pub disk_free: Option<u64>,
    pub disk_total: Option<u64>,
    pub connected: bool,
    pub key_fingerprint: String,
    pub alias: String,
    pub accept_backup: bool,
    pub gender: Gender,
    pub role: NodeRole,
}

/// 节点栏 UDP 广播/收听心跳状态
#[derive(Debug, Clone)]
pub struct UdpPulse {
    pub lan_on: bool,
    pub is_master: bool,
    pub broadcasting: bool,
    pub tx_pulse: bool,
    pub rx_pulse: bool,
    pub last_tx_ago_secs: Option<u64>,
    pub last_rx_ago_secs: Option<u64>,
}

/// 左侧列表私有备份状态（相对主机托管元数据）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupUiStatus {
    Public,
    Synced,
    Pending,
    Queued,
    Offline,
}

impl BackupUiStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Public => "公开",
            Self::Synced => "已备份",
            Self::Pending => "待备份",
            Self::Queued => "备份中",
            Self::Offline => "未连接",
        }
    }
}

struct DiscoEntry {
    node_id: String,
    addr: String,
    last_seen: Instant,
    disk_free: Option<u64>,
    disk_total: Option<u64>,
    key_fingerprint: String,
    alias: String,
    accept_backup: bool,
    gender: Gender,
    role: NodeRole,
    /// 是否来自 UDP 广播（从机登记则为 false）
    #[allow(dead_code)]
    via_udp: bool,
}

struct PeerHandle {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    remote_id: RwLock<String>,
    remote_role: RwLock<NodeRole>,
    /// 对端授予本机的出站权限（AclOffer）
    offer_private: AtomicBool,
    offer_public: AtomicBool,
    cancel: Arc<tokio::sync::Notify>,
    active: bool,
}

pub struct SyncEngine {
    node_id: String,
    port: u16,
    peers_cfg: Vec<String>,
    salt_fp: String,
    lan_discovery: bool,
    node_role: NodeRole,
    node_visible: bool,
    accept_foreign_backup: bool,
    identity_fp: RwLock<String>,
    identity_alias: RwLock<String>,
    data_dir: PathBuf,
    store: Arc<MemoStore>,
    person_store: Arc<PersonStore>,
    service: RwLock<Weak<MemoService>>,
    peers: RwLock<HashMap<String, Arc<PeerHandle>>>,
    discovered: RwLock<HashMap<String, DiscoEntry>>,
    dial_backoff: RwLock<HashMap<String, (Instant, Duration)>>,
    dialing: AtomicUsize,
    last_full_sync: RwLock<HashMap<String, Instant>>,
    sync_block_reason: RwLock<Option<String>>,
    listen_error: RwLock<Option<String>>,
    acl: PeerAclStore,
    /// owner_fp -> memo_id -> 主机侧备份元数据
    backup_meta: RwLock<HashMap<String, HashMap<String, BackupMetaItem>>>,
    backup_meta_ready: RwLock<HashSet<String>>,
    pending_hosted: RwLock<Vec<HostedBlob>>,
    backup_targets: RwLock<Vec<String>>,
    last_udp_tx: RwLock<Option<Instant>>,
    last_udp_rx: RwLock<Option<Instant>>,
    udp_tx_pulse_until: RwLock<Option<Instant>>,
    udp_rx_pulse_until: RwLock<Option<Instant>>,
    stop: tokio::sync::Notify,
}

fn compute_salt_fp(cluster_salt_hex: &str) -> String {
    let hash = Sha256::digest(cluster_salt_hex.as_bytes());
    hex::encode(&hash[..8])
}

fn status_from_age(age_secs: u64) -> PeerStatus {
    if age_secs <= ONLINE_SECS {
        PeerStatus::Online
    } else if age_secs <= STALE_SECS {
        PeerStatus::Stale
    } else {
        PeerStatus::Offline
    }
}

impl SyncEngine {
    pub fn new(
        node_id: String,
        port: u16,
        peers: Vec<String>,
        store: Arc<MemoStore>,
        person_store: Arc<PersonStore>,
        cluster_salt_hex: String,
        lan_discovery: bool,
        data_dir: PathBuf,
    ) -> Arc<Self> {
        Self::new_with_identity(
            node_id,
            port,
            peers,
            store,
            person_store,
            cluster_salt_hex,
            lan_discovery,
            NodeRole::Slave,
            true,
            true,
            String::new(),
            String::new(),
            data_dir,
        )
    }

    pub fn new_with_identity(
        node_id: String,
        port: u16,
        peers: Vec<String>,
        store: Arc<MemoStore>,
        person_store: Arc<PersonStore>,
        cluster_salt_hex: String,
        lan_discovery: bool,
        node_role: NodeRole,
        node_visible: bool,
        accept_foreign_backup: bool,
        identity_fp: String,
        identity_alias: String,
        data_dir: PathBuf,
    ) -> Arc<Self> {
        let acl = {
            let _ = std::fs::create_dir_all(&data_dir);
            PeerAclStore::open(&data_dir).unwrap_or_else(|_| {
                let _ = std::fs::write(data_dir.join("peer_acl.json"), "{\n  \"entries\": []\n}");
                PeerAclStore::open(&data_dir).expect("peer_acl.json")
            })
        };

        Arc::new(Self {
            node_id,
            port,
            peers_cfg: peers,
            salt_fp: compute_salt_fp(&cluster_salt_hex),
            lan_discovery,
            node_role,
            node_visible,
            accept_foreign_backup,
            identity_fp: RwLock::new(identity_fp),
            identity_alias: RwLock::new(identity_alias),
            data_dir,
            store,
            person_store,
            service: RwLock::new(Weak::new()),
            peers: RwLock::new(HashMap::new()),
            discovered: RwLock::new(HashMap::new()),
            dial_backoff: RwLock::new(HashMap::new()),
            dialing: AtomicUsize::new(0),
            last_full_sync: RwLock::new(HashMap::new()),
            sync_block_reason: RwLock::new(None),
            listen_error: RwLock::new(None),
            acl,
            backup_meta: RwLock::new(HashMap::new()),
            backup_meta_ready: RwLock::new(HashSet::new()),
            pending_hosted: RwLock::new(Vec::new()),
            backup_targets: RwLock::new(Vec::new()),
            last_udp_tx: RwLock::new(None),
            last_udp_rx: RwLock::new(None),
            udp_tx_pulse_until: RwLock::new(None),
            udp_rx_pulse_until: RwLock::new(None),
            stop: tokio::sync::Notify::new(),
        })
    }

    pub fn set_backup_targets(&self, targets: Vec<String>) {
        *self.backup_targets.write() = targets;
    }

    fn is_backup_target(&self, node_id: &str) -> bool {
        let t = self.backup_targets.read();
        t.is_empty() || t.iter().any(|id| id == node_id)
    }

    fn hosted_dest_ok(&self, p: &PeerHandle) -> bool {
        if !p.offer_private.load(Ordering::Relaxed) {
            return false;
        }
        let id = p.remote_id.read().clone();
        if id.is_empty() {
            return self.backup_targets.read().is_empty();
        }
        self.is_backup_target(&id)
    }

    pub fn node_role(&self) -> NodeRole {
        self.node_role
    }

    pub fn udp_pulse(&self) -> UdpPulse {
        let now = Instant::now();
        let last_tx = *self.last_udp_tx.read();
        let last_rx = *self.last_udp_rx.read();
        let tx_until = *self.udp_tx_pulse_until.read();
        let rx_until = *self.udp_rx_pulse_until.read();
        UdpPulse {
            lan_on: self.lan_discovery,
            is_master: self.node_role.is_master(),
            broadcasting: self.should_broadcast_udp(),
            tx_pulse: tx_until.map(|t| now < t).unwrap_or(false),
            rx_pulse: rx_until.map(|t| now < t).unwrap_or(false),
            last_tx_ago_secs: last_tx.map(|t| now.duration_since(t).as_secs()),
            last_rx_ago_secs: last_rx.map(|t| now.duration_since(t).as_secs()),
        }
    }

    pub fn backup_status_for(
        &self,
        owner_fp: &str,
        memo_id: &str,
        version: u64,
        modified_at: &str,
        visibility: MemoVisibility,
    ) -> BackupUiStatus {
        if visibility == MemoVisibility::Public {
            return BackupUiStatus::Public;
        }
        if self
            .pending_hosted
            .read()
            .iter()
            .any(|b| b.memo_id == memo_id && b.owner_fp == owner_fp)
        {
            return BackupUiStatus::Queued;
        }
        let has_private_peer = self
            .peers
            .read()
            .values()
            .any(|p| p.offer_private.load(Ordering::Relaxed));
        let meta_ready = self.backup_meta_ready.read().contains(owner_fp);
        if !has_private_peer || !meta_ready {
            return BackupUiStatus::Offline;
        }
        let guard = self.backup_meta.read();
        if let Some(remote) = guard.get(owner_fp).and_then(|m| m.get(memo_id)) {
            if remote.version == version
                && remote.content_modified_at.trim() == modified_at.trim()
            {
                return BackupUiStatus::Synced;
            }
        }
        BackupUiStatus::Pending
    }

    fn note_udp_tx(&self) {
        let now = Instant::now();
        *self.last_udp_tx.write() = Some(now);
        *self.udp_tx_pulse_until.write() = Some(now + Duration::from_millis(UDP_PULSE_MS));
    }

    fn note_udp_rx(&self) {
        let now = Instant::now();
        *self.last_udp_rx.write() = Some(now);
        *self.udp_rx_pulse_until.write() = Some(now + Duration::from_millis(UDP_PULSE_MS));
    }

    pub fn set_service(&self, svc: &Arc<MemoService>) {
        *self.service.write() = Arc::downgrade(svc);
    }

    pub fn list_peer_acls(&self) -> Vec<PeerAclEntry> {
        self.acl.list()
    }

    pub fn set_peer_acl(
        &self,
        node_id: &str,
        allow_private_backup: bool,
        allow_public_sync: bool,
    ) -> anyhow::Result<Option<PeerAclEntry>> {
        let out = self
            .acl
            .set_flags(node_id, allow_private_backup, allow_public_sync)?;
        if let Some(ref e) = out {
            self.push_acl_offer_to(node_id, e.allow_private_backup, e.allow_public_sync);
        }
        Ok(out)
    }

    fn push_acl_offer_to(&self, remote_id: &str, allow_private: bool, allow_public: bool) {
        let Some((_, peer)) = self.find_peer_by_remote_id(remote_id) else {
            return;
        };
        let env = Envelope {
            msg_type: MsgType::AclOffer,
            from: self.node_id.clone(),
            payload: serde_json::json!({
                "allow_private_backup": allow_private,
                "allow_public_sync": allow_public,
            }),
        };
        let _ = self.send_env(&peer, &env);
    }

    fn local_disk(&self) -> DiskSpace {
        probe_data_dir(&self.data_dir)
    }

    fn should_broadcast_udp(&self) -> bool {
        self.node_role.is_master() && self.node_visible
    }

    fn push_error(&self, msg: String) {
        let mut g = self.listen_error.write();
        match g.as_mut() {
            Some(prev) => {
                prev.push_str("; ");
                prev.push_str(&msg);
            }
            None => *g = Some(msg),
        }
    }

    pub fn listen_error(&self) -> Option<String> {
        self.listen_error.read().clone()
    }

    pub fn take_sync_block_reason(&self) -> Option<String> {
        self.sync_block_reason.write().take()
    }

    pub fn peek_sync_block_reason(&self) -> Option<String> {
        self.sync_block_reason.read().clone()
    }

    fn set_block_reason(&self, msg: impl Into<String>) {
        *self.sync_block_reason.write() = Some(msg.into());
    }

    pub fn start(self: &Arc<Self>) {
        let eng = self.clone();
        tokio::spawn(async move {
            if let Err(e) = eng.clone().accept_loop().await {
                eng.push_error(format!("TCP 监听 {}: {}", eng.port, e));
            }
        });
        let eng = self.clone();
        tokio::spawn(async move {
            eng.dial_loop().await;
        });
        if self.lan_discovery {
            let eng = self.clone();
            tokio::spawn(async move {
                eng.discover_loop().await;
            });
        }
    }

    async fn accept_loop(self: Arc<Self>) -> anyhow::Result<()> {
        let listener = TcpListener::bind(("0.0.0.0", self.port)).await?;
        loop {
            tokio::select! {
                _ = self.stop.notified() => break,
                res = listener.accept() => {
                    if let Ok((stream, addr)) = res {
                        let eng = self.clone();
                        tokio::spawn(async move {
                            let _ = eng.handle_conn(stream, addr.to_string(), false).await;
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// 从机：拨所有已发现主机。主机：仅拨其他主机（较小 node_id 主动拨）。
    fn dial_targets(&self) -> Vec<String> {
        let now = Instant::now();
        let backoff = self.dial_backoff.read();
        let mut set = HashSet::new();

        for e in self.discovered.read().values() {
            if !e.role.is_master() {
                continue;
            }
            if self.is_peer_connected(&e.node_id, &e.addr) {
                continue;
            }
            match self.node_role {
                NodeRole::Slave => {
                    set.insert(e.addr.clone());
                }
                NodeRole::Master => {
                    if self.node_id < e.node_id {
                        set.insert(e.addr.clone());
                    }
                }
            }
        }

        for a in &self.peers_cfg {
            let known_role = self
                .discovered
                .read()
                .values()
                .find(|e| e.addr == *a)
                .map(|e| (e.role, e.node_id.clone()));
            if let Some((role, nid)) = known_role {
                if !role.is_master() {
                    continue;
                }
                if self.node_role.is_master() && self.node_id >= nid {
                    continue;
                }
            } else if self.node_role.is_master() {
                // 静态 peers：主机仍尝试；从机也尝试（当作主机地址）
            }
            set.insert(a.clone());
        }

        set.into_iter()
            .filter(|addr| {
                if self.is_addr_connected(addr) {
                    return false;
                }
                if let Some((next, _)) = backoff.get(addr) {
                    if now < *next {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    fn note_dial_fail(&self, addr: &str) {
        let mut map = self.dial_backoff.write();
        let next_backoff = map
            .get(addr)
            .map(|(_, d)| {
                if *d < Duration::from_secs(15) {
                    Duration::from_secs(15)
                } else if *d < Duration::from_secs(60) {
                    Duration::from_secs(60)
                } else {
                    Duration::from_secs(60)
                }
            })
            .unwrap_or(Duration::from_secs(5));
        map.insert(addr.to_string(), (Instant::now() + next_backoff, next_backoff));
    }

    fn note_dial_ok(&self, addr: &str) {
        self.dial_backoff.write().remove(addr);
    }

    async fn dial_loop(self: Arc<Self>) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = self.stop.notified() => break,
                _ = interval.tick() => {
                    self.refresh_connected_slaves();
                    self.prune_discovered();
                    self.drop_stale_tcp();
                    for addr in self.dial_targets() {
                        if self.dialing.load(Ordering::Relaxed) >= MAX_CONCURRENT_DIALS {
                            break;
                        }
                        if self.is_addr_connected(&addr) {
                            continue;
                        }
                        self.dialing.fetch_add(1, Ordering::Relaxed);
                        let eng = self.clone();
                        tokio::spawn(async move {
                            let result = TcpStream::connect(&addr).await;
                            eng.dialing.fetch_sub(1, Ordering::Relaxed);
                            match result {
                                Ok(stream) => {
                                    eng.note_dial_ok(&addr);
                                    let _ = eng.handle_conn(stream, addr, true).await;
                                }
                                Err(_) => eng.note_dial_fail(&addr),
                            }
                        });
                    }
                }
            }
        }
    }

    /// 已 TCP 连接的从机不会 UDP 广播：刷新 last_seen，避免被 prune/drop。
    fn refresh_connected_slaves(&self) {
        let peers = self.peers.read();
        let mut disco = self.discovered.write();
        for p in peers.values() {
            let rid = p.remote_id.read().clone();
            if rid.is_empty() {
                continue;
            }
            let role = *p.remote_role.read();
            if role.is_master() {
                continue;
            }
            if let Some(e) = disco.get_mut(&rid) {
                e.last_seen = Instant::now();
            }
        }
    }

    fn drop_stale_tcp(&self) {
        let now = Instant::now();
        let disco = self.discovered.read();
        let mut to_cancel = Vec::new();
        for (key, peer) in self.peers.read().iter() {
            let rid = peer.remote_id.read().clone();
            if rid.is_empty() {
                continue;
            }
            let role = *peer.remote_role.read();
            // 从机连接：不因 UDP 缺失拆线
            if !role.is_master() {
                continue;
            }
            let offline = match disco.get(&rid) {
                None => true,
                Some(e) => now.duration_since(e.last_seen) > DISCOVER_TTL,
            };
            if offline {
                to_cancel.push((key.clone(), peer.clone()));
            }
        }
        drop(disco);
        for (key, peer) in to_cancel {
            peer.cancel.notify_waiters();
            self.peers.write().remove(&key);
        }
    }

    async fn discover_loop(self: Arc<Self>) {
        let sock = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).await {
            Ok(s) => s,
            Err(e) => {
                self.push_error(format!("UDP 发现端口 {DISCOVERY_PORT} 绑定失败: {e}"));
                return;
            }
        };
        if let Err(e) = sock.set_broadcast(true) {
            self.push_error(format!("UDP 广播开启失败: {e}"));
        }

        let broadcast: SocketAddr = format!("255.255.255.255:{DISCOVERY_PORT}")
            .parse()
            .expect("broadcast addr");
        let mut buf = [0u8; 2048];
        let mut interval = tokio::time::interval(DISCOVER_INTERVAL);

        loop {
            tokio::select! {
                _ = self.stop.notified() => break,
                _ = interval.tick() => {
                    self.prune_discovered();
                    if !self.should_broadcast_udp() {
                        continue;
                    }
                    let disk = self.local_disk();
                    let gender = self
                        .service
                        .read()
                        .upgrade()
                        .map(|s| s.current_person_gender().announce_code().to_string())
                        .unwrap_or_default();
                    let ann = Announce {
                        v: 2,
                        node_id: self.node_id.clone(),
                        tcp_port: self.port,
                        salt_fp: self.salt_fp.clone(),
                        key_fingerprint: self.identity_fp.read().clone(),
                        alias: self.identity_alias.read().clone(),
                        accept_backup: self.accept_foreign_backup,
                        disk_free: Some(disk.free_bytes),
                        disk_total: Some(disk.total_bytes),
                        gender,
                        role: NodeRole::Master.as_str().into(),
                    };
                    if let Ok(bytes) = serde_json::to_vec(&ann) {
                        if sock.send_to(&bytes, broadcast).await.is_ok() {
                            self.note_udp_tx();
                        }
                    }
                }
                res = sock.recv_from(&mut buf) => {
                    let Ok((n, from)) = res else { continue };
                    let Ok(ann) = serde_json::from_slice::<Announce>(&buf[..n]) else {
                        continue;
                    };
                    if ann.v != 2 {
                        continue;
                    }
                    if ann.node_id == self.node_id {
                        continue;
                    }
                    if ann.salt_fp != self.salt_fp {
                        continue;
                    }
                    if ann.tcp_port == 0 {
                        continue;
                    }
                    let role = NodeRole::from_announce(&ann.role);
                    // 只把主机写入发现表供 dial；忽略从机广播（从机本不应发）
                    if !role.is_master() {
                        continue;
                    }
                    self.note_udp_rx();
                    let addr = format!("{}:{}", from.ip(), ann.tcp_port);
                    self.discovered.write().insert(
                        ann.node_id.clone(),
                        DiscoEntry {
                            node_id: ann.node_id,
                            addr,
                            last_seen: Instant::now(),
                            disk_free: ann.disk_free,
                            disk_total: ann.disk_total,
                            key_fingerprint: ann.key_fingerprint,
                            alias: ann.alias,
                            accept_backup: ann.accept_backup,
                            gender: Gender::from_announce(&ann.gender),
                            role: NodeRole::Master,
                            via_udp: true,
                        },
                    );
                }
            }
        }
    }

    fn prune_discovered(&self) {
        let now = Instant::now();
        let connected: HashSet<String> = self
            .peers
            .read()
            .values()
            .filter_map(|p| {
                let id = p.remote_id.read().clone();
                if id.is_empty() {
                    None
                } else {
                    Some(id)
                }
            })
            .collect();
        self.discovered.write().retain(|id, e| {
            if !e.role.is_master() && connected.contains(id) {
                return true;
            }
            now.duration_since(e.last_seen) <= DISCOVER_TTL
        });
    }

    fn is_addr_connected(&self, addr: &str) -> bool {
        self.peers.read().contains_key(addr)
    }

    fn is_peer_connected(&self, node_id: &str, addr: &str) -> bool {
        let peers = self.peers.read();
        if peers.contains_key(addr) {
            return true;
        }
        for p in peers.values() {
            if p.remote_id.read().as_str() == node_id {
                return true;
            }
        }
        false
    }

    fn find_peer_by_remote_id(&self, remote_id: &str) -> Option<(String, Arc<PeerHandle>)> {
        for (k, p) in self.peers.read().iter() {
            if p.remote_id.read().as_str() == remote_id {
                return Some((k.clone(), p.clone()));
            }
        }
        None
    }

    fn should_skip_full_sync(&self, remote_id: &str) -> bool {
        if remote_id.is_empty() {
            return false;
        }
        if let Some(t) = self.last_full_sync.read().get(remote_id) {
            if t.elapsed() < FULL_SYNC_COOLDOWN {
                return true;
            }
        }
        false
    }

    fn mark_full_sync(&self, remote_id: &str) {
        if remote_id.is_empty() {
            return;
        }
        self.last_full_sync
            .write()
            .insert(remote_id.to_string(), Instant::now());
    }

    async fn handle_conn(
        self: Arc<Self>,
        stream: TcpStream,
        key: String,
        active: bool,
    ) -> anyhow::Result<()> {
        let (reader, mut writer) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let peer = Arc::new(PeerHandle {
            tx: tx.clone(),
            remote_id: RwLock::new(String::new()),
            remote_role: RwLock::new(NodeRole::Slave),
            offer_private: AtomicBool::new(true),
            offer_public: AtomicBool::new(self.node_role.is_master()),
            cancel: Arc::new(tokio::sync::Notify::new()),
            active,
        });
        self.peers.write().insert(key.clone(), peer.clone());

        let cancel_w = peer.cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_w.notified() => break,
                    msg = rx.recv() => {
                        let Some(buf) = msg else { break };
                        if writer.write_all(&buf).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let disk = self.local_disk();
        let hello = Envelope {
            msg_type: MsgType::Hello,
            from: self.node_id.clone(),
            payload: serde_json::json!({
                "node_id": self.node_id,
                "disk_free": disk.free_bytes,
                "disk_total": disk.total_bytes,
                "role": self.node_role.as_str(),
                "alias": self.identity_alias.read().clone(),
                "key_fingerprint": self.identity_fp.read().clone(),
                "tcp_port": self.port,
            }),
        };
        let _ = self.send_env(&peer, &hello);

        if active {
            let disk = self.local_disk();
            if disk.is_low() && disk.free_bytes < SKIP_FULL_SYNC_FREE {
                self.set_block_reason(format!(
                    "本机磁盘空间不足（可用 {}），已跳过全量同步。{}",
                    disk.format_pair(),
                    disk::disk_help_hint()
                ));
            }
        }

        let mut lines = BufReader::new(reader).lines();
        loop {
            tokio::select! {
                _ = peer.cancel.notified() => break,
                line = lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if let Ok(env) = serde_json::from_str::<Envelope>(&line) {
                                self.on_message(&peer, &key, env);
                            }
                        }
                        _ => break,
                    }
                }
            }
        }
        self.peers.write().remove(&key);
        Ok(())
    }

    fn maybe_request_sync(&self, peer: &PeerHandle, remote_id: &str) {
        if !peer.active {
            return;
        }
        if !peer.offer_public.load(Ordering::Relaxed) {
            return;
        }
        let disk = self.local_disk();
        if disk.is_low() && disk.free_bytes < SKIP_FULL_SYNC_FREE {
            return;
        }
        if self.should_skip_full_sync(remote_id) {
            return;
        }
        let req = Envelope {
            msg_type: MsgType::SyncRequest,
            from: self.node_id.clone(),
            payload: serde_json::to_value(SyncRequest { since_seq: 0 }).unwrap_or_default(),
        };
        let _ = self.send_env(peer, &req);
    }

    fn send_env(&self, peer: &PeerHandle, env: &Envelope) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(env)?;
        line.push(b'\n');
        peer.tx.send(line)?;
        Ok(())
    }

    fn on_hello(&self, peer: &Arc<PeerHandle>, conn_key: &str, env: &Envelope) {
        let Some(id) = env.payload.get("node_id").and_then(|v| v.as_str()) else {
            return;
        };
        let id = id.to_string();
        let remote_role = env
            .payload
            .get("role")
            .and_then(|v| v.as_str())
            .map(NodeRole::from_announce)
            .unwrap_or(NodeRole::Slave);
        let alias = env
            .payload
            .get("alias")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let key_fp = env
            .payload
            .get("key_fingerprint")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tcp_port = env
            .payload
            .get("tcp_port")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u16;

        let disk_free = env.payload.get("disk_free").and_then(|v| v.as_u64());
        let disk_total = env.payload.get("disk_total").and_then(|v| v.as_u64());

        // 去重：同一 remote 只保留一条 TCP
        if let Some((other_key, other)) = self.find_peer_by_remote_id(&id) {
            if other_key != conn_key {
                let prefer_this = match (self.node_role, remote_role) {
                    // 从机主动连主机：优先保留本条（从机 dial）
                    (NodeRole::Slave, NodeRole::Master) if peer.active => true,
                    (NodeRole::Master, NodeRole::Slave) if !peer.active => true,
                    // 主机↔主机：较大 node_id 且主动拨出则放弃本条
                    (NodeRole::Master, NodeRole::Master)
                        if peer.active && self.node_id > id =>
                    {
                        false
                    }
                    (NodeRole::Master, NodeRole::Master) => true,
                    _ => peer.active, // 默认保留主动方
                };
                if prefer_this {
                    other.cancel.notify_waiters();
                    self.peers.write().remove(&other_key);
                } else {
                    peer.cancel.notify_waiters();
                    self.peers.write().remove(conn_key);
                    return;
                }
            }
        }

        // 主机↔主机单边：本机更大却主动拨出 → 关掉（避免双连）
        if peer.active
            && self.node_role.is_master()
            && remote_role.is_master()
            && self.node_id > id
        {
            peer.cancel.notify_waiters();
            self.peers.write().remove(conn_key);
            return;
        }

        // 主机不应主动维持「拨向从机」的连接（从机应拨主机）
        if peer.active && self.node_role.is_master() && !remote_role.is_master() {
            peer.cancel.notify_waiters();
            self.peers.write().remove(conn_key);
            return;
        }

        *peer.remote_id.write() = id.clone();
        *peer.remote_role.write() = remote_role;

        let addr = if tcp_port > 0 {
            // 尽量保留已有发现地址
            self.discovered
                .read()
                .get(&id)
                .map(|e| e.addr.clone())
                .unwrap_or_else(|| conn_key.to_string())
        } else {
            conn_key.to_string()
        };

        {
            let mut disco = self.discovered.write();
            let entry = disco.entry(id.clone()).or_insert_with(|| DiscoEntry {
                node_id: id.clone(),
                addr: addr.clone(),
                last_seen: Instant::now(),
                disk_free,
                disk_total,
                key_fingerprint: key_fp.clone(),
                alias: alias.clone(),
                accept_backup: true,
                gender: Gender::Unknown,
                role: remote_role,
                via_udp: false,
            });
            entry.last_seen = Instant::now();
            entry.role = remote_role;
            if disk_free.is_some() {
                entry.disk_free = disk_free;
            }
            if disk_total.is_some() {
                entry.disk_total = disk_total;
            }
            if !alias.is_empty() {
                entry.alias = alias.clone();
            }
            if !key_fp.is_empty() {
                entry.key_fingerprint = key_fp.clone();
            }
            if !addr.is_empty() {
                entry.addr = addr.clone();
            }
        }

        if let Ok(entry) = self.acl.upsert_seen(&id, remote_role, &alias, &key_fp, &addr) {
            let offer = Envelope {
                msg_type: MsgType::AclOffer,
                from: self.node_id.clone(),
                payload: serde_json::json!({
                    "allow_private_backup": entry.allow_private_backup,
                    "allow_public_sync": entry.allow_public_sync,
                }),
            };
            let _ = self.send_env(peer, &offer);
        }

        self.maybe_request_sync(peer, &id);
    }

    fn request_backup_meta(&self, peer: &PeerHandle) {
        let owner = self.identity_fp.read().clone();
        if owner.is_empty() {
            return;
        }
        let env = Envelope {
            msg_type: MsgType::BackupMetaRequest,
            from: self.node_id.clone(),
            payload: serde_json::json!({ "owner_fp": owner }),
        };
        let _ = self.send_env(peer, &env);
    }

    fn apply_backup_meta(&self, peer: &PeerHandle, owner_fp: &str, items: Vec<BackupMetaItem>) {
        let mut map = HashMap::new();
        for it in &items {
            map.insert(it.memo_id.clone(), it.clone());
        }
        self.backup_meta
            .write()
            .insert(owner_fp.to_string(), map);
        self.backup_meta_ready
            .write()
            .insert(owner_fp.to_string());
        self.flush_pending_hosted();
        // 本机仍有正确标题时，补推无 title_hint 的旧托管备份
        if let Some(svc) = self.service.read().upgrade() {
            if owner_fp == svc.session_fp() {
                let _ = svc.purge_expired_trash();
                let _ = svc.republish_private_backups();
            }
        }
        self.maybe_request_backup_pull(peer, owner_fp, &items);
    }

    /// 主机有、本机无或更旧 → 向对端拉取密文
    fn maybe_request_backup_pull(
        &self,
        peer: &PeerHandle,
        owner_fp: &str,
        items: &[BackupMetaItem],
    ) {
        if !peer.offer_private.load(Ordering::Relaxed) {
            return;
        }
        let Some(svc) = self.service.read().upgrade() else {
            return;
        };
        if owner_fp != svc.session_fp() {
            return;
        }
        // 本机已有托管密文时先恢复（换机 wipe 后可能仍残留，或上次半拉成功）
        if let Ok(n) = svc.restore_from_hosted() {
            if n > 0 {
                self.notify();
            }
        }
        let local_hosted: HashMap<String, BackupMetaItem> = svc
            .hosted_meta_for(owner_fp)
            .into_iter()
            .map(|m| (m.memo_id.clone(), m))
            .collect();
        let mut need: Vec<String> = Vec::new();
        for it in items {
            let store_ok = match svc.store().get(&it.memo_id) {
                None => false,
                Some(local) => {
                    if local.version > it.version {
                        true
                    } else if local.version < it.version {
                        false
                    } else {
                        // 同版本：修改时间与删除态均对齐才算够
                        let time_ok = it.content_modified_at.is_empty()
                            || local.modified_at.trim() == it.content_modified_at.trim();
                        time_ok && local.deleted == it.deleted
                    }
                }
            };
            if store_ok {
                continue;
            }
            let hosted_ok = match local_hosted.get(&it.memo_id) {
                Some(h) => h.version > it.version || (h.version == it.version && h.deleted == it.deleted),
                None => false,
            };
            if hosted_ok {
                continue;
            }
            need.push(it.memo_id.clone());
        }
        if need.is_empty() {
            return;
        }
        const BATCH: usize = 50;
        for chunk in need.chunks(BATCH) {
            let env = Envelope {
                msg_type: MsgType::BackupPullRequest,
                from: self.node_id.clone(),
                payload: serde_json::json!({
                    "owner_fp": owner_fp,
                    "memo_ids": chunk,
                }),
            };
            let _ = self.send_env(peer, &env);
        }
    }

    fn needs_hosted_push(&self, blob: &HostedBlob) -> bool {
        let ready = self.backup_meta_ready.read().contains(&blob.owner_fp);
        if !ready {
            return false;
        }
        let guard = self.backup_meta.read();
        let Some(by_memo) = guard.get(&blob.owner_fp) else {
            return true;
        };
        let Some(remote) = by_memo.get(&blob.memo_id) else {
            return true;
        };
        if remote.version != blob.version {
            return true;
        }
        let local_m = blob.content_modified_at.trim();
        let remote_m = remote.content_modified_at.trim();
        if local_m != remote_m {
            return true;
        }
        // 主机仍是旧备份（无标题）而本机已带 title_hint → 补推完整备份
        if !remote.has_title_hint && !blob.title_hint.trim().is_empty() {
            return true;
        }
        // 本机已软删而主机尚未标记 deleted → 推 tombstone
        if blob.deleted && !remote.deleted {
            return true;
        }
        // 本机已恢复而主机仍是 deleted → 推活备份
        if !blob.deleted && remote.deleted {
            return true;
        }
        false
    }

    fn queue_or_note_hosted(&self, blob: HostedBlob) {
        self.pending_hosted.write().push(blob);
    }

    fn flush_pending_hosted(&self) {
        let pending = std::mem::take(&mut *self.pending_hosted.write());
        for blob in pending {
            if !self.needs_hosted_push(&blob) {
                continue;
            }
            let env = Envelope {
                msg_type: MsgType::PrivateBackupPush,
                from: self.node_id.clone(),
                payload: serde_json::to_value(&blob).unwrap_or_default(),
            };
            let Ok(mut line) = serde_json::to_vec(&env) else {
                continue;
            };
            line.push(b'\n');
            for p in self.peers.read().values() {
                if self.hosted_dest_ok(p) {
                    let _ = p.tx.send(line.clone());
                }
            }
            self.note_hosted_acked(&blob);
        }
    }

    fn note_hosted_acked(&self, blob: &HostedBlob) {
        self.backup_meta
            .write()
            .entry(blob.owner_fp.clone())
            .or_default()
            .insert(
                blob.memo_id.clone(),
                BackupMetaItem {
                    memo_id: blob.memo_id.clone(),
                    version: blob.version,
                    content_modified_at: blob.content_modified_at.clone(),
                    backed_up_at: if blob.backed_up_at.is_empty() {
                        chrono::Local::now().to_rfc3339()
                    } else {
                        blob.backed_up_at.clone()
                    },
                    has_title_hint: !blob.title_hint.trim().is_empty(),
                    deleted: blob.deleted,
                },
            );
    }

    fn disk_allows_incremental(&self) -> bool {
        self.local_disk().free_bytes >= INCREMENTAL_MIN_FREE
    }

    fn inbound_public_allowed(&self, remote_id: &str) -> bool {
        self.acl.allow_public_sync(remote_id)
    }

    fn inbound_private_allowed(&self, remote_id: &str) -> bool {
        self.accept_foreign_backup && self.acl.allow_private_backup(remote_id)
    }

    fn on_message(&self, peer: &Arc<PeerHandle>, conn_key: &str, env: Envelope) {
        match env.msg_type {
            MsgType::Hello => {
                self.on_hello(peer, conn_key, &env);
            }
            MsgType::AclOffer => {
                let priv_ok = env
                    .payload
                    .get("allow_private_backup")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let pub_ok = env
                    .payload
                    .get("allow_public_sync")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                peer.offer_private.store(priv_ok, Ordering::Relaxed);
                peer.offer_public.store(pub_ok, Ordering::Relaxed);
                let rid = peer.remote_id.read().clone();
                if !rid.is_empty() && pub_ok {
                    self.maybe_request_sync(peer, &rid);
                }
                if priv_ok {
                    self.request_backup_meta(peer);
                }
            }
            MsgType::BackupMetaRequest => {
                let owner = env
                    .payload
                    .get("owner_fp")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if owner.is_empty() {
                    return;
                }
                let rid = peer.remote_id.read().clone();
                if !rid.is_empty() && !self.inbound_private_allowed(&rid) {
                    return;
                }
                let peer_fp = self
                    .discovered
                    .read()
                    .get(&rid)
                    .map(|e| e.key_fingerprint.clone())
                    .unwrap_or_default();
                if peer_fp.is_empty() || peer_fp != owner {
                    return;
                }
                let items = if let Some(svc) = self.service.read().upgrade() {
                    svc.hosted_meta_for(owner)
                } else {
                    Vec::new()
                };
                let resp = Envelope {
                    msg_type: MsgType::BackupMetaResponse,
                    from: self.node_id.clone(),
                    payload: serde_json::json!({
                        "owner_fp": owner,
                        "items": items,
                    }),
                };
                let _ = self.send_env(peer, &resp);
            }
            MsgType::BackupMetaResponse => {
                let owner = env
                    .payload
                    .get("owner_fp")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let items: Vec<BackupMetaItem> = env
                    .payload
                    .get("items")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                if !owner.is_empty() {
                    self.apply_backup_meta(peer, &owner, items);
                }
            }
            MsgType::BackupPullRequest => {
                let rid = peer.remote_id.read().clone();
                if rid.is_empty() || !self.inbound_private_allowed(&rid) {
                    return;
                }
                let owner = env
                    .payload
                    .get("owner_fp")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if owner.is_empty() {
                    return;
                }
                let peer_fp = self
                    .discovered
                    .read()
                    .get(&rid)
                    .map(|e| e.key_fingerprint.clone())
                    .unwrap_or_default();
                // 仅允许拉取对端自身身份下的私有备份
                if peer_fp.is_empty() || peer_fp != owner {
                    return;
                }
                let memo_ids: Vec<String> = env
                    .payload
                    .get("memo_ids")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                if memo_ids.is_empty() {
                    return;
                }
                let blobs = if let Some(svc) = self.service.read().upgrade() {
                    svc.hosted_blobs_for_ids(&owner, &memo_ids)
                } else {
                    Vec::new()
                };
                if blobs.is_empty() {
                    return;
                }
                let resp = Envelope {
                    msg_type: MsgType::BackupPullResponse,
                    from: self.node_id.clone(),
                    payload: serde_json::json!({
                        "owner_fp": owner,
                        "blobs": blobs,
                    }),
                };
                let _ = self.send_env(peer, &resp);
            }
            MsgType::BackupPullResponse => {
                if !peer.offer_private.load(Ordering::Relaxed) {
                    return;
                }
                let owner = env
                    .payload
                    .get("owner_fp")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let blobs: Vec<HostedBlob> = env
                    .payload
                    .get("blobs")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                if blobs.is_empty() {
                    return;
                }
                let Some(svc) = self.service.read().upgrade() else {
                    return;
                };
                if !owner.is_empty() && owner != svc.session_fp() {
                    return;
                }
                let mut ingested = 0usize;
                for blob in blobs {
                    if blob.owner_fp != svc.session_fp() {
                        continue;
                    }
                    match svc.ingest_hosted_blob(blob) {
                        Ok(true) => ingested += 1,
                        _ => {}
                    }
                }
                if ingested > 0 {
                    match svc.restore_from_hosted() {
                        Ok(n) if n > 0 => self.notify(),
                        _ if ingested > 0 => self.notify(),
                        _ => {}
                    }
                }
            }
            MsgType::MemoUpdate => {
                let rid = peer.remote_id.read().clone();
                if rid.is_empty() || !self.inbound_public_allowed(&rid) {
                    return;
                }
                if !self.disk_allows_incremental() {
                    self.set_block_reason(format!(
                        "本机磁盘可用空间低于 128MiB，已拒绝增量同步。{}",
                        disk::disk_help_hint()
                    ));
                    return;
                }
                if let Ok(item) = serde_json::from_value::<MemoItem>(env.payload) {
                    if let Some(svc) = self.service.read().upgrade() {
                        match svc.merge_public_remote(item, &env.from) {
                            Ok(true) => self.notify(),
                            _ => {}
                        }
                    }
                }
            }
            MsgType::PrivateBackupPush => {
                let rid = peer.remote_id.read().clone();
                let alias = self
                    .discovered
                    .read()
                    .get(&rid)
                    .map(|e| e.alias.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| rid.clone());
                if rid.is_empty() || !self.inbound_private_allowed(&rid) {
                    return;
                }
                if let Ok(blob) =
                    serde_json::from_value::<memo_core::hosted::HostedBlob>(env.payload)
                {
                    let payload_len = blob.ciphertext.len() as u64;
                    let need = estimate_sync_need(payload_len);
                    let mut disk = self.local_disk();
                    if disk.free_bytes < need || disk.free_bytes < INCREMENTAL_MIN_FREE {
                        if let Some(svc) = self.service.read().upgrade() {
                            match svc.purge_hosted_for_space(need) {
                                Ok(n) if n > 0 => {
                                    self.set_block_reason(format!(
                                        "磁盘不足，已清理 {n} 条过期/非永久托管备份后重试接收「{alias}」的备份"
                                    ));
                                    disk = self.local_disk();
                                }
                                _ => {}
                            }
                        }
                    }
                    if disk.free_bytes < need || disk.free_bytes < INCREMENTAL_MIN_FREE {
                        self.set_block_reason(format!(
                            "主机磁盘不足，已拒绝来自「{alias}」的私有备份（需要约 {}，可用 {}）。{}",
                            disk::format_bytes(need.max(INCREMENTAL_MIN_FREE)),
                            disk::format_bytes(disk.free_bytes),
                            disk::disk_help_hint()
                        ));
                        return;
                    }
                    if let Some(svc) = self.service.read().upgrade() {
                        match svc.ingest_hosted_blob(blob) {
                            Ok(true) => {}
                            _ => {}
                        }
                    }
                }
            }
            MsgType::PrivateBackupPurge => {
                let rid = peer.remote_id.read().clone();
                if rid.is_empty() || !self.inbound_private_allowed(&rid) {
                    return;
                }
                let owner = env
                    .payload
                    .get("owner_fp")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let memo_id = env
                    .payload
                    .get("memo_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if owner.is_empty() || memo_id.is_empty() {
                    return;
                }
                let peer_fp = self
                    .discovered
                    .read()
                    .get(&rid)
                    .map(|e| e.key_fingerprint.clone())
                    .unwrap_or_default();
                if peer_fp.is_empty() || peer_fp != owner {
                    return;
                }
                if let Some(svc) = self.service.read().upgrade() {
                    let _ = svc.remove_hosted_blob(&owner, &memo_id);
                    // 本地 backup_meta 同步去掉
                    if let Some(by) = self.backup_meta.write().get_mut(&owner) {
                        by.remove(&memo_id);
                    }
                }
            }
            MsgType::PersonUpdate => {
                let rid = peer.remote_id.read().clone();
                if rid.is_empty() || !self.inbound_public_allowed(&rid) {
                    return;
                }
                if !self.disk_allows_incremental() {
                    self.set_block_reason(format!(
                        "本机磁盘可用空间低于 128MiB，已拒绝增量同步。{}",
                        disk::disk_help_hint()
                    ));
                    return;
                }
                if let Ok(item) = serde_json::from_value::<Person>(env.payload) {
                    match self.person_store.merge(item, &env.from) {
                        Ok(_) => self.notify(),
                        Err(_) => {}
                    }
                }
            }
            MsgType::TaskUpdate => {
                // 旧版对等节点任务更新：忽略
            }
            MsgType::SyncRequest => {
                let rid = peer.remote_id.read().clone();
                if rid.is_empty() || !self.inbound_public_allowed(&rid) {
                    return;
                }
                let items: Vec<MemoItem> = if let Some(svc) = self.service.read().upgrade() {
                    self.store
                        .all()
                        .into_iter()
                        .filter_map(|it| svc.for_public_broadcast(&it))
                        .collect()
                } else {
                    self.store
                        .all()
                        .into_iter()
                        .filter(|it| matches!(it.visibility, memo_core::MemoVisibility::Public))
                        .map(|mut it| {
                            if it.deleted {
                                it.content.clear();
                            }
                            it
                        })
                        .collect()
                };
                let resp_body = SyncResponse {
                    items,
                    persons: self.person_store.all(),
                    tasks: Vec::new(),
                };
                let payload = serde_json::to_value(&resp_body).unwrap_or_default();
                let payload_len = serde_json::to_vec(&payload).map(|v| v.len()).unwrap_or(0) as u64;
                let need = estimate_sync_need(payload_len);
                let disk = self.local_disk();
                if disk.free_bytes < need {
                    let reject = SyncReject {
                        reason: format!(
                            "本机磁盘不足以发送全量同步（需要约 {}，可用 {}）。{}",
                            disk::format_bytes(need),
                            disk::format_bytes(disk.free_bytes),
                            disk::disk_help_hint()
                        ),
                        need_bytes: need,
                        free_bytes: disk.free_bytes,
                    };
                    let env = Envelope {
                        msg_type: MsgType::SyncReject,
                        from: self.node_id.clone(),
                        payload: serde_json::to_value(reject).unwrap_or_default(),
                    };
                    let _ = self.send_env(peer, &env);
                    self.set_block_reason(format!(
                        "因本机磁盘不足，已拒绝对端全量同步请求。{}",
                        disk::disk_help_hint()
                    ));
                    return;
                }
                let resp = Envelope {
                    msg_type: MsgType::SyncResponse,
                    from: self.node_id.clone(),
                    payload,
                };
                let _ = self.send_env(peer, &resp);
            }
            MsgType::SyncResponse => {
                let rid = peer.remote_id.read().clone();
                if rid.is_empty() || !self.inbound_public_allowed(&rid) {
                    return;
                }
                let payload_len =
                    serde_json::to_vec(&env.payload).map(|v| v.len()).unwrap_or(0) as u64;
                let need = estimate_sync_need(payload_len);
                let disk = self.local_disk();
                if disk.free_bytes < need {
                    self.set_block_reason(format!(
                        "本机磁盘不足以接收全量同步（需要约 {}，可用 {}），已跳过合并。{}",
                        disk::format_bytes(need),
                        disk::format_bytes(disk.free_bytes),
                        disk::disk_help_hint()
                    ));
                    return;
                }
                if let Ok(resp) = serde_json::from_value::<SyncResponse>(env.payload) {
                    let mut notify = false;
                    if let Some(svc) = self.service.read().upgrade() {
                        for it in resp.items {
                            match svc.merge_public_remote(it, &env.from) {
                                Ok(true) => notify = true,
                                _ => {}
                            }
                        }
                    }
                    for it in resp.persons {
                        match self.person_store.merge(it, &env.from) {
                            Ok(_) => notify = true,
                            Err(_) => {}
                        }
                    }
                    let _ = resp.tasks; // 忽略旧版任务
                    if notify {
                        self.mark_full_sync(&env.from);
                        self.notify();
                    }
                }
            }
            MsgType::SyncReject => {
                if let Ok(rej) = serde_json::from_value::<SyncReject>(env.payload) {
                    self.set_block_reason(format!(
                        "对端 {} 拒绝全量同步：{}（需要 {}，对端可用 {}）",
                        env.from,
                        rej.reason,
                        disk::format_bytes(rej.need_bytes),
                        disk::format_bytes(rej.free_bytes)
                    ));
                }
            }
        }
    }

    fn notify(&self) {
        if let Some(svc) = self.service.read().upgrade() {
            svc.notify_updated();
        }
    }

    pub fn connected_peers(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (k, p) in self.peers.read().iter() {
            let id = p.remote_id.read().clone();
            let name = if id.is_empty() { k.clone() } else { id };
            if seen.insert(name.clone()) {
                out.push(name);
            }
        }
        out
    }

    pub fn peer_statuses(&self) -> Vec<DiscoveredPeer> {
        self.prune_discovered();
        let now = Instant::now();
        let mut by_id: HashMap<String, DiscoveredPeer> = HashMap::new();

        for e in self.discovered.read().values() {
            let age = now.duration_since(e.last_seen).as_secs();
            let sync_ready = self.is_peer_connected(&e.node_id, &e.addr);
            by_id.insert(
                e.node_id.clone(),
                DiscoveredPeer {
                    node_id: e.node_id.clone(),
                    addr: e.addr.clone(),
                    last_seen_secs: age,
                    status: status_from_age(age),
                    sync_ready,
                    disk_free: e.disk_free,
                    disk_total: e.disk_total,
                    connected: sync_ready,
                    key_fingerprint: e.key_fingerprint.clone(),
                    alias: e.alias.clone(),
                    accept_backup: e.accept_backup,
                    gender: e.gender,
                    role: e.role,
                },
            );
        }

        for a in &self.peers_cfg {
            let known = by_id.values().any(|p| p.addr == *a);
            if known {
                continue;
            }
            let mut node_id = a.clone();
            let mut sync_ready = false;
            let mut role = NodeRole::Master;
            if let Some(p) = self.peers.read().get(a) {
                sync_ready = true;
                let rid = p.remote_id.read().clone();
                if !rid.is_empty() {
                    node_id = rid;
                }
                role = *p.remote_role.read();
            }
            if by_id.contains_key(&node_id) {
                continue;
            }
            by_id.insert(
                node_id.clone(),
                DiscoveredPeer {
                    node_id,
                    addr: a.clone(),
                    last_seen_secs: u64::MAX / 4,
                    status: PeerStatus::Undiscovered,
                    sync_ready,
                    disk_free: None,
                    disk_total: None,
                    connected: sync_ready,
                    key_fingerprint: String::new(),
                    alias: String::new(),
                    accept_backup: true,
                    gender: Gender::Unknown,
                    role,
                },
            );
        }

        let mut out: Vec<_> = by_id.into_values().collect();
        out.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        out
    }

    pub fn discovered_peers(&self) -> Vec<DiscoveredPeer> {
        self.peer_statuses()
    }

    pub fn online_count(&self) -> usize {
        self.peer_statuses()
            .iter()
            .filter(|p| p.status == PeerStatus::Online)
            .count()
    }

    pub fn sync_ready_count(&self) -> usize {
        self.peer_statuses().iter().filter(|p| p.sync_ready).count()
    }

    pub fn peer_count_warning(&self) -> bool {
        self.peer_statuses().len() >= MAX_PEER_WARN
    }

    pub fn local_disk_space(&self) -> DiskSpace {
        self.local_disk()
    }

    pub fn stop(&self) {
        self.stop.notify_waiters();
    }
}

pub struct EngineBroadcaster {
    engine: Arc<SyncEngine>,
}

impl EngineBroadcaster {
    pub fn new(engine: Arc<SyncEngine>) -> Self {
        Self { engine }
    }

    fn broadcast_env_filtered<F>(&self, env: Envelope, mut allow: F)
    where
        F: FnMut(&PeerHandle) -> bool,
    {
        let Ok(mut line) = serde_json::to_vec(&env) else {
            return;
        };
        line.push(b'\n');
        for p in self.engine.peers.read().values() {
            if allow(p) {
                let _ = p.tx.send(line.clone());
            }
        }
    }
}

impl Broadcaster for EngineBroadcaster {
    fn broadcast_memo(&self, item: &MemoItem) {
        let Some(svc) = self.engine.service.read().upgrade() else {
            return;
        };
        let Some(pub_item) = svc.for_public_broadcast(item) else {
            return;
        };
        self.broadcast_env_filtered(
            Envelope {
                msg_type: MsgType::MemoUpdate,
                from: self.engine.node_id.clone(),
                payload: serde_json::to_value(pub_item).unwrap_or_default(),
            },
            |p| p.offer_public.load(Ordering::Relaxed),
        );
    }

    fn broadcast_person(&self, item: &Person) {
        self.broadcast_env_filtered(
            Envelope {
                msg_type: MsgType::PersonUpdate,
                from: self.engine.node_id.clone(),
                payload: serde_json::to_value(item).unwrap_or_default(),
            },
            |p| p.offer_public.load(Ordering::Relaxed),
        );
    }

    fn broadcast_hosted(&self, blob: &memo_core::hosted::HostedBlob) {
        let blob = blob.clone();
        if !self.engine.backup_meta_ready.read().contains(&blob.owner_fp) {
            self.engine.queue_or_note_hosted(blob);
            // 向已授权连接催一次元数据
            for p in self.engine.peers.read().values() {
                if p.offer_private.load(Ordering::Relaxed) {
                    self.engine.request_backup_meta(p);
                }
            }
            return;
        }
        if !self.engine.needs_hosted_push(&blob) {
            return;
        }
        self.broadcast_env_filtered(
            Envelope {
                msg_type: MsgType::PrivateBackupPush,
                from: self.engine.node_id.clone(),
                payload: serde_json::to_value(&blob).unwrap_or_default(),
            },
            |p| self.engine.hosted_dest_ok(p),
        );
        self.engine.note_hosted_acked(&blob);
    }

    fn broadcast_hosted_purge(&self, owner_fp: &str, memo_id: &str, version: u64) {
        self.broadcast_env_filtered(
            Envelope {
                msg_type: MsgType::PrivateBackupPurge,
                from: self.engine.node_id.clone(),
                payload: serde_json::json!({
                    "owner_fp": owner_fp,
                    "memo_id": memo_id,
                    "version": version,
                }),
            },
            |p| self.engine.hosted_dest_ok(p),
        );
        if let Some(by) = self.engine.backup_meta.write().get_mut(owner_fp) {
            by.remove(memo_id);
        }
    }
}
