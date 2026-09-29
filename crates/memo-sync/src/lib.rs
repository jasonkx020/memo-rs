use memo_core::disk::{
    self, estimate_sync_need, probe_data_dir, DiskSpace, INCREMENTAL_MIN_FREE, SKIP_FULL_SYNC_FREE,
};
use memo_core::person::{Person, PersonStore};
use memo_core::store::{Broadcaster, MemoItem, MemoStore};
use memo_core::task::{TaskItem, TaskStore};
use memo_core::MemoService;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;

/// 局域网 UDP 发现固定端口
pub const DISCOVERY_PORT: u16 = 17000;
const DISCOVER_INTERVAL: Duration = Duration::from_secs(2);
/// 超过此时长无 Announce 则从发现表移除（Offline）
const DISCOVER_TTL: Duration = Duration::from_secs(20);
/// UDP 在线 / 不稳定阈值
const ONLINE_SECS: u64 = 8;
const STALE_SECS: u64 = 20;
const FULL_SYNC_COOLDOWN: Duration = Duration::from_secs(120);
const MAX_CONCURRENT_DIALS: usize = 4;
const MAX_PEER_WARN: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgType {
    Hello,
    MemoUpdate,
    PersonUpdate,
    TaskUpdate,
    SyncRequest,
    SyncResponse,
    SyncReject,
    PrivateBackupPush,
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
    #[serde(default)]
    pub tasks: Vec<TaskItem>,
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
}

fn default_true_announce() -> bool {
    true
}

/// UDP 成员状态（控制面）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerStatus {
    /// 最近收到 Announce
    Online,
    /// Announce 偏旧，可能丢包/休眠
    Stale,
    /// 超时未见 Announce
    Offline,
    /// 仅配置地址、从未发现
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
    /// 当前是否存在到该节点的 TCP 同步通道
    pub sync_ready: bool,
    pub disk_free: Option<u64>,
    pub disk_total: Option<u64>,
    /// 兼容旧字段：等同 sync_ready
    pub connected: bool,
    pub key_fingerprint: String,
    pub alias: String,
    pub accept_backup: bool,
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
}

struct PeerHandle {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    remote_id: RwLock<String>,
    cancel: Arc<tokio::sync::Notify>,
    /// 本连接是否由本机主动 dial
    active: bool,
}

