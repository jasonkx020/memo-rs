//! 主界面月历：过去/今天/未来底色、跨天色条、当天清单与未安排。

use crate::theme;
use chrono::{Datelike, Duration, Local, NaiveDate};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::service::MemoView;
use memo_core::store::MemoCategory;
use memo_core::{covers_calendar_day, due_date_part, event_end_date};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use crate::msg::BgMsg;
use memo_core::service::MemoService;
use memo_core::store::MemoLifecycle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl Default for CalendarUi {
    fn default() -> Self {
        let t = Local::now().date_naive();
        Self {
            year: t.year(),
            month: t.month(),
            selected: t,
            filter: DayFilter::Open,
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
        .filter(|m| on_calendar(m) && covers_calendar_day(&m.due_date, &m.end_date, day) && search_hit(m, q))
        .collect()
}

fn is_span(m: &MemoView) -> bool {
    match (due_date_part(&m.due_date), event_end_date(&m.due_date, &m.end_date)) {
        (Some(a), Some(b)) => b > a,
        _ => false,
    }
}

fn cat_chip_color(cat: MemoCategory) -> Color32 {
    theme::category_icon_color(cat.canonical())
}

fn chip_fill(cat: MemoCategory, past: bool) -> Color32 {
    let c = cat_chip_color(cat);
    if !past {
        return c;
    }
    Color32::from_rgb(
        ((c.r() as u16 + 0x9A * 2) / 3) as u8,
        ((c.g() as u16 + 0xA1 * 2) / 3) as u8,
        ((c.b() as u16 + 0xAD * 2) / 3) as u8,
    )
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
    let avail = ui.available_width();
    let side_w = 320.0_f32.min(avail * 0.38).max(240.0);
    let cal_w = (avail - side_w - 8.0).max(200.0);
    let full_h = ui.available_height().max(120.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.allocate_ui_with_layout(
            Vec2::new(cal_w, full_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(cal_w);
                show_month(ui, cal, all, &q, selected, editing, open_new);
            },
        );
        ui.allocate_ui_with_layout(
            Vec2::new(side_w, full_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(side_w);
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
    });
}

pub(crate) fn show_month(
    ui: &mut egui::Ui,
    cal: &mut CalendarUi,
    all: &[MemoView],
    q: &str,
    selected: &mut Option<String>,
    editing: &mut bool,
    open_new: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{}年{}月", cal.year, cal.month))
                .size(15.0)
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
            if ui.add_sized(Vec2::new(28.0, 26.0), egui::Button::new("›")).clicked() {
                shift_month(cal, 1);
            }
            if ui.add_sized(Vec2::new(28.0, 26.0), egui::Button::new("‹")).clicked() {
                shift_month(cal, -1);
            }
        });
    });
    ui.add_space(4.0);
    let cell_w = ((ui.available_width() - 18.0) / 7.0).max(36.0);
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
    let first = NaiveDate::from_ymd_opt(cal.year, cal.month, 1).unwrap_or_else(today);
    let pad = first.weekday().num_days_from_monday() as i64;
    let start = first - Duration::try_days(pad).unwrap_or_default();
    for week in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for d in 0..7 {
                let day = start + Duration::try_days(week * 7 + d).unwrap_or_default();
                day_cell(ui, cal, all, q, day, cell_w, selected, editing);
            }
        });
        ui.add_space(3.0);
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
    selected: &mut Option<String>,
    editing: &mut bool,
) {
    let t = today();
    let in_month = day.month() == cal.month && day.year() == cal.year;
    let is_sel = day == cal.selected;
    let h = 78.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(cell_w.max(40.0), h), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        cal.selected = day;
        *selected = None;
        *editing = false;
    }
    let past = day < t;
    let fill = if day == t {
        theme::calendar_today_bg()
    } else if past {
        theme::calendar_past_bg()
    } else {
        theme::calendar_future_bg()
    };
    let mut stroke_c = if day == t {
        theme::calendar_today_fg()
    } else if past {
        Color32::from_rgb(0xD0, 0xD3, 0xDA)
    } else {
        Color32::from_rgb(0xC5, 0xE4, 0xD6)
    };
    let mut sw = if day == t { 1.4 } else { 1.0 };
    if is_sel {
        stroke_c = theme::shell_accent();
        sw = 2.0;
    }
    ui.painter()
        .rect(rect, Rounding::same(6.0), fill, Stroke::new(sw, stroke_c));
    let mark = egui::Rect::from_min_max(
        rect.left_top(),
        egui::pos2(rect.left() + 3.0, rect.bottom()),
    );
    let mark_c = if day == t {
        theme::calendar_today_fg()
    } else if past {
        theme::calendar_past_fg()
    } else {
        theme::calendar_future_fg()
    };
    ui.painter()
        .rect_filled(mark, Rounding::same(2.0), mark_c);
    if !in_month {
        ui.painter().rect_filled(
            rect,
            Rounding::same(6.0),
            if past {
                Color32::from_black_alpha(18)
            } else {
                Color32::from_white_alpha(90)
            },
        );
    }
    let num_fg = if !in_month {
        theme::text_muted()
    } else if day == t {
        theme::calendar_today_fg()
    } else if past {
        theme::calendar_past_fg()
    } else {
        theme::calendar_future_fg()
    };
    ui.painter().text(
        egui::pos2(rect.left() + 8.0, rect.top() + 4.0),
        egui::Align2::LEFT_TOP,
        format!("{}", day.day()),
        egui::FontId::proportional(if day == t { 13.0 } else { 12.0 }),
        num_fg,
    );
    if day == t && in_month {
        let badge = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 14.0, rect.top() + 11.0),
            Vec2::new(18.0, 14.0),
        );
        ui.painter().rect_filled(
            badge,
            Rounding::same(4.0),
            theme::calendar_today_fg(),
        );
        ui.painter().text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            "今",
            egui::FontId::proportional(9.0),
            Color32::WHITE,
        );
    }
    let on = events_on(all, day, q);
    let spans: Vec<_> = on.iter().copied().filter(|m| is_span(m)).collect();
    let singles: Vec<_> = on.iter().copied().filter(|m| !is_span(m)).collect();
    let mut y = rect.top() + 20.0;
    let mut shown = 0usize;
    if let Some(m) = spans.first() {
        let bar = egui::Rect::from_min_size(egui::pos2(rect.left() + 6.0, y), Vec2::new(rect.width() - 9.0, 14.0));
        let start = due_date_part(&m.due_date);
        ui.painter()
            .rect_filled(bar, Rounding::same(3.0), chip_fill(m.category, past));
        if start == Some(day) {
            ui.painter().text(
                egui::pos2(bar.left() + 4.0, bar.center().y),
                egui::Align2::LEFT_CENTER,
                &m.title,
                egui::FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
        y += 16.0;
        shown += 1;
    }
    let open: Vec<_> = singles.iter().copied().filter(|m| !m.done).collect();
    let chip_src = if open.is_empty() { singles } else { open };
    let slots = if spans.is_empty() { 2 } else { 1 };
    for m in chip_src.iter().take(slots) {
        let bar = egui::Rect::from_min_size(egui::pos2(rect.left() + 6.0, y), Vec2::new(rect.width() - 9.0, 13.0));
        ui.painter()
            .rect_filled(bar, Rounding::same(3.0), chip_fill(m.category, past));
        ui.painter().text(
            egui::pos2(bar.left() + 4.0, bar.center().y),
            egui::Align2::LEFT_CENTER,
            &m.title,
            egui::FontId::proportional(10.0),
            Color32::WHITE,
        );
        y += 15.0;
        shown += 1;
    }
    let hidden = on.len().saturating_sub(shown);
    if hidden > 0 {
        ui.painter().text(
            egui::pos2(rect.left() + 6.0, y),
            egui::Align2::LEFT_TOP,
            format!("+{hidden}"),
            egui::FontId::proportional(10.5),
            theme::shell_accent(),
        );
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
    let t = today();
    let kind_fg = if cal.selected < t {
        theme::calendar_past_fg()
    } else if cal.selected == t {
        theme::calendar_today_fg()
    } else {
        theme::calendar_future_fg()
    };
    let kind = if cal.selected < t {
        "过去"
    } else if cal.selected == t {
        "今天"
    } else {
        "未来"
    };
    let list = events_on(all, cal.selected, q);
    let n_done = list.iter().filter(|m| m.done).count();
    let n_open = list.len() - n_done;
    let busy = list.len() >= 10;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{}月{}日", cal.selected.month(), cal.selected.day()))
                .size(15.0)
                .strong(),
        );
        ui.label(RichText::new("·").size(15.0).color(theme::text_muted()));
        ui.label(
            RichText::new(kind)
                .size(15.0)
                .strong()
                .color(kind_fg),
        );
        if !list.is_empty() {
            ui.label(
                RichText::new(format!(
                    "{} 条{}",
                    list.len(),
                    if n_done > 0 {
                        format!(" · {n_done} 已完成")
                    } else {
                        String::new()
                    }
                ))
                .size(12.5)
                .color(theme::text_muted()),
            );
        }
    });
    if busy || n_done > 0 {
        ui.horizontal(|ui| {
            filter_chip(ui, cal, DayFilter::Open, &format!("未完成 {n_open}"));
            filter_chip(ui, cal, DayFilter::All, &format!("全部 {}", list.len()));
            if n_done > 0 {
                filter_chip(ui, cal, DayFilter::Done, &format!("已完成 {n_done}"));
            }
        });
    }
    if !list.is_empty() {
        ui.label(
            RichText::new("右键可打开、编辑或移入回收站")
                .size(11.0)
                .color(theme::text_muted()),
        );
    }
    ui.add_space(4.0);
    let shown: Vec<_> = list
        .iter()
        .copied()
        .filter(|m| match cal.filter {
            DayFilter::Open => !m.done,
            DayFilter::All => true,
            DayFilter::Done => m.done,
        })
        .collect();
    egui::ScrollArea::vertical()
        .id_source("cal_day_list")
        .max_height(ui.available_height() - 120.0)
        .show(ui, |ui| {
            if shown.is_empty() {
                ui.label(
                    theme::muted_label(if list.is_empty() {
                        "这天还没有安排"
                    } else {
                        "这一屏没有条目"
                    }),
                );
            } else if busy {
                for m in &shown {
                    compact_row(ui, svc, m, selected, editing, picked, pick_edit, pick_delete, tx);
                }
            } else {
                for m in &shown {
                    card_row(ui, svc, m, selected, editing, picked, pick_edit, pick_delete, tx);
                }
            }
        });
    ui.add_space(10.0);
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
                ui.spacing_mut().item_spacing.y = 8.0;
                for m in uns {
                    Frame::none()
                        .fill(Color32::from_white_alpha(120))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::symmetric(8.0, 6.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                cat_icon(ui, m, 14.0);
                                // 左侧标题限宽，保证右侧按钮始终有可点区域
                                let btn_reserve = 168.0;
                                let left_w = (ui.available_width() - btn_reserve).max(48.0);
                                ui.allocate_ui_with_layout(
                                    Vec2::new(left_w, 26.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        let title = if m.title.is_empty() {
                                            "(无标题)".to_string()
                                        } else {
                                            m.title.clone()
                                        };
                                        let title_resp = ui
                                            .add(
                                                egui::Label::new(
                                                    RichText::new(title)
                                                        .size(13.0)
                                                        .color(title_color(m, true)),
                                                )
                                                .truncate(true)
                                                .sense(Sense::click()),
                                            );
                                        title_resp.context_menu(|ui| {
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
                                        for tag in m.tags.iter().take(1) {
                                            tag_pill(ui, tag);
                                        }
                                    },
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let place = ui.add(
                                            egui::Button::new(
                                                RichText::new("放到今天")
                                                    .size(11.0)
                                                    .color(theme::success()),
                                            )
                                            .fill(Color32::from_white_alpha(200))
                                            .stroke(Stroke::new(
                                                1.0,
                                                Color32::from_rgb(0xB7, 0xE4, 0xC7),
                                            ))
                                            .rounding(Rounding::same(6.0))
                                            .min_size(Vec2::new(72.0, 24.0)),
                                        );
                                        if place.clicked() {
                                            let t = today();
                                            cal.year = t.year();
                                            cal.month = t.month();
                                            cal.selected = t;
                                            place_today(svc, m, tx);
                                        }
                                        let _ = done_button(ui, svc, m, tx);
                                    },
                                );
                            });
                        });
                }
            }
        });
}

