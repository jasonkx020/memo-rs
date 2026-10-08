//! 主界面壳层：顶栏页签 + 内容区 + 右侧节点树。

use crate::calendar_view::{self, CalendarUi};
use crate::doc_editor;
use crate::gender_private_view::{self, GenderPrivateUi};
use crate::male_view;
use crate::memo_form::{self, FormMode, MemoFormState};
use crate::msg::BgMsg;
use crate::nav::NavItem;
use crate::period_view::{self, PeriodCareRemind};
use crate::theme;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::config::NodeRole;
use memo_core::disk::DiskSpace;
use memo_core::identity_keys::IdentityKeys;
use memo_core::service::{MemoService, MemoView};
use memo_core::store::{
    MemoCategory, MemoLifecycle, MemoPriority, MemoVisibility,
};
use memo_sync::{DiscoveredPeer, PeerStatus};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

/// 性别私密列表置顶入口（虚拟备忘，不入库）。
pub const HUB_GENDER_PRIVATE: &str = "hub:gender-private";
/// 新建草稿伪 id（已改用弹窗，仅兼容清理残留选中）。
pub const NEW_MEMO_DRAFT: &str = "new:draft";

pub fn is_private_hub(id: &str) -> bool {
    id == HUB_GENDER_PRIVATE
}

pub fn is_new_draft(id: &str) -> bool {
    id == NEW_MEMO_DRAFT
}

pub fn begin_new_memo(
    show_new_memo: &mut bool,
    editing: &mut bool,
    edit_form: &mut MemoFormState,
    nav: NavItem,
    calendar_ymd: Option<&str>,
) {
    use crate::memo_form::FormScope;
    let (cat, scope) = match nav {
        NavItem::Credentials => (MemoCategory::Credentials, FormScope::Credentials),
        NavItem::GenderPrivate => (MemoCategory::GenderPrivate, FormScope::GenderPrivate),
        _ => (MemoCategory::Todo, FormScope::Calendar),
    };
    let stamp = today_ymd();
    let prefill = if nav.is_calendar() {
        calendar_ymd.map(|d| format!("{d} 09:00"))
    } else {
        None
    };
    *edit_form = MemoFormState::begin_create(cat, &stamp, prefill.as_deref(), scope);
    *editing = false;
    *show_new_memo = true;
}

fn hub_memo_for(nav: NavItem) -> Option<MemoView> {
    if !nav.is_gender_private() {
        return None;
    }
    Some(MemoView {
        id: HUB_GENDER_PRIVATE.into(),
        title: "性别私密 · 关怀与健康".into(),
        content: "经期与体检可分开查看，不需要的可关闭；其下可新建其它私密备忘。".into(),
        deleted: false,
        version: 0,
        node_id: String::new(),
        visibility: MemoVisibility::Private,
        owner_fp: String::new(),
        modified_at: String::new(),
        lifecycle: MemoLifecycle::Permanent,
        deleted_at: String::new(),
        category: MemoCategory::GenderPrivate,
        due_date: String::new(),
        end_date: String::new(),
        remind_before_days: 0,
        remind_seen_for: String::new(),
        done: false,
        tags: vec!["专属".into()],
        priority: MemoPriority::Normal,
    })
}

/// 日历产生的系统备忘不出现在私密分类列表（仍保留在专属页内使用）。
pub(crate) fn is_private_calendar_memo(m: &MemoView) -> bool {
    if !m.category.is_gender_private() {
        return false;
    }
    m.title == "经期日"
        || m.title == "经期"
        || m.title.contains("经期开始")
        || m.title.starts_with("备注 ")
        || m.title.starts_with("日记 ")
}

fn select_gender_hub(selected: &mut Option<String>, editing: &mut bool) {
    *selected = Some(HUB_GENDER_PRIVATE.into());
    *editing = false;
}

pub struct ShellUi {
    pub nav: NavItem,
    pub gender: GenderPrivateUi,
    pub show_nodes: bool,
    pub cal: CalendarUi,
}

impl Default for ShellUi {
    fn default() -> Self {
        Self {
            nav: NavItem::Calendar,
            gender: GenderPrivateUi::default(),
            show_nodes: false,
            cal: CalendarUi::default(),
        }
    }
}

/// 壳层交互回传。
#[derive(Debug, Clone, Copy, Default)]
pub struct ShellAction {
    pub switch: bool,
    pub sync: bool,
    pub open_new_memo: bool,
    pub backup_dirty: bool,
}

fn status_dot_color(st: PeerStatus) -> Color32 {
    match st {
        PeerStatus::Online => theme::success(),
        PeerStatus::Stale => theme::warn(),
        PeerStatus::Offline | PeerStatus::Undiscovered => theme::text_muted(),
    }
}

fn peer_backup_checked(targets: &[String], id: &str) -> bool {
    targets.is_empty() || targets.iter().any(|t| t == id)
}

fn set_peer_backup(targets: &mut Vec<String>, id: &str, want: bool, all_ids: &[String]) {
    if want {
        if targets.is_empty() {
            return;
        }
        if !targets.iter().any(|t| t == id) {
            targets.push(id.to_string());
        }
        if !all_ids.is_empty() && all_ids.iter().all(|x| targets.iter().any(|t| t == x)) {
            targets.clear();
        }
    } else if targets.is_empty() {
        *targets = all_ids
            .iter()
            .filter(|x| *x != id)
            .cloned()
            .collect();
    } else {
        targets.retain(|t| t != id);
    }
}

fn show_nodes_panel(
    ui: &mut egui::Ui,
    discovered: &[DiscoveredPeer],
    local_node_id: &str,
    node_role: NodeRole,
    node_visible: bool,
    lan_discovery: bool,
    local_disk: DiskSpace,
    backup_targets: &mut Vec<String>,
    online_count: usize,
    action: &mut ShellAction,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("节点")
                .size(14.0)
                .strong()
                .color(theme::text()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let _ = pill(
                ui,
                theme::shell_chip_green(),
                theme::shell_chip_green_fg(),
                &format!("{online_count} 节点在线"),
                22.0,
            );
        });
    });
    ui.add_space(6.0);
    ui.label(
        RichText::new(if lan_discovery {
            "局域网发现已开"
        } else {
            "局域网发现已关（仅手填对端）"
        })
        .size(11.0)
        .color(theme::text_muted()),
    );
    ui.add_space(8.0);

    ui.label(
        RichText::new("本机")
            .size(11.5)
            .strong()
            .color(theme::text_muted()),
    );
    ui.add_space(4.0);
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::same(8.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(local_node_id)
                    .size(12.5)
                    .strong()
                    .color(theme::text()),
            );
            let vis = if node_role.is_master() {
                if node_visible {
                    "主机 · 显示中"
                } else {
                    "主机 · 已隐藏"
                }
            } else {
                "从机 · 不广播"
            };
            ui.label(
                RichText::new(vis)
                    .size(11.0)
                    .color(theme::text_muted()),
            );
            if local_disk.total_bytes > 0 {
                ui.label(
                    RichText::new(format!("磁盘 {}", local_disk.format_pair()))
                        .size(11.0)
                        .color(if local_disk.is_low() {
                            theme::warn()
                        } else {
                            theme::text_muted()
                        }),
                );
            }
        });

    ui.add_space(10.0);
    ui.label(
        RichText::new("对端")
            .size(11.5)
            .strong()
            .color(theme::text_muted()),
    );
    ui.label(
        RichText::new("空勾选=推送到所有可托管节点")
            .size(10.5)
            .color(theme::text_muted()),
    );
    ui.add_space(4.0);

    let hostable: Vec<String> = discovered
        .iter()
        .filter(|p| p.accept_backup)
        .map(|p| p.node_id.clone())
        .collect();

    egui::ScrollArea::vertical()
        .id_source("shell_nodes_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if discovered.is_empty() {
                ui.label(
                    RichText::new("尚未发现其他节点")
                        .size(12.0)
                        .color(theme::text_muted()),
                );
                return;
            }
            for p in discovered {
                let name = if p.alias.trim().is_empty() {
                    p.node_id.clone()
                } else {
                    p.alias.clone()
                };
                Frame::none()
                    .fill(theme::card())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(Margin::same(8.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (dot, _) =
                                ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                            ui.painter().circle_filled(
                                dot.center(),
                                4.0,
                                status_dot_color(p.status),
                            );
                            ui.label(
                                RichText::new(&name)
                                    .size(12.5)
                                    .strong()
                                    .color(theme::text()),
                            );
                        });
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                p.status.label(),
                                if p.connected { "已连接" } else { "未连接" }
                            ))
                            .size(11.0)
                            .color(theme::text_muted()),
                        );
                        if !p.addr.is_empty() {
                            ui.label(
                                RichText::new(&p.addr)
                                    .size(10.5)
                                    .color(theme::text_muted()),
                            );
                        }
                        if let (Some(free), Some(total)) = (p.disk_free, p.disk_total) {
                            if total > 0 {
                                ui.label(
                                    RichText::new(format!(
                                        "剩余 {} / 共 {}",
                                        memo_core::disk::format_bytes(free),
                                        memo_core::disk::format_bytes(total)
                                    ))
                                    .size(10.5)
                                    .color(theme::text_muted()),
                                );
                            }
                        }
                        let mut on = peer_backup_checked(backup_targets, &p.node_id);
                        ui.add_enabled_ui(p.accept_backup, |ui| {
                            if ui
                                .checkbox(&mut on, "托管到此节点")
                                .on_disabled_hover_text("对端不接受托管")
                                .changed()
                            {
                                set_peer_backup(
                                    backup_targets,
                                    &p.node_id,
                                    on,
                                    &hostable,
                                );
                                action.backup_dirty = true;
                            }
                        });
                        if !p.accept_backup {
                            ui.label(
                                RichText::new("对端不接受托管")
                                    .size(10.5)
                                    .color(theme::text_muted()),
                            );
                        }
                    });
                ui.add_space(6.0);
            }
        });
}

fn today_ymd() -> String {
    chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string()
}

fn avatar_letter(alias: &str) -> String {
    let ch = alias
        .chars()
        .find(|c| !c.is_whitespace())
        .unwrap_or('用');
    ch.to_uppercase().to_string()
}

