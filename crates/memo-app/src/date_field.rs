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

/// 选日顶栏可编辑年份范围（生日/到期均够用）。
const YEAR_MIN: i32 = 1900;
const YEAR_MAX: i32 = 2100;

#[derive(Clone, Debug, Default)]
enum YmEdit {
    #[default]
    None,
    Year {
        draft: String,
        focus: bool,
    },
    Month {
        draft: String,
        focus: bool,
    },
}

fn parse_year_input(s: &str) -> Option<i32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let y: i32 = t.parse().ok()?;
    if (YEAR_MIN..=YEAR_MAX).contains(&y) {
        Some(y)
    } else {
        None
    }
}

fn parse_month_input(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let m: u32 = t.parse().ok()?;
    if (1..=12).contains(&m) {
        Some(m)
    } else {
        None
    }
}

const NAV_H: f32 = 28.0;
const NAV_BTN_W: f32 = 28.0;
const NAV_FONT: f32 = 13.0;
/// 中文/数字与框线的光学中线微调（向下为正），顶栏共用同一值。
const NAV_OPTICAL_Y: f32 = 0.5;

fn nav_font() -> egui::FontId {
    egui::FontId::proportional(NAV_FONT)
}

fn nav_mid_y(rect: egui::Rect) -> f32 {
    rect.center().y + NAV_OPTICAL_Y
}

fn paint_nav_chip(ui: &mut egui::Ui, rect: egui::Rect, hovered: bool) {
    let fill = if hovered {
        theme::accent_soft()
    } else {
        theme::card()
    };
    let stroke = if hovered {
        Stroke::new(1.0, theme::accent())
    } else {
        Stroke::new(1.0, theme::border())
    };
    ui.painter()
        .rect(rect, Rounding::same(7.0), fill, stroke);
}

