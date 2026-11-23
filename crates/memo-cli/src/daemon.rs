use crate::args::Args;
use crate::identity_unlock::unlock_identity;
use memo_core::config::{self, Config, NodeRole};
use memo_core::identity_keys::IdentityKeys;
use memo_core::node_lock;
use memo_sync::{start_node, NodeStartOpts};
use std::io::{self, Write};
use std::sync::Arc;

pub fn apply_no_ui_role(cfg: &mut Config, args: &Args) -> NodeRole {
    let role = args
        .role
        .map(NodeRole::from)
        .unwrap_or(NodeRole::Master);
    cfg.node_role = role;
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

pub fn run_no_ui(mut cfg: Config, args: Args) -> anyhow::Result<()> {
    let _lock = node_lock::try_acquire(std::path::Path::new(&cfg.data_dir))?;
    let role = apply_no_ui_role(&mut cfg, &args);
    let (identity, local_password) = unlock_identity(&cfg, &args)?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("memo-node")
        .build()?;
    let _enter = rt.enter();
    let started = start_node(
        cfg.clone(),
        &identity,
        &local_password,
        NodeStartOpts::storage_node(role),
    )?;
    let svc = started.svc;
    let engine = started.engine;

    eprintln!(
        "memo no-ui · 节点 {} · 角色 {} · 身份 {} ({}) · 监听 {} · 发现={} 可见={} 受理托管={} · ui={} · data_dir={}",
        cfg.node_id,
        role.as_str(),
        identity.alias,
        IdentityKeys::short_fp(&identity.fingerprint),
        cfg.listen_port,
        cfg.lan_discovery,
        cfg.node_visible,
        cfg.accept_foreign_backup,
        cfg!(feature = "ui"),
        cfg.data_dir,
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

fn run_repl(svc: &Arc<memo_core::service::MemoService>, engine: &Arc<memo_sync::SyncEngine>) {
    println!("命令: add | list | peers | discover | verify | quit");
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
