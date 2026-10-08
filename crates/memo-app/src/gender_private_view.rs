//! 性别私密专属页：经期 / 体检分段（可各自关闭）+ 选中日底栏。

use crate::male_view;
use crate::period_view;
use crate::theme;
use chrono::{Datelike, NaiveDate};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::cycle::{analyze_period_marks, CycleConfig};
use memo_core::male_health::MaleHealthConfig;
use memo_core::service::{MemoService, MemoView};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HubTab {
    #[default]
    Period,
    Checkup,
}

#[derive(Debug, Clone)]
pub struct GenderPrivateUi {
    pub year: i32,
    pub month: u32,
    pub selected_day: Option<String>,
    pub note_draft: String,
    pub tip: String,
    pub show_advanced: bool,
    pub care_dismissed: bool,
    pub tab: HubTab,
    /// 列表预警：「查看过期任务」时滚到过期分组。
    pub focus_overdue: bool,
}

impl Default for GenderPrivateUi {
    fn default() -> Self {
        let today = chrono::Local::now().date_naive();
        Self {
            year: today.year(),
            month: today.month(),
            selected_day: None,
            note_draft: String::new(),
            tip: String::new(),
            show_advanced: false,
            care_dismissed: false,
            tab: HubTab::Period,
            focus_overdue: false,
        }
    }
}

fn today_ymd() -> String {
    chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string()
}

fn load_note_draft(st: &mut GenderPrivateUi, memos: &[MemoView]) {
    let Some(ymd) = st.selected_day.clone() else {
        st.note_draft.clear();
        return;
    };
    if let Some(m) = period_view::notes_on_day(memos, &ymd).first() {
        st.note_draft = if m.content.is_empty() {
            m.title.clone()
        } else {
            m.content.clone()
        };
    } else {
        st.note_draft.clear();
    }
}

fn legend_chip(ui: &mut egui::Ui, color: Color32, label: &str) {
    legend_item(ui, |ui, row_h| {
        let (slot, _) = ui.allocate_exact_size(Vec2::new(10.0, row_h), Sense::hover());
        let swatch = egui::Rect::from_center_size(slot.center(), Vec2::splat(9.0));
        ui.painter()
            .rect_filled(swatch, Rounding::same(2.5), color);
        ui.label(
            RichText::new(label)
                .size(11.0)
                .color(theme::text_muted()),
        );
    });
}

fn legend_dot(ui: &mut egui::Ui, color: Color32, label: &str) {
    legend_item(ui, |ui, row_h| {
        let (slot, _) = ui.allocate_exact_size(Vec2::new(10.0, row_h), Sense::hover());
        ui.painter()
            .circle_filled(slot.center(), 2.4, color);
        ui.label(
            RichText::new(label)
                .size(11.0)
                .color(theme::text_muted()),
        );
    });
}

fn legend_text(ui: &mut egui::Ui, label: &str) {
    legend_item(ui, |ui, _row_h| {
        ui.label(
            RichText::new(label)
                .size(11.0)
                .color(theme::text_muted()),
        );
    });
}

/// 色块与文字同一行高并垂直居中；项间距统一。
fn legend_item(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui, f32)) {
    let row_h = 16.0;
    ui.horizontal(|ui| {
        ui.set_min_height(row_h);
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            add_contents(ui, row_h);
        });
    });
    ui.add_space(6.0);
}

/// 图例色与格子一致（不用硬编码）。
fn legend_period_color() -> Color32 {
    theme::period_day()
}

fn legend_predict_color() -> Color32 {
    theme::period_pink_bg()
}

fn legend_fertile_color() -> Color32 {
    theme::period_fertile()
}

fn legend_note_color() -> Color32 {
    theme::text_muted()
}

fn legend_checkup_past_color() -> Color32 {
    theme::text_muted()
}

/// 台风预警色：蓝 < 黄 < 橙 < 红（越近越紧急）。
fn legend_checkup_blue() -> Color32 {
    Color32::from_rgb(0x24, 0x92, 0xFF)
}
fn legend_checkup_yellow() -> Color32 {
    Color32::from_rgb(0xE6, 0xB8, 0x00)
}
fn legend_checkup_orange() -> Color32 {
    Color32::from_rgb(0xF0, 0x80, 0x00)
}
fn legend_checkup_red() -> Color32 {
    Color32::from_rgb(0xE6, 0x00, 0x12)
}

