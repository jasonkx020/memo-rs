//! 主界面月历：事件色块、过期/到期预警、当天分组侧栏与日历化详情。

use crate::doc_editor;
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

    let spans: Vec<_> = on.iter().copied().filter(|m| is_span(m)).collect();
    let singles: Vec<_> = on.iter().copied().filter(|m| !is_span(m)).collect();
    let mut y = rect.top() + 22.0;
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
            let id = ui.id().with(("span", day, &m.id));
            let r = ui.interact(bar, id, Sense::click());
            if r.clicked() {
                chip_hit = true;
                *selected = Some(m.id.clone());
                *picked = Some(m.id.clone());
                *editing = false;
                cal.selected = day;
                cal.more_popup_day = None;
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
            *selected = Some(m.id.clone());
            *picked = Some(m.id.clone());
            *editing = false;
            cal.selected = day;
            cal.more_popup_day = None;
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
        cal.selected = day;
        *selected = None;
        *editing = false;
        cal.more_popup_day = None;
        cal.focus_overdue = false;
    }
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
                                        RichText::new(format!("●  {title}"))
                                            .size(12.5)
                                            .color(theme::text()),
                                    )
                                    .fill(Color32::TRANSPARENT)
                                    .frame(false),
                                );
                                // 色点覆盖绘制
                                let _ = dot;
                                if r.clicked() {
                                    *selected = Some(m.id.clone());
                                    *picked = Some(m.id.clone());
                                    *editing = false;
                                    cal.selected = day;
                                    cal.more_popup_day = None;
                                }
                                // 左侧色点
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
    ui.add_space(4.0);

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
    ui.painter().text(
        egui::pos2(resp.rect.left() + 14.0, resp.rect.top() + 10.0),
        egui::Align2::LEFT_TOP,
        format!("{icon}  {title}"),
        egui::FontId::proportional(13.0),
        fg,
    );
    ui.painter().text(
        egui::pos2(resp.rect.left() + 14.0, resp.rect.top() + 30.0),
        egui::Align2::LEFT_TOP,
        sub,
        egui::FontId::proportional(11.0),
        theme::text_muted(),
    );
}

fn section_header(ui: &mut egui::Ui, title: &str, n: usize, fg: Color32, badge_bg: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(13.0).strong().color(fg));
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
                let mark = if m.done { "☑" } else { "☐" };
                if ui
                    .add(
                        egui::Button::new(RichText::new(mark).size(14.0).color(theme::text_muted()))
                            .fill(Color32::TRANSPARENT)
                            .frame(false),
                    )
                    .clicked()
                {
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
                                    theme::warn_overdue_fg(),
                                    Color32::WHITE,
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
                                    theme::warn_due_today_fg(),
                                    Color32::WHITE,
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
            tx,
        );
    });
    ui.add_space(5.0);
}

