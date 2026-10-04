//! 男性私密 · 健康提醒与私密日记（体检日 + 日历备注）。

use crate::date_field;
use crate::theme;
use chrono::{Datelike, NaiveDate};
use eframe::egui::{self, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::male_health::MaleHealthConfig;
use memo_core::service::{MemoService, MemoView};
use memo_core::store::{MemoCategory, MemoLifecycle, MemoPriority, MemoVisibility};
use std::sync::Arc;

const REF_DISCLAIMER: &str =
    "系统仅根据你填写的体检日给出提醒，不能替代正规体检与就医。身体不适请及时咨询医生。";

#[derive(Debug, Clone)]
pub struct MaleUi {
    pub year: i32,
    pub month: u32,
    pub selected_day: Option<String>,
    pub note_draft: String,
    pub tip: String,
}

impl Default for MaleUi {
    fn default() -> Self {
        let today = chrono::Local::now().date_naive();
        Self {
            year: today.year(),
            month: today.month(),
            selected_day: None,
            note_draft: String::new(),
            tip: String::new(),
        }
    }
}

/// 男性健康关怀提醒。
#[derive(Debug, Clone)]
pub struct MaleCareRemind {
    pub active: bool,
    pub seen: bool,
    pub next_checkup: String,
    pub days_until: i64,
    pub title: String,
    pub body: String,
}

impl MaleCareRemind {
    pub fn needs_attention(&self) -> bool {
        self.active && !self.seen
    }
}

fn today_ymd() -> String {
    chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string()
}

fn parse_ymd(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

pub(crate) fn ensure_male_person(svc: &MemoService) -> Result<String, String> {
    svc.ensure_session_person()
        .map_err(|e| e.to_string())?;
    svc.current_person_id()
        .ok_or_else(|| "请先在设置中指定当前人员".into())
}

fn diary_memos(memos: &[MemoView]) -> Vec<&MemoView> {
    memos
        .iter()
        .filter(|m| {
            m.category.is_gender_private()
                && !m.due_date.is_empty()
                && m.title != "经期日"
                && m.title != "经期"
                && !m.title.contains("经期开始")
                && !m.title.starts_with("备注 ")
        })
        .collect()
}

fn notes_on_day<'a>(memos: &'a [MemoView], ymd: &str) -> Vec<&'a MemoView> {
    diary_memos(memos)
        .into_iter()
        .filter(|m| {
            memo_core::due_date_part(&m.due_date)
                .map(|d| d.format("%Y-%m-%d").to_string() == ymd)
                .unwrap_or(false)
        })
        .collect()
}

fn days_with_notes(memos: &[MemoView]) -> Vec<String> {
    let mut days: Vec<String> = diary_memos(memos)
        .into_iter()
        .filter_map(|m| memo_core::due_date_part(&m.due_date).map(|d| d.format("%Y-%m-%d").to_string()))
        .collect();
    days.sort();
    days.dedup();
    days
}

fn warm_copy(days_until: i64) -> (String, String) {
    match days_until {
        d if d > 3 => (
            "体检快到啦".into(),
            "提前安排时间，准备好过往报告与问题清单，照顾好自己。".into(),
        ),
        2 | 3 => (
            "体检临近".into(),
            "这两天注意作息，少熬夜；需要空腹的项目记得提前确认。".into(),
        ),
        1 => (
            "明天可能要体检".into(),
            "今晚早点休息，按医院要求禁食/禁饮。有疑问可先电话确认。".into(),
        ),
        0 => (
            "今天是计划体检日".into(),
            "放松心态，按流程完成即可。结果出来后可记在本页日记里。".into(),
        ),
        _ => (
            "体检日已过".into(),
            "若已完成，可更新「上次体检」并约定下次；若延期，改一下下次日期即可。".into(),
        ),
    }
}

/// 根据本机配置生成关怀提醒。
pub fn care_remind(svc: &MemoService) -> Option<MaleCareRemind> {
    let cfg = svc.current_male_health().unwrap_or_default();
    if !cfg.checkup_enabled {
        return None;
    }
    if !cfg.care_active() {
        return None;
    }
    let days = cfg.days_until_checkup()?;
    let (title, body) = warm_copy(days);
    Some(MaleCareRemind {
        active: true,
        seen: cfg.care_seen(),
        next_checkup: cfg.next_checkup.clone(),
        days_until: days,
        title,
        body,
    })
}

pub fn ack_care_remind(svc: &MemoService, next_checkup: &str) {
    let Ok(pid) = ensure_male_person(svc) else {
        return;
    };
    let mut cfg = svc.get_male_health(&pid).unwrap_or_default();
    if cfg.remind_seen_for == next_checkup {
        return;
    }
    cfg.remind_seen_for = next_checkup.to_string();
    if cfg.next_checkup.is_empty() {
        cfg.next_checkup = next_checkup.to_string();
    }
    let _ = svc.set_male_health(&pid, cfg);
}

