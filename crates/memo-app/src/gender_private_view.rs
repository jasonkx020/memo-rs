//! 性别私密专属页：一张关怀日历叠画经期 / 体检图层（可各自关闭）+ 选中日分组底栏。

use crate::male_view;
use crate::period_view;
use crate::theme;
use chrono::{Datelike, NaiveDate};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::cycle::{analyze_period_marks, CycleConfig};
use memo_core::male_health::MaleHealthConfig;
use memo_core::service::{MemoService, MemoView};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct GenderPrivateUi {
    pub year: i32,
    pub month: u32,
    pub selected_day: Option<String>,
    pub note_draft: String,
    /// 备注相对已加载内容是否有未保存改动。
    pub note_dirty: bool,
    pub tip: String,
    pub care_dismissed: bool,
    /// 列表预警：「查看过期任务」时滚到过期分组。
    pub focus_overdue: bool,
    /// 引导态：等待用户点选经期开始日。
    pub mark_picking_start: bool,
    /// 待闭合的经期开始日（再点结束日自动标整段）。
    pub period_range_start: Option<String>,
    /// 请求打开应用设置（由 shell 消费）。
    pub request_open_settings: bool,
}

impl Default for GenderPrivateUi {
    fn default() -> Self {
        let today = chrono::Local::now().date_naive();
        Self {
            year: today.year(),
            month: today.month(),
            selected_day: None,
            note_draft: String::new(),
            note_dirty: false,
            tip: String::new(),
            care_dismissed: false,
            focus_overdue: false,
            mark_picking_start: false,
            period_range_start: None,
            request_open_settings: false,
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
        st.note_dirty = false;
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
    st.note_dirty = false;
}

fn cancel_mark_guide(st: &mut GenderPrivateUi, status_line: &mut String) {
    st.mark_picking_start = false;
    st.period_range_start = None;
    st.tip.clear();
    *status_line = "已取消标记".into();
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
    // 与下一项的水平间距由外层 horizontal_wrapped 控制，不再额外加空白
    ui.add_space(8.0);
}

/// 图例色与格子一致（不用硬编码）。
fn legend_period_color() -> Color32 {
    theme::period_day()
}

fn legend_predict_color() -> Color32 {
    theme::period_pink_bg()
}

/// 经期前三天警告橙（软底，脉冲时加深）。
fn legend_period_prewarn_color() -> Color32 {
    Color32::from_rgb(0xFB, 0x92, 0x3C)
}

fn legend_period_prewarn_soft() -> Color32 {
    Color32::from_rgb(0xFF, 0xED, 0xD5)
}

/// 今天是否落在「下次经期开始前 1～3 天」（只闪当前日，不三天同闪）。
fn is_today_period_prewarn(today: NaiveDate, next: NaiveDate) -> bool {
    let d = (next - today).num_days();
    d >= 1 && d <= 3
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

fn lerp_rgb(a: Color32, b: Color32, t: f32) -> Color32 {
    Color32::from_rgb(
        lerp_u8(a.r(), b.r(), t),
        lerp_u8(a.g(), b.g(), t),
        lerp_u8(a.b(), b.b(), t),
    )
}

fn period_prewarn_fill(pulse: f32) -> Color32 {
    lerp_rgb(legend_period_prewarn_soft(), legend_period_prewarn_color(), pulse)
}

/// 经期格背景脉冲：已标记在实粉上闪，预测在浅粉上闪。
fn period_day_pulse_fill(marked: bool, pulse: f32) -> Color32 {
    if marked {
        let soft = Color32::from_rgb(0xF9, 0xA8, 0xD4);
        lerp_rgb(soft, theme::period_day(), pulse)
    } else {
        let soft = Color32::from_rgb(0xFD, 0xE8, 0xF2);
        lerp_rgb(soft, theme::period_pink_bg(), 0.35 + pulse * 0.65)
    }
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

fn persist_mark_checkup(
    svc: &Arc<MemoService>,
    health: &mut MaleHealthConfig,
    ymd: &str,
    status_line: &mut String,
    tip: &mut String,
) {
    match male_view::ensure_male_person(svc) {
        Ok(pid) => {
            health.mark_checkup_day(ymd);
            match svc.set_male_health(&pid, health.clone()) {
                Ok(()) => {
                    *status_line = if health.checkup_repeat && !health.next_checkup.is_empty() {
                        format!(
                            "已标体检 {ymd}；每 {} 个月，下次 {}",
                            health.checkup_interval_months, health.next_checkup
                        )
                    } else {
                        format!("已标体检 {ymd}")
                    };
                    tip.clear();
                }
                Err(e) => *status_line = format!("保存失败: {e}"),
            }
        }
        Err(e) => *status_line = e,
    }
}

fn persist_clear_checkup(
    svc: &Arc<MemoService>,
    health: &mut MaleHealthConfig,
    ymd: &str,
    status_line: &mut String,
    tip: &mut String,
) {
    match male_view::ensure_male_person(svc) {
        Ok(pid) => {
            health.clear_checkup_day(ymd);
            match svc.set_male_health(&pid, health.clone()) {
                Ok(()) => {
                    *status_line = format!("已取消体检 {ymd}");
                    tip.clear();
                }
                Err(e) => *status_line = format!("保存失败: {e}"),
            }
        }
        Err(e) => *status_line = e,
    }
}

/// 底栏/菜单：重复开关与间隔（月）。
fn checkup_repeat_controls(ui: &mut egui::Ui, health: &mut MaleHealthConfig, svc: &Arc<MemoService>) {
    let mut changed = false;
    if ui
        .checkbox(&mut health.checkup_repeat, "按周期重复")
        .on_hover_text("开启后，标注已过/今天的体检日时自动推算下次")
        .changed()
    {
        changed = true;
    }
    if health.checkup_repeat {
        ui.label(RichText::new("每").size(12.0).color(theme::text_muted()));
        let mut m = health.checkup_interval_months.max(1) as i32;
        let dv = ui.add(
            egui::DragValue::new(&mut m)
                .clamp_range(1..=36)
                .suffix(" 个月"),
        );
        if dv.changed() {
            health.checkup_interval_months = m as u32;
            changed = true;
        }
    }
    if changed {
        if health.checkup_interval_months == 0 {
            health.checkup_interval_months = 12;
        }
        save_health(svc, health);
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
    let before = (
        cycle.last_start.clone(),
        cycle.cycle_days,
        cycle.period_days,
        cycle.manual_cycle,
        cycle.manual_period,
        cycle.ovulation_preset,
    );
    period_view::apply_effective_cfg(&mut cycle, &stats);
    let changed = before.0 != cycle.last_start
        || before.1 != cycle.cycle_days
        || before.2 != cycle.period_days
        || before.3
        || before.4
        || before.5 != memo_core::OvulationPreset::Standard;
    if changed {
        save_cycle(svc, &cycle);
    }
    let next_period = period_view::next_expected(&cycle);

    let mut health = svc.current_male_health().unwrap_or_default();
    let last_checkup_ymd = health.last_checkup.clone();
    let checkup_ymd = health.next_checkup.clone();
    let days_to_checkup = health.days_until_checkup();

    if !cycle.period_enabled {
        st.mark_picking_start = false;
        st.period_range_start = None;
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
    let _ = alias_show;

    // —— 顶栏：关怀日历 + 图层芯片 + ⋯（无 Tab）——
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("关怀日历")
                .size(15.0)
                .strong()
                .color(theme::text()),
        );
        ui.label(
            RichText::new("仅本机可见")
                .size(11.0)
                .color(theme::text_muted()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let more = ui
                .add(
                    egui::Button::new(RichText::new("⋯").size(16.0).color(theme::text_muted()))
                        .fill(theme::card())
                        .stroke(Stroke::new(1.0, theme::border()))
                        .rounding(Rounding::same(7.0))
                        .min_size(Vec2::new(30.0, 28.0)),
                )
                .on_hover_text("关闭图层或打开应用设置");
            let more_id = ui.make_persistent_id("care_module_more");
            if more.clicked() {
                ui.memory_mut(|m| m.toggle_popup(more_id));
            }
            egui::popup_below_widget(ui, more_id, &more, |ui| {
                ui.set_min_width(200.0);
                if pe
                    && ui
                        .button("关闭「经期」图层")
                        .on_hover_text("标记仍保留，可随时再开启")
                        .clicked()
                {
                    cycle.period_enabled = false;
                    save_cycle(svc, cycle);
                    st.mark_picking_start = false;
                    st.period_range_start = None;
                    *status_line = "已关闭经期记录（标记仍保留）".into();
                    ui.memory_mut(|m| m.close_popup());
                }
                if ce
                    && ui
                        .button("关闭「体检」图层")
                        .on_hover_text("日期仍保留，可随时再开启")
                        .clicked()
                {
                    health.checkup_enabled = false;
                    save_health(svc, health);
                    *status_line = "已关闭体检提醒（日期仍保留）".into();
                    ui.memory_mut(|m| m.close_popup());
                }
                if pe || ce {
                    ui.separator();
                }
                if ui.button("在应用设置中管理…").clicked() {
                    st.request_open_settings = true;
                    ui.memory_mut(|m| m.close_popup());
                }
                ui.label(
                    RichText::new("关闭后数据仍保留；日历只隐藏对应图层。")
                        .size(11.0)
                        .color(theme::text_muted()),
                );
            });
            ui.add_space(6.0);
            // RTL：先体检再经期 → 视觉上经期在左
            if ce {
                layer_chip(ui, "体检", theme::shell_accent_soft(), theme::shell_accent());
                ui.add_space(4.0);
            }
            if pe {
                layer_chip(
                    ui,
                    "经期",
                    theme::shell_women_pill().linear_multiply(0.45),
                    theme::shell_women_pill_fg(),
                );
            }
        });
    });

    // 细关怀条（仅对已开启模块）
    if !st.care_dismissed && (pe || ce) {
        let period_care = pe.then(|| period_view::care_remind(svc, memos)).flatten();
        let health_care = ce.then(|| male_view::care_remind(svc)).flatten();
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

    if !pe && !ce {
        show_empty_enable(ui, svc, cycle, health, st, status_line);
        return;
    }

    // 引导条：两步标经期
    if pe && (st.mark_picking_start || st.period_range_start.is_some()) {
        show_mark_guide(ui, st, status_line);
        ui.add_space(4.0);
    }

    show_merged_care_page(
        ui,
        svc,
        st,
        memos,
        marked,
        cycle,
        next_period,
        health,
        last_checkup_ymd,
        checkup_ymd,
        days_to_checkup,
        status_line,
    );
}

fn layer_chip(ui: &mut egui::Ui, label: &str, fill: Color32, fg: Color32) {
    Frame::none()
        .fill(fill)
        .rounding(Rounding::same(999.0))
        .inner_margin(Margin::symmetric(9.0, 3.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(11.0).strong().color(fg));
        });
}

fn show_mark_guide(ui: &mut egui::Ui, st: &mut GenderPrivateUi, status_line: &mut String) {
    Frame::none()
        .fill(theme::shell_women_pill().linear_multiply(0.35))
        .stroke(Stroke::new(1.0, theme::period_day().linear_multiply(0.55)))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (title, sub) = if st.mark_picking_start {
                    (
                        "第 1 步 · 点选经期开始日".to_string(),
                        "只能选今天及以前；点错可取消。".to_string(),
                    )
                } else if let Some(start) = &st.period_range_start {
                    let short = if start.len() >= 10 {
                        &start[5..]
                    } else {
                        start.as_str()
                    };
                    (
                        format!("第 2 步 · 点选结束日（开始 {short}）"),
                        "点击日历结束日即可写入整段。".to_string(),
                    )
                } else {
                    return;
                };
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(title)
                            .size(13.0)
                            .strong()
                            .color(theme::shell_women_pill_fg()),
                    );
                    ui.label(
                        RichText::new(sub)
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::ghost_button(ui, "取消").clicked() {
                        cancel_mark_guide(st, status_line);
                    }
                });
            });
        });
}

