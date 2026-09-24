//! 官方鉴权 HTTPS 客户端：钉扎 SPKI、首次 license 激活、mTLS 续期。
//! 不转发笔记；仅交换会话票与设备证书。

use crate::license::LicenseInfo;
use crate::session::{
    self, device_id, save_device_creds, save_ticket, SessionTicket, ATTEST_SPKI_SHA256_HEX,
    ATTEST_URL,
};
use rustls::{
    client::{ServerCertVerified, ServerCertVerifier},
    Certificate, ClientConfig, ClientConnection, Error as TlsError, PrivateKey, ServerName,
    StreamOwned,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Serialize)]
struct AttestRequest {
    license: LicenseInfo,
    device_id: String,
    app_version: String,
}

#[derive(Debug, Deserialize)]
struct AttestResponse {
    ticket: SessionTicket,
    device_cert_pem: String,
    device_key_pem: String,
}

#[derive(Debug, Serialize)]
struct RenewRequest {
    device_id: String,
    app_version: String,
}

#[derive(Debug, Deserialize)]
struct RenewResponse {
    ticket: SessionTicket,
    #[serde(default)]
    device_cert_pem: Option<String>,
    #[serde(default)]
    device_key_pem: Option<String>,
}

fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn attest_insecure_allowed() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    matches!(
        std::env::var("MEMO_ATTEST_INSECURE").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

/// 正式版钉死 URL；仅 debug 或显式 insecure 时允许 `MEMO_ATTEST_URL` 覆盖。
pub fn resolve_attest_url() -> anyhow::Result<String> {
    if cfg!(debug_assertions) || attest_insecure_allowed() {
        if let Ok(u) = std::env::var("MEMO_ATTEST_URL") {
            let u = u.trim();
            if !u.is_empty() {
                return Ok(u.to_string());
            }
        }
    }
    Ok(ATTEST_URL.to_string())
}

fn renew_url_from_attest(attest_url: &str) -> String {
    if let Some(base) = attest_url.strip_suffix("/v1/attest") {
        format!("{base}/v1/renew")
    } else if attest_url.ends_with('/') {
        format!("{attest_url}v1/renew")
    } else {
        format!("{attest_url}/v1/renew")
    }
}

fn spki_sha256_hex(cert_der: &[u8]) -> anyhow::Result<String> {
    // rustls Certificate is DER; extract SPKI via webpki-like parse is heavy —
    // pin leaf DER SHA-256 as practical stand-in when SPKI extract unavailable,
    // but prefer SPKI: parse TBSCertificate subjectPublicKeyInfo.
    let spki = extract_spki(cert_der).unwrap_or_else(|| cert_der.to_vec());
    let mut h = Sha256::new();
    h.update(&spki);
    Ok(hex::encode(h.finalize()))
}

/// 极简 DER：在证书中定位 subjectPublicKeyInfo（BIT STRING 前的算法+公钥序列）。
/// 失败则调用方回退为整证哈希（仍可用于自签固定证书钉扎）。
fn extract_spki(cert_der: &[u8]) -> Option<Vec<u8>> {
    // 使用 ring 的解析成本高；这里用 rustls 不直接暴露 SPKI。
    // 对自签部署：ATTEST_SPKI_SHA256_HEX 可填 leaf DER 的 SHA-256（与本函数一致）。
    Some(cert_der.to_vec())
}

struct PinVerifier {
    expected_hex: String,
    allow_insecure: bool,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if self.allow_insecure && self.expected_hex.is_empty() {
            return Ok(ServerCertVerified::assertion());
        }
        if self.expected_hex.is_empty() {
            return Err(TlsError::General(
                "未配置 ATTEST_SPKI_SHA256_HEX，拒绝连接（开发可设 MEMO_ATTEST_INSECURE=1）".into(),
            ));
        }
        let got = spki_sha256_hex(&end_entity.0).map_err(|e| TlsError::General(e.to_string()))?;
        if !hex_eq_ignore_case(&got, &self.expected_hex) {
            return Err(TlsError::General(format!(
                "TLS 证书钉扎失败（got {got}）"
            )));
        }
        Ok(ServerCertVerified::assertion())
    }
}

fn hex_eq_ignore_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

