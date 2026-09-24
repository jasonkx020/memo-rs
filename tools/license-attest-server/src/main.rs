//! 官方会话票鉴权服：只签发/续期/吊销，不转发笔记。
//!
//! 环境变量：
//! - `MEMO_ATTEST_PRIVATE_HEX` 或 `--attest-key`：会话票 Ed25519 私钥
//! - `MEMO_LICENSE_PRIVATE_HEX` 不需要；用内置 LICENSE_PUBKEY 验用户 license
//! - `MEMO_ATTEST_ADMIN_TOKEN`：吊销接口 Bearer

use chrono::Utc;
use clap::Parser;
use memo_core::license::{self, LicenseInfo, LicenseStatus, LICENSE_PUBKEY_HEX};
use memo_core::session::{self, SessionTicket};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use rustls::{Certificate, PrivateKey, ServerConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

const MAX_DEVICES: usize = 3;

#[derive(Parser, Debug)]
#[command(name = "license-attest-server", about = "Memo 会话票鉴权服（无同步中继）")]
struct Args {
    /// 监听地址
    #[arg(long, default_value = "0.0.0.0:8443")]
    listen: String,
    /// 数据目录（证书、设备库）
    #[arg(long, default_value = "./attest-data")]
    data_dir: PathBuf,
    /// 会话票私钥 hex 文件
    #[arg(long)]
    attest_key: Option<PathBuf>,
    /// 管理吊销 Bearer token（也可用环境变量 MEMO_ATTEST_ADMIN_TOKEN）
    #[arg(long)]
    admin_token: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    /// licensee -> 状态
    licenses: HashMap<String, LicenseRecord>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LicenseRecord {
    revoked: bool,
    devices: HashMap<String, DeviceRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceRecord {
    revoked: bool,
    /// 客户端证书公钥指纹（SHA256 hex）
    cert_fp: String,
}

#[derive(Clone)]
struct AppState {
    attest_key_hex: String,
    data_dir: PathBuf,
    registry: Arc<Mutex<Registry>>,
    admin_token: String,
}

#[derive(Deserialize)]
struct AttestBody {
    license: LicenseInfo,
    device_id: String,
    #[serde(default)]
    #[allow(dead_code)]
    app_version: String,
}

#[derive(Serialize)]
struct AttestOut {
    ticket: SessionTicket,
    device_cert_pem: String,
    device_key_pem: String,
}

#[derive(Deserialize)]
struct RenewBody {
    device_id: String,
    #[serde(default)]
    #[allow(dead_code)]
    app_version: String,
}

#[derive(Serialize)]
struct RenewOut {
    ticket: SessionTicket,
}

#[derive(Deserialize)]
struct RevokeBody {
    licensee: String,
    #[serde(default)]
    device_id: Option<String>,
}

fn load_attest_key(args: &Args) -> anyhow::Result<String> {
    if let Ok(v) = std::env::var("MEMO_ATTEST_PRIVATE_HEX") {
        let v = v.trim().to_string();
        if v.len() == 64 {
            return Ok(v);
        }
    }
    if let Some(p) = &args.attest_key {
        return Ok(fs::read_to_string(p)?.trim().to_string());
    }
    let candidates = [
        PathBuf::from("attest-private.hex"),
        PathBuf::from("../attest-private.hex"),
        PathBuf::from("../../attest-private.hex"),
    ];
    for c in candidates {
        if c.exists() {
            return Ok(fs::read_to_string(c)?.trim().to_string());
        }
    }
    anyhow::bail!("未找到会话票私钥：设置 MEMO_ATTEST_PRIVATE_HEX 或 --attest-key")
}

fn registry_path(data_dir: &Path) -> PathBuf {
    data_dir.join("registry.json")
}

fn load_registry(data_dir: &Path) -> Registry {
    let p = registry_path(data_dir);
    fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_registry(data_dir: &Path, reg: &Registry) -> anyhow::Result<()> {
    fs::create_dir_all(data_dir)?;
    let p = registry_path(data_dir);
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(reg)?)?;
    fs::rename(tmp, p)?;
    Ok(())
}

fn load_or_create_tls(data_dir: &Path) -> anyhow::Result<(ServerConfig, String, rcgen::Certificate)> {
    fs::create_dir_all(data_dir)?;
    let ca_cert_path = data_dir.join("ca.crt.pem");
    let ca_key_path = data_dir.join("ca.key.pem");
    let srv_cert_path = data_dir.join("server.crt.pem");
    let srv_key_path = data_dir.join("server.key.pem");

    let ca = if ca_cert_path.exists() && ca_key_path.exists() {
        let key_pem = fs::read_to_string(&ca_key_path)?;
        let key_pair = KeyPair::from_pem(&key_pem)?;
        let mut params = CertificateParams::new(vec!["memo-attest-ca".into()]);
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_pair = Some(key_pair);
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "Memo Attest CA");
        rcgen::Certificate::from_params(params)?
    } else {
        let mut params = CertificateParams::new(vec!["memo-attest-ca".into()]);
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "Memo Attest CA");
        let ca = rcgen::Certificate::from_params(params)?;
        fs::write(&ca_cert_path, ca.serialize_pem()?)?;
        fs::write(&ca_key_path, ca.serialize_private_key_pem())?;
        ca
    };

    let (server_cert_pem, server_key_pem) = if srv_cert_path.exists() && srv_key_path.exists() {
        (
            fs::read_to_string(&srv_cert_path)?,
            fs::read_to_string(&srv_key_path)?,
        )
    } else {
        let mut params = CertificateParams::new(vec![
            "localhost".into(),
            "license.memo-rs.example".into(),
        ]);
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "memo-attest");
        params.subject_alt_names = vec![
            SanType::DnsName("localhost".into()),
            SanType::DnsName("license.memo-rs.example".into()),
        ];
        let cert = rcgen::Certificate::from_params(params)?;
        let cert_pem = cert.serialize_pem_with_signer(&ca)?;
        let key_pem = cert.serialize_private_key_pem();
        fs::write(&srv_cert_path, &cert_pem)?;
        fs::write(&srv_key_path, &key_pem)?;
        (cert_pem, key_pem)
    };

    let certs = rustls_pemfile::certs(&mut server_cert_pem.as_bytes())?
        .into_iter()
        .map(Certificate)
        .collect::<Vec<_>>();
    let mut keys = rustls_pemfile::pkcs8_private_keys(&mut server_key_pem.as_bytes())?;
    let key = PrivateKey(
        keys.pop()
            .ok_or_else(|| anyhow::anyhow!("server key missing"))?,
    );

    let mut client_roots = rustls::RootCertStore::empty();
    let ca_pem = fs::read_to_string(&ca_cert_path)?;
    for c in rustls_pemfile::certs(&mut ca_pem.as_bytes())? {
        let _ = client_roots.add(&Certificate(c));
    }
    let client_auth = rustls::server::AllowAnyAnonymousOrAuthenticatedClient::new(client_roots);

    let mut config = ServerConfig::builder()
        .with_safe_defaults()
        .with_client_cert_verifier(Arc::new(client_auth))
        .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok((config, server_cert_pem, ca))
}

