//! 主界面月历：事件色块、过期/到期预警、当天分组侧栏与日历化详情。

use crate::date_field;
use crate::doc_editor;
use crate::huangli;
use crate::memo_form;
use crate::theme;
use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use eframe::egui::{self, Color32, Frame, Id, Margin, Order, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::service::MemoView;
use memo_core::store::{MemoCategory, MemoPriority};
use memo_core::{
    covers_calendar_day, deadline_date, due_date_part, due_time_hm, event_end_date, is_due_today,
    is_overdue, overdue_days,
};
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

pub struct CalendarUi {
    pub year: i32,
    pub month: u32,
    pub selected: NaiveDate,
    pub filter: DayFilter,
    /// `+N 更多` 弹层所在日
    pub more_popup_day: Option<NaiveDate>,
    /// 点「查看过期任务」后侧栏优先展示过期分组
    pub focus_overdue: bool,
}

impl Default for CalendarUi {
    fn default() -> Self {
        let t = Local::now().date_naive();
        Self {
            year: t.year(),
            month: t.month(),
            selected: t,
            filter: DayFilter::Open,
            more_popup_day: None,
            focus_overdue: false,
        }
    }
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
    let (cal_w, side_w, full_h) = split_cal_detail(ui);

    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), full_h),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.set_max_height(full_h);
            ui.set_clip_rect(ui.max_rect());
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.allocate_ui_with_layout(
                Vec2::new(cal_w, full_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(cal_w);
                    ui.set_max_height(full_h);
                    ui.set_clip_rect(ui.max_rect());
                    show_month(ui, cal, all, &q, selected, editing, open_new, picked);
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(side_w, full_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(side_w);
                    ui.set_max_height(full_h);
                    ui.set_clip_rect(ui.max_rect());
                    show_day_side(
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
        },
    );
}

/// 左半日历 / 右半详情：各占可用宽度一半。
pub(crate) fn split_cal_detail(ui: &egui::Ui) -> (f32, f32, f32) {
    let gap = 8.0_f32;
    let avail = ui.available_width().max(gap + 2.0);
    let cal_w = ((avail - gap) * 0.5).max(160.0);
    let side_w = (avail - cal_w - gap).max(160.0);
    let full_h = ui.available_height().max(80.0);
    (cal_w, side_w, full_h)
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
    let overdue = collect_overdue(all, q);
    let due_today = collect_due_today(all, q);
    if !overdue.is_empty() || !due_today.is_empty() {
        show_warn_banner(ui, cal, &overdue, due_today.len());
        ui.add_space(6.0);
    }

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{}年{}月", cal.year, cal.month))
                .size(16.0)
                .strong()
                .color(theme::text()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("+").color(Color32::WHITE).strong().size(14.0))
                        .fill(theme::shell_accent())
                        .rounding(Rounding::same(6.0))
                        .min_size(Vec2::new(28.0, 26.0)),
                )
                .clicked()
            {
                *open_new = true;
            }
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
    let cell_w = ((ui.available_width() - 18.0) / 7.0).max(1.0);
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
    // 格子宽高随半幅日历等比（优先正方形）；垂直空间不够时再等比缩小，保证完整入窗
    let row_gap = 3.0_f32;
    let grid_h = ui.available_height().max(1.0);
    let cell_h_by_w = cell_w;
    let cell_h_by_h = ((grid_h - row_gap * 5.0) / 6.0).max(1.0);
    let cell_h = cell_h_by_w.min(cell_h_by_h).min(96.0);
    let first = NaiveDate::from_ymd_opt(cal.year, cal.month, 1).unwrap_or_else(today);
    let pad = first.weekday().num_days_from_monday() as i64;
    let start = first - Duration::try_days(pad).unwrap_or_default();
    for week in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for d in 0..7 {
                let day = start + Duration::try_days(week * 7 + d).unwrap_or_default();
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
                    picked,
                );
            }
        });
        if week < 5 {
            ui.add_space(row_gap);
        }
    }
}

