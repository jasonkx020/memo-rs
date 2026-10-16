//! 主界面月历：窄导航、日/周工作区、过期预警与日历化详情抽屉。

use crate::date_field;
use crate::doc_editor;
use crate::huangli;
use crate::memo_form;
use crate::page_shell;
use crate::theme;
use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use eframe::egui::{self, Color32, Frame, Id, Margin, Order, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::service::MemoView;
use memo_core::store::{MemoCategory, MemoPriority};
use memo_core::{
    covers_calendar_day, deadline_date, due_date_part, due_time_hm, event_end_date, is_due_today,
    is_overdue, overdue_days,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use crate::msg::BgMsg;
use memo_core::service::MemoService;
use memo_core::store::MemoLifecycle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DayFilter {
    Open,
    All,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalScope {
    Day,
    Week,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalListLayout {
    List,
    Grid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalUiPrefs {
    scope: CalScope,
    list_layout: CalListLayout,
}

pub struct CalendarUi {
    pub year: i32,
    pub month: u32,
    pub selected: NaiveDate,
    pub filter: DayFilter,
    /// `+N 更多` 弹层所在日
    pub more_popup_day: Option<NaiveDate>,
    pub scope: CalScope,
    pub list_layout: CalListLayout,
    /// 周视图所在周的周一
    pub week_monday: NaiveDate,
    /// 下一帧滚到日议程「已过期」区块
    scroll_overdue: bool,
    /// 偏好落盘脏标记
    prefs_dirty: bool,
}

impl Default for CalendarUi {
    fn default() -> Self {
        let t = Local::now().date_naive();
        Self {
            year: t.year(),
            month: t.month(),
            selected: t,
            filter: DayFilter::All,
            more_popup_day: None,
            scope: CalScope::Day,
            list_layout: CalListLayout::List,
            week_monday: monday_of(t),
            scroll_overdue: false,
            prefs_dirty: false,
        }
    }
}

fn monday_of(d: NaiveDate) -> NaiveDate {
    let wd = d.weekday().num_days_from_monday() as i64;
    d - Duration::try_days(wd).unwrap_or_default()
}

fn add_days(d: NaiveDate, n: i64) -> NaiveDate {
    d + Duration::try_days(n).unwrap_or_default()
}

fn cal_prefs_path(data_dir: &Path, person_fp: &str) -> PathBuf {
    data_dir
        .join("identities")
        .join(person_fp)
        .join("calendar_ui.json")
}

impl CalendarUi {
    pub fn load_prefs(&mut self, data_dir: &Path, person_fp: &str) {
        let path = cal_prefs_path(data_dir, person_fp);
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(p) = serde_json::from_str::<CalUiPrefs>(&raw) else {
            return;
        };
        self.scope = p.scope;
        self.list_layout = p.list_layout;
        self.prefs_dirty = false;
    }

    pub fn save_prefs_if_dirty(&mut self, data_dir: &Path, person_fp: &str) {
        if !self.prefs_dirty {
            return;
        }
        let path = cal_prefs_path(data_dir, person_fp);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let p = CalUiPrefs {
            scope: self.scope,
            list_layout: self.list_layout,
        };
        if let Ok(raw) = serde_json::to_string_pretty(&p) {
            let _ = std::fs::write(path, raw);
            self.prefs_dirty = false;
        }
    }

    fn mark_prefs_dirty(&mut self) {
        self.prefs_dirty = true;
    }
}

fn jump_cal_to(cal: &mut CalendarUi, day: NaiveDate) {
    cal.year = day.year();
    cal.month = day.month();
    cal.selected = day;
    cal.week_monday = monday_of(day);
    cal.scope = CalScope::Day;
    cal.mark_prefs_dirty();
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn on_calendar(m: &MemoView) -> bool {
    if m.category.canonical() == MemoCategory::Credentials {
        return false;
    }
    if m.category.is_gender_private() {
        return false;
    }
    due_date_part(&m.due_date).is_some()
}

fn calendar_open<'a>(list: &'a [MemoView]) -> impl Iterator<Item = &'a MemoView> {
    list.iter().filter(|m| on_calendar(m) && !m.done)
}

fn unscheduled<'a>(list: &'a [MemoView], q: &str) -> Vec<&'a MemoView> {
    list.iter()
        .filter(|m| {
            m.category.canonical() != MemoCategory::Credentials
                && !m.category.is_gender_private()
                && due_date_part(&m.due_date).is_none()
                && search_hit(m, q)
        })
        .collect()
}

fn search_hit(m: &MemoView, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let tags = m.tags.join(" ").to_lowercase();
    m.title.to_lowercase().contains(q) || m.content.to_lowercase().contains(q) || tags.contains(q)
}

fn events_on<'a>(list: &'a [MemoView], day: NaiveDate, q: &str) -> Vec<&'a MemoView> {
    list.iter()
        .filter(|m| {
            on_calendar(m) && covers_calendar_day(&m.due_date, &m.end_date, day) && search_hit(m, q)
        })
        .collect()
}

fn is_span(m: &MemoView) -> bool {
    match (
        due_date_part(&m.due_date),
        event_end_date(&m.due_date, &m.end_date),
    ) {
        (Some(a), Some(b)) => b > a,
        _ => false,
    }
}

fn memo_overdue(m: &MemoView) -> bool {
    is_overdue(&m.due_date, &m.end_date, m.done)
}

fn memo_due_today(m: &MemoView) -> bool {
    is_due_today(&m.due_date, &m.end_date, m.done)
}

fn collect_overdue<'a>(all: &'a [MemoView], q: &str) -> Vec<&'a MemoView> {
    let mut v: Vec<_> = calendar_open(all)
        .filter(|m| memo_overdue(m) && search_hit(m, q))
        .collect();
    v.sort_by_key(|m| deadline_date(&m.due_date, &m.end_date));
    v
}

/// 任务页过期条数（顶栏告警用，与议程过期列表同口径）。
pub fn task_overdue_count(all: &[MemoView]) -> usize {
    collect_overdue(all, "").len()
}

fn collect_due_today<'a>(all: &'a [MemoView], q: &str) -> Vec<&'a MemoView> {
    calendar_open(all)
        .filter(|m| memo_due_today(m) && search_hit(m, q))
        .collect()
}

fn weekday_zh(d: NaiveDate) -> &'static str {
    match d.weekday() {
        Weekday::Mon => "周一",
        Weekday::Tue => "周二",
        Weekday::Wed => "周三",
        Weekday::Thu => "周四",
        Weekday::Fri => "周五",
        Weekday::Sat => "周六",
        Weekday::Sun => "周日",
    }
}

fn format_md(d: NaiveDate) -> String {
    format!("{}月{}日", d.month(), d.day())
}

fn format_mmdd(d: NaiveDate) -> String {
    format!("{:02}-{:02}", d.month(), d.day())
}

fn content_excerpt(m: &MemoView, max_chars: usize) -> Option<String> {
    let t = m.content.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    Some(truncate_title(t, max_chars))
}

/// 左栏顶统计四段：共 / 待办 / 过期 / 已完成。
#[derive(Clone, Copy)]
struct TaskRailStats {
    total: usize,
    pending: usize,
    overdue: usize,
    done: usize,
}

fn task_rail_stats(cal: &CalendarUi, all: &[MemoView], q: &str) -> TaskRailStats {
    match cal.scope {
        CalScope::Day => {
            let overdue = collect_overdue(all, q).len();
            let day_events = events_on(all, cal.selected, q);
            let day_list: Vec<_> = day_events
                .iter()
                .copied()
                .filter(|m| match cal.filter {
                    DayFilter::Open => !m.done,
                    DayFilter::All => true,
                    DayFilter::Done => m.done,
                })
                .collect();
            let pending = day_list
                .iter()
                .filter(|m| !m.done && !memo_overdue(m))
                .count();
            let done = day_list.iter().filter(|m| m.done).count();
            TaskRailStats {
                total: day_list.len(),
                pending,
                overdue,
                done,
            }
        }
        CalScope::Week => {
            let mut week_items: Vec<&MemoView> = Vec::new();
            for i in 0..7 {
                let d = add_days(cal.week_monday, i);
                week_items.extend(events_on(all, d, q));
            }
            week_items.sort_by(|a, b| a.id.cmp(&b.id));
            week_items.dedup_by(|a, b| a.id == b.id);
            let pending = week_items.iter().filter(|m| !m.done).count();
            let overdue = week_items.iter().filter(|m| memo_overdue(m)).count();
            let done = week_items.iter().filter(|m| m.done).count();
            TaskRailStats {
                total: week_items.len(),
                pending,
                overdue,
                done,
            }
        }
    }
}

/// 任务左栏顶行：标题 · 分色今日/共/待办/过期/已完成 · + 新建。
fn show_task_rail_header(
    ui: &mut egui::Ui,
    today_n: usize,
    stats: TaskRailStats,
    open_new: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            RichText::new("任务")
                .size(18.0)
                .strong()
                .color(theme::text()),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let seg = 13.0;
            ui.label(
                RichText::new(format!("今日 {today_n}"))
                    .size(seg)
                    .color(theme::shell_accent()),
            );
            let sep = |ui: &mut egui::Ui| {
                ui.label(
                    RichText::new(" · ")
                        .size(seg)
                        .color(theme::text_muted()),
                );
            };
            sep(ui);
            ui.label(
                RichText::new(format!("共 {} 条", stats.total))
                    .size(seg)
                    .color(theme::text_muted()),
            );
            sep(ui);
            ui.label(
                RichText::new(format!("待办 {}", stats.pending))
                    .size(seg)
                    .color(theme::warn_due_today_fg()),
            );
            sep(ui);
            ui.label(
                RichText::new(format!("过期 {}", stats.overdue))
                    .size(seg)
                    .strong()
                    .color(if stats.overdue == 0 {
                        theme::text_muted()
                    } else {
                        theme::warn_overdue_fg()
                    }),
            );
            sep(ui);
            ui.label(
                RichText::new(format!("已完成 {}", stats.done))
                    .size(seg)
                    .color(theme::success()),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("+ 新建")
                            .color(Color32::WHITE)
                            .strong()
                            .size(13.0),
                    )
                    .fill(theme::shell_accent())
                    .rounding(Rounding::same(7.0))
                    .min_size(Vec2::new(64.0, 28.0)),
                )
                .clicked()
            {
                *open_new = true;
            }
        });
    });
}