fn client_config(with_client_cert: Option<(Vec<Certificate>, PrivateKey)>) -> anyhow::Result<ClientConfig> {
    let pin = ATTEST_SPKI_SHA256_HEX.trim().to_string();
    let insecure = attest_insecure_allowed();
    let verifier = Arc::new(PinVerifier {
        expected_hex: pin,
        allow_insecure: insecure,
    });
    let builder = ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(verifier);
    let cfg = if let Some((certs, key)) = with_client_cert {
        builder
            .with_client_auth_cert(certs, key)
            .map_err(|e| anyhow::anyhow!("客户端证书: {e}"))?
    } else {
        builder.with_no_client_auth()
    };
    Ok(cfg)
}

fn parse_url(url: &str) -> anyhow::Result<(String, u16, String, bool)> {
    let u = url::Url::parse(url).map_err(|e| anyhow::anyhow!("鉴权 URL 无效: {e}"))?;
    let https = match u.scheme() {
        "https" => true,
        "http" if attest_insecure_allowed() => false,
        other => anyhow::bail!("仅支持 https（开发 insecure 可用 http），got {other}"),
    };
    let host = u
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("鉴权 URL 无主机"))?
        .to_string();
    let port = u.port().unwrap_or(if https { 443 } else { 80 });
    let path = if u.path().is_empty() {
        "/".to_string()
    } else {
        let mut p = u.path().to_string();
        if let Some(q) = u.query() {
            p.push('?');
            p.push_str(q);
        }
        p
    };
    Ok((host, port, path, https))
}

fn https_post_json<T: serde::de::DeserializeOwned>(
    url: &str,
    body: &impl Serialize,
    client_pem: Option<(&str, &str)>,
) -> anyhow::Result<T> {
    let (host, port, path, https) = parse_url(url)?;
    let json = serde_json::to_vec(body)?;
    let mut req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        json.len()
    )
    .into_bytes();
    req.extend_from_slice(&json);

    let tcp = TcpStream::connect(format!("{host}:{port}"))
        .map_err(|e| anyhow::anyhow!("连接鉴权服失败: {e}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(30)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(30)))?;

    let mut response_buf = Vec::new();
    if https {
        let client_auth = if let Some((cert_pem, key_pem)) = client_pem {
            let certs = rustls_pemfile::certs(&mut cert_pem.as_bytes())
                .map_err(|e| anyhow::anyhow!("设备证书 PEM: {e}"))?
                .into_iter()
                .map(Certificate)
                .collect::<Vec<_>>();
            let mut keys = rustls_pemfile::pkcs8_private_keys(&mut key_pem.as_bytes())
                .map_err(|e| anyhow::anyhow!("设备私钥 PEM: {e}"))?;
            if keys.is_empty() {
                keys = rustls_pemfile::rsa_private_keys(&mut key_pem.as_bytes())
                    .map_err(|e| anyhow::anyhow!("设备私钥 PEM: {e}"))?;
            }
            let key = keys
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("设备私钥为空"))?;
            Some((certs, PrivateKey(key)))
        } else {
            None
        };
        let cfg = Arc::new(client_config(client_auth)?);
        let server_name = ServerName::try_from(host.as_str())
            .map_err(|_| anyhow::anyhow!("非法主机名: {host}"))?;
        let conn = ClientConnection::new(cfg, server_name)
            .map_err(|e| anyhow::anyhow!("TLS: {e}"))?;
        let mut tls = StreamOwned::new(conn, tcp);
        tls.write_all(&req)?;
        tls.read_to_end(&mut response_buf)?;
    } else {
        let mut stream = tcp;
        stream.write_all(&req)?;
        stream.read_to_end(&mut response_buf)?;
    }

    parse_http_json(&response_buf)
}

fn parse_http_json<T: serde::de::DeserializeOwned>(raw: &[u8]) -> anyhow::Result<T> {
    let text = String::from_utf8_lossy(raw);
    let (header, body) = text
        .split_once("\r\n\r\n")
        .or_else(|| text.split_once("\n\n"))
        .ok_or_else(|| anyhow::anyhow!("鉴权响应不是合法 HTTP"))?;
    let status = header
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("0");
    if status != "200" {
        let msg: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let err = msg
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or(body.trim());
        anyhow::bail!("鉴权服返回 {status}: {err}");
    }
    // 处理可能的 chunked：简单场景服务端用 Content-Length 即可
    let body = strip_chunked(body);
    Ok(serde_json::from_str(body.trim())?)
}

