//! Calendar tab: left navigator + wide central overview / task detail.

use chrono::{Datelike, Local, NaiveDate, TimeDelta};
use eframe::egui::{self, Color32, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::person::PersonView;
use memo_core::task::{TaskStatus, TaskView};

use crate::china_calendar::{self, DayKind};
use crate::theme;

fn days(n: i64) -> TimeDelta {
    TimeDelta::try_days(n).expect("days in range")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalMode {
    Month,
    Week,
    Gantt,
}

#[derive(Debug, Clone)]
pub struct CalUi {
    pub mode: CalMode,
    pub year: i32,
    pub month: u32,
    /// Monday of the visible week (YYYY-MM-DD).
    pub week_monday: String,
    pub selected_day: String,
    pub selected_task: Option<String>,
}

impl Default for CalUi {
    fn default() -> Self {
        let today = Local::now().date_naive();
        Self {
            mode: CalMode::Month,
            year: today.year(),
            month: today.month(),
            week_monday: monday_of(today).format("%Y-%m-%d").to_string(),
            selected_day: today.format("%Y-%m-%d").to_string(),
            selected_task: None,
        }
    }
}

pub fn monday_of(d: NaiveDate) -> NaiveDate {
    let wd = d.weekday().num_days_from_monday() as i64;
    d - days(wd)
}

pub fn parse_ymd(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn fmt_hm(hour: f32) -> String {
    format!(
        "{:02}:{:02}",
        hour.floor() as i32,
        ((hour.fract() * 60.0).round() as i32).clamp(0, 59),
    )
}

/// 同日时段：HH:MM-HH:MM；跨日则带日期。
pub fn format_datetime_range(
    start_date: &str,
    start_hour: f32,
    end_date: &str,
    end_hour: f32,
) -> String {
    if start_date == end_date {
        format!("{}-{}", fmt_hm(start_hour), fmt_hm(end_hour))
    } else {
        format!(
            "{} {} → {} {}",
            start_date,
            fmt_hm(start_hour),
            end_date,
            fmt_hm(end_hour)
        )
    }
}

/// 改工时：按开始日期时刻重算结束日期时刻。
pub fn sync_end_from_hours(
    start_date: &str,
    start_hour: f32,
    hours: f32,
    end_date: &mut String,
    end_hour: &mut f32,
) {
    let (ed, eh) = memo_core::task::end_from_duration(start_date, start_hour, hours.max(0.25));
    *end_date = ed;
    *end_hour = eh;
}

/// 改结束：若早于开始则推到开始+0.25h。
pub fn ensure_end_after_start(
    start_date: &str,
    start_hour: f32,
    end_date: &mut String,
    end_hour: &mut f32,
) {
    if memo_core::task::duration_hours(start_date, start_hour, end_date, *end_hour).is_err() {
        let (ed, eh) = memo_core::task::end_from_duration(start_date, start_hour, 0.25);
        *end_date = ed;
        *end_hour = eh;
    }
}

fn person_color(idx: usize) -> Color32 {
    const PALETTE: [Color32; 8] = [
        Color32::from_rgb(0x2F, 0x6F, 0xAD),
        Color32::from_rgb(0x2A, 0x9D, 0x8F),
        Color32::from_rgb(0xE9, 0xC4, 0x6A),
        Color32::from_rgb(0xF4, 0xA2, 0x61),
        Color32::from_rgb(0xE7, 0x6F, 0x51),
        Color32::from_rgb(0x6A, 0x4C, 0x93),
        Color32::from_rgb(0x45, 0x7B, 0x9D),
        Color32::from_rgb(0x58, 0x8B, 0x76),
    ];
    PALETTE[idx % PALETTE.len()]
}

fn person_index(persons: &[PersonView], id: &str) -> usize {
    persons.iter().position(|p| p.id == id).unwrap_or(0)
}

fn person_name<'a>(persons: &'a [PersonView], id: &str) -> &'a str {
    persons
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.name.as_str())
        .unwrap_or("?")
}

fn status_fg(status: TaskStatus) -> Color32 {
    match status {
        TaskStatus::NotStarted => theme::TEXT_MUTED,
        TaskStatus::InProgress => theme::ACCENT,
        TaskStatus::Paused => theme::WARN,
        TaskStatus::Blocked => Color32::from_rgb(0xC2, 0x41, 0x0C),
        TaskStatus::Cancelled => theme::TEXT_MUTED,
        TaskStatus::Done => theme::SUCCESS,
    }
}