fn show_empty_enable(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    cycle: &mut CycleConfig,
    health: &mut MaleHealthConfig,
    st: &mut GenderPrivateUi,
    status_line: &mut String,
) {
    ui.vertical_centered(|ui| {
        ui.add_space(28.0);
        ui.label(
            RichText::new("关怀模块均已关闭")
                .size(15.0)
                .strong()
                .color(theme::text()),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new("标记与日期仍保留。可在此开启，或稍后在应用设置里打开。")
                .size(12.5)
                .color(theme::text_muted()),
        );
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if theme::primary_button(ui, "开启经期").clicked() {
                cycle.period_enabled = true;
                save_cycle(svc, cycle);
                *status_line = "已开启经期记录".into();
            }
            if theme::ghost_button(ui, "开启体检").clicked() {
                health.checkup_enabled = true;
                save_health(svc, health);
                *status_line = "已开启体检提醒".into();
            }
            if theme::ghost_button(ui, "打开应用设置").clicked() {
                st.request_open_settings = true;
            }
        });
    });
}

/// 应用设置「关怀与健康」分区：总开关与体检调参。
pub fn show_care_health_settings(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    status_line: &mut String,
) {
    let mut cycle = svc.current_cycle().unwrap_or_default();
    let mut health = svc.current_male_health().unwrap_or_default();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("经期记录")
                .size(13.0)
                .strong()
                .color(theme::text()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut on = cycle.period_enabled;
            if ui.checkbox(&mut on, "").changed() {
                cycle.period_enabled = on;
                save_cycle(svc, &cycle);
                *status_line = if on {
                    "已开启经期记录".into()
                } else {
                    "已关闭经期记录（标记仍保留）".into()
                };
            }
        });
    });
    ui.label(
        RichText::new("关闭后日历隐藏经期图层；已有标记仍保留。")
            .size(11.5)
            .color(theme::text_muted()),
    );
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("体检提醒")
                .size(13.0)
                .strong()
                .color(theme::text()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut on = health.checkup_enabled;
            if ui.checkbox(&mut on, "").changed() {
                health.checkup_enabled = on;
                save_health(svc, &health);
                *status_line = if on {
                    "已开启体检提醒".into()
                } else {
                    "已关闭体检提醒（日期仍保留）".into()
                };
            }
        });
    });
    ui.label(
        RichText::new("关闭后隐藏体检图层；日期仍保留。")
            .size(11.5)
            .color(theme::text_muted()),
    );

    if health.checkup_enabled {
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("提前提醒")
                    .size(12.0)
                    .color(theme::text_muted()),
            );
            let mut d = health.remind_days as i32;
            let dv = ui.add(
                egui::DragValue::new(&mut d)
                    .clamp_range(1..=90)
                    .suffix(" 天"),
            );
            if dv.changed() {
                health.remind_days = d as u32;
                save_health(svc, &health);
            }
            ui.add_space(10.0);
            checkup_repeat_controls(ui, &mut health, svc);
        });
    }

    ui.add_space(6.0);
    ui.label(
        RichText::new("推算与提醒仅供参考，不能替代就医。")
            .size(11.0)
            .color(theme::text_muted()),
    );
}

