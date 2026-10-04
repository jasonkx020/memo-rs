//! 女性私密 · 生理期记录（标记经期日 → 自动推算周期 / 经期长 / 下次预计）。

use crate::theme;
use chrono::{Datelike, NaiveDate};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::cycle::{
    apply_mark_stats, analyze_period_marks, CycleConfig, PeriodMarkStats,
};
use memo_core::service::{MemoService, MemoView};
use memo_core::store::{MemoCategory, MemoLifecycle, MemoVisibility};
use std::sync::Arc;

const REF_DISCLAIMER: &str =
    "系统仅根据你标记的历史经期日给出参考时间，不能确保完全正确。月经不调、压力、疾病、药物等都可能影响周期，请以自身感受为准，必要时咨询医生。";

/// 经期临近 / 进行中的关怀提醒（未点开「女性私密」前持续友好提示）。
#[derive(Debug, Clone)]
pub struct PeriodCareRemind {
    /// 是否处于提醒窗口（预计前 3 天～经期窗口内）
    pub active: bool,
    /// 用户是否已点开查看过本轮预计
    pub seen: bool,
    /// 本轮预计开始日 YYYY-MM-DD
    pub expected_start: String,
    /// 距预计开始的天数（负值表示已过预计日）
    pub days_until: i64,
    pub title: String,
    pub body: String,
}

impl PeriodCareRemind {
    pub fn needs_attention(&self) -> bool {
        self.active && !self.seen
    }
}

/// 根据当前推算生成关怀提醒；无法推算时返回 None。
pub fn care_remind(svc: &MemoService, memos: &[MemoView]) -> Option<PeriodCareRemind> {
    let pid = svc.current_person_id()?;
    let mut cfg = svc.get_cycle(&pid).unwrap_or_default();
    if !cfg.period_enabled {
        return None;
    }
    let marked = period_marked_days(memos);
    let stats = analyze_period_marks(&marked);
    apply_effective_cfg(&mut cfg, &stats);
    if !cfg.can_predict() {
        return None;
    }
    let next = next_expected(&cfg)?;
    let expected_start = next.format("%Y-%m-%d").to_string();
    let days = days_until(next);
    let period_len = cfg.period_days.max(1) as i64;
    if days > 3 || days < -(period_len - 1) {
        return None;
    }
    let (title, body) = warm_copy(days);
    let seen = cfg.remind_seen_for == expected_start;
    Some(PeriodCareRemind {
        active: true,
        seen,
        expected_start,
        days_until: days,
        title,
        body,
    })
}

/// 用户点开「女性私密」后标记本轮提醒已读。
pub fn ack_care_remind(svc: &MemoService, expected_start: &str) {
    let Ok(pid) = ensure_cycle_person(svc) else {
        return;
    };
    let mut cfg = svc.get_cycle(&pid).unwrap_or_default();
    if cfg.remind_seen_for == expected_start {
        return;
    }
    cfg.remind_seen_for = expected_start.to_string();
    if cfg.last_start.is_empty() {
        let marked = period_marked_days(&svc.list());
        let stats = analyze_period_marks(&marked);
        apply_effective_cfg(&mut cfg, &stats);
    }
    let _ = svc.set_cycle(&pid, cfg);
}

pub(crate) fn apply_effective_cfg(cfg: &mut CycleConfig, stats: &PeriodMarkStats) {
    cfg.manual_cycle = false;
    cfg.manual_period = false;
    cfg.ovulation_preset = memo_core::OvulationPreset::Standard;
    apply_mark_stats(cfg, stats);
}

fn warm_copy(days_until: i64) -> (String, String) {
    match days_until {
        d if d >= 3 => (
            "经期快到啦".into(),
            "大约还有几天就会来潮。记得温柔待己，提前备好需要的东西～（仅供参考）".into(),
        ),
        2 => (
            "还有两天呢".into(),
            "可以悄悄准备好卫生用品，给自己多一点从容。累了就歇一歇。".into(),
        ),
        1 => (
            "明天或许会来潮".into(),
            "今晚早点休息，喝点温水，给身体多一点柔软。不舒服也没关系。".into(),
        ),
        0 => (
            "今天可能是经期开始日".into(),
            "抱抱自己，慢慢来。需要的话记一记感受；预测仅供参考哦。".into(),
        ),
        d if d >= -2 => (
            "经期可能已经开始".into(),
            "多喝温水、注意保暖。疼痛或不适时请好好休息，不必硬撑。".into(),
        ),
        _ => (
            "经期期间，好好照顾自己".into(),
            "温水、休息、保暖。若与实际不符，可在日历上补标真实经期日，帮下次推算更准一些。".into(),
        ),
    }
}