fn issue_device_cert(
    ca: &rcgen::Certificate,
    device_id: &str,
) -> anyhow::Result<(String, String, String)> {
    let mut params = CertificateParams::new(vec![device_id.to_string()]);
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, device_id);
    let cert = rcgen::Certificate::from_params(params)?;
    let cert_pem = cert.serialize_pem_with_signer(ca)?;
    let key_pem = cert.serialize_private_key_pem();
    let der = cert.serialize_der_with_signer(ca)?;
    let mut h = Sha256::new();
    h.update(&der);
    let fp = hex::encode(h.finalize());
    Ok((cert_pem, key_pem, fp))
}

fn cert_fp_from_der(der: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(der);
    hex::encode(h.finalize())
}

fn handle_attest(state: &AppState, ca: &rcgen::Certificate, body: &[u8]) -> (u16, String) {
    let req: AttestBody = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return json_err(400, &format!("bad json: {e}")),
    };
    if req.device_id.trim().is_empty() {
        return json_err(400, "device_id required");
    }
    match license::verify_with_pubkey(&req.license, LICENSE_PUBKEY_HEX) {
        LicenseStatus::Licensed { licensee, .. } => {
            let mut reg = state.registry.lock().unwrap();
            let rec = reg.licenses.entry(licensee.clone()).or_default();
            if rec.revoked {
                return json_err(403, "license revoked");
            }
            if !rec.devices.contains_key(&req.device_id) && rec.devices.len() >= MAX_DEVICES {
                let active = rec.devices.values().filter(|d| !d.revoked).count();
                if active >= MAX_DEVICES {
                    return json_err(403, "device limit reached");
                }
            }
            if let Some(d) = rec.devices.get(&req.device_id) {
                if d.revoked {
                    return json_err(403, "device revoked");
                }
            }
            let (cert_pem, key_pem, fp) = match issue_device_cert(ca, &req.device_id) {
                Ok(v) => v,
                Err(e) => return json_err(500, &e.to_string()),
            };
            rec.devices.insert(
                req.device_id.clone(),
                DeviceRecord {
                    revoked: false,
                    cert_fp: fp,
                },
            );
            let ticket = match session::issue_ticket(
                &state.attest_key_hex,
                &licensee,
                &req.device_id,
                Utc::now(),
            ) {
                Ok(t) => t,
                Err(e) => return json_err(500, &e.to_string()),
            };
            if let Err(e) = save_registry(&state.data_dir, &reg) {
                return json_err(500, &e.to_string());
            }
            let out = AttestOut {
                ticket,
                device_cert_pem: cert_pem,
                device_key_pem: key_pem,
            };
            (
                200,
                serde_json::to_string(&out).unwrap_or_else(|_| "{}".into()),
            )
        }
        LicenseStatus::Invalid(e) => json_err(403, &e),
        LicenseStatus::Community => json_err(403, "not licensed"),
    }
}