/// Compact left navigator: mode + month/week nav + day task list.
pub fn show_left(
    ui: &mut egui::Ui,
    cal: &mut CalUi,
    tasks: &[TaskView],
    persons: &[PersonView],
) -> Option<String> {
    let mut clicked: Option<String> = None;

    ui.horizontal(|ui| {
        for (label, mode) in [
            ("月历", CalMode::Month),
            ("周视图", CalMode::Week),
            ("甘特", CalMode::Gantt),
        ] {
            let sel = cal.mode == mode;
            if ui
                .selectable_label(sel, RichText::new(label).size(13.0))
                .clicked()
            {
                cal.mode = mode;
                // overview in center; keep selection unless switching away from detail intent
            }
        }
    });
    ui.add_space(6.0);

    match cal.mode {
        CalMode::Month => {
            show_month_nav(ui, cal, tasks);
        }
        CalMode::Week | CalMode::Gantt => {
            show_week_nav(ui, cal);
        }
    }

    ui.add_space(8.0);
    ui.separator();
    ui.label(
        RichText::new(format!("当日 · {}", cal.selected_day))
            .strong()
            .size(13.0)
            .color(theme::TEXT),
    );
    let day_tasks: Vec<_> = tasks
        .iter()
        .filter(|t| t.spans_date(&cal.selected_day))
        .collect();
    let total: f32 = day_tasks.iter().map(|t| t.hours).sum();
    ui.label(theme::muted_label(format!("合计 {:.1} h · 点选查看详情", total)).small());
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if day_tasks.is_empty() {
                ui.label(theme::muted_label("当天暂无任务").small());
            }
            for t in day_tasks {
                let name = person_name(persons, &t.assignee_id);
                let sel = cal.selected_task.as_deref() == Some(t.id.as_str());
                let row_w = ui.available_width().max(40.0);
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(row_w, 48.0), Sense::click());
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                if ui.is_rect_visible(rect) {
                    let fill = theme::list_row_fill(sel, resp.hovered());
                    ui.painter().rect(
                        rect,
                        Rounding::same(theme::ROUND_CTRL),
                        fill,
                        Stroke::new(1.0, theme::BORDER),
                    );
                    let pad = 8.0;
                    ui.painter().text(
                        egui::pos2(rect.left() + pad, rect.top() + 6.0),
                        egui::Align2::LEFT_TOP,
                        format!(
                            "{}  {:.1}h  [{}]",
                            format_datetime_range(&t.date, t.start_hour, &t.end_date, t.end_hour),
                            t.hours,
                            t.status.short_label()
                        ),
                        egui::FontId::proportional(11.0),
                        status_fg(t.status),
                    );
                    ui.painter().text(
                        egui::pos2(rect.left() + pad, rect.top() + 22.0),
                        egui::Align2::LEFT_TOP,
                        format!("{} · {}", t.title, name),
                        egui::FontId::proportional(13.0),
                        theme::TEXT,
                    );
                }
                if resp.clicked() {
                    cal.selected_task = Some(t.id.clone());
                    clicked = Some(t.id.clone());
                }
                ui.add_space(4.0);
            }
        });

    clicked
}

/// Wide central overview when no task is selected.
pub fn show_central_overview(
    ui: &mut egui::Ui,
    cal: &mut CalUi,
    tasks: &[TaskView],
    persons: &[PersonView],
) -> Option<String> {
    match cal.mode {
        CalMode::Month => show_day_agenda(ui, cal, tasks, persons),
        CalMode::Week => show_week(ui, cal, tasks, persons, true),
        CalMode::Gantt => show_gantt(ui, cal, tasks, persons, true),
    }
}

