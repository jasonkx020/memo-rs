//! 日期 / 日期时间选择：弹层选年月日，可选时分，写入 `YYYY-MM-DD` 或 `YYYY-MM-DD HH:MM`。

use crate::theme;
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};

pub fn parse_ymd(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M")
        .ok()
        .map(|dt| dt.date())
}

pub fn parse_due(s: &str) -> Option<NaiveDateTime> {
    memo_core::parse_due_local(s)
}

pub fn format_ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub fn format_due(dt: NaiveDateTime) -> String {
    memo_core::format_due(dt)
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn default_time() -> NaiveTime {
    NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_else(|| NaiveTime::from_hms_opt(0, 0, 0).unwrap())
}

/// 日期字段：按钮打开日历弹层；`allow_clear` 为真时可清空。返回是否变更。
pub fn show(ui: &mut egui::Ui, id_salt: &str, ymd: &mut String, allow_clear: bool) -> bool {
    show_inner(ui, id_salt, ymd, allow_clear, false)
}

/// 到期时间：年月日 + 时分，读写 `YYYY-MM-DD HH:MM`；可清空。
pub fn show_datetime(
    ui: &mut egui::Ui,
    id_salt: &str,
    due: &mut String,
    allow_clear: bool,
) -> bool {
    show_inner(ui, id_salt, due, allow_clear, true)
}

/// 紧凑横向：标签旁的日期选择（用于侧栏表单）。
pub fn show_compact(
    ui: &mut egui::Ui,
    id_salt: &str,
    ymd: &mut String,
    allow_clear: bool,
) -> bool {
    show(ui, id_salt, ymd, allow_clear)
}

fn show_inner(
    ui: &mut egui::Ui,
    id_salt: &str,
    value: &mut String,
    allow_clear: bool,
    with_time: bool,
) -> bool {
    let mut changed = false;
    let popup_id = ui.make_persistent_id(egui::Id::new(("date_field_popup", id_salt)));

    let display = if value.trim().is_empty() {
        if with_time {
            "选择到期时间…".to_string()
        } else {
            "选择日期…".to_string()
        }
    } else if with_time {
        if parse_due(value).is_some() {
            value.trim().to_string()
        } else {
            format!("{} ⚠", value.trim())
        }
    } else if parse_ymd(value).is_some() {
        value.trim().chars().take(10).collect()
    } else {
        format!("{} ⚠", value.trim())
    };

    ui.horizontal(|ui| {
        let btn = ui.add(
            egui::Button::new(
                RichText::new(format!("📅  {display}"))
                    .size(13.5)
                    .color(theme::text()),
            )
            .fill(theme::card())
            .stroke(Stroke::new(1.0, theme::border()))
            .rounding(Rounding::same(8.0))
            .min_size(Vec2::new((ui.available_width() - 52.0).max(120.0), 30.0)),
        );
        if btn.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }

        if allow_clear && !value.trim().is_empty() {
            if ui
                .add(
                    egui::Button::new(RichText::new("清除").size(12.0).color(theme::text_muted()))
                        .frame(false),
                )
                .on_hover_text(if with_time {
                    "清空=永久有效"
                } else {
                    "清空日期"
                })
                .clicked()
            {
                value.clear();
                changed = true;
                ui.memory_mut(|m| m.close_popup());
            }
        }

        egui::popup_below_widget(ui, popup_id, &btn, |ui| {
            ui.set_min_width(if with_time { 280.0 } else { 240.0 });
            if show_picker_body(ui, id_salt, value, with_time) {
                changed = true;
                if !with_time {
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        });
    });

    changed
}

fn show_picker_body(
    ui: &mut egui::Ui,
    id_salt: &str,
    value: &mut String,
    with_time: bool,
) -> bool {
    let parsed = parse_due(value);
    let mut pick = parsed.map(|dt| dt.date()).unwrap_or_else(today);
    let mut hour = parsed.map(|dt| dt.hour()).unwrap_or(9);
    let mut minute = parsed.map(|dt| dt.minute()).unwrap_or(0);

    let state_id = ui.make_persistent_id(egui::Id::new(("date_field_nav", id_salt)));
    let mut view = ui.ctx().data_mut(|d| {
        d.get_temp::<(i32, u32)>(state_id)
            .unwrap_or((pick.year(), pick.month()))
    });

    let mut picked = false;

    ui.horizontal(|ui| {
        if ui.small_button("◀").clicked() {
            if view.1 <= 1 {
                view.0 -= 1;
                view.1 = 12;
            } else {
                view.1 -= 1;
            }
        }
        ui.label(
            RichText::new(format!("{:04} 年 {:02} 月", view.0, view.1))
                .strong()
                .color(theme::text()),
        );
        if ui.small_button("▶").clicked() {
            if view.1 >= 12 {
                view.0 += 1;
                view.1 = 1;
            } else {
                view.1 += 1;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("今天").clicked() {
                pick = today();
                view = (pick.year(), pick.month());
                if with_time {
                    let t = NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or_else(default_time);
                    *value = format_due(pick.and_time(t));
                } else {
                    *value = format_ymd(pick);
                }
                picked = true;
            }
        });
    });
    ui.add_space(6.0);

    Frame::none()
        .fill(theme::panel())
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::same(6.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for w in ["一", "二", "三", "四", "五", "六", "日"] {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                    ui.painter().text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        w,
                        egui::FontId::proportional(11.0),
                        theme::text_muted(),
                    );
                }
            });
            ui.add_space(2.0);

            let Some(first) = NaiveDate::from_ymd_opt(view.0, view.1, 1) else {
                return;
            };
            let next = if view.1 == 12 {
                NaiveDate::from_ymd_opt(view.0 + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(view.0, view.1 + 1, 1)
            };
            let Some(next) = next else {
                return;
            };
            let days = (next - first).num_days() as u32;
            let offset = first.weekday().number_from_monday() as usize - 1;
            let today_d = today();
            let sel = parse_ymd(value);

            let mut day_i = 0u32;
            for row in 0..6 {
                if day_i >= days && row > 0 {
                    break;
                }
                ui.horizontal(|ui| {
                    for col in 0..7 {
                        let cell = row * 7 + col;
                        if cell < offset || day_i >= days {
                            ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                            continue;
                        }
                        day_i += 1;
                        let day = day_i;
                        let Some(date) = NaiveDate::from_ymd_opt(view.0, view.1, day) else {
                            continue;
                        };
                        let is_today = date == today_d;
                        let is_sel = sel == Some(date);
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                        let fill = if is_sel {
                            theme::shell_accent()
                        } else if resp.hovered() {
                            theme::c().list_hover
                        } else if is_today {
                            theme::shell_accent_soft()
                        } else {
                            Color32::TRANSPARENT
                        };
                        ui.painter()
                            .rect_filled(rect, Rounding::same(6.0), fill);
                        let fg = if is_sel {
                            Color32::WHITE
                        } else if is_today {
                            theme::shell_accent()
                        } else {
                            theme::text()
                        };
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            day.to_string(),
                            egui::FontId::proportional(13.0),
                            fg,
                        );
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            pick = date;
                            if with_time {
                                let t =
                                    NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or_else(default_time);
                                *value = format_due(pick.and_time(t));
                            } else {
                                *value = format_ymd(date);
                            }
                            picked = true;
                        }
                    }
                });
            }
        });

    if with_time {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("时间").size(12.5).color(theme::text_muted()));
            let mut h = hour as i32;
            let mut m = minute as i32;
            let h_changed = ui
                .add(egui::DragValue::new(&mut h).clamp_range(0..=23).suffix(" 时"))
                .changed();
            let m_changed = ui
                .add(egui::DragValue::new(&mut m).clamp_range(0..=59).suffix(" 分"))
                .changed();
            if h_changed || m_changed || (picked && !value.trim().is_empty()) {
                hour = h.clamp(0, 23) as u32;
                minute = m.clamp(0, 59) as u32;
                let date = parse_ymd(value).unwrap_or(pick);
                let t = NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or_else(default_time);
                *value = format_due(date.and_time(t));
                picked = true;
            }
        });
        ui.label(
            RichText::new("不填=永久有效")
                .size(11.0)
                .color(theme::text_muted()),
        );
    }

    ui.ctx().data_mut(|d| d.insert_temp(state_id, view));
    picked
}