#[derive(Debug, Clone)]
pub struct PeriodUi {
    pub year: i32,
    pub month: u32,
    /// 状态提示（本面板内）
    pub tip: String,
    /// 当前选中的日期（写备注）
    pub selected_day: Option<String>,
    pub note_draft: String,
    /// 是否展开「怎么计算」说明
    pub show_guide: bool,
    /// 是否展开特例调整
    pub show_override: bool,
}

impl Default for PeriodUi {
    fn default() -> Self {
        let today = chrono::Local::now().date_naive();
        Self {
            year: today.year(),
            month: today.month(),
            tip: String::new(),
            selected_day: None,
            note_draft: String::new(),
            show_guide: false,
            show_override: false,
        }
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

pub(crate) fn next_expected(cfg: &CycleConfig) -> Option<NaiveDate> {
    cfg.next_predicted_start()
}

fn days_until(d: NaiveDate) -> i64 {
    (d - chrono::Local::now().date_naive()).num_days()
}

/// 确保会话人员可用于周期写入。
pub(crate) fn ensure_cycle_person(svc: &MemoService) -> Result<String, String> {
    let _ = svc.ensure_session_person();
    svc.current_person_id()
        .ok_or_else(|| "请先完成身份解锁".into())
}

/// 已标记的经期日（含历史「经期开始」标题，兼容旧数据）。
pub(crate) fn period_marked_days(memos: &[MemoView]) -> Vec<String> {
    let mut days: Vec<String> = memos
        .iter()
        .filter(|m| {
            m.category.is_gender_private()
                && !m.due_date.is_empty()
                && (m.title == "经期"
                    || m.title == "经期日"
                    || m.title.contains("经期开始"))
        })
        .filter_map(|m| memo_core::due_date_part(&m.due_date).map(|d| d.format("%Y-%m-%d").to_string()))
        .collect();
    days.sort();
    days.dedup();
    days
}

fn is_marked_day(days: &[String], ymd: &str) -> bool {
    days.iter().any(|d| d == ymd)
}

fn due_matches_ymd(due: &str, ymd: &str) -> bool {
    memo_core::due_date_part(due)
        .map(|d| d.format("%Y-%m-%d").to_string() == ymd)
        .unwrap_or(false)
}

pub(crate) fn notes_on_day<'a>(memos: &'a [MemoView], ymd: &str) -> Vec<&'a MemoView> {
    memos
        .iter()
        .filter(|m| {
            m.category.is_gender_private()
                && due_matches_ymd(&m.due_date, ymd)
                && m.title != "经期"
                && m.title != "经期日"
                && !m.title.contains("经期开始")
        })
        .collect()
}

fn find_period_day_memo<'a>(memos: &'a [MemoView], ymd: &str) -> Option<&'a MemoView> {
    memos.iter().find(|m| {
        m.category.is_gender_private()
            && due_matches_ymd(&m.due_date, ymd)
            && (m.title == "经期"
                || m.title == "经期日"
                || m.title.contains("经期开始"))
    })
}

pub(crate) fn sync_cfg_from_marks(
    svc: &MemoService,
    pid: &str,
    cfg: &mut CycleConfig,
    marked: &[String],
) {
    let stats = analyze_period_marks(marked);
    apply_mark_stats(cfg, &stats);
    // 标记变更后下次预测与关怀以新预计开始日为准
    cfg.remind_seen_for.clear();
    let _ = svc.set_cycle(pid, cfg.clone());
}

fn legend_dot(ui: &mut egui::Ui, color: Color32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::same(3.0), color);
    ui.label(
        RichText::new(label)
            .size(11.5)
            .color(theme::text_muted()),
    );
    ui.add_space(10.0);
}

