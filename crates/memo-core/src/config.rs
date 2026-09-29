use directories::ProjectDirs;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Argon2Params {
    pub memory: u32,
    pub iterations: u32,
    pub parallelism: u32,
    pub salt_len: u32,
    pub key_len: u32,
}

impl Default for Argon2Params {
    fn default() -> Self {
        Self {
            memory: 65536,
            iterations: 3,
            parallelism: 2,
            salt_len: 16,
            key_len: 32,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub node_id: String,
    /// 本机友好显示名（可选，仅本地展示）
    #[serde(default)]
    pub node_display_name: String,
    pub data_dir: String,
    pub listen_port: u16,
    #[serde(default)]
    pub peers: Vec<String>,
    /// 本机主密码 Argon2 盐（仅本地开库；各机可不同，与发现无关）
    pub salt_hex: String,
    /// 局域网发现/集群身份盐（同集群须一致；与主密码无关）
    #[serde(default)]
    pub cluster_salt_hex: String,
    /// 局域网 UDP 广播自动发现（端口 17000）
    #[serde(default = "default_true")]
    pub lan_discovery: bool,
    #[serde(default)]
    pub argon2: Argon2Params,
}

fn app_root() -> anyhow::Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "DistributedMemo")
        .ok_or_else(|| anyhow::anyhow!("无法解析用户配置目录"))?;
    Ok(dirs.config_dir().to_path_buf())
}

pub fn settings_path() -> anyhow::Result<PathBuf> {
    Ok(app_root()?.join("settings.json"))
}

fn random_hex(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

pub fn default_config() -> anyhow::Result<Config> {
    let root = app_root()?;
    Ok(Config {
        node_id: format!("node-{}", &random_hex(4)[..8.min(8)]),
        node_display_name: String::new(),
        data_dir: root.join("data").to_string_lossy().into_owned(),
        listen_port: 7000,
        peers: vec![],
        salt_hex: random_hex(16),
        cluster_salt_hex: random_hex(16),
        lan_discovery: true,
        argon2: Argon2Params::default(),
    })
}

fn normalize(mut cfg: Config) -> Config {
    cfg.argon2 = Argon2Params::default();
    if cfg.data_dir.trim().is_empty() {
        if let Ok(root) = app_root() {
            cfg.data_dir = root.join("data").to_string_lossy().into_owned();
        }
    }
    if cfg.listen_port == 0 {
        cfg.listen_port = 7000;
    }
    cfg.peers = cfg
        .peers
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if cfg.node_id.trim().is_empty() {
        cfg.node_id = format!("node-{}", &random_hex(4)[..8]);
    }
    if cfg.salt_hex.len() != 32 {
        cfg.salt_hex = random_hex(16);
    }
    // 旧配置无 cluster_salt_hex：兼容迁移为与 salt_hex 相同，避免已部署集群突然失联。
    // 新装则 default_config 已生成独立集群盐。
    if cfg.cluster_salt_hex.len() != 32 {
        cfg.cluster_salt_hex = if cfg.salt_hex.len() == 32 {
            cfg.salt_hex.clone()
        } else {
            random_hex(16)
        };
    }
    cfg
}

/// 从用户配置目录加载；不存在则创建默认并写入。
pub fn load_settings() -> anyhow::Result<Config> {
    let path = settings_path()?;
    if !path.exists() {
        let cfg = normalize(default_config()?);
        save_settings(&cfg)?;
        return Ok(cfg);
    }
    let raw = fs::read_to_string(&path)?;
    let needs_cluster_migrate = !raw.contains("\"cluster_salt_hex\"");
    let cfg: Config = serde_json::from_str(&raw)?;
    let cfg = normalize(cfg);
    if needs_cluster_migrate {
        // 把迁移出的 cluster_salt_hex 落盘，便于多机对齐
        let _ = save_settings(&cfg);
    }
    Ok(cfg)
}

pub fn save_settings(cfg: &Config) -> anyhow::Result<()> {
    let cfg = normalize(cfg.clone());
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&cfg)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn needs_restart(before: &Config, after: &Config) -> bool {
    before.node_id != after.node_id
        || before.data_dir != after.data_dir
        || before.listen_port != after.listen_port
        || before.salt_hex != after.salt_hex
        || before.cluster_salt_hex != after.cluster_salt_hex
        || before.lan_discovery != after.lan_discovery
}

pub fn ensure_data_dir(cfg: &Config) -> anyhow::Result<()> {
    fs::create_dir_all(Path::new(&cfg.data_dir))?;
    Ok(())
}