fn show_month_nav(ui: &mut egui::Ui, cal: &mut CalUi, tasks: &[TaskView]) {
    ui.horizontal(|ui| {
        if ui.button("◀").clicked() {
            if cal.month == 1 {
                cal.month = 12;
                cal.year -= 1;
            } else {
                cal.month -= 1;
            }
        }
        ui.label(
            RichText::new(format!("{}-{:02}", cal.year, cal.month))
                .strong()
                .color(theme::TEXT),
        );
        if ui.button("▶").clicked() {
            if cal.month == 12 {
                cal.month = 1;
                cal.year += 1;
            } else {
                cal.month += 1;
            }
        }
        if ui.small_button("今天").clicked() {
            let t = Local::now().date_naive();
            cal.year = t.year();
            cal.month = t.month();
            cal.selected_day = t.format("%Y-%m-%d").to_string();
            cal.week_monday = monday_of(t).format("%Y-%m-%d").to_string();
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::muted_label("图例：").small());
        ui.label(RichText::new("工作日").small().color(theme::TEXT));
        ui.label(RichText::new("周末").small().color(Color32::from_rgb(0xDC, 0x26, 0x26)));
        ui.label(RichText::new("休=法定假").small().color(Color32::from_rgb(0xDC, 0x26, 0x26)));
        ui.label(RichText::new("班=调休上班").small().color(theme::ACCENT));
    });
    ui.add_space(4.0);

    let first = NaiveDate::from_ymd_opt(cal.year, cal.month, 1)
        .unwrap_or_else(|| Local::now().date_naive());
    let start = monday_of(first);

    // 表头与日期格共用同一列宽，避免 horizontal 内反复 available_width()/7 导致错位
    let cell_w = (ui.available_width() / 7.0).max(26.0);
    let col = cell_w - 2.0;

    ui.horizontal(|ui| {
        for name in ["一", "二", "三", "四", "五", "六", "日"] {
            ui.allocate_ui_with_layout(
                Vec2::new(col, 16.0),
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    let weekend = name == "六" || name == "日";
                    ui.label(
                        RichText::new(name)
                            .small()
                            .color(if weekend {
                                Color32::from_rgb(0xDC, 0x26, 0x26)
                            } else {
                                theme::TEXT_MUTED
                            }),
                    );
                },
            );
        }
    });

    let cell_h = 36.0_f32;
    let mut day = start;
    for _row in 0..6 {
        let month_of_row_start = day.month();
        ui.horizontal(|ui| {
            for _col in 0..7 {
                let ymd = day.format("%Y-%m-%d").to_string();
                let in_month = day.month() == cal.month;
                let info = china_calendar::classify(day);
                let day_hours: f32 = tasks
                    .iter()
                    .filter(|t| t.spans_date(&ymd))
                    .map(|t| t.hours)
                    .sum();
                let sel = cal.selected_day == ymd;
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(col, cell_h), Sense::click());
                if ui.is_rect_visible(rect) {
                    let fill = if sel {
                        theme::ACCENT_SOFT
                    } else if info.kind == DayKind::Holiday {
                        Color32::from_rgb(0xFE, 0xE2, 0xE2)
                    } else if info.kind == DayKind::MakeupWork {
                        Color32::from_rgb(0xDB, 0xEA, 0xFE)
                    } else if info.kind == DayKind::Weekend {
                        Color32::from_rgb(0xFE, 0xF2, 0xF2)
                    } else if resp.hovered() {
                        theme::PANEL
                    } else {
                        theme::CARD
                    };
                    ui.painter().rect(
                        rect,
                        Rounding::same(4.0),
                        fill,
                        Stroke::new(1.0, theme::BORDER),
                    );
                    let day_color = if !in_month {
                        theme::TEXT_MUTED
                    } else if info.is_rest() {
                        Color32::from_rgb(0xDC, 0x26, 0x26)
                    } else if info.kind == DayKind::MakeupWork {
                        theme::ACCENT
                    } else {
                        theme::TEXT
                    };
                    ui.painter().text(
                        egui::pos2(rect.left() + 4.0, rect.top() + 2.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}", day.day()),
                        egui::FontId::proportional(12.0),
                        day_color,
                    );
                    if let Some(badge) = info.badge() {
                        let badge_color = if info.kind == DayKind::MakeupWork {
                            theme::ACCENT
                        } else {
                            Color32::from_rgb(0xDC, 0x26, 0x26)
                        };
                        ui.painter().text(
                            egui::pos2(rect.right() - 3.0, rect.top() + 2.0),
                            egui::Align2::RIGHT_TOP,
                            badge,
                            egui::FontId::proportional(10.0),
                            badge_color,
                        );
                    }
                    if day_hours > 0.0 {
                        ui.painter().circle_filled(
                            egui::pos2(rect.center().x, rect.bottom() - 7.0),
                            2.5,
                            theme::ACCENT,
                        );
                    }
                }
                if resp.clicked() {
                    cal.selected_day = ymd.clone();
                    cal.week_monday = monday_of(day).format("%Y-%m-%d").to_string();
                    cal.selected_task = None;
                }
                let tip = format!("{} · {}", ymd, info.describe());
                let _ = resp.on_hover_text(tip);
                day += days(1);
            }
        });
        if month_of_row_start != cal.month && day.month() != cal.month && _row >= 4 {
            break;
        }
    }
}