fn filter_memos(nav: NavItem, search: &str, list: &[MemoView]) -> Vec<MemoView> {
    let q = search.trim().to_lowercase();
    let mut out: Vec<MemoView> = list
        .iter()
        .filter(|m| {
            let cat_ok = match nav {
                NavItem::Calendar => {
                    m.category.canonical() != MemoCategory::Credentials
                        && !m.category.is_gender_private()
                }
                NavItem::Credentials => m.category.canonical() == MemoCategory::Credentials,
                NavItem::Trash => false,
                NavItem::GenderPrivate => m.category.is_gender_private(),
            };
            if !cat_ok {
                return false;
            }
            if nav.is_gender_private() && is_private_calendar_memo(m) {
                return false;
            }
            if q.is_empty() {
                return true;
            }
            let tags = m.tags.join(" ").to_lowercase();
            m.title.to_lowercase().contains(&q)
                || m.content.to_lowercase().contains(&q)
                || tags.contains(&q)
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| b.version.cmp(&a.version));
    if let Some(hub) = hub_memo_for(nav) {
        out.insert(0, hub);
    }
    out
}

fn count_category(list: &[MemoView], cat: MemoCategory) -> usize {
    list.iter()
        .filter(|m| m.category.canonical() == cat)
        .count()
}

#[allow(dead_code)]
fn count_due_today(list: &[MemoView]) -> usize {
    list.iter()
        .filter(|m| memo_core::due_is_due_today(&m.due_date) && !m.category.is_gender_private())
        .count()
}

fn parse_tags(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c == '，' || c == ';' || c == '；' || c.is_whitespace())
        .map(|t| t.trim().trim_start_matches('#').trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn format_tags(tags: &[String]) -> String {
    tags.iter()
        .map(|t| {
            if t.starts_with('#') {
                t.clone()
            } else {
                format!("#{t}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_due_mmdd(due: &str) -> String {
    memo_core::display_due(due)
}

fn memo_is_overdue(m: &MemoView) -> bool {
    memo_core::is_overdue(&m.due_date, &m.end_date, m.done)
}

fn memo_is_due_today(m: &MemoView) -> bool {
    memo_core::is_due_today(&m.due_date, &m.end_date, m.done)
}

const PRIVATE_DUE_SOON_DAYS: i64 = 3;

fn memo_is_due_soon(m: &MemoView) -> bool {
    memo_core::is_due_soon(&m.due_date, &m.end_date, m.done, PRIVATE_DUE_SOON_DAYS)
}

/// 私密列表：过期 / 今天到期 / 即将到期预警条。
fn show_private_warn_banner(
    ui: &mut egui::Ui,
    gender_ui: &mut GenderPrivateUi,
    overdue: &[&MemoView],
    due_today_n: usize,
    due_soon_n: usize,
    selected: &mut Option<String>,
    title_draft: &mut String,
    body_draft: &mut String,
    visibility_draft: &mut MemoVisibility,
    category_draft: &mut MemoCategory,
    due_date_draft: &mut String,
    end_date_draft: &mut String,
    tags_draft: &mut String,
    done_draft: &mut bool,
    priority_draft: &mut MemoPriority,
    editing: &mut bool,
    memo_doc: &mut doc_editor::Doc,
    edit_form: &mut MemoFormState,
) {
    let n_od = overdue.len();
    let mut parts = Vec::new();
    if n_od > 0 {
        parts.push(format!("{n_od} 条已过期"));
    }
    if due_today_n > 0 {
        parts.push(format!("{due_today_n} 条今天到期"));
    }
    if due_soon_n > 0 {
        parts.push(format!("{due_soon_n} 条即将到期"));
    }
    if parts.is_empty() {
        return;
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
                        gender_ui.focus_overdue = true;
                        if let Some(m) = overdue.first() {
                            load_selection(
                                m,
                                selected,
                                title_draft,
                                body_draft,
                                visibility_draft,
                                category_draft,
                                due_date_draft,
                                end_date_draft,
                                tags_draft,
                                done_draft,
                                priority_draft,
                                editing,
                                memo_doc,
                                edit_form,
                            );
                            *editing = false;
                        }
                    }
                });
            });
        });
}

fn private_section_header(ui: &mut egui::Ui, title: &str, n: usize, fg: Color32, bg: Color32) {
    Frame::none()
        .fill(bg)
        .rounding(Rounding::same(6.0))
        .inner_margin(Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(12.0).strong().color(fg));
                ui.label(RichText::new(format!("{n}")).size(11.0).color(fg));
            });
        });
    ui.add_space(4.0);
}

fn format_auto_lock(remaining: Duration) -> String {
    let secs = remaining.as_secs();
    let m = secs / 60;
    let s = secs % 60;
    format!("{m:02}:{s:02}")
}

fn pill(ui: &mut egui::Ui, bg: Color32, fg: Color32, label: &str, h: f32) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let text_w = ui.fonts(|f| {
        f.layout_no_wrap(label.to_owned(), font.clone(), fg)
            .size()
            .x
    });
    let pad_x = 10.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(text_w + pad_x * 2.0, h), Sense::click());
    ui.painter()
        .rect(rect, Rounding::same(h * 0.5), bg, Stroke::NONE);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        fg,
    );
    resp
}

#[derive(Clone, Copy)]
enum TopIcon {
    Sync,
    Nodes,
    Settings,
    Lock,
}

fn paint_top_icon(painter: &egui::Painter, center: egui::Pos2, kind: TopIcon, color: Color32) {
    match kind {
        TopIcon::Sync => {
            // 双向弧箭头（同步）
            let r = 5.5;
            painter.circle_stroke(center, r, Stroke::new(1.6, color));
            painter.line_segment(
                [
                    egui::pos2(center.x + r * 0.15, center.y - r - 1.0),
                    egui::pos2(center.x + r + 1.2, center.y - r * 0.2),
                ],
                Stroke::new(1.6, color),
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - r * 0.15, center.y + r + 1.0),
                    egui::pos2(center.x - r - 1.2, center.y + r * 0.2),
                ],
                Stroke::new(1.6, color),
            );
        }
        TopIcon::Nodes => {
            // 三节点小树
            let top = egui::pos2(center.x, center.y - 5.5);
            let bl = egui::pos2(center.x - 5.5, center.y + 5.0);
            let br = egui::pos2(center.x + 5.5, center.y + 5.0);
            painter.line_segment([top, bl], Stroke::new(1.5, color));
            painter.line_segment([top, br], Stroke::new(1.5, color));
            painter.circle_filled(top, 2.2, color);
            painter.circle_filled(bl, 2.2, color);
            painter.circle_filled(br, 2.2, color);
        }
        TopIcon::Settings => {
            // 简易齿轮：外圆 + 中心孔 + 十字刻度
            painter.circle_stroke(center, 6.0, Stroke::new(1.5, color));
            painter.circle_stroke(center, 2.2, Stroke::new(1.4, color));
            for (dx, dy) in [
                (0.0, -7.2),
                (0.0, 7.2),
                (-7.2, 0.0),
                (7.2, 0.0),
                (5.1, 5.1),
                (-5.1, 5.1),
                (5.1, -5.1),
                (-5.1, -5.1),
            ] {
                painter.line_segment(
                    [
                        egui::pos2(center.x + dx * 0.55, center.y + dy * 0.55),
                        egui::pos2(center.x + dx, center.y + dy),
                    ],
                    Stroke::new(1.5, color),
                );
            }
        }
        TopIcon::Lock => {
            let body = egui::Rect::from_center_size(egui::pos2(center.x, center.y + 1.5), Vec2::new(9.0, 7.0));
            painter.rect_stroke(body, 1.5, Stroke::new(1.5, color));
            painter.circle_stroke(
                egui::pos2(center.x, center.y - 2.5),
                3.2,
                Stroke::new(1.5, color),
            );
        }
    }
}

fn top_icon_color(kind: TopIcon) -> Color32 {
    match kind {
        TopIcon::Sync => theme::success(),
        TopIcon::Nodes => theme::nav_icon_color(NavItem::Calendar),
        TopIcon::Settings => theme::category_icon_color(MemoCategory::Credentials),
        TopIcon::Lock => theme::warn(),
    }
}

fn top_icon_button(ui: &mut egui::Ui, kind: TopIcon, label: &str) -> egui::Response {
    top_icon_button_on(ui, kind, label, false)
}

fn top_icon_button_on(
    ui: &mut egui::Ui,
    kind: TopIcon,
    label: &str,
    on: bool,
) -> egui::Response {
    let font = egui::FontId::proportional(13.0);
    let text_w = ui.fonts(|f| {
        f.layout_no_wrap(label.to_owned(), font.clone(), theme::text())
            .size()
            .x
    });
    let icon_w = 16.0;
    let gap = 5.0;
    let pad_x = 8.0;
    let h = 24.0;
    let w = pad_x * 2.0 + icon_w + gap + text_w;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let icon_fg = top_icon_color(kind);
    let label_fg = if on || resp.hovered() {
        icon_fg
    } else {
        theme::text()
    };
    let icon_c = egui::pos2(rect.left() + pad_x + icon_w * 0.5, rect.center().y);
    paint_top_icon(ui.painter(), icon_c, kind, icon_fg);
    ui.painter().text(
        egui::pos2(rect.left() + pad_x + icon_w + gap, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        label_fg,
    );
    resp
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

fn list_category_icon(ui: &mut egui::Ui, cat: MemoCategory, done: bool) -> egui::Response {
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

fn sync_mark(ui: &mut egui::Ui, draft_empty_title: bool) {
    let (color, glyph) = if draft_empty_title {
        (theme::warn(), "●")
    } else {
        (theme::success(), "✓")
    };
    ui.label(RichText::new(glyph).size(13.0).color(color));
}

fn show_status_strip(ui: &mut egui::Ui, online: usize, connected: usize, pending: usize) {
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("在线节点: {online}/{connected}"))
                        .size(12.0)
                        .color(theme::text_muted()),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!("待同步: {pending} 条"))
                        .size(12.0)
                        .color(theme::text_muted()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new("数据本地加密 · 仅同步已授权节点")
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                });
            });
        });
}