#[derive(Clone, Copy)]
enum CheckupWarn {
    Past,
    Blue,
    Yellow,
    Orange,
    Red,
}

fn checkup_days_until(ymd: &str) -> Option<i64> {
    let d = NaiveDate::parse_from_str(ymd, "%Y-%m-%d").ok()?;
    let today = chrono::Local::now().date_naive();
    Some((d - today).num_days())
}

fn checkup_warn(ymd: &str, remind_days: u32) -> CheckupWarn {
    let Some(d) = checkup_days_until(ymd) else {
        return CheckupWarn::Past;
    };
    if d < 0 {
        return CheckupWarn::Past;
    }
    if d == 0 {
        return CheckupWarn::Red;
    }
    let remind = remind_days.max(1) as i64;
    let mid = (remind / 2).max(1);
    if d > remind {
        CheckupWarn::Blue
    } else if d > mid {
        CheckupWarn::Yellow
    } else {
        CheckupWarn::Orange
    }
}

fn checkup_date_color(ymd: &str, remind_days: u32) -> Color32 {
    match checkup_warn(ymd, remind_days) {
        CheckupWarn::Past => legend_checkup_past_color(),
        CheckupWarn::Blue => legend_checkup_blue(),
        CheckupWarn::Yellow => legend_checkup_yellow(),
        CheckupWarn::Orange => legend_checkup_orange(),
        CheckupWarn::Red => legend_checkup_red(),
    }
}

fn checkup_stroke_w(ymd: &str, remind_days: u32) -> f32 {
    match checkup_warn(ymd, remind_days) {
        CheckupWarn::Past => 1.5,
        CheckupWarn::Blue => 1.8,
        CheckupWarn::Yellow => 2.0,
        CheckupWarn::Orange => 2.1,
        CheckupWarn::Red => 2.3,
    }
}

fn checkup_countdown_short(ymd: &str) -> Option<String> {
    match checkup_days_until(ymd)? {
        d if d < 0 => None,
        0 => Some("今".into()),
        d => Some(d.to_string()),
    }
}

fn checkup_proximity_label(ymd: &str, remind_days: u32) -> &'static str {
    match checkup_warn(ymd, remind_days) {
        CheckupWarn::Past => "已过",
        CheckupWarn::Blue => "蓝",
        CheckupWarn::Yellow => "黄",
        CheckupWarn::Orange => "橙",
        CheckupWarn::Red => "红",
    }
}

fn checkup_mmdd(ymd: &str) -> &str {
    if ymd.len() >= 10 {
        &ymd[5..]
    } else {
        ymd
    }
}

fn save_cycle(svc: &Arc<MemoService>, cycle: &CycleConfig) {
    if let Ok(pid) = period_view::ensure_cycle_person(svc) {
        let _ = svc.set_cycle(&pid, cycle.clone());
    }
}

fn save_health(svc: &Arc<MemoService>, health: &MaleHealthConfig) {
    if let Ok(pid) = male_view::ensure_male_person(svc) {
        let _ = svc.set_male_health(&pid, health.clone());
    }
}

/// 绘制日历化性别私密专属页。
pub fn show(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    status_line: &mut String,
) {
    let alias = svc.session_alias();
    let alias_show = if alias.is_empty() { "当前用户" } else { alias };
    let avail_h = ui.available_height().max(200.0);
    let avail_w = ui.available_width();

    let marked = period_view::period_marked_days(memos);
    let stats = analyze_period_marks(&marked);
    let mut cycle = svc.current_cycle().unwrap_or_default();
    let drop_manual = cycle.manual_cycle
        || cycle.manual_period
        || cycle.ovulation_preset != memo_core::OvulationPreset::Standard;
    period_view::apply_effective_cfg(&mut cycle, &stats);
    if drop_manual {
        save_cycle(svc, &cycle);
    }
    let next_period = period_view::next_expected(&cycle);

    let mut health = svc.current_male_health().unwrap_or_default();
    let last_checkup_ymd = health.last_checkup.clone();
    let checkup_ymd = health.next_checkup.clone();
    let days_to_checkup = health.days_until_checkup();

    // 若当前 tab 对应模块已关，自动切到仍开启的一侧
    match (cycle.period_enabled, health.checkup_enabled) {
        (true, false) => st.tab = HubTab::Period,
        (false, true) => st.tab = HubTab::Checkup,
        (false, false) => {}
        (true, true) => {}
    }

    egui::ScrollArea::vertical()
        .id_source("gender_private_scroll")
        .auto_shrink([false, false])
        .max_height(avail_h)
        .show(ui, |ui| {
            ui.set_width(avail_w);
            show_body(
                ui,
                svc,
                st,
                memos,
                status_line,
                &alias_show,
                &marked,
                &mut cycle,
                next_period,
                &mut health,
                &last_checkup_ymd,
                &checkup_ymd,
                days_to_checkup,
            );
        });
}

