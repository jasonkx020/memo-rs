//! 生理期本机配置（按人员 id，加密落盘；不同步到其他节点）。

use chrono::{Duration, NaiveDate};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::enc_store;

const CYCLE_FILE: &str = "cycle.json.enc";

fn default_cycle_days() -> u32 {
    0
}

fn default_period_days() -> u32 {
    0
}

fn default_true() -> bool {
    true
}

/// 排卵/易孕窗预设（基于黄体期长度估算；仅供参考）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OvulationPreset {
    /// 关闭易孕/排卵着色
    Off,
    /// 标准：黄体期约 14 天（默认）
    #[default]
    Standard,
    /// 偏早排卵：黄体期约 16 天
    Early,
    /// 偏晚排卵：黄体期约 12 天
    Late,
}

impl OvulationPreset {
    pub const ALL: &'static [Self] = &[Self::Off, Self::Standard, Self::Early, Self::Late];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "关闭",
            Self::Standard => "标准（黄体期约 14 天）",
            Self::Early => "偏早排卵（黄体期约 16 天）",
            Self::Late => "偏晚排卵（黄体期约 12 天）",
        }
    }

    /// 估算黄体期天数；`Off` 返回 None。
    pub fn luteal_days(self) -> Option<u32> {
        match self {
            Self::Off => None,
            Self::Standard => Some(14),
            Self::Early => Some(16),
            Self::Late => Some(12),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleConfig {
    /// 最近一次经期开始日 YYYY-MM-DD（由标记自动得出，或特例写入）
    pub last_start: String,
    /// 有效周期天数；0 = 尚不足以推算
    #[serde(default = "default_cycle_days")]
    pub cycle_days: u32,
    /// 有效经期持续天数；0 = 尚不足以推算
    #[serde(default = "default_period_days")]
    pub period_days: u32,
    /// 用户手动指定周期（月经不调等特例）
    #[serde(default)]
    pub manual_cycle: bool,
    /// 用户手动指定经期长度
    #[serde(default)]
    pub manual_period: bool,
    /// 已查看关怀提醒所对应的「预计开始日」YYYY-MM-DD（未查看则持续提示）
    #[serde(default)]
    pub remind_seen_for: String,
    /// 排卵/易孕窗预设
    #[serde(default)]
    pub ovulation_preset: OvulationPreset,
    /// 是否在性别私密页展示经期模块（关闭仅藏 UI，不删标记）
    #[serde(default = "default_true")]
    pub period_enabled: bool,
}

impl Default for CycleConfig {
    fn default() -> Self {
        Self {
            last_start: String::new(),
            cycle_days: default_cycle_days(),
            period_days: default_period_days(),
            manual_cycle: false,
            manual_period: false,
            remind_seen_for: String::new(),
            ovulation_preset: OvulationPreset::Standard,
            period_enabled: true,
        }
    }
}

impl CycleConfig {
    /// 是否具备下次经期推算条件。
    pub fn can_predict(&self) -> bool {
        !self.last_start.is_empty() && self.cycle_days >= 15 && self.period_days >= 1
    }

    /// 排卵日相对经期开始的偏移（第 0 天=开始日）；无法估算时为 None。
    /// 一律按标准黄体期约 14 天：排卵日 ≈ 周期天数 − 14。
    pub fn ovulation_offset(&self) -> Option<u32> {
        const LUTEAL: u32 = 14;
        if !self.can_predict() {
            return None;
        }
        let cycle = self.cycle_days;
        let period = self.period_days.max(1);
        let off = cycle.saturating_sub(LUTEAL);
        if off <= period || off >= cycle {
            return None;
        }
        Some(off)
    }

    /// 易孕窗相对开始偏移 [start, end]（含端点）；不含经期日。
    pub fn fertile_offset_range(&self) -> Option<(u32, u32)> {
        let ov = self.ovulation_offset()?;
        let period = self.period_days.max(1);
        let cycle = self.cycle_days;
        let start = ov.saturating_sub(5).max(period);
        let end = (ov + 1).min(cycle.saturating_sub(1));
        if start > end {
            return None;
        }
        Some((start, end))
    }

    /// 即将到来的一轮周期开始日（易孕/排卵仅相对此锚点，不做多轮模运算）。
    /// 若以 `last_start` 为锚的易孕末日仍 ≥ 今天，用 last；否则用下一预测开始日。
    pub fn upcoming_cycle_start_for_fertile(&self) -> Option<NaiveDate> {
        if !self.can_predict() {
            return None;
        }
        let (_, end_off) = self.fertile_offset_range()?;
        let last = NaiveDate::parse_from_str(&self.last_start, "%Y-%m-%d").ok()?;
        let today = chrono::Local::now().date_naive();
        let fertile_end = last + Duration::try_days(end_off as i64)?;
        if fertile_end >= today {
            return Some(last);
        }
        self.next_predicted_start()
    }

    fn offset_from_anchor(&self, ymd: &str, anchor: NaiveDate) -> Option<i64> {
        let day = NaiveDate::parse_from_str(ymd, "%Y-%m-%d").ok()?;
        Some((day - anchor).num_days())
    }

    /// 估算排卵日：仅「即将到来的一轮」。
    pub fn is_ovulation_day(&self, ymd: &str) -> bool {
        let Some(ov) = self.ovulation_offset() else {
            return false;
        };
        let Some(anchor) = self.upcoming_cycle_start_for_fertile() else {
            return false;
        };
        self.offset_from_anchor(ymd, anchor) == Some(ov as i64)
    }

    /// 易孕参考窗（含排卵日）：仅即将到来的一轮；与该轮经期窗重叠时不算。
    pub fn is_fertile_day(&self, ymd: &str) -> bool {
        let Some((a, b)) = self.fertile_offset_range() else {
            return false;
        };
        let Some(anchor) = self.upcoming_cycle_start_for_fertile() else {
            return false;
        };
        let Some(off) = self.offset_from_anchor(ymd, anchor) else {
            return false;
        };
        let period = self.period_days.max(1) as i64;
        if off >= 0 && off < period {
            return false;
        }
        off >= a as i64 && off <= b as i64
    }

    /// 判断某日是否落在经期窗口（按固定周期模运算，含历史/未来各轮；内部/兼容用）。
    pub fn is_period_day(&self, ymd: &str) -> bool {
        if !self.can_predict() {
            return false;
        }
        let Some(day) = NaiveDate::parse_from_str(ymd, "%Y-%m-%d").ok() else {
            return false;
        };
        let Some(start) = NaiveDate::parse_from_str(&self.last_start, "%Y-%m-%d").ok() else {
            return false;
        };
        let cycle = self.cycle_days as i64;
        let period = self.period_days.max(1) as i64;
        let delta = (day - start).num_days();
        let offset = ((delta % cycle) + cycle) % cycle;
        offset < period
    }

    /// 唯一下一次（或进行中）预测经期开始日：从 `last_start + cycle` 起，
    /// 若整段经期已早于今天则逐周期推进。
    pub fn next_predicted_start(&self) -> Option<NaiveDate> {
        if !self.can_predict() {
            return None;
        }
        let last = NaiveDate::parse_from_str(&self.last_start, "%Y-%m-%d").ok()?;
        let cycle = self.cycle_days as i64;
        let period = self.period_days.max(1) as i64;
        let today = chrono::Local::now().date_naive();
        let step = Duration::try_days(cycle)?;
        let mut next = last + step;
        // 最多推进若干年，避免异常配置死循环
        for _ in 0..48 {
            let end = next + Duration::try_days(period - 1).unwrap_or_default();
            if end >= today {
                return Some(next);
            }
            next += step;
        }
        Some(next)
    }

    /// 日历用：仅标「下一次」预测窗口内、且不早于今天的日期。
    pub fn is_predicted_period_day(&self, ymd: &str) -> bool {
        let Some(day) = NaiveDate::parse_from_str(ymd, "%Y-%m-%d").ok() else {
            return false;
        };
        let today = chrono::Local::now().date_naive();
        if day < today {
            return false;
        }
        let Some(start) = self.next_predicted_start() else {
            return false;
        };
        let period = self.period_days.max(1) as i64;
        let end = start + Duration::try_days(period - 1).unwrap_or_default();
        day >= start && day <= end
    }

    /// 返回落在 [from, to] 内的**下次**预测经期日（含端点，不含过去）。
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
            if self.is_predicted_period_day(&ymd) {
                out.push(ymd);
            }
            d += Duration::try_days(1).unwrap_or_default();
        }
        out
    }
}