/// 绘制主壳；返回切换身份 / 同步请求。
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    svc: &Arc<MemoService>,
    shell: &mut ShellUi,
    search: &mut String,
    selected: &mut Option<String>,
    title_draft: &mut String,
    body_draft: &mut String,
    visibility_draft: &mut MemoVisibility,
    category_draft: &mut MemoCategory,
    due_date_draft: &mut String,
    end_date_draft: &mut String,
    tags_draft: &mut String,
    done_draft: &mut bool,
    priority_draft: &mut MemoPriority,
    editing: &mut bool,
    memo_doc: &mut doc_editor::Doc,
    edit_form: &mut MemoFormState,
    status_line: &mut String,
    show_settings: &mut bool,
    show_export: &mut bool,
    export_pw: &mut String,
    export_path: &mut String,
    export_ids: &mut Option<Vec<String>>,
    show_history: &mut bool,
    history_entity: &mut String,
    history_events: &mut Vec<memo_core::service::HistoryEvent>,
    history_sel_a: &mut Option<usize>,
    history_sel_b: &mut Option<usize>,
    show_delete: &mut bool,
    purge_confirm_id: &mut Option<String>,
    show_help: &mut bool,
    show_about: &mut bool,
    data_dir: &str,
    show_backup_badge: bool,
    discovered: &[DiscoveredPeer],
    connected_count: usize,
    auto_lock_remaining: Option<Duration>,
    pending_sync_hint: usize,
    local_node_id: &str,
    node_role: NodeRole,
    node_visible: bool,
    lan_discovery: bool,
    local_disk: DiskSpace,
    backup_targets: &mut Vec<String>,
    tx: &Sender<BgMsg>,
) -> ShellAction {
    let mut action = ShellAction::default();
    let alias = svc.session_alias();
    let alias_show = if alias.is_empty() {
        IdentityKeys::short_fp(svc.session_fp())
    } else {
        alias.to_string()
    };
    let all = svc.list();
    let trash = if shell.nav.is_trash() {
        svc.list_trash()
    } else {
        Vec::new()
    };
    let online_count = discovered
        .iter()
        .filter(|p| p.status == PeerStatus::Online)
        .count();

    // 经期 / 体检关怀：未点开则侧栏闪烁 + 顶栏温柔提示
    let mut women_care = period_view::care_remind(svc, &all);
    let mut male_care = male_view::care_remind(svc);
    if let Some(r) = women_care.as_ref() {
        if r.needs_attention() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }
    if let Some(r) = male_care.as_ref() {
        if r.needs_attention() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }
    // 关怀已读：点开置顶「性别私密」入口后才标记
    if selected.as_deref() == Some(HUB_GENDER_PRIVATE) {
        if let Some(r) = women_care.as_ref() {
            if r.needs_attention() {
                period_view::ack_care_remind(svc, &r.expected_start);
                if let Some(c) = women_care.as_mut() {
                    c.seen = true;
                }
            }
        }
        if let Some(r) = male_care.as_ref() {
            if r.needs_attention() {
                male_view::ack_care_remind(svc, &r.next_checkup);
                if let Some(c) = male_care.as_mut() {
                    c.seen = true;
                }
            }
        }
    }
    let care_pulse = women_care
        .as_ref()
        .filter(|r| r.needs_attention())
        .map(|_| {
            let t = ctx.input(|i| i.time);
            (t * 2.2).sin() as f32 * 0.5 + 0.5
        })
        .map(|u| 0.35 + u * 0.65);
    let male_pulse = male_care
        .as_ref()
        .filter(|r| r.needs_attention())
        .map(|_| {
            let t = ctx.input(|i| i.time);
            (t * 2.2).sin() as f32 * 0.5 + 0.5
        })
        .map(|u| 0.35 + u * 0.65);

    // —— 顶栏 ——
    egui::TopBottomPanel::top("shell_top")
        .exact_height(44.0)
        .frame(
            Frame::none()
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::border()))
                .inner_margin(Margin::symmetric(12.0, 4.0)),
        )
        .show(ctx, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.spacing_mut().item_spacing.y = 0.0;

                let letter = avatar_letter(&alias_show);
                let av_color = theme::avatar_color(0);
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 13.0, av_color);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &letter,
                    egui::FontId::proportional(13.0),
                    Color32::WHITE,
                );

                let menu = egui::menu::menu_button(
                    ui,
                    RichText::new(format!("{alias_show} ▾"))
                        .strong()
                        .size(14.0)
                        .color(theme::text()),
                    |ui| {
                        if ui.button("设置").clicked() {
                            *show_settings = true;
                            ui.close_menu();
                        }
                        if ui.button("切换身份").clicked() {
                            action.switch = true;
                            ui.close_menu();
                        }
                        if ui.button("帮助").clicked() {
                            *show_help = true;
                            ui.close_menu();
                        }
                        if ui.button("关于").clicked() {
                            *show_about = true;
                            ui.close_menu();
                        }
                        if ui.button("导出").clicked() {
                            let dir = std::path::PathBuf::from(data_dir).join("exports");
                            let path = dir.join(format!(
                                "memo-all-{}.txt",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0)
                            ));
                            *export_ids = None;
                            *export_path = path.display().to_string();
                            export_pw.clear();
                            *show_export = true;
                            ui.close_menu();
                        }
                    },
                );
                let chip_h = menu.response.rect.height().clamp(22.0, 28.0);

                show_top_tabs(
                    ui,
                    shell,
                    &all,
                    svc.list_trash().len(),
                    care_pulse.or(male_pulse),
                    selected,
                    editing,
                );

                if let Some(remaining) = auto_lock_remaining {
                    let _ = pill(
                        ui,
                        theme::shell_chip_gray(),
                        theme::shell_chip_gray_fg(),
                        &format!("{} 后自动锁定", format_auto_lock(remaining)),
                        chip_h,
                    );
                }

                ui.add_space(8.0);
                let right_reserve = 220.0;
                let search_w = (ui.available_width() - right_reserve).clamp(120.0, 320.0);
                let (search_rect, search_click) =
                    ui.allocate_exact_size(Vec2::new(search_w, chip_h), Sense::click());
                ui.painter().rect(
                    search_rect,
                    Rounding::same(chip_h * 0.5),
                    theme::bg(),
                    Stroke::new(1.0, theme::border()),
                );
                let font = egui::FontId::proportional(13.0);
                let line_h = ui.fonts(|f| f.row_height(&font));
                let text_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        search_rect.left() + 10.0,
                        search_rect.center().y - line_h * 0.5,
                    ),
                    egui::vec2((search_rect.width() - 20.0).max(40.0), line_h),
                );
                let te = ui.put(
                    text_rect,
                    egui::TextEdit::singleline(search)
                        .id(egui::Id::new("shell_top_search"))
                        .hint_text("")
                        .frame(false)
                        .margin(egui::vec2(0.0, 0.0))
                        .desired_width(text_rect.width()),
                );
                if search.is_empty() {
                    ui.painter().text(
                        egui::pos2(text_rect.left(), search_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        "搜索我的备忘…",
                        font,
                        theme::text_muted(),
                    );
                }
                if search_click.clicked() {
                    te.request_focus();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if top_icon_button(ui, TopIcon::Lock, "锁定").clicked() {
                        action.switch = true;
                    }
                    if top_icon_button(ui, TopIcon::Settings, "设置").clicked() {
                        *show_settings = true;
                    }
                    if top_icon_button_on(ui, TopIcon::Nodes, "节点", shell.show_nodes)
                        .on_hover_text(if shell.show_nodes {
                            "隐藏右侧节点树"
                        } else {
                            "显示右侧节点树"
                        })
                        .clicked()
                    {
                        shell.show_nodes = !shell.show_nodes;
                    }
                    if top_icon_button(ui, TopIcon::Sync, "同步").clicked() {
                        action.sync = true;
                    }
                });
            });
        });

    // —— 右节点树 ——
    if shell.show_nodes {
        egui::SidePanel::right("shell_nodes")
            .exact_width(252.0)
            .resizable(false)
            .frame(theme::right_panel_frame())
            .show(ctx, |ui| {
                show_nodes_panel(
                    ui,
                    discovered,
                    local_node_id,
                    node_role,
                    node_visible,
                    lan_discovery,
                    local_disk,
                    backup_targets,
                    online_count,
                    &mut action,
                );
            });
    }

    // —— 内容 ——
    // 注意：CentralPanel 的 Frame 背景按 content min_rect 绘制，且 clip 默认为整屏。
    // 若子控件把 min_rect 撑高，白底会盖住底栏。因此内容放在独立 child_ui，
    // 父级只 allocate 固定 available 区域，禁止撑高。
    egui::CentralPanel::default()
        .frame(
            Frame::none()
                .fill(theme::bg())
                .inner_margin(Margin::same(12.0)),
        )
        .show(ctx, |ui| {
            let bounds = ui.max_rect();
            ui.set_clip_rect(bounds);
            {
                let mut body = ui.child_ui(bounds, egui::Layout::top_down(egui::Align::Min));
                body.set_clip_rect(bounds);
                show_status_strip(&mut body, online_count, connected_count, pending_sync_hint);
                body.add_space(8.0);
                if let Some(r) = women_care.as_ref().filter(|r| r.needs_attention()) {
                    body.add_space(8.0);
                    show_period_care_banner(&mut body, r, &mut shell.nav, selected, editing);
                }
                if let Some(r) = male_care.as_ref().filter(|r| r.needs_attention()) {
                    body.add_space(8.0);
                    show_male_care_banner(&mut body, r, &mut shell.nav, selected, editing);
                }
                body.add_space(8.0);

                let body_h = body.available_height().max(80.0);
                let body_w = body.available_width();
                let body_rect = egui::Rect::from_min_size(body.cursor().min, Vec2::new(body_w, body_h));
                let body_rect = body_rect.intersect(bounds);
                {
                    let mut inner =
                        body.child_ui(body_rect, egui::Layout::top_down(egui::Align::Min));
                    inner.set_clip_rect(body_rect);
                    show_shell_body(
                        &mut inner,
                        svc,
                        shell,
                        search,
                        selected,
                        title_draft,
                        body_draft,
                        visibility_draft,
                        category_draft,
                        due_date_draft,
                        end_date_draft,
                        tags_draft,
                        done_draft,
                        priority_draft,
                        editing,
                        memo_doc,
                        edit_form,
                        status_line,
                        show_delete,
                        show_export,
                        export_pw,
                        export_path,
                        export_ids,
                        show_history,
                        history_entity,
                        history_events,
                        history_sel_a,
                        history_sel_b,
                        data_dir,
                        show_backup_badge,
                        &all,
                        &trash,
                        purge_confirm_id,
                        &mut action,
                        tx,
                    );
                }
            }
            // 固定占用 CentralPanel 可用区域，避免 Frame 随内容增高盖住底栏
            ui.allocate_rect(bounds, Sense::hover());
        });

    action
}