pub fn show(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    cal: &mut CalendarUi,
    all: &[MemoView],
    search: &str,
    selected: &mut Option<String>,
    editing: &mut bool,
    open_new: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    pick_delete: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    let q = search.trim().to_lowercase();
    let today_n = collect_due_today(all, &q).len();
    let rail_stats = task_rail_stats(cal, all, &q);
    // 页头进左栏，「+ 新建」不占右栏高度，日历顶对齐上移
    let (side_w, cal_w, full_h) = split_cal_detail(ui);

    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), full_h),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.set_max_height(full_h);
            ui.set_clip_rect(ui.max_rect());
            ui.spacing_mut().item_spacing.x = page_shell::PAGE_GAP;
            ui.allocate_ui_with_layout(
                Vec2::new(side_w, full_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(side_w);
                    ui.set_max_height(full_h);
                    ui.set_clip_rect(ui.max_rect());
                    show_task_rail_header(ui, today_n, rail_stats, open_new);
                    ui.add_space(page_shell::PAGE_GAP);
                    show_work_side(
                        ui,
                        svc,
                        cal,
                        all,
                        &q,
                        selected,
                        editing,
                        open_new,
                        picked,
                        pick_edit,
                        pick_delete,
                        open_sticky,
                        tx,
                    );
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(cal_w, full_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(cal_w);
                    ui.set_min_height(full_h);
                    ui.set_max_height(full_h);
                    ui.set_clip_rect(ui.max_rect());
                    show_month(ui, cal, all, &q, selected, editing, open_new, picked);
                },
            );
        },
    );
}

/// 右侧工作区：日议程或周任务。
fn show_work_side(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    cal: &mut CalendarUi,
    all: &[MemoView],
    q: &str,
    selected: &mut Option<String>,
    editing: &mut bool,
    open_new: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    pick_delete: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    show_scope_header(ui, cal);
    ui.add_space(4.0);
    match cal.scope {
        CalScope::Day => show_day_side(
            ui,
            svc,
            cal,
            all,
            q,
            selected,
            editing,
            open_new,
            picked,
            pick_edit,
            pick_delete,
            open_sticky,
            tx,
        ),
        CalScope::Week => show_week_side(
            ui,
            svc,
            cal,
            all,
            q,
            selected,
            editing,
            picked,
            pick_edit,
            pick_delete,
            open_sticky,
            tx,
        ),
    }
}

fn show_scope_header(ui: &mut egui::Ui, cal: &mut CalendarUi) {
    ui.horizontal(|ui| {
        // 日 | 周
        let day_on = cal.scope == CalScope::Day;
        let week_on = cal.scope == CalScope::Week;
        Frame::none()
            .fill(theme::panel())
            .stroke(Stroke::new(1.0, theme::border()))
            .rounding(Rounding::same(8.0))
            .inner_margin(Margin::same(2.0))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let day_btn = egui::Button::new(
                    RichText::new("日")
                        .size(12.0)
                        .strong()
                        .color(if day_on {
                            theme::shell_accent()
                        } else {
                            theme::text_muted()
                        }),
                )
                .fill(if day_on {
                    Color32::WHITE
                } else {
                    Color32::TRANSPARENT
                })
                .rounding(Rounding::same(6.0))
                .min_size(Vec2::new(32.0, 26.0));
                if ui.add(day_btn).clicked() {
                    cal.scope = CalScope::Day;
                    cal.mark_prefs_dirty();
                }
                let week_btn = egui::Button::new(
                    RichText::new("周")
                        .size(12.0)
                        .strong()
                        .color(if week_on {
                            theme::shell_accent()
                        } else {
                            theme::text_muted()
                        }),
                )
                .fill(if week_on {
                    Color32::WHITE
                } else {
                    Color32::TRANSPARENT
                })
                .rounding(Rounding::same(6.0))
                .min_size(Vec2::new(32.0, 26.0));
                if ui.add(week_btn).clicked() {
                    cal.week_monday = monday_of(cal.selected);
                    cal.scope = CalScope::Week;
                    cal.mark_prefs_dirty();
                }
            });

        let title = match cal.scope {
            CalScope::Day => format!(
                "{} · {}",
                format_md(cal.selected),
                weekday_zh(cal.selected)
            ),
            CalScope::Week => {
                let end = add_days(cal.week_monday, 6);
                format!("{} – {}", format_md(cal.week_monday), format_md(end))
            }
        };
        ui.label(
            RichText::new(title)
                .size(16.0)
                .strong()
                .color(theme::text()),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match cal.scope {
                CalScope::Day => {
                    let list_on = cal.list_layout == CalListLayout::List;
                    let grid_on = cal.list_layout == CalListLayout::Grid;
                    Frame::none()
                        .fill(theme::panel())
                        .stroke(Stroke::new(1.0, theme::border()))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::same(2.0))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for (label, on, layout) in [
                                ("宫格", grid_on, CalListLayout::Grid),
                                ("列表", list_on, CalListLayout::List),
                            ] {
                                let b = egui::Button::new(
                                    RichText::new(label)
                                        .size(12.0)
                                        .strong()
                                        .color(if on {
                                            theme::shell_accent()
                                        } else {
                                            theme::text_muted()
                                        }),
                                )
                                .fill(if on {
                                    Color32::WHITE
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(40.0, 26.0));
                                if ui.add(b).clicked() {
                                    cal.list_layout = layout;
                                    cal.mark_prefs_dirty();
                                }
                            }
                        });
                }
                CalScope::Week => {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("本周").size(12.0))
                                .fill(Color32::WHITE)
                                .stroke(Stroke::new(1.0, theme::border()))
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(44.0, 26.0)),
                        )
                        .clicked()
                    {
                        cal.week_monday = monday_of(today());
                        cal.selected = today();
                        cal.year = cal.selected.year();
                        cal.month = cal.selected.month();
                    }
                    if ui
                        .add(
                            egui::Button::new("›")
                                .fill(Color32::WHITE)
                                .stroke(Stroke::new(1.0, theme::border()))
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(28.0, 26.0)),
                        )
                        .clicked()
                    {
                        cal.week_monday = add_days(cal.week_monday, 7);
                        cal.selected = cal.week_monday;
                        cal.year = cal.selected.year();
                        cal.month = cal.selected.month();
                    }
                    if ui
                        .add(
                            egui::Button::new("‹")
                                .fill(Color32::WHITE)
                                .stroke(Stroke::new(1.0, theme::border()))
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(28.0, 26.0)),
                        )
                        .clicked()
                    {
                        cal.week_monday = add_days(cal.week_monday, -7);
                        cal.selected = cal.week_monday;
                        cal.year = cal.selected.year();
                        cal.month = cal.selected.month();
                    }
                }
            }
        });
    });
}

/// 右月历约 420 固定 / 左议程吃剩余（与私密共用 `page_shell::page_rail_split`）。
pub(crate) fn split_cal_detail(ui: &egui::Ui) -> (f32, f32, f32) {
    let avail = ui.available_width().max(page_shell::PAGE_GAP + 2.0);
    let (side_w, cal_w) = page_shell::page_rail_split(avail);
    let full_h = ui.available_height().max(80.0);
    (side_w, cal_w, full_h)
}

pub(crate) fn show_month(
    ui: &mut egui::Ui,
    cal: &mut CalendarUi,
    all: &[MemoView],
    q: &str,
    selected: &mut Option<String>,
    editing: &mut bool,
    open_new: &mut bool,
    picked: &mut Option<String>,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{}年{}月", cal.year, cal.month))
                .size(16.0)
                .strong()
                .color(theme::text()),
        );
        // 「+ 新建」已上移到页头；左栏只保留月历导航
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new("今天").min_size(Vec2::new(44.0, 26.0)))
                .clicked()
            {
                let t = today();
                cal.year = t.year();
                cal.month = t.month();
                cal.selected = t;
            }
            if ui
                .add_sized(Vec2::new(28.0, 26.0), egui::Button::new("›"))
                .clicked()
            {
                shift_month(cal, 1);
            }
            if ui
                .add_sized(Vec2::new(28.0, 26.0), egui::Button::new("‹"))
                .clicked()
            {
                shift_month(cal, -1);
            }
        });
    });
    ui.add_space(4.0);
    show_huangli_row(ui, cal.selected);
    ui.add_space(6.0);
    // 网格区关掉默认行距，只用显式 gap，避免底部多出空隙或溢出
    ui.spacing_mut().item_spacing.y = 0.0;
    let gap = 3.0_f32;
    let cell_w = ((ui.available_width() - gap * 6.0) / 7.0).max(1.0);
    ui.columns(7, |cols| {
        for (i, name) in ["一", "二", "三", "四", "五", "六", "日"].iter().enumerate() {
            cols[i].label(
                RichText::new(*name)
                    .size(11.0)
                    .strong()
                    .color(theme::text_muted()),
            );
        }
    });
    // 纵向刚好填满剩余高度：6 行 + 5 个 gap
    let n_rows = 6_usize;
    let remain = ui.available_height().max(1.0);
    let cell_h = ((remain - gap * (n_rows - 1) as f32) / n_rows as f32).max(1.0);
    let first = NaiveDate::from_ymd_opt(cal.year, cal.month, 1).unwrap_or_else(today);
    let pad = first.weekday().num_days_from_monday() as i64;
    let start = first - Duration::try_days(pad).unwrap_or_default();
    for week in 0..n_rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.spacing_mut().item_spacing.y = 0.0;
            for d in 0..7 {
                let day = start + Duration::try_days((week * 7 + d) as i64).unwrap_or_default();
                day_cell(
                    ui,
                    cal,
                    all,
                    q,
                    day,
                    cell_w,
                    cell_h,
                    selected,
                    editing,
                    open_new,
                    picked,
                );
            }
        });
        if week + 1 < n_rows {
            ui.add_space(gap);
        }
    }
}

fn shift_month(cal: &mut CalendarUi, delta: i32) {
    let mut m = cal.month as i32 + delta;
    let mut y = cal.year;
    while m < 1 {
        m += 12;
        y -= 1;
    }
    while m > 12 {
        m -= 12;
        y += 1;
    }
    cal.year = y;
    cal.month = m as u32;
}