fn show_week_nav(ui: &mut egui::Ui, cal: &mut CalUi) {
    let monday =
        parse_ymd(&cal.week_monday).unwrap_or_else(|| monday_of(Local::now().date_naive()));
    ui.horizontal(|ui| {
        if ui.button("◀").clicked() {
            cal.week_monday = (monday - days(7)).format("%Y-%m-%d").to_string();
        }
        if ui.small_button("本周").clicked() {
            let m = monday_of(Local::now().date_naive());
            cal.week_monday = m.format("%Y-%m-%d").to_string();
            cal.selected_day = Local::now().date_naive().format("%Y-%m-%d").to_string();
        }
        if ui.button("▶").clicked() {
            cal.week_monday = (monday + days(7)).format("%Y-%m-%d").to_string();
        }
    });
    ui.label(
        RichText::new(format!(
            "{} ~ {}",
            monday.format("%m-%d"),
            (monday + days(6)).format("%m-%d")
        ))
        .strong()
        .color(theme::TEXT),
    );
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for i in 0..7 {
            let d = monday + days(i);
            let ymd = d.format("%Y-%m-%d").to_string();
            let wd = ["一", "二", "三", "四", "五", "六", "日"][i as usize];
            let sel = cal.selected_day == ymd;
            let info = china_calendar::classify(d);
            let mark = info.badge().unwrap_or("");
            let label = if mark.is_empty() {
                format!("{wd}{}", d.day())
            } else {
                format!("{wd}{}{mark}", d.day())
            };
            let color = if info.is_rest() {
                Color32::from_rgb(0xDC, 0x26, 0x26)
            } else if info.kind == DayKind::MakeupWork {
                theme::ACCENT
            } else {
                theme::TEXT
            };
            if ui
                .selectable_label(sel, RichText::new(label).size(12.0).color(color))
                .on_hover_text(info.describe())
                .clicked()
            {
                cal.selected_day = ymd;
                cal.selected_task = None;
            }
        }
    });
}