fn handle_renew(
    state: &AppState,
    peer_certs: Option<Vec<Certificate>>,
    body: &[u8],
) -> (u16, String) {
    let req: RenewBody = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return json_err(400, &format!("bad json: {e}")),
    };
    let Some(certs) = peer_certs.filter(|c| !c.is_empty()) else {
        return json_err(401, "client certificate required");
    };
    let fp = cert_fp_from_der(&certs[0].0);
    let mut reg = state.registry.lock().unwrap();
    let mut found_licensee: Option<String> = None;
    for (lic, rec) in reg.licenses.iter() {
        if rec.revoked {
            continue;
        }
        if let Some(d) = rec.devices.get(&req.device_id) {
            if !d.revoked && d.cert_fp == fp {
                found_licensee = Some(lic.clone());
                break;
            }
        }
    }
    let Some(licensee) = found_licensee else {
        return json_err(403, "device cert not recognized");
    };
    let ticket = match session::issue_ticket(
        &state.attest_key_hex,
        &licensee,
        &req.device_id,
        Utc::now(),
    ) {
        Ok(t) => t,
        Err(e) => return json_err(500, &e.to_string()),
    };
    let _ = &mut reg; // registry unchanged on renew
    let out = RenewOut { ticket };
    (
        200,
        serde_json::to_string(&out).unwrap_or_else(|_| "{}".into()),
    )
}

fn handle_revoke(state: &AppState, auth: Option<&str>, body: &[u8]) -> (u16, String) {
    let expected = format!("Bearer {}", state.admin_token);
    if state.admin_token.is_empty() || auth != Some(expected.as_str()) {
        return json_err(401, "unauthorized");
    }
    let req: RevokeBody = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => return json_err(400, &format!("bad json: {e}")),
    };
    let mut reg = state.registry.lock().unwrap();
    let rec = reg.licenses.entry(req.licensee.clone()).or_default();
    if let Some(did) = req.device_id {
        if let Some(d) = rec.devices.get_mut(&did) {
            d.revoked = true;
        } else {
            return json_err(404, "device not found");
        }
    } else {
        rec.revoked = true;
    }
    if let Err(e) = save_registry(&state.data_dir, &reg) {
        return json_err(500, &e.to_string());
    }
    (200, r#"{"ok":true}"#.into())
}

fn json_err(code: u16, msg: &str) -> (u16, String) {
    (
        code,
        serde_json::json!({ "error": msg }).to_string(),
    )
}