#[allow(clippy::too_many_arguments)]
fn show_body(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    status_line: &mut String,
    alias_show: &str,
    marked: &[String],
    cycle: &mut CycleConfig,
    next_period: Option<NaiveDate>,
    health: &mut MaleHealthConfig,
    last_checkup_ymd: &str,
    checkup_ymd: &str,
    days_to_checkup: Option<i64>,
) {
    let pe = cycle.period_enabled;
    let ce = health.checkup_enabled;

    // —— 顶栏：标题 + 右上经期/体检分段（对齐效果图）——
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("性别私密")
                .size(16.0)
                .strong()
                .color(theme::text()),
        );
        ui.label(
            RichText::new("🔒")
                .size(11.0)
                .color(theme::text_muted()),
        )
        .on_hover_text(format!("已加密 · {alias_show}"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = if st.show_advanced { "收起设置" } else { "设置" };
            if ui.small_button(label).clicked() {
                st.show_advanced = !st.show_advanced;
            }
            ui.add_space(6.0);
            if pe && ce {
                // 右到左绘制：先体检再经期，视觉上经期在左
                segment_tab(ui, st, HubTab::Checkup, "体检");
                ui.add_space(4.0);
                segment_tab(ui, st, HubTab::Period, "经期");
            } else if pe {
                ui.label(
                    RichText::new("经期")
                        .size(13.0)
                        .strong()
                        .color(theme::shell_accent()),
                );
            } else if ce {
                ui.label(
                    RichText::new("体检")
                        .size(13.0)
                        .strong()
                        .color(theme::shell_accent()),
                );
            }
        });
    });

    // 细关怀条（仅对已开启模块）
    if !st.care_dismissed {
        let period_care = period_view::care_remind(svc, memos);
        let health_care = male_view::care_remind(svc);
        let line = match (
            period_care.as_ref().filter(|c| c.needs_attention()),
            health_care.as_ref().filter(|c| c.needs_attention()),
        ) {
            (Some(p), Some(h)) => Some(format!("♡ {} · {}", p.title, h.title)),
            (Some(p), None) => Some(format!("♡ {}", p.title)),
            (None, Some(h)) => Some(format!("♡ {}", h.title)),
            _ => None,
        };
        if let Some(text) = line {
            ui.add_space(4.0);
            Frame::none()
                .fill(theme::accent_soft())
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(10.0, 5.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(text)
                                .size(12.0)
                                .color(theme::shell_accent()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("×").clicked() {
                                st.care_dismissed = true;
                            }
                        });
                    });
                });
        }
    }

    ui.add_space(4.0);

    if st.show_advanced {
        show_hub_settings(ui, svc, cycle, health, status_line);
        ui.add_space(4.0);
    }

    if !pe && !ce {
        show_empty_enable(ui);
        return;
    }

    match st.tab {
        HubTab::Period if pe => show_period_page(
            ui,
            svc,
            st,
            memos,
            marked,
            cycle,
            next_period,
            status_line,
        ),
        HubTab::Checkup if ce => show_checkup_page(
            ui,
            svc,
            st,
            memos,
            health,
            last_checkup_ymd,
            checkup_ymd,
            days_to_checkup,
            status_line,
        ),
        _ => {
            // 兜底：切到仍开启的一侧
            if pe {
                st.tab = HubTab::Period;
                show_period_page(
                    ui,
                    svc,
                    st,
                    memos,
                    marked,
                    cycle,
                    next_period,
                    status_line,
                );
            } else if ce {
                st.tab = HubTab::Checkup;
                show_checkup_page(
                    ui,
                    svc,
                    st,
                    memos,
                    health,
                    last_checkup_ymd,
                    checkup_ymd,
                    days_to_checkup,
                    status_line,
                );
            }
        }
    }
}

