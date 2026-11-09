// Windows 下默认以 GUI 子系统运行，双击不弹出 DOS 黑框。
// headless / --help / --no-ui 时再 AllocConsole。
#![cfg_attr(windows, windows_subsystem = "windows")]

use clap::{Parser, ValueEnum};
use memo_core::config::{self, Config, NodeRole};
use memo_core::identity_keys::{self, IdentityKeys};
use memo_sync::{EngineBroadcaster, SyncEngine};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "memo", about = "分布式安全备忘录 (Rust)")]
struct Args {
    /// 无 UI 守护节点（与 --no-ui 同义；服务器推荐）
    #[arg(long)]
    headless: bool,
    /// 强制无界面（服务器；Linux 无 DISPLAY 时也会自动启用）
    #[arg(long)]
    no_ui: bool,
    /// 强制图形界面（覆盖自动 no-ui 检测）
    #[arg(long)]
    gui: bool,
    /// 开机自启动：主窗隐藏，仅托盘（仍需解锁身份）
    #[arg(long)]
    tray: bool,
    /// 身份指纹（no-ui 建议指定；否则需交互选择）
    #[arg(long)]
    identity: Option<String>,
    /// .memokey 文件路径
    #[arg(long)]
    identity_file: Option<String>,
    /// 登录密码文件（一行；也可用环境变量 MEMO_LOGIN_PASSWORD）
    #[arg(long)]
    password_file: Option<PathBuf>,
    /// 无 UI 时的节点角色（默认 master）
    #[arg(long, value_enum)]
    role: Option<CliRole>,
    /// 将本次角色写回 settings.json
    #[arg(long)]
    persist_role: bool,
    /// 无 UI 时进入交互 REPL（默认常驻守护，不进 REPL）
    #[arg(long)]
    repl: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliRole {
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

#[cfg(windows)]
fn ensure_console_if_needed() {
    let need = std::env::args().any(|a| {
        matches!(
            a.as_str(),
            "--headless"
                | "--no-ui"
                | "--repl"
                | "--help"
                | "-h"
                | "--version"
                | "-V"
        )
    }) || std::env::var_os("MEMO_NO_UI").is_some();
    if need {
        unsafe {
            windows_sys::Win32::System::Console::AllocConsole();
        }
    }
}

#[cfg(not(windows))]
fn ensure_console_if_needed() {}

/// Linux：无 DISPLAY 且无 WAYLAND_DISPLAY → 视为无窗口。
/// macOS/Windows：不自动猜，需 --no-ui / MEMO_NO_UI / --headless。
fn auto_no_ui() -> bool {
    if std::env::var_os("MEMO_NO_UI").is_some() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        let display = std::env::var_os("DISPLAY").filter(|v| !v.is_empty());
        let wayland = std::env::var_os("WAYLAND_DISPLAY").filter(|v| !v.is_empty());
        return display.is_none() && wayland.is_none();
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn want_no_ui(args: &Args) -> bool {
    if args.gui {
        return false;
    }
    args.headless || args.no_ui || auto_no_ui()
}

fn read_password_prompt(prompt: &str) -> String {
    eprint!("{prompt}");
    let _ = io::stderr().flush();
    rpassword::read_password().unwrap_or_default()
}

fn read_and_confirm_local_password() -> anyhow::Result<String> {
    loop {
        let a = read_password_prompt("设置登录密码（至少 6 位）: ");
        let b = read_password_prompt("确认登录密码: ");
        match identity_keys::validate_local_password(&a, &b) {
            Ok(()) => return Ok(a.trim().to_string()),
            Err(e) => eprintln!("{e}"),
        }
    }
}

fn resolve_local_password(args: &Args) -> anyhow::Result<Option<String>> {
    if let Ok(p) = std::env::var("MEMO_LOGIN_PASSWORD") {
        let t = p.trim().to_string();
        if !t.is_empty() {
            return Ok(Some(t));
        }
    }
    if let Some(path) = args.password_file.as_ref() {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("读取密码文件失败: {e}"))?;
        let line = raw.lines().next().unwrap_or("").trim().to_string();
        if line.is_empty() {
            anyhow::bail!("密码文件为空: {}", path.display());
        }
        return Ok(Some(line));
    }
    Ok(None)
}

fn main() -> anyhow::Result<()> {
    ensure_console_if_needed();
    let args = Args::parse();
    let cfg = config::load_settings()?;
    if want_no_ui(&args) {
        run_no_ui(cfg, args)
    } else {
        memo_app::run_gui_with_opts(cfg, args.tray).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

fn apply_no_ui_role(cfg: &mut Config, args: &Args) -> NodeRole {
    let role = args
        .role
        .map(NodeRole::from)
        .unwrap_or(NodeRole::Master);
    cfg.node_role = role;
    // 服务器主机：默认对外可见并受理托管（不改用户已显式写 false 的意图——仅在本次覆盖角色；
    // 可见/托管仍尊重 settings，但若从未关掉则保持 true）
    if role.is_master() {
        // 保持 cfg 中的 accept_foreign_backup / node_visible / lan_discovery
    }
    if args.persist_role {
        let mut to_save = cfg.clone();
        to_save.node_role = role;
        if let Err(e) = config::save_settings(&to_save) {
            eprintln!("警告: 无法写回 settings: {e}");
        } else {
            eprintln!("已将 node_role={} 写入 settings", role.as_str());
        }
    }
    role
}

fn unlock_identity(
    cfg: &Config,
    args: &Args,
) -> anyhow::Result<(IdentityKeys, String)> {
    let data = PathBuf::from(&cfg.data_dir);
    let env_pass = resolve_local_password(args)?;

    if let Some(path) = args.identity_file.as_deref() {
        let p = PathBuf::from(path);
        let enc = IdentityKeys::peek_encrypted(p.as_path()).unwrap_or(false);
        let id = if enc {
            let pack_pass = if let Some(ref pw) = env_pass {
                // 非交互：旧版包口令与登录密码共用文件不合适；要求交互或 MEMO_PACK_PASSWORD
                std::env::var("MEMO_PACK_PASSWORD")
                    .unwrap_or_else(|_| pw.clone())
            } else {
                let pack_pass = read_password_prompt("旧版加密包口令: ");
                if pack_pass.trim().is_empty() {
                    anyhow::bail!("请输入旧版加密包口令");
                }
                pack_pass
            };
            IdentityKeys::import_file(p.as_path(), pack_pass.trim())?
        } else {
            IdentityKeys::import_file(p.as_path(), "")?
        };
        let local = if let Some(pw) = env_pass {
            identity_keys::validate_local_password(&pw, &pw)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            pw
        } else {
            eprintln!("请为本机设置登录密码（与密钥包无关）");
            read_and_confirm_local_password()?
        };
        return Ok((id, local));
    }

    let fp = if let Some(fp) = args.identity.as_deref() {
        fp.to_string()
    } else {
        let list = IdentityKeys::list(data.as_path())?;
        if list.is_empty() {
            anyhow::bail!("无身份。请先用 GUI 初始化，或指定 --identity / --identity-file");
        }
        if list.len() == 1 {
            eprintln!(
                "使用唯一身份 {} ({})",
                list[0].alias,
                IdentityKeys::short_fp(&list[0].fingerprint)
            );
            list[0].fingerprint.clone()
        } else if env_pass.is_some() {
            anyhow::bail!("多身份时请指定 --identity <指纹>");
        } else {
            eprintln!("可用身份:");
            for m in &list {
                eprintln!("  {}  {}", m.fingerprint, m.alias);
            }
            eprint!("输入指纹: ");
            let _ = io::stderr().flush();
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            line.trim().to_string()
        }
    };

    if IdentityKeys::needs_password_setup(data.as_path(), &fp) {
        let local = if let Some(pw) = env_pass {
            identity_keys::validate_local_password(&pw, &pw)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            pw
        } else {
            eprintln!("此身份尚未设置登录密码，请先设置：");
            read_and_confirm_local_password()?
        };
        let id = IdentityKeys::load(data.as_path(), &fp, "")?;
        Ok((id, local))
    } else {
        let local = if let Some(pw) = env_pass {
            pw
        } else {
            let local = read_password_prompt("登录密码: ");
            if local.trim().is_empty() {
                anyhow::bail!("请输入登录密码（或设置 MEMO_LOGIN_PASSWORD / --password-file）");
            }
            local.trim().to_string()
        };
        let id = IdentityKeys::load(data.as_path(), &fp, local.trim())?;
        Ok((id, local))
    }
}

fn run_no_ui(mut cfg: Config, args: Args) -> anyhow::Result<()> {
    let role = apply_no_ui_role(&mut cfg, &args);
    let (identity, local_password) = unlock_identity(&cfg, &args)?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("memo-node")
        .build()?;
    let _enter = rt.enter();
    let (svc, _) =
        memo_core::service::unlock_with_identity(cfg.clone(), &identity, &local_password)?;
    let _ = svc.restore_from_hosted();
    let _ = svc.purge_expired_trash();

    let engine = SyncEngine::new_with_identity(
        cfg.node_id.clone(),
        cfg.listen_port,
        cfg.peers.clone(),
        svc.store(),
        svc.person_store(),
        cfg.cluster_salt_hex.clone(),
        cfg.lan_discovery,
        role,
        cfg.node_visible,
        cfg.accept_foreign_backup,
        identity.fingerprint.clone(),
        identity.alias.clone(),
        PathBuf::from(&cfg.data_dir),
    );
    engine.set_service(&svc);
    engine.set_backup_targets(cfg.backup_targets.clone());
    svc.set_broadcaster(Arc::new(EngineBroadcaster::new(engine.clone())));
    engine.start();

    eprintln!(
        "memo no-ui · 节点 {} · 角色 {} · 身份 {} ({}) · 监听 {} · 发现={} 可见={} 受理托管={}",
        cfg.node_id,
        role.as_str(),
        identity.alias,
        IdentityKeys::short_fp(&identity.fingerprint),
        cfg.listen_port,
        cfg.lan_discovery,
        cfg.node_visible,
        cfg.accept_foreign_backup,
    );

    if args.repl {
        run_repl(&svc, &engine);
        engine.stop();
        return Ok(());
    }

    eprintln!("守护运行中（Ctrl+C / SIGTERM 退出）。交互调试请加 --repl");
    rt.block_on(wait_shutdown());
    eprintln!("正在停止同步引擎…");
    engine.stop();
    Ok(())
}

async fn wait_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).ok();
        let mut sigint = signal(SignalKind::interrupt()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                if let Some(s) = sigterm.as_mut() {
                    s.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
            _ = async {
                if let Some(s) = sigint.as_mut() {
                    s.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn run_repl(svc: &Arc<memo_core::service::MemoService>, engine: &Arc<SyncEngine>) {
    println!(
        "命令: add | list | peers | discover | verify | quit"
    );
    let stdin = io::stdin();
    loop {
        eprint!("> ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        match parts[0] {
            "quit" | "exit" => break,
            "list" => {
                for m in svc.list() {
                    println!(
                        "{} [{}] {}  v{}",
                        &m.id[..8.min(m.id.len())],
                        m.visibility.label(),
                        m.title,
                        m.version
                    );
                }
            }
            "add" => {
                if parts.len() < 2 {
                    println!("用法: add <标题>");
                    continue;
                }
                let title = parts[1..].join(" ");
                match svc.add(
                    &title,
                    "",
                    memo_core::store::MemoSpace::Normal,
                    memo_core::store::LanShare::Off,
                ) {
                    Ok(id) => println!("ok {id}"),
                    Err(e) => eprintln!("{e}"),
                }
            }
            "peers" => {
                for p in engine.connected_peers() {
                    println!("{p}");
                }
            }
            "discover" => {
                for p in engine.discovered_peers() {
                    println!(
                        "{} {} {} backup={} fp={}",
                        p.node_id,
                        p.status.label(),
                        p.alias,
                        p.accept_backup,
                        IdentityKeys::short_fp(&p.key_fingerprint)
                    );
                }
            }
            "verify" => {
                let (ok, detail) = svc.verify_audit();
                println!("{} {}", if ok { "ok" } else { "fail" }, detail);
            }
            other => println!("未知命令: {other}"),
        }
    }
}