/// Full-width day agenda with readable plan text.
fn show_day_agenda(
    ui: &mut egui::Ui,
    cal: &mut CalUi,
    tasks: &[TaskView],
    persons: &[PersonView],
) -> Option<String> {
    let mut clicked = None;
    let day_info = china_calendar::classify_ymd(&cal.selected_day)
        .unwrap_or(china_calendar::DayInfo {
            kind: DayKind::Workday,
            name: None,
        });
    ui.horizontal(|ui| {
        ui.heading(
            RichText::new(format!("{} 工作安排", cal.selected_day))
                .size(20.0)
                .color(theme::TEXT),
        );
        ui.label(
            RichText::new(day_info.describe())
                .size(14.0)
                .color(if day_info.is_rest() {
                    Color32::from_rgb(0xDC, 0x26, 0x26)
                } else if day_info.kind == DayKind::MakeupWork {
                    theme::ACCENT
                } else {
                    theme::TEXT_MUTED
                }),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let total: f32 = tasks
                .iter()
                .filter(|t| t.spans_date(&cal.selected_day))
                .map(|t| t.hours)
                .sum();
            ui.label(
                RichText::new(format!("合计 {:.1} 小时", total))
                    .size(14.0)
                    .color(theme::TEXT_MUTED),
            );
        });
    });
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);

    let day_tasks: Vec<_> = tasks
        .iter()
        .filter(|t| t.spans_date(&cal.selected_day))
        .collect();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if day_tasks.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(theme::muted_label("当天暂无任务，可点右上角「新建任务」").size(15.0));
                });
                return;
            }
            for t in day_tasks {
                let name = person_name(persons, &t.assignee_id);
                let sel = cal.selected_task.as_deref() == Some(t.id.as_str());
                let stroke = if sel {
                    Stroke::new(1.5, theme::ACCENT)
                } else {
                    Stroke::new(1.0, theme::BORDER)
                };
                let resp = egui::Frame::none()
                    .fill(theme::CARD)
                    .stroke(stroke)
                    .rounding(Rounding::same(theme::ROUND_CARD))
                    .inner_margin(Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format_datetime_range(
                                    &t.date,
                                    t.start_hour,
                                    &t.end_date,
                                    t.end_hour,
                                ))
                                    .strong()
                                    .size(14.0)
                                    .color(theme::ACCENT),
                            );
                            ui.label(
                                RichText::new(format!("{:.1}h", t.hours))
                                    .size(13.0)
                                    .color(theme::TEXT_MUTED),
                            );
                            ui.label(
                                RichText::new(name).size(13.0).color(theme::TEXT_MUTED),
                            );
                            ui.label(
                                RichText::new(t.status.label())
                                    .strong()
                                    .size(12.0)
                                    .color(status_fg(t.status)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("查看详情 ›")
                                            .small()
                                            .color(theme::ACCENT),
                                    );
                                },
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(&t.title)
                                .strong()
                                .size(17.0)
                                .color(theme::TEXT),
                        );
                        ui.add_space(6.0);
                        if t.plan.trim().is_empty() {
                            ui.label(theme::muted_label("（无工作计划）").size(14.0));
                        } else {
                            ui.label(
                                RichText::new(t.plan.as_str())
                                    .size(14.5)
                                    .color(theme::TEXT),
                            );
                        }
                    })
                    .response
                    .interact(Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    cal.selected_task = Some(t.id.clone());
                    clicked = Some(t.id.clone());
                }
                ui.add_space(10.0);
            }
        });

    clicked
}