fn day_cell(
    ui: &mut egui::Ui,
    cal: &mut CalendarUi,
    all: &[MemoView],
    q: &str,
    day: NaiveDate,
    cell_w: f32,
    cell_h: f32,
    selected: &mut Option<String>,
    editing: &mut bool,
    open_new: &mut bool,
    picked: &mut Option<String>,
) {
    let t = today();
    let in_month = day.month() == cal.month && day.year() == cal.year;
    let is_sel = day == cal.selected;
    let h = cell_h.max(1.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(cell_w.max(1.0), h), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let past = day < t;
    let in_week = cal.scope == CalScope::Week
        && day >= cal.week_monday
        && day <= add_days(cal.week_monday, 6);
    let fill = if is_sel {
        theme::accent_soft()
    } else if day == t {
        theme::calendar_today_bg()
    } else if in_week {
        theme::accent_soft().linear_multiply(0.45)
    } else if past {
        theme::calendar_past_bg()
    } else {
        theme::calendar_future_bg()
    };
    ui.painter().rect(
        rect,
        Rounding::same(6.0),
        fill,
        Stroke::new(
            if is_sel { 1.2 } else { 1.0 },
            if is_sel {
                theme::shell_accent()
            } else {
                theme::border()
            },
        ),
    );
    if !in_month {
        ui.painter().rect_filled(
            rect,
            Rounding::same(6.0),
            Color32::from_white_alpha(90),
        );
    }

    let on = events_on(all, day, q);
    let has_overdue = on.iter().any(|m| memo_overdue(m));

    // 日期数字（选中：蓝圆）
    let num = format!("{}", day.day());
    let num_center = egui::pos2(rect.left() + 14.0, rect.top() + 12.0);
    if is_sel {
        ui.painter()
            .circle_filled(num_center, 10.0, theme::shell_accent());
        ui.painter().text(
            num_center,
            egui::Align2::CENTER_CENTER,
            num,
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    } else {
        let num_fg = if !in_month {
            theme::text_muted()
        } else if day == t {
            theme::calendar_today_fg()
        } else {
            theme::text()
        };
        ui.painter().text(
            egui::pos2(rect.left() + 8.0, rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            num,
            egui::FontId::proportional(12.0),
            num_fg,
        );
    }
    if has_overdue {
        ui.painter().circle_filled(
            egui::pos2(rect.right() - 8.0, rect.top() + 10.0),
            3.5,
            theme::warn_overdue_fg(),
        );
    }

    // 农历/节气小字（轻量路径，禁止在此算宜忌）
    {
        let mark = huangli::cell_mark(day);
        let lunar_fg = if !in_month {
            theme::text_muted().linear_multiply(0.55)
        } else if mark.is_jie_qi {
            theme::shell_accent()
        } else {
            theme::text_muted()
        };
        ui.painter().text(
            egui::pos2(rect.left() + 8.0, rect.top() + 18.0),
            egui::Align2::LEFT_TOP,
            truncate_title(&mark.text, 4),
            egui::FontId::proportional(10.0),
            lunar_fg,
        );
    }

    // 导航密度：细跨日条 + 色点 + 数量（点数量打开更多弹层）
    let spans: Vec<_> = on.iter().copied().filter(|m| is_span(m)).collect();
    let mut chip_hit = false;
    let mut y = rect.top() + 30.0;
    let bottom = rect.bottom() - 2.0;

    if let Some(m) = spans.first() {
        if y + 8.0 <= bottom - 14.0 {
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 4.0, y),
                Vec2::new(rect.width() - 8.0, 6.0),
            );
            let fill_c = if m.done {
                theme::text_muted().linear_multiply(0.55)
            } else if memo_overdue(m) {
                theme::warn_overdue_fg()
            } else {
                theme::category_icon_color(m.category.canonical())
            };
            ui.painter()
                .rect_filled(bar, Rounding::same(2.0), fill_c);
            let id = ui.id().with(("span", day, &m.id));
            let r = ui.interact(bar, id, Sense::click());
            if r.clicked() {
                chip_hit = true;
                select_calendar_day(cal, day, selected, editing);
            }
            day_cell_new_memo_menu(&r, cal, day, selected, editing, open_new);
            y += 10.0;
        }
    }

    let mut ordered: Vec<_> = on.iter().copied().filter(|m| !m.done).collect();
    ordered.extend(on.iter().copied().filter(|m| m.done));
    let dot_y = (rect.bottom() - 12.0).max(y + 4.0);
    let mut x = rect.left() + 7.0;
    let max_dots = 4usize;
    for m in ordered.iter().take(max_dots) {
        let c = if m.done {
            theme::text_muted().linear_multiply(0.7)
        } else if memo_overdue(m) {
            theme::warn_overdue_fg()
        } else if m.priority == MemoPriority::High {
            theme::warn_overdue_fg()
        } else {
            theme::category_icon_color(m.category.canonical())
        };
        ui.painter()
            .circle_filled(egui::pos2(x, dot_y), 3.0, c);
        x += 8.0;
    }

    if !on.is_empty() {
        let count_pos = egui::pos2(rect.right() - 5.0, rect.bottom() - 4.0);
        ui.painter().text(
            count_pos,
            egui::Align2::RIGHT_BOTTOM,
            format!("{}", on.len()),
            egui::FontId::proportional(9.0),
            theme::text_muted(),
        );
        let more_rect = egui::Rect::from_min_max(
            egui::pos2(rect.right() - 22.0, rect.bottom() - 16.0),
            rect.right_bottom(),
        );
        let id = ui.id().with(("more", day));
        let r = ui.interact(more_rect, id, Sense::click());
        if r.clicked() {
            chip_hit = true;
            cal.selected = day;
            cal.week_monday = monday_of(day);
            cal.more_popup_day = Some(day);
        }
        day_cell_new_memo_menu(&r, cal, day, selected, editing, open_new);
        if cal.more_popup_day == Some(day) {
            show_more_popup(ui, cal, &on, day, more_rect, selected, editing, picked);
        }
    } else if cal.more_popup_day == Some(day) {
        cal.more_popup_day = None;
    }

    if resp.clicked() && !chip_hit {
        select_calendar_day(cal, day, selected, editing);
    }
    day_cell_new_memo_menu(&resp, cal, day, selected, editing, open_new);
}

/// 日格子任意位置（空白 / 备忘条 / 更多）右键 → 新建备忘（关联该日）。
fn day_cell_new_memo_menu(
    resp: &egui::Response,
    cal: &mut CalendarUi,
    day: NaiveDate,
    selected: &mut Option<String>,
    editing: &mut bool,
    open_new: &mut bool,
) {
    resp.context_menu(|ui| {
        theme::style_context_menu(ui);
        if theme::menu_item_mark(ui, "+", "新建任务").clicked() {
            select_calendar_day(cal, day, selected, editing);
            *open_new = true;
            ui.close_menu();
        }
    });
}

/// 选中日历某一天并收起详情（不打开备忘）。
fn select_calendar_day(
    cal: &mut CalendarUi,
    day: NaiveDate,
    selected: &mut Option<String>,
    editing: &mut bool,
) {
    cal.selected = day;
    cal.week_monday = monday_of(day);
    *selected = None;
    *editing = false;
    cal.more_popup_day = None;
}

fn truncate_title(title: &str, max_chars: usize) -> String {
    let t = if title.trim().is_empty() {
        "(无标题)"
    } else {
        title.trim()
    };
    let mut out = String::new();
    for (i, ch) in t.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// 日历色块上的完成划线（painter 文本无 strikethrough）。
#[allow(dead_code)]
fn paint_strike_line(ui: &egui::Ui, bar: egui::Rect, color: Color32) {
    let y = bar.center().y;
    ui.painter().hline(
        (bar.left() + 3.0)..=(bar.right() - 3.0),
        y,
        Stroke::new(1.0, color.linear_multiply(0.85)),
    );
}

fn show_more_popup(
    ui: &mut egui::Ui,
    cal: &mut CalendarUi,
    on: &[&MemoView],
    day: NaiveDate,
    anchor: egui::Rect,
    selected: &mut Option<String>,
    editing: &mut bool,
    picked: &mut Option<String>,
) {
    let popup_id = Id::new(("cal_more_popup", day));
    egui::Area::new(popup_id)
        .order(Order::Foreground)
        .fixed_pos(egui::pos2(anchor.left(), anchor.bottom() + 2.0))
        .constrain(true)
        .show(ui.ctx(), |ui| {
            Frame::none()
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::border()))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::same(8.0))
                .show(ui, |ui| {
                    ui.set_min_width(160.0);
                    ui.set_max_width(220.0);
                    ui.set_max_height(220.0);
                    egui::ScrollArea::vertical()
                        .id_source(("cal_more_scroll", day))
                        .show(ui, |ui| {
                            for m in on {
                                let dot = if memo_overdue(m) {
                                    theme::warn_overdue_fg()
                                } else if memo_due_today(m) {
                                    theme::warn_due_today_fg()
                                } else {
                                    theme::category_icon_color(m.category.canonical())
                                };
                                let title = if m.title.is_empty() {
                                    "(无标题)"
                                } else {
                                    m.title.as_str()
                                };
                                let mut title_rt = RichText::new(format!("    {title}"))
                                    .size(12.5)
                                    .color(if m.done {
                                        theme::text_muted()
                                    } else {
                                        theme::text()
                                    });
                                if m.done {
                                    title_rt = title_rt.strikethrough();
                                }
                                let r = ui.add(
                                    egui::Button::new(title_rt)
                                        .fill(Color32::TRANSPARENT)
                                        .frame(false),
                                );
                                if r.clicked() {
                                    *selected = Some(m.id.clone());
                                    *picked = Some(m.id.clone());
                                    *editing = false;
                                    cal.selected = day;
                                    cal.more_popup_day = None;
                                }
                                // 左侧色点（几何绘制，不用 ● 字符）
                                let rect = r.rect;
                                ui.painter().circle_filled(
                                    egui::pos2(rect.left() + 8.0, rect.center().y),
                                    3.5,
                                    dot,
                                );
                            }
                        });
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("关闭")
                                    .size(11.5)
                                    .color(theme::text_muted()),
                            )
                            .fill(theme::panel()),
                        )
                        .clicked()
                    {
                        cal.more_popup_day = None;
                    }
                });
        });
    // 点击外部关闭
    if ui.input(|i| i.pointer.any_click()) {
        let ptr = ui.input(|i| i.pointer.interact_pos());
        if let Some(pos) = ptr {
            if !anchor.contains(pos) {
                // Area 自己的 rect 难取；下一帧若再点空白格会清
                let _ = pos;
            }
        }
    }
}