/// 顶栏导航钮：固定方块；文字与年月共用中线锚点。
fn nav_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let size = Vec2::new(NAV_BTN_W, NAV_H);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    paint_nav_chip(ui, rect, hovered);
    let fg = if hovered {
        theme::accent()
    } else {
        theme::text()
    };
    ui.painter().text(
        egui::pos2(rect.center().x, nav_mid_y(rect)),
        egui::Align2::CENTER_CENTER,
        label,
        nav_font(),
        fg,
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn today_chip(ui: &mut egui::Ui) -> egui::Response {
    let label = "今天";
    let w = ui.fonts(|f| {
        f.layout_no_wrap(label.to_string(), nav_font(), theme::text())
            .size()
            .x
    }) + 20.0;
    let size = Vec2::new(w.max(44.0), NAV_H);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    paint_nav_chip(ui, rect, hovered);
    ui.painter().text(
        egui::pos2(rect.center().x, nav_mid_y(rect)),
        egui::Align2::CENTER_CENTER,
        label,
        nav_font(),
        theme::text(),
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn ym_label(ui: &mut egui::Ui, text: &str, editable: bool) -> egui::Response {
    let text_w = ui.fonts(|f| {
        f.layout_no_wrap(text.to_string(), nav_font(), theme::text())
            .size()
            .x
    });
    let sense = if editable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(text_w.max(8.0), NAV_H), sense);
    ui.painter().text(
        egui::pos2(rect.min.x, nav_mid_y(rect)),
        egui::Align2::LEFT_CENTER,
        text,
        nav_font(),
        theme::text(),
    );
    if editable && resp.hovered() {
        ui.painter().rect_stroke(
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x - 2.0, rect.center().y - 9.0),
                egui::pos2(rect.max.x + 2.0, rect.center().y + 9.0),
            ),
            Rounding::same(4.0),
            Stroke::new(1.0, theme::border()),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
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
            format!("{} (无效)", value.trim())
        }
    } else if parse_ymd(value).is_some() {
        value.trim().chars().take(10).collect()
    } else {
        format!("{} (无效)", value.trim())
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
            .min_size(Vec2::new(
                (ui.available_width()
                    - if allow_clear && !value.trim().is_empty() {
                        52.0
                    } else {
                        0.0
                    })
                .max(40.0),
                30.0,
            )),
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

/// 弹层内日历本体；选日或改时分时返回 `true`（不负责关闭弹层）。
pub fn show_picker_body(
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
    view.0 = view.0.clamp(YEAR_MIN, YEAR_MAX);
    view.1 = view.1.clamp(1, 12);

    let edit_id = ui.make_persistent_id(egui::Id::new(("date_field_ym_edit", id_salt)));
    let mut edit = ui
        .ctx()
        .data_mut(|d| d.get_temp::<YmEdit>(edit_id).unwrap_or(YmEdit::None));

    let mut picked = false;

    // 顶栏：固定行高 + 垂直居中，导航钮与年月同一基线
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), NAV_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_height(NAV_H);
            ui.spacing_mut().item_spacing.x = 4.0;

            if nav_btn(ui, "‹‹").clicked() {
                view.0 = (view.0 - 1).clamp(YEAR_MIN, YEAR_MAX);
                edit = YmEdit::None;
            }
            if nav_btn(ui, "‹").clicked() {
                if view.1 <= 1 {
                    view.0 = (view.0 - 1).clamp(YEAR_MIN, YEAR_MAX);
                    view.1 = 12;
                } else {
                    view.1 -= 1;
                }
                edit = YmEdit::None;
            }

            ui.add_space(2.0);
            match &mut edit {
                YmEdit::Year { draft, focus } => {
                    let te = ui.add(
                        egui::TextEdit::singleline(draft)
                            .desired_width(48.0)
                            .margin(Vec2::new(4.0, 4.0))
                            .font(egui::FontId::proportional(13.5))
                            .id(egui::Id::new(("date_ym_year", id_salt))),
                    );
                    let _ = ym_label(ui, "年", false);
                    ui.add_space(2.0);
                    let _ = ym_label(ui, &format!("{:02}", view.1), false);
                    let _ = ym_label(ui, "月", false);
                    if *focus {
                        te.request_focus();
                        *focus = false;
                    } else {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if esc {
                            edit = YmEdit::None;
                        } else if enter || te.lost_focus() {
                            if let Some(y) = parse_year_input(draft) {
                                view.0 = y;
                            }
                            edit = YmEdit::None;
                        }
                    }
                }
                YmEdit::Month { draft, focus } => {
                    let _ = ym_label(ui, &format!("{:04}", view.0), false);
                    let _ = ym_label(ui, "年", false);
                    ui.add_space(2.0);
                    let te = ui.add(
                        egui::TextEdit::singleline(draft)
                            .desired_width(28.0)
                            .margin(Vec2::new(4.0, 4.0))
                            .font(egui::FontId::proportional(13.5))
                            .id(egui::Id::new(("date_ym_month", id_salt))),
                    );
                    let _ = ym_label(ui, "月", false);
                    if *focus {
                        te.request_focus();
                        *focus = false;
                    } else {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if esc {
                            edit = YmEdit::None;
                        } else if enter || te.lost_focus() {
                            if let Some(m) = parse_month_input(draft) {
                                view.1 = m;
                            }
                            edit = YmEdit::None;
                        }
                    }
                }
                YmEdit::None => {
                    let y_resp = ym_label(ui, &format!("{:04}", view.0), true);
                    if y_resp.double_clicked() {
                        edit = YmEdit::Year {
                            draft: format!("{:04}", view.0),
                            focus: true,
                        };
                    }
                    y_resp.on_hover_text("双击输入年份");
                    let _ = ym_label(ui, "年", false);
                    ui.add_space(2.0);
                    let m_resp = ym_label(ui, &format!("{:02}", view.1), true);
                    if m_resp.double_clicked() {
                        edit = YmEdit::Month {
                            draft: format!("{:02}", view.1),
                            focus: true,
                        };
                    }
                    m_resp.on_hover_text("双击输入月份");
                    let _ = ym_label(ui, "月", false);
                }
            }
            ui.add_space(2.0);

            if nav_btn(ui, "›").clicked() {
                if view.1 >= 12 {
                    view.0 = (view.0 + 1).clamp(YEAR_MIN, YEAR_MAX);
                    view.1 = 1;
                } else {
                    view.1 += 1;
                }
                edit = YmEdit::None;
            }
            if nav_btn(ui, "››").clicked() {
                view.0 = (view.0 + 1).clamp(YEAR_MIN, YEAR_MAX);
                edit = YmEdit::None;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.set_height(NAV_H);
                if today_chip(ui).clicked() {
                    pick = today();
                    view = (pick.year(), pick.month());
                    edit = YmEdit::None;
                    if with_time {
                        let t =
                            NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or_else(default_time);
                        *value = format_due(pick.and_time(t));
                    } else {
                        *value = format_ymd(pick);
                    }
                    picked = true;
                }
            });
        },
    );
    ui.ctx().data_mut(|d| d.insert_temp(edit_id, edit));
    ui.add_space(8.0);

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