fn stat_cell(ui: &mut egui::Ui, label: &str, value: &str, accent: bool, tip: &str) {
    ui.vertical_centered(|ui| {
        let r = ui.label(
            RichText::new(label)
                .size(12.0)
                .color(theme::text_muted()),
        );
        if !tip.is_empty() {
            r.on_hover_text(tip);
        }
        ui.add_space(4.0);
        let r2 = ui.label(
            RichText::new(value)
                .size(18.0)
                .strong()
                .color(if accent {
                    theme::period_day()
                } else {
                    theme::text()
                }),
        );
        if !tip.is_empty() {
            r2.on_hover_text(tip);
        }
    });
}

fn guide_line(ui: &mut egui::Ui, title: &str, body: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("· {title}"))
                .size(12.5)
                .strong()
                .color(theme::shell_women_pill_fg()),
        );
        ui.label(RichText::new(body).size(12.5).color(theme::text()));
    });
    ui.add_space(4.0);
}

fn fmt_days(n: Option<u32>) -> String {
    n.map(|v| format!("{v} 天")).unwrap_or_else(|| "—".into())
}

/// 绘制生理期面板（旧全页布局；专属页已改用日历化入口）。
#[allow(dead_code)]
pub fn show(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    period: &mut PeriodUi,
    memos: &[MemoView],
    status_line: &mut String,
) {
    let alias = svc.session_alias();
    let alias_show = if alias.is_empty() { "当前用户" } else { alias };
    let avail_h = ui.available_height();
    let avail_w = ui.available_width();
    let body_h = (avail_h * 0.58).clamp(260.0, 520.0);

    egui::ScrollArea::vertical()
        .id_source("period_view_scroll")
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
                            RichText::new("女性私密 · 生理期记录")
                                .size(20.0)
                                .strong()
                                .color(theme::text()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            Frame::none()
                                .fill(theme::shell_women_pill())
                                .rounding(Rounding::same(10.0))
                                .inner_margin(Margin::symmetric(10.0, 4.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!("已加密 · 仅 {alias_show} 可见"))
                                            .size(12.0)
                                            .strong()
                                            .color(theme::shell_women_pill_fg()),
                                    );
                                });
                        });
                    });
                    ui.add_space(12.0);

                    // —— 关怀提示（本轮已读后仍温和展示，不再闪烁）——
                    if let Some(care) = care_remind(svc, memos) {
                        Frame::none()
                            .fill(theme::shell_women_pill())
                            .stroke(Stroke::new(1.0, theme::period_day().linear_multiply(0.45)))
                            .rounding(Rounding::same(12.0))
                            .inner_margin(Margin::symmetric(14.0, 12.0))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("♡  {}", care.title))
                                        .size(15.0)
                                        .strong()
                                        .color(theme::shell_women_pill_fg()),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(&care.body)
                                        .size(13.0)
                                        .color(theme::shell_women_pill_fg()),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("预测仅供参考，请以自身感受为准。")
                                        .size(11.5)
                                        .color(theme::text_muted()),
                                );
                            });
                        ui.add_space(10.0);
                    }

                    let pid = svc.current_person_id();
                    let mut cfg = pid
                        .as_ref()
                        .and_then(|id| svc.get_cycle(id))
                        .unwrap_or_default();
                    let marked = period_marked_days(memos);
                    let stats = analyze_period_marks(&marked);

                    // 无手动覆盖时，用标记结果刷新展示用 cfg（不强制每次落盘，标记时已 sync）
                    if !cfg.manual_cycle || !cfg.manual_period || cfg.last_start.is_empty() {
                        let mut tmp = cfg.clone();
                        apply_mark_stats(&mut tmp, &stats);
                        if !cfg.manual_cycle {
                            cfg.cycle_days = tmp.cycle_days;
                        }
                        if !cfg.manual_period {
                            cfg.period_days = tmp.period_days;
                        }
                        if let Some(ls) = &stats.last_start {
                            cfg.last_start = ls.clone();
                        }
                    }

                    let next = next_expected(&cfg);
                    let next_label = match next {
                        Some(d) => {
                            let n = days_until(d);
                            if n >= 0 {
                                format!("{}（{}天后）", d.format("%m-%d"), n)
                            } else {
                                format!("{}（已过{}天）", d.format("%m-%d"), -n)
                            }
                        }
                        None => "—".into(),
                    };
                    let cycle_label = if cfg.manual_cycle && cfg.cycle_days >= 15 {
                        format!("{} 天·手调", cfg.cycle_days)
                    } else {
                        fmt_days(stats.avg_cycle.or_else(|| {
                            if cfg.cycle_days >= 15 {
                                Some(cfg.cycle_days)
                            } else {
                                None
                            }
                        }))
                    };
                    let period_label = if cfg.manual_period && cfg.period_days >= 1 {
                        format!("{} 天·手调", cfg.period_days)
                    } else {
                        fmt_days(
                            stats
                                .avg_period
                                .or(stats.last_period)
                                .or_else(|| {
                                    if cfg.period_days >= 1 {
                                        Some(cfg.period_days)
                                    } else {
                                        None
                                    }
                                }),
                        )
                    };
                    let last_cycle_label = stats
                        .last_gap
                        .map(|g| format!("{g} 天"))
                        .unwrap_or_else(|| {
                            stats
                                .last_start
                                .as_ref()
                                .filter(|s| s.len() >= 10)
                                .map(|s| s[5..].to_string())
                                .unwrap_or_else(|| "—".into())
                        });

                    // —— 摘要 ——
                    Frame::none()
                        .fill(theme::period_pink_bg())
                        .stroke(Stroke::new(1.0, theme::border()))
                        .rounding(Rounding::same(12.0))
                        .inner_margin(Margin::symmetric(14.0, 12.0))
                        .show(ui, |ui| {
                            ui.columns(4, |cols| {
                                cols[0].vertical(|ui| {
                                    stat_cell(
                                        ui,
                                        "下次预计",
                                        &next_label,
                                        true,
                                        "至少标记两次完整经期的日期后，由「最近开始日 + 平均周期」自动推算。仅供参考。",
                                    );
                                });
                                cols[1].vertical(|ui| {
                                    stat_cell(
                                        ui,
                                        "平均周期",
                                        &cycle_label,
                                        false,
                                        "相邻两次经期开始日之间的间隔平均值（忽略异常长短）。月经不调时可在右侧特例调整。",
                                    );
                                });
                                cols[2].vertical(|ui| {
                                    stat_cell(
                                        ui,
                                        "最近周期",
                                        &last_cycle_label,
                                        false,
                                        "最近两次经期开始之间的间隔；不足两次时显示最近开始月-日。",
                                    );
                                });
                                cols[3].vertical(|ui| {
                                    stat_cell(
                                        ui,
                                        "经期长度",
                                        &period_label,
                                        false,
                                        "根据你标出的连续经期日自动平均。一次经期里逐日标记即可。",
                                    );
                                });
                            });
                        });

                    ui.add_space(8.0);

                    // —— 参考声明 ——
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

                    ui.add_space(10.0);

                    // —— 怎么用 ——
                    Frame::none()
                        .fill(theme::panel())
                        .stroke(Stroke::new(1.0, theme::border()))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(12.0, 8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let title = if period.show_guide {
                                    "▾ 怎么记录与推算（点击收起）"
                                } else {
                                    "▸ 怎么记录与推算（点击展开）"
                                };
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(title)
                                                .size(13.0)
                                                .strong()
                                                .color(theme::shell_women_pill_fg()),
                                        )
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(Stroke::NONE),
                                    )
                                    .clicked()
                                {
                                    period.show_guide = !period.show_guide;
                                }
                            });
                            if period.show_guide {
                                ui.add_space(6.0);
                                guide_line(
                                    ui,
                                    "标记经期日：",
                                    "在日历上把来潮的每一天都标出来（右键，或选中后再点一次）。不要只标第一天——连续标几天，系统才能知道经期有多长。",
                                );
                                guide_line(
                                    ui,
                                    "至少两次：",
                                    "请标记至少两次历史经期（例如上次 3/1～3/5、这次 3/29～4/2）。满两次后自动算周期与下次预计。",
                                );
                                guide_line(
                                    ui,
                                    "经期长度：",
                                    "同一次经期里连续标记的天数。多次取平均。",
                                );
                                guide_line(
                                    ui,
                                    "周期：",
                                    "两次经期「第一天」之间的间隔天数，取历史平均。",
                                );
                                guide_line(
                                    ui,
                                    "特例调整：",
                                    "月经不调或近期有特殊情况时，可在右侧临时改周期/经期长度；仍只是参考。",
                                );
                                guide_line(
                                    ui,
                                    "隐私：",
                                    "推算参数仅本机；经期日与备注写入「女性私密」，仅当前身份可解密。",
                                );
                            }
                        });

                    ui.add_space(10.0);

                    // —— 情景提示 ——
                    Frame::none()
                        .fill(theme::period_pink_bg())
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(14.0, 10.0))
                        .show(ui, |ui| {
                            let tip = situation_tip(&stats, next);
                            ui.label(
                                RichText::new(tip)
                                    .size(13.0)
                                    .color(theme::shell_women_pill_fg()),
                            );
                        });

                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        let cal_w = (ui.available_width() * 0.58).clamp(280.0, 520.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(cal_w, body_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_height(body_h);
                                show_calendar_block(
                                    ui,
                                    svc,
                                    period,
                                    memos,
                                    &marked,
                                    &mut cfg,
                                    status_line,
                                );
                            },
                        );

                        ui.add_space(12.0);

                        // 右侧全部展开，高度随内容走；只靠外层滚动
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                show_side_panel(
                                    ui,
                                    svc,
                                    period,
                                    memos,
                                    &marked,
                                    &stats,
                                    &mut cfg,
                                    status_line,
                                );
                            },
                        );
                    });
                });
        });
}