pub(crate) fn show_day_side(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    cal: &mut CalendarUi,
    all: &[MemoView],
    q: &str,
    selected: &mut Option<String>,
    editing: &mut bool,
    open_new: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    pick_delete: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    let overdue = collect_overdue(all, q);
    let due_today = collect_due_today(all, q);
    let day_events = events_on(all, cal.selected, q);
    let day_list: Vec<_> = day_events
        .iter()
        .copied()
        .filter(|m| match cal.filter {
            DayFilter::Open => !m.done,
            DayFilter::All => true,
            DayFilter::Done => m.done,
        })
        .collect();

    // 当天：待处理（未过期）/ 已完成；过期统一进底部全局过期区
    let open_pending: Vec<_> = day_list
        .iter()
        .copied()
        .filter(|m| !m.done && !memo_overdue(m))
        .collect();
    let day_done: Vec<_> = day_list.iter().copied().filter(|m| m.done).collect();
    let day_empty = open_pending.is_empty() && day_done.is_empty();

    if !overdue.is_empty() {
        if overdue_banner(ui, overdue.len()) {
            cal.scroll_overdue = true;
        }
        ui.add_space(6.0);
    }
    if !due_today.is_empty() {
        if summary_card(
            ui,
            theme::warn_due_today_bg(),
            theme::warn_due_today_fg(),
            theme::warn_due_today_border(),
            "⏰",
            &format!("{} 条任务今天到期", due_today.len()),
            "记得在今日内完成",
            "查看当天",
        ) {
            jump_cal_to(cal, today());
        }
        ui.add_space(6.0);
    }

    let want_scroll_od = cal.scroll_overdue;
    let list_h = ui.available_height().max(40.0);
    egui::ScrollArea::vertical()
        .id_source("cal_day_list")
        .auto_shrink([false, false])
        .max_height(list_h)
        .show(ui, |ui| {
            if day_empty {
                // 当天无任务：左栏中部居中新建入口
                let empty_h = if overdue.is_empty() {
                    ui.available_height().max(160.0)
                } else {
                    160.0
                };
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), empty_h),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.add_space((empty_h * 0.28).max(24.0));
                        ui.label(
                            RichText::new("这天还没有任务")
                                .size(13.0)
                                .color(theme::text_muted()),
                        );
                        ui.add_space(14.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("+ 新建")
                                        .size(15.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(theme::shell_accent())
                                .rounding(Rounding::same(10.0))
                                .min_size(Vec2::new(128.0, 40.0)),
                            )
                            .clicked()
                        {
                            *open_new = true;
                        }
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("为这一天安排一件事")
                                .size(12.0)
                                .color(theme::text_muted()),
                        );
                    },
                );
            } else {
                section_header(
                    ui,
                    "进行中 / 待处理",
                    open_pending.len(),
                    theme::text_muted(),
                    theme::panel(),
                );
                if open_pending.is_empty() {
                    ui.label(
                        RichText::new("暂无待处理")
                            .size(12.5)
                            .color(theme::text_muted()),
                    );
                } else {
                    show_task_cards(
                        ui,
                        svc,
                        &open_pending,
                        cal.list_layout,
                        selected,
                        editing,
                        picked,
                        pick_edit,
                        pick_delete,
                        open_sticky,
                        tx,
                    );
                }

                if !day_done.is_empty() {
                    ui.add_space(10.0);
                    section_header(
                        ui,
                        "已完成",
                        day_done.len(),
                        theme::text_muted(),
                        theme::panel(),
                    );
                    show_task_cards(
                        ui,
                        svc,
                        &day_done,
                        cal.list_layout,
                        selected,
                        editing,
                        picked,
                        pick_edit,
                        pick_delete,
                        open_sticky,
                        tx,
                    );
                }
            }

            if !overdue.is_empty() {
                ui.add_space(10.0);
                let od_resp = ui.push_id("cal_overdue_section", |ui| {
                    section_header(
                        ui,
                        "已过期",
                        overdue.len(),
                        theme::warn_overdue_fg(),
                        theme::warn_overdue_bg(),
                    );
                });
                if want_scroll_od {
                    od_resp.response.scroll_to_me(Some(egui::Align::TOP));
                }
                show_task_cards(
                    ui,
                    svc,
                    &overdue,
                    cal.list_layout,
                    selected,
                    editing,
                    picked,
                    pick_edit,
                    pick_delete,
                    open_sticky,
                    tx,
                );
            }

            show_unscheduled_block(
                ui,
                svc,
                cal,
                all,
                q,
                selected,
                editing,
                picked,
                pick_edit,
                pick_delete,
                tx,
            );
        });
    if want_scroll_od {
        cal.scroll_overdue = false;
    }
}

/// 过期提醒条：黄历/统计下方；按钮滚到列表「已过期」区。
fn overdue_banner(ui: &mut egui::Ui, n: usize) -> bool {
    let mut jump = false;
    Frame::none()
        .fill(theme::warn_overdue_bg())
        .stroke(Stroke::new(1.0, theme::warn_overdue_border()))
        .rounding(Rounding::same(9.0))
        .inner_margin(Margin::symmetric(12.0, 9.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("⚠")
                        .size(14.0)
                        .color(theme::warn_overdue_fg()),
                );
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!("{n} 条任务已过期"))
                            .size(13.0)
                            .strong()
                            .color(theme::warn_overdue_fg()),
                    );
                    ui.label(
                        RichText::new("请尽快处理，或改期")
                            .size(11.5)
                            .color(theme::text_muted()),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("查看过期任务")
                                    .size(12.0)
                                    .strong()
                                    .color(theme::warn_overdue_fg()),
                            )
                            .fill(Color32::WHITE)
                            .stroke(Stroke::new(1.0, theme::warn_overdue_fg()))
                            .rounding(Rounding::same(7.0))
                            .min_size(Vec2::new(96.0, 28.0)),
                        )
                        .clicked()
                    {
                        jump = true;
                    }
                });
            });
        });
    jump
}

fn show_task_cards(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    list: &[&MemoView],
    layout: CalListLayout,
    selected: &mut Option<String>,
    editing: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    pick_delete: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    match layout {
        CalListLayout::List => {
            for m in list {
                let accent = task_accent(m);
                warn_task_card(
                    ui,
                    svc,
                    m,
                    accent,
                    selected,
                    editing,
                    picked,
                    pick_edit,
                    pick_delete,
                    open_sticky,
                    tx,
                );
            }
        }
        CalListLayout::Grid => {
            let gap = 8.0_f32;
            let col_w = ((ui.available_width() - gap) * 0.5).max(140.0);
            egui::Grid::new("cal_task_grid")
                .num_columns(2)
                .spacing([gap, gap])
                .min_col_width(col_w)
                .show(ui, |ui| {
                    for (i, m) in list.iter().enumerate() {
                        let accent = task_accent(m);
                        ui.allocate_ui_with_layout(
                            Vec2::new(col_w, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_max_width(col_w);
                                warn_task_card(
                                    ui,
                                    svc,
                                    m,
                                    accent,
                                    selected,
                                    editing,
                                    picked,
                                    pick_edit,
                                    pick_delete,
                                    open_sticky,
                                    tx,
                                );
                            },
                        );
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                    if list.len() % 2 == 1 {
                        ui.end_row();
                    }
                });
        }
    }
}

fn task_accent(m: &MemoView) -> TaskAccent {
    if memo_overdue(m) {
        TaskAccent::Overdue
    } else if memo_due_today(m) {
        TaskAccent::DueToday
    } else {
        TaskAccent::Normal
    }
}

/// 返回是否点击了右侧跳转。
fn summary_card(
    ui: &mut egui::Ui,
    bg: Color32,
    fg: Color32,
    border: Color32,
    icon: &str,
    title: &str,
    sub: &str,
    link: &str,
) -> bool {
    let mut jumped = false;
    Frame::none()
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let accent_h = 36.0;
                let (bar, _) = ui.allocate_exact_size(Vec2::new(4.0, accent_h), Sense::hover());
                ui.painter().rect_filled(bar, Rounding::same(2.0), fg);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!("{icon}  {title}"))
                            .size(13.0)
                            .strong()
                            .color(fg),
                    );
                    ui.label(RichText::new(sub).size(11.0).color(theme::text_muted()));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(link).size(12.0).color(fg),
                            )
                            .fill(Color32::WHITE)
                            .stroke(Stroke::new(1.0, fg))
                            .rounding(Rounding::same(6.0)),
                        )
                        .clicked()
                    {
                        jumped = true;
                    }
                });
            });
        });
    jumped
}

fn section_header(ui: &mut egui::Ui, title: &str, n: usize, fg: Color32, badge_bg: Color32) {
    ui.horizontal(|ui| {
        let g = theme::layout_galley(ui, title, egui::FontId::proportional(13.0), fg);
        let (rect, _) = ui.allocate_exact_size(g.size(), Sense::hover());
        ui.painter()
            .galley(theme::galley_pos_left_center(rect.left_center(), &g), g, fg);
        Frame::none()
            .fill(badge_bg)
            .rounding(Rounding::same(99.0))
            .inner_margin(Margin::symmetric(7.0, 1.0))
            .show(ui, |ui| {
                ui.label(RichText::new(format!("{n}")).size(11.0).color(fg));
            });
    });
    ui.add_space(4.0);
}

/// 选中日黄历：宜忌后台算完再刷新（农历已并入日期标题）。
fn show_huangli_row(ui: &mut egui::Ui, day: NaiveDate) {
    let al = huangli::cached_full(day);
    if al.is_none() {
        huangli::ensure_full_async(day, ui.ctx().clone());
    }
    let lunar = al
        .as_ref()
        .map(|a| a.lunar_line())
        .unwrap_or_else(|| huangli::lunar_summary_line(day));
    let (yi, ji) = match &al {
        Some(a) => a.display_yi_ji_text(36),
        None => ("…".into(), "…".into()),
    };
    ui.horizontal(|ui| {
        ui.set_min_width(ui.available_width());
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() * 0.42);
            ui.label(
                RichText::new(lunar)
                    .size(12.0)
                    .color(theme::text_muted()),
            );
        });
        let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, 28.0), Sense::hover());
        ui.painter()
            .rect_filled(sep, Rounding::ZERO, theme::border());
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("宜")
                        .size(11.0)
                        .strong()
                        .color(theme::success()),
                );
                ui.label(RichText::new(yi).size(11.5).color(theme::text_muted()));
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("忌")
                        .size(11.0)
                        .strong()
                        .color(theme::warn_overdue_fg()),
                );
                ui.label(RichText::new(ji).size(11.5).color(theme::text_muted()));
            });
        });
    });
}