/// CentralPanel 主体（已扣除同步条/关怀条高度，且限制在底栏之上）。
#[allow(clippy::too_many_arguments)]
fn show_shell_body(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    shell: &mut ShellUi,
    search: &str,
    selected: &mut Option<String>,
    title_draft: &mut String,
    body_draft: &mut String,
    visibility_draft: &mut MemoVisibility,
    category_draft: &mut MemoCategory,
    due_date_draft: &mut String,
    end_date_draft: &mut String,
    tags_draft: &mut String,
    done_draft: &mut bool,
    priority_draft: &mut MemoPriority,
    editing: &mut bool,
    memo_doc: &mut doc_editor::Doc,
    edit_form: &mut MemoFormState,
    status_line: &mut String,
    show_delete: &mut bool,
    show_export: &mut bool,
    export_pw: &mut String,
    export_path: &mut String,
    export_ids: &mut Option<Vec<String>>,
    show_history: &mut bool,
    history_entity: &mut String,
    history_events: &mut Vec<memo_core::service::HistoryEvent>,
    history_sel_a: &mut Option<usize>,
    history_sel_b: &mut Option<usize>,
    data_dir: &str,
    show_backup_badge: bool,
    all: &[MemoView],
    trash: &[MemoView],
    purge_confirm_id: &mut Option<String>,
    action: &mut ShellAction,
    tx: &Sender<BgMsg>,
) {
            if shell.nav.is_trash() {
                show_trash(ui, svc, &trash, purge_confirm_id, status_line, tx);
            } else if shell.nav.is_calendar() {
                let q = search.trim().to_lowercase();
                let detail_open = selected
                    .as_ref()
                    .map(|id| !is_new_draft(id) && all.iter().any(|m| m.id == *id))
                    .unwrap_or(false);
                if detail_open {
                    // 左半日历、右半详情/编辑，与未选中时同一比例
                    let (cal_w, side_w, full_h) = calendar_view::split_cal_detail(ui);
                    let mut picked = None;
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
                                calendar_view::show_month(
                                    ui,
                                    &mut shell.cal,
                                    &all,
                                    &q,
                                    selected,
                                    editing,
                                    &mut action.open_new_memo,
                                    &mut picked,
                                );
                            },
                        );
                        ui.allocate_ui_with_layout(
                            Vec2::new(side_w, full_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_max_width(side_w);
                                ui.set_max_height(full_h);
                                ui.set_clip_rect(ui.max_rect());
                                if *editing {
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add(
                                                egui::Button::new("← 返回详情")
                                                    .min_size(Vec2::new(80.0, 24.0)),
                                            )
                                            .clicked()
                                        {
                                            *editing = false;
                                        }
                                    });
                                    ui.add_space(4.0);
                                    show_detail(
                                        ui,
                                        svc,
                                        &all,
                                        &all,
                                        &mut shell.gender,
                                        selected,
                                        title_draft,
                                        body_draft,
                                        visibility_draft,
                                        category_draft,
                                        due_date_draft,
                                        end_date_draft,
                                        tags_draft,
                                        done_draft,
                                        priority_draft,
                                        editing,
                                        memo_doc,
                                        edit_form,
                                        status_line,
                                        show_delete,
                                        show_export,
                                        export_pw,
                                        export_path,
                                        export_ids,
                                        show_history,
                                        history_entity,
                                        history_events,
                                        history_sel_a,
                                        history_sel_b,
                                        data_dir,
                                        tx,
                                    );
                                } else if let Some(m) = selected
                                    .as_ref()
                                    .and_then(|id| all.iter().find(|x| x.id == *id))
                                {
                                    let m = m.clone();
                                    calendar_view::show_event_detail(
                                        ui,
                                        svc,
                                        &m,
                                        selected,
                                        editing,
                                        show_delete,
                                        status_line,
                                        tx,
                                    );
                                }
                            },
                        );
                    });
                    if let Some(id) = picked {
                        if let Some(m) = all.iter().find(|m| m.id == id) {
                            load_selection(
                                m,
                                selected,
                                title_draft,
                                body_draft,
                                visibility_draft,
                                category_draft,
                                due_date_draft,
                                end_date_draft,
                                tags_draft,
                                done_draft,
                                priority_draft,
                                editing,
                                memo_doc,
                                edit_form,
                            );
                            *editing = false;
                        }
                    } else if !*editing {
                        if let Some(id) = selected.clone() {
                            if let Some(m) = all.iter().find(|m| m.id == id) {
                                if title_draft != &m.title {
                                    load_selection(
                                        m,
                                        selected,
                                        title_draft,
                                        body_draft,
                                        visibility_draft,
                                        category_draft,
                                        due_date_draft,
                                        end_date_draft,
                                        tags_draft,
                                        done_draft,
                                        priority_draft,
                                        editing,
                                        memo_doc,
                                        edit_form,
                                    );
                                    *editing = false;
                                }
                            }
                        }
                    }
                } else {
                    let mut picked = None;
                    let mut pick_edit = false;
                    let mut pick_delete = false;
                    calendar_view::show(
                        ui,
                        svc,
                        &mut shell.cal,
                        &all,
                        search,
                        selected,
                        editing,
                        &mut action.open_new_memo,
                        &mut picked,
                        &mut pick_edit,
                        &mut pick_delete,
                        tx,
                    );
                    if let Some(id) = picked {
                        if let Some(m) = all.iter().find(|m| m.id == id) {
                            load_selection(
                                m,
                                selected,
                                title_draft,
                                body_draft,
                                visibility_draft,
                                category_draft,
                                due_date_draft,
                                end_date_draft,
                                tags_draft,
                                done_draft,
                                priority_draft,
                                editing,
                                memo_doc,
                                edit_form,
                            );
                            if pick_edit {
                                *editing = true;
                            }
                            if pick_delete {
                                *show_delete = true;
                            }
                        }
                    }
                }
            } else {
                let filtered = filter_memos(shell.nav, search, &all);
                // 性别私密：无有效选中时默认打开「关怀与健康」日历页
                if shell.nav.is_gender_private() {
                    let sel_ok = selected
                        .as_ref()
                        .map(|id| {
                            is_private_hub(id)
                                || (!is_new_draft(id) && filtered.iter().any(|m| m.id == *id))
                        })
                        .unwrap_or(false);
                    if !sel_ok {
                        select_gender_hub(selected, editing);
                    }
                }
                show_split(
                    ui,
                    svc,
                    &filtered,
                    &all,
                    selected,
                    title_draft,
                    body_draft,
                    visibility_draft,
                    category_draft,
                    due_date_draft,
                    end_date_draft,
                    tags_draft,
                    done_draft,
                    priority_draft,
                    editing,
                    memo_doc,
                    edit_form,
                    status_line,
                    show_delete,
                    show_export,
                    export_pw,
                    export_path,
                    export_ids,
                    show_history,
                    history_entity,
                    history_events,
                    history_sel_a,
                    history_sel_b,
                    data_dir,
                    show_backup_badge,
                    shell.nav,
                    &mut shell.gender,
                    &mut action.open_new_memo,
                    tx,
                );
            }
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum BadgeKind {
    Gray,
    Blue,
    Soft,
}

fn show_top_tabs(
    ui: &mut egui::Ui,
    shell: &mut ShellUi,
    all: &[MemoView],
    trash_n: usize,
    care_pulse: Option<f32>,
    selected: &mut Option<String>,
    editing: &mut bool,
) {
    let cred_n = count_category(all, MemoCategory::Credentials);
    let priv_n = all
        .iter()
        .filter(|m| m.category.is_gender_private() && !is_private_calendar_memo(m))
        .count();
    ui.add_space(4.0);
    for item in NavItem::TABS {
        let sel = shell.nav == *item;
        let trash = item.is_trash();
        let h = 24.0;
        let label = if trash { "" } else { item.label() };
        let icon = item.icon();
        let badge = match item {
            NavItem::Credentials => cred_n,
            NavItem::GenderPrivate => priv_n,
            NavItem::Trash => trash_n,
            NavItem::Calendar => 0,
        };
        let icon_fg = if item.is_gender_private() && care_pulse.is_some() {
            theme::shell_women_pill_fg()
        } else {
            theme::nav_icon_color(*item)
        };
        let label_fg = if sel {
            theme::shell_accent()
        } else if trash {
            theme::text_muted()
        } else if item.is_gender_private() && care_pulse.is_some() {
            theme::shell_women_pill_fg()
        } else {
            theme::text()
        };
        let icon_g = theme::layout_galley(
            ui,
            icon,
            egui::FontId::proportional(if trash { 14.0 } else { 13.0 }),
            icon_fg,
        );
        let label_g = if trash {
            None
        } else {
            Some(theme::layout_galley(
                ui,
                label,
                egui::FontId::proportional(13.0),
                label_fg,
            ))
        };
        let icon_slot = if trash {
            14.0_f32.max(icon_g.mesh_bounds.width())
        } else {
            16.0_f32.max(icon_g.mesh_bounds.width())
        };
        let w = if trash {
            28.0
        } else {
            let text_w = label_g.as_ref().map(|g| g.size().x).unwrap_or(0.0);
            let extra = if badge > 0 { 22.0 } else { 0.0 };
            (10.0 + icon_slot + 6.0 + text_w + extra + 10.0).max(48.0)
        };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let fill = if sel {
            theme::shell_nav_selected()
        } else if resp.hovered() {
            theme::panel()
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect(rect, Rounding::same(12.0), fill, Stroke::NONE);
        let cy = rect.center().y;
        if trash {
            ui.painter().galley(
                theme::galley_pos_center(rect.center(), &icon_g),
                icon_g,
                icon_fg,
            );
            if badge > 0 {
                let br = egui::Rect::from_center_size(
                    egui::pos2(rect.right() - 4.0, rect.top() + 4.0),
                    Vec2::new(14.0, 14.0),
                );
                ui.painter()
                    .rect_filled(br, Rounding::same(7.0), theme::shell_accent_soft());
                ui.painter().text(
                    br.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{badge}"),
                    egui::FontId::proportional(9.0),
                    theme::shell_accent(),
                );
            }
        } else {
            let icon_cx = rect.left() + 10.0 + icon_slot * 0.5;
            ui.painter().galley(
                theme::galley_pos_center(egui::pos2(icon_cx, cy), &icon_g),
                icon_g,
                icon_fg,
            );
            if let Some(label_g) = label_g {
                ui.painter().galley(
                    theme::galley_pos_left_center(
                        egui::pos2(rect.left() + 10.0 + icon_slot + 6.0, cy),
                        &label_g,
                    ),
                    label_g,
                    label_fg,
                );
            }
            if badge > 0 {
                let br = egui::Rect::from_center_size(
                    egui::pos2(rect.right() - 12.0, cy),
                    Vec2::new(16.0, 16.0),
                );
                ui.painter()
                    .rect_filled(br, Rounding::same(8.0), theme::shell_accent_soft());
                ui.painter().text(
                    br.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{badge}"),
                    egui::FontId::proportional(10.0),
                    theme::shell_accent(),
                );
            }
        }
        if resp.clicked() {
            shell.nav = *item;
            if item.is_gender_private() {
                select_gender_hub(selected, editing);
            } else {
                *selected = None;
                *editing = false;
            }
        }
        if trash {
            resp.on_hover_text("回收站");
        }
    }
}

fn show_period_care_banner(
    ui: &mut egui::Ui,
    care: &PeriodCareRemind,
    nav: &mut NavItem,
    selected: &mut Option<String>,
    editing: &mut bool,
) {
    let pulse = {
        let t = ui.ctx().input(|i| i.time);
        let u = (t * 2.2).sin() as f32 * 0.5 + 0.5;
        0.55 + u * 0.45
    };
    let bg = theme::shell_women_pill();
    let fg = theme::shell_women_pill_fg();
    let stroke_c = theme::period_day().linear_multiply(pulse);

    let resp = Frame::none()
        .fill(bg)
        .stroke(Stroke::new(1.5, stroke_c))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::symmetric(14.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!("♡  {}", care.title))
                            .size(14.0)
                            .strong()
                            .color(fg),
                    );
                    ui.label(RichText::new(&care.body).size(12.5).color(fg));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::primary_button(ui, "去看看").clicked() {
                        *nav = NavItem::GenderPrivate;
                        select_gender_hub(selected, editing);
                    }
                });
            });
        })
        .response;
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        *nav = NavItem::GenderPrivate;
        select_gender_hub(selected, editing);
    }
}

