//! 签发 license.json。私钥来自环境变量 MEMO_LICENSE_PRIVATE_HEX，
//! 或 --key-file（默认读取仓库根目录 gitignored 的 license-private.hex）。

use clap::Parser;
use memo_core::license;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "license-sign", about = "签发 DistributedMemo license.json")]
struct Args {
    /// 被授权方名称
    #[arg(long)]
    licensee: String,

    /// 版本名称（默认：商业版）
    #[arg(long, default_value = "商业版")]
    edition: String,

    /// 过期时间 RFC3339，可选
    #[arg(long)]
    expires: Option<String>,

    /// 输出路径
    #[arg(long, default_value = "license.json")]
    out: PathBuf,

    /// 私钥 hex 文件（32 字节）
    #[arg(long)]
    key_file: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let key_hex = if let Ok(v) = std::env::var("MEMO_LICENSE_PRIVATE_HEX") {
        v
    } else {
        let path = args.key_file.unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("license-private.hex")
        });
        fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("读取私钥失败 {}: {e}", path.display()))?
    };
    let lic = license::sign_license(
        key_hex.trim(),
        &args.licensee,
        &args.edition,
        args.expires.as_deref(),
    )?;
    // 确认可用内置公钥验签
    match license::verify_with_pubkey(&lic, license::LICENSE_PUBKEY_HEX) {
        license::LicenseStatus::Licensed { .. } => {}
        other => anyhow::bail!("签发后自检失败: {other:?}"),
    }
    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&args.out, serde_json::to_string_pretty(&lic)?)?;
    println!("已写入 {}", args.out.display());
    Ok(())
}