fn situation_tip(stats: &PeriodMarkStats, next: Option<NaiveDate>) -> String {
    let n_ep = stats.episodes.len();
    if n_ep == 0 {
        return "请在日历上标记经期日：把每次来潮的每一天都标上。至少完整标记两次历史经期后，即可自动推算周期与下次开始日。"
            .into();
    }
    if n_ep == 1 {
        let ep = &stats.episodes[0];
        return format!(
            "已记录 1 次经期（{} 起，共 {} 天）。请再标记另一次经期的各天，即可自动计算周期与下次预计。",
            ep.start, ep.days
        );
    }
    if let Some(d) = next {
        let n = days_until(d);
        if n >= 0 {
            format!(
                "参考预计：经期约 {} 开始（约 {} 天后）。粉=已标记/预测经期，绿=易孕参考。{}",
                d.format("%m-%d"),
                n,
                "请记住仅为参考，不能确保准确。"
            )
        } else {
            "按推算可能已进入经期窗口：请对照日历补标实际经期日，以便修正下次预计。".into()
        }
    } else {
        format!(
            "已有 {n_ep} 次经期记录，但间隔异常或不足，暂无法推算下次。可继续标记，或在右侧做特例调整。"
        )
    }
}

fn show_calendar_block(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    period: &mut PeriodUi,
    memos: &[MemoView],
    marked: &[String],
    cfg: &mut CycleConfig,
    status_line: &mut String,
) {
    ui.horizontal(|ui| {
        if ui.button("‹").clicked() {
            if period.month == 1 {
                period.month = 12;
                period.year -= 1;
            } else {
                period.month -= 1;
            }
        }
        ui.label(
            RichText::new(format!("{}年{}月", period.year, period.month))
                .strong()
                .size(15.0)
                .color(theme::text()),
        );
        if ui.button("›").clicked() {
            if period.month == 12 {
                period.month = 1;
                period.year += 1;
            } else {
                period.month += 1;
            }
        }
        if theme::ghost_button(ui, "今天").clicked() {
            let t = chrono::Local::now().date_naive();
            period.year = t.year();
            period.month = t.month();
            period.selected_day = Some(today_ymd());
            load_note_draft(period, memos);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            legend_dot(ui, theme::period_day(), "经期");
            legend_dot(ui, theme::period_fertile(), "易孕");
        });
    });
    ui.add_space(8.0);

    if theme::primary_button(ui, "标记今天为经期日").clicked() {
        let ymd = today_ymd();
        toggle_period_day(svc, &ymd, memos, cfg, status_line, &mut period.tip);
        period.selected_day = Some(ymd);
        load_note_draft(period, memos);
        let t = chrono::Local::now().date_naive();
        period.year = t.year();
        period.month = t.month();
    }
    ui.add_space(8.0);

    let first = NaiveDate::from_ymd_opt(period.year, period.month, 1);
    let Some(first) = first else {
        return;
    };
    let days_in_month = if period.month == 12 {
        NaiveDate::from_ymd_opt(period.year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(period.year, period.month + 1, 1)
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
                let ymd = format!("{:04}-{:02}-{:02}", period.year, period.month, day);
                let is_marked = is_marked_day(marked, &ymd);
                let is_predicted = !is_marked && cfg.is_predicted_period_day(&ymd);
                let is_period = is_marked || is_predicted;
                let is_today = ymd == today_ymd();
                let is_sel = period.selected_day.as_deref() == Some(ymd.as_str());
                let has_note = !notes_on_day(memos, &ymd).is_empty();

                let is_fertile = !is_period && cfg.is_fertile_day(&ymd);
                let is_ovulation = !is_period && cfg.is_ovulation_day(&ymd);

                let fill = if is_marked {
                    theme::period_day()
                } else if is_predicted {
                    theme::period_day().linear_multiply(0.55)
                } else if is_ovulation {
                    theme::period_fertile()
                } else if is_fertile {
                    theme::period_fertile().linear_multiply(0.72)
                } else if is_sel {
                    theme::shell_accent_soft()
                } else {
                    theme::panel()
                };
                let stroke = if is_sel {
                    Stroke::new(2.0, theme::shell_accent())
                } else if is_today {
                    Stroke::new(1.5, theme::shell_accent())
                } else if is_marked {
                    Stroke::new(1.5, theme::period_day())
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
                    if is_period {
                        Color32::WHITE
                    } else {
                        theme::text()
                    },
                );
                if has_note {
                    ui.painter().circle_filled(
                        egui::pos2(rect.center().x, rect.bottom() - 6.0),
                        2.0,
                        if is_period {
                            Color32::WHITE
                        } else {
                            theme::shell_accent()
                        },
                    );
                }

                if resp.clicked() {
                    let same = period.selected_day.as_deref() == Some(ymd.as_str());
                    period.selected_day = Some(ymd.clone());
                    load_note_draft(period, memos);
                    if same {
                        toggle_period_day(svc, &ymd, memos, cfg, status_line, &mut period.tip);
                    }
                }
                if resp.secondary_clicked() {
                    toggle_period_day(svc, &ymd, memos, cfg, status_line, &mut period.tip);
                    period.selected_day = Some(ymd);
                    load_note_draft(period, memos);
                }
                day += 1;
            }
        });
    }

    ui.add_space(6.0);
    ui.label(
        RichText::new("左键选日写备注；再点一次或右键 = 标记/取消经期日。深粉=已标记，浅粉=预测参考")
            .size(11.5)
            .color(theme::text_muted()),
    );
}