fn segment_tab(ui: &mut egui::Ui, st: &mut GenderPrivateUi, tab: HubTab, label: &str) {
    let selected = st.tab == tab;
    let fill = if selected {
        theme::shell_accent_soft()
    } else {
        theme::panel()
    };
    let stroke = if selected {
        Stroke::new(2.0, theme::shell_accent())
    } else {
        Stroke::new(1.0, theme::border())
    };
    let text_c = if selected {
        theme::shell_accent()
    } else {
        theme::text()
    };
    let btn = ui.add(
        egui::Button::new(RichText::new(label).size(13.5).strong().color(text_c))
            .fill(fill)
            .stroke(stroke)
            .rounding(Rounding::same(8.0))
            .min_size(Vec2::new(72.0, 28.0)),
    );
    if btn.clicked() {
        st.tab = tab;
    }
}

fn show_empty_enable(ui: &mut egui::Ui) {
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("经期记录与体检提醒均已关闭")
                    .size(15.0)
                    .strong()
                    .color(theme::text()),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("可在右上角「设置」中分别或同时重新开启；已有标记与日期不会被删除。")
                    .size(12.5)
                    .color(theme::text_muted()),
            );
        });
}

fn show_hub_settings(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    cycle: &mut CycleConfig,
    health: &mut MaleHealthConfig,
    status_line: &mut String,
) {
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("经期与体检可同时开启，互不影响。")
                    .size(12.0)
                    .color(theme::text_muted()),
            );
            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                if cycle.period_enabled {
                    if theme::ghost_button(ui, "关闭经期记录").clicked() {
                        cycle.period_enabled = false;
                        save_cycle(svc, cycle);
                        *status_line = "已关闭经期记录（标记仍保留）".into();
                    }
                } else if theme::primary_button(ui, "开启经期记录").clicked() {
                    cycle.period_enabled = true;
                    save_cycle(svc, cycle);
                    *status_line = "已开启经期记录".into();
                }
                ui.add_space(8.0);
                if health.checkup_enabled {
                    if theme::ghost_button(ui, "关闭体检提醒").clicked() {
                        health.checkup_enabled = false;
                        save_health(svc, health);
                        *status_line = "已关闭体检提醒（日期仍保留）".into();
                    }
                } else if theme::primary_button(ui, "开启体检提醒").clicked() {
                    health.checkup_enabled = true;
                    save_health(svc, health);
                    *status_line = "已开启体检提醒".into();
                }
            });

            if cycle.period_enabled {
                ui.add_space(6.0);
                if cycle.can_predict() {
                    let mut line = format!(
                        "周期与排卵均由手标自动推算（周期约 {} 天，经期约 {} 天）",
                        cycle.cycle_days, cycle.period_days
                    );
                    if let Some(ov) = cycle.ovulation_offset() {
                        line.push_str(&format!("；排卵约在开始后第 {} 天", ov + 1));
                    }
                    ui.label(
                        RichText::new(line)
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                } else {
                    ui.label(
                        RichText::new("再完整标记至少一次经期后，即可自动推算下次经期与易孕窗。")
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                }
            }

            if health.checkup_enabled {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    let mut shown = false;
                    for y in [&health.last_checkup, &health.next_checkup] {
                        if y.is_empty() {
                            continue;
                        }
                        shown = true;
                        let rd = health.remind_days.max(1);
                        ui.label(
                            RichText::new(checkup_proximity_label(y, rd))
                                .size(12.0)
                                .color(theme::text_muted()),
                        );
                        let mut date_txt = y.to_string();
                        if let Some(cd) = checkup_countdown_short(y) {
                            if cd != "今" {
                                date_txt.push_str(&format!(" 还有{cd}天"));
                            } else {
                                date_txt.push_str(" 今天");
                            }
                        }
                        ui.label(
                            RichText::new(date_txt)
                                .size(12.0)
                                .color(checkup_date_color(y, rd)),
                        );
                        ui.add_space(10.0);
                    }
                    if !shown {
                        ui.label(
                            RichText::new("尚未标记体检日")
                                .size(12.0)
                                .color(theme::text_muted()),
                        );
                        ui.add_space(10.0);
                    }
                    ui.label(
                        RichText::new("提前提醒")
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                    let mut d = health.remind_days as i32;
                    if ui
                        .add(egui::DragValue::new(&mut d).clamp_range(1..=90).suffix(" 天"))
                        .changed()
                    {
                        health.remind_days = d as u32;
                        save_health(svc, health);
                    }
                });
            }

            ui.add_space(4.0);
            ui.label(
                RichText::new("推算与提醒仅供参考，不能替代就医。关闭后数据仍保留，可随时再开启。")
                    .size(11.0)
                    .color(theme::text_muted()),
            );
        });
}