fn show_week(
    ui: &mut egui::Ui,
    cal: &mut CalUi,
    tasks: &[TaskView],
    persons: &[PersonView],
    wide: bool,
) -> Option<String> {
    let mut clicked = None;
    let monday =
        parse_ymd(&cal.week_monday).unwrap_or_else(|| monday_of(Local::now().date_naive()));

    ui.horizontal(|ui| {
        if ui.button("◀ 上周").clicked() {
            cal.week_monday = (monday - days(7)).format("%Y-%m-%d").to_string();
        }
        if ui.small_button("本周").clicked() {
            cal.week_monday = monday_of(Local::now().date_naive())
                .format("%Y-%m-%d")
                .to_string();
        }
        if ui.button("下周 ▶").clicked() {
            cal.week_monday = (monday + days(7)).format("%Y-%m-%d").to_string();
        }
        ui.label(
            RichText::new(format!(
                "周视图  {} ~ {}",
                monday.format("%Y-%m-%d"),
                (monday + days(6)).format("%Y-%m-%d")
            ))
            .strong()
            .size(16.0)
            .color(theme::TEXT),
        );
        ui.label(theme::muted_label("点击色块打开任务详情").small());
    });
    ui.add_space(8.0);

    let col_w = (ui.available_width() / 7.0).max(if wide { 90.0 } else { 40.0 });
    let hour_start = 8.0_f32;
    let hour_end = 20.0_f32;
    let row_h = if wide { 28.0 } else { 14.0 };
    let header_h = 28.0_f32;
    let body_h = (hour_end - hour_start) * row_h;

    ui.horizontal(|ui| {
        for i in 0..7 {
            let d = monday + days(i);
            let ymd = d.format("%Y-%m-%d").to_string();
            let sel = cal.selected_day == ymd;
            let info = china_calendar::classify(d);
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(col_w - 2.0, header_h), Sense::click());
            let fill = if sel {
                theme::ACCENT_SOFT
            } else if info.kind == DayKind::Holiday {
                Color32::from_rgb(0xFE, 0xE2, 0xE2)
            } else if info.kind == DayKind::MakeupWork {
                Color32::from_rgb(0xDB, 0xEA, 0xFE)
            } else if info.kind == DayKind::Weekend {
                Color32::from_rgb(0xFE, 0xF2, 0xF2)
            } else {
                theme::PANEL
            };
            ui.painter().rect_filled(rect, Rounding::same(4.0), fill);
            let wd = ["一", "二", "三", "四", "五", "六", "日"][i as usize];
            let badge = info.badge().unwrap_or("");
            let text = if badge.is_empty() {
                format!("{} {}", wd, d.format("%m/%d"))
            } else {
                format!("{} {}{}", wd, d.format("%m/%d"), badge)
            };
            let color = if info.is_rest() {
                Color32::from_rgb(0xDC, 0x26, 0x26)
            } else if info.kind == DayKind::MakeupWork {
                theme::ACCENT
            } else {
                theme::TEXT
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(if wide { 13.0 } else { 11.0 }),
                color,
            );
            if resp.on_hover_text(info.describe()).clicked() {
                cal.selected_day = ymd;
                cal.selected_task = None;
            }
        }
    });

    let (body_rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), body_h), Sense::hover());
    ui.painter().rect(
        body_rect,
        Rounding::same(4.0),
        theme::CARD,
        Stroke::new(1.0, theme::BORDER),
    );

    // hour labels on left edge of first column when wide
    if wide {
        for h in (hour_start as i32)..(hour_end as i32) {
            let y = body_rect.top() + (h as f32 - hour_start) * row_h;
            ui.painter().text(
                egui::pos2(body_rect.left() + 4.0, y + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{h:02}:00"),
                egui::FontId::proportional(10.0),
                theme::TEXT_MUTED,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(body_rect.left(), y),
                    egui::pos2(body_rect.right(), y),
                ],
                Stroke::new(1.0, Color32::from_rgb(0xF1, 0xF5, 0xF9)),
            );
        }
    }

    for i in 0..7 {
        let d = monday + days(i);
        let ymd = d.format("%Y-%m-%d").to_string();
        let x0 = body_rect.left() + i as f32 * col_w;
        let col = egui::Rect::from_min_size(
            egui::pos2(x0, body_rect.top()),
            Vec2::new(col_w - 2.0, body_h),
        );
        ui.painter().line_segment(
            [col.left_top(), col.left_bottom()],
            Stroke::new(1.0, theme::BORDER),
        );

        for t in tasks.iter().filter(|t| t.spans_date(&ymd)) {
            let y = ((t.start_hour - hour_start).clamp(0.0, hour_end - hour_start)) * row_h;
            let h = (t.hours * row_h).clamp(if wide { 22.0 } else { 12.0 }, body_h - y);
            let r = egui::Rect::from_min_size(
                egui::pos2(col.left() + 2.0, col.top() + y),
                Vec2::new((col.width() - 4.0).max(8.0), h),
            );
            let color = person_color(person_index(persons, &t.assignee_id));
            ui.painter()
                .rect_filled(r, Rounding::same(4.0), color.linear_multiply(0.9));
            let label = if wide {
                format!("[{}] {} ({:.1}h)", t.status.short_label(), t.title, t.hours)
            } else {
                format!("[{}]{}", t.status.short_label(), t.title)
            };
            ui.painter().text(
                egui::pos2(r.left() + 4.0, r.top() + 3.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(if wide { 12.0 } else { 10.0 }),
                Color32::WHITE,
            );
            let id = ui.interact(r, ui.id().with(("w", &t.id)), Sense::click());
            if id.on_hover_text(&t.plan).clicked() {
                cal.selected_task = Some(t.id.clone());
                cal.selected_day = ymd.clone();
                clicked = Some(t.id.clone());
            }
        }
    }

    clicked
}

