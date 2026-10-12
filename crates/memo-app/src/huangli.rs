//! 黄历：农历日、干支、节气与都市「上班宜忌」（本地 lunar_rust，不联网）。
//!
//! 性能要点：`get_day_yi` / `get_day_ji` 单次约秒级，绝不能在月历格每帧调用。
//! 格子只用轻量农历/节气；宜忌异步计算并按日缓存（缓存即为上班精简版）。

use chrono::{Datelike, NaiveDate};
use eframe::egui;
use lunar_rust::lunar::LunarRefHelper;
use lunar_rust::solar::{self, SolarRefHelper};
use memo_core::license::LicenseStatus;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// 社区版宜/忌各最多条数；商业版取过滤后上限。
pub const COMMUNITY_YI_JI_LIMIT: usize = 2;
pub const LICENSED_YI_JI_LIMIT: usize = 6;

#[derive(Debug, Clone)]
pub struct DayAlmanac {
    pub lunar_month: String,
    pub lunar_day: String,
    pub gan_zhi: String,
    pub jie_qi: String,
    /// 已过滤的上班宜忌（非完整传统列表）
    pub yi: Vec<String>,
    pub ji: Vec<String>,
}

impl DayAlmanac {
    pub fn cell_label(&self) -> String {
        if !self.jie_qi.is_empty() {
            self.jie_qi.clone()
        } else {
            self.lunar_day.clone()
        }
    }

    pub fn lunar_line(&self) -> String {
        let mut s = format!("农历{}月{}", self.lunar_month, self.lunar_day);
        if !self.gan_zhi.is_empty() {
            s.push_str(" · ");
            s.push_str(&self.gan_zhi);
        }
        if !self.jie_qi.is_empty() {
            s.push_str(" · ");
            s.push_str(&self.jie_qi);
        }
        s
    }

    /// 按授权截断后的宜/忌文案；`show_upsell` 表示社区版被截断、可提示升级。
    pub fn display_yi_ji_text(&self, licensed: bool, max_chars: usize) -> (String, String, bool) {
        let limit = if licensed {
            LICENSED_YI_JI_LIMIT
        } else {
            COMMUNITY_YI_JI_LIMIT
        };
        let yi_view: Vec<_> = self.yi.iter().take(limit).cloned().collect();
        let ji_view: Vec<_> = self.ji.iter().take(limit).cloned().collect();
        let show_upsell = !licensed
            && (self.yi.len() > COMMUNITY_YI_JI_LIMIT || self.ji.len() > COMMUNITY_YI_JI_LIMIT);
        (
            join_or_placeholder(&yi_view, max_chars),
            join_or_placeholder(&ji_view, max_chars),
            show_upsell,
        )
    }
}

#[derive(Debug, Clone)]
pub struct CellMark {
    pub text: String,
    pub is_jie_qi: bool,
}

fn label_cache() -> &'static Mutex<HashMap<NaiveDate, CellMark>> {
    static C: OnceLock<Mutex<HashMap<NaiveDate, CellMark>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn full_cache() -> &'static Mutex<HashMap<NaiveDate, DayAlmanac>> {
    static C: OnceLock<Mutex<HashMap<NaiveDate, DayAlmanac>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pending() -> &'static Mutex<HashSet<NaiveDate>> {
    static C: OnceLock<Mutex<HashSet<NaiveDate>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 当前是否为有效商业授权（本地 `license.json`）。
pub fn is_licensed() -> bool {
    let Ok(settings) = memo_core::config::settings_path() else {
        return false;
    };
    let Some(dir) = settings.parent() else {
        return false;
    };
    matches!(
        memo_core::license::load_status(dir),
        LicenseStatus::Licensed { .. }
    )
}

/// 月历格用：仅农历日/节气（微秒级），带缓存。
pub fn cell_mark(d: NaiveDate) -> CellMark {
    if let Some(v) = label_cache().lock().get(&d).cloned() {
        return v;
    }
    let mark = compute_cell_mark(d);
    label_cache().lock().insert(d, mark.clone());
    mark
}