fn filter_chip(ui: &mut egui::Ui, cal: &mut CalendarUi, f: DayFilter, label: &str) {
    let on = cal.filter == f;
    let fill = if on {
        theme::shell_nav_selected()
    } else {
        theme::card()
    };
    let fg = if on {
        theme::shell_accent()
    } else {
        theme::text()
    };
    if ui
        .add(
            egui::Button::new(RichText::new(label).size(11.5).color(fg))
                .fill(fill)
                .stroke(Stroke::new(1.0, if on { Color32::from_rgb(0xBF, 0xDB, 0xFE) } else { theme::border() }))
                .rounding(Rounding::same(99.0)),
        )
        .clicked()
    {
        cal.filter = f;
    }
}

fn title_color(m: &MemoView, relaxed: bool) -> Color32 {
    if m.done {
        return theme::text_muted();
    }
    if relaxed {
        return theme::text_muted();
    }
    if memo_core::due_is_due_today(&m.due_date) {
        theme::danger()
    } else {
        theme::text()
    }
}

/// 标题按中心线绘制；图标由 `done_icon` 单独可点。
fn memo_line(
    ui: &mut egui::Ui,
    m: &MemoView,
    _icon_size: f32,
    title_size: f32,
    relaxed: bool,
    _with_icon: bool,
) {
    let title = if m.title.is_empty() {
        "(无标题)"
    } else {
        m.title.as_str()
    };
    let fg = title_color(m, relaxed);
    let title_g = theme::layout_galley(
        ui,
        title,
        egui::FontId::proportional(title_size),
        fg,
    );
    let h = title_size * 1.25;
    let w = title_g.size().x + 2.0;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(w.min(ui.available_width().max(w)), h),
        Sense::hover(),
    );
    let cy = rect.center().y;
    let title_pos = theme::galley_pos_left_center(egui::pos2(rect.left(), cy), &title_g);
    ui.painter().galley(title_pos, title_g.clone(), fg);
    if m.done {
        let x0 = title_pos.x;
        let x1 = (x0 + title_g.size().x).min(rect.right());
        ui.painter()
            .hline(x0..=x1, cy, Stroke::new(1.0, theme::text_muted()));
    }
}