fn show_male_care_banner(
    ui: &mut egui::Ui,
    care: &male_view::MaleCareRemind,
    nav: &mut NavItem,
    selected: &mut Option<String>,
    editing: &mut bool,
) {
    let pulse = {
        let t = ui.ctx().input(|i| i.time);
        let u = (t * 2.2).sin() as f32 * 0.5 + 0.5;
        0.55 + u * 0.45
    };
    let bg = theme::shell_men_pill();
    let fg = theme::shell_men_pill_fg();
    let stroke_c = theme::shell_men_pill_fg().linear_multiply(pulse);

    let resp = Frame::none()
        .fill(bg)
        .stroke(Stroke::new(1.5, stroke_c))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::symmetric(14.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!("♡  {}", care.title))
                            .size(14.0)
                            .strong()
                            .color(fg),
                    );
                    ui.label(RichText::new(&care.body).size(12.5).color(fg));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::primary_button(ui, "去看看").clicked() {
                        *nav = NavItem::GenderPrivate;
                        select_gender_hub(selected, editing);
                    }
                });
            });
        })
        .response;
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        *nav = NavItem::GenderPrivate;
        select_gender_hub(selected, editing);
    }
}

#[allow(dead_code)]
fn nav_row(
    ui: &mut egui::Ui,
    current: &mut NavItem,
    item: NavItem,
    badge: Option<(usize, BadgeKind)>,
    care_pulse: Option<f32>,
    locked: bool,
    show_settings: &mut bool,
    selected: &mut Option<String>,
    editing: &mut bool,
) {
    let sel = *current == item && !locked;
    let height = 40.0;
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::click(),
    );
    // 整行一体交互：不用 Label 子控件，避免抢走悬停并把光标变成 I 型
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = if locked {
        resp.on_hover_text("请先在设置中选择当前身份性别")
    } else {
        resp
    };

    let care_pulse = if locked { None } else { care_pulse };
    let fill = if locked {
        Color32::TRANSPARENT
    } else if sel {
        theme::shell_nav_selected()
    } else if resp.hovered() {
        theme::c().list_hover
    } else if let Some(p) = care_pulse.filter(|_| item.is_gender_private()) {
        theme::shell_women_pill().linear_multiply(0.35 + p * 0.45)
    } else if let Some(p) = care_pulse.filter(|_| item.is_gender_private()) {
        theme::shell_men_pill().linear_multiply(0.35 + p * 0.45)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, Rounding::same(8.0), fill);

    if let Some(p) = care_pulse.filter(|_| item.is_gender_private()) {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            Rounding::same(8.0),
            Stroke::new(1.2, theme::period_day().linear_multiply(p)),
        );
    } else if let Some(p) = care_pulse.filter(|_| item.is_gender_private()) {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            Rounding::same(8.0),
            Stroke::new(1.2, theme::shell_men_pill_fg().linear_multiply(p)),
        );
    }

    let muted = theme::text_muted().linear_multiply(0.75);
    let icon_fg = if locked {
        muted
    } else {
        theme::nav_icon_color(item)
    };
    let label_fg = if locked {
        muted
    } else if sel {
        theme::shell_accent()
    } else if care_pulse.is_some() && item.is_gender_private() {
        theme::shell_women_pill_fg()
    } else if care_pulse.is_some() && item.is_gender_private() {
        theme::shell_men_pill_fg()
    } else {
        theme::text()
    };

    let pad_x = 10.0;
    let icon = item.icon();
    let label = item.label();
    let icon_g = theme::layout_galley(
        ui,
        icon,
        egui::FontId::proportional(19.5),
        icon_fg,
    );
    let label_g = theme::layout_galley(
        ui,
        label,
        egui::FontId::proportional(13.5),
        label_fg,
    );
    let icon_slot = 28.0_f32.max(icon_g.mesh_bounds.width());
    let cy = rect.center().y;
    ui.painter().galley(
        theme::galley_pos_center(
            egui::pos2(rect.left() + pad_x + icon_slot * 0.5, cy),
            &icon_g,
        ),
        icon_g,
        icon_fg,
    );
    ui.painter().galley(
        theme::galley_pos_left_center(
            egui::pos2(rect.left() + pad_x + icon_slot + 8.0, cy),
            &label_g,
        ),
        label_g,
        label_fg,
    );

    // 右侧徽章（纯绘制，不抢悬停）
    let mut right = rect.right() - pad_x;
    if item.is_gender_private() || item.is_gender_private() {
        let (text, bg, fg) = if locked {
            ("未设置", theme::shell_chip_gray(), theme::shell_chip_gray_fg())
        } else if let Some(p) = care_pulse {
            if item.is_gender_private() {
                (
                    "提醒",
                    theme::shell_men_pill_fg().linear_multiply(0.55 + p * 0.45),
                    Color32::WHITE,
                )
            } else {
                (
                    "提醒",
                    theme::period_day().linear_multiply(0.55 + p * 0.45),
                    Color32::WHITE,
                )
            }
        } else if item.is_gender_private() {
            (
                "已开启",
                theme::shell_men_pill(),
                theme::shell_men_pill_fg(),
            )
        } else {
            (
                "已开启",
                theme::shell_women_pill(),
                theme::shell_women_pill_fg(),
            )
        };
        let font = egui::FontId::proportional(10.5);
        let tw = ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font.clone(), fg).size().x);
        let bw = tw + 14.0;
        let bh = 18.0;
        let br = egui::Rect::from_min_size(
            egui::pos2(right - bw, rect.center().y - bh * 0.5),
            Vec2::new(bw, bh),
        );
        ui.painter()
            .rect_filled(br, Rounding::same(10.0), bg);
        ui.painter().text(
            br.center(),
            egui::Align2::CENTER_CENTER,
            text,
            font,
            fg,
        );
        right = br.left() - 6.0;
        let _ = right;
    } else if let Some((n, kind)) = badge {
        if n > 0 {
            let (bg, fg) = match kind {
                BadgeKind::Gray => (theme::shell_chip_gray(), theme::shell_chip_gray_fg()),
                BadgeKind::Blue => (theme::shell_accent(), Color32::WHITE),
                BadgeKind::Soft => (theme::shell_accent_soft(), theme::shell_accent()),
            };
            let text = n.to_string();
            let font = egui::FontId::proportional(11.0);
            let tw = ui.fonts(|f| {
                f.layout_no_wrap(text.clone(), font.clone(), fg).size().x
            });
            let bw = (tw + 12.0).max(20.0);
            let bh = 18.0;
            let br = egui::Rect::from_min_size(
                egui::pos2(right - bw, rect.center().y - bh * 0.5),
                Vec2::new(bw, bh),
            );
            ui.painter().rect_filled(br, Rounding::same(8.0), bg);
            ui.painter().text(
                br.center(),
                egui::Align2::CENTER_CENTER,
                text,
                font,
                fg,
            );
        }
    }

    if resp.clicked() {
        if locked {
            *show_settings = true;
        } else {
            *current = item;
            if item.is_gender_private() {
                select_gender_hub(selected, editing);
            } else {
                *selected = None;
                *editing = false;
            }
        }
    }
}

fn create_memo_from_form(
    svc: &Arc<MemoService>,
    form: &MemoFormState,
    tx: &Sender<BgMsg>,
) {
    let Ok((title, body)) = memo_form::validate_title(form) else {
        return;
    };
    let category = form.category.canonical();
    let due = form.due_date.clone();
    let end = form.end_date.clone();
    let tags = memo_form::tags_vec(form);
    let priority = form.priority;
    let remind = form.remind_before_days;
    let visibility = if category.is_gender_private() {
        MemoVisibility::Private
    } else {
        form.visibility
    };
        let svc = svc.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        match svc.add_full(
            &title,
            &body,
            visibility,
            MemoLifecycle::Permanent,
            category,
            &due,
            &end,
            &tags,
            priority,
            remind,
        ) {
            Ok(id) => {
                let _ = tx.send(BgMsg::CreatedMemo {
                    id,
                    title,
                    body,
                    category,
                    due_date: due,
                    end_date: end,
                    priority,
                    tags,
                });
            }
            Err(e) => {
                let _ = tx.send(BgMsg::Error(e.to_string()));
            }
        }
    });
}

/// 供详情区新建表单保存时调用。
pub fn submit_new_memo(svc: &Arc<MemoService>, form: &MemoFormState, tx: &Sender<BgMsg>) {
    create_memo_from_form(svc, form, tx);
}