/// 一次经期片段：开始日 + 持续天数。
#[derive(Debug, Clone)]
pub struct PeriodEpisode {
    pub start: String,
    pub days: u32,
}

/// 由已标记的经期日推算的统计结果。
#[derive(Debug, Clone, Default)]
pub struct PeriodMarkStats {
    pub episodes: Vec<PeriodEpisode>,
    /// 相邻两次开始日间隔的平均值（15～45 天）
    pub avg_cycle: Option<u32>,
    pub last_gap: Option<u32>,
    /// 各次经期持续天数的平均值（1～10 天）
    pub avg_period: Option<u32>,
    pub last_period: Option<u32>,
    pub last_start: Option<String>,
}

/// 将标记的经期日聚类为片段，并推算周期 / 经期长度。
/// 连续日期（相邻差 1 天）视为同一次经期；至少两次片段才能推平均周期。
pub fn analyze_period_marks(marked_ymds: &[String]) -> PeriodMarkStats {
    let mut dates: Vec<NaiveDate> = marked_ymds
        .iter()
        .filter_map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .collect();
    dates.sort();
    dates.dedup();

    let mut episodes: Vec<PeriodEpisode> = Vec::new();
    if !dates.is_empty() {
        let mut start = dates[0];
        let mut prev = dates[0];
        let mut len = 1u32;
        for &d in &dates[1..] {
            if (d - prev).num_days() == 1 {
                len += 1;
                prev = d;
            } else {
                episodes.push(PeriodEpisode {
                    start: start.format("%Y-%m-%d").to_string(),
                    days: len,
                });
                start = d;
                prev = d;
                len = 1;
            }
        }
        episodes.push(PeriodEpisode {
            start: start.format("%Y-%m-%d").to_string(),
            days: len,
        });
    }

    let mut gaps = Vec::new();
    for w in episodes.windows(2) {
        let Some(a) = NaiveDate::parse_from_str(&w[0].start, "%Y-%m-%d").ok() else {
            continue;
        };
        let Some(b) = NaiveDate::parse_from_str(&w[1].start, "%Y-%m-%d").ok() else {
            continue;
        };
        let g = (b - a).num_days();
        if (15..=45).contains(&g) {
            gaps.push(g as u32);
        }
    }

    let period_lens: Vec<u32> = episodes
        .iter()
        .map(|e| e.days)
        .filter(|d| (1..=10).contains(d))
        .collect();

    PeriodMarkStats {
        last_start: episodes.last().map(|e| e.start.clone()),
        last_period: episodes.last().map(|e| e.days),
        last_gap: gaps.last().copied(),
        avg_cycle: if gaps.is_empty() {
            None
        } else {
            Some(gaps.iter().sum::<u32>() / gaps.len() as u32)
        },
        avg_period: if period_lens.is_empty() {
            None
        } else {
            Some(period_lens.iter().sum::<u32>() / period_lens.len() as u32)
        },
        episodes,
    }
}

