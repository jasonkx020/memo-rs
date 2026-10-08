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

/// 事件截止日：跨天取结束日，否则取开始日。
pub fn deadline_date(due: &str, end: &str) -> Option<NaiveDate> {
    event_end_date(due, end)
}

/// 未完成且截止日早于今天。
pub fn is_overdue(due: &str, end: &str, done: bool) -> bool {
    if done {
        return false;
    }
    let Some(d) = deadline_date(due, end) else {
        return false;
    };
    d < Local::now().date_naive()
}

/// 未完成且截止日为今天。
pub fn is_due_today(due: &str, end: &str, done: bool) -> bool {
    if done {
        return false;
    }
    let Some(d) = deadline_date(due, end) else {
        return false;
    };
    d == Local::now().date_naive()
}

/// 已过期天数（未过期或无截止日为 None）。
pub fn overdue_days(due: &str, end: &str, done: bool) -> Option<i64> {
    if !is_overdue(due, end, done) {
        return None;
    }
    let d = deadline_date(due, end)?;
    Some((Local::now().date_naive() - d).num_days())
}

/// 距截止日天数：今天为 0，过期为负；无截止日或已完成为 None。
pub fn days_until_due(due: &str, end: &str, done: bool) -> Option<i64> {
    if done {
        return None;
    }
    let d = deadline_date(due, end)?;
    Some((d - Local::now().date_naive()).num_days())
}

/// 未完成且截止日在未来 `1..=within_days` 天内（不含今天与已过期）。
pub fn is_due_soon(due: &str, end: &str, done: bool, within_days: i64) -> bool {
    let within = within_days.max(1);
    match days_until_due(due, end, done) {
        Some(n) => n >= 1 && n <= within,
        None => false,
    }
}

/// 到期字段中的时刻部分 `HH:MM`（若有）。
pub fn due_time_hm(s: &str) -> Option<String> {
    let dt = parse_due_local(s)?;
    // 旧版仅日期会落成 09:00；有显式时分的字符串才展示
    let raw = s.trim();
    if raw.len() >= 16 && raw.as_bytes().get(10) == Some(&b' ') {
        return Some(dt.format("%H:%M").to_string());
    }
    None
}

/// 规范化结束日：无开始日则结束必须空；有开始日则结束>=开始。
/// 单日（结束=开始或不填）存空字符串，兼容旧数据。
pub fn normalize_end_date(due: &str, end: &str) -> Result<String, &'static str> {
    let due = due.trim();
    let end = end.trim();
    if due.is_empty() {
        if end.is_empty() {
            return Ok(String::new());
        }
        return Err("未安排日子时不要填结束日");
    }
    let start = due_date_part(due).ok_or("开始日无效")?;
    let end_d = if end.is_empty() {
        start
    } else {
        due_date_part(end).ok_or("结束日无效")?
    };
    if end_d < start {
        return Err("结束日不能早于开始日");
    }
    if end_d == start {
        return Ok(String::new());
    }
    Ok(end_d.format("%Y-%m-%d").to_string())
}

/// 日历上的结束日；无 due 则无区间。
pub fn event_end_date(due: &str, end: &str) -> Option<NaiveDate> {
    let start = due_date_part(due)?;
    Some(due_date_part(end).unwrap_or(start).max(start))
}

/// 该备忘是否覆盖某一日历日（含跨天）。
pub fn covers_calendar_day(due: &str, end: &str, day: NaiveDate) -> bool {
    let Some(start) = due_date_part(due) else {
        return false;
    };
    let end = event_end_date(due, end).unwrap_or(start);
    day >= start && day <= end
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_date_empty_when_single_day_or_blank() {
        assert_eq!(normalize_end_date("", "").unwrap(), "");
        assert!(normalize_end_date("", "2026-01-02").is_err());
        assert_eq!(normalize_end_date("2026-01-02 09:00", "").unwrap(), "");
        assert_eq!(
            normalize_end_date("2026-01-02 09:00", "2026-01-02").unwrap(),
            ""
        );
        assert_eq!(
            normalize_end_date("2026-01-02 09:00", "2026-01-05").unwrap(),
            "2026-01-05"
        );
        assert!(normalize_end_date("2026-01-05", "2026-01-02").is_err());
    }

    #[test]
    fn covers_span_inclusive() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 3).unwrap();
        assert!(covers_calendar_day("2026-01-02", "2026-01-05", d));
        assert!(!covers_calendar_day("2026-01-04", "2026-01-05", d));
        assert!(covers_calendar_day("2026-01-03 09:00", "", d));
    }

    #[test]
    fn overdue_uses_end_date_for_span() {
        let today = Local::now().date_naive();
        let start = (today - Duration::try_days(5).unwrap()).format("%Y-%m-%d").to_string();
        let end_past = (today - Duration::try_days(2).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        let end_future = (today + Duration::try_days(2).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        assert!(is_overdue(&start, &end_past, false));
        assert!(!is_overdue(&start, &end_future, false));
        assert!(!is_overdue(&start, &end_past, true));
        assert!(due_time_hm("2026-10-08 18:00").as_deref() == Some("18:00"));
        assert!(due_time_hm("2026-10-08").is_none());
    }

    #[test]
    fn due_soon_is_1_to_within_days() {
        let today = Local::now().date_naive();
        let in_2 = (today + Duration::try_days(2).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        let in_5 = (today + Duration::try_days(5).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        let today_s = today.format("%Y-%m-%d").to_string();
        assert!(is_due_soon(&in_2, "", false, 3));
        assert!(!is_due_soon(&in_5, "", false, 3));
        assert!(!is_due_soon(&today_s, "", false, 3));
        assert_eq!(days_until_due(&in_2, "", false), Some(2));
        assert_eq!(days_until_due(&today_s, "", false), Some(0));
    }
}
