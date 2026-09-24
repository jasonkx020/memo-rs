// Windows 下默认以 GUI 子系统运行，双击不弹出 DOS 黑框。
// headless / --help 时再 AllocConsole。
#![cfg_attr(windows, windows_subsystem = "windows")]

use clap::Parser;
use memo_core::config::{self, Config};
use memo_sync::{EngineBroadcaster, SyncEngine};
use std::io::{self, Write};
use std::sync::Arc;
use zeroize::Zeroize;

#[derive(Parser, Debug)]
#[command(name = "memo", about = "分布式安全备忘录 (Rust)")]
struct Args {
    /// 无 UI 守护节点（REPL）
    #[arg(long)]
    headless: bool,
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
        run_headless(cfg)
    } else {
        memo_app::run_gui(cfg).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

fn run_headless(cfg: Config) -> anyhow::Result<()> {
    eprint!("请输入主密码: ");
    let _ = io::stderr().flush();
    let mut password = rpassword::read_password()?;
    if password.is_empty() {
        anyhow::bail!("主密码不能为空");
    }

    let rt = tokio::runtime::Runtime::new()?;
    let _enter = rt.enter();
    let (svc, _) = memo_core::service::unlock(cfg.clone(), password.as_bytes())?;
    password.zeroize();

    let lic = if let Ok(p) = config::settings_path() {
        p.parent()
            .map(memo_core::license::load_status)
            .unwrap_or(memo_core::license::LicenseStatus::Community)
    } else {
        memo_core::license::LicenseStatus::Community
    };
    let sync_enabled = lic.allows_lan_sync();

    let engine = SyncEngine::new(
        cfg.node_id.clone(),
        cfg.listen_port,
        cfg.peers.clone(),
        svc.store(),
        cfg.salt_hex.clone(),
        cfg.lan_discovery && sync_enabled,
    );
    engine.set_service(&svc);
    if sync_enabled {
        svc.set_broadcaster(Arc::new(EngineBroadcaster::new(engine.clone())));
        engine.start();
        println!(
            "headless · 节点 {} · 同步已启用（{}）",
            cfg.node_id,
            memo_core::license::status_label(&lic)
        );
    } else {
        println!(
            "headless · 节点 {} · 社区版仅本机（{}）",
            cfg.node_id,
            memo_core::license::status_label(&lic)
        );
    }
    println!("命令: add | list | edit | del | export | verify | peers | discover | quit");
    if let Ok(p) = config::settings_path() {
        println!("设置: {}", p.display());
    }

    let stdin = io::stdin();
    loop {
        print!("> ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts[0] {
            "quit" | "exit" => break,
            "add" => {
                let title = parts.get(1).copied().unwrap_or("");
                let content = parts.get(2..).map(|s| s.join(" ")).unwrap_or_default();
                match svc.add(title, &content) {
                    Ok(id) => println!("id: {id}"),
                    Err(e) => println!("err: {e}"),
                }
            }
            "list" => {
                for v in svc.list() {
                    let preview: String = v.content.chars().take(40).collect();
                    println!(
                        "{}\t{}\tv{}\t{preview}",
                        &v.id[..8.min(v.id.len())],
                        v.title,
                        v.version
                    );
                }
            }
            "edit" => {
                if parts.len() < 3 {
                    println!("用法: edit <id> <title>");
                    continue;
                }
                let id = parts[1];
                let title = parts[2..].join(" ");
                print!("新内容: ");
                let _ = io::stdout().flush();
                let mut body = String::new();
                stdin.read_line(&mut body)?;
                match svc.edit(id, &title, body.trim_end()) {
                    Ok(()) => println!("ok"),
                    Err(e) => println!("err: {e}"),
                }
            }
            "del" | "delete" => {
                if parts.len() < 2 {
                    println!("用法: del <id>");
                    continue;
                }
                eprint!("请输入主密码以确认删除: ");
                let _ = io::stderr().flush();
                let mut pw = rpassword::read_password()?;
                match svc.delete(parts[1], &pw) {
                    Ok(()) => println!("ok"),
                    Err(e) => println!("err: {e}"),
                }
                pw.zeroize();
            }
            "verify" => {
                let (ok, detail) = svc.verify_audit();
                if ok {
                    println!("审计: 完整");
                } else {
                    println!("审计失败: {detail}");
                }
            }
            "peers" => println!("{:?}", engine.connected_peers()),
            "discover" => {
                let list = engine.discovered_peers();
                if list.is_empty() {
                    println!("(无发现节点)");
                } else {
                    for d in list {
                        let st = if d.connected { "在线" } else { "已发现" };
                        println!(
                            "{}\t{}\t{}\t{}s前",
                            d.node_id, d.addr, st, d.last_seen_secs
                        );
                    }
                }
                if let Some(err) = engine.listen_error() {
                    println!("listen_error: {err}");
                }
            }
            "export" => {
                if parts.len() < 2 {
                    println!("用法: export <path.txt>");
                    continue;
                }
                eprint!("请输入主密码以确认导出: ");
                let _ = io::stderr().flush();
                let mut pw = rpassword::read_password()?;
                match svc.verify_password(&pw) {
                    Ok(()) => match svc.export_txt(parts[1].into(), None) {
                        Ok(()) => println!("ok: {}", parts[1]),
                        Err(e) => println!("err: {e}"),
                    },
                    Err(e) => println!("err: {e}"),
                }
                pw.zeroize();
            }
            _ => println!("未知命令"),
        }
    }
    engine.stop();
    Ok(())
}