/// 用标记统计刷新配置中的自动字段（尊重 manual_* 覆盖）。
pub fn apply_mark_stats(cfg: &mut CycleConfig, stats: &PeriodMarkStats) {
    if let Some(ls) = &stats.last_start {
        cfg.last_start = ls.clone();
    } else {
        cfg.last_start.clear();
    }
    if !cfg.manual_period {
        if let Some(p) = stats.avg_period.or(stats.last_period) {
            cfg.period_days = p;
        } else {
            cfg.period_days = 0;
        }
    }
    if !cfg.manual_cycle {
        if let Some(c) = stats.avg_cycle {
            cfg.cycle_days = c;
        } else {
            cfg.cycle_days = 0;
        }
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
        let data = enc_store::load_json::<CycleFile>(data_dir, CYCLE_FILE, key)?.unwrap_or_default();
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
        if !cfg.last_start.is_empty() {
            if cfg.last_start.len() != 10 {
                anyhow::bail!("上次开始日格式应为 YYYY-MM-DD");
            }
            if NaiveDate::parse_from_str(&cfg.last_start, "%Y-%m-%d").is_err() {
                anyhow::bail!("上次开始日无效");
            }
        }
        if cfg.manual_cycle && !(15..=45).contains(&cfg.cycle_days) {
            anyhow::bail!("手动周期须在 15～45 天");
        }
        if cfg.manual_period && !(1..=10).contains(&cfg.period_days) {
            anyhow::bail!("手动经期长度须在 1～10 天");
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
    fn analyze_two_episodes() {
        let days = vec![
            "2026-03-01".into(),
            "2026-03-02".into(),
            "2026-03-03".into(),
            "2026-03-04".into(),
            "2026-03-05".into(),
            "2026-03-29".into(),
            "2026-03-30".into(),
            "2026-03-31".into(),
            "2026-04-01".into(),
            "2026-04-02".into(),
        ];
        let s = analyze_period_marks(&days);
        assert_eq!(s.episodes.len(), 2);
        assert_eq!(s.episodes[0].days, 5);
        assert_eq!(s.episodes[1].days, 5);
        assert_eq!(s.avg_cycle, Some(28));
        assert_eq!(s.avg_period, Some(5));
        assert_eq!(s.last_start.as_deref(), Some("2026-03-29"));
    }

    #[test]
    fn predict_needs_cycle() {
        let mut cfg = CycleConfig::default();
        assert!(!cfg.can_predict());
        cfg.last_start = "2026-03-29".into();
        cfg.cycle_days = 28;
        cfg.period_days = 5;
        assert!(cfg.can_predict());
        assert!(cfg.is_period_day("2026-03-29"));
        assert!(cfg.is_period_day("2026-04-02"));
        assert!(!cfg.is_period_day("2026-04-03"));
    }

    #[test]
    fn predicted_only_next_window() {
        let mut cfg = CycleConfig::default();
        // 固定「今天」无关：用足够远的 last_start，使下次落在可解析窗口
        let today = chrono::Local::now().date_naive();
        let last = today - Duration::try_days(10).unwrap();
        cfg.last_start = last.format("%Y-%m-%d").to_string();
        cfg.cycle_days = 28;
        cfg.period_days = 5;

        let next = cfg.next_predicted_start().expect("next");
        assert_eq!(next, last + Duration::try_days(28).unwrap());

        // 下次窗口内且 >= today → 预测
        let in_win = next.format("%Y-%m-%d").to_string();
        assert!(cfg.is_predicted_period_day(&in_win));
        let last_of = (next + Duration::try_days(4).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        assert!(cfg.is_predicted_period_day(&last_of));
        // 更远的下一周期不标
        let far = (next + Duration::try_days(28).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        assert!(!cfg.is_predicted_period_day(&far));
        // 过去未标记日不标预测
        let past = (today - Duration::try_days(1).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        assert!(!cfg.is_predicted_period_day(&past));
    }

    #[test]
    fn next_predicted_advances_when_overdue() {
        let mut cfg = CycleConfig::default();
        let today = chrono::Local::now().date_naive();
        // 上次开始在很久以前，第一次预计已整段过完
        let last = today - Duration::try_days(40).unwrap();
        cfg.last_start = last.format("%Y-%m-%d").to_string();
        cfg.cycle_days = 28;
        cfg.period_days = 5;
        let next = cfg.next_predicted_start().expect("next");
        let end = next + Duration::try_days(4).unwrap();
        assert!(end >= today);
        // 应是 last+28 之后再推进至少一轮
        assert!(next > last + Duration::try_days(28).unwrap());
    }

    #[test]
    fn fertile_standard_28_day_cycle() {
        let mut cfg = CycleConfig::default();
        let today = chrono::Local::now().date_naive();
        // last_start 取「今天往前 5 天」，本轮易孕末日仍在未来 → 锚点=last
        let last = today - Duration::try_days(5).unwrap();
        cfg.last_start = last.format("%Y-%m-%d").to_string();
        cfg.cycle_days = 28;
        cfg.period_days = 5;
        cfg.ovulation_preset = OvulationPreset::Standard;
        assert_eq!(cfg.ovulation_offset(), Some(14));
        assert_eq!(cfg.fertile_offset_range(), Some((9, 15)));
        assert_eq!(cfg.upcoming_cycle_start_for_fertile(), Some(last));

        let ovu = (last + Duration::try_days(14).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        let fert_a = (last + Duration::try_days(9).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        let fert_b = (last + Duration::try_days(15).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        assert!(cfg.is_ovulation_day(&ovu));
        assert!(cfg.is_fertile_day(&fert_a));
        assert!(cfg.is_fertile_day(&fert_b));
        assert!(!cfg.is_fertile_day(&last.format("%Y-%m-%d").to_string())); // 经期
        // 更远下一轮不标
        let far_ovu = (last + Duration::try_days(28 + 14).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        assert!(!cfg.is_ovulation_day(&far_ovu));
        assert!(!cfg.is_fertile_day(&far_ovu));
    }

    #[test]
    fn fertile_advances_to_next_cycle_when_window_passed() {
        let mut cfg = CycleConfig::default();
        let today = chrono::Local::now().date_naive();
        // last 在 20 天前：本轮易孕末日约 last+15 已过 → 锚点切到 next
        let last = today - Duration::try_days(20).unwrap();
        cfg.last_start = last.format("%Y-%m-%d").to_string();
        cfg.cycle_days = 28;
        cfg.period_days = 5;
        cfg.ovulation_preset = OvulationPreset::Standard;
        let next = cfg.next_predicted_start().expect("next");
        assert_eq!(cfg.upcoming_cycle_start_for_fertile(), Some(next));
        let next_ovu = (next + Duration::try_days(14).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        assert!(cfg.is_ovulation_day(&next_ovu));
        let old_ovu = (last + Duration::try_days(14).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        assert!(!cfg.is_ovulation_day(&old_ovu));
    }
}
