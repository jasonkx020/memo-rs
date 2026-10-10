// Windows 下默认以 GUI 子系统运行，双击不弹出 DOS 黑框。
// headless / --help 时再 AllocConsole。
#![cfg_attr(windows, windows_subsystem = "windows")]

use clap::Parser;
use memo_core::config::{self, Config};
use memo_core::identity_keys::IdentityKeys;
use memo_core::store::MemoVisibility;
use memo_sync::{EngineBroadcaster, SyncEngine};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "memo", about = "分布式安全备忘录 (Rust)")]
struct Args {
    /// 无 UI 守护节点（REPL）
    #[arg(long)]
    headless: bool,
    /// 开机自启动：主窗隐藏，仅托盘（仍需解锁身份）
    #[arg(long)]
    tray: bool,
    /// 身份指纹（headless 必填，或配合 --identity-file）
    #[arg(long)]
    identity: Option<String>,
    /// .memokey 文件路径
    #[arg(long)]
    identity_file: Option<String>,
}

#[cfg(windows)]
fn ensure_console_if_needed() {
    let need = std::env::args().any(|a| {
        matches!(
            a.as_str(),
            "--headless" | "--help" | "-h" | "--version" | "-V"
        )
    });
    if need {
        unsafe {
            windows_sys::Win32::System::Console::AllocConsole();
        }
    }
}

#[cfg(not(windows))]
fn ensure_console_if_needed() {}

fn main() -> anyhow::Result<()> {
    ensure_console_if_needed();
    let args = Args::parse();
    let cfg = config::load_settings()?;
    if args.headless {
        run_headless(cfg, args)
    } else {
        memo_app::run_gui_with_opts(cfg, args.tray).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

fn run_headless(cfg: Config, args: Args) -> anyhow::Result<()> {
    let identity = if let Some(path) = args.identity_file.as_deref() {
        eprint!("若密钥包有保险口令请输入（可空）: ");
        let _ = io::stderr().flush();
        let pass = rpassword::read_password().unwrap_or_default();
        IdentityKeys::import_file(PathBuf::from(path).as_path(), &pass)?
    } else if let Some(fp) = args.identity.as_deref() {
        IdentityKeys::load(PathBuf::from(&cfg.data_dir).as_path(), fp)?
    } else {
        let list = IdentityKeys::list(PathBuf::from(&cfg.data_dir).as_path())?;
        if list.is_empty() {
            anyhow::bail!("无身份。请先用 GUI 初始化，或指定 --identity / --identity-file");
        }
        eprintln!("可用身份:");
        for m in &list {
            eprintln!("  {}  {}", m.fingerprint, m.alias);
        }
        eprint!("输入指纹: ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        IdentityKeys::load(
            PathBuf::from(&cfg.data_dir).as_path(),
            line.trim(),
        )?
    };

    let rt = tokio::runtime::Runtime::new()?;
    let _enter = rt.enter();
    let (svc, _) = memo_core::service::unlock_with_identity(cfg.clone(), &identity)?;
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
        cfg.node_role,
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

    println!(
        "headless · 节点 {} · 身份 {} ({})\n命令: add | list | edit | del | export | verify | peers | discover | quit",
        cfg.node_id,
        identity.alias,
        IdentityKeys::short_fp(&identity.fingerprint)
    );

    let stdin = io::stdin();
    loop {
        eprint!("> ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
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
                match svc.add(&title, "", MemoVisibility::Private) {
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
    Ok(())
}