fn compute_cell_mark(d: NaiveDate) -> CellMark {
    let solar = solar::from_ymd(d.year() as i64, d.month() as i64, d.day() as i64);
    let lunar = solar.get_lunar();
    let jq = lunar.get_jie_qi();
    if !jq.trim().is_empty() {
        CellMark {
            text: jq.trim().to_string(),
            is_jie_qi: true,
        }
    } else {
        CellMark {
            text: lunar.get_day_in_chinese(),
            is_jie_qi: false,
        }
    }
}

/// 已缓存的上班黄历（含宜忌）；未算完则 `None`。
pub fn cached_full(d: NaiveDate) -> Option<DayAlmanac> {
    full_cache().lock().get(&d).cloned()
}

/// 轻量农历摘要（侧栏在宜忌未就绪时先展示）。
pub fn lunar_summary_line(d: NaiveDate) -> String {
    if let Some(al) = cached_full(d) {
        return al.lunar_line();
    }
    let solar = solar::from_ymd(d.year() as i64, d.month() as i64, d.day() as i64);
    let lunar = solar.get_lunar();
    let month = lunar.get_month_in_chinese();
    let day = lunar.get_day_in_chinese();
    let gz = lunar.get_day_in_gan_zhi();
    let jq = lunar.get_jie_qi().trim().to_string();
    label_cache().lock().insert(
        d,
        CellMark {
            text: if jq.is_empty() {
                day.clone()
            } else {
                jq.clone()
            },
            is_jie_qi: !jq.is_empty(),
        },
    );
    let mut s = format!("农历{month}月{day}");
    if !gz.is_empty() {
        s.push_str(" · ");
        s.push_str(&gz);
    }
    if !jq.is_empty() {
        s.push_str(" · ");
        s.push_str(&jq);
    }
    s
}

/// 后台计算宜忌并写入缓存；同一天不重复开线程。算完 `request_repaint`。
pub fn ensure_full_async(d: NaiveDate, ctx: egui::Context) {
    {
        let cache = full_cache().lock();
        if cache.contains_key(&d) {
            return;
        }
    }
    {
        let mut pend = pending().lock();
        if !pend.insert(d) {
            return;
        }
    }
    std::thread::spawn(move || {
        let al = compute_full(d);
        label_cache().lock().insert(
            d,
            CellMark {
                is_jie_qi: !al.jie_qi.is_empty(),
                text: al.cell_label(),
            },
        );
        full_cache().lock().insert(d, al);
        pending().lock().remove(&d);
        ctx.request_repaint();
    });
}

fn compute_full(d: NaiveDate) -> DayAlmanac {
    let solar = solar::from_ymd(d.year() as i64, d.month() as i64, d.day() as i64);
    let lunar = solar.get_lunar();
    let jie_qi = lunar.get_jie_qi().trim().to_string();
    let (yi, ji) = urban_work_yi_ji(&lunar.get_day_yi(None), &lunar.get_day_ji(None));
    DayAlmanac {
        lunar_month: lunar.get_month_in_chinese(),
        lunar_day: lunar.get_day_in_chinese(),
        gan_zhi: lunar.get_day_in_gan_zhi(),
        jie_qi,
        yi,
        ji,
    }
}

/// 都市上班宜忌：保留商务/通勤/工地/出差/剃头等，丢掉嫁娶安葬等传统项。
fn urban_work_yi_ji(yi: &[String], ji: &[String]) -> (Vec<String>, Vec<String>) {
    (
        filter_rank_take(yi, LICENSED_YI_JI_LIMIT),
        filter_rank_take(ji, LICENSED_YI_JI_LIMIT),
    )
}