#[derive(Clone, Copy)]
enum TaskAccent {
    Overdue,
    DueToday,
    Normal,
}

fn warn_task_card(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    m: &MemoView,
    accent: TaskAccent,
    selected: &mut Option<String>,
    editing: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    pick_delete: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    let bar_c = if m.done {
        theme::border()
    } else {
        match accent {
            TaskAccent::Overdue => theme::warn_overdue_fg(),
            TaskAccent::DueToday => theme::warn_due_today_fg(),
            TaskAccent::Normal => theme::shell_accent(),
        }
    };
    let ring_c = if m.done {
        theme::success()
    } else {
        match accent {
            TaskAccent::Overdue => theme::warn_overdue_fg(),
            TaskAccent::DueToday => theme::warn_due_today_fg(),
            TaskAccent::Normal => theme::shell_accent(),
        }
    };
    let title_fg = if m.done {
        theme::text_muted()
    } else if matches!(accent, TaskAccent::Overdue) {
        theme::warn_overdue_fg()
    } else {
        theme::text()
    };
    let excerpt = content_excerpt(m, 36);
    let deadline = deadline_date(&m.due_date, &m.end_date);
    let fill = if matches!(accent, TaskAccent::Overdue) && !m.done {
        theme::warn_overdue_bg()
    } else {
        theme::card()
    };
    let stroke_c = if matches!(accent, TaskAccent::Overdue) && !m.done {
        theme::warn_overdue_border()
    } else {
        theme::border()
    };

    let mut consumed = false;
    let outer = Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke_c))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                if task_done_checkbox(ui, m.done, ring_c).clicked() {
                    toggle_done(svc, m, tx, selected, editing);
                    consumed = true;
                }
                ui.add_space(8.0);

                let right_w = 128.0_f32;
                let mid_w = (ui.available_width() - right_w - 6.0).max(80.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(mid_w, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_max_width(mid_w);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let title = if m.title.is_empty() {
                                "(无标题)"
                            } else {
                                m.title.as_str()
                            };
                            let mut title_rt = RichText::new(title)
                                .size(15.0)
                                .strong()
                                .color(title_fg);
                            if m.done {
                                title_rt = title_rt.strikethrough();
                            }
                            ui.label(title_rt);
                            for (i, tag) in m.tags.iter().take(3).enumerate() {
                                weak_tag_pill(ui, tag, i as u64);
                            }
                        });
                        if let Some(ex) = &excerpt {
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(ex)
                                    .size(12.0)
                                    .color(theme::text_muted()),
                            );
                        }
                    },
                );

                ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                    ui.set_min_width(right_w);
                    task_priority_pill(ui, m);
                    ui.add_space(3.0);
                    let due_line = format_task_due_line(m, deadline, accent);
                    if !due_line.is_empty() {
                        let due_fg = if matches!(accent, TaskAccent::Overdue) && !m.done {
                            theme::warn_overdue_fg()
                        } else {
                            theme::text_muted()
                        };
                        ui.label(RichText::new(due_line).size(11.5).color(due_fg));
                    }
                });
            });
        });

    let r = outer.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 4.0, r.bottom())),
        Rounding {
            nw: 10.0,
            ne: 0.0,
            sw: 10.0,
            se: 0.0,
        },
        bar_c,
    );
    let resp = outer.response.interact(Sense::click());
    if resp.clicked() && !consumed {
        pick_memo(m, selected, editing, picked, pick_edit, false);
    }
    resp.context_menu(|ui| {
        item_menu(
            ui,
            svc,
            m,
            selected,
            editing,
            picked,
            pick_edit,
            pick_delete,
            open_sticky,
            tx,
        );
    });
    warn_task_hover_preview(ui, &resp, &m.id, &m.title, &m.content);
    ui.add_space(6.0);
}

#[derive(Clone)]
struct HoverPreviewState {
    memo_id: String,
    since: f64,
}

/// 滚动中不弹；同一条目静止悬停满约 0.35s 后才显示完整标题/正文。
fn warn_task_hover_preview(
    ui: &mut egui::Ui,
    resp: &egui::Response,
    memo_id: &str,
    title: &str,
    content: &str,
) {
    const DELAY: f64 = 0.35;
    let state_id = Id::new("warn_task_hover_preview");
    let scrolling = ui.input(|i| {
        i.raw_scroll_delta.y.abs() > 0.0 || i.smooth_scroll_delta.y.abs() > 0.0
    });
    if scrolling {
        ui.ctx().data_mut(|d| {
            d.remove::<HoverPreviewState>(state_id);
        });
        return;
    }

    let now = ui.input(|i| i.time);
    if resp.hovered() {
        let ready = ui.ctx().data_mut(|d| {
            let st = d.get_temp_mut_or_insert_with(state_id, || HoverPreviewState {
                memo_id: memo_id.to_string(),
                since: now,
            });
            if st.memo_id != memo_id {
                st.memo_id = memo_id.to_string();
                st.since = now;
            }
            now - st.since >= DELAY
        });
        if ready {
            let tip_title = if title.trim().is_empty() {
                "(无标题)"
            } else {
                title
            };
            let tip_body = content.trim();
            let tip_id = Id::new(("warn_task_hover_tip", memo_id));
            egui::show_tooltip_at_pointer(ui.ctx(), tip_id, |ui| {
                ui.set_max_width(320.0);
                ui.label(
                    RichText::new(tip_title)
                        .size(13.0)
                        .strong()
                        .color(theme::text()),
                );
                if !tip_body.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(tip_body)
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                }
            });
        }
    } else {
        ui.ctx().data_mut(|d| {
            if d.get_temp::<HoverPreviewState>(state_id)
                .map(|st| st.memo_id == memo_id)
                .unwrap_or(false)
            {
                d.remove::<HoverPreviewState>(state_id);
            }
        });
    }
}

/// 圆形完成勾选：未完成描边圆；已完成绿底白勾。
fn task_done_checkbox(ui: &mut egui::Ui, done: bool, ring: Color32) -> egui::Response {
    let size = Vec2::splat(22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let c = rect.center();
    let r = 9.0;
    let painter = ui.painter();
    if done {
        painter.circle_filled(c, r, theme::success());
        let stroke = Stroke::new(2.0, Color32::WHITE);
        painter.line_segment(
            [
                egui::pos2(c.x - 4.0, c.y + 0.5),
                egui::pos2(c.x - 1.0, c.y + 3.5),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(c.x - 1.0, c.y + 3.5),
                egui::pos2(c.x + 4.5, c.y - 3.0),
            ],
            stroke,
        );
    } else {
        painter.circle_stroke(c, r, Stroke::new(1.6, ring));
    }
    resp.on_hover_text(if done {
        "点击取消完成"
    } else {
        "点击标记完成"
    })
}

#[allow(dead_code)]
fn paint_note_glyph(ui: &mut egui::Ui, color: Color32) {
    let size = Vec2::splat(12.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    let r = egui::Rect::from_center_size(rect.center(), Vec2::new(9.0, 10.0));
    p.rect_stroke(r, Rounding::same(1.5), Stroke::new(1.2, color));
    p.line_segment(
        [
            egui::pos2(r.left() + 2.0, r.top() + 3.0),
            egui::pos2(r.right() - 2.0, r.top() + 3.0),
        ],
        Stroke::new(1.0, color),
    );
    p.line_segment(
        [
            egui::pos2(r.left() + 2.0, r.top() + 5.5),
            egui::pos2(r.right() - 2.5, r.top() + 5.5),
        ],
        Stroke::new(1.0, color),
    );
}

fn task_priority_pill(ui: &mut egui::Ui, m: &MemoView) {
    let pri = match m.priority {
        MemoPriority::High => "高",
        MemoPriority::Normal => "普通",
        MemoPriority::Low => "低",
    };
    let mut bg = theme::priority_pill_bg(m.priority);
    let mut fg = theme::priority_pill_fg(m.priority);
    if m.done {
        bg = theme::panel();
        fg = theme::text_muted();
    }
    status_badge(ui, pri, bg, fg, theme::border());
}

fn format_task_due_line(
    m: &MemoView,
    deadline: Option<NaiveDate>,
    accent: TaskAccent,
) -> String {
    let Some(d) = deadline else {
        return String::new();
    };
    if matches!(accent, TaskAccent::Overdue) && !m.done {
        let days = overdue_days(&m.due_date, &m.end_date, m.done)
            .unwrap_or(1)
            .max(1);
        return format!("{} 截止 · 逾期 {} 天", format_mmdd(d), days);
    }
    if is_span(m) {
        if let (Some(a), Some(b)) = (
            due_date_part(&m.due_date),
            event_end_date(&m.due_date, &m.end_date),
        ) {
            if b > a {
                return format!("{} ~ {}", format_mmdd(a), format_mmdd(b));
            }
        }
    }
    if let Some(hm) = due_time_hm(&m.due_date) {
        if memo_due_today(m) {
            return format!("今天 {hm}");
        }
        return format!("{} {hm}", format_mmdd(d));
    }
    format_mmdd(d)
}

fn weak_tag_pill(ui: &mut egui::Ui, tag: &str, seed: u64) {
    let (bg, _fg) = theme::tag_soft_pair(seed + tag.len() as u64);
    let bg = Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), 90);
    let fg = theme::text_muted();
    let label = if tag.starts_with('#') {
        tag.to_string()
    } else {
        format!("#{tag}")
    };
    Frame::none()
        .fill(bg)
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(3.0))
        .inner_margin(Margin::symmetric(5.0, 1.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(10.0).color(fg));
        });
}