fn show_warn_banner(
    ui: &mut egui::Ui,
    cal: &mut CalendarUi,
    overdue: &[&MemoView],
    due_today_n: usize,
) {
    let n_od = overdue.len();
    let mut parts = Vec::new();
    if n_od > 0 {
        parts.push(format!("{n_od} 条任务已过期"));
    }
    if due_today_n > 0 {
        parts.push(format!("{due_today_n} 条今天到期"));
    }
    let text = format!("⚠ 有 {}", parts.join("，"));
    Frame::none()
        .fill(theme::warn_overdue_bg())
        .stroke(Stroke::new(1.0, theme::warn_overdue_border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(text)
                        .size(12.5)
                        .color(theme::warn_overdue_fg()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if n_od > 0
                        && ui
                            .add(
                                egui::Button::new(
                                    RichText::new("查看过期任务")
                                        .size(12.0)
                                        .color(theme::warn_overdue_fg()),
                                )
                                .fill(Color32::WHITE)
                                .stroke(Stroke::new(1.0, theme::warn_overdue_fg()))
                                .rounding(Rounding::same(6.0)),
                            )
                            .clicked()
                    {
                        cal.focus_overdue = true;
                        if let Some(d) = overdue
                            .first()
                            .and_then(|m| deadline_date(&m.due_date, &m.end_date))
                        {
                            cal.year = d.year();
                            cal.month = d.month();
                            cal.selected = d;
                        }
                    }
                });
            });
        });
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
    let fill = if is_sel {
        theme::accent_soft()
    } else if day == t {
        theme::calendar_today_bg()
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

    let spans: Vec<_> = on.iter().copied().filter(|m| is_span(m)).collect();
    let singles: Vec<_> = on.iter().copied().filter(|m| !is_span(m)).collect();
    let mut y = rect.top() + 30.0;
    let mut shown = 0usize;
    let mut chip_hit = false;
    let bottom = rect.bottom() - 2.0;
    let fits = |y: f32, h: f32| y + h <= bottom;

    if fits(y, 14.0) {
        if let Some(m) = spans.first() {
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 4.0, y),
                Vec2::new(rect.width() - 8.0, 14.0),
            );
            let start = due_date_part(&m.due_date);
            let end = event_end_date(&m.due_date, &m.end_date);
            let fill_c = if memo_overdue(m) {
                theme::warn_overdue_fg()
            } else {
                theme::category_icon_color(m.category.canonical())
            };
            ui.painter()
                .rect_filled(bar, Rounding::same(3.0), fill_c);
            let label = if start == Some(day) {
                if let (Some(a), Some(b)) = (start, end) {
                    if b > a {
                        format!(
                            "{} ({}号 - {}号)",
                            truncate_title(&m.title, 10),
                            a.day(),
                            b.day()
                        )
                    } else {
                        truncate_title(&m.title, 14)
                    }
                } else {
                    truncate_title(&m.title, 14)
                }
            } else {
                format!("{} (续)", truncate_title(&m.title, 10))
            };
            ui.painter().text(
                egui::pos2(bar.left() + 4.0, bar.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(10.0),
                Color32::WHITE,
            );
            // 点备忘条只选中当天，不打开详情（与点格子空白一致）
            let id = ui.id().with(("span", day, &m.id));
            let r = ui.interact(bar, id, Sense::click());
            if r.clicked() {
                chip_hit = true;
                select_calendar_day(cal, day, selected, editing);
            }
            y += 16.0;
            shown += 1;
        }
    }

    let open: Vec<_> = singles.iter().copied().filter(|m| !m.done).collect();
    let chip_src = if open.is_empty() { singles } else { open };
    // 矮格子少画几条，把空间留给「+N 更多」
    let slots = if h < 48.0 {
        0
    } else if h < 64.0 {
        if spans.is_empty() { 1 } else { 0 }
    } else if h < 80.0 {
        if spans.is_empty() { 2 } else { 1 }
    } else if spans.is_empty() {
        3
    } else {
        2
    };
    for m in chip_src.iter().take(slots) {
        if !fits(y, 13.0) {
            break;
        }
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 4.0, y),
            Vec2::new(rect.width() - 8.0, 13.0),
        );
        let (bg, fg) = if memo_overdue(m) {
            (theme::warn_overdue_bg(), theme::warn_overdue_fg())
        } else if memo_due_today(m) {
            (theme::warn_due_today_bg(), theme::warn_due_today_fg())
        } else {
            (
                theme::chip_soft_bg(m.category),
                theme::chip_soft_fg(m.category),
            )
        };
        ui.painter().rect_filled(bar, Rounding::same(3.0), bg);
        ui.painter().text(
            egui::pos2(bar.left() + 4.0, bar.center().y),
            egui::Align2::LEFT_CENTER,
            truncate_title(&m.title, 12),
            egui::FontId::proportional(10.0),
            fg,
        );
        let id = ui.id().with(("chip", day, &m.id));
        let r = ui.interact(bar, id, Sense::click());
        if r.clicked() {
            chip_hit = true;
            select_calendar_day(cal, day, selected, editing);
        }
        y += 15.0;
        shown += 1;
    }

    let hidden = on.len().saturating_sub(shown);
    if hidden > 0 && fits(y, 14.0) {
        let more_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 4.0, y),
            Vec2::new(rect.width() - 8.0, 14.0),
        );
        ui.painter().text(
            egui::pos2(more_rect.left(), more_rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("+{hidden} 更多"),
            egui::FontId::proportional(10.5),
            theme::text_muted(),
        );
        let id = ui.id().with(("more", day));
        let r = ui.interact(more_rect, id, Sense::click());
        if r.clicked() {
            chip_hit = true;
            cal.selected = day;
            cal.more_popup_day = Some(day);
        }
        if cal.more_popup_day == Some(day) {
            show_more_popup(ui, cal, &on, day, more_rect, selected, editing, picked);
        }
    } else if cal.more_popup_day == Some(day) {
        cal.more_popup_day = None;
    }

    if resp.clicked() && !chip_hit {
        select_calendar_day(cal, day, selected, editing);
    }
}