fn tag_pill(ui: &mut egui::Ui, tag: &str) {
    let label = if tag.starts_with('#') {
        tag.to_string()
    } else {
        format!("#{tag}")
    };
    Frame::none()
        .fill(theme::shell_tag_bg())
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(6.0, 1.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label)
                    .size(11.0)
                    .color(theme::shell_tag_fg()),
            );
        });
}

/// 分类图标（仅展示，点击不完成）。
fn cat_icon(ui: &mut egui::Ui, m: &MemoView, size: f32) {
    let cat = m.category.canonical();
    let color = if m.done {
        let c = theme::category_icon_color(cat);
        let mute = theme::text_muted();
        Color32::from_rgb(
            ((c.r() as u16 + mute.r() as u16 * 2) / 3) as u8,
            ((c.g() as u16 + mute.g() as u16 * 2) / 3) as u8,
            ((c.b() as u16 + mute.b() as u16 * 2) / 3) as u8,
        )
    } else {
        theme::category_icon_color(cat)
    };
    let g = theme::layout_galley(
        ui,
        crate::memo_form::category_icon(cat),
        egui::FontId::proportional(size),
        color,
    );
    let slot = Vec2::splat((size * 1.4).max(18.0));
    let (rect, _) = ui.allocate_exact_size(slot, Sense::hover());
    ui.painter()
        .galley(theme::galley_pos_center(rect.center(), &g), g, color);
}