fn show_trash(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    trash: &[MemoView],
    purge_confirm_id: &mut Option<String>,
    status_line: &mut String,
    tx: &Sender<BgMsg>,
) {
    ui.horizontal(|ui| {
        theme::icon_label_heading(
            ui,
            NavItem::Trash.icon(),
            NavItem::Trash.label(),
            theme::nav_icon_color(NavItem::Trash),
            theme::text(),
            30.0,
            20.0,
        );
        ui.label(
            RichText::new(format!("{} 条", trash.len()))
                .size(13.0)
                .color(theme::text_muted()),
        );
    });
    ui.label(
        theme::muted_label(format!(
            "已删除备忘保留 {} 天，可恢复或彻底清除。",
            memo_core::TRASH_RETENTION_DAYS
        )),
    );
    ui.add_space(10.0);
    if trash.is_empty() {
        ui.label(theme::muted_label("回收站为空"));
        return;
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        for m in trash {
            theme::card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if m.title.is_empty() {
                            "(无标题)"
                        } else {
                            m.title.as_str()
                        })
                        .strong()
                        .color(theme::text()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::danger_button(ui, "彻底清除").clicked() {
                            *purge_confirm_id = Some(m.id.clone());
                        }
                        if theme::ghost_button(ui, "恢复").clicked() {
                            let svc = svc.clone();
                            let id = m.id.clone();
                            let tx = tx.clone();
                            std::thread::spawn(move || match svc.undelete(&id) {
                                Ok(()) => {
                                    let _ = tx.send(BgMsg::Info("已恢复".into()));
                                    let _ = tx.send(BgMsg::Refresh);
                                }
                                Err(e) => {
                                    let _ = tx.send(BgMsg::Error(e.to_string()));
                                }
                            });
                        }
                    });
                });
                if !m.deleted_at.is_empty() {
                    ui.label(
                        RichText::new(format!("删除于 {}", m.deleted_at))
                            .small()
                            .color(theme::text_muted()),
                    );
                }
            });
            ui.add_space(6.0);
        }
    });
    let _ = status_line;
}

fn load_selection(
    m: &MemoView,
    selected: &mut Option<String>,
    title_draft: &mut String,
    body_draft: &mut String,
    visibility_draft: &mut MemoVisibility,
    category_draft: &mut MemoCategory,
    due_date_draft: &mut String,
    end_date_draft: &mut String,
    tags_draft: &mut String,
    done_draft: &mut bool,
    priority_draft: &mut MemoPriority,
    editing: &mut bool,
    memo_doc: &mut doc_editor::Doc,
    edit_form: &mut MemoFormState,
) {
    *selected = Some(m.id.clone());
    *title_draft = m.title.clone();
    *body_draft = m.content.clone();
    *visibility_draft = m.visibility;
    *category_draft = m.category.canonical();
    *due_date_draft = m.due_date.clone();
    *end_date_draft = m.end_date.clone();
    *tags_draft = format_tags(&m.tags);
    *done_draft = m.done;
    *priority_draft = m.priority;
    *editing = false;
    *memo_doc = doc_editor::Doc::from_store(&m.title, &m.content);
    *edit_form = MemoFormState::load_from_drafts(
        m.category.canonical(),
        &m.title,
        &m.content,
        &m.due_date,
        &m.end_date,
        m.priority,
        tags_draft,
        m.done,
        m.visibility,
        m.remind_before_days,
    );
}

