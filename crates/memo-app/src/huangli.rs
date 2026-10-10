//! 黄历：农历日、干支、节气与每日宜忌（本地 lunar_rust，不联网）。
//!
//! 性能要点：`get_day_yi` / `get_day_ji` 单次约秒级，绝不能在月历格每帧调用。
//! 格子只用轻量农历/节气；宜忌异步计算并按日缓存。

use chrono::{Datelike, NaiveDate};
use eframe::egui;
use lunar_rust::lunar::LunarRefHelper;
use lunar_rust::solar::{self, SolarRefHelper};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct DayAlmanac {
    pub lunar_month: String,
    pub lunar_day: String,
    pub gan_zhi: String,
    pub jie_qi: String,
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

    pub fn yi_text(&self, max_chars: usize) -> String {
        join_truncate(&self.yi, max_chars)
    }

    pub fn ji_text(&self, max_chars: usize) -> String {
        join_truncate(&self.ji, max_chars)
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

/// 已缓存的完整黄历（含宜忌）；未算完则 `None`。
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
    DayAlmanac {
        lunar_month: lunar.get_month_in_chinese(),
        lunar_day: lunar.get_day_in_chinese(),
        gan_zhi: lunar.get_day_in_gan_zhi(),
        jie_qi,
        yi: lunar.get_day_yi(None),
        ji: lunar.get_day_ji(None),
    }
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