#[allow(clippy::too_many_arguments)]
fn show_merged_care_page(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    marked: &[String],
    cycle: &mut CycleConfig,
    next_period: Option<NaiveDate>,
    health: &mut MaleHealthConfig,
    last_checkup_ymd: &str,
    checkup_ymd: &str,
    days_to_checkup: Option<i64>,
    status_line: &mut String,
) {
    let pe = cycle.period_enabled;
    let ce = health.checkup_enabled;

    // 图例与状态分行：同一行左右布局在窄宽下会互相覆盖
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().item_spacing.y = 2.0;
        if pe {
            legend_chip(ui, legend_period_color(), "经期");
            legend_chip(ui, legend_predict_color(), "预测");
            legend_chip(ui, legend_period_prewarn_color(), "临近");
            if cycle.ovulation_offset().is_some() {
                legend_chip(ui, legend_fertile_color(), "易孕");
                legend_text(ui, "排");
            }
        }
        if pe && ce {
            legend_text(ui, "·");
        }
        if ce {
            legend_chip(ui, theme::shell_accent(), "体检");
        }
        legend_dot(ui, legend_note_color(), "点=有备注");
    });

    let mut parts = Vec::new();
    if pe {
        if let Some(n) = next_period {
            parts.push(format!("预计经期 {}", n.format("%m-%d")));
        }
    }
    if ce {
        if !checkup_ymd.is_empty() {
            let mut s = format!("下次体检 {}", checkup_mmdd(checkup_ymd));
            if let Some(d) = days_to_checkup {
                if d > 0 {
                    s.push_str(&format!(" · 还有 {d} 天"));
                } else if d == 0 {
                    s.push_str(" · 今天");
                }
            }
            parts.push(s);
        } else if !last_checkup_ymd.is_empty() {
            parts.push(format!("上次 {}", checkup_mmdd(last_checkup_ymd)));
        } else {
            parts.push("体检 —".into());
        }
    }
    if !parts.is_empty() {
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(parts.join(" · "))
                    .size(11.0)
                    .color(theme::text_muted()),
            );
        });
    }
    ui.add_space(4.0);

    // 无底栏：月历刚好填满剩余高度；经期/体检/备注改日格右键
    let cal_h = ui.available_height().max(160.0);
    let remind_days = health.remind_days.max(1);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), cal_h),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_height(cal_h);
            ui.set_max_height(cal_h);
            show_calendar_frame(ui, st, memos, |ui, st| {
                draw_month_grid(
                    ui,
                    st,
                    memos,
                    marked,
                    last_checkup_ymd,
                    checkup_ymd,
                    svc,
                    cycle,
                    health,
                    status_line,
                    remind_days,
                    next_period,
                );
            });
        },
    );
}

