use clap::{Parser, ValueEnum};
use memo_core::config::NodeRole;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "memo", about = "分布式安全备忘录 (Rust)")]
pub struct Args {
    /// 无界面（隐藏别名，请优先使用 --no-ui）
    #[arg(long, hide = true)]
    pub headless: bool,
    /// 强制无界面（服务器；Linux 无 DISPLAY 时也会自动启用）
    #[arg(long)]
    pub no_ui: bool,
    /// 强制图形界面（覆盖自动 no-ui；需编译时启用 ui 特性）
    #[arg(long)]
    pub gui: bool,
    /// 开机自启动：主窗隐藏，仅托盘（需 ui 特性）
    #[arg(long)]
    pub tray: bool,
    /// 显式初始化存储节点身份（一般不必；无身份且已设密码时会自动创建）
    #[arg(long)]
    pub init: bool,
    /// 节点显示别名（默认 storage；表示本机节点，不是终端用户）
    #[arg(long, default_value = "storage")]
    pub alias: String,
    /// 节点身份指纹（也可用环境变量 MEMO_IDENTITY）
    #[arg(long)]
    pub identity: Option<String>,
    /// .memokey 文件路径
    #[arg(long)]
    pub identity_file: Option<String>,
    /// 节点解锁密码文件（一行；也可用 MEMO_LOGIN_PASSWORD）
    #[arg(long)]
    pub password_file: Option<PathBuf>,
    /// 无 UI 时的节点角色（默认 master）
    #[arg(long, value_enum)]
    pub role: Option<CliRole>,
    /// 将本次角色写回 settings.json
    #[arg(long)]
    pub persist_role: bool,
    /// 无 UI 时进入交互 REPL（默认常驻守护）
    #[arg(long)]
    pub repl: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliRole {
    Master,
    Slave,
}

impl From<CliRole> for NodeRole {
    fn from(r: CliRole) -> Self {
        match r {
            CliRole::Master => NodeRole::Master,
            CliRole::Slave => NodeRole::Slave,
        }
    }
}