#[allow(clippy::too_many_arguments)]
fn show_split(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    filtered: &[MemoView],
    all_memos: &[MemoView],
    selected: &mut Option<String>,
    title_draft: &mut String,
    body_draft: &mut String,
    visibility_draft: &mut MemoVisibility,
    category_draft: &mut MemoCategory,
    due_date_draft: &mut String,
    end_date_draft: &mut String,
    tags_draft: &mut String,
    done_draft: &mut bool,
    priority_draft: &mut MemoPriority,
    editing: &mut bool,
    memo_doc: &mut doc_editor::Doc,
    edit_form: &mut MemoFormState,
    status_line: &mut String,
    show_delete: &mut bool,
    show_export: &mut bool,
    export_pw: &mut String,
    export_path: &mut String,
    export_ids: &mut Option<Vec<String>>,
    show_history: &mut bool,
    history_entity: &mut String,
    history_events: &mut Vec<memo_core::service::HistoryEvent>,
    history_sel_a: &mut Option<usize>,
    history_sel_b: &mut Option<usize>,
    data_dir: &str,
    show_backup_badge: bool,
    nav: NavItem,
    gender_ui: &mut GenderPrivateUi,
    open_new_memo: &mut bool,
    tx: &Sender<BgMsg>,
) {
    let avail = ui.available_width();
    // 必须在 horizontal 之前取高度：horizontal 内 available_height 不可靠
    let full_h = ui.available_height().max(120.0);
    let gap = 8.0;
    let (list_w, detail_w) = if nav.is_gender_private() {
        // 私密页：左半列表 / 右半详情或专属日历（对齐效果图）
        let list_w = ((avail - gap) * 0.5).max(160.0);
        let detail_w = (avail - list_w - gap).max(160.0);
        (list_w, detail_w)
    } else {
        let min_detail = 240.0;
        let mut list_w = (avail * 0.38).clamp(160.0, 400.0);
        if list_w + min_detail + gap > avail {
            list_w = (avail - min_detail - gap).max(120.0).min(list_w);
        }
        let detail_w = (avail - list_w - gap).max(120.0);
        (list_w, detail_w)
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(
            Vec2::new(list_w, full_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(full_h);
                ui.set_max_height(full_h);
                ui.set_max_width(list_w);
                ui.set_clip_rect(ui.max_rect());
                ui.horizontal(|ui| {
                    theme::icon_label_heading(
                        ui,
                        nav.icon(),
                        nav.label(),
                        theme::nav_icon_color(nav),
                        theme::text(),
                        24.0,
                        16.0,
                    );
                    let list_n = filtered.iter().filter(|m| !is_private_hub(&m.id)).count();
                    if hub_memo_for(nav).is_some() {
                        Frame::none()
                            .fill(theme::shell_accent())
                            .rounding(Rounding::same(10.0))
                            .inner_margin(Margin::symmetric(8.0, 2.0))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("+ {list_n} 条"))
                                        .size(11.5)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                            });
                    } else {
                        ui.label(
                            RichText::new(format!("{list_n} 条"))
                                .size(13.0)
                                .color(theme::text_muted()),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("+")
                                        .color(Color32::WHITE)
                                        .strong()
                                        .size(14.0),
                                )
                                .fill(theme::shell_accent())
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(28.0, 26.0)),
                            )
                            .clicked()
                        {
                            *open_new_memo = true;
                        }
                    });
                });
                ui.add_space(8.0);

                // 私密页：过期 / 今天到期 / 即将到期(3天内) / 其它
                let private_warn = nav.is_gender_private();
                let mut overdue_ids = std::collections::HashSet::new();
                let mut due_today_ids = std::collections::HashSet::new();
                let mut due_soon_ids = std::collections::HashSet::new();
                let display: Vec<&MemoView> = if private_warn {
                    let hub: Vec<_> = filtered.iter().filter(|m| is_private_hub(&m.id)).collect();
                    let rest: Vec<_> = filtered.iter().filter(|m| !is_private_hub(&m.id)).collect();
                    let mut overdue: Vec<&MemoView> =
                        rest.iter().copied().filter(|m| memo_is_overdue(m)).collect();
                    overdue.sort_by_key(|m| memo_core::deadline_date(&m.due_date, &m.end_date));
                    let due_today: Vec<&MemoView> = rest
                        .iter()
                        .copied()
                        .filter(|m| memo_is_due_today(m))
                        .collect();
                    let mut due_soon: Vec<&MemoView> = rest
                        .iter()
                        .copied()
                        .filter(|m| memo_is_due_soon(m))
                        .collect();
                    due_soon.sort_by_key(|m| memo_core::deadline_date(&m.due_date, &m.end_date));
                    for m in &overdue {
                        overdue_ids.insert(m.id.as_str());
                    }
                    for m in &due_today {
                        due_today_ids.insert(m.id.as_str());
                    }
                    for m in &due_soon {
                        due_soon_ids.insert(m.id.as_str());
                    }
                    let other: Vec<&MemoView> = rest
                        .iter()
                        .copied()
                        .filter(|m| {
                            !overdue_ids.contains(m.id.as_str())
                                && !due_today_ids.contains(m.id.as_str())
                                && !due_soon_ids.contains(m.id.as_str())
                        })
                        .collect();
                    if !overdue.is_empty() || !due_today.is_empty() || !due_soon.is_empty() {
                        show_private_warn_banner(
                            ui,
                            gender_ui,
                            &overdue,
                            due_today.len(),
                            due_soon.len(),
                            selected,
                            title_draft,
                            body_draft,
                            visibility_draft,
                            category_draft,
                            due_date_draft,
                            end_date_draft,
                            tags_draft,
                            done_draft,
                            priority_draft,
                            editing,
                            memo_doc,
                            edit_form,
                        );
                        ui.add_space(6.0);
                    }
                    let mut out = Vec::with_capacity(filtered.len());
                    out.extend(hub);
                    out.extend(overdue);
                    out.extend(due_today);
                    out.extend(due_soon);
                    out.extend(other);
                    out
                } else {
                    filtered.iter().collect()
                };

                let scroll_h = ui.available_height().max(80.0);
                egui::ScrollArea::vertical()
                    .id_source("shell_memo_list")
                    .max_height(scroll_h)
                    .min_scrolled_height(scroll_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if filtered.is_empty() {
                            ui.label(theme::muted_label("暂无备忘"));
                            return;
                        }
                        let mut last_group = "";
                        let other_n = display
                            .iter()
                            .filter(|m| {
                                !is_private_hub(&m.id)
                                    && !overdue_ids.contains(m.id.as_str())
                                    && !due_today_ids.contains(m.id.as_str())
                                    && !due_soon_ids.contains(m.id.as_str())
                            })
                            .count();
                        let mut focus_scrolled = false;
                        for m in display {
                            let is_hub = is_private_hub(&m.id);
                            let is_od = overdue_ids.contains(m.id.as_str());
                            let is_dt = due_today_ids.contains(m.id.as_str());
                            let is_soon = due_soon_ids.contains(m.id.as_str());
                            if private_warn && !is_hub {
                                let group = if is_od {
                                    "overdue"
                                } else if is_dt {
                                    "due_today"
                                } else if is_soon {
                                    "due_soon"
                                } else {
                                    "other"
                                };
                                if group != last_group {
                                    match group {
                                        "overdue" => private_section_header(
                                            ui,
                                            "⚠ 已过期",
                                            overdue_ids.len(),
                                            theme::warn_overdue_fg(),
                                            theme::warn_overdue_bg(),
                                        ),
                                        "due_today" => private_section_header(
                                            ui,
                                            "⏰ 今天到期",
                                            due_today_ids.len(),
                                            theme::warn_due_today_fg(),
                                            theme::warn_due_today_bg(),
                                        ),
                                        "due_soon" => private_section_header(
                                            ui,
                                            "⏳ 即将到期 (3天内)",
                                            due_soon_ids.len(),
                                            theme::warn_due_soon_fg(),
                                            theme::warn_due_soon_bg(),
                                        ),
                                        "other"
                                            if matches!(
                                                last_group,
                                                "overdue" | "due_today" | "due_soon"
                                            ) && other_n > 0 =>
                                        {
                                            private_section_header(
                                                ui,
                                                "其它私密",
                                                other_n,
                                                theme::text_muted(),
                                                theme::panel(),
                                            );
                                        }
                                        _ => {}
                                    }
                                    last_group = group;
                                }
                            }
                            let is_sel = selected.as_deref() == Some(m.id.as_str());
                            let stripe = if is_hub {
                                None
                            } else if is_od {
                                Some(theme::warn_overdue_fg())
                            } else if is_dt {
                                Some(theme::warn_due_today_fg())
                            } else if is_soon {
                                Some(theme::warn_due_soon_fg())
                            } else {
                                None
                            };
                            let border = if is_sel {
                                theme::shell_accent()
                            } else if is_hub {
                                theme::shell_accent()
                            } else if let Some(c) = stripe {
                                c
                            } else {
                                theme::border()
                            };
                            // 效果图：白底 + 色边；hub 浅蓝底
                            let card_fill = if is_hub {
                                theme::accent_soft()
                            } else {
                                theme::card()
                            };
                            let mut toggled_done = false;
                            let resp = Frame::none()
                                .fill(card_fill)
                                .stroke(Stroke::new(if is_sel { 1.8 } else { 1.0 }, border))
                                .rounding(Rounding::same(10.0))
                                .inner_margin(Margin::symmetric(12.0, 10.0))
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        if is_hub {
                                            let (r, _) =
                                                ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                                            ui.painter().circle_filled(
                                                r.center(),
                                                7.0,
                                                theme::shell_accent(),
                                            );
                                        } else {
                                            let cb = list_category_icon(ui, m.category, m.done);
                                            if cb.clicked() {
                                                toggled_done = true;
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
                                                            let _ =
                                                                tx.send(BgMsg::Error(e.to_string()));
                                                        }
                                                    }
                                                });
                                            }
                                        }

                                        let title_fg = if m.done && !is_hub {
                                            theme::text_muted()
                                        } else if is_od {
                                            theme::warn_overdue_fg()
                                        } else if is_dt {
                                            theme::warn_due_today_fg()
                                        } else if is_soon {
                                            theme::warn_due_soon_fg()
                                        } else {
                                            theme::text()
                                        };
                                        let mut title = RichText::new(if m.title.is_empty() {
                                            "(无标题)"
                                        } else {
                                            m.title.as_str()
                                        })
                                        .strong()
                                        .size(14.0)
                                        .color(title_fg);
                                        if m.done && !is_hub {
                                            title = title.strikethrough();
                                        }
                                        ui.label(title);

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if is_hub {
                                                    Frame::none()
                                                        .fill(theme::shell_accent())
                                                        .rounding(Rounding::same(8.0))
                                                        .inner_margin(Margin::symmetric(8.0, 2.0))
                                                        .show(ui, |ui| {
                                                            ui.label(
                                                                RichText::new("专属")
                                                                    .size(11.0)
                                                                    .strong()
                                                                    .color(Color32::WHITE),
                                                            );
                                                        });
                                                } else {
                                                    if !private_warn {
                                                        sync_mark(ui, m.title.is_empty());
                                                        if show_backup_badge
                                                            && m.visibility
                                                                == MemoVisibility::Private
                                                        {
                                                            ui.label(
                                                                RichText::new("私")
                                                                    .small()
                                                                    .color(theme::shell_accent()),
                                                            );
                                                        }
                                                    }
                                                    if !m.due_date.is_empty() {
                                                        let (due_txt, due_fg) = if is_od {
                                                            (
                                                                format!(
                                                                    "到期 {}",
                                                                    format_due_mmdd(&m.due_date)
                                                                ),
                                                                theme::warn_overdue_fg(),
                                                            )
                                                        } else if is_dt {
                                                            (
                                                                format!(
                                                                    "到期 {}",
                                                                    format_due_mmdd(&m.due_date)
                                                                ),
                                                                theme::warn_due_today_fg(),
                                                            )
                                                        } else if is_soon {
                                                            let n = memo_core::days_until_due(
                                                                &m.due_date,
                                                                &m.end_date,
                                                                m.done,
                                                            )
                                                            .unwrap_or(0);
                                                            (
                                                                format!("还有 {n} 天"),
                                                                theme::warn_due_soon_fg(),
                                                            )
                                                        } else {
                                                            (
                                                                format!(
                                                                    "到期 {}",
                                                                    format_due_mmdd(&m.due_date)
                                                                ),
                                                                theme::text_muted(),
                                                            )
                                                        };
                                                        ui.label(
                                                            RichText::new(due_txt)
                                                                .size(12.0)
                                                                .strong()
                                                                .color(due_fg),
                                                        );
                                                    }
                                                    if !private_warn {
                                                        if m.priority != MemoPriority::Normal {
                                                            ui.label(
                                                                RichText::new(m.priority.label())
                                                                    .size(11.0)
                                                                    .color(
                                                                        if m.priority
                                                                            == MemoPriority::High
                                                                        {
                                                                            theme::danger()
                                                                        } else {
                                                                            theme::text_muted()
                                                                        },
                                                                    ),
                                                            );
                                                        }
                                                        for tag in m.tags.iter().take(3) {
                                                            tag_pill(ui, tag);
                                                        }
                                                    }
                                                }
                                            },
                                        );
                                    });
                                    if is_hub && !m.content.is_empty() {
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(&m.content)
                                                .size(12.0)
                                                .color(theme::text_muted()),
                                        );
                                    } else if private_warn && !is_hub {
                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            Frame::none()
                                                .fill(theme::accent_soft())
                                                .rounding(Rounding::same(6.0))
                                                .inner_margin(Margin::symmetric(6.0, 1.0))
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        RichText::new("专属")
                                                            .size(10.5)
                                                            .color(theme::shell_accent()),
                                                    );
                                                });
                                            if show_backup_badge
                                                && m.visibility == MemoVisibility::Private
                                            {
                                                ui.label(
                                                    RichText::new("私")
                                                        .size(11.0)
                                                        .color(theme::text_muted()),
                                                );
                                            }
                                        });
                                    }
                                })
                                .response
                                .interact(Sense::click());
                            if let Some(c) = stripe {
                                let r = resp.rect;
                                let bar = egui::Rect::from_min_max(
                                    egui::pos2(r.left() + 1.0, r.top() + 6.0),
                                    egui::pos2(r.left() + 4.5, r.bottom() - 6.0),
                                );
                                ui.painter().rect_filled(bar, Rounding::same(2.0), c);
                            }
                            if private_warn
                                && gender_ui.focus_overdue
                                && is_od
                                && !focus_scrolled
                            {
                                resp.scroll_to_me(Some(egui::Align::TOP));
                                focus_scrolled = true;
                                gender_ui.focus_overdue = false;
                            }
                            if resp.clicked() && !toggled_done {
                                if is_hub {
                                    *selected = Some(m.id.clone());
                                    *editing = false;
                                } else {
                                    load_selection(
                                        m,
                                        selected,
                                        title_draft,
                                        body_draft,
                                        visibility_draft,
                                        category_draft,
                                        due_date_draft,
                                        end_date_draft,
                                        tags_draft,
                                        done_draft,
                                        priority_draft,
                                        editing,
                                        memo_doc,
                                        edit_form,
                                    );
                                }
                            }
                            if !is_hub {
                                resp.context_menu(|ui| {
                                    if ui.button("打开").clicked() {
                                        load_selection(
                                            m,
                                            selected,
                                            title_draft,
                                            body_draft,
                                            visibility_draft,
                                            category_draft,
                                            due_date_draft,
                                            end_date_draft,
                                            tags_draft,
                                            done_draft,
                                            priority_draft,
                                            editing,
                                            memo_doc,
                                            edit_form,
                                        );
                                        ui.close_menu();
                                    }
                                    if ui
                                        .add(egui::Button::new(
                                            RichText::new("移入回收站…").color(theme::danger()),
                                        ))
                                        .clicked()
                                    {
                                        *selected = Some(m.id.clone());
                                        *title_draft = m.title.clone();
                                        *show_delete = true;
                                        ui.close_menu();
                                    }
                                });
                            }
                            ui.add_space(6.0);
                        }
                    });
            },
        );

        ui.allocate_ui_with_layout(
            Vec2::new(detail_w, full_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(full_h);
                ui.set_max_height(full_h);
                ui.set_max_width(detail_w);
                ui.set_clip_rect(ui.max_rect());
                show_detail(
                    ui,
                    svc,
                    filtered,
                    all_memos,
                    gender_ui,
                    selected,
                    title_draft,
                    body_draft,
                    visibility_draft,
                    category_draft,
                    due_date_draft,
                    end_date_draft,
                    tags_draft,
                    done_draft,
                    priority_draft,
                    editing,
                    memo_doc,
                    edit_form,
                    status_line,
                    show_delete,
                    show_export,
                    export_pw,
                    export_path,
                    export_ids,
                    show_history,
                    history_entity,
                    history_events,
                    history_sel_a,
                    history_sel_b,
                    data_dir,
                    tx,
                );
            },
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn show_detail(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    filtered: &[MemoView],
    all_memos: &[MemoView],
    gender_ui: &mut GenderPrivateUi,
    selected: &mut Option<String>,
    title_draft: &mut String,
    body_draft: &mut String,
    visibility_draft: &mut MemoVisibility,
    category_draft: &mut MemoCategory,
    due_date_draft: &mut String,
    end_date_draft: &mut String,
    tags_draft: &mut String,
    done_draft: &mut bool,
    priority_draft: &mut MemoPriority,
    editing: &mut bool,
    memo_doc: &mut doc_editor::Doc,
    edit_form: &mut MemoFormState,
    status_line: &mut String,
    show_delete: &mut bool,
    show_export: &mut bool,
    export_pw: &mut String,
    export_path: &mut String,
    export_ids: &mut Option<Vec<String>>,
    show_history: &mut bool,
    history_entity: &mut String,
    history_events: &mut Vec<memo_core::service::HistoryEvent>,
    history_sel_a: &mut Option<usize>,
    history_sel_b: &mut Option<usize>,
    data_dir: &str,
    tx: &Sender<BgMsg>,
) {
    ui.set_clip_rect(ui.max_rect());
    ui.set_max_width((ui.max_rect().width() - 6.0).max(80.0));
    if selected.as_ref().is_some_and(|id| is_new_draft(id)) {
        *selected = None;
        *editing = false;
    }
    if selected.is_none() {
        ui.centered_and_justified(|ui| {
            ui.label(
                theme::muted_label("点月历或列表中的一条查看详情\n或点「+」新建（弹窗填写）")
                    .size(15.0),
            );
        });
        return;
    }
    let id = selected.clone().unwrap();
    if id == HUB_GENDER_PRIVATE {
        gender_private_view::show(ui, svc, gender_ui, all_memos, status_line);
        return;
    }
    let meta = filtered.iter().find(|m| m.id == id);

    ui.horizontal_wrapped(|ui| {
        if !*editing {
            if theme::primary_button(ui, "编辑").clicked() {
                *editing = true;
                *edit_form = MemoFormState::load_from_drafts(
                    *category_draft,
                    title_draft,
                    body_draft,
                    due_date_draft,
                    end_date_draft,
                    *priority_draft,
                    tags_draft,
                    *done_draft,
                    *visibility_draft,
                    meta.map(|m| m.remind_before_days).unwrap_or(0),
                );
                *status_line = "已进入编辑模式，修改后请点「保存」".into();
            }
        } else {
            if theme::success_button(ui, "保存").clicked() {
                match memo_form::validate_title(edit_form) {
                    Ok((t, b)) => {
                        *title_draft = t.clone();
                        *body_draft = b.clone();
                        *category_draft = edit_form.category.canonical();
                        *due_date_draft = edit_form.due_date.clone();
                        *end_date_draft = edit_form.end_date.clone();
                        *priority_draft = edit_form.priority;
                        *tags_draft = edit_form.tags.clone();
                        *done_draft = edit_form.done;
                        *visibility_draft = edit_form.visibility;
                        *memo_doc = edit_form.doc.clone();
                        let vis = edit_form.visibility;
                        let cat = edit_form.category.canonical();
                        let due = edit_form.due_date.clone();
                        let end = edit_form.end_date.clone();
                        let done = edit_form.done;
                        let tags = memo_form::tags_vec(edit_form);
                        let prio = edit_form.priority;
                        let remind = edit_form.remind_before_days;
                        let seen = meta
                            .map(|m| {
                                if m.due_date.trim() == due.trim() {
                                    m.remind_seen_for.clone()
                                } else {
                                    String::new()
                                }
                            })
                            .unwrap_or_default();
                        let svc = svc.clone();
                        let id2 = id.clone();
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            match svc.edit_full(
                                &id2,
                                &t,
                                &b,
                                vis,
                                MemoLifecycle::Permanent,
                                cat,
                                &due,
                                &end,
                                done,
                                &tags,
                                prio,
                                remind,
                                &seen,
                            ) {
                                Ok(()) => {
                                    let _ = tx.send(BgMsg::Info("已保存".into()));
                                    let _ = tx.send(BgMsg::Refresh);
                                }
                                Err(e) => {
                                    let _ = tx.send(BgMsg::Error(e.to_string()));
                                }
                            }
                        });
                        *editing = false;
                    }
                    Err(e) => {
                        *status_line = e;
                        edit_form.request_title_focus();
                    }
                }
            }
            if theme::ghost_button(ui, "取消编辑").clicked() {
                *editing = false;
                if let Some(m) = meta {
                    *title_draft = m.title.clone();
                    *body_draft = m.content.clone();
                    *visibility_draft = m.visibility;
                    *category_draft = m.category.canonical();
                    *due_date_draft = m.due_date.clone();
                    *end_date_draft = m.end_date.clone();
                    *tags_draft = format_tags(&m.tags);
                    *done_draft = m.done;
                    *priority_draft = m.priority;
                    *memo_doc = doc_editor::Doc::from_store(&m.title, &m.content);
                    *edit_form = MemoFormState::load_from_drafts(
                        m.category.canonical(),
                        &m.title,
                        &m.content,
                        &m.due_date,
                        &m.end_date,
                        m.priority,
                        tags_draft,
                        m.done,
                        m.visibility,
                        m.remind_before_days,
                    );
                }
            }
        }
        ui.separator();
        if theme::ghost_button(ui, "复制全文").clicked() {
            ui.output_mut(|o| o.copied_text = format!("{}\n\n{}", title_draft, body_draft));
            *status_line = "已复制到剪贴板".into();
        }
        if theme::ghost_button(ui, "导出").clicked() {
            let dir = std::path::PathBuf::from(data_dir).join("exports");
            let path = dir.join(format!(
                "memo-{}-{}.txt",
                &id[..8.min(id.len())],
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            *export_ids = Some(vec![id.clone()]);
            *export_path = path.display().to_string();
            export_pw.clear();
            *show_export = true;
        }
        if theme::ghost_button(ui, "历史").clicked() {
            match svc.history_for("memo", &id) {
                Ok(ev) => {
                    *history_entity = "memo".into();
                    *history_events = ev;
                    *history_sel_a = None;
                    *history_sel_b = None;
                    *show_history = true;
                }
                Err(e) => *status_line = format!("读取历史失败: {e}"),
            }
        }
        if theme::danger_button(ui, "删除").clicked() {
            *show_delete = true;
        }
    });

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(8.0);

    let mut body_scroll = egui::ScrollArea::vertical()
        .id_source("shell_memo_body")
        .auto_shrink([false, false]);
    if *editing && edit_form.force_scroll_top() {
        body_scroll = body_scroll.vertical_scroll_offset(0.0);
        edit_form.tick_force_scroll_top();
    }
    body_scroll.show(ui, |ui| {
            ui.set_max_width((ui.max_rect().width() - 6.0).max(80.0));
            if *editing {
                memo_form::show_fields(ui, edit_form, FormMode::Edit, "shell_edit_form");
            } else {
                let title_show = if title_draft.is_empty() {
                    "(无标题)"
                } else {
                    title_draft.as_str()
                };
                let is_gender = category_draft.is_gender_private();
                ui.horizontal(|ui| {
                    let mut heading = RichText::new(title_show).size(22.0).color(theme::text());
                    if *done_draft {
                        heading = heading.strikethrough();
                    }
                    ui.heading(heading);
                    Frame::none()
                        .fill(theme::panel())
                        .rounding(Rounding::same(6.0))
                        .inner_margin(Margin::symmetric(8.0, 2.0))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("只读")
                                    .size(11.0)
                                    .color(theme::text_muted()),
                            );
                        });
                });
                if let Some(m) = meta {
                    let src = svc.display_name_for(&m.node_id);
                    let vis = if m.visibility == MemoVisibility::Private {
                        format!("🔒 {}", m.visibility.label())
                    } else {
                        m.visibility.label().to_string()
                    };
                    ui.label(
                        RichText::new(format!(
                            "{} · {} · v{} · {} · {}",
                            m.category.label(),
                            m.priority.label(),
                            m.version,
                            src,
                            vis
                        ))
                        .small()
                        .color(theme::text_muted()),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    if *done_draft {
                        ui.label(RichText::new("已完成").small().color(theme::success()));
                    }
                    let tags = parse_tags(tags_draft);
                    for t in &tags {
                        tag_pill(ui, t);
                    }
                });

                // 到期横幅（过期 / 今天 / 即将）
                let due_l = if due_date_draft.is_empty() {
                    String::new()
                } else {
                    let d = memo_core::display_due(due_date_draft);
                    if !end_date_draft.trim().is_empty() {
                        format!("{d} ～ {}", end_date_draft.trim())
                    } else {
                        d
                    }
                };
                let od = memo_core::is_overdue(due_date_draft, end_date_draft, *done_draft);
                let dt = memo_core::is_due_today(due_date_draft, end_date_draft, *done_draft);
                let soon = memo_core::is_due_soon(
                    due_date_draft,
                    end_date_draft,
                    *done_draft,
                    PRIVATE_DUE_SOON_DAYS,
                );
                let days_left =
                    memo_core::days_until_due(due_date_draft, end_date_draft, *done_draft);
                if od || dt || soon {
                    ui.add_space(8.0);
                    let (bg, fg, border, line) = if od {
                        let n = memo_core::overdue_days(
                            due_date_draft,
                            end_date_draft,
                            *done_draft,
                        )
                        .unwrap_or(1);
                        (
                            theme::warn_overdue_bg(),
                            theme::warn_overdue_fg(),
                            theme::warn_overdue_border(),
                            format!("⚠ 到期 {due_l} (已逾期 {n} 天)"),
                        )
                    } else if dt {
                        (
                            theme::warn_due_today_bg(),
                            theme::warn_due_today_fg(),
                            theme::warn_due_today_border(),
                            format!("⏰ 到期 {due_l} (今天到期)"),
                        )
                    } else {
                        let n = days_left.unwrap_or(1);
                        (
                            theme::warn_due_soon_bg(),
                            theme::warn_due_soon_fg(),
                            theme::warn_due_soon_border(),
                            format!("⏳ 到期 {due_l} (还有 {n} 天)"),
                        )
                    };
                    Frame::none()
                        .fill(bg)
                        .stroke(Stroke::new(1.0, border))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::symmetric(12.0, 10.0))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(RichText::new(line).size(13.5).strong().color(fg));
                        });
                } else if !due_l.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("到期 {due_l}"))
                            .size(12.5)
                            .color(theme::text_muted()),
                    );
                }

                ui.add_space(12.0);
                ui.label(
                    RichText::new("备注内容")
                        .size(12.0)
                        .color(theme::text_muted()),
                );
                ui.add_space(4.0);

                let note_resp = Frame::none()
                    .fill(theme::panel())
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(14.0, 12.0))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if body_draft.trim().is_empty() {
                            ui.label(
                                RichText::new(
                                    "（仅本人可见。可记经期感受、体检、用药保健、情绪，或为伴侣留下的笔记）",
                                )
                                .size(13.5)
                                .italics()
                                .color(theme::text_muted()),
                            );
                        } else {
                            doc_editor::show_viewer_body(ui, body_draft);
                        }
                        if is_gender {
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                let (bar, _) =
                                    ui.allocate_exact_size(Vec2::new(3.0, 36.0), Sense::hover());
                                ui.painter().rect_filled(
                                    bar,
                                    Rounding::same(1.5),
                                    theme::border(),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(
                                        "经期日与体检日请在「性别私密」专属页设置；系统提醒仅供参考，不能替代就医。",
                                    )
                                    .size(12.0)
                                    .color(theme::text_muted()),
                                );
                            });
                        }
                    });
                theme::paint_dashed_rect(
                    ui.painter(),
                    note_resp.response.rect,
                    Stroke::new(1.2, theme::border()),
                );
            }
        });
}