#[allow(clippy::too_many_arguments)]
fn show_period_page(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    marked: &[String],
    cycle: &mut CycleConfig,
    next_period: Option<NaiveDate>,
    status_line: &mut String,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        legend_chip(ui, legend_period_color(), "经期");
        legend_chip(ui, legend_predict_color(), "预测");
        if cycle.ovulation_offset().is_some() {
            legend_chip(ui, legend_fertile_color(), "易孕");
            legend_text(ui, "排");
        }
        legend_dot(ui, legend_note_color(), "点=有备注");
        if let Some(n) = next_period {
            ui.label(
                RichText::new(format!("预计 {}", n.format("%m-%d")))
                    .size(11.0)
                    .color(theme::text_muted()),
            );
        }
    });
    ui.add_space(4.0);

    let bottom_reserve = 140.0;
    let cal_h = (ui.available_height() - bottom_reserve).clamp(160.0, 480.0);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), cal_h),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            show_calendar_frame(ui, st, memos, |ui, st| {
                draw_month_grid(
                    ui,
                    st,
                    memos,
                    marked,
                    "",
                    "",
                    svc,
                    cycle,
                    status_line,
                    HubTab::Period,
                    0,
                );
            });
        },
    );

    ui.add_space(8.0);
    show_period_bottom(ui, svc, st, memos, cycle, status_line);
}

#[allow(clippy::too_many_arguments)]
fn show_checkup_page(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    health: &mut MaleHealthConfig,
    last_checkup_ymd: &str,
    checkup_ymd: &str,
    days_to_checkup: Option<i64>,
    status_line: &mut String,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        legend_dot(ui, legend_checkup_past_color(), "已过");
        legend_dot(ui, legend_checkup_blue(), "蓝");
        legend_dot(ui, legend_checkup_yellow(), "黄");
        legend_dot(ui, legend_checkup_orange(), "橙");
        legend_dot(ui, legend_checkup_red(), "红");
        legend_text(ui, "检");
        legend_dot(ui, legend_note_color(), "点=有备注");
        let rd = health.remind_days.max(1);
        let mut seen = String::new();
        for y in [health.last_checkup.as_str(), checkup_ymd] {
            if y.is_empty() || y == seen {
                continue;
            }
            seen = y.to_string();
            legend_dot(ui, checkup_date_color(y, rd), checkup_mmdd(y));
        }
        if let Some(d) = days_to_checkup.filter(|_| !checkup_ymd.is_empty()) {
            let extra = if d > 0 {
                format!("还有 {d} 天")
            } else if d == 0 {
                "今天".into()
            } else {
                format!("已过 {} 天", -d)
            };
            ui.label(
                RichText::new(extra)
                    .size(11.0)
                    .color(checkup_date_color(checkup_ymd, rd)),
            );
        }
    });
    ui.add_space(4.0);

    let bottom_reserve = 140.0;
    let cal_h = (ui.available_height() - bottom_reserve).clamp(160.0, 480.0);
    let mut dummy_cycle = CycleConfig::default();
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), cal_h),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            show_calendar_frame(ui, st, memos, |ui, st| {
                draw_month_grid(
                    ui,
                    st,
                    memos,
                    &[],
                    last_checkup_ymd,
                    checkup_ymd,
                    svc,
                    &mut dummy_cycle,
                    status_line,
                    HubTab::Checkup,
                    health.remind_days.max(1),
                );
            });
        },
    );

    ui.add_space(8.0);
    show_checkup_bottom(ui, svc, st, memos, health, status_line);
}

fn show_calendar_frame(
    ui: &mut egui::Ui,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    draw: impl FnOnce(&mut egui::Ui, &mut GenderPrivateUi),
) {
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("‹").clicked() {
                    if st.month <= 1 {
                        st.month = 12;
                        st.year -= 1;
                    } else {
                        st.month -= 1;
                    }
                }
                ui.label(
                    RichText::new(format!("{:04} 年 {:02} 月", st.year, st.month))
                        .strong()
                        .size(14.0)
                        .color(theme::text()),
                );
                if ui.small_button("›").clicked() {
                    if st.month >= 12 {
                        st.month = 1;
                        st.year += 1;
                    } else {
                        st.month += 1;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("今天").clicked() {
                        let t = chrono::Local::now().date_naive();
                        st.year = t.year();
                        st.month = t.month();
                        st.selected_day = Some(today_ymd());
                        load_note_draft(st, memos);
                    }
                });
            });
            ui.add_space(6.0);
            draw(ui, st);
        });
}