fn show_gantt(
    ui: &mut egui::Ui,
    cal: &mut CalUi,
    tasks: &[TaskView],
    persons: &[PersonView],
    wide: bool,
) -> Option<String> {
    let mut clicked = None;
    let monday =
        parse_ymd(&cal.week_monday).unwrap_or_else(|| monday_of(Local::now().date_naive()));

    ui.horizontal(|ui| {
        if ui.button("◀ 上周").clicked() {
            cal.week_monday = (monday - days(7)).format("%Y-%m-%d").to_string();
        }
        if ui.small_button("本周").clicked() {
            cal.week_monday = monday_of(Local::now().date_naive())
                .format("%Y-%m-%d")
                .to_string();
        }
        if ui.button("下周 ▶").clicked() {
            cal.week_monday = (monday + days(7)).format("%Y-%m-%d").to_string();
        }
        ui.label(
            RichText::new("甘特 · 人员 × 日")
                .strong()
                .size(16.0)
                .color(theme::TEXT),
        );
        ui.label(theme::muted_label("条宽∝工时 · 点击打开详情").small());
    });
    ui.add_space(8.0);

    let name_w = if wide { 96.0 } else { 72.0 };
    let col_w = ((ui.available_width() - name_w - 56.0) / 7.0).max(if wide { 72.0 } else { 36.0 });
    let row_h = if wide { 52.0 } else { 40.0 };

    ui.horizontal(|ui| {
        ui.allocate_exact_size(Vec2::new(name_w, 20.0), Sense::hover());
        for i in 0..7 {
            let d = monday + days(i);
            ui.allocate_ui_with_layout(
                Vec2::new(col_w, 20.0),
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.label(
                        RichText::new(format!("{}", d.format("%m/%d")))
                            .small()
                            .color(theme::TEXT_MUTED),
                    );
                },
            );
        }
        ui.label(theme::muted_label("合计").small());
    });

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if persons.is_empty() {
                ui.label(theme::muted_label("请先添加人员").size(14.0));
                return;
            }
            for (pi, p) in persons.iter().enumerate() {
                let mut week_hours = 0.0_f32;
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(name_w, row_h),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                RichText::new(&p.name)
                                    .strong()
                                    .size(if wide { 14.0 } else { 12.0 })
                                    .color(theme::TEXT),
                            );
                        },
                    );
                    for i in 0..7 {
                        let d = monday + days(i);
                        let ymd = d.format("%Y-%m-%d").to_string();
                        let day_tasks: Vec<_> = tasks
                            .iter()
                            .filter(|t| t.spans_date(&ymd) && t.assignee_id == p.id)
                            .collect();
                        let day_h: f32 = day_tasks.iter().map(|t| t.hours).sum();
                        week_hours += day_h;
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(col_w - 2.0, row_h), Sense::hover());
                        ui.painter().rect(
                            rect,
                            Rounding::same(4.0),
                            theme::CARD,
                            Stroke::new(1.0, theme::BORDER),
                        );
                        if day_h > 0.0 {
                            let bar_w =
                                (rect.width() * (day_h / 8.0).clamp(0.18, 1.0)).max(10.0);
                            let bar = egui::Rect::from_min_size(
                                egui::pos2(rect.left() + 3.0, rect.center().y - 10.0),
                                Vec2::new(bar_w.min(rect.width() - 6.0), 20.0),
                            );
                            let color = person_color(pi);
                            ui.painter().rect_filled(bar, Rounding::same(4.0), color);
                            let bar_label = if wide && day_tasks.len() == 1 {
                                format!(
                                    "[{}] {} {:.0}h",
                                    day_tasks[0].status.short_label(),
                                    day_tasks[0].title,
                                    day_h
                                )
                            } else if day_tasks.len() == 1 {
                                format!("[{}]{:.0}h", day_tasks[0].status.short_label(), day_h)
                            } else {
                                format!("{:.0}h", day_h)
                            };
                            ui.painter().text(
                                bar.center(),
                                egui::Align2::CENTER_CENTER,
                                bar_label,
                                egui::FontId::proportional(11.0),
                                Color32::WHITE,
                            );
                            if let Some(t) = day_tasks.first() {
                                let tip = format!("{} — {}", t.title, t.plan);
                                let id =
                                    ui.interact(bar, ui.id().with(("g", &t.id)), Sense::click());
                                if id.on_hover_text(tip).clicked() {
                                    cal.selected_task = Some(t.id.clone());
                                    cal.selected_day = ymd;
                                    clicked = Some(t.id.clone());
                                }
                            }
                        }
                    }
                    ui.label(
                        RichText::new(format!("{:.1}h", week_hours))
                            .strong()
                            .size(13.0)
                            .color(theme::TEXT),
                    );
                });
                ui.add_space(6.0);
            }
        });

    clicked
}
