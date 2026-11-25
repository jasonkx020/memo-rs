//! 男性健康本机配置（按人员 id，加密落盘；不同步到其他节点）。

use chrono::{Datelike, NaiveDate};
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

fn default_interval_months() -> u32 {
    12
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
    /// 最近确认关怀的日历日 YYYY-MM-DD（按日已读；窗口内次日再提示）
    #[serde(default)]
    pub remind_seen_for: String,
    /// 是否在性别私密页展示体检模块（关闭仅藏 UI，不删日期）
    #[serde(default = "default_true")]
    pub checkup_enabled: bool,
    /// 体检是否按月周期重复
    #[serde(default)]
    pub checkup_repeat: bool,
    /// 重复间隔（月），1～36；仅 `checkup_repeat` 时生效
    #[serde(default = "default_interval_months")]
    pub checkup_interval_months: u32,
}

impl Default for MaleHealthConfig {
    fn default() -> Self {
        Self {
            last_checkup: String::new(),
            next_checkup: String::new(),
            remind_days: default_remind_days(),
            remind_seen_for: String::new(),
            checkup_enabled: true,
            checkup_repeat: false,
            checkup_interval_months: default_interval_months(),
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

    /// 今天是否已确认过关怀（`remind_seen_for` = 今日 YYYY-MM-DD）。
    pub fn care_seen(&self) -> bool {
        let today = chrono::Local::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        !self.next_checkup.is_empty() && self.remind_seen_for == today
    }

    /// `ymd` 加 `months` 个月（日对齐，遇月末钳到当月最后一天）。
    pub fn ymd_plus_months(ymd: &str, months: u32) -> Option<String> {
        let d = NaiveDate::parse_from_str(ymd, "%Y-%m-%d").ok()?;
        let months = months.max(1);
        let total = (d.year() as i64) * 12 + (d.month() as i64 - 1) + months as i64;
        let y = (total / 12) as i32;
        let m = (total % 12) as u32 + 1;
        let next_month = if m == 12 {
            NaiveDate::from_ymd_opt(y + 1, 1, 1)?
        } else {
            NaiveDate::from_ymd_opt(y, m + 1, 1)?
        };
        let last_day = (next_month - chrono::Duration::try_days(1)?).day();
        let day = d.day().min(last_day);
        NaiveDate::from_ymd_opt(y, m, day).map(|nd| nd.format("%Y-%m-%d").to_string())
    }

    /// 标注体检日：过去→上次；今天及未来→下次。开启重复且标的是已过/今天完成时，自动推下次。
    pub fn mark_checkup_day(&mut self, ymd: &str) {
        let today = chrono::Local::now().date_naive();
        let Ok(day) = NaiveDate::parse_from_str(ymd, "%Y-%m-%d") else {
            return;
        };
        let interval = self.checkup_interval_months.clamp(1, 36);
        self.checkup_interval_months = interval;
        if day < today {
            self.last_checkup = ymd.to_string();
            if self.checkup_repeat {
                if let Some(n) = Self::ymd_plus_months(ymd, interval) {
                    self.next_checkup = n;
                    self.remind_seen_for.clear();
                }
            }
        } else {
            // 今天：视为本次体检日（记入上次），重复则推下次
            if day == today {
                self.last_checkup = ymd.to_string();
                if self.checkup_repeat {
                    if let Some(n) = Self::ymd_plus_months(ymd, interval) {
                        self.next_checkup = n;
                    } else {
                        self.next_checkup.clear();
                    }
                } else if self.next_checkup == ymd {
                    self.next_checkup.clear();
                }
                self.remind_seen_for.clear();
            } else {
                self.next_checkup = ymd.to_string();
                self.remind_seen_for.clear();
            }
        }
    }

    /// 取消某日上的体检标注（上次或下次）。
    pub fn clear_checkup_day(&mut self, ymd: &str) {
        if self.last_checkup == ymd {
            self.last_checkup.clear();
        }
        if self.next_checkup == ymd {
            self.next_checkup.clear();
            self.remind_seen_for.clear();
        }
    }

    pub fn is_checkup_day(&self, ymd: &str) -> bool {
        (!self.last_checkup.is_empty() && self.last_checkup == ymd)
            || (!self.next_checkup.is_empty() && self.next_checkup == ymd)
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
        if cfg.checkup_interval_months == 0 || cfg.checkup_interval_months > 36 {
            anyhow::bail!("体检重复间隔须在 1～36 个月");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_months_clamps_end_of_month() {
        assert_eq!(
            MaleHealthConfig::ymd_plus_months("2026-01-31", 1).as_deref(),
            Some("2026-02-28")
        );
        assert_eq!(
            MaleHealthConfig::ymd_plus_months("2026-03-15", 12).as_deref(),
            Some("2027-03-15")
        );
    }

    #[test]
    fn mark_past_with_repeat_sets_next() {
        let mut cfg = MaleHealthConfig {
            checkup_repeat: true,
            checkup_interval_months: 6,
            ..Default::default()
        };
        let today = chrono::Local::now().date_naive();
        let past = (today - chrono::Duration::try_days(10).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        cfg.mark_checkup_day(&past);
        assert_eq!(cfg.last_checkup, past);
        let expect = MaleHealthConfig::ymd_plus_months(&past, 6).unwrap();
        assert_eq!(cfg.next_checkup, expect);
    }
}