#[allow(clippy::too_many_arguments)]
fn draw_month_grid(
    ui: &mut egui::Ui,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    marked: &[String],
    last_checkup_ymd: &str,
    checkup_ymd: &str,
    svc: &Arc<MemoService>,
    cycle: &mut CycleConfig,
    status_line: &mut String,
    mode: HubTab,
    remind_days: u32,
) {
    let Some(first) = NaiveDate::from_ymd_opt(st.year, st.month, 1) else {
        return;
    };
    let start_pad = first.weekday().num_days_from_monday() as i64;
    let grid_start = first - chrono::Duration::try_days(start_pad).unwrap_or_default();
    let cells: Vec<(NaiveDate, bool)> = (0..42)
        .map(|i| {
            let d = grid_start + chrono::Duration::try_days(i).unwrap_or_default();
            let in_month = d.year() == st.year && d.month() == st.month;
            (d, in_month)
        })
        .collect();
    let n_rows = (0..6)
        .rev()
        .find(|r| (0..7).any(|c| cells[r * 7 + c].1))
        .map(|r| r + 1)
        .unwrap_or(5)
        .max(4);

    let grid_w = ui.available_width().max(200.0);
    let cell_w = ((grid_w - 6.0) / 7.0).clamp(28.0, 56.0);
    let remain_h = (ui.available_height() - 22.0).max(120.0);
    let cell_h = (remain_h / n_rows as f32).clamp(28.0, cell_w.clamp(32.0, 48.0));
    let cell = Vec2::new(cell_w, cell_h);

    ui.horizontal(|ui| {
        for w in ["一", "二", "三", "四", "五", "六", "日"] {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(cell.x, 18.0), Sense::hover());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                w,
                egui::FontId::proportional(11.0),
                theme::text_muted(),
            );
        }
    });

    for row in 0..n_rows {
        ui.horizontal(|ui| {
            for col in 0..7 {
                let (date, in_month) = cells[row * 7 + col];
                let ymd = date.format("%Y-%m-%d").to_string();
                let day = date.day();
                let is_today = ymd == today_ymd();
                let is_sel = st.selected_day.as_deref() == Some(ymd.as_str());
                let has_note = !period_view::notes_on_day(memos, &ymd).is_empty();

                let (fill, stroke, fg_mark, badge, countdown) = match mode {
                    HubTab::Period => {
                        let is_marked = marked.iter().any(|d| d == &ymd);
                        let is_predicted = !is_marked && cycle.is_predicted_period_day(&ymd);
                        let is_ovulation =
                            !is_marked && !is_predicted && cycle.is_ovulation_day(&ymd);
                        let is_fertile = !is_marked
                            && !is_predicted
                            && !is_ovulation
                            && cycle.is_fertile_day(&ymd);
                        // 手标实填；预测浅粉底+粉描边；易孕/排卵不实填，仅高对比描边
                        let fill = if is_marked {
                            theme::period_day()
                        } else if is_predicted {
                            theme::period_pink_bg()
                        } else if is_sel {
                            theme::shell_accent_soft()
                        } else {
                            theme::panel()
                        };
                        let stroke = if is_sel {
                            Stroke::new(2.0, theme::shell_accent())
                        } else if is_today {
                            Stroke::new(1.5, theme::shell_accent())
                        } else if is_ovulation {
                            Stroke::new(2.2, theme::period_fertile())
                        } else if is_fertile {
                            Stroke::new(1.6, theme::period_fertile())
                        } else if is_predicted {
                            Stroke::new(1.4, theme::period_day())
                        } else if is_marked {
                            Stroke::new(1.0, theme::period_day())
                        } else {
                            Stroke::new(1.0, theme::border())
                        };
                        let badge = if is_ovulation {
                            Some(("排", theme::period_fertile()))
                        } else {
                            None
                        };
                        (fill, stroke, is_marked, badge, None)
                    }
                    HubTab::Checkup => {
                        let is_check = (!last_checkup_ymd.is_empty() && last_checkup_ymd == ymd)
                            || (!checkup_ymd.is_empty() && checkup_ymd == ymd);
                        let check_col = checkup_date_color(&ymd, remind_days);
                        let fill = if is_sel {
                            theme::shell_accent_soft()
                        } else {
                            theme::panel()
                        };
                        let stroke = if is_sel {
                            Stroke::new(2.0, theme::shell_accent())
                        } else if is_check {
                            let w = checkup_stroke_w(&ymd, remind_days);
                            Stroke::new(w, check_col)
                        } else if is_today {
                            Stroke::new(1.5, theme::shell_accent())
                        } else {
                            Stroke::new(1.0, theme::border())
                        };
                        let badge = if is_check {
                            Some(("检", check_col))
                        } else {
                            None
                        };
                        let cd = if is_check {
                            checkup_countdown_short(&ymd)
                        } else {
                            None
                        };
                        (fill, stroke, false, badge, cd)
                    }
                };

                let (rect, resp) = ui.allocate_exact_size(cell, Sense::click());
                ui.painter()
                    .rect(rect.shrink(1.0), Rounding::same(6.0), fill, stroke);

                let fg = if fg_mark {
                    Color32::WHITE
                } else if in_month {
                    theme::text()
                } else {
                    theme::text_muted()
                };
                let lift = has_note || badge.is_some() || countdown.is_some();
                ui.painter().text(
                    egui::pos2(
                        rect.center().x,
                        rect.center().y - if lift { 3.0 } else { 0.0 },
                    ),
                    egui::Align2::CENTER_CENTER,
                    day.to_string(),
                    egui::FontId::proportional(13.0),
                    fg,
                );
                let badge_col = badge.map(|(_, c)| c);
                if let Some((txt, col)) = badge {
                    ui.painter().text(
                        egui::pos2(rect.right() - 3.0, rect.top() + 2.0),
                        egui::Align2::RIGHT_TOP,
                        txt,
                        egui::FontId::proportional(12.0),
                        col,
                    );
                }
                if let Some(cd) = countdown {
                    ui.painter().text(
                        egui::pos2(rect.left() + 4.0, rect.top() + 2.0),
                        egui::Align2::LEFT_TOP,
                        cd,
                        egui::FontId::proportional(11.0),
                        badge_col.unwrap_or_else(theme::text_muted),
                    );
                }
                if has_note {
                    let dot_c = if fg_mark {
                        Color32::WHITE
                    } else {
                        theme::shell_accent()
                    };
                    ui.painter().circle_filled(
                        egui::pos2(rect.center().x, rect.bottom() - 6.0),
                        2.2,
                        dot_c,
                    );
                }

                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    st.selected_day = Some(ymd.clone());
                    load_note_draft(st, memos);
                }
                if mode == HubTab::Period && resp.secondary_clicked() {
                    st.selected_day = Some(ymd.clone());
                    period_view::toggle_period_day(
                        svc,
                        &ymd,
                        memos,
                        cycle,
                        status_line,
                        &mut st.tip,
                    );
                    load_note_draft(st, memos);
                }
            }
        });
    }
}

