//! 商用闭环：授权状态、外链与 license.json 导入。

use memo_core::config;
use memo_core::license::{self, LicenseStatus};
use std::fs;
use std::path::{Path, PathBuf};

/// 商用官网基址（与 commerce-hub `COMMERCE_PUBLIC_BASE` 对齐）。
pub const PUBLIC_BASE: &str = "http://127.0.0.1:8080";
/// 购买授权页。
pub const BUY_URL: &str = "http://127.0.0.1:8080/memo/buy";
/// 支持 / 反馈页。
pub const SUPPORT_URL: &str = "http://127.0.0.1:8080/memo/support";

/// 是否强制校验商业授权才能用局域网同步等能力。
/// 当前版本全部免费开放；后续若收费，改为 `true` 即可恢复门控。
pub const LICENSE_ENFORCED: bool = false;

/// 社区版同步拦截时的状态条文案（仅 `LICENSE_ENFORCED` 时使用）。
pub const SYNC_BLOCKED_MSG: &str = "社区版仅本机；请在关于/设置中导入商业授权后再同步";

/// 商业功能是否可用（未强制授权时始终为 true）。
pub fn commercial_features_ok() -> bool {
    !LICENSE_ENFORCED || is_licensed()
}

/// 配置目录（`settings.json` 所在目录）。
pub fn config_dir() -> anyhow::Result<PathBuf> {
    let p = config::settings_path()?;
    Ok(p
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(p))
}

pub fn load_status() -> LicenseStatus {
    match config_dir() {
        Ok(dir) => license::load_status(&dir),
        Err(_) => LicenseStatus::Community,
    }
}

pub fn is_licensed() -> bool {
    matches!(load_status(), LicenseStatus::Licensed { .. })
}

pub fn edition_label() -> String {
    license::status_label(&load_status())
}

/// 打开系统浏览器。
pub fn open_url(url: &str) {
    crate::update_check::open_download_page(url);
}

pub fn open_buy_page() {
    let _ = PUBLIC_BASE;
    open_url(BUY_URL);
}

pub fn open_support_page() {
    open_url(SUPPORT_URL);
}

/// 在资源管理器中打开配置目录。
pub fn open_config_dir() {
    if let Ok(dir) = config_dir() {
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("explorer")
                .arg(dir)
                .spawn();
        }
        #[cfg(not(windows))]
        {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

/// 将用户选择的授权文件复制为配置目录下的 `license.json` 并验签。
pub fn import_license_from(path: &Path) -> anyhow::Result<LicenseStatus> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir)?;
    let dest = dir.join("license.json");
    let raw = fs::read(path).map_err(|e| anyhow::anyhow!("无法读取授权文件: {e}"))?;
    // 先写入临时再替换，避免半截文件
    let tmp = dir.join("license.json.tmp");
    fs::write(&tmp, &raw).map_err(|e| anyhow::anyhow!("无法写入授权文件: {e}"))?;
    fs::rename(&tmp, &dest).map_err(|e| anyhow::anyhow!("无法安装授权文件: {e}"))?;
    let st = license::load_status(&dir);
    match &st {
        LicenseStatus::Licensed { .. } => Ok(st),
        LicenseStatus::Community => Err(anyhow::anyhow!("授权未生效")),
        LicenseStatus::Invalid(e) => Err(anyhow::anyhow!(e.clone())),
    }
}

/// 弹出文件对话框导入授权；成功返回状态标签。
pub fn import_license_via_dialog() -> anyhow::Result<String> {
    let path = crate::save_dialog::open_license_dialog()
        .ok_or_else(|| anyhow::anyhow!("已取消"))?;
    let st = import_license_from(&path)?;
    Ok(license::status_label(&st))
}