/// 选中日历某一天并收起详情（不打开备忘）。
fn select_calendar_day(
    cal: &mut CalendarUi,
    day: NaiveDate,
    selected: &mut Option<String>,
    editing: &mut bool,
) {
    cal.selected = day;
    *selected = None;
    *editing = false;
    cal.more_popup_day = None;
    cal.focus_overdue = false;
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
                                let r = ui.add(
                                    egui::Button::new(
                                        RichText::new(format!("    {title}"))
                                            .size(12.5)
                                            .color(theme::text()),
                                    )
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
    _open_new: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    pick_delete: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    let overdue = collect_overdue(all, q);
    let due_today = collect_due_today(all, q);
    let day_events = events_on(all, cal.selected, q);
    let other: Vec<_> = day_events
        .iter()
        .copied()
        .filter(|m| !memo_overdue(m) && !(cal.selected == today() && memo_due_today(m)))
        .filter(|m| match cal.filter {
            DayFilter::Open => !m.done,
            DayFilter::All => true,
            DayFilter::Done => m.done,
        })
        .collect();

    // 摘要卡
    if !overdue.is_empty() {
        summary_card(
            ui,
            theme::warn_overdue_bg(),
            theme::warn_overdue_fg(),
            theme::warn_overdue_border(),
            "🔥",
            &format!("{} 条任务已过期", overdue.len()),
            "请尽快处理，或延期到新日期",
        );
        ui.add_space(6.0);
    }
    if !due_today.is_empty() {
        summary_card(
            ui,
            theme::warn_due_today_bg(),
            theme::warn_due_today_fg(),
            theme::warn_due_today_border(),
            "⏰",
            &format!("{} 条任务今天到期", due_today.len()),
            "记得在今日内完成",
        );
        ui.add_space(6.0);
    }

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} · {}",
                format_md(cal.selected),
                weekday_zh(cal.selected)
            ))
            .size(14.0)
            .strong()
            .color(theme::text()),
        );
    });
    ui.add_space(6.0);
    show_huangli_for_day(ui, cal.selected);
    ui.add_space(6.0);

    let list_h = ui.available_height().max(40.0);
    egui::ScrollArea::vertical()
        .id_source("cal_day_list")
        .auto_shrink([false, false])
        .max_height(list_h)
        .show(ui, |ui| {
            if cal.focus_overdue || !overdue.is_empty() {
                section_header(
                    ui,
                    "⚠ 已过期",
                    overdue.len(),
                    theme::warn_overdue_fg(),
                    theme::warn_overdue_bg(),
                );
                for m in &overdue {
                    warn_task_card(
                        ui,
                        svc,
                        m,
                        TaskAccent::Overdue,
                        selected,
                        editing,
                        picked,
                        pick_edit,
                        pick_delete,
                        open_sticky,
                        tx,
                    );
                }
                ui.add_space(8.0);
                if cal.focus_overdue {
                    cal.focus_overdue = false;
                }
            }

            if !due_today.is_empty() {
                section_header(
                    ui,
                    "⏰ 今天到期",
                    due_today.len(),
                    theme::warn_due_today_fg(),
                    theme::warn_due_today_bg(),
                );
                for m in &due_today {
                    warn_task_card(
                        ui,
                        svc,
                        m,
                        TaskAccent::DueToday,
                        selected,
                        editing,
                        picked,
                        pick_edit,
                        pick_delete,
                        open_sticky,
                        tx,
                    );
                }
                ui.add_space(8.0);
            }

            section_header(
                ui,
                "其他任务",
                other.len(),
                theme::text_muted(),
                theme::panel(),
            );
            if other.is_empty() {
                ui.label(
                    RichText::new("这天没有其它安排")
                        .size(12.5)
                        .color(theme::text_muted()),
                );
            } else {
                for m in &other {
                    warn_task_card(
                        ui,
                        svc,
                        m,
                        TaskAccent::Normal,
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

            ui.add_space(12.0);
            show_unscheduled_block(ui, svc, cal, all, q, selected, editing, picked, pick_edit, pick_delete, tx);
        });
}

fn summary_card(
    ui: &mut egui::Ui,
    bg: Color32,
    fg: Color32,
    border: Color32,
    icon: &str,
    title: &str,
    sub: &str,
) {
    let full = ui.available_width();
    let (_, resp) = ui.allocate_exact_size(Vec2::new(full, 52.0), Sense::hover());
    ui.painter()
        .rect(resp.rect, Rounding::same(8.0), bg, Stroke::new(1.0, border));
    let accent = egui::Rect::from_min_max(
        resp.rect.left_top(),
        egui::pos2(resp.rect.left() + 4.0, resp.rect.bottom()),
    );
    ui.painter()
        .rect_filled(accent, Rounding::same(2.0), fg);
    let title_g = theme::layout_galley(
        ui,
        &format!("{icon}  {title}"),
        egui::FontId::proportional(13.0),
        fg,
    );
    let sub_g = theme::layout_galley(
        ui,
        sub,
        egui::FontId::proportional(11.0),
        theme::text_muted(),
    );
    ui.painter().galley(
        egui::pos2(resp.rect.left() + 14.0, resp.rect.top() + 10.0),
        title_g,
        fg,
    );
    ui.painter().galley(
        egui::pos2(resp.rect.left() + 14.0, resp.rect.top() + 30.0),
        sub_g,
        theme::text_muted(),
    );
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

/// 选中日黄历：农历即时显示；宜忌后台算完再刷新。
fn show_huangli_for_day(ui: &mut egui::Ui, day: NaiveDate) {
    if let Some(al) = huangli::cached_full(day) {
        show_huangli_card(ui, Some(&al), &al.lunar_line());
    } else {
        let line = huangli::lunar_summary_line(day);
        show_huangli_card(ui, None, &line);
        huangli::ensure_full_async(day, ui.ctx().clone());
    }
}

fn show_huangli_card(ui: &mut egui::Ui, al: Option<&huangli::DayAlmanac>, lunar_line: &str) {
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(lunar_line)
                    .size(12.0)
                    .color(theme::text()),
            );
            ui.add_space(4.0);
            match al {
                Some(al) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("宜")
                                .size(12.0)
                                .strong()
                                .color(theme::success()),
                        );
                        ui.label(
                            RichText::new(al.yi_text(40))
                                .size(12.0)
                                .color(theme::success()),
                        );
                    });
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("忌")
                                .size(12.0)
                                .strong()
                                .color(theme::danger()),
                        );
                        ui.label(
                            RichText::new(al.ji_text(40))
                                .size(12.0)
                                .color(theme::danger().linear_multiply(0.85)),
                        );
                    });
                }
                None => {
                    ui.label(
                        RichText::new("宜忌加载中…")
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                }
            }
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
    let (bar_c, soft_bg) = match accent {
        TaskAccent::Overdue => (theme::warn_overdue_fg(), theme::warn_overdue_bg()),
        TaskAccent::DueToday => (theme::warn_due_today_fg(), theme::warn_due_today_bg()),
        TaskAccent::Normal => (theme::shell_accent(), theme::card()),
    };
    let mut consumed = false;
    let outer = Frame::none()
        .fill(soft_bg)
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                if warn_card_category_icon(ui, m.category, m.done).clicked() {
                    toggle_done(svc, m, tx);
                    consumed = true;
                }
                ui.vertical(|ui| {
                    let title = if m.title.is_empty() {
                        "(无标题)"
                    } else {
                        m.title.as_str()
                    };
                    ui.label(
                        RichText::new(title)
                            .size(13.5)
                            .strong()
                            .color(if m.done {
                                theme::text_muted()
                            } else {
                                theme::text()
                            }),
                    );
                    ui.horizontal_wrapped(|ui| {
                        match accent {
                            TaskAccent::Overdue => {
                                let days = overdue_days(&m.due_date, &m.end_date, m.done)
                                    .unwrap_or(1)
                                    .max(1);
                                status_badge(
                                    ui,
                                    &format!("已过期 {days} 天"),
                                    Color32::WHITE,
                                    theme::warn_overdue_fg(),
                                    theme::warn_overdue_border(),
                                );
                                if let Some(d) = deadline_date(&m.due_date, &m.end_date) {
                                    ui.label(
                                        RichText::new(format!("截止 {}", format_md(d)))
                                            .size(11.0)
                                            .color(theme::text_muted()),
                                    );
                                }
                            }
                            TaskAccent::DueToday => {
                                let badge = if let Some(hm) = due_time_hm(&m.due_date) {
                                    format!("今天 {hm} 截止")
                                } else {
                                    "今天到期".into()
                                };
                                status_badge(
                                    ui,
                                    &badge,
                                    Color32::WHITE,
                                    theme::warn_due_today_fg(),
                                    theme::warn_due_today_border(),
                                );
                            }
                            TaskAccent::Normal => {
                                if is_span(m) {
                                    if let (Some(a), Some(b)) = (
                                        due_date_part(&m.due_date),
                                        event_end_date(&m.due_date, &m.end_date),
                                    ) {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} - {}",
                                                format_md(a),
                                                format_md(b)
                                            ))
                                            .size(11.0)
                                            .color(theme::shell_accent()),
                                        );
                                    }
                                } else {
                                    ui.label(
                                        RichText::new(if m.done { "已完成" } else { "未完成" })
                                            .size(11.0)
                                            .color(theme::text_muted()),
                                    );
                                }
                            }
                        }
                    });
                    if !m.tags.is_empty() {
                        ui.add_space(2.0);
                        ui.horizontal_wrapped(|ui| {
                            for (i, tag) in m.tags.iter().enumerate() {
                                let (bg, fg) = theme::tag_soft_pair(i as u64 + tag.len() as u64);
                                let label = if tag.starts_with('#') {
                                    tag.clone()
                                } else {
                                    format!("#{tag}")
                                };
                                Frame::none()
                                    .fill(bg)
                                    .rounding(Rounding::same(99.0))
                                    .inner_margin(Margin::symmetric(6.0, 1.0))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(label).size(11.0).color(fg));
                                    });
                            }
                        });
                    }
                });
            });
        });
    // 左色条
    let r = outer.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 4.0, r.bottom())),
        Rounding {
            nw: 8.0,
            ne: 0.0,
            sw: 8.0,
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
    ui.add_space(5.0);
}

