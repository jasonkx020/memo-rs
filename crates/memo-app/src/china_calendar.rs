//! 中国法定节假日与调休（离线表，按国务院办公厅通知维护）。
//! 覆盖 2025–2026；未收录年份仅按周六日判断周末。

use chrono::{Datelike, NaiveDate, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayKind {
    /// 普通工作日（周一至周五，且非放假）
    Workday,
    /// 周末休息（周六日，且未调休上班）
    Weekend,
    /// 法定放假 / 连休
    Holiday,
    /// 周末调休上班
    MakeupWork,
}

#[derive(Debug, Clone, Copy)]
pub struct DayInfo {
    pub kind: DayKind,
    /// 节日名，如「春节」；调休上班时可为空
    pub name: Option<&'static str>,
}

impl DayInfo {
    pub fn is_rest(self) -> bool {
        matches!(self.kind, DayKind::Weekend | DayKind::Holiday)
    }

    #[allow(dead_code)]
    pub fn is_work(self) -> bool {
        matches!(self.kind, DayKind::Workday | DayKind::MakeupWork)
    }

    /// 月历角标：休 / 班 / 空
    pub fn badge(self) -> Option<&'static str> {
        match self.kind {
            DayKind::Holiday => Some("休"),
            DayKind::MakeupWork => Some("班"),
            DayKind::Weekend => Some("末"),
            DayKind::Workday => None,
        }
    }

    /// 详情/总览用完整说明
    pub fn describe(self) -> String {
        match self.kind {
            DayKind::Workday => "工作日".into(),
            DayKind::Weekend => "周末".into(),
            DayKind::Holiday => match self.name {
                Some(n) => format!("休假 · {n}"),
                None => "法定休假".into(),
            },
            DayKind::MakeupWork => match self.name {
                Some(n) => format!("调休上班 · {n}"),
                None => "调休上班".into(),
            },
        }
    }
}

struct HolidaySpan {
    start: (i32, u32, u32),
    end: (i32, u32, u32),
    name: &'static str,
}

struct MakeupDay {
    ymd: (i32, u32, u32),
    name: &'static str,
}

fn ymd(t: (i32, u32, u32)) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(t.0, t.1, t.2)
}

fn in_span(d: NaiveDate, s: &HolidaySpan) -> bool {
    let Some(a) = ymd(s.start) else {
        return false;
    };
    let Some(b) = ymd(s.end) else {
        return false;
    };
    d >= a && d <= b
}

/// 国务院办公厅公布的放假区间（含调休并入的休息日）。
const HOLIDAYS: &[HolidaySpan] = &[
    // —— 2025 ——
    HolidaySpan {
        start: (2025, 1, 1),
        end: (2025, 1, 1),
        name: "元旦",
    },
    HolidaySpan {
        start: (2025, 1, 28),
        end: (2025, 2, 4),
        name: "春节",
    },
    HolidaySpan {
        start: (2025, 4, 4),
        end: (2025, 4, 6),
        name: "清明",
    },
    HolidaySpan {
        start: (2025, 5, 1),
        end: (2025, 5, 5),
        name: "劳动节",
    },
    HolidaySpan {
        start: (2025, 5, 31),
        end: (2025, 6, 2),
        name: "端午",
    },
    HolidaySpan {
        start: (2025, 10, 1),
        end: (2025, 10, 8),
        name: "国庆中秋",
    },
    // —— 2026 ——
    HolidaySpan {
        start: (2026, 1, 1),
        end: (2026, 1, 3),
        name: "元旦",
    },
    HolidaySpan {
        start: (2026, 2, 15),
        end: (2026, 2, 23),
        name: "春节",
    },
    HolidaySpan {
        start: (2026, 4, 4),
        end: (2026, 4, 6),
        name: "清明",
    },
    HolidaySpan {
        start: (2026, 5, 1),
        end: (2026, 5, 5),
        name: "劳动节",
    },
    HolidaySpan {
        start: (2026, 6, 19),
        end: (2026, 6, 21),
        name: "端午",
    },
    HolidaySpan {
        start: (2026, 9, 25),
        end: (2026, 9, 27),
        name: "中秋",
    },
    HolidaySpan {
        start: (2026, 10, 1),
        end: (2026, 10, 7),
        name: "国庆",
    },
];

/// 周末调休上班日。
const MAKEUP: &[MakeupDay] = &[
    // 2025
    MakeupDay {
        ymd: (2025, 1, 26),
        name: "春节调休",
    },
    MakeupDay {
        ymd: (2025, 2, 8),
        name: "春节调休",
    },
    MakeupDay {
        ymd: (2025, 4, 27),
        name: "劳动节调休",
    },
    MakeupDay {
        ymd: (2025, 9, 28),
        name: "国庆调休",
    },
    MakeupDay {
        ymd: (2025, 10, 11),
        name: "国庆调休",
    },
    // 2026
    MakeupDay {
        ymd: (2026, 1, 4),
        name: "元旦调休",
    },
    MakeupDay {
        ymd: (2026, 2, 14),
        name: "春节调休",
    },
    MakeupDay {
        ymd: (2026, 2, 28),
        name: "春节调休",
    },
    MakeupDay {
        ymd: (2026, 5, 9),
        name: "劳动节调休",
    },
    MakeupDay {
        ymd: (2026, 9, 20),
        name: "国庆调休",
    },
    MakeupDay {
        ymd: (2026, 10, 10),
        name: "国庆调休",
    },
];

pub fn classify(d: NaiveDate) -> DayInfo {
    for m in MAKEUP {
        if ymd(m.ymd) == Some(d) {
            return DayInfo {
                kind: DayKind::MakeupWork,
                name: Some(m.name),
            };
        }
    }
    for h in HOLIDAYS {
        if in_span(d, h) {
            return DayInfo {
                kind: DayKind::Holiday,
                name: Some(h.name),
            };
        }
    }
    match d.weekday() {
        Weekday::Sat | Weekday::Sun => DayInfo {
            kind: DayKind::Weekend,
            name: None,
        },
        _ => DayInfo {
            kind: DayKind::Workday,
            name: None,
        },
    }
}

pub fn classify_ymd(ymd_str: &str) -> Option<DayInfo> {
    NaiveDate::parse_from_str(ymd_str, "%Y-%m-%d")
        .ok()
        .map(classify)
}
