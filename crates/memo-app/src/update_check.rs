//! Version update check: compare logic ready; network fetch is a stub.
//! No auto-download/install — prompt only and open download page.

use std::cmp::Ordering;
use std::sync::Mutex;

/// Future: URL hosting latest.json.
pub const VERSION_JSON_URL: &str = "https://memo.example.com/update/latest.json";
/// Default download page when JSON omits download_url.
pub const DOWNLOAD_PAGE_URL: &str = "https://memo.example.com/download";

/// Remote latest version payload.
#[derive(Debug, Clone)]
pub struct RemoteLatest {
    pub version: String,
    pub download_url: String,
}

/// Result after comparing with local version.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub latest: String,
    pub download_url: String,
    pub newer: bool,
}

type FetchFn = fn() -> Result<RemoteLatest, String>;

static FETCH_OVERRIDE: Mutex<Option<FetchFn>> = Mutex::new(None);

/// Test/debug: inject fake remote; pass None to restore stub.
#[allow(dead_code)]
pub fn set_fetch_override(f: Option<FetchFn>) {
    if let Ok(mut g) = FETCH_OVERRIDE.lock() {
        *g = f;
    }
}

/// Future: HTTP GET VERSION_JSON_URL and parse JSON.
/// This stage: stub returns not-configured error.
pub fn fetch_remote_latest() -> Result<RemoteLatest, String> {
    if let Ok(g) = FETCH_OVERRIDE.lock() {
        if let Some(f) = *g {
            return f();
        }
    }
    // TODO(network): ureq (or other) GET + parse JSON
    let _ = VERSION_JSON_URL;
    Err("更新检查接口尚未配置".into())
}

/// Dot-separated version compare (non-numeric as 0). Less => local < remote.
pub fn compare_semver(local: &str, remote: &str) -> Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.trim()
            .trim_start_matches('v')
            .split(|c: char| c == '.' || c == '-' || c == '+')
            .filter(|p| !p.is_empty())
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    };
    let a = parse(local);
    let b = parse(remote);
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    Ordering::Equal
}

/// Unified check entry: fetch remote (may stub) and compare with local.
pub fn check_against_local(local: &str) -> Result<UpdateInfo, String> {
    let remote = fetch_remote_latest()?;
    let download_url = if remote.download_url.trim().is_empty() {
        DOWNLOAD_PAGE_URL.to_string()
    } else {
        remote.download_url
    };
    let newer = matches!(compare_semver(local, &remote.version), Ordering::Less);
    Ok(UpdateInfo {
        latest: remote.version,
        download_url,
        newer,
    })
}

/// Open download page in the system browser.
pub fn open_download_page(url: &str) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_semver_basic() {
        assert_eq!(compare_semver("0.1.0", "0.2.0"), Ordering::Less);
        assert_eq!(compare_semver("0.2.0", "0.1.0"), Ordering::Greater);
        assert_eq!(compare_semver("0.1.0", "0.1.0"), Ordering::Equal);
        assert_eq!(compare_semver("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_semver("v0.1.0", "0.1.1"), Ordering::Less);
    }

    #[test]
    fn check_against_local_uses_override() {
        set_fetch_override(Some(|| {
            Ok(RemoteLatest {
                version: "9.9.9".into(),
                download_url: "https://example.com/dl".into(),
            })
        }));
        let info = check_against_local("0.1.0").unwrap();
        assert!(info.newer);
        assert_eq!(info.latest, "9.9.9");
        assert_eq!(info.download_url, "https://example.com/dl");
        set_fetch_override(None);
        let err = check_against_local("0.1.0").unwrap_err();
        assert!(err.contains("尚未配置"));
    }
}
