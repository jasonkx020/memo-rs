use memo_core::store::{Broadcaster, MemoItem, MemoStore};
use memo_core::MemoService;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;

/// 局域网 UDP 发现固定端口
pub const DISCOVERY_PORT: u16 = 17000;
const DISCOVER_INTERVAL: Duration = Duration::from_secs(2);
const DISCOVER_TTL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgType {
    Hello,
    MemoUpdate,
    SyncRequest,
    SyncResponse,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Announce {
    v: u8,
    node_id: String,
    tcp_port: u16,
    salt_fp: String,
}

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub node_id: String,
    pub addr: String,
    /// 距上次收到广播的秒数
    pub last_seen_secs: u64,
    pub connected: bool,
}

struct DiscoEntry {
    node_id: String,
    addr: String,
    last_seen: Instant,
}

struct PeerHandle {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    remote_id: RwLock<String>,
}

pub struct SyncEngine {
    node_id: String,
    port: u16,
    peers_cfg: Vec<String>,
    salt_fp: String,
    lan_discovery: bool,
    store: Arc<MemoStore>,
    service: RwLock<Weak<MemoService>>,
    peers: RwLock<HashMap<String, Arc<PeerHandle>>>,
    discovered: RwLock<HashMap<String, DiscoEntry>>,
    listen_error: RwLock<Option<String>>,
    stop: tokio::sync::Notify,
}

fn compute_salt_fp(salt_hex: &str) -> String {
    let hash = Sha256::digest(salt_hex.as_bytes());
    hex::encode(&hash[..8])
}

impl SyncEngine {
    pub fn new(
        node_id: String,
        port: u16,
        peers: Vec<String>,
        store: Arc<MemoStore>,
        salt_hex: String,
        lan_discovery: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            node_id,
            port,
            peers_cfg: peers,
            salt_fp: compute_salt_fp(&salt_hex),
            lan_discovery,
            store,
            service: RwLock::new(Weak::new()),
            peers: RwLock::new(HashMap::new()),
            discovered: RwLock::new(HashMap::new()),
            listen_error: RwLock::new(None),
            stop: tokio::sync::Notify::new(),
        })
    }

    pub fn set_service(&self, svc: &Arc<MemoService>) {
        *self.service.write() = Arc::downgrade(svc);
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

    fn dial_targets(&self) -> Vec<String> {
        let mut set = HashSet::new();
        for a in &self.peers_cfg {
            set.insert(a.clone());
        }
        for e in self.discovered.read().values() {
            set.insert(e.addr.clone());
        }
        set.into_iter().collect()
    }

    async fn dial_loop(self: Arc<Self>) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = self.stop.notified() => break,
                _ = interval.tick() => {
                    for addr in self.dial_targets() {
                        if self.is_addr_connected(&addr) {
                            continue;
                        }
                        let eng = self.clone();
                        tokio::spawn(async move {
                            if let Ok(stream) = TcpStream::connect(&addr).await {
                                let _ = eng.handle_conn(stream, addr, true).await;
                            }
                        });
                    }
                }
            }
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
        let mut buf = [0u8; 1024];
        let mut interval = tokio::time::interval(DISCOVER_INTERVAL);

        loop {
            tokio::select! {
                _ = self.stop.notified() => break,
                _ = interval.tick() => {
                    self.prune_discovered();
                    let ann = Announce {
                        v: 1,
                        node_id: self.node_id.clone(),
                        tcp_port: self.port,
                        salt_fp: self.salt_fp.clone(),
                    };
                    if let Ok(bytes) = serde_json::to_vec(&ann) {
                        let _ = sock.send_to(&bytes, broadcast).await;
                    }
                }
                res = sock.recv_from(&mut buf) => {
                    let Ok((n, from)) = res else { continue };
                    let Ok(ann) = serde_json::from_slice::<Announce>(&buf[..n]) else {
                        continue;
                    };
                    if ann.v != 1 {
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
        let peers = self.peers.read();
        if peers.contains_key(addr) {
            return true;
        }
        false
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
        });
        self.peers.write().insert(key.clone(), peer.clone());

        tokio::spawn(async move {
            while let Some(buf) = rx.recv().await {
                if writer.write_all(&buf).await.is_err() {
                    break;
                }
            }
        });

        let hello = Envelope {
            msg_type: MsgType::Hello,
            from: self.node_id.clone(),
            payload: serde_json::json!({ "node_id": self.node_id }),
        };
        let _ = self.send_env(&peer, &hello);
        if active {
            let req = Envelope {
                msg_type: MsgType::SyncRequest,
                from: self.node_id.clone(),
                payload: serde_json::to_value(SyncRequest { since_seq: 0 })?,
            };
            let _ = self.send_env(&peer, &req);
        }

        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(env) = serde_json::from_str::<Envelope>(&line) {
                self.on_message(&peer, env);
            }
        }
        self.peers.write().remove(&key);
        Ok(())
    }

    fn send_env(&self, peer: &PeerHandle, env: &Envelope) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(env)?;
        line.push(b'\n');
        peer.tx.send(line)?;
        Ok(())
    }

    fn on_message(&self, peer: &Arc<PeerHandle>, env: Envelope) {
        match env.msg_type {
            MsgType::Hello => {
                if let Some(id) = env.payload.get("node_id").and_then(|v| v.as_str()) {
                    *peer.remote_id.write() = id.to_string();
                }
            }
            MsgType::MemoUpdate => {
                if let Ok(item) = serde_json::from_value::<MemoItem>(env.payload) {
                    match self.store.merge(item, &env.from) {
                        Ok(true) => self.notify(),
                        Ok(false) => self.notify(), // 冲突也通知 UI 拉取
                        Err(_) => {}
                    }
                }
            }
            MsgType::SyncRequest => {
                let items = self.store.all();
                let resp = Envelope {
                    msg_type: MsgType::SyncResponse,
                    from: self.node_id.clone(),
                    payload: serde_json::to_value(SyncResponse { items }).unwrap_or_default(),
                };
                let _ = self.send_env(peer, &resp);
            }
            MsgType::SyncResponse => {
                if let Ok(resp) = serde_json::from_value::<SyncResponse>(env.payload) {
                    let mut notify = false;
                    for it in resp.items {
                        match self.store.merge(it, &env.from) {
                            Ok(true) => notify = true,
                            Ok(false) => notify = true,
                            Err(_) => {}
                        }
                    }
                    if notify {
                        self.notify();
                    }
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

    pub fn discovered_peers(&self) -> Vec<DiscoveredPeer> {
        self.prune_discovered();
        let now = Instant::now();
        let mut out: Vec<_> = self
            .discovered
            .read()
            .values()
            .map(|e| DiscoveredPeer {
                node_id: e.node_id.clone(),
                addr: e.addr.clone(),
                last_seen_secs: now.duration_since(e.last_seen).as_secs(),
                connected: self.is_peer_connected(&e.node_id, &e.addr),
            })
            .collect();
        out.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        out
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
}

impl Broadcaster for EngineBroadcaster {
    fn broadcast_memo(&self, item: &MemoItem) {
        let env = Envelope {
            msg_type: MsgType::MemoUpdate,
            from: self.engine.node_id.clone(),
            payload: serde_json::to_value(item).unwrap_or_default(),
        };
        let Ok(mut line) = serde_json::to_vec(&env) else {
            return;
        };
        line.push(b'\n');
        for p in self.engine.peers.read().values() {
            let _ = p.tx.send(line.clone());
        }
    }
}