fn load_note_draft(ui_st: &mut MaleUi, memos: &[MemoView]) {
    let Some(ymd) = ui_st.selected_day.clone() else {
        ui_st.note_draft.clear();
        return;
    };
    if let Some(m) = notes_on_day(memos, &ymd).into_iter().next() {
        ui_st.note_draft = if m.content.is_empty() {
            m.title.clone()
        } else {
            m.content.clone()
        };
    } else {
        ui_st.note_draft.clear();
    }
}

fn save_day_note(
    svc: &Arc<MemoService>,
    ymd: &str,
    note: &str,
    status_line: &mut String,
) {
    let body = note.trim();
    if body.is_empty() {
        *status_line = "备注为空，未保存".into();
        return;
    }
    let title = {
        let line = body.lines().next().unwrap_or("健康日记").trim();
        if line.is_empty() {
            format!("日记 {ymd}")
        } else {
            line.chars().take(40).collect()
        }
    };
    let list = svc.list();
    if let Some(m) = notes_on_day(&list, ymd).into_iter().next() {
        match svc.edit_full(
            &m.id,
            &title,
            body,
            MemoVisibility::Private,
            MemoLifecycle::Permanent,
            MemoCategory::GenderPrivate,
            ymd,
            m.done,
            &m.tags,
            m.priority,
            m.remind_before_days,
            &m.remind_seen_for,
        ) {
            Ok(()) => *status_line = format!("已更新 {ymd} 日记"),
            Err(e) => *status_line = format!("保存失败: {e}"),
        }
        return;
    }
    match svc.add_full(
        &title,
        body,
        MemoVisibility::Private,
        MemoLifecycle::Permanent,
        MemoCategory::GenderPrivate,
        ymd,
        &[],
        MemoPriority::Normal,
        0,
    ) {
        Ok(_) => *status_line = format!("已保存 {ymd} 日记"),
        Err(e) => *status_line = format!("保存失败: {e}"),
    }
}

/// 绘制体检健康面板（旧全页布局；专属页已改用日历化入口）。
#[allow(dead_code)]
pub fn show(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    male: &mut MaleUi,
    memos: &[MemoView],
    status_line: &mut String,
) {
    let alias = svc.session_alias();
    let alias_show = if alias.is_empty() { "当前用户" } else { alias };
    let avail_h = ui.available_height();
    let avail_w = ui.available_width();
    let body_h = (avail_h * 0.55).clamp(260.0, 520.0);

    let pid = svc.current_person_id();
    let mut cfg = pid
        .as_ref()
        .and_then(|id| svc.get_male_health(id))
        .unwrap_or_default();
    let note_days = days_with_notes(memos);
    let days_until = cfg.days_until_checkup();

    egui::ScrollArea::vertical()
        .id_source("male_view_scroll")
        .auto_shrink([false, false])
        .max_height(avail_h)
        .show(ui, |ui| {
            ui.set_width(avail_w);
            Frame::none()
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::border()))
                .rounding(Rounding::same(14.0))
                .inner_margin(Margin::symmetric(18.0, 16.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("男性私密 · 健康与私密笔记")
                                .size(20.0)
                                .strong()
                                .color(theme::text()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            Frame::none()
                                .fill(theme::shell_men_pill())
                                .rounding(Rounding::same(10.0))
                                .inner_margin(Margin::symmetric(10.0, 4.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!("已加密 · 仅 {alias_show} 可见"))
                                            .size(12.0)
                                            .strong()
                                            .color(theme::shell_men_pill_fg()),
                                    );
                                });
                        });
                    });
                    ui.add_space(12.0);

                    // 摘要
                    Frame::none()
                        .fill(theme::shell_men_pill())
                        .rounding(Rounding::same(12.0))
                        .inner_margin(Margin::symmetric(14.0, 12.0))
                        .show(ui, |ui| {
                            ui.columns(3, |cols| {
                                cols[0].vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new("下次体检")
                                            .size(12.0)
                                            .color(theme::text_muted()),
                                    );
                                    let v = if cfg.next_checkup.len() >= 10 {
                                        &cfg.next_checkup[5..]
                                    } else if cfg.next_checkup.is_empty() {
                                        "—"
                                    } else {
                                        cfg.next_checkup.as_str()
                                    };
                                    ui.label(
                                        RichText::new(v)
                                            .size(18.0)
                                            .strong()
                                            .color(theme::shell_men_pill_fg()),
                                    );
                                });
                                cols[1].vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new("距今天数")
                                            .size(12.0)
                                            .color(theme::text_muted()),
                                    );
                                    let v = match days_until {
                                        Some(d) if d >= 0 => format!("{d} 天"),
                                        Some(d) => format!("已过 {} 天", -d),
                                        None => "—".into(),
                                    };
                                    ui.label(
                                        RichText::new(v)
                                            .size(18.0)
                                            .strong()
                                            .color(theme::text()),
                                    );
                                });
                                cols[2].vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new("健康日记")
                                            .size(12.0)
                                            .color(theme::text_muted()),
                                    );
                                    ui.label(
                                        RichText::new(format!("{} 天", note_days.len()))
                                            .size(18.0)
                                            .strong()
                                            .color(theme::text()),
                                    );
                                });
                            });
                        });

                    ui.add_space(8.0);
                    Frame::none()
                        .fill(theme::panel())
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::symmetric(12.0, 8.0))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(REF_DISCLAIMER)
                                    .size(12.0)
                                    .color(theme::warn()),
                            );
                        });

                    if let Some(care) = care_remind(svc) {
                        ui.add_space(10.0);
                        Frame::none()
                            .fill(theme::shell_men_pill())
                            .stroke(Stroke::new(1.0, theme::shell_men_pill_fg().linear_multiply(0.4)))
                            .rounding(Rounding::same(12.0))
                            .inner_margin(Margin::symmetric(14.0, 12.0))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("♡  {}", care.title))
                                        .size(15.0)
                                        .strong()
                                        .color(theme::shell_men_pill_fg()),
                                );
                                ui.label(
                                    RichText::new(&care.body)
                                        .size(13.0)
                                        .color(theme::shell_men_pill_fg()),
                                );
                            });
                    }

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let cal_w = (ui.available_width() * 0.58).clamp(280.0, 520.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(cal_w, body_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                show_calendar(ui, male, memos, &note_days);
                            },
                        );
                        ui.add_space(12.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                show_side(
                                    ui,
                                    svc,
                                    male,
                                    memos,
                                    &mut cfg,
                                    &note_days,
                                    status_line,
                                );
                            },
                        );
                    });
                });
        });
}

