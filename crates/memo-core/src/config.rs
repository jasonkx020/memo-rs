use directories::ProjectDirs;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::identity_keys::SCHEMA_VERSION;

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

fn default_auto_lock_minutes() -> u32 {
    3
}

fn default_schema() -> u32 {
    SCHEMA_VERSION
}

fn default_node_role() -> NodeRole {
    NodeRole::Slave
}

/// 节点角色：主机广播并受理登记；从机只收听并连主机。主机同时具备向其他主机备份/同步的出站能力。
/// 外观主题偏好：浅色 / 深色 / 跟随系统 / 柔美（女性向舒心配色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
    /// 柔美：浅粉雾面、玫瑰强调色，长时间使用更柔和
    Blush,
}

impl ThemePreference {
    pub fn label(self) -> &'static str {
        match self {
            ThemePreference::System => "跟随系统",
            ThemePreference::Light => "浅色",
            ThemePreference::Dark => "深色",
            ThemePreference::Blush => "柔美",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    Master,
    #[default]
    Slave,
}

impl NodeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeRole::Master => "master",
            NodeRole::Slave => "slave",
        }
    }

    pub fn from_announce(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "master" => NodeRole::Master,
            _ => NodeRole::Slave,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NodeRole::Master => "主机",
            NodeRole::Slave => "从机",
        }
    }

    pub fn is_master(self) -> bool {
        matches!(self, NodeRole::Master)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 配置 schema；与数据目录 schema 对齐
    #[serde(default = "default_schema")]
    pub schema_version: u32,
    pub node_id: String,
    /// 本机友好显示名（可选，仅本地展示）
    #[serde(default)]
    pub node_display_name: String,
    pub data_dir: String,
    pub listen_port: u16,
    #[serde(default)]
    pub peers: Vec<String>,
    /// 遗留字段（身份模型下不再用于开库）；保留以免旧 settings 反序列化失败后被误用
    #[serde(default)]
    pub salt_hex: String,
    /// 局域网发现/集群身份盐（同集群须一致）
    #[serde(default)]
    pub cluster_salt_hex: String,
    #[serde(default = "default_true")]
    pub lan_discovery: bool,
    /// 主机 / 从机；默认从机（大规模更安全）
    #[serde(default = "default_node_role")]
    pub node_role: NodeRole,
    /// 是否对外 UDP 广播本节点（仅主机有效；从机永不广播）
    #[serde(default = "default_true")]
    pub node_visible: bool,
    /// 是否接受他节点私人密文托管
    #[serde(default = "default_true")]
    pub accept_foreign_backup: bool,
    /// 是否推送本身份私人备份
    #[serde(default = "default_true")]
    pub backup_enabled: bool,
    /// 左侧备忘列表是否显示备份状态标签
    #[serde(default = "default_true")]
    pub show_backup_status: bool,
    /// 外观：浅色 / 深色 / 跟随系统
    #[serde(default)]
    pub theme: ThemePreference,
    /// 主界面空闲多久后自动锁定（分钟）；0 = 不自动锁定。默认 3。
    #[serde(default = "default_auto_lock_minutes")]
    pub auto_lock_minutes: u32,
    /// 开机自启动（本机；Windows 写 Run 注册表）
    #[serde(default)]
    pub start_on_boot: bool,
    /// 备份目标 node_id；空 = 所有可见且 accept_backup 的在线节点
    #[serde(default)]
    pub backup_targets: Vec<String>,
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

/// 枚举可用盘符（Windows）；非 Windows 返回空。
pub fn list_drives() -> Vec<String> {
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        for c in b'C'..=b'Z' {
            let root = format!("{}:\\", c as char);
            if Path::new(&root).exists() {
                out.push(format!("{}:", c as char));
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// 默认数据盘：优先 D:，否则 C:；非 Windows 用配置根下 MemoData。
pub fn default_data_drive() -> String {
    #[cfg(windows)]
    {
        let drives = list_drives();
        if drives.iter().any(|d| d.eq_ignore_ascii_case("D:")) {
            return "D:".into();
        }
        if drives.iter().any(|d| d.eq_ignore_ascii_case("C:")) {
            return "C:".into();
        }
        "C:".into()
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

pub fn memo_data_path(drive: &str, node_id: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let drive = drive.trim().trim_end_matches('\\').trim_end_matches('/');
        let drive = if drive.ends_with(':') {
            drive.to_string()
        } else {
            format!("{drive}:")
        };
        PathBuf::from(format!("{drive}\\MemoData\\{node_id}"))
    }
    #[cfg(not(windows))]
    {
        let _ = drive;
        app_root()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("MemoData")
            .join(node_id)
    }
}

pub fn default_config() -> anyhow::Result<Config> {
    let node_id = format!("node-{}", &random_hex(4)[..8.min(8)]);
    let drive = default_data_drive();
    let data_dir = memo_data_path(&drive, &node_id)
        .to_string_lossy()
        .into_owned();
    Ok(Config {
        schema_version: SCHEMA_VERSION,
        node_id,
        node_display_name: String::new(),
        data_dir,
        listen_port: 7000,
        peers: vec![],
        salt_hex: random_hex(16),
        cluster_salt_hex: random_hex(16),
        lan_discovery: true,
        node_role: NodeRole::Slave,
        node_visible: true,
        accept_foreign_backup: true,
        backup_enabled: true,
        show_backup_status: true,
        theme: ThemePreference::System,
        auto_lock_minutes: default_auto_lock_minutes(),
        start_on_boot: false,
        backup_targets: vec![],
        argon2: Argon2Params::default(),
    })
}

fn normalize(mut cfg: Config) -> Config {
    cfg.argon2 = Argon2Params::default();
    cfg.schema_version = SCHEMA_VERSION;
    if cfg.auto_lock_minutes > 120 {
        cfg.auto_lock_minutes = 120;
    }
    if cfg.data_dir.trim().is_empty() {
        let drive = default_data_drive();
        cfg.data_dir = memo_data_path(&drive, &cfg.node_id)
            .to_string_lossy()
            .into_owned();
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
    if cfg.cluster_salt_hex.len() != 32 {
        cfg.cluster_salt_hex = random_hex(16);
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
    let cfg: Config = serde_json::from_str(&raw)?;
    let cfg = normalize(cfg);
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
        || before.cluster_salt_hex != after.cluster_salt_hex
        || before.lan_discovery != after.lan_discovery
        || before.node_role != after.node_role
        || before.node_visible != after.node_visible
}

pub fn ensure_data_dir(cfg: &Config) -> anyhow::Result<()> {
    fs::create_dir_all(Path::new(&cfg.data_dir))?;
    Ok(())
}
