//! 生理期本机配置（按人员 id，加密落盘；不同步到其他节点）。

use chrono::{Duration, NaiveDate};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::enc_store;

const CYCLE_FILE: &str = "cycle.json.enc";

fn default_cycle_days() -> u32 {
    28
}

fn default_period_days() -> u32 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleConfig {
    /// 上次经期开始日 YYYY-MM-DD
    pub last_start: String,
    #[serde(default = "default_cycle_days")]
    pub cycle_days: u32,
    #[serde(default = "default_period_days")]
    pub period_days: u32,
}

impl Default for CycleConfig {
    fn default() -> Self {
        Self {
            last_start: String::new(),
            cycle_days: default_cycle_days(),
            period_days: default_period_days(),
        }
    }
}

impl CycleConfig {
    /// 判断某日是否落在经期（按固定周期向前/后推算）。
    pub fn is_period_day(&self, ymd: &str) -> bool {
        let Some(day) = NaiveDate::parse_from_str(ymd, "%Y-%m-%d").ok() else {
            return false;
        };
        let Some(start) = NaiveDate::parse_from_str(&self.last_start, "%Y-%m-%d").ok() else {
            return false;
        };
        let cycle = self.cycle_days.max(1) as i64;
        let period = self.period_days.max(1) as i64;
        let delta = (day - start).num_days();
        let offset = ((delta % cycle) + cycle) % cycle;
        offset < period
    }

    /// 返回落在 [from, to] 内的经期日（含端点）。
    pub fn period_days_in_range(&self, from: &str, to: &str) -> Vec<String> {
        let Some(mut d) = NaiveDate::parse_from_str(from, "%Y-%m-%d").ok() else {
            return Vec::new();
        };
        let Some(end) = NaiveDate::parse_from_str(to, "%Y-%m-%d").ok() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while d <= end {
            let ymd = d.format("%Y-%m-%d").to_string();
            if self.is_period_day(&ymd) {
                out.push(ymd);
            }
            d += Duration::try_days(1).unwrap_or_default();
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CycleFile {
    by_person: HashMap<String, CycleConfig>,
}

pub struct CycleStore {
    data: Mutex<CycleFile>,
    data_dir: PathBuf,
    key: parking_lot::Mutex<Vec<u8>>,
}

impl CycleStore {
    pub fn open(data_dir: &Path, key: &[u8]) -> anyhow::Result<Self> {
        let data = enc_store::load_json::<CycleFile>(data_dir, CYCLE_FILE, key)?
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
        enc_store::save_json(&self.data_dir, CYCLE_FILE, &key, &*data)
    }

    pub fn get(&self, person_id: &str) -> Option<CycleConfig> {
        if person_id.is_empty() {
            return None;
        }
        self.data.lock().by_person.get(person_id).cloned()
    }

    pub fn set(&self, person_id: &str, cfg: CycleConfig) -> anyhow::Result<()> {
        if person_id.is_empty() {
            anyhow::bail!("未选择当前人员");
        }
        if cfg.last_start.len() != 10 {
            anyhow::bail!("上次开始日格式应为 YYYY-MM-DD");
        }
        if NaiveDate::parse_from_str(&cfg.last_start, "%Y-%m-%d").is_err() {
            anyhow::bail!("上次开始日无效");
        }
        if cfg.cycle_days == 0 || cfg.period_days == 0 {
            anyhow::bail!("周期天数与持续天数须大于 0");
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