async fn handle_connection(
    state: AppState,
    ca: Arc<rcgen::Certificate>,
    acceptor: TlsAcceptor,
    stream: TcpStream,
) -> anyhow::Result<()> {
    let tls = acceptor.accept(stream).await?;
    let peer_certs = tls
        .get_ref()
        .1
        .peer_certificates()
        .map(|c| c.to_vec());

    let mut tls = tls;
    let mut buf = vec![0u8; 64 * 1024];
    let n = tls.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    let raw = &buf[..n];
    let (headers, body) = split_http(raw);
    let mut method = "";
    let mut path = "";
    let mut auth: Option<String> = None;
    let mut content_length = body.len();
    for (i, line) in headers.lines().enumerate() {
        if i == 0 {
            let mut parts = line.split_whitespace();
            method = parts.next().unwrap_or("");
            path = parts.next().unwrap_or("");
            continue;
        }
        if let Some(v) = line.strip_prefix("Authorization: ") {
            auth = Some(v.trim().to_string());
        }
        if let Some(v) = line.strip_prefix("Content-Length: ") {
            if let Ok(n) = v.trim().parse::<usize>() {
                content_length = n;
            }
        }
    }
    let mut body = body.to_vec();
    while body.len() < content_length {
        let m = tls.read(&mut buf).await?;
        if m == 0 {
            break;
        }
        body.extend_from_slice(&buf[..m]);
    }
    if body.len() > content_length {
        body.truncate(content_length);
    }

    let (code, resp_body) = if method == "POST" && path == "/v1/attest" {
        handle_attest(&state, &ca, &body)
    } else if method == "POST" && path == "/v1/renew" {
        handle_renew(&state, peer_certs, &body)
    } else if method == "POST" && path == "/v1/revoke" {
        handle_revoke(&state, auth.as_deref(), &body)
    } else if method == "GET" && path == "/health" {
        (200, r#"{"ok":true,"relay":false}"#.into())
    } else {
        json_err(404, "not found")
    };

    let resp = format!(
        "HTTP/1.1 {code} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
        reason(code),
        resp_body.len(),
    );
    tls.write_all(resp.as_bytes()).await?;
    tls.shutdown().await.ok();
    Ok(())
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    }
}

fn split_http(raw: &[u8]) -> (String, &[u8]) {
    if let Some(i) = find_header_end(raw) {
        let headers = String::from_utf8_lossy(&raw[..i]).into_owned();
        let body = &raw[i..];
        // skip CRLFCRLF
        let body = if body.starts_with(b"\r\n\r\n") {
            &body[4..]
        } else if body.starts_with(b"\n\n") {
            &body[2..]
        } else {
            body
        };
        (headers, body)
    } else {
        (String::from_utf8_lossy(raw).into_owned(), &[][..])
    }
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .or_else(|| raw.windows(2).position(|w| w == b"\n\n"))
}

fn pin_hex(cert_pem: &str) -> String {
    memo_core::attest::pin_hex_for_cert_pem(cert_pem).unwrap_or_default()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let attest_key_hex = load_attest_key(&args)?;
    // 自检：公钥须匹配客户端内嵌
    let sk_bytes = hex::decode(attest_key_hex.trim())?;
    if sk_bytes.len() != 32 {
        anyhow::bail!("attest key length");
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&sk_bytes);
    let sk = ed25519_dalek::SigningKey::from_bytes(&arr);
    let pub_hex = hex::encode(sk.verifying_key().to_bytes());
    if pub_hex != session::ATTEST_PUBKEY_HEX {
        eprintln!(
            "警告: 私钥对应公钥 {pub_hex} 与客户端 ATTEST_PUBKEY_HEX 不一致"
        );
    }

    fs::create_dir_all(&args.data_dir)?;
    let (tls_cfg, server_cert_pem, ca) = load_or_create_tls(&args.data_dir)?;
    let admin = args
        .admin_token
        .or_else(|| std::env::var("MEMO_ATTEST_ADMIN_TOKEN").ok())
        .unwrap_or_default();

    let state = AppState {
        attest_key_hex,
        data_dir: args.data_dir.clone(),
        registry: Arc::new(Mutex::new(load_registry(&args.data_dir))),
        admin_token: admin,
    };

    println!("listen {}", args.listen);
    println!("pin (leaf DER SHA-256) = {}", pin_hex(&server_cert_pem));
    println!("填入客户端 ATTEST_SPKI_SHA256_HEX；本服不转发任何同步数据");

    let listener = TcpListener::bind(&args.listen).await?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_cfg));
    let ca = Arc::new(ca);

    loop {
        let (stream, addr) = listener.accept().await?;
        let state = state.clone();
        let acceptor = acceptor.clone();
        let ca = ca.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(state, ca, acceptor, stream).await {
                eprintln!("{addr}: {e}");
            }
        });
    }
}

// signing key check uses ed25519_dalek::SigningKey above
