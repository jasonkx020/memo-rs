//! 备忘到期时间解析与提醒时刻计算。

use chrono::{Duration, Local, NaiveDate, NaiveDateTime, NaiveTime};

/// 解析到期字段：`YYYY-MM-DD HH:MM` 或旧版 `YYYY-MM-DD`（按当天 09:00）。
pub fn parse_due_local(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M") {
        return Some(dt);
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let t = NaiveTime::from_hms_opt(9, 0, 0)?;
        return Some(d.and_time(t));
    }
    None
}

/// 到期日的日期部分（用于「今天到期」过滤）。
pub fn due_date_part(s: &str) -> Option<NaiveDate> {
    parse_due_local(s).map(|dt| dt.date())
}

pub fn format_due(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d %H:%M").to_string()
}

/// 提醒触发时刻 = 到期 − 提前天数（在到期时刻的同一钟点）。
pub fn remind_at(due: &str, before_days: u32) -> Option<NaiveDateTime> {
    let dt = parse_due_local(due)?;
    let days = before_days as i64;
    Some(dt - Duration::try_days(days).unwrap_or_default())
}

pub fn due_is_due_today(s: &str) -> bool {
    let Some(d) = due_date_part(s) else {
        return false;
    };
    d == Local::now().date_naive()
}

/// 列表/详情短展示。
pub fn display_due(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return "永久".into();
    }
    if let Some(dt) = parse_due_local(s) {
        return format!("到期 {}", dt.format("%m-%d %H:%M"));
    }
    format!("到期 {s}")
}