fn show_calendar(
    ui: &mut egui::Ui,
    male: &mut MaleUi,
    memos: &[MemoView],
    note_days: &[String],
) {
    ui.horizontal(|ui| {
        if ui.button("‹").clicked() {
            if male.month == 1 {
                male.month = 12;
                male.year -= 1;
            } else {
                male.month -= 1;
            }
        }
        ui.label(
            RichText::new(format!("{}年{}月", male.year, male.month))
                .strong()
                .size(15.0)
                .color(theme::text()),
        );
        if ui.button("›").clicked() {
            if male.month == 12 {
                male.month = 1;
                male.year += 1;
            } else {
                male.month += 1;
            }
        }
        if theme::ghost_button(ui, "今天").clicked() {
            let t = chrono::Local::now().date_naive();
            male.year = t.year();
            male.month = t.month();
            male.selected_day = Some(today_ymd());
            load_note_draft(male, memos);
        }
    });
    ui.add_space(8.0);

    let first = NaiveDate::from_ymd_opt(male.year, male.month, 1);
    let Some(first) = first else {
        return;
    };
    let days_in_month = if male.month == 12 {
        NaiveDate::from_ymd_opt(male.year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(male.year, male.month + 1, 1)
    }
    .map(|d| (d - first).num_days() as u32)
    .unwrap_or(30);
    let start_pad = first.weekday().num_days_from_monday() as usize;
    let grid_w = ui.available_width().max(210.0);
    let cell = ((grid_w - 6.0) / 7.0).clamp(28.0, 48.0);

    ui.horizontal(|ui| {
        for w in ["一", "二", "三", "四", "五", "六", "日"] {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(cell, 18.0), Sense::hover());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                w,
                egui::FontId::proportional(11.0),
                theme::text_muted(),
            );
        }
    });

    let mut day = 1u32;
    let mut pad_left = start_pad;
    while day <= days_in_month {
        ui.horizontal(|ui| {
            for _ in 0..7 {
                if pad_left > 0 {
                    let _ = ui.allocate_exact_size(Vec2::splat(cell), Sense::hover());
                    pad_left -= 1;
                    continue;
                }
                if day > days_in_month {
                    let _ = ui.allocate_exact_size(Vec2::splat(cell), Sense::hover());
                    continue;
                }
                let ymd = format!("{:04}-{:02}-{:02}", male.year, male.month, day);
                let has_note = note_days.iter().any(|d| d == &ymd);
                let is_today = ymd == today_ymd();
                let is_sel = male.selected_day.as_deref() == Some(ymd.as_str());
                let fill = if is_sel {
                    theme::shell_men_pill()
                } else if has_note {
                    theme::shell_accent_soft()
                } else {
                    theme::panel()
                };
                let stroke = if is_sel {
                    Stroke::new(2.0, theme::shell_men_pill_fg())
                } else if is_today {
                    Stroke::new(1.5, theme::shell_accent())
                } else {
                    Stroke::new(1.0, theme::border())
                };
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(cell), Sense::click());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                ui.painter()
                    .rect(rect.shrink(2.0), Rounding::same(8.0), fill, stroke);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    day.to_string(),
                    egui::FontId::proportional(13.0),
                    theme::text(),
                );
                if has_note {
                    ui.painter().circle_filled(
                        egui::pos2(rect.center().x, rect.bottom() - 6.0),
                        2.0,
                        theme::shell_men_pill_fg(),
                    );
                }
                if resp.clicked() {
                    male.selected_day = Some(ymd);
                    load_note_draft(male, memos);
                }
                day += 1;
            }
        });
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new("点击日期写健康日记；蓝点表示该日已有记录")
            .size(11.5)
            .color(theme::text_muted()),
    );
}

