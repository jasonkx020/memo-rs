//! Version update check against commerce-hub latest.json.
//! No auto-download/install — prompt only and open download page.

use std::cmp::Ordering;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::Duration;

/// commerce-hub latest.json（与 PUBLIC_BASE 对齐）。
pub const VERSION_JSON_URL: &str = "http://127.0.0.1:8080/api/v1/products/memo/latest.json";
/// Default download page when JSON omits download_url.
pub const DOWNLOAD_PAGE_URL: &str = "http://127.0.0.1:8080/memo/download";

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

/// Test/debug: inject fake remote; pass None to restore network fetch.
#[allow(dead_code)]
pub fn set_fetch_override(f: Option<FetchFn>) {
    if let Ok(mut g) = FETCH_OVERRIDE.lock() {
        *g = f;
    }
}

/// HTTP GET `VERSION_JSON_URL` and parse `{version, download_url}`.
pub fn fetch_remote_latest() -> Result<RemoteLatest, String> {
    if let Ok(g) = FETCH_OVERRIDE.lock() {
        if let Some(f) = *g {
            return f();
        }
    }
    let body = http_get_text(VERSION_JSON_URL)?;
    parse_latest_json(&body)
}

fn parse_latest_json(body: &str) -> Result<RemoteLatest, String> {
    let v = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|e| format!("latest.json 解析失败: {e}"))?;
    let version = v
        .get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if version.is_empty() {
        return Err("latest.json 缺少 version".into());
    }
    let download_url = v
        .get("download_url")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(RemoteLatest {
        version,
        download_url,
    })
}

/// Minimal HTTP/1.0 GET for http:// host:port/path (local commerce-hub).
fn http_get_text(url: &str) -> Result<String, String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| "仅支持 http:// 更新地址".to_string())?;
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let host = hostport.split(':').next().unwrap_or(hostport);
    let mut stream = TcpStream::connect(hostport)
        .map_err(|e| format!("连接更新服务失败 ({hostport}): {e}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(8)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(8)));
    let req = format!(
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("请求更新失败: {e}"))?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取更新响应失败: {e}"))?;
    let text = String::from_utf8_lossy(&buf);
    let (_headers, body) = text
        .split_once("\r\n\r\n")
        .or_else(|| text.split_once("\n\n"))
        .ok_or_else(|| "更新响应格式无效".to_string())?;
    if let Some(status_line) = text.lines().next() {
        if !status_line.contains("200") {
            return Err(format!("更新接口返回: {status_line}"));
        }
    }
    Ok(body.trim().to_string())
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

/// Unified check entry: fetch remote and compare with local.
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
    }

    #[test]
    fn parse_latest_json_ok() {
        let r = parse_latest_json(
            r#"{"version":"0.3.1","download_url":"http://x/dl","notes":"n"}"#,
        )
        .unwrap();
        assert_eq!(r.version, "0.3.1");
        assert_eq!(r.download_url, "http://x/dl");
    }
}