fn status_badge(ui: &mut egui::Ui, text: &str, bg: Color32, fg: Color32, border: Color32) {
    Frame::none()
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .rounding(Rounding::same(4.0))
        .inner_margin(Margin::symmetric(6.0, 2.0))
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
            let uns = unscheduled(all, q);
            if uns.is_empty() {
                ui.label(
                    RichText::new("没有随手记")
                        .size(12.5)
                        .color(theme::text_muted()),
                );
            } else {
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
            }
        });
}

/// 日历选中备忘后的只读详情卡。
pub fn show_event_detail(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    m: &MemoView,
    selected: &mut Option<String>,
    editing: &mut bool,
    show_delete: &mut bool,
    status_line: &mut String,
    tx: &Sender<BgMsg>,
) {
    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::new("← 返回列表").min_size(Vec2::new(88.0, 26.0)))
            .clicked()
        {
            *selected = None;
            *editing = false;
        }
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
                *editing = true;
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
                .color(theme::text());
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
                let (st_bg, st_fg, st_label) = if m.done {
                    (theme::panel(), theme::text_muted(), "已完成")
                } else {
                    (theme::success_soft(), theme::success(), "进行中")
                };
                detail_attr_pill(ui, &format!("状态：{st_label}"), st_bg, st_fg);
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
                        toggle_done(svc, m, tx);
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
                    toggle_done(svc, m, tx);
                }
            });
            ui.add_space(16.0);
        });
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

fn toggle_done(svc: &Arc<MemoService>, m: &MemoView, tx: &Sender<BgMsg>) {
    let svc = svc.clone();
    let id = m.id.clone();
    let next = !m.done;
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
    if ui.button("打开").clicked() {
        pick_memo(m, selected, editing, picked, pick_edit, false);
        ui.close_menu();
    }
    if ui.button("编辑").clicked() {
        pick_memo(m, selected, editing, picked, pick_edit, true);
        ui.close_menu();
    }
    if ui.button("生成桌面便签").clicked() {
        *open_sticky = Some(m.id.clone());
        ui.close_menu();
    }
    let done_l = if m.done { "取消完成" } else { "标记完成" };
    if ui.button(done_l).clicked() {
        toggle_done(svc, m, tx);
        ui.close_menu();
    }
    if ui
        .add(egui::Button::new(
            RichText::new("移入回收站…").color(theme::danger()),
        ))
        .clicked()
    {
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