fn show_calendar_frame(
    ui: &mut egui::Ui,
    st: &mut GenderPrivateUi,
    memos: &[MemoView],
    draw: impl FnOnce(&mut egui::Ui, &mut GenderPrivateUi),
) {
    let frame_h = ui.available_height();
    // 内高须扣掉上下 margin + 描边，否则 Frame 外高 ≈ frame_h + 20 会撑破右栏
    const FRAME_PAD: f32 = 10.0;
    let stroke_h = 2.0; // 上下各 1px
    let inner_budget = (frame_h - FRAME_PAD * 2.0 - stroke_h).max(80.0);
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(FRAME_PAD))
        .show(ui, |ui| {
            ui.set_min_height(inner_budget);
            ui.set_max_height(inner_budget);
            ui.spacing_mut().item_spacing.y = 0.0;
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
                    let today_btn = ui.add(
                        egui::Button::new(
                            RichText::new("今天").size(12.0).color(theme::text()),
                        )
                        .fill(theme::panel())
                        .stroke(Stroke::new(1.0, theme::border()))
                        .rounding(Rounding::same(8.0))
                        .min_size(Vec2::new(48.0, 26.0)),
                    );
                    if today_btn.clicked() {
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
    health: &mut MaleHealthConfig,
    status_line: &mut String,
    remind_days: u32,
    next_period: Option<NaiveDate>,
) {
    let pe = cycle.period_enabled;
    let ce = health.checkup_enabled;
    let care_pulse = {
        let t = ui.ctx().input(|i| i.time);
        let u = (t * 2.2).sin() as f32 * 0.5 + 0.5;
        0.25 + u * 0.75
    };
    let today = chrono::Local::now().date_naive();
    let today_in_prewarn = pe
        && next_period.is_some_and(|n| is_today_period_prewarn(today, n));
    let mut any_pulse = false;
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
    // 固定 6 行，与任务月历一致，纵向刚好填满
    let n_rows = 6_usize;

    let grid_w = ui.available_width().max(200.0);
    let gap = 3.0_f32;
    let cell_w = ((grid_w - gap * 6.0) / 7.0).max(1.0);
    ui.spacing_mut().item_spacing.y = 0.0;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.spacing_mut().item_spacing.y = 0.0;
        for w in ["一", "二", "三", "四", "五", "六", "日"] {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(cell_w, 18.0), Sense::hover());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                w,
                egui::FontId::proportional(11.0),
                theme::text_muted(),
            );
        }
    });
    ui.add_space(gap);

    // 星期行之后再量剩余高度，6 行 + 5 gap 刚好铺满；减 1px 防浮点微溢出
    let remain_h = ui.available_height().max(1.0);
    let cell_h =
        ((remain_h - gap * (n_rows - 1) as f32) / n_rows as f32 - 1.0).max(1.0);
    let cell = Vec2::new(cell_w, cell_h);

    for row in 0..n_rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.spacing_mut().item_spacing.y = 0.0;
            for col in 0..7 {
                let (date, in_month) = cells[row * 7 + col];
                let ymd = date.format("%Y-%m-%d").to_string();
                let day = date.day();
                let is_today = ymd == today_ymd();
                let is_sel = st.selected_day.as_deref() == Some(ymd.as_str());
                let has_note = !period_view::notes_on_day(memos, &ymd).is_empty();

                let is_marked = pe && marked.iter().any(|d| d == &ymd);
                let is_predicted =
                    pe && !is_marked && cycle.is_predicted_period_day(&ymd);
                // 临近：仅当前日闪橙（今天落在经期前 1～3 天时）
                let is_prewarn =
                    today_in_prewarn && is_today && !is_marked && !is_predicted;
                // 经期格（已标 / 预测）背景脉冲
                let is_period_pulse = is_marked || is_predicted;
                if is_prewarn || is_period_pulse {
                    any_pulse = true;
                }
                let is_ovulation =
                    pe && !is_marked && !is_predicted && cycle.is_ovulation_day(&ymd);
                let is_fertile = pe
                    && !is_marked
                    && !is_predicted
                    && !is_ovulation
                    && cycle.is_fertile_day(&ymd);
                let is_check = ce
                    && ((!last_checkup_ymd.is_empty() && last_checkup_ymd == ymd)
                        || (!checkup_ymd.is_empty() && checkup_ymd == ymd)
                        || health.is_checkup_day(&ymd));
                let check_col = checkup_date_color(&ymd, remind_days);

                let fill = if is_prewarn {
                    period_prewarn_fill(care_pulse)
                } else if is_period_pulse {
                    period_day_pulse_fill(is_marked, care_pulse)
                } else if is_sel {
                    theme::shell_accent_soft()
                } else if is_check {
                    theme::shell_accent_soft().linear_multiply(0.55)
                } else {
                    theme::panel()
                };

                let stroke = if is_sel {
                    Stroke::new(2.0, theme::shell_accent())
                } else if is_check && is_marked {
                    Stroke::new(2.0, Color32::WHITE)
                } else if is_check {
                    Stroke::new(checkup_stroke_w(&ymd, remind_days), check_col)
                } else if is_prewarn {
                    Stroke::new(
                        1.6 + care_pulse * 0.6,
                        legend_period_prewarn_color(),
                    )
                } else if is_period_pulse {
                    Stroke::new(
                        if is_marked { 1.2 } else { 1.4 } + care_pulse * 0.4,
                        theme::period_day(),
                    )
                } else if is_today {
                    Stroke::new(1.5, theme::shell_accent())
                } else if is_ovulation {
                    Stroke::new(2.2, theme::period_fertile())
                } else if is_fertile {
                    Stroke::new(1.6, theme::period_fertile())
                } else {
                    Stroke::new(1.0, theme::border())
                };

                // 角标：体检优先；否则排卵
                let badge = if is_check {
                    Some((
                        "检",
                        if is_marked {
                            Color32::WHITE
                        } else {
                            check_col
                        },
                    ))
                } else if is_ovulation {
                    Some(("排", theme::period_fertile()))
                } else {
                    None
                };
                let countdown = if is_check && !is_marked {
                    checkup_countdown_short(&ymd)
                } else {
                    None
                };

                let (rect, resp) = ui.allocate_exact_size(cell, Sense::click());
                ui.painter()
                    .rect(rect.shrink(1.0), Rounding::same(6.0), fill, stroke);

                let fg = if is_marked {
                    Color32::WHITE
                } else if is_sel {
                    theme::shell_accent()
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
                        egui::FontId::proportional(11.0),
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
                    let dot_c = if is_marked {
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
                    if pe && st.mark_picking_start {
                        st.mark_picking_start = false;
                        period_view::mark_period_start(
                            svc,
                            &ymd,
                            memos,
                            cycle,
                            status_line,
                            &mut st.tip,
                            &mut st.period_range_start,
                        );
                        load_note_draft(st, memos);
                    } else if pe && st.period_range_start.is_some() {
                        period_view::mark_period_end(
                            svc,
                            &ymd,
                            memos,
                            cycle,
                            status_line,
                            &mut st.tip,
                            &mut st.period_range_start,
                        );
                        st.mark_picking_start = false;
                        load_note_draft(st, memos);
                    } else {
                        load_note_draft(st, memos);
                    }
                }

                // 始终可右键：经期 / 体检 / 备注（替代原底栏）
                resp.context_menu(|ui| {
                    theme::style_context_menu(ui);
                    ui.set_min_width(220.0);
                    let guiding =
                        pe && (st.mark_picking_start || st.period_range_start.is_some());
                    if guiding {
                        if theme::menu_item_mark(ui, "×", "取消标记").clicked() {
                            cancel_mark_guide(st, status_line);
                            ui.close_menu();
                        }
                        theme::menu_sep(ui);
                    }

                    if st.selected_day.as_deref() != Some(ymd.as_str()) {
                        st.selected_day = Some(ymd.clone());
                        load_note_draft(st, memos);
                    }

                    let day_label = if ymd.len() >= 10 {
                        format!("{} 日", &ymd[5..])
                    } else {
                        ymd.clone()
                    };
                    ui.label(
                        RichText::new(day_label)
                            .size(12.5)
                            .strong()
                            .color(theme::text()),
                    );
                    ui.add_space(4.0);

                    if pe {
                        let is_future = NaiveDate::parse_from_str(&ymd, "%Y-%m-%d")
                            .ok()
                            .map(|d| d > chrono::Local::now().date_naive())
                            .unwrap_or(true);
                        let marked_here = marked.iter().any(|d| d == &ymd);
                        theme::menu_section(ui, "经期");
                        if marked_here {
                            if theme::menu_item_danger_mark(ui, "清", "清除本段经期")
                                .on_hover_text("删除该日所在整段")
                                .clicked()
                            {
                                period_view::clear_period_episode(
                                    svc,
                                    &ymd,
                                    cycle,
                                    status_line,
                                    &mut st.tip,
                                );
                                load_note_draft(st, memos);
                                ui.close_menu();
                            }
                            if theme::menu_item_mark(ui, "重", "重新标记…").clicked() {
                                st.mark_picking_start = true;
                                st.period_range_start = None;
                                *status_line = "请点选经期开始日".into();
                                ui.close_menu();
                            }
                        } else {
                            ui.add_enabled_ui(!is_future, |ui| {
                                if theme::menu_item_mark(ui, "经", "标记经期")
                                    .on_hover_text("两步引导：开始 → 结束")
                                    .on_disabled_hover_text("未来日期不能标经期")
                                    .clicked()
                                {
                                    st.mark_picking_start = true;
                                    st.period_range_start = None;
                                    *status_line = "请点选经期开始日".into();
                                    ui.close_menu();
                                }
                                if theme::menu_item_mark(ui, "起", "从此日开始")
                                    .on_hover_text("以本日为开始，再点结束日")
                                    .clicked()
                                {
                                    st.mark_picking_start = false;
                                    period_view::mark_period_start(
                                        svc,
                                        &ymd,
                                        memos,
                                        cycle,
                                        status_line,
                                        &mut st.tip,
                                        &mut st.period_range_start,
                                    );
                                    load_note_draft(st, memos);
                                    ui.close_menu();
                                }
                                if theme::menu_item_mark(ui, "止", "经期结束")
                                    .on_hover_text("先有开始日，再点结束：自动标整段")
                                    .clicked()
                                {
                                    period_view::mark_period_end(
                                        svc,
                                        &ymd,
                                        memos,
                                        cycle,
                                        status_line,
                                        &mut st.tip,
                                        &mut st.period_range_start,
                                    );
                                    st.mark_picking_start = false;
                                    load_note_draft(st, memos);
                                    ui.close_menu();
                                }
                            });
                        }
                    }

                    if pe && ce {
                        theme::menu_sep(ui);
                    }

                    if ce {
                        theme::menu_section(ui, "体检");
                        let marked_here = health.is_checkup_day(&ymd);
                        if marked_here {
                            if theme::menu_item_danger_mark(ui, "消", "取消体检日").clicked() {
                                persist_clear_checkup(
                                    svc,
                                    health,
                                    &ymd,
                                    status_line,
                                    &mut st.tip,
                                );
                                load_note_draft(st, memos);
                                ui.close_menu();
                            }
                        } else if theme::menu_item_mark(ui, "检", "标为体检日")
                            .on_hover_text(if health.checkup_repeat {
                                format!(
                                    "已过/今天会记入上次，并按每 {} 个月推下次",
                                    health.checkup_interval_months.max(1)
                                )
                            } else {
                                "已过→上次体检；今天及未来→下次体检".into()
                            })
                            .clicked()
                        {
                            persist_mark_checkup(svc, health, &ymd, status_line, &mut st.tip);
                            load_note_draft(st, memos);
                            ui.close_menu();
                        }
                    }

                    if pe || ce {
                        theme::menu_sep(ui);
                    }
                    theme::menu_section(ui, "备注");
                    show_note_editor(ui, svc, st, &ymd, status_line);
                });
            }
        });
        if row + 1 < n_rows {
            ui.add_space(gap);
        }
    }
    if any_pulse {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(80));
    }
}