fn status_badge(ui: &mut egui::Ui, text: &str, bg: Color32, fg: Color32) {
    Frame::none()
        .fill(bg)
        .rounding(Rounding::same(4.0))
        .inner_margin(Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(fg));
        });
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
            if ui
                .add(
                    egui::Button::new(RichText::new("🗑").size(14.0))
                        .min_size(Vec2::new(28.0, 26.0)),
                )
                .on_hover_text("删除")
                .clicked()
            {
                *show_delete = true;
            }
            if ui
                .add(
                    egui::Button::new(RichText::new("📋").size(14.0))
                        .min_size(Vec2::new(28.0, 26.0)),
                )
                .on_hover_text("复制全文")
                .clicked()
            {
                ui.output_mut(|o| {
                    o.copied_text = format!("{}\n\n{}", m.title, m.content);
                });
                *status_line = "已复制到剪贴板".into();
            }
            if ui
                .add(
                    egui::Button::new(RichText::new("✏").size(14.0))
                        .min_size(Vec2::new(28.0, 26.0)),
                )
                .on_hover_text("编辑")
                .clicked()
            {
                *editing = true;
            }
        });
    });
    ui.add_space(8.0);

    // 顶栏固定；正文可滚，避免底部操作被裁切
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
            ui.add_space(12.0);

            ui.label(
                RichText::new("时间")
                    .size(12.0)
                    .strong()
                    .color(theme::text_muted()),
            );
            ui.add_space(4.0);
            if let Some(start) = due_date_part(&m.due_date) {
                ui.label(
                    RichText::new(format!(
                        "📅  {}年{}月{}日 ({})",
                        start.year(),
                        start.month(),
                        start.day(),
                        weekday_zh(start)
                    ))
                    .size(13.0)
                    .color(theme::text()),
                );
                ui.add_space(4.0);
                let time_line = if is_span(m) {
                    if let Some(end) = event_end_date(&m.due_date, &m.end_date) {
                        format!("{} ～ {}", format_md(start), format_md(end))
                    } else {
                        format_md(start)
                    }
                } else if let Some(hm) = due_time_hm(&m.due_date) {
                    format!("{hm} 开始")
                } else {
                    "全天".into()
                };
                Frame::none()
                    .fill(theme::accent_soft())
                    .rounding(Rounding::same(99.0))
                    .inner_margin(Margin::symmetric(10.0, 4.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("🕐  {time_line}"))
                                .size(12.5)
                                .color(theme::shell_accent()),
                        );
                    });
            }
            ui.add_space(12.0);

            ui.label(
                RichText::new("属性")
                    .size(12.0)
                    .strong()
                    .color(theme::text_muted()),
            );
            ui.add_space(4.0);
            attr_row(ui, "🚩 优先级", |ui| {
                Frame::none()
                    .fill(theme::priority_pill_bg(m.priority))
                    .rounding(Rounding::same(99.0))
                    .inner_margin(Margin::symmetric(8.0, 2.0))
                    .show(ui, |ui| {
                        let label = match m.priority {
                            MemoPriority::High => "高优先级",
                            MemoPriority::Normal => "普通",
                            MemoPriority::Low => "低",
                        };
                        ui.label(
                            RichText::new(label)
                                .size(12.0)
                                .color(theme::priority_pill_fg(m.priority)),
                        );
                    });
            });
            attr_row(ui, "☑ 状态", |ui| {
                ui.label(
                    RichText::new(if m.done { "已完成" } else { "进行中" })
                        .size(13.0)
                        .color(if m.done {
                            theme::success()
                        } else {
                            theme::text()
                        }),
                );
            });
            attr_row(ui, "📁 分类", |ui| {
                Frame::none()
                    .fill(theme::chip_soft_bg(m.category))
                    .rounding(Rounding::same(99.0))
                    .inner_margin(Margin::symmetric(8.0, 2.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(m.category.label())
                                .size(12.0)
                                .color(theme::chip_soft_fg(m.category)),
                        );
                    });
            });
            ui.add_space(10.0);

            if !m.tags.is_empty() {
                ui.label(
                    RichText::new("标签")
                        .size(12.0)
                        .strong()
                        .color(theme::text_muted()),
                );
                ui.add_space(4.0);
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
                            .inner_margin(Margin::symmetric(8.0, 2.0))
                            .show(ui, |ui| {
                                ui.label(RichText::new(label).size(12.0).color(fg));
                            });
                    }
                });
                ui.add_space(10.0);
            }

            ui.label(
                RichText::new("描述")
                    .size(12.0)
                    .strong()
                    .color(theme::text_muted()),
            );
            ui.add_space(4.0);
            Frame::none()
                .fill(theme::panel())
                .rounding(Rounding::same(10.0))
                .inner_margin(Margin::same(10.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if m.content.trim().is_empty() {
                        ui.label(
                            RichText::new("暂无描述")
                                .size(12.5)
                                .color(theme::text_muted()),
                        );
                    } else {
                        doc_editor::show_viewer_body(ui, &m.content);
                    }
                });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("📅  改期").size(13.0))
                            .min_size(Vec2::new(88.0, 32.0)),
                    )
                    .clicked()
                {
                    *editing = true;
                    *status_line = "请修改日期后保存".into();
                }
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
                            RichText::new("✓ 标记完成")
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

fn attr_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .size(12.5)
                .color(theme::text_muted()),
        );
        ui.add_space(8.0);
        add(ui);
    });
    ui.add_space(4.0);
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