fn status_badge(ui: &mut egui::Ui, text: &str, bg: Color32, fg: Color32, border: Color32) {
    Frame::none()
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .rounding(Rounding::same(99.0))
        .inner_margin(Margin::symmetric(8.0, 3.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).strong().color(fg));
        });
}

#[derive(Clone, Copy)]
enum DetailToolIcon {
    Edit,
    Copy,
    Trash,
}

/// 详情顶栏工具按钮：自绘效果图风格（黄铅笔 / 棕剪贴板 / 灰网垃圾桶）。
fn detail_tool_button(ui: &mut egui::Ui, kind: DetailToolIcon, tip: &str) -> egui::Response {
    let size = Vec2::new(32.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(rect, Rounding::same(6.0), theme::panel());
    }
    let icon_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(18.0));
    match kind {
        DetailToolIcon::Edit => paint_tool_edit(ui.painter(), icon_rect),
        DetailToolIcon::Copy => paint_tool_copy(ui.painter(), icon_rect),
        DetailToolIcon::Trash => paint_tool_trash(ui.painter(), icon_rect),
    }
    resp.on_hover_text(tip)
}

/// 斜向黄铅笔：粉橡皮 + 金属箍 + 黄杆 + 深色笔尖。
fn paint_tool_edit(painter: &egui::Painter, rect: egui::Rect) {
    let ink = theme::text();
    let yellow = Color32::from_rgb(0xF5, 0xC5, 0x42);
    let eraser = Color32::from_rgb(0xE8, 0x8A, 0x8A);
    let metal = Color32::from_rgb(0xA8, 0xA8, 0xB0);
    let tip = Color32::from_rgb(0x3A, 0x3A, 0x3E);
    let c = rect.center();
    // 沿右上→左下对角线摆放
    let dir = Vec2::new(-0.72, 0.72);
    let len = rect.width() * 0.92;
    let half = dir * (len * 0.5);
    let a = c + half; // 笔尖端
    let b = c - half; // 橡皮端
    let n = Vec2::new(-dir.y, dir.x);
    let w = rect.width() * 0.16;

    let body_a = a + dir * (len * 0.18);
    let body_b = b + dir * (len * 0.22);
    let quad = |p0: egui::Pos2, p1: egui::Pos2, hw: f32, fill: Color32| {
        let pts = [
            p0 + n * hw,
            p0 - n * hw,
            p1 - n * hw,
            p1 + n * hw,
        ];
        painter.add(egui::Shape::convex_polygon(pts.to_vec(), fill, Stroke::new(1.0, ink)));
    };
    // 笔杆
    quad(body_a, body_b, w, yellow);
    // 金属箍
    let m0 = b + dir * (len * 0.22);
    let m1 = b + dir * (len * 0.12);
    quad(m0, m1, w * 0.95, metal);
    // 橡皮
    let e0 = b + dir * (len * 0.12);
    quad(e0, b, w * 0.9, eraser);
    // 笔尖三角
    let tip_base = a + dir * (len * 0.18);
    painter.add(egui::Shape::convex_polygon(
        vec![
            a,
            tip_base + n * w,
            tip_base - n * w,
        ],
        tip,
        Stroke::new(1.0, ink),
    ));
}

/// 棕底剪贴板 + 白纸横线 + 灰夹。
fn paint_tool_copy(painter: &egui::Painter, rect: egui::Rect) {
    let ink = theme::text();
    let board = Color32::from_rgb(0xC4, 0xA2, 0x7A);
    let paper = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    let clip = Color32::from_rgb(0x9A, 0x9A, 0xA0);
    let r = rect.shrink2(Vec2::new(3.0, 1.5));
    painter.rect_filled(r, Rounding::same(2.5), board);
    painter.rect_stroke(r, Rounding::same(2.5), Stroke::new(1.1, ink));
    let paper_r = egui::Rect::from_min_max(
        egui::pos2(r.left() + 2.2, r.top() + 4.5),
        egui::pos2(r.right() - 2.2, r.bottom() - 2.0),
    );
    painter.rect_filled(paper_r, Rounding::same(1.5), paper);
    painter.rect_stroke(paper_r, Rounding::same(1.5), Stroke::new(0.9, ink));
    let lx0 = paper_r.left() + 2.0;
    let lx1 = paper_r.right() - 2.0;
    for i in 0..3 {
        let y = paper_r.top() + 3.2 + i as f32 * 3.2;
        painter.line_segment(
            [egui::pos2(lx0, y), egui::pos2(lx1, y)],
            Stroke::new(1.0, ink.linear_multiply(0.55)),
        );
    }
    // 顶部夹子
    let clip_r = egui::Rect::from_center_size(
        egui::pos2(r.center().x, r.top() + 2.2),
        Vec2::new(r.width() * 0.55, 4.2),
    );
    painter.rect_filled(clip_r, Rounding::same(1.2), clip);
    painter.rect_stroke(clip_r, Rounding::same(1.2), Stroke::new(1.0, ink));
}

/// 灰蓝网眼垃圾桶：外轮廓 + 点阵。
fn paint_tool_trash(painter: &egui::Painter, rect: egui::Rect) {
    let ink = theme::text();
    let bin = Color32::from_rgb(0x8E, 0x95, 0xA0);
    let r = rect.shrink2(Vec2::new(3.5, 1.5));
    let top_w = r.width();
    let bot_w = r.width() * 0.72;
    let top_y = r.top() + 3.0;
    let bot_y = r.bottom();
    let cx = r.center().x;
    let pts = [
        egui::pos2(cx - top_w * 0.5, top_y),
        egui::pos2(cx + top_w * 0.5, top_y),
        egui::pos2(cx + bot_w * 0.5, bot_y),
        egui::pos2(cx - bot_w * 0.5, bot_y),
    ];
    painter.add(egui::Shape::convex_polygon(
        pts.to_vec(),
        bin,
        Stroke::new(1.15, ink),
    ));
    // 沿口
    let rim = egui::Rect::from_center_size(
        egui::pos2(cx, r.top() + 1.6),
        Vec2::new(top_w + 1.5, 3.0),
    );
    painter.rect_filled(rim, Rounding::same(1.0), bin);
    painter.rect_stroke(rim, Rounding::same(1.0), Stroke::new(1.1, ink));
    // 网眼点
    let rows = 3;
    let cols = 3;
    for row in 0..rows {
        for col in 0..cols {
            let t = (row as f32 + 1.0) / (rows as f32 + 1.0);
            let y = top_y + (bot_y - top_y) * t;
            let half = top_w * 0.5 * (1.0 - t) + bot_w * 0.5 * t;
            let u = (col as f32 + 1.0) / (cols as f32 + 1.0);
            let x = cx - half + 2.0 * half * u;
            painter.circle_filled(egui::pos2(x, y), 0.85, ink.linear_multiply(0.65));
        }
    }
}

/// 侧栏任务卡左侧分类图标（可点击切换完成）；用 layout_galley 避免 ☐ 等符号在 Windows 上落成空方框。
/// 任务列表卡已改用圆形勾选，本函数暂留作他用。
#[allow(dead_code)]
fn warn_card_category_icon(ui: &mut egui::Ui, cat: MemoCategory, done: bool) -> egui::Response {
    let size = Vec2::splat(20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cat = cat.canonical();
    let color = if done {
        let c = theme::category_icon_color(cat);
        let m = theme::text_muted();
        Color32::from_rgb(
            ((c.r() as u16 + m.r() as u16 * 2) / 3) as u8,
            ((c.g() as u16 + m.g() as u16 * 2) / 3) as u8,
            ((c.b() as u16 + m.b() as u16 * 2) / 3) as u8,
        )
    } else {
        theme::category_icon_color(cat)
    };
    let g = theme::layout_galley(
        ui,
        memo_form::category_icon(cat),
        egui::FontId::proportional(16.0),
        color,
    );
    ui.painter()
        .galley(theme::galley_pos_center(rect.center(), &g), g, color);
    resp.on_hover_text(if done {
        format!("{} · 点击取消完成", cat.label())
    } else {
        format!("{} · 点击标记完成", cat.label())
    })
}

fn show_week_side(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    cal: &mut CalendarUi,
    all: &[MemoView],
    q: &str,
    selected: &mut Option<String>,
    editing: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    pick_delete: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    let list_h = ui.available_height().max(40.0);
    egui::ScrollArea::vertical()
        .id_source("cal_week_list")
        .auto_shrink([false, false])
        .max_height(list_h)
        .show(ui, |ui| {
            for i in 0..7i64 {
                let d = add_days(cal.week_monday, i);
                let day_list = events_on(all, d, q);
                let is_today = d == today();
                Frame::none()
                    .fill(if is_today {
                        theme::accent_soft()
                    } else {
                        theme::card()
                    })
                    .stroke(Stroke::new(1.0, theme::border()))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(10.0, 8.0))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}",
                                    weekday_zh(d),
                                    format_md(d)
                                ))
                                .size(13.0)
                                .strong()
                                .color(theme::text()),
                            );
                            if is_today {
                                ui.label(
                                    RichText::new("今天")
                                        .size(11.0)
                                        .color(theme::shell_accent()),
                                );
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{} 条", day_list.len()))
                                        .size(11.0)
                                        .color(theme::text_muted()),
                                );
                            });
                        });
                        ui.add_space(6.0);
                        if day_list.is_empty() {
                            ui.label(
                                RichText::new("这天没有安排")
                                    .size(12.0)
                                    .color(theme::text_muted()),
                            );
                        } else {
                            for m in &day_list {
                                warn_task_card(
                                    ui,
                                    svc,
                                    m,
                                    task_accent(m),
                                    selected,
                                    editing,
                                    picked,
                                    pick_edit,
                                    pick_delete,
                                    open_sticky,
                                    tx,
                                );
                            }
                        }
                    });
                ui.add_space(8.0);
            }
        });
}