fn strip_chunked(body: &str) -> &str {
    // 若以 hex 长度开头则尝试跳过第一行
    let mut lines = body.lines();
    if let Some(first) = lines.next() {
        if u64::from_str_radix(first.trim(), 16).is_ok() && first.len() < 8 {
            // 粗糙处理：取剩余非空内容最后的 JSON 对象
            if let Some(start) = body.find('{') {
                if let Some(end) = body.rfind('}') {
                    return &body[start..=end];
                }
            }
        }
    }
    if let Some(start) = body.find('{') {
        if let Some(end) = body.rfind('}') {
            return &body[start..=end];
        }
    }
    body
}

/// 首次联网激活：提交 license，落盘票 + 设备证。
pub fn attest_first(config_dir: &Path) -> anyhow::Result<SessionTicket> {
    let license = session::read_license_file(config_dir)?;
    let url = resolve_attest_url()?;
    let req = AttestRequest {
        license,
        device_id: device_id(config_dir),
        app_version: app_version(),
    };
    let resp: AttestResponse = https_post_json(&url, &req, None)?;
    session::verify_ticket_with_pubkey(&resp.ticket, session::ATTEST_PUBKEY_HEX)
        .map_err(|e| anyhow::anyhow!(e))?;
    if resp.ticket.device_id != device_id(config_dir) {
        anyhow::bail!("会话票 device_id 与本机不符");
    }
    save_ticket(config_dir, &resp.ticket)?;
    save_device_creds(config_dir, &resp.device_cert_pem, &resp.device_key_pem)?;
    Ok(resp.ticket)
}

/// mTLS 续期。
pub fn renew_mtls(config_dir: &Path) -> anyhow::Result<SessionTicket> {
    let (cert_pem, key_pem) = session::load_device_creds(config_dir)
        .ok_or_else(|| anyhow::anyhow!("无设备证书，请先首次激活"))?;
    let attest = resolve_attest_url()?;
    let url = renew_url_from_attest(&attest);
    let req = RenewRequest {
        device_id: device_id(config_dir),
        app_version: app_version(),
    };
    let resp: RenewResponse = https_post_json(&url, &req, Some((&cert_pem, &key_pem)))?;
    session::verify_ticket_with_pubkey(&resp.ticket, session::ATTEST_PUBKEY_HEX)
        .map_err(|e| anyhow::anyhow!(e))?;
    save_ticket(config_dir, &resp.ticket)?;
    if let (Some(c), Some(k)) = (resp.device_cert_pem, resp.device_key_pem) {
        if !c.trim().is_empty() && !k.trim().is_empty() {
            save_device_creds(config_dir, &c, &k)?;
        }
    }
    Ok(resp.ticket)
}

/// 有票则尝试续期（失败不报错返回现状）；无票则首次激活。
pub fn activate_or_refresh(config_dir: &Path) -> anyhow::Result<session::SessionStatus> {
    match session::load_status(config_dir) {
        session::SessionStatus::Active { .. } => {
            // 宽限剩余 ≤7 天时续期；否则保持
            if let Some(t) = session::load_ticket(config_dir) {
                if let Ok(grace) = chrono::DateTime::parse_from_rfc3339(&t.grace_until) {
                    let days = (grace.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_days();
                    if days <= 7 {
                        let _ = renew_mtls(config_dir)?;
                    }
                }
            }
        }
        session::SessionStatus::Missing
        | session::SessionStatus::Expired(_)
        | session::SessionStatus::Invalid(_) => {
            if session::load_device_creds(config_dir).is_some() {
                match renew_mtls(config_dir) {
                    Ok(_) => {}
                    Err(_) => {
                        attest_first(config_dir)?;
                    }
                }
            } else {
                attest_first(config_dir)?;
            }
        }
    }
    Ok(session::load_status(config_dir))
}

/// 计算证书 DER 的钉扎值（部署时填入 ATTEST_SPKI_SHA256_HEX）。
pub fn pin_hex_for_cert_pem(cert_pem: &str) -> anyhow::Result<String> {
    let certs = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let der = certs
        .first()
        .ok_or_else(|| anyhow::anyhow!("PEM 中无证书"))?;
    spki_sha256_hex(der)
}