fn show_note_editor(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    st: &mut GenderPrivateUi,
    ymd: &str,
    status_line: &mut String,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let te = ui.add(
            egui::TextEdit::multiline(&mut st.note_draft)
                .desired_width((ui.available_width() - 64.0).max(120.0))
                .desired_rows(2)
                .hint_text(theme::hint("当天备注 / 健康日记…")),
        );
        if te.changed() {
            st.note_dirty = true;
        }
        let save = ui.add_enabled(
            st.note_dirty,
            egui::Button::new(RichText::new("保存").size(12.5).color(Color32::WHITE))
                .fill(if st.note_dirty {
                    theme::shell_accent()
                } else {
                    theme::text_muted()
                })
                .rounding(Rounding::same(8.0))
                .min_size(Vec2::new(56.0, 32.0)),
        );
        let do_save = save.clicked() || (te.lost_focus() && st.note_dirty);
        if do_save {
            period_view::save_day_note(svc, ymd, &st.note_draft, status_line);
            st.note_dirty = false;
        }
    });
    ui.label(
        RichText::new(if st.note_dirty {
            "未保存 · 失焦将自动保存"
        } else if st.note_draft.trim().is_empty() {
            "备注未改动"
        } else {
            "已有备注"
        })
        .size(11.0)
        .color(theme::text_muted()),
    );
}