pub struct SyncEngine {
    node_id: String,
    port: u16,
    peers_cfg: Vec<String>,
    salt_fp: String,
    lan_discovery: bool,
    node_visible: bool,
    accept_foreign_backup: bool,
    identity_fp: RwLock<String>,
    identity_alias: RwLock<String>,
    data_dir: PathBuf,
    store: Arc<MemoStore>,
    person_store: Arc<PersonStore>,
    task_store: Arc<TaskStore>,
    service: RwLock<Weak<MemoService>>,
    peers: RwLock<HashMap<String, Arc<PeerHandle>>>,
    discovered: RwLock<HashMap<String, DiscoEntry>>,
    /// addr -> (下次可拨时间, 当前退避)
    dial_backoff: RwLock<HashMap<String, (Instant, Duration)>>,
    dialing: AtomicUsize,
    /// remote_id -> 上次成功全量同步时间
    last_full_sync: RwLock<HashMap<String, Instant>>,
    sync_block_reason: RwLock<Option<String>>,
    listen_error: RwLock<Option<String>>,
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
        task_store: Arc<TaskStore>,
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
            task_store,
            cluster_salt_hex,
            lan_discovery,
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
        task_store: Arc<TaskStore>,
        cluster_salt_hex: String,
        lan_discovery: bool,
        node_visible: bool,
        accept_foreign_backup: bool,
        identity_fp: String,
        identity_alias: String,
        data_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            node_id,
            port,
            peers_cfg: peers,
            salt_fp: compute_salt_fp(&cluster_salt_hex),
            lan_discovery,
            node_visible,
            accept_foreign_backup,
            identity_fp: RwLock::new(identity_fp),
            identity_alias: RwLock::new(identity_alias),
            data_dir,
            store,
            person_store,
            task_store,
            service: RwLock::new(Weak::new()),
            peers: RwLock::new(HashMap::new()),
            discovered: RwLock::new(HashMap::new()),
            dial_backoff: RwLock::new(HashMap::new()),
            dialing: AtomicUsize::new(0),
            last_full_sync: RwLock::new(HashMap::new()),
            sync_block_reason: RwLock::new(None),
            listen_error: RwLock::new(None),
            stop: tokio::sync::Notify::new(),
        })
    }

    pub fn set_service(&self, svc: &Arc<MemoService>) {
        *self.service.write() = Arc::downgrade(svc);
    }

    fn local_disk(&self) -> DiskSpace {
        probe_data_dir(&self.data_dir)
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

    /// 返回应主动 dial 的地址列表（单边规则 + 退避）。
    fn dial_targets(&self) -> Vec<String> {
        let now = Instant::now();
        let backoff = self.dial_backoff.read();
        let mut set = HashSet::new();

        for e in self.discovered.read().values() {
            // 单边：仅较小 node_id 主动拨较大者
            if self.node_id < e.node_id {
                set.insert(e.addr.clone());
            }
        }
        for a in &self.peers_cfg {
            // 静态 peers：若已知对端 id 且本机更大，则不 dial
            let known_id = self
                .discovered
                .read()
                .values()
                .find(|e| e.addr == *a)
                .map(|e| e.node_id.clone());
            if let Some(id) = known_id {
                if self.node_id >= id {
                    continue;
                }
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

    /// UDP 已 Offline 的节点：拆掉对应 TCP，避免僵连。
    fn drop_stale_tcp(&self) {
        let now = Instant::now();
        let disco = self.discovered.read();
        let mut to_cancel = Vec::new();
        for (key, peer) in self.peers.read().iter() {
            let rid = peer.remote_id.read().clone();
            if rid.is_empty() {
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
                    let disk = self.local_disk();
                    if self.node_visible {
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
                        };
                        if let Ok(bytes) = serde_json::to_vec(&ann) {
                            let _ = sock.send_to(&bytes, broadcast).await;
                        }
                    }
                }
                res = sock.recv_from(&mut buf) => {
                    let Ok((n, from)) = res else { continue };
                    let Ok(ann) = serde_json::from_slice::<Announce>(&buf[..n]) else {
                        continue;
                    };
                    if ann.v != 2 {
                        // 旧协议不兼容：忽略
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
                        },
                    );
                }
            }
        }
    }

    fn prune_discovered(&self) {
        let now = Instant::now();
        self.discovered
            .write()
            .retain(|_, e| now.duration_since(e.last_seen) <= DISCOVER_TTL);
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
            }),
        };
        let _ = self.send_env(&peer, &hello);

        // 主动方稍后在 Hello 完成后按冷却/磁盘决定是否 SyncRequest（见 on_hello）
        if active {
            // 先发；若 Hello 判定应关闭本连接，会 cancel。磁盘过低则不发全量。
            let disk = self.local_disk();
            if disk.is_low() && disk.free_bytes < SKIP_FULL_SYNC_FREE {
                self.set_block_reason(format!(
                    "本机磁盘空间不足（可用 {}），已跳过全量同步。{}",
                    disk.format_pair(),
                    disk::disk_help_hint()
                ));
            } else {
                // SyncRequest 延后到 Hello 拿到 remote_id 后，以便应用冷却与单边去重
                // 这里仍先标记；真正发送在 maybe_request_sync
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

        // 更新发现表中的磁盘（若 Hello 带了）
        let disk_free = env
            .payload
            .get("disk_free")
            .and_then(|v| v.as_u64());
        let disk_total = env
            .payload
            .get("disk_total")
            .and_then(|v| v.as_u64());
        if let Some(e) = self.discovered.write().get_mut(&id) {
            if disk_free.is_some() {
                e.disk_free = disk_free;
            }
            if disk_total.is_some() {
                e.disk_total = disk_total;
            }
        }

        // 去重：已有到该 remote_id 的其他连接 → 关掉后建的这条
        if let Some((other_key, other)) = self.find_peer_by_remote_id(&id) {
            if other_key != conn_key {
                // 本机 node_id 更大且本连接是主动拨出 → 关掉自己，保留对方
                if self.node_id > id && peer.active {
                    peer.cancel.notify_waiters();
                    self.peers.write().remove(conn_key);
                    return;
                }
                // 否则关掉另一条（后到的或非优先）
                other.cancel.notify_waiters();
                self.peers.write().remove(&other_key);
            }
        }

        // 单边规则：本机更大却主动拨出，且对端应会拨我们 → 关掉主动连接
        if peer.active && self.node_id > id {
            peer.cancel.notify_waiters();
            self.peers.write().remove(conn_key);
            return;
        }

        *peer.remote_id.write() = id.clone();
        self.maybe_request_sync(peer, &id);
    }

    fn disk_allows_incremental(&self) -> bool {
        self.local_disk().free_bytes >= INCREMENTAL_MIN_FREE
    }

    fn on_message(&self, peer: &Arc<PeerHandle>, conn_key: &str, env: Envelope) {
        match env.msg_type {
            MsgType::Hello => {
                self.on_hello(peer, conn_key, &env);
            }
            MsgType::MemoUpdate => {
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
                if let Ok(blob) = serde_json::from_value::<memo_core::hosted::HostedBlob>(env.payload)
                {
                    if let Some(svc) = self.service.read().upgrade() {
                        match svc.ingest_hosted_blob(blob) {
                            Ok(true) => {}
                            _ => {}
                        }
                    }
                }
            }
            MsgType::PersonUpdate => {
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
                if !self.disk_allows_incremental() {
                    self.set_block_reason(format!(
                        "本机磁盘可用空间低于 128MiB，已拒绝增量同步。{}",
                        disk::disk_help_hint()
                    ));
                    return;
                }
                if let Ok(item) = serde_json::from_value::<TaskItem>(env.payload) {
                    match self.task_store.merge(item, &env.from) {
                        Ok(_) => self.notify(),
                        Err(_) => {}
                    }
                }
            }
            MsgType::SyncRequest => {
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
                        .filter(|it| {
                            matches!(it.visibility, memo_core::MemoVisibility::Public) && !it.deleted
                        })
                        .collect()
                };
                let resp_body = SyncResponse {
                    items,
                    persons: self.person_store.all(),
                    tasks: self.task_store.all(),
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
                    for it in resp.tasks {
                        match self.task_store.merge(it, &env.from) {
                            Ok(_) => notify = true,
                            Err(_) => {}
                        }
                    }
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

    /// 合并 UDP 发现 + TCP 通道 + 配置 peers 的状态列表。
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
                },
            );
        }

        // 配置的静态 peers：无发现记录时标未发现
        for a in &self.peers_cfg {
            let known = by_id.values().any(|p| p.addr == *a);
            if known {
                continue;
            }
            // 若 TCP 已连且有 remote_id，用 id；否则用地址作占位
            let mut node_id = a.clone();
            let mut sync_ready = false;
            if let Some(p) = self.peers.read().get(a) {
                sync_ready = true;
                let rid = p.remote_id.read().clone();
                if !rid.is_empty() {
                    node_id = rid;
                }
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

    fn broadcast_env(&self, env: Envelope) {
        let Ok(mut line) = serde_json::to_vec(&env) else {
            return;
        };
        line.push(b'\n');
        for p in self.engine.peers.read().values() {
            let _ = p.tx.send(line.clone());
        }
    }
}

impl Broadcaster for EngineBroadcaster {
    fn broadcast_memo(&self, item: &MemoItem) {
        let Some(svc) = self.engine.service.read().upgrade() else {
            return;
        };
        let Some(pub_item) = svc.for_public_broadcast(item) else {
            return; // 私密不走 mesh LWW
        };
        self.broadcast_env(Envelope {
            msg_type: MsgType::MemoUpdate,
            from: self.engine.node_id.clone(),
            payload: serde_json::to_value(pub_item).unwrap_or_default(),
        });
    }

    fn broadcast_person(&self, item: &Person) {
        self.broadcast_env(Envelope {
            msg_type: MsgType::PersonUpdate,
            from: self.engine.node_id.clone(),
            payload: serde_json::to_value(item).unwrap_or_default(),
        });
    }

    fn broadcast_task(&self, item: &TaskItem) {
        self.broadcast_env(Envelope {
            msg_type: MsgType::TaskUpdate,
            from: self.engine.node_id.clone(),
            payload: serde_json::to_value(item).unwrap_or_default(),
        });
    }

    fn broadcast_hosted(&self, blob: &memo_core::hosted::HostedBlob) {
        self.broadcast_env(Envelope {
            msg_type: MsgType::PrivateBackupPush,
            from: self.engine.node_id.clone(),
            payload: serde_json::to_value(blob).unwrap_or_default(),
        });
    }
}