fn filter_rank_take(items: &[String], limit: usize) -> Vec<String> {
    let mut scored: Vec<(i32, String)> = items
        .iter()
        .filter_map(|s| {
            let t = s.trim();
            if t.is_empty() || is_discard(t) {
                return None;
            }
            urban_priority(t).map(|p| (p, t.to_string()))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut out = Vec::new();
    for (_, s) in scored {
        if out.iter().any(|x: &String| x == &s) {
            continue;
        }
        out.push(s);
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// 优先级：数字越小越靠前。未命中上班关键词则 None（丢弃）。
fn urban_priority(s: &str) -> Option<i32> {
    // 商务 / 老板
    const BIZ: &[&str] = &["签约", "交易", "开市", "纳财", "立券", "买卖", "开仓", "纳畜"];
    // 职员 / 通勤
    const WORK: &[&str] = &["上班", "赴任", "求职", "见贵", "会友", "上官", "入学"];
    // 工人 / 工地
    const BUILD: &[&str] = &["动土", "修造", "入宅", "安床", "竖柱", "上梁", "作梁"];
    // 出差
    const TRAVEL: &[&str] = &["出行", "移徙", "远行", "开道"];
    // 都市生活（明确保留剃头）
    const LIFE: &[&str] = &["剃头", "理发", "整容"];
    // 弱相关
    const WEAK: &[&str] = &["祭祀", "祈福", "订盟"];

    if contains_any(s, BIZ) {
        return Some(10);
    }
    if contains_any(s, WORK) {
        return Some(20);
    }
    if contains_any(s, BUILD) {
        return Some(30);
    }
    if contains_any(s, TRAVEL) {
        return Some(40);
    }
    if contains_any(s, LIFE) {
        return Some(50);
    }
    if contains_any(s, WEAK) {
        return Some(80);
    }
    None
}

fn is_discard(s: &str) -> bool {
    // 强传统项丢弃（不含剃头；祭祀走 WEAK 优先级）
    const HARD: &[&str] = &[
        "嫁娶", "安葬", "破土", "开光", "作灶", "捕捉", "伐木", "田猎", "成服", "除服",
        "启攒", "移柩", "入殓", "安香", "出火", "谢土", "纳采", "问名", "冠笄", "进人口",
        "分居", "合寿木", "行丧",
    ];
    contains_any(s, HARD)
}

fn contains_any(s: &str, keys: &[&str]) -> bool {
    keys.iter().any(|k| s.contains(k))
}

fn join_or_placeholder(items: &[String], max_chars: usize) -> String {
    if items.is_empty() {
        return "今日无突出上班宜忌".into();
    }
    join_truncate(items, max_chars)
}

fn join_truncate(items: &[String], max_chars: usize) -> String {
    if items.is_empty() {
        return "—".into();
    }
    let take = items.iter().take(8);
    let mut out = String::new();
    for (i, it) in take.enumerate() {
        let piece = if i == 0 {
            it.clone()
        } else {
            format!("、{it}")
        };
        let next_len = out.chars().count() + piece.chars().count();
        if next_len > max_chars {
            if out.is_empty() {
                return truncate_chars(it, max_chars);
            }
            out.push('…');
            break;
        }
        out.push_str(&piece);
    }
    if items.len() > 8 && !out.ends_with('…') {
        out.push('…');
    }
    out
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_shave_and_biz_drops_wedding() {
        let raw = vec![
            "嫁娶".into(),
            "签约".into(),
            "剃头".into(),
            "安葬".into(),
            "出行".into(),
            "修造".into(),
        ];
        let out = filter_rank_take(&raw, 6);
        assert!(out.contains(&"签约".into()));
        assert!(out.contains(&"剃头".into()));
        assert!(out.contains(&"出行".into()));
        assert!(!out.iter().any(|s| s.contains("嫁娶")));
        assert!(!out.iter().any(|s| s.contains("安葬")));
        assert_eq!(out[0], "签约");
    }
}