/// 右侧详情抽屉：遮罩 + 面板（滑入/滑出）。返回是否已关闭完成。
pub fn show_cal_drawer(
    ctx: &egui::Context,
    host: egui::Rect,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    let anim = page_shell::drawer_anim_begin(ctx, "cal_detail");
    let mut request_close =
        !anim.closing && ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let drawer_w = host.width().min(page_shell::PAGE_RAIL_TARGET);
    let (panel, dim, alpha) =
        page_shell::drawer_layout(host, drawer_w, anim.t.max(0.001));

    if dim.width() > 1.0 && dim.height() > 1.0 {
        egui::Area::new(Id::new("cal_drawer_backdrop"))
            .order(Order::Middle)
            .fixed_pos(dim.min)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(dim.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, Rounding::ZERO, Color32::from_black_alpha(alpha));
                if resp.clicked() && !anim.closing {
                    request_close = true;
                }
            });
    }

    // 内高须扣 Frame 上下 margin + 描边，否则外高撑破 host 盖住底栏
    const PAD_X: f32 = 12.0;
    const PAD_Y: f32 = 10.0;
    let inner_h = (panel.height() - PAD_Y * 2.0 - 2.0).max(80.0);

    egui::Area::new(Id::new("cal_drawer_panel"))
        .order(Order::Foreground)
        .fixed_pos(panel.min)
        .interactable(true)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_max_size(panel.size());
            ui.set_clip_rect(panel);
            Frame::none()
                .fill(theme::panel())
                .stroke(Stroke::new(1.0, theme::border()))
                .inner_margin(Margin::symmetric(PAD_X, PAD_Y))
                .show(ui, |ui| {
                    ui.set_min_size(Vec2::new(drawer_w, inner_h));
                    ui.set_max_height(inner_h);
                    ui.set_max_width(drawer_w);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("关闭")
                                        .size(13.0)
                                        .strong()
                                        .color(theme::text_muted()),
                                )
                                .fill(Color32::TRANSPARENT),
                            )
                            .clicked()
                            && !anim.closing
                        {
                            request_close = true;
                        }
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_source("cal_drawer_scroll")
                        .auto_shrink([false, false])
                        .max_height((inner_h - 48.0).max(80.0))
                        .show(ui, |ui| {
                            ui.set_min_width((drawer_w - PAD_X * 2.0).max(200.0));
                            add_contents(ui);
                        });
                });
        });

    page_shell::drawer_anim_end(ctx, &anim, request_close)
}

fn show_unscheduled_block(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    cal: &mut CalendarUi,
    all: &[MemoView],
    q: &str,
    selected: &mut Option<String>,
    editing: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    _pick_delete: &mut bool,
    tx: &Sender<BgMsg>,
) {
    let uns = unscheduled(all, q);
    // 无随手记时整块不显示
    if uns.is_empty() {
        return;
    }
    ui.add_space(12.0);
    Frame::none()
        .fill(theme::success_soft())
        .stroke(Stroke::new(1.0, Color32::from_rgb(0xB7, 0xE4, 0xC7)))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("未安排 · 随手记")
                    .size(12.5)
                    .strong()
                    .color(theme::success()),
            );
            ui.add_space(6.0);
            for m in uns {
                ui.horizontal(|ui| {
                    let title = if m.title.is_empty() {
                        "(无标题)"
                    } else {
                        m.title.as_str()
                    };
                    if ui
                        .add(
                            egui::Label::new(
                                RichText::new(title).size(13.0).color(theme::text()),
                            )
                            .sense(Sense::click()),
                        )
                        .clicked()
                    {
                        pick_memo(m, selected, editing, picked, pick_edit, false);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("放到今天")
                                        .size(11.0)
                                        .color(theme::success()),
                                )
                                .fill(Color32::from_white_alpha(200)),
                            )
                            .clicked()
                        {
                            let t = today();
                            cal.year = t.year();
                            cal.month = t.month();
                            cal.selected = t;
                            place_today(svc, m, tx);
                        }
                    });
                });
                ui.add_space(4.0);
            }
        });
}

/// 日历选中备忘后的只读详情卡。
/// 返回 `true` 表示用户点了「编辑」（由壳层打开表单抽屉，详情保持只读）。
pub fn show_event_detail(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    m: &MemoView,
    selected: &mut Option<String>,
    editing: &mut bool,
    show_delete: &mut bool,
    status_line: &mut String,
    tx: &Sender<BgMsg>,
) -> bool {
    let mut want_edit = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // 效果图顺序：编辑 · 复制 · 删除（右对齐时反向添加）
            if detail_tool_button(ui, DetailToolIcon::Trash, "删除").clicked() {
                *show_delete = true;
            }
            if detail_tool_button(ui, DetailToolIcon::Copy, "复制全文").clicked() {
                ui.output_mut(|o| {
                    o.copied_text = format!("{}\n\n{}", m.title, m.content);
                });
                *status_line = "已复制到剪贴板".into();
            }
            if detail_tool_button(ui, DetailToolIcon::Edit, "编辑").clicked() {
                want_edit = true;
            }
        });
    });
    ui.add_space(8.0);

    let scroll_h = ui.available_height().max(80.0);
    egui::ScrollArea::vertical()
        .id_source("cal_event_detail_scroll")
        .auto_shrink([false, false])
        .max_height(scroll_h)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            ui.horizontal(|ui| {
                let dot = if memo_overdue(m) {
                    theme::warn_overdue_fg()
                } else if m.priority == MemoPriority::High {
                    theme::warn_overdue_fg()
                } else {
                    theme::category_icon_color(m.category.canonical())
                };
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 5.0, dot);
                let mut heading = RichText::new(if m.title.is_empty() {
                    "(无标题)"
                } else {
                    m.title.as_str()
                })
                .size(20.0)
                .strong()
                .color(if m.done {
                    theme::text_muted()
                } else {
                    theme::text()
                });
                if m.done {
                    heading = heading.strikethrough();
                }
                ui.label(heading);
            });
            ui.add_space(14.0);

            // —— 时间（效果图：红顶日历 + 红针时钟，日期深色 / 跨度蓝字）——
            detail_section_label(ui, "时间");
            if let Some(start) = due_date_part(&m.due_date) {
                detail_time_row(
                    ui,
                    DetailTimeIcon::Calendar,
                    &format!(
                        "{}年{}月{}日 ({})",
                        start.year(),
                        start.month(),
                        start.day(),
                        weekday_zh(start)
                    ),
                    theme::text(),
                );
                ui.add_space(6.0);
                let time_line = if is_span(m) {
                    if let Some(end) = event_end_date(&m.due_date, &m.end_date) {
                        format!("{} ~ {}", format_md(start), format_md(end))
                    } else {
                        format_md(start)
                    }
                } else if let Some(hm) = due_time_hm(&m.due_date) {
                    format!("{hm} 开始")
                } else {
                    "全天".into()
                };
                detail_time_row(ui, DetailTimeIcon::Clock, &time_line, theme::accent());
            } else {
                detail_time_row(ui, DetailTimeIcon::Calendar, "未设置日期", theme::text_muted());
            }
            ui.add_space(16.0);

            // —— 属性（横排 soft pill：键：值）——
            detail_section_label(ui, "属性");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.spacing_mut().item_spacing.y = 6.0;
                let pri = match m.priority {
                    MemoPriority::High => "高",
                    MemoPriority::Normal => "普通",
                    MemoPriority::Low => "低",
                };
                detail_attr_pill(
                    ui,
                    &format!("优先级：{pri}"),
                    theme::priority_pill_bg(m.priority),
                    theme::priority_pill_fg(m.priority),
                );
                detail_attr_pill(
                    ui,
                    &format!("分类：{}", m.category.label()),
                    theme::chip_soft_bg(m.category),
                    theme::chip_soft_fg(m.category),
                );
                for (i, tag) in m.tags.iter().enumerate() {
                    let (bg, fg) = theme::tag_soft_pair(i as u64 + tag.len() as u64);
                    let label = if tag.starts_with('#') {
                        tag.clone()
                    } else {
                        format!("#{tag}")
                    };
                    detail_attr_pill(ui, &label, bg, fg);
                }
            });
            ui.add_space(16.0);

            // —— 描述 ——
            detail_section_label(ui, "描述");
            Frame::none()
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::border()))
                .rounding(Rounding::same(10.0))
                .inner_margin(Margin::same(12.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.set_min_height(72.0);
                    if m.content.trim().is_empty() {
                        ui.label(
                            RichText::new("情况说明，自由书写即可...")
                                .size(13.0)
                                .italics()
                                .color(theme::text_muted()),
                        );
                    } else {
                        doc_editor::show_viewer_body(ui, &m.content);
                    }
                });

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                show_reschedule_button(ui, svc, m, status_line, tx);
                if m.done {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("撤销完成").size(13.0))
                                .min_size(Vec2::new(100.0, 32.0)),
                        )
                        .clicked()
                    {
                        toggle_done(svc, m, tx, selected, editing);
                    }
                } else if ui
                    .add(
                        egui::Button::new(
                            RichText::new("标记完成")
                                .size(13.5)
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(theme::shell_accent())
                        .rounding(Rounding::same(8.0))
                        .min_size(Vec2::new(120.0, 32.0)),
                    )
                    .clicked()
                {
                    toggle_done(svc, m, tx, selected, editing);
                }
            });
            ui.add_space(16.0);
        });
    want_edit
}

fn detail_section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(12.0)
            .strong()
            .color(theme::text_muted()),
    );
    ui.add_space(8.0);
}

#[derive(Clone, Copy)]
enum DetailTimeIcon {
    Calendar,
    Clock,
}