#[allow(clippy::too_many_arguments)]
fn show_side_panel(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    period: &mut PeriodUi,
    memos: &[MemoView],
    marked: &[String],
    stats: &PeriodMarkStats,
    cfg: &mut CycleConfig,
    status_line: &mut String,
) {
    Frame::none()
        .fill(theme::panel())
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(12.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("自动推算")
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.label(
                RichText::new("由日历上的经期日自动计算，无需手填周期/经期天数。")
                    .size(11.5)
                    .color(theme::text_muted()),
            );
            ui.add_space(6.0);

            let ep_n = stats.episodes.len();
            ui.label(
                RichText::new(format!(
                    "已识别 {ep_n} 次经期 · 已标记 {} 天",
                    marked.len()
                ))
                .size(12.5)
                .color(theme::text()),
            );

            if let Some(c) = stats.avg_cycle {
                ui.label(
                    RichText::new(format!("推算周期：约 {c} 天"))
                        .size(12.5)
                        .color(theme::shell_women_pill_fg()),
                );
            } else {
                ui.label(
                    RichText::new("推算周期：需再完整标记至少 1 次经期")
                        .size(12.5)
                        .color(theme::text_muted()),
                );
            }
            if let Some(p) = stats.avg_period.or(stats.last_period) {
                ui.label(
                    RichText::new(format!("推算经期长度：约 {p} 天"))
                        .size(12.5)
                        .color(theme::shell_women_pill_fg()),
                );
            } else {
                ui.label(
                    RichText::new("推算经期长度：请逐日标记来潮日")
                        .size(12.5)
                        .color(theme::text_muted()),
                );
            }

            ui.add_space(8.0);
            let ov_title = if period.show_override {
                "▾ 特例调整（月经不调等）"
            } else {
                "▸ 特例调整（月经不调等）"
            };
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(ov_title)
                            .size(12.5)
                            .strong()
                            .color(theme::text()),
                    )
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE),
                )
                .clicked()
            {
                period.show_override = !period.show_override;
            }

            if period.show_override {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("仅在特殊情况使用。覆盖后将不再随新标记自动更新该项。")
                        .size(11.5)
                        .color(theme::text_muted()),
                );
                ui.add_space(4.0);

                let mut manual_c = cfg.manual_cycle;
                if ui
                    .checkbox(&mut manual_c, "手动指定周期天数")
                    .changed()
                {
                    cfg.manual_cycle = manual_c;
                    if !manual_c {
                        apply_mark_stats(cfg, stats);
                    } else if cfg.cycle_days < 15 {
                        cfg.cycle_days = stats.avg_cycle.unwrap_or(28);
                    }
                    if let Ok(pid) = ensure_cycle_person(svc) {
                        let _ = svc.set_cycle(&pid, cfg.clone());
                        *status_line = if cfg.manual_cycle {
                            "已启用手动周期（仅参考）".into()
                        } else {
                            "已恢复自动推算周期".into()
                        };
                    }
                }
                if cfg.manual_cycle {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("周期").small().color(theme::text_muted()));
                        let mut cycle = cfg.cycle_days.max(15) as i32;
                        let r = ui.add(
                            egui::DragValue::new(&mut cycle)
                                .clamp_range(15..=45)
                                .suffix(" 天"),
                        );
                        if r.changed() {
                            cfg.cycle_days = cycle as u32;
                            if let Ok(pid) = ensure_cycle_person(svc) {
                                let _ = svc.set_cycle(&pid, cfg.clone());
                                *status_line = "已更新手动周期（仅参考）".into();
                            }
                        }
                    });
                }

                let mut manual_p = cfg.manual_period;
                if ui
                    .checkbox(&mut manual_p, "手动指定经期长度")
                    .changed()
                {
                    cfg.manual_period = manual_p;
                    if !manual_p {
                        apply_mark_stats(cfg, stats);
                    } else if cfg.period_days < 1 {
                        cfg.period_days = stats.avg_period.or(stats.last_period).unwrap_or(5);
                    }
                    if let Ok(pid) = ensure_cycle_person(svc) {
                        let _ = svc.set_cycle(&pid, cfg.clone());
                        *status_line = if cfg.manual_period {
                            "已启用手动经期长度（仅参考）".into()
                        } else {
                            "已恢复自动推算经期长度".into()
                        };
                    }
                }
                if cfg.manual_period {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("经期").small().color(theme::text_muted()));
                        let mut plen = cfg.period_days.max(1) as i32;
                        let r = ui.add(
                            egui::DragValue::new(&mut plen)
                                .clamp_range(1..=10)
                                .suffix(" 天"),
                        );
                        if r.changed() {
                            cfg.period_days = plen as u32;
                            if let Ok(pid) = ensure_cycle_person(svc) {
                                let _ = svc.set_cycle(&pid, cfg.clone());
                                *status_line = "已更新手动经期长度（仅参考）".into();
                            }
                        }
                    });
                }

                ui.add_space(4.0);
                ui.label(
                    RichText::new(REF_DISCLAIMER)
                        .size(11.0)
                        .color(theme::warn()),
                );
            }
        });

    ui.add_space(10.0);

    Frame::none()
        .fill(theme::panel())
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(12.0))
        .show(ui, |ui| {
            let day_label = period
                .selected_day
                .clone()
                .unwrap_or_else(|| "未选择日期".into());
            ui.label(
                RichText::new(format!("当日备注 · {day_label}"))
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("可记录感受、症状等，仅本人可见。")
                    .size(11.5)
                    .color(theme::text_muted()),
            );
            ui.add_space(6.0);
            let enabled = period.selected_day.is_some();
            ui.add_enabled_ui(enabled, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut period.note_draft)
                        .desired_width(ui.available_width())
                        .desired_rows(4)
                        .hint_text(theme::hint("例如：轻微腹痛，多喝温水…")),
                );
                ui.add_space(6.0);
                if theme::success_button(ui, "保存备注").clicked() {
                    if let Some(ymd) = period.selected_day.clone() {
                        save_day_note(svc, &ymd, &period.note_draft, status_line);
                    }
                }
            });
        });

    ui.add_space(10.0);

    ui.label(
        RichText::new("已识别的经期")
            .strong()
            .size(14.0)
            .color(theme::text()),
    );
    ui.add_space(4.0);
    let recent: Vec<_> = stats.episodes.iter().rev().take(8).collect();
    if recent.is_empty() {
        ui.label(theme::muted_label("暂无标记。右键日历日期，逐日标出经期。"));
    } else {
        for ep in recent {
            ui.horizontal(|ui| {
                if ui
                    .link(RichText::new(&ep.start).color(theme::period_day()))
                    .clicked()
                {
                    if let Some(dt) = parse_ymd(&ep.start) {
                        period.year = dt.year();
                        period.month = dt.month();
                    }
                    period.selected_day = Some(ep.start.clone());
                    load_note_draft(period, memos);
                }
                ui.label(
                    RichText::new(format!("共 {} 天", ep.days))
                        .small()
                        .color(theme::text_muted()),
                );
            });
        }
    }

    if !period.tip.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new(&period.tip).small().color(theme::warn()));
    }
}

