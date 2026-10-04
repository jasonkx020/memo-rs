//! 男性健康本机配置（按人员 id，加密落盘；不同步到其他节点）。

use chrono::NaiveDate;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::enc_store;

const MALE_HEALTH_FILE: &str = "male_health.json.enc";

fn default_remind_days() -> u32 {
    7
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaleHealthConfig {
    /// 上次体检日 YYYY-MM-DD
    #[serde(default)]
    pub last_checkup: String,
    /// 下次体检日 YYYY-MM-DD
    #[serde(default)]
    pub next_checkup: String,
    /// 提前提醒天数
    #[serde(default = "default_remind_days")]
    pub remind_days: u32,
    /// 已查看关怀提醒所对应的下次体检日
    #[serde(default)]
    pub remind_seen_for: String,
    /// 是否在性别私密页展示体检模块（关闭仅藏 UI，不删日期）
    #[serde(default = "default_true")]
    pub checkup_enabled: bool,
}

impl Default for MaleHealthConfig {
    fn default() -> Self {
        Self {
            last_checkup: String::new(),
            next_checkup: String::new(),
            remind_days: default_remind_days(),
            remind_seen_for: String::new(),
            checkup_enabled: true,
        }
    }
}

impl MaleHealthConfig {
    pub fn days_until_checkup(&self) -> Option<i64> {
        let next = NaiveDate::parse_from_str(&self.next_checkup, "%Y-%m-%d").ok()?;
        let today = chrono::Local::now().date_naive();
        Some((next - today).num_days())
    }

    /// 是否处于提醒窗口（提前 remind_days 天至体检当日及之后短窗口）。
    pub fn care_active(&self) -> bool {
        let Some(days) = self.days_until_checkup() else {
            return false;
        };
        let ahead = self.remind_days.max(1) as i64;
        // 提前 N 天起提醒；过期后仍提示 3 天，方便补记
        days <= ahead && days >= -3
    }

    pub fn care_seen(&self) -> bool {
        !self.next_checkup.is_empty() && self.remind_seen_for == self.next_checkup
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MaleHealthFile {
    by_person: HashMap<String, MaleHealthConfig>,
}

pub struct MaleHealthStore {
    data: Mutex<MaleHealthFile>,
    data_dir: PathBuf,
    key: parking_lot::Mutex<Vec<u8>>,
}

impl MaleHealthStore {
    pub fn open(data_dir: &Path, key: &[u8]) -> anyhow::Result<Self> {
        let data = enc_store::load_json::<MaleHealthFile>(data_dir, MALE_HEALTH_FILE, key)?
            .unwrap_or_default();
        Ok(Self {
            data: Mutex::new(data),
            data_dir: data_dir.to_path_buf(),
            key: parking_lot::Mutex::new(key.to_vec()),
        })
    }

    fn persist(&self) -> anyhow::Result<()> {
        let data = self.data.lock();
        let key = self.key.lock();
        enc_store::save_json(&self.data_dir, MALE_HEALTH_FILE, &key, &*data)
    }

    pub fn get(&self, person_id: &str) -> Option<MaleHealthConfig> {
        if person_id.is_empty() {
            return None;
        }
        self.data.lock().by_person.get(person_id).cloned()
    }

    pub fn set(&self, person_id: &str, cfg: MaleHealthConfig) -> anyhow::Result<()> {
        if person_id.is_empty() {
            anyhow::bail!("未选择当前人员");
        }
        for (label, s) in [
            ("上次体检日", cfg.last_checkup.as_str()),
            ("下次体检日", cfg.next_checkup.as_str()),
        ] {
            if !s.is_empty() {
                if s.len() != 10 {
                    anyhow::bail!("{label}格式应为 YYYY-MM-DD");
                }
                if NaiveDate::parse_from_str(s, "%Y-%m-%d").is_err() {
                    anyhow::bail!("{label}无效");
                }
            }
        }
        if cfg.remind_days == 0 || cfg.remind_days > 90 {
            anyhow::bail!("提前提醒天数须在 1～90");
        }
        {
            let mut data = self.data.lock();
            data.by_person.insert(person_id.to_string(), cfg);
        }
        self.persist()
    }

    pub fn clear(&self, person_id: &str) -> anyhow::Result<()> {
        {
            let mut data = self.data.lock();
            data.by_person.remove(person_id);
        }
        self.persist()
    }
}