fn show_side(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    male: &mut MaleUi,
    memos: &[MemoView],
    cfg: &mut MaleHealthConfig,
    note_days: &[String],
    status_line: &mut String,
) {
    Frame::none()
        .fill(theme::panel())
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(12.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("体检提醒")
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.label(
                RichText::new("填写下次体检日；临近时会温暖提醒你（仅供参考）。")
                    .size(11.5)
                    .color(theme::text_muted()),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("上次").small().color(theme::text_muted()));
                date_field::show_compact(ui, "male_last_checkup", &mut cfg.last_checkup, true);
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("下次").small().color(theme::text_muted()));
                date_field::show_compact(ui, "male_next_checkup", &mut cfg.next_checkup, true);
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("提前").small().color(theme::text_muted()));
                let mut d = cfg.remind_days as i32;
                if ui
                    .add(egui::DragValue::new(&mut d).clamp_range(1..=90).suffix(" 天"))
                    .changed()
                {
                    cfg.remind_days = d as u32;
                }
            });
            ui.add_space(6.0);
            if theme::primary_button(ui, "保存体检设置").clicked() {
                match ensure_male_person(svc) {
                    Ok(id) => match svc.set_male_health(&id, cfg.clone()) {
                        Ok(()) => {
                            *status_line = "体检提醒已保存（仅本机）".into();
                            male.tip.clear();
                        }
                        Err(e) => *status_line = format!("保存失败: {e}"),
                    },
                    Err(e) => *status_line = e,
                }
            }
        });

    ui.add_space(10.0);
    Frame::none()
        .fill(theme::panel())
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(12.0))
        .show(ui, |ui| {
            let day_label = male
                .selected_day
                .clone()
                .unwrap_or_else(|| "未选择日期".into());
            ui.label(
                RichText::new(format!("当日日记 · {day_label}"))
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.label(
                RichText::new("睡眠、运动、用药保健、情绪等，仅本人可见。")
                    .size(11.5)
                    .color(theme::text_muted()),
            );
            ui.add_space(6.0);
            let enabled = male.selected_day.is_some();
            ui.add_enabled_ui(enabled, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut male.note_draft)
                        .desired_width(ui.available_width())
                        .desired_rows(5)
                        .hint_text(theme::hint("例如：睡眠 7h，慢跑 30 分钟…")),
                );
                ui.add_space(6.0);
                if theme::success_button(ui, "保存日记").clicked() {
                    if let Some(ymd) = male.selected_day.clone() {
                        save_day_note(svc, &ymd, &male.note_draft, status_line);
                    }
                }
            });
        });

    ui.add_space(10.0);
    ui.label(
        RichText::new("最近日记")
            .strong()
            .size(14.0)
            .color(theme::text()),
    );
    let recent: Vec<_> = note_days.iter().rev().take(8).collect();
    if recent.is_empty() {
        ui.label(theme::muted_label("暂无日记。点日历日期开始记录。"));
    } else {
        for d in recent {
            if ui
                .link(RichText::new(d).color(theme::shell_men_pill_fg()))
                .clicked()
            {
                if let Some(dt) = parse_ymd(d) {
                    male.year = dt.year();
                    male.month = dt.month();
                }
                male.selected_day = Some(d.clone());
                load_note_draft(male, memos);
            }
        }
    }
    if !male.tip.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new(&male.tip).small().color(theme::warn()));
    }
}