fn show_period_bottom(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    cycle: &mut CycleConfig,
    status_line: &mut String,
) {
    let ymd = st.selected_day.clone();
    let marked = period_view::period_marked_days(memos);
    let is_marked = ymd
        .as_ref()
        .map(|d| marked.iter().any(|x| x == d))
        .unwrap_or(false);

    Frame::none()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let title = ymd
                    .as_deref()
                    .map(|d| {
                        if d.len() >= 10 {
                            format!("{} 日", &d[5..])
                        } else {
                            d.to_string()
                        }
                    })
                    .unwrap_or_else(|| "未选择日期".into());
                ui.label(
                    RichText::new(title)
                        .strong()
                        .size(13.5)
                        .color(theme::text()),
                );
                ui.label(
                    RichText::new("左键选日写备注 · 右键标经期（仅今天及以前）")
                        .size(11.0)
                        .color(theme::text_muted()),
                );
            });

            if let Some(ymd) = ymd.clone() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let is_future = NaiveDate::parse_from_str(&ymd, "%Y-%m-%d")
                        .ok()
                        .map(|d| d > chrono::Local::now().date_naive())
                        .unwrap_or(true);
                    if is_marked {
                        if theme::ghost_button(ui, "取消经期").clicked() {
                            period_view::toggle_period_day(
                                svc,
                                &ymd,
                                memos,
                                cycle,
                                status_line,
                                &mut st.tip,
                            );
                        }
                    } else {
                        ui.add_enabled_ui(!is_future, |ui| {
                            let r = theme::ghost_button(ui, "标为经期").on_disabled_hover_text(
                                "只能标记今天及之前的经期日，未来日请看预测",
                            );
                            if r.clicked() {
                                period_view::toggle_period_day(
                                    svc,
                                    &ymd,
                                    memos,
                                    cycle,
                                    status_line,
                                    &mut st.tip,
                                );
                            }
                        });
                    }
                });

                show_note_editor(ui, svc, st, &ymd, status_line);
            } else {
                ui.add_space(2.0);
                ui.label(
                    RichText::new("在日历上点选日期")
                        .size(12.0)
                        .color(theme::text_muted()),
                );
            }

            if !st.tip.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(&st.tip).small().color(theme::warn()));
            }
        });
}