fn load_note_draft(period: &mut PeriodUi, memos: &[MemoView]) {
    let Some(ymd) = period.selected_day.clone() else {
        period.note_draft.clear();
        return;
    };
    let notes = notes_on_day(memos, &ymd);
    if let Some(m) = notes.first() {
        period.note_draft = if m.content.is_empty() {
            m.title.clone()
        } else {
            m.content.clone()
        };
    } else {
        period.note_draft.clear();
    }
}

pub(crate) fn save_day_note(
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
        let line = body.lines().next().unwrap_or("私密备注").trim();
        if line.is_empty() {
            format!("备注 {ymd}")
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
            Ok(()) => *status_line = format!("已更新 {ymd} 备注"),
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
        memo_core::store::MemoPriority::Normal,
        0,
    ) {
        Ok(_) => *status_line = format!("已保存 {ymd} 备注"),
        Err(e) => *status_line = format!("保存失败: {e}"),
    }
}

pub(crate) fn toggle_period_day(
    svc: &Arc<MemoService>,
    ymd: &str,
    memos: &[MemoView],
    cfg: &mut CycleConfig,
    status_line: &mut String,
    tip: &mut String,
) {
    let pid = match ensure_cycle_person(svc) {
        Ok(id) => id,
        Err(e) => {
            *status_line = e;
            return;
        }
    };

    if let Some(m) = find_period_day_memo(memos, ymd) {
        match svc.delete(&m.id) {
            Ok(()) => {
                let mut marked = period_marked_days(memos);
                marked.retain(|d| d != ymd);
                sync_cfg_from_marks(svc, &pid, cfg, &marked);
                *status_line = format!("已取消 {ymd} 经期日");
                tip.clear();
            }
            Err(e) => *status_line = format!("取消失败: {e}"),
        }
        return;
    }

    // 只能标记今天及之前（未来日留给预测，不可人工标经期）
    let is_future = NaiveDate::parse_from_str(ymd, "%Y-%m-%d")
        .ok()
        .map(|d| d > chrono::Local::now().date_naive())
        .unwrap_or(true);
    if is_future {
        *status_line = "只能标记今天及之前的经期日，未来日请看预测".into();
        return;
    }

    match svc.add_full(
        "经期日",
        "",
        MemoVisibility::Private,
        MemoLifecycle::Permanent,
        MemoCategory::GenderPrivate,
        ymd,
        &[],
        memo_core::store::MemoPriority::Normal,
        0,
    ) {
        Ok(id) => {
            // 日历标记日不进到期提醒队列（避免误弹系统通知）
            let _ = svc.ack_remind(&id);
            let mut marked = period_marked_days(memos);
            if !marked.iter().any(|d| d == ymd) {
                marked.push(ymd.to_string());
                marked.sort();
            }
            sync_cfg_from_marks(svc, &pid, cfg, &marked);
            *status_line = format!("已标记经期日 {ymd}");
            tip.clear();
        }
        Err(e) => *status_line = format!("操作失败: {e}"),
    }
}