fn done_button(ui: &mut egui::Ui, svc: &Arc<MemoService>, m: &MemoView, tx: &Sender<BgMsg>) -> bool {
    let (label, fg, fill) = if m.done {
        (
            "已完成",
            theme::success(),
            theme::success_soft(),
        )
    } else {
        (
            "未完成",
            theme::text_muted(),
            theme::panel(),
        )
    };
    let clicked = ui
        .add(
            egui::Button::new(RichText::new(label).size(11.0).color(fg))
                .fill(fill)
                .stroke(Stroke::new(1.0, theme::border()))
                .rounding(Rounding::same(6.0))
                .min_size(Vec2::new(64.0, 22.0)),
        )
        .clicked();
    if clicked {
        toggle_done(svc, m, tx);
    }
    clicked
}

fn apply_row_input(
    _ui: &egui::Ui,
    resp: &egui::Response,
    _consumed: bool,
    m: &MemoView,
    selected: &mut Option<String>,
    editing: &mut bool,
    picked: &mut Option<String>,
    pick_edit: &mut bool,
    pick_delete: &mut bool,
    svc: &Arc<MemoService>,
    tx: &Sender<BgMsg>,
) {
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
}

fn toggle_done(svc: &Arc<MemoService>, m: &MemoView, tx: &Sender<BgMsg>) {
    let svc = svc.clone();
    let id = m.id.clone();
    let next = !m.done;
    let tx = tx.clone();
    std::thread::spawn(move || {
        match svc.set_done(&id, next) {
            Ok(()) => {
                let _ = tx.send(BgMsg::Refresh);
            }
            Err(e) => {
                let _ = tx.send(BgMsg::Error(e.to_string()));
            }
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
    let done_l = if m.done {
        "取消完成"
    } else {
        "标记完成"
    };
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

fn compact_row(
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
    let mut consumed = false;
    let r = Frame::none()
        .inner_margin(Margin::symmetric(4.0, 2.0))
        .rounding(Rounding::same(6.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 26.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    cat_icon(ui, m, 14.0);
                    memo_line(ui, m, 14.0, 13.0, false, false);
                    for tag in m.tags.iter().take(2) {
                        tag_pill(ui, tag);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if done_button(ui, svc, m, tx) {
                            consumed = true;
                        }
                    });
                },
            );
        });
    let resp = r.response.interact(Sense::click());
    apply_row_input(
        ui,
        &resp,
        consumed,
        m,
        selected,
        editing,
        picked,
        pick_edit,
        pick_delete,
        svc,
        tx,
    );
}

fn card_row(
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
    let sel = selected.as_deref() == Some(m.id.as_str());
    let urgent = !m.done && memo_core::due_is_due_today(&m.due_date);
    let mut consumed = false;
    let r = Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(
            if sel { 1.5 } else { 1.0 },
            if sel {
                theme::shell_accent()
            } else if urgent {
                theme::danger()
            } else {
                theme::border()
            },
        ))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 26.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    cat_icon(ui, m, 16.0);
                    memo_line(ui, m, 16.0, 13.5, false, false);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if done_button(ui, svc, m, tx) {
                            consumed = true;
                        }
                        for tag in m.tags.iter().take(3) {
                            tag_pill(ui, tag);
                        }
                    });
                },
            );
        });
    let resp = r.response.interact(Sense::click());
    apply_row_input(
        ui,
        &resp,
        consumed,
        m,
        selected,
        editing,
        picked,
        pick_edit,
        pick_delete,
        svc,
        tx,
    );
    ui.add_space(4.0);
}

fn place_today(svc: &Arc<MemoService>, m: &MemoView, tx: &Sender<BgMsg>) {
    let due = format!("{} 09:00", ymd(today()));
    let svc = svc.clone();
    let id = m.id.clone();
    let tx = tx.clone();
    // 与 set_done 一样：从存储重读再写，只改 due，避免视图层正文异常导致静默失败
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