fn detail_time_row(ui: &mut egui::Ui, kind: DetailTimeIcon, text: &str, text_color: Color32) {
    ui.horizontal(|ui| {
        let (irect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
        match kind {
            DetailTimeIcon::Calendar => paint_detail_calendar_icon(ui.painter(), irect),
            DetailTimeIcon::Clock => paint_detail_clock_icon(ui.painter(), irect),
        }
        ui.add_space(6.0);
        ui.label(RichText::new(text).size(13.5).color(text_color));
    });
}

/// 效果图日历：红顶条 + 浅底 + 深描边。
fn paint_detail_calendar_icon(painter: &egui::Painter, rect: egui::Rect) {
    let r = rect.shrink(1.0);
    let red = theme::mac_red();
    let ink = theme::text();
    let body = Color32::from_rgb(0xF7, 0xF7, 0xF8);
    painter.rect_filled(r, Rounding::same(3.0), body);
    let header_h = r.height() * 0.32;
    let header = egui::Rect::from_min_max(r.left_top(), egui::pos2(r.right(), r.top() + header_h));
    painter.rect_filled(
        header,
        Rounding {
            nw: 3.0,
            ne: 3.0,
            sw: 0.0,
            se: 0.0,
        },
        red,
    );
    painter.rect_stroke(r, Rounding::same(3.0), Stroke::new(1.15, ink));
    // 装订小点
    let cy = r.top() + header_h * 0.55;
    for x in [r.left() + r.width() * 0.32, r.left() + r.width() * 0.68] {
        painter.circle_filled(egui::pos2(x, cy), 1.1, Color32::WHITE);
    }
    // 底格两点示意日期
    let gy = r.top() + header_h + (r.height() - header_h) * 0.55;
    painter.circle_filled(egui::pos2(r.left() + r.width() * 0.35, gy), 1.0, ink.linear_multiply(0.45));
    painter.circle_filled(egui::pos2(r.left() + r.width() * 0.62, gy), 1.0, ink.linear_multiply(0.45));
}

/// 效果图时钟：深色圆框 + 红色指针。
fn paint_detail_clock_icon(painter: &egui::Painter, rect: egui::Rect) {
    let c = rect.center();
    let rad = rect.width().min(rect.height()) * 0.42;
    let ink = theme::text();
    let red = theme::mac_red();
    painter.circle_stroke(c, rad, Stroke::new(1.35, ink));
    // 时针（较短）
    painter.line_segment(
        [c, egui::pos2(c.x + rad * 0.15, c.y - rad * 0.45)],
        Stroke::new(1.6, red),
    );
    // 分针（较长）
    painter.line_segment(
        [c, egui::pos2(c.x + rad * 0.55, c.y + rad * 0.12)],
        Stroke::new(1.45, red),
    );
    painter.circle_filled(c, 1.35, red);
}

fn detail_attr_pill(ui: &mut egui::Ui, text: &str, bg: Color32, fg: Color32) {
    Frame::none()
        .fill(bg)
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(10.0, 5.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.5).color(fg));
        });
}

fn toggle_done(
    svc: &Arc<MemoService>,
    m: &MemoView,
    tx: &Sender<BgMsg>,
    selected: &mut Option<String>,
    editing: &mut bool,
) {
    let next = !m.done;
    // 标记完成：退出详情回到列表（撤销完成仍留在详情）
    if next {
        if selected.as_deref() == Some(m.id.as_str()) {
            *selected = None;
        }
        *editing = false;
    }
    let svc = svc.clone();
    let id = m.id.clone();
    let tx = tx.clone();
    std::thread::spawn(move || match svc.set_done(&id, next) {
        Ok(()) => {
            let _ = tx.send(BgMsg::Refresh);
        }
        Err(e) => {
            let _ = tx.send(BgMsg::Error(e.to_string()));
        }
    });
}

fn pick_memo(
    m: &MemoView,
    selected: &mut Option<String>,
    editing: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    edit: bool,
) {
    *selected = Some(m.id.clone());
    *editing = edit;
    *picked = Some(m.id.clone());
    *pick_edit = edit;
}

fn item_menu(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    m: &MemoView,
    selected: &mut Option<String>,
    editing: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    pick_delete: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    theme::style_context_menu(ui);
    if theme::menu_item_mark(ui, "开", "打开").clicked() {
        pick_memo(m, selected, editing, picked, pick_edit, false);
        ui.close_menu();
    }
    if theme::menu_item_mark(ui, "编", "编辑").clicked() {
        pick_memo(m, selected, editing, picked, pick_edit, true);
        ui.close_menu();
    }
    if theme::menu_item_mark(ui, "签", "生成桌面便签").clicked() {
        *open_sticky = Some(m.id.clone());
        ui.close_menu();
    }
    theme::menu_sep(ui);
    let done_l = if m.done { "取消完成" } else { "标记完成" };
    let done_mark = if m.done { "↩" } else { "✓" };
    if theme::menu_item_mark(ui, done_mark, done_l).clicked() {
        toggle_done(svc, m, tx, selected, editing);
        ui.close_menu();
    }
    theme::menu_sep(ui);
    if theme::menu_item_danger(ui, "移入回收站…").clicked() {
        *selected = Some(m.id.clone());
        *picked = Some(m.id.clone());
        *pick_delete = true;
        ui.close_menu();
    }
}

/// 「改期」：自管 Area 面板选日期/时间；仅点「确定改期」或「取消」才关闭（选日不关）。
fn show_reschedule_button(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    m: &MemoView,
    status_line: &mut String,
    tx: &Sender<BgMsg>,
) {
    let open_id = ui.make_persistent_id(Id::new(("cal_reschedule_open", m.id.as_str())));
    let draft_id = ui.make_persistent_id(Id::new(("cal_reschedule_draft", m.id.as_str())));
    let span = is_span(m);
    let with_time = !span;

    let btn = ui.add(
        egui::Button::new(RichText::new("改期").size(13.0)).min_size(Vec2::new(88.0, 32.0)),
    );
    if btn.clicked() {
        let initial = if m.due_date.trim().is_empty() {
            if with_time {
                format!("{} 09:00", ymd(today()))
            } else {
                ymd(today())
            }
        } else if with_time {
            m.due_date.clone()
        } else {
            date_field::format_ymd(due_date_part(&m.due_date).unwrap_or_else(today))
        };
        ui.ctx().data_mut(|d| {
            d.insert_temp(draft_id, initial);
            d.insert_temp(open_id, true);
        });
    }

    let open = ui.ctx().data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
    if !open {
        return;
    }

    let anchor = btn.rect.left_bottom() + Vec2::new(0.0, 4.0);
    let mut close = false;
    let mut confirmed: Option<String> = None;

    egui::Area::new(Id::new(("cal_reschedule_area", m.id.as_str())))
        .order(Order::Foreground)
        .fixed_pos(anchor)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            Frame::none()
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::border()))
                .rounding(Rounding::same(10.0))
                .inner_margin(Margin::same(12.0))
                .show(ui, |ui| {
                    ui.set_min_width(if with_time { 288.0 } else { 248.0 });
                    ui.label(
                        RichText::new(if with_time {
                            "选择新的到期时间"
                        } else {
                            "选择新的开始日期"
                        })
                        .size(12.5)
                        .strong()
                        .color(theme::text()),
                    );
                    ui.add_space(6.0);

                    let mut draft = ui.ctx().data_mut(|d| {
                        d.get_temp::<String>(draft_id).unwrap_or_else(|| {
                            if with_time {
                                format!("{} 09:00", ymd(today()))
                            } else {
                                ymd(today())
                            }
                        })
                    });
                    let _ = date_field::show_picker_body(
                        ui,
                        &format!("cal_rs_{}", m.id),
                        &mut draft,
                        with_time,
                    );
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(draft_id, draft.clone()));

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("确定改期")
                                        .size(13.0)
                                        .color(Color32::WHITE)
                                        .strong(),
                                )
                                .fill(theme::shell_accent())
                                .rounding(Rounding::same(8.0))
                                .min_size(Vec2::new(100.0, 30.0)),
                            )
                            .clicked()
                        {
                            if draft.trim().is_empty() {
                                *status_line = "请先选择日期".into();
                            } else {
                                confirmed = Some(draft.clone());
                                close = true;
                            }
                        }
                        if ui
                            .add(
                                egui::Button::new(RichText::new("取消").size(13.0))
                                    .min_size(Vec2::new(64.0, 30.0)),
                            )
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
        });

    if let Some(due) = confirmed {
        reschedule_memo(svc, m, &due, tx);
        *status_line = "已改期".into();
    }
    if close {
        ui.ctx().data_mut(|d| {
            d.insert_temp(open_id, false);
            d.remove_temp::<String>(draft_id);
        });
    }
}

fn reschedule_memo(svc: &Arc<MemoService>, m: &MemoView, new_due: &str, tx: &Sender<BgMsg>) {
    let new_due = new_due.trim().to_string();
    let new_end = compute_rescheduled_end(m, &new_due);
    let svc = svc.clone();
    let id = m.id.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let Some(prev) = svc.list().into_iter().find(|x| x.id == id) else {
            let _ = tx.send(BgMsg::Error("备忘不存在或已删除".into()));
            return;
        };
        match svc.edit_full(
            &prev.id,
            &prev.title,
            &prev.content,
            prev.visibility,
            MemoLifecycle::Permanent,
            prev.category,
            &new_due,
            &new_end,
            prev.done,
            &prev.tags,
            prev.priority,
            prev.remind_before_days,
            "", // 改期后重置提醒已读标记
        ) {
            Ok(()) => {
                let _ = tx.send(BgMsg::Info(format!(
                    "「{}」已改期为 {}",
                    prev.title,
                    memo_core::display_due(&new_due)
                )));
                let _ = tx.send(BgMsg::Refresh);
            }
            Err(e) => {
                let _ = tx.send(BgMsg::Error(format!("改期失败: {e}")));
            }
        }
    });
}

/// 跨度事件改期时按原时长平移结束日；非跨度清空 end。
fn compute_rescheduled_end(m: &MemoView, new_due: &str) -> String {
    if !is_span(m) {
        return String::new();
    }
    let (Some(old_start), Some(new_start), Some(old_end)) = (
        due_date_part(&m.due_date),
        due_date_part(new_due),
        event_end_date(&m.due_date, &m.end_date),
    ) else {
        return m.end_date.clone();
    };
    let delta = new_start.signed_duration_since(old_start);
    date_field::format_ymd(old_end + delta)
}

fn place_today(svc: &Arc<MemoService>, m: &MemoView, tx: &Sender<BgMsg>) {
    let due = format!("{} 09:00", ymd(today()));
    let svc = svc.clone();
    let id = m.id.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let Some(prev) = svc.list().into_iter().find(|x| x.id == id) else {
            let _ = tx.send(BgMsg::Error("备忘不存在或已删除".into()));
            return;
        };
        match svc.edit_full(
            &prev.id,
            &prev.title,
            &prev.content,
            prev.visibility,
            MemoLifecycle::Permanent,
            prev.category,
            &due,
            "",
            prev.done,
            &prev.tags,
            prev.priority,
            prev.remind_before_days,
            &prev.remind_seen_for,
        ) {
            Ok(()) => {
                let _ = tx.send(BgMsg::Info(format!("「{}」已放到今天", prev.title)));
                let _ = tx.send(BgMsg::Refresh);
            }
            Err(e) => {
                let _ = tx.send(BgMsg::Error(format!("放到今天失败: {e}")));
            }
        }
    });
}