fn show_checkup_bottom(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    health: &mut MaleHealthConfig,
    status_line: &mut String,
) {
    let ymd = st.selected_day.clone();
    let _ = memos;

    Frame::none()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let title = ymd
                    .as_deref()
                    .map(|d| {
                        if d.len() >= 10 {
                            format!("{} 日", &d[5..])
                        } else {
                            d.to_string()
                        }
                    })
                    .unwrap_or_else(|| "未选择日期".into());
                ui.label(
                    RichText::new(title)
                        .strong()
                        .size(13.5)
                        .color(theme::text()),
                );
                ui.label(
                    RichText::new("左键选日 · 标为体检（已过/未到自动区分）")
                        .size(11.0)
                        .color(theme::text_muted()),
                );
            });

            if let Some(ymd) = ymd.clone() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let is_marked = health.last_checkup == ymd || health.next_checkup == ymd;
                    if is_marked {
                        if theme::ghost_button(ui, "取消体检").clicked() {
                            match male_view::ensure_male_person(svc) {
                                Ok(pid) => {
                                    if health.last_checkup == ymd {
                                        health.last_checkup.clear();
                                    }
                                    if health.next_checkup == ymd {
                                        health.next_checkup.clear();
                                        health.remind_seen_for.clear();
                                    }
                                    match svc.set_male_health(&pid, health.clone()) {
                                        Ok(()) => {
                                            *status_line = format!("已取消体检 {ymd}");
                                            st.tip.clear();
                                        }
                                        Err(e) => *status_line = format!("保存失败: {e}"),
                                    }
                                }
                                Err(e) => *status_line = e,
                            }
                        }
                    } else if theme::ghost_button(ui, "标为体检").clicked() {
                        match male_view::ensure_male_person(svc) {
                            Ok(pid) => {
                                if ymd.as_str() < today_ymd().as_str() {
                                    health.last_checkup = ymd.clone();
                                } else {
                                    health.next_checkup = ymd.clone();
                                    health.remind_seen_for.clear();
                                }
                                match svc.set_male_health(&pid, health.clone()) {
                                    Ok(()) => {
                                        *status_line = format!("已标体检 {ymd}");
                                        st.tip.clear();
                                    }
                                    Err(e) => *status_line = format!("保存失败: {e}"),
                                }
                            }
                            Err(e) => *status_line = e,
                        }
                    }
                });

                show_note_editor(ui, svc, st, &ymd, status_line);
            } else {
                ui.add_space(2.0);
                ui.label(
                    RichText::new("在日历上点选日期")
                        .size(12.0)
                        .color(theme::text_muted()),
                );
            }

            if !st.tip.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(&st.tip).small().color(theme::warn()));
            }
        });
}

fn show_note_editor(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    st: &mut GenderPrivateUi,
    ymd: &str,
    status_line: &mut String,
) {
    ui.add_space(4.0);
    ui.add(
        egui::TextEdit::multiline(&mut st.note_draft)
            .desired_width(ui.available_width())
            .desired_rows(2)
            .hint_text(theme::hint("当天备注 / 健康日记…")),
    );
    ui.add_space(4.0);
    if theme::primary_button(ui, "保存备注").clicked() {
        period_view::save_day_note(svc, ymd, &st.note_draft, status_line);
    }
}
