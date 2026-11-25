use crate::args::Args;
use mijian_core::config::Config;
use mijian_core::identity_keys::{self, IdentityKeys};
use std::io::{self, Write};
use std::path::PathBuf;

fn read_password_prompt(prompt: &str) -> String {
    eprint!("{prompt}");
    let _ = io::stderr().flush();
    rpassword::read_password().unwrap_or_default()
}

fn read_and_confirm_node_password() -> anyhow::Result<String> {
    loop {
        let a = read_password_prompt("设置节点解锁密码（至少 6 位）: ");
        let b = read_password_prompt("确认节点解锁密码: ");
        match identity_keys::validate_local_password(&a, &b) {
            Ok(()) => return Ok(a.trim().to_string()),
            Err(e) => eprintln!("{e}"),
        }
    }
}

pub fn resolve_local_password(args: &Args) -> anyhow::Result<Option<String>> {
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

fn resolve_identity_fp(args: &Args) -> Option<String> {
    args.identity
        .clone()
        .or_else(|| {
            std::env::var("MEMO_IDENTITY")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
}

/// 生成存储/主节点机器身份（非终端用户账号）。
fn bootstrap_node_identity(
    cfg: &Config,
    args: &Args,
    allow_prompt: bool,
) -> anyhow::Result<(IdentityKeys, String)> {
    let data = PathBuf::from(&cfg.data_dir);
    let existing = IdentityKeys::list(data.as_path())?;
    if !existing.is_empty() {
        anyhow::bail!(
            "数据目录已有节点身份，勿重复初始化。请直接启动或指定 --identity / MEMO_IDENTITY（已有 {} 个）",
            existing.len()
        );
    }
    let alias = args.alias.trim();
    if alias.is_empty() {
        anyhow::bail!("--alias 不能为空");
    }
    let local = if let Some(pw) = resolve_local_password(args)? {
        identity_keys::validate_local_password(&pw, &pw).map_err(|e| anyhow::anyhow!("{e}"))?;
        pw
    } else if allow_prompt {
        eprintln!("首次初始化存储节点：请设置节点解锁密码（密封私钥，不是业务用户）");
        read_and_confirm_node_password()?
    } else {
        anyhow::bail!(
            "无节点身份。请设置 MEMO_LOGIN_PASSWORD 或 --password-file 后重启（将自动创建存储节点），\
             或显式: memo --no-ui --init --alias storage"
        );
    };
    let id = IdentityKeys::generate(alias);
    eprintln!(
        "已创建存储节点身份 {} ({})，正在密封私钥…（这不是终端用户账号）",
        id.alias,
        IdentityKeys::short_fp(&id.fingerprint)
    );
    Ok((id, local))
}

pub fn unlock_identity(
    cfg: &Config,
    args: &Args,
) -> anyhow::Result<(IdentityKeys, String)> {
    let data = PathBuf::from(&cfg.data_dir);
    let env_pass = resolve_local_password(args)?;

    if args.init {
        return bootstrap_node_identity(cfg, args, true);
    }

    if let Some(path) = args.identity_file.as_deref() {
        let p = PathBuf::from(path);
        let enc = IdentityKeys::peek_encrypted(p.as_path()).unwrap_or(false);
        let id = if enc {
            let pack_pass = if let Some(ref pw) = env_pass {
                std::env::var("MEMO_PACK_PASSWORD").unwrap_or_else(|_| pw.clone())
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
            eprintln!("请为本机设置节点解锁密码（与密钥包无关）");
            read_and_confirm_node_password()?
        };
        return Ok((id, local));
    }

    let fp = if let Some(fp) = resolve_identity_fp(args) {
        fp
    } else {
        let list = IdentityKeys::list(data.as_path())?;
        if list.is_empty() {
            return bootstrap_node_identity(cfg, args, false);
        }
        if list.len() == 1 {
            eprintln!(
                "使用唯一节点身份 {} ({})",
                list[0].alias,
                IdentityKeys::short_fp(&list[0].fingerprint)
            );
            list[0].fingerprint.clone()
        } else if env_pass.is_some() {
            anyhow::bail!("多身份时请指定 --identity <指纹> 或 MEMO_IDENTITY");
        } else {
            eprintln!("可用节点身份:");
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
            eprintln!("此节点身份尚未设置解锁密码，请先设置：");
            read_and_confirm_node_password()?
        };
        let id = IdentityKeys::load(data.as_path(), &fp, "")?;
        Ok((id, local))
    } else {
        let local = if let Some(pw) = env_pass {
            pw
        } else {
            let local = read_password_prompt("节点解锁密码: ");
            if local.trim().is_empty() {
                anyhow::bail!("请输入节点解锁密码（或设置 MEMO_LOGIN_PASSWORD / --password-file）");
            }
            local.trim().to_string()
        };
        let id = IdentityKeys::load(data.as_path(), &fp, local.trim())?;
        Ok((id, local))
    }
}
