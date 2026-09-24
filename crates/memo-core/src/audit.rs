use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const GENESIS_PREV: &str = "GENESIS";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: u64,
    pub time: String,
    pub prev: String,
    pub data: serde_json::Value,
    pub hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sig: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum EventType {
    Create,
    Modify,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventData {
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub memo_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

pub fn compute_hash(seq: u64, prev: &str, time: &str, data: &serde_json::Value) -> String {
    let payload = format!("{seq}|{prev}|{time}|{data}");
    let dig = Sha256::digest(payload.as_bytes());
    hex::encode(dig)
}

pub struct KeyPair {
    pub signing: SigningKey,
}

impl KeyPair {
    pub fn load_or_create(data_dir: &Path) -> anyhow::Result<Self> {
        let dir = data_dir.join("keys");
        fs::create_dir_all(&dir)?;
        let priv_path = dir.join("private.key");
        if priv_path.exists() {
            let hex_str = fs::read_to_string(&priv_path)?;
            let bytes = hex::decode(hex_str.trim())?;
            if bytes.len() != 32 {
                anyhow::bail!("私钥长度错误");
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            let signing = SigningKey::from_bytes(&arr);
            return Ok(Self { signing });
        }
        let signing = SigningKey::generate(&mut OsRng);
        fs::write(&priv_path, hex::encode(signing.to_bytes()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&priv_path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&priv_path, perms)?;
        }
        let vk = signing.verifying_key();
        fs::write(dir.join("public.key"), hex::encode(vk.to_bytes()))?;
        Ok(Self { signing })
    }

    pub fn sign_hash(&self, hash_hex: &str) -> anyhow::Result<String> {
        let raw = hex::decode(hash_hex)?;
        let sig = self.signing.sign(&raw);
        Ok(hex::encode(sig.to_bytes()))
    }

    pub fn verify_sig(&self, hash_hex: &str, sig_hex: &str) -> bool {
        let Ok(raw) = hex::decode(hash_hex) else {
            return false;
        };
        let Ok(sig_bytes) = hex::decode(sig_hex) else {
            return false;
        };
        let Ok(sig) = Signature::from_slice(&sig_bytes) else {
            return false;
        };
        let vk: VerifyingKey = self.signing.verifying_key();
        vk.verify(&raw, &sig).is_ok()
    }
}

pub struct AuditLog {
    path: PathBuf,
    file: Mutex<File>,
    keys: KeyPair,
    last_seq: Mutex<u64>,
    last_hash: Mutex<String>,
}

impl AuditLog {
    pub fn open(path: PathBuf, keys: KeyPair) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut last_seq = 0u64;
        let mut last_hash = GENESIS_PREV.to_string();
        if path.exists() {
            let f = File::open(&path)?;
            for line in BufReader::new(f).lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let e: AuditEntry = serde_json::from_str(&line)?;
                last_seq = e.seq;
                last_hash = e.hash;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
            keys,
            last_seq: Mutex::new(last_seq),
            last_hash: Mutex::new(last_hash),
        })
    }

    pub fn append_value(&self, data: serde_json::Value) -> anyhow::Result<()> {
        let mut seq_g = self.last_seq.lock().unwrap();
        let mut hash_g = self.last_hash.lock().unwrap();
        let seq = *seq_g + 1;
        let prev = hash_g.clone();
        let time = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        let hash = compute_hash(seq, &prev, &time, &data);
        let sig = self.keys.sign_hash(&hash)?;
        let entry = AuditEntry {
            seq,
            time,
            prev,
            data,
            hash: hash.clone(),
            sig,
        };
        let mut line = serde_json::to_string(&entry)?;
        line.push('\n');
        let mut f = self.file.lock().unwrap();
        f.write_all(line.as_bytes())?;
        f.flush()?;
        *seq_g = seq;
        *hash_g = hash;
        Ok(())
    }

    pub fn read_all(&self) -> anyhow::Result<Vec<AuditEntry>> {
        let f = File::open(&self.path)?;
        let mut out = Vec::new();
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            out.push(serde_json::from_str(&line)?);
        }
        Ok(out)
    }

    pub fn verify_file(&self) -> Result<(), String> {
        let f = File::open(&self.path).map_err(|e| e.to_string())?;
        let mut prev = GENESIS_PREV.to_string();
        let mut expect_seq = 1u64;
        for line in BufReader::new(f).lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            let e: AuditEntry = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            if e.seq != expect_seq {
                return Err(format!(
                    "断裂于 #{}: seq 期望 {} 实际 {}",
                    e.seq, expect_seq, e.seq
                ));
            }
            if e.prev != prev {
                return Err(format!("断裂于 #{}: prev 不匹配", e.seq));
            }
            let want = compute_hash(e.seq, &e.prev, &e.time, &e.data);
            if want != e.hash {
                return Err(format!("断裂于 #{}: hash 不匹配", e.seq));
            }
            if e.sig.is_empty() || !self.keys.verify_sig(&e.hash, &e.sig) {
                return Err(format!("断裂于 #{}: 签名无效", e.seq));
            }
            prev = e.hash;
            expect_seq += 1;
        }
        Ok(())
    }
}
