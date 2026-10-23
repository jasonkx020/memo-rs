//! 主界面壳层：顶栏页签 + 内容区 + 右侧节点树。

use crate::calendar_view::{self, CalendarUi};
use crate::doc_editor;
use crate::gender_private_view::{self, GenderPrivateUi};
use crate::male_view;
use crate::memo_form::{self, FormMode, MemoFormState};
use crate::msg::BgMsg;
use crate::nav::NavItem;
use crate::page_shell;
use crate::period_view::{self, PeriodCareRemind};
use crate::theme;
use chrono::{Datelike, Timelike};
use eframe::egui::{
    self, Color32, Frame, Id, Margin, Order, RichText, Rounding, Sense, Stroke, Vec2,
};
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

/// 新建表单抽屉固定 anim id（勿按 nav 分裂，否则切页签残留 tween 易崩）。
pub const NEW_MEMO_DRAWER_SALT: &str = "memo_new_drawer";

pub fn begin_new_memo(
    show_new_memo: &mut bool,
    editing: &mut bool,
    edit_form: &mut MemoFormState,
    nav: NavItem,
    calendar_ymd: Option<&str>,
    data_dir: &str,
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
    edit_form.data_dir = Some(std::path::PathBuf::from(data_dir));
    *editing = false;
    *show_new_memo = true;
}

fn hub_memo_for(nav: NavItem) -> Option<MemoView> {
    if !nav.is_gender_private() {
        return None;
    }
    Some(MemoView {
        id: HUB_GENDER_PRIVATE.into(),
        title: "关怀与健康".into(),
        content: "一张日历叠画经期与体检；模块可分开开关。其下可再建其它私密备忘。".into(),
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
        tags: vec!["日历".into()],
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

/// 备忘页列表筛选（会话态，不落盘）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CredFilter {
    #[default]
    All,
    /// 账号证件
    TypeCredentials,
    TypeWork,
    TypeLife,
    TypeFinance,
    TypeInspiration,
    TypeEmergency,
    /// 随手记
    TypeGeneral,
    /// 高优先级视为置顶
    Pinned,
    Tagged,
    PrivateOnly,
}

/// 备忘页列表排序（会话态，不落盘）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CredSort {
    #[default]
    Updated,
    Created,
    Title,
}

pub struct ShellUi {
    pub nav: NavItem,
    pub gender: GenderPrivateUi,
    pub show_nodes: bool,
    pub cal: CalendarUi,
    pub cred_filter: CredFilter,
    pub cred_sort: CredSort,
    /// 备忘页按具体标签过滤（None = 不限）；文本搜索用顶栏
    pub cred_tag: Option<String>,
    /// CentralPanel 内容区，供备忘新建/编辑抽屉锚定
    pub content_host: egui::Rect,
    /// 顶栏告警上次按自然日重算的日期（常开跨天检测）
    pub last_nav_warn_ymd: String,
    /// 表单 / 详情抽屉宽度（按身份持久化）
    pub drawer: page_shell::DrawerWidths,
}

impl Default for ShellUi {
    fn default() -> Self {
        Self {
            nav: NavItem::Calendar,
            gender: GenderPrivateUi::default(),
            show_nodes: false,
            cal: CalendarUi::default(),
            cred_filter: CredFilter::All,
            cred_sort: CredSort::Updated,
            cred_tag: None,
            content_host: egui::Rect::NOTHING,
            last_nav_warn_ymd: String::new(),
            drawer: page_shell::DrawerWidths::default(),
        }
    }
}

/// 备忘页可承载的档案分类（排除待办、性别私密）。
fn is_memo_archive_cat(c: MemoCategory) -> bool {
    matches!(
        c.canonical(),
        MemoCategory::Credentials
            | MemoCategory::Work
            | MemoCategory::Life
            | MemoCategory::Finance
            | MemoCategory::Inspiration
            | MemoCategory::Emergency
            | MemoCategory::General
    )
}

fn cred_filter_category(filter: CredFilter) -> Option<MemoCategory> {
    match filter {
        CredFilter::TypeCredentials => Some(MemoCategory::Credentials),
        CredFilter::TypeWork => Some(MemoCategory::Work),
        CredFilter::TypeLife => Some(MemoCategory::Life),
        CredFilter::TypeFinance => Some(MemoCategory::Finance),
        CredFilter::TypeInspiration => Some(MemoCategory::Inspiration),
        CredFilter::TypeEmergency => Some(MemoCategory::Emergency),
        CredFilter::TypeGeneral => Some(MemoCategory::General),
        _ => None,
    }
}

/// 备忘表单抽屉操作结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoDrawerAction {
    None,
    Save,
    Dismiss,
}

/// 任务 / 备忘 / 私密：预览与编辑共用同一侧栏抽屉（勿再叠第二层表单抽屉）。
pub fn nav_uses_unified_side_drawer(nav: NavItem) -> bool {
    nav.is_calendar() || nav.is_credentials() || nav.is_gender_private()
}

/// 统一抽屉内的编辑表单正文（滚动区内）。
fn show_unified_edit_fields(
    ui: &mut egui::Ui,
    edit_form: &mut MemoFormState,
    data_dir: &str,
    id_salt: &str,
) {
    if edit_form.data_dir.is_none() {
        edit_form.data_dir = Some(std::path::PathBuf::from(data_dir));
    }
    if edit_form.attach_id.trim().is_empty() {
        edit_form.attach_id = uuid::Uuid::new_v4().to_string().replace('-', "");
    }
    memo_form::show_fields(ui, edit_form, FormMode::Edit, id_salt);
}

/// 统一抽屉底栏：保存 / 取消（取消回预览，不关抽屉）。
fn show_unified_edit_footer(
    ui: &mut egui::Ui,
    want_save: &mut bool,
    want_cancel: &mut bool,
) {
    ui.horizontal(|ui| {
        if theme::page_primary_button(ui, "保存").clicked() {
            *want_save = true;
        }
        if theme::page_outline_button(ui, "取消").clicked() {
            *want_cancel = true;
        }
    });
}

fn parse_modified_at(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(t).ok()
}

fn format_relative_modified(s: &str) -> String {
    let Some(dt) = parse_modified_at(s) else {
        let t = s.trim();
        if t.is_empty() {
            return "—".into();
        }
        return t.chars().take(16).collect();
    };
    let local = dt.with_timezone(&chrono::Local);
    let today = chrono::Local::now().date_naive();
    let d = local.date_naive();
    let days = (today - d).num_days();
    if days == 0 {
        format!("今天 {:02}:{:02}", local.hour(), local.minute())
    } else if days == 1 {
        format!("昨天 {:02}:{:02}", local.hour(), local.minute())
    } else if days >= 0 && days <= 7 {
        format!("{}月{}日", d.month(), d.day())
    } else {
        format!("{}月{}日", d.month(), d.day())
    }
}

fn memo_excerpt(content: &str, max_chars: usize) -> Option<String> {
    let t = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (i, ch) in t.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    Some(out)
}

fn apply_cred_list_view(
    list: &mut Vec<MemoView>,
    filter: CredFilter,
    sort: CredSort,
    tag_filter: Option<&str>,
) {
    let tag_f = tag_filter.map(|t| t.trim()).filter(|t| !t.is_empty());
    list.retain(|m| {
        if is_private_hub(&m.id) || is_new_draft(&m.id) {
            return false;
        }
        let type_ok = if let Some(cat) = cred_filter_category(filter) {
            m.category.canonical() == cat
        } else {
            match filter {
                CredFilter::All => true,
                CredFilter::Pinned => m.priority == MemoPriority::High,
                CredFilter::Tagged => !m.tags.is_empty(),
                CredFilter::PrivateOnly => m.visibility == MemoVisibility::Private,
                _ => true,
            }
        };
        if !type_ok {
            return false;
        }
        if let Some(t) = tag_f {
            if !m.tags.iter().any(|x| x == t) {
                return false;
            }
        }
        true
    });
    // 置顶（高优先级）沉在最前，再按所选排序
    list.sort_by(|a, b| {
        let pa = (a.priority == MemoPriority::High) as u8;
        let pb = (b.priority == MemoPriority::High) as u8;
        pb.cmp(&pa).then_with(|| match sort {
            CredSort::Updated => {
                let ka = parse_modified_at(&a.modified_at).map(|d| d.timestamp());
                let kb = parse_modified_at(&b.modified_at).map(|d| d.timestamp());
                kb.cmp(&ka).then_with(|| b.version.cmp(&a.version))
            }
            CredSort::Created => a.version.cmp(&b.version),
            CredSort::Title => {
                let ta = if a.title.is_empty() {
                    "(无标题)"
                } else {
                    a.title.as_str()
                };
                let tb = if b.title.is_empty() {
                    "(无标题)"
                } else {
                    b.title.as_str()
                };
                ta.cmp(tb)
            }
        })
    });
}

/// 壳层交互回传。
#[derive(Debug, Clone, Default)]
pub struct ShellAction {
    pub switch: bool,
    /// 从私密页锁定：已切到任务页，下一帧再执行锁定
    pub defer_lock_after_task: bool,
    pub sync: bool,
    pub open_new_memo: bool,
    pub backup_dirty: bool,
    /// 右键「生成桌面便签」的备忘 id
    pub open_sticky: Option<String>,
}

/// 若在私密页（顶栏或置顶入口），切回任务页并清选中；返回是否曾在私密。
pub fn leave_private_for_lock(
    shell: &mut ShellUi,
    selected: &mut Option<String>,
    editing: &mut bool,
) -> bool {
    let on_private = shell.nav.is_gender_private()
        || selected.as_deref() == Some(HUB_GENDER_PRIVATE);
    if on_private {
        shell.nav = NavItem::Calendar;
        *selected = None;
        *editing = false;
    }
    on_private
}

fn request_lock(
    shell: &mut ShellUi,
    selected: &mut Option<String>,
    editing: &mut bool,
    action: &mut ShellAction,
) {
    if leave_private_for_lock(shell, selected, editing) {
        action.defer_lock_after_task = true;
    } else {
        action.switch = true;
    }
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

/// 顶栏控件统一外高（分段底 / 搜索 / 工具托盘一致）。
const TOP_BAR_CTRL_H: f32 = 30.0;
/// 分段内项高（页签、图标钮）。
const TOP_BAR_ITEM_H: f32 = 26.0;
/// 分段底内边距：外高 = ITEM + 2*PAD。
const TOP_BAR_SEG_PAD: f32 = 2.0;

/// 顶栏身份入口：头像 + 名称 + 轻量箭头，合成一颗可点药丸。
fn show_identity_chip(ui: &mut egui::Ui, alias: &str) -> egui::Response {
    let letter = avatar_letter(alias);
    let av_idx = alias
        .bytes()
        .fold(0usize, |a, b| a.wrapping_add(b as usize));
    let av_color = theme::avatar_color(av_idx);
    let hover_id = ui.id().with("identity_chip_hover");
    let hovered = ui.memory(|m| m.data.get_temp::<bool>(hover_id).unwrap_or(false));
    let fill = if hovered {
        theme::shell_accent_soft()
    } else {
        Color32::from_rgb(250, 251, 252)
    };
    let stroke_c = if hovered {
        Color32::from_rgb(184, 206, 248)
    } else {
        theme::border()
    };
    let pad_v = ((TOP_BAR_CTRL_H - TOP_BAR_ITEM_H) * 0.5).max(0.0);
    let inner = Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke_c))
        .rounding(Rounding::same(999.0))
        .inner_margin(Margin {
            left: 3.0,
            right: 8.0,
            top: pad_v,
            bottom: pad_v,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.set_min_height(TOP_BAR_ITEM_H);
                ui.set_max_height(TOP_BAR_ITEM_H);
                let av_sz = 20.0;
                let (av, _) = ui.allocate_exact_size(Vec2::splat(av_sz), Sense::hover());
                ui.painter().circle_filled(av.center(), av_sz * 0.5, av_color);
                ui.painter().text(
                    av.center(),
                    egui::Align2::CENTER_CENTER,
                    &letter,
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
                let display = if alias.chars().count() > 8 {
                    let t: String = alias.chars().take(7).collect();
                    format!("{t}…")
                } else {
                    alias.to_string()
                };
                ui.label(
                    RichText::new(display)
                        .size(12.5)
                        .strong()
                        .color(theme::text()),
                );
                ui.label(
                    RichText::new("▾")
                        .size(10.0)
                        .color(theme::text_muted()),
                );
            });
        });
    let resp = inner
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let now = resp.hovered();
    ui.memory_mut(|m| m.data.insert_temp(hover_id, now));
    if now != hovered {
        ui.ctx().request_repaint();
    }
    resp
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
                    // 任务页：排除性别私密与账号证件；无到期的档案分类归备忘页
                    let c = m.category.canonical();
                    !c.is_gender_private()
                        && c != MemoCategory::Credentials
                        && !(is_memo_archive_cat(c) && m.due_date.trim().is_empty())
                }
                NavItem::Credentials => {
                    // 备忘页：账号证件，或其它档案分类且无到期（不上日历）
                    let c = m.category.canonical();
                    c == MemoCategory::Credentials
                        || (is_memo_archive_cat(c) && m.due_date.trim().is_empty())
                }
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

#[allow(dead_code)]
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
                let g = theme::layout_galley(ui, title, egui::FontId::proportional(12.0), fg);
                let (rect, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                ui.painter().galley(
                    theme::galley_pos_left_center(rect.left_center(), &g),
                    g,
                    fg,
                );
                ui.label(RichText::new(format!("{n}")).size(11.0).color(fg));
            });
        });
    ui.add_space(4.0);
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

fn top_icon_button(ui: &mut egui::Ui, kind: TopIcon, tip: &str) -> egui::Response {
    top_icon_button_on(ui, kind, tip, false)
}

/// 顶栏工具：仅图标，悬停提示；选中态白底。
fn top_icon_button_on(
    ui: &mut egui::Ui,
    kind: TopIcon,
    tip: &str,
    on: bool,
) -> egui::Response {
    let size = Vec2::splat(TOP_BAR_ITEM_H);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fill = if on || hovered {
        theme::card()
    } else {
        Color32::TRANSPARENT
    };
    if fill.a() > 0 {
        ui.painter()
            .rect(rect, Rounding::same(6.0), fill, Stroke::NONE);
    }
    let fg = if on {
        theme::shell_accent()
    } else if hovered {
        match kind {
            TopIcon::Lock => theme::warn(),
            TopIcon::Sync => theme::success(),
            _ => theme::shell_accent(),
        }
    } else {
        theme::text_muted()
    };
    paint_top_icon(ui.painter(), rect.center(), kind, fg);
    resp.on_hover_text(tip)
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
    let color = if draft_empty_title {
        theme::warn()
    } else {
        theme::success()
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    if draft_empty_title {
        ui.painter().circle_filled(rect.center(), 3.5, color);
    } else {
        // 用笔画勾代替 ✓，避免 dingbat 在部分 CJK 字体下落成空方框
        let c = rect.center();
        let stroke = Stroke::new(1.8, color);
        ui.painter().line_segment(
            [egui::pos2(c.x - 4.0, c.y), egui::pos2(c.x - 1.0, c.y + 3.0)],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(c.x - 1.0, c.y + 3.0),
                egui::pos2(c.x + 4.5, c.y - 3.5),
            ],
            stroke,
        );
    }
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
    // 新建抽屉打开中：与 editing 一起锁定顶栏页签
    show_new_memo: bool,
    show_settings: &mut bool,
    show_change_pw: &mut bool,
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
    local_node_id: &str,
    node_role: NodeRole,
    node_visible: bool,
    lan_discovery: bool,
    local_disk: DiskSpace,
    backup_targets: &mut Vec<String>,
    tx: &Sender<BgMsg>,
    // 有新版本时顶栏提示（版本号）；None 不显示
    update_hint_ver: Option<&str>,
    update_hint_url: Option<&str>,
) -> ShellAction {
    let nav_locked = show_new_memo || *editing;
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

    // 经期 / 体检关怀 + 任务/私密过期 → 顶栏页签脉冲
    let mut women_care = period_view::care_remind(svc, &all);
    let mut male_care = male_view::care_remind(svc);
    // 关怀已读：进入私密页即标记今日已读（窗口内次日再提示）
    if shell.nav.is_gender_private() {
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
    let task_overdue_n = calendar_view::task_overdue_count(&all);
    let private_overdue_n = all
        .iter()
        .filter(|m| {
            m.category.is_gender_private()
                && !is_private_hub(&m.id)
                && !is_private_calendar_memo(m)
                && memo_is_overdue(m)
        })
        .count();
    let care_needs = women_care
        .as_ref()
        .is_some_and(|r| r.needs_attention())
        || male_care.as_ref().is_some_and(|r| r.needs_attention());
    let task_warn = task_overdue_n > 0 && !shell.nav.is_calendar();
    let private_warn =
        (care_needs || private_overdue_n > 0) && !shell.nav.is_gender_private();
    let pulse_u = {
            let t = ctx.input(|i| i.time);
        let u = (t * 2.2).sin() as f32 * 0.5 + 0.5;
        0.35 + u * 0.65
    };
    let task_pulse = task_warn.then_some(pulse_u);
    let private_pulse = private_warn.then_some(pulse_u);
    if task_warn || private_warn {
        ctx.request_repaint_after(Duration::from_millis(80));
    }

    // —— 顶栏 ——
    egui::TopBottomPanel::top("shell_top")
        .exact_height(38.0)
        .frame(
            Frame::none()
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::border()))
                .inner_margin(Margin::symmetric(10.0, 4.0)),
        )
        .show(ctx, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_height(ui.available_height());
                ui.set_max_height(ui.available_height());
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.spacing_mut().item_spacing.y = 0.0;

                let identity_resp = show_identity_chip(ui, &alias_show);
                let identity_menu_id = ui.make_persistent_id("shell_identity_menu");
                if identity_resp.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(identity_menu_id));
                }
                egui::popup_below_widget(ui, identity_menu_id, &identity_resp, |ui| {
                    ui.set_min_width(148.0);
                        if ui.button("设置").clicked() {
                            *show_settings = true;
                        ui.memory_mut(|m| m.close_popup());
                        }
                        if ui.button("切换身份").clicked() {
                            request_lock(shell, selected, editing, &mut action);
                        ui.memory_mut(|m| m.close_popup());
                    }
                    if ui.button("重置密码").clicked() {
                        *show_change_pw = true;
                        ui.memory_mut(|m| m.close_popup());
                        }
                        if ui.button("帮助").clicked() {
                            *show_help = true;
                        ui.memory_mut(|m| m.close_popup());
                        }
                        if ui.button("关于").clicked() {
                            *show_about = true;
                        ui.memory_mut(|m| m.close_popup());
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
                        ui.memory_mut(|m| m.close_popup());
                        }
                });

                show_top_tabs(
                    ui,
                    shell,
                    &all,
                    svc.list_trash().len(),
                    task_pulse,
                    private_pulse,
                    task_overdue_n,
                    private_overdue_n,
                    care_needs,
                    selected,
                    editing,
                    nav_locked,
                );

                if let (Some(ver), Some(url)) = (update_hint_ver, update_hint_url) {
                    show_update_hint(ui, ver, url);
                }

                // 右侧：搜索 + 图标工具托盘（外高与页签分段一致）
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.spacing_mut().item_spacing.y = 0.0;
                    Frame::none()
                        .fill(Color32::from_rgb(238, 241, 246))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::same(TOP_BAR_SEG_PAD))
                        .show(ui, |ui| {
                            ui.set_min_height(TOP_BAR_ITEM_H);
                            ui.set_max_height(TOP_BAR_ITEM_H);
                            ui.spacing_mut().item_spacing.x = 2.0;
                            ui.spacing_mut().item_spacing.y = 0.0;
                            // RTL：锁定 → 设置 → 节点 → 同步
                            if top_icon_button(ui, TopIcon::Lock, "锁定").clicked() {
                                request_lock(shell, selected, editing, &mut action);
                            }
                            if top_icon_button(ui, TopIcon::Settings, "设置").clicked() {
                                *show_settings = true;
                            }
                            if top_icon_button_on(
                                ui,
                                TopIcon::Nodes,
                                if shell.show_nodes {
                                    "隐藏右侧节点树"
                                } else {
                                    "显示右侧节点树"
                                },
                                shell.show_nodes,
                            )
                            .clicked()
                            {
                                shell.show_nodes = !shell.show_nodes;
                            }
                            if top_icon_button(ui, TopIcon::Sync, "同步").clicked() {
                                action.sync = true;
                            }
                        });

                    let search_w = page_shell::fit_avail_width(ui, ui.available_width().min(240.0));
                    let (search_rect, search_click) = ui
                        .allocate_exact_size(Vec2::new(search_w, TOP_BAR_CTRL_H), Sense::click());
                ui.painter().rect(
                    search_rect,
                        Rounding::same(10.0),
                        Color32::from_rgb(247, 249, 252),
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
                            "搜索备忘…",
                        font,
                        theme::text_muted(),
                    );
                }
                if search_click.clicked() {
                    te.request_focus();
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
            shell.content_host = bounds;
            ui.set_clip_rect(bounds);
            {
                let mut body = ui.child_ui(bounds, egui::Layout::top_down(egui::Align::Min));
                body.set_clip_rect(bounds);
                if let Some(r) = women_care.as_ref().filter(|r| r.needs_attention()) {
                    show_period_care_banner(
                        &mut body,
                        r,
                        &mut shell.nav,
                        selected,
                        editing,
                        nav_locked,
                    );
                    body.add_space(8.0);
                }
                if let Some(r) = male_care.as_ref().filter(|r| r.needs_attention()) {
                    show_male_care_banner(
                        &mut body,
                        r,
                        &mut shell.nav,
                        selected,
                        editing,
                        nav_locked,
                    );
                    body.add_space(8.0);
                }

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

    if shell.gender.request_open_settings {
        shell.gender.request_open_settings = false;
        *show_settings = true;
    }

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
                let detail_open = selected
                    .as_ref()
                    .map(|id| !is_new_draft(id) && all.iter().any(|m| m.id == *id))
                    .unwrap_or(false);
                    let mut picked = None;
                let mut pick_edit = false;
                let mut pick_delete = false;
                let host = ui.available_rect_before_wrap();
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
                    &mut action.open_sticky,
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
                            begin_memo_edit(
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
                                        edit_form,
                                        status_line,
                            );
                        }
                        if pick_delete {
                            *show_delete = true;
                        }
                    }
                }
                // 预览 / 编辑共用同一抽屉（同宽度、同动画，仅内容切换）
                if detail_open {
                    let m_owned = selected
                        .as_ref()
                        .and_then(|id| all.iter().find(|x| x.id == *id))
                        .cloned();
                    let mut want_edit = false;
                    let mut want_save = false;
                    let mut want_cancel = false;
                    let mut drawer_dirty = false;
                    let editing_now = *editing;
                    // 编辑态勿 Esc 关闭：须点「×」或「取消」
                    let drawer_title = if editing_now { "编辑任务" } else { "" };
                    let close = calendar_view::show_cal_drawer(
                        ui.ctx(),
                        host,
                        "task_side_drawer",
                        drawer_title,
                        !editing_now,
                        editing_now,
                        &mut shell.drawer.form_w,
                        &mut drawer_dirty,
                        |ui| {
                            if editing_now {
                                show_unified_edit_fields(
                                    ui,
                                    edit_form,
                                    data_dir,
                                    "task_side_edit",
                                );
                            } else if let Some(m) = m_owned.as_ref() {
                                want_edit = calendar_view::show_event_detail(
                                    ui,
                                    svc,
                                    m,
                                    selected,
                                    editing,
                                    show_delete,
                                    status_line,
                                    tx,
                                );
                            }
                        },
                        |ui| {
                            if editing_now {
                                show_unified_edit_footer(ui, &mut want_save, &mut want_cancel);
                            }
                        },
                    );
                    if drawer_dirty {
                        shell.drawer.detail_w = shell.drawer.form_w;
                        shell.drawer.mark_dirty();
                    }
                    if want_edit {
                        begin_memo_edit(
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
                            edit_form,
                            status_line,
                        );
                    }
                    if want_save {
                        save_memo_edit(
                            svc,
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
                            tx,
                        );
                    }
                    if want_cancel {
                        cancel_memo_edit(
                            selected,
                            &all,
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
                    if close {
                        if *editing {
                            cancel_memo_edit(
                                selected,
                                &all,
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
                        *selected = None;
                        *editing = false;
                    } else if let Some(id) = selected.clone() {
                        if let Some(m) = all.iter().find(|m| m.id == id) {
                            if !*editing && title_draft != &m.title {
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
                let mut filtered = filter_memos(shell.nav, search, &all);
                if shell.nav.is_credentials() {
                    apply_cred_list_view(
                        &mut filtered,
                        shell.cred_filter,
                        shell.cred_sort,
                        shell.cred_tag.as_deref(),
                    );
                }
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
                    &mut shell.cred_filter,
                    &mut shell.cred_sort,
                    &mut shell.cred_tag,
                        &mut action.open_new_memo,
                        &mut action.open_sticky,
                        tx,
                    );
                // 备忘：预览 / 编辑同一抽屉
                if shell.nav.is_credentials() {
                    let memo_open = selected.as_ref().is_some_and(|id| {
                        !is_private_hub(id)
                            && !is_new_draft(id)
                            && filtered.iter().any(|m| m.id == *id)
                    });
                    if memo_open {
                        let host = shell.content_host;
                        let meta = selected
                            .as_ref()
                            .and_then(|id| filtered.iter().find(|m| m.id == *id));
                        let mut want_save = false;
                        let mut want_cancel = false;
                        let mut drawer_dirty = false;
                        let editing_now = *editing;
                        // 编辑态勿 Esc 关闭：须点「×」或「取消」
                        let drawer_title = if editing_now { "编辑备忘" } else { "" };
                        let close = calendar_view::show_cal_drawer(
                            ui.ctx(),
                            host,
                            "cred_side_drawer",
                            drawer_title,
                            !editing_now,
                            editing_now,
                            &mut shell.drawer.form_w,
                            &mut drawer_dirty,
                            |ui| {
                                if editing_now {
                                    show_unified_edit_fields(
                                        ui,
                                        edit_form,
                                        data_dir,
                                        "cred_side_edit",
                                    );
                                } else {
                                    show_cred_detail_readonly(
                                        ui,
                                        svc,
                                        meta,
                                        &filtered,
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
                                        edit_form,
                                        status_line,
                                        show_delete,
                                        &mut action.open_sticky,
                                    );
                                }
                            },
                            |ui| {
                                if editing_now {
                                    show_unified_edit_footer(
                                        ui,
                                        &mut want_save,
                                        &mut want_cancel,
                                    );
                                }
                            },
                        );
                        if drawer_dirty {
                            shell.drawer.detail_w = shell.drawer.form_w;
                            shell.drawer.mark_dirty();
                        }
                        if want_save {
                            save_memo_edit(
                                svc,
                                &filtered,
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
                                tx,
                            );
                        }
                        if want_cancel {
                            cancel_memo_edit(
                                selected,
                                &filtered,
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
                        if close {
                            if *editing {
                                cancel_memo_edit(
                                    selected,
                                    &filtered,
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
                            *selected = None;
                            *editing = false;
                        }
                    }
                }
                // 私密：预览 / 编辑同一抽屉
                if shell.nav.is_gender_private() {
                    let memo_open = selected.as_ref().is_some_and(|id| {
                        !is_private_hub(id)
                            && !is_new_draft(id)
                            && filtered.iter().any(|m| m.id == *id)
                    });
                    if memo_open {
                        let host = shell.content_host;
                        let mut want_save = false;
                        let mut want_cancel = false;
                        let mut drawer_dirty = false;
                        let editing_now = *editing;
                        // 编辑态勿 Esc 关闭：须点「×」或「取消」
                        let drawer_title = if editing_now { "编辑私密" } else { "" };
                        let close = calendar_view::show_cal_drawer(
                            ui.ctx(),
                            host,
                            "priv_side_drawer",
                            drawer_title,
                            !editing_now,
                            editing_now,
                            &mut shell.drawer.form_w,
                            &mut drawer_dirty,
                            |ui| {
                                if editing_now {
                                    show_unified_edit_fields(
                                        ui,
                                        edit_form,
                                        data_dir,
                                        "priv_side_edit",
                                    );
                                } else {
                                    show_private_memo_detail(
                                        ui,
                                        svc,
                                        &filtered,
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
                                    );
                                }
                            },
                            |ui| {
                                if editing_now {
                                    show_unified_edit_footer(
                                        ui,
                                        &mut want_save,
                                        &mut want_cancel,
                                    );
                                }
                            },
                        );
                        if drawer_dirty {
                            shell.drawer.detail_w = shell.drawer.form_w;
                            shell.drawer.mark_dirty();
                        }
                        if want_save {
                            save_memo_edit(
                                svc,
                                &filtered,
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
                                tx,
                            );
                        }
                        if want_cancel {
                            cancel_memo_edit(
                                selected,
                                &filtered,
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
                        if close {
                            if *editing {
                                cancel_memo_edit(
                                    selected,
                                    &filtered,
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
                            select_gender_hub(selected, editing);
                        }
                    }
                }
            }
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum BadgeKind {
    Gray,
    Blue,
    Soft,
}

/// 顶栏页签旁：有新版本时可点提示，打开官网下载页。
fn show_update_hint(ui: &mut egui::Ui, latest: &str, download_url: &str) {
    ui.add_space(6.0);
    let label = if latest.is_empty() {
        "有版本更新".to_string()
    } else {
        format!("有版本更新 {latest}")
    };
    let resp = ui.add(
        egui::Button::new(
            RichText::new(label)
                .size(12.0)
                .color(theme::shell_accent()),
        )
        .fill(theme::shell_accent_soft())
        .stroke(Stroke::new(1.0, theme::shell_accent().linear_multiply(0.35)))
        .rounding(Rounding::same(8.0))
        .frame(true)
        .min_size(Vec2::new(0.0, TOP_BAR_ITEM_H)),
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let tip = if latest.is_empty() {
        "发现新版本，点击前往官网下载".to_string()
    } else {
        format!("发现新版本 {latest}，点击前往官网下载")
    };
    let resp = resp.on_hover_text(tip);
    if resp.clicked() {
        crate::update_check::open_download_page(download_url);
    }
}

fn show_top_tabs(
    ui: &mut egui::Ui,
    shell: &mut ShellUi,
    all: &[MemoView],
    trash_n: usize,
    task_pulse: Option<f32>,
    private_pulse: Option<f32>,
    task_overdue_n: usize,
    private_overdue_n: usize,
    care_needs: bool,
    selected: &mut Option<String>,
    editing: &mut bool,
    nav_locked: bool,
) {
    let cred_n = all
        .iter()
        .filter(|m| {
            let c = m.category.canonical();
            c == MemoCategory::Credentials
                || (is_memo_archive_cat(c) && m.due_date.trim().is_empty())
        })
        .count();
    let priv_n = all
        .iter()
        .filter(|m| m.category.is_gender_private() && !is_private_calendar_memo(m))
        .count();

    // 分段底：统一页签容器（外高 = TOP_BAR_CTRL_H）
    Frame::none()
        .fill(Color32::from_rgb(238, 241, 246))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::same(TOP_BAR_SEG_PAD))
        .show(ui, |ui| {
            ui.set_min_height(TOP_BAR_ITEM_H);
            ui.set_max_height(TOP_BAR_ITEM_H);
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.set_min_height(TOP_BAR_ITEM_H);
                ui.set_max_height(TOP_BAR_ITEM_H);
    for item in NavItem::TABS {
        let sel = shell.nav == *item;
        let trash = item.is_trash();
        // 新建/编辑中：其它页签不可点；当前页仍高亮
        let tab_locked = nav_locked && !sel;
                    let h = TOP_BAR_ITEM_H;
        let label = if trash { "" } else { item.label() };
        let icon = item.icon();
        let badge = match item {
            NavItem::Credentials => cred_n,
            NavItem::GenderPrivate => priv_n,
            NavItem::Trash => trash_n,
            NavItem::Calendar => 0,
        };
                    let warn_p = if item.is_calendar() {
                        task_pulse
                    } else if item.is_gender_private() {
                        private_pulse
        } else {
                        None
        };
                    let warn = warn_p.is_some() && !sel && !tab_locked;
                    let icon_fg = if tab_locked {
                        theme::text_muted().linear_multiply(0.55)
                    } else if warn {
                        theme::warn_overdue_fg()
                    } else if sel {
            theme::shell_accent()
        } else {
                        theme::text_muted()
                    };
                    let label_fg = if tab_locked {
                        theme::text_muted().linear_multiply(0.55)
                    } else if warn {
                        theme::warn_overdue_fg()
                    } else if sel {
            theme::text()
                    } else {
                        theme::text_muted()
        };
        let icon_g = theme::layout_galley(
            ui,
            icon,
                        egui::FontId::proportional(13.0),
            icon_fg,
        );
                    let label_g = if label.is_empty() {
            None
        } else {
            Some(theme::layout_galley(
                ui,
                label,
                egui::FontId::proportional(13.0),
                label_fg,
            ))
        };
                    let icon_slot = 16.0_f32.max(icon_g.mesh_bounds.width());
                    let text_w = label_g.as_ref().map(|g| g.size().x).unwrap_or(0.0);
                    let badge_w = if badge > 0 { 20.0 } else { 0.0 };
        let w = if trash {
                        (8.0 + icon_slot + badge_w + 8.0).max(32.0)
        } else {
                        (10.0 + icon_slot + 6.0 + text_w + if badge > 0 { 6.0 + badge_w } else { 0.0 } + 10.0)
                            .max(52.0)
        };
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(w, h),
            if tab_locked {
                Sense::hover()
            } else {
                Sense::click()
            },
        );
        if resp.hovered() {
            ui.ctx().set_cursor_icon(if tab_locked {
                egui::CursorIcon::NotAllowed
            } else {
                egui::CursorIcon::PointingHand
            });
        }
                    // 选中 / 悬停 / 告警脉冲底
                    if sel {
                        ui.painter().rect(
                            rect,
                            Rounding::same(8.0),
                            theme::card(),
                            Stroke::new(1.0, Color32::from_black_alpha(12)),
                        );
                    } else if tab_locked {
                        // 禁用：无悬停高亮
                    } else if let Some(p) = warn_p {
                        let mut bg = theme::warn_overdue_bg();
                        let a = (90.0 + p * 140.0).clamp(60.0, 230.0) as u8;
                        bg = Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), a);
                        ui.painter().rect(
                            rect,
                            Rounding::same(8.0),
                            bg,
                            Stroke::new(1.0, theme::warn_overdue_fg().linear_multiply(0.35 + p * 0.55)),
                        );
                    } else if resp.hovered() {
                        ui.painter().rect(
                            rect,
                            Rounding::same(8.0),
                            Color32::from_white_alpha(140),
                            Stroke::NONE,
                        );
                    }
                    let cy = rect.center().y;
                    let mut x = rect.left() + if trash { 8.0 } else { 10.0 };
            ui.painter().galley(
                        theme::galley_pos_center(
                            egui::pos2(x + icon_slot * 0.5, cy),
                            &icon_g,
                        ),
                icon_g,
                icon_fg,
            );
                    x += icon_slot;
            if let Some(label_g) = label_g {
                        x += 6.0;
                ui.painter().galley(
                            theme::galley_pos_left_center(egui::pos2(x, cy), &label_g),
                    label_g,
                    label_fg,
                );
                        x += text_w;
            }
            if badge > 0 {
                        x += if trash { 4.0 } else { 6.0 };
                let br = egui::Rect::from_center_size(
                            egui::pos2(x + badge_w * 0.5, cy),
                            Vec2::new(badge_w.min(18.0).max(16.0), 16.0),
                        );
                        let (bbg, bfg) = if warn {
                            (theme::warn_overdue_bg(), theme::warn_overdue_fg())
                        } else if sel {
                            (theme::shell_accent_soft(), theme::shell_accent())
                        } else {
                            (Color32::from_rgb(228, 232, 239), theme::text_muted())
                        };
                ui.painter()
                            .rect_filled(br, Rounding::same(8.0), bbg);
                ui.painter().text(
                    br.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{badge}"),
                    egui::FontId::proportional(10.0),
                            bfg,
                        );
                    } else if warn {
                        // 无数量角标时仍画警示小圆点
                        let dot = egui::pos2(rect.right() - 7.0, rect.top() + 6.0);
                        let p = warn_p.unwrap_or(1.0);
                        let mut c = theme::warn_overdue_fg();
                        c = Color32::from_rgba_unmultiplied(
                            c.r(),
                            c.g(),
                            c.b(),
                            (140.0 + p * 115.0).clamp(120.0, 255.0) as u8,
                        );
                        ui.painter().circle_filled(dot, 3.2, c);
        }
        if !tab_locked && resp.clicked() {
            shell.nav = *item;
            if item.is_gender_private() {
                select_gender_hub(selected, editing);
            } else {
                *selected = None;
                *editing = false;
            }
        }
        if tab_locked {
            resp.on_hover_text("请先关闭新建或编辑");
        } else if trash {
            resp.on_hover_text("回收站");
                    } else if item.is_calendar() && task_overdue_n > 0 {
                        resp.on_hover_text(format!("有 {task_overdue_n} 条过期任务"));
                    } else if item.is_gender_private() && (care_needs || private_overdue_n > 0) {
                        let tip = match (care_needs, private_overdue_n > 0) {
                            (true, true) => format!(
                                "关怀提醒 · {private_overdue_n} 条过期私密"
                            ),
                            (true, false) => "有关怀提醒（经期/体检）".into(),
                            (false, true) => format!("有 {private_overdue_n} 条过期私密"),
                            _ => String::new(),
                        };
                        if !tip.is_empty() {
                            resp.on_hover_text(tip);
                        }
                    }
                }
            });
        });
}

fn show_period_care_banner(
    ui: &mut egui::Ui,
    care: &PeriodCareRemind,
    nav: &mut NavItem,
    selected: &mut Option<String>,
    editing: &mut bool,
    nav_locked: bool,
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
                    if !nav_locked
                        && theme::primary_button(ui, "去看看").clicked()
                    {
                        *nav = NavItem::GenderPrivate;
                        select_gender_hub(selected, editing);
                    }
                });
            });
        })
        .response;
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if nav_locked {
            egui::CursorIcon::NotAllowed
        } else {
            egui::CursorIcon::PointingHand
        });
    }
    if nav_locked {
        resp.on_hover_text("请先关闭新建或编辑");
    } else if resp.clicked() {
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
    nav_locked: bool,
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
                    if !nav_locked
                        && theme::primary_button(ui, "去看看").clicked()
                    {
                        *nav = NavItem::GenderPrivate;
                        select_gender_hub(selected, editing);
                    }
                });
            });
        })
        .response;
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if nav_locked {
            egui::CursorIcon::NotAllowed
        } else {
            egui::CursorIcon::PointingHand
        });
    }
    if nav_locked {
        resp.on_hover_text("请先关闭新建或编辑");
    } else if resp.clicked() {
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
    let attach_id = form.attach_id.clone();
    let svc = svc.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        match svc.add_full_with_id(
            &attach_id,
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
        &m.id,
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
    cred_filter: &mut CredFilter,
    cred_sort: &mut CredSort,
    cred_tag: &mut Option<String>,
    open_new_memo: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    let avail = ui.available_width();
    // 必须在 horizontal 之前取高度：horizontal 内 available_height 不可靠
    let full_h = ui.available_height().max(120.0);
    let gap = page_shell::PAGE_GAP;
    // 任务 / 私密 / 备忘：左列表固定 · 右内容伸展
    let (list_w, detail_w) = if nav.is_gender_private() || nav.is_credentials() {
        page_shell::page_rail_split(avail)
    } else {
        // 勿 .max(120/160) 硬底：窄窗时 list+detail 会超过 avail → allocate 退进程
        if !avail.is_finite() || avail <= gap + 2.0 {
            (1.0, 1.0)
        } else {
            let usable = avail - gap;
            let mut list_w = (usable * 0.38).min(400.0).max(1.0);
            if usable >= 400.0 {
                list_w = list_w.max(160.0).min(400.0);
            }
            let detail_w = (usable - list_w).max(1.0);
            (list_w, detail_w)
        }
    };

    let list_n = filtered.iter().filter(|m| !is_private_hub(&m.id)).count();
    let credentials_page = nav.is_credentials();
    let private_page = nav.is_gender_private();
    // 备忘 / 私密：页头进左栏，右栏顶对齐上移

    let body_h = if credentials_page || private_page {
        (ui.available_height()).max(80.0)
    } else {
        full_h
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(
            Vec2::new(list_w, body_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(body_h);
                ui.set_max_height(body_h);
                ui.set_max_width(list_w);
                ui.set_clip_rect(ui.max_rect());
                if credentials_page {
                    page_shell::show_page_header(
                        ui,
                        "备忘",
                        &format!("{list_n} 条 · 永久保存"),
                        open_new_memo,
                    );
                    ui.add_space(page_shell::PAGE_GAP);
                } else if private_page {
                    page_shell::show_page_header(
                        ui,
                        "私密",
                        &format!("{list_n} 条"),
                        open_new_memo,
                    );
                    ui.add_space(page_shell::PAGE_GAP);
                } else if !credentials_page {
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
                }

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
                            let border = if credentials_page && is_sel {
                                theme::border()
                            } else if is_sel {
                                theme::shell_accent()
                            } else if is_hub {
                                theme::period_day()
                            } else if let Some(c) = stripe {
                                c
                            } else {
                                theme::border()
                            };
                            // 备忘选中：浅蓝底；其它：白底 / hub 浅蓝
                            let card_fill = if credentials_page && is_sel {
                                theme::shell_accent_soft()
                            } else if is_hub {
                                theme::shell_women_pill().linear_multiply(0.45)
                            } else {
                                theme::card()
                            };
                            let mut toggled_done = false;
                            let resp = Frame::none()
                                .fill(card_fill)
                                .stroke(Stroke::new(
                                    if is_sel && !credentials_page { 1.8 } else { 1.0 },
                                    border,
                                ))
                                .rounding(Rounding::same(if credentials_page { 8.0 } else { 12.0 }))
                                .inner_margin(Margin::symmetric(
                                    if credentials_page { 10.0 } else { 12.0 },
                                    if credentials_page { 7.0 } else { 10.0 },
                                ))
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    if credentials_page && !is_hub {
                                        show_cred_list_row_body(ui, m);
                                    } else {
                                    ui.horizontal(|ui| {
                                        if is_hub {
                                                let (r, _) = ui.allocate_exact_size(
                                                    Vec2::splat(18.0),
                                                    Sense::hover(),
                                                );
                                            ui.painter().circle_filled(
                                                r.center(),
                                                    4.0,
                                                    theme::period_day(),
                                            );
                                        } else {
                                                let cb =
                                                    list_category_icon(ui, m.category, m.done);
                                            if cb.clicked() {
                                                toggled_done = true;
                                                let next = !m.done;
                                                if next
                                                        && selected.as_deref()
                                                            == Some(m.id.as_str())
                                                {
                                                    *selected = None;
                                                    *editing = false;
                                                }
                                                let svc = svc.clone();
                                                let id = m.id.clone();
                                                let tx = tx.clone();
                                                std::thread::spawn(move || {
                                                    match svc.set_done(&id, next) {
                                                        Ok(()) => {
                                                            let _ = tx.send(BgMsg::Refresh);
                                                        }
                                                        Err(e) => {
                                                                let _ = tx.send(BgMsg::Error(
                                                                    e.to_string(),
                                                                ));
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
                                            let mut title = RichText::new(if m.title.is_empty()
                                            {
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
                                                            .fill(theme::card())
                                                            .rounding(Rounding::same(4.0))
                                                            .inner_margin(Margin::symmetric(
                                                                6.0, 1.0,
                                                            ))
                                                        .show(ui, |ui| {
                                                            ui.label(
                                                                    RichText::new("日历")
                                                                        .size(10.5)
                                                                    .strong()
                                                                        .color(
                                                                            theme::shell_women_pill_fg(),
                                                                        ),
                                                            );
                                                        });
                                                } else {
                                                    if !private_warn {
                                                        sync_mark(ui, m.title.is_empty());
                                                    }
                                                        let show_si = show_backup_badge
                                                            && m.visibility
                                                                == MemoVisibility::Private;
                                                        if show_si {
                                                        private_badge_pill(ui);
                                                    }
                                                        if !m.due_date.is_empty() {
                                                        let (due_txt, due_fg) = if is_od {
                                                            (
                                                                format!(
                                                                    "到期 {}",
                                                                        format_due_mmdd(
                                                                            &m.due_date
                                                                        )
                                                                ),
                                                                theme::warn_overdue_fg(),
                                                            )
                                                        } else if is_dt {
                                                            (
                                                                format!(
                                                                    "到期 {}",
                                                                        format_due_mmdd(
                                                                            &m.due_date
                                                                        )
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
                                                                        format_due_mmdd(
                                                                            &m.due_date
                                                                        )
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
                                                                    RichText::new(
                                                                        m.priority.label(),
                                                                    )
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
                                            }
                                    }
                                })
                                .response
                                .interact(Sense::click());
                            if credentials_page && is_sel {
                                let r = resp.rect;
                                let bar = egui::Rect::from_min_max(
                                    egui::pos2(r.left(), r.top() + 4.0),
                                    egui::pos2(r.left() + 4.0, r.bottom() - 4.0),
                                );
                                ui.painter().rect_filled(
                                    bar,
                                    Rounding::same(2.0),
                                    theme::shell_accent(),
                                );
                            } else if let Some(c) = stripe {
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
                                    // 关掉备忘抽屉，右栏日历本就在
                                    select_gender_hub(selected, editing);
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
                                    theme::style_context_menu(ui);
                                    if theme::menu_item_mark(ui, "开", "打开").clicked() {
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
                                    if theme::menu_item_mark(ui, "签", "生成桌面便签").clicked()
                                    {
                                        *open_sticky = Some(m.id.clone());
                                        ui.close_menu();
                                    }
                                    theme::menu_sep(ui);
                                    if theme::menu_item_danger(ui, "移入回收站…").clicked() {
                                        *selected = Some(m.id.clone());
                                        *title_draft = m.title.clone();
                                        *show_delete = true;
                                        ui.close_menu();
                                    }
                                });
                            }
                            ui.add_space(if credentials_page { 4.0 } else { 6.0 });
                        }
                    });
            },
        );

        ui.allocate_ui_with_layout(
            Vec2::new(detail_w, body_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(body_h);
                ui.set_max_height(body_h);
                ui.set_max_width(detail_w);
                ui.set_clip_rect(ui.max_rect());
                if private_page {
                    // 右栏永久关怀日历；备忘查看走叠层抽屉
                    gender_private_view::show(ui, svc, gender_ui, all_memos, status_line);
                } else if credentials_page {
                    show_cred_filter_rail(ui, all_memos, cred_filter, cred_sort, cred_tag);
                } else {
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
                        open_sticky,
                    tx,
                );
                }
            },
        );
    });
}

fn private_badge_pill(ui: &mut egui::Ui) {
    Frame::none()
        .fill(theme::shell_accent_soft())
        .rounding(Rounding::same(6.0))
        .inner_margin(Margin::symmetric(6.0, 1.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new("私")
                    .size(11.0)
                    .strong()
                    .color(theme::shell_accent()),
            );
        });
}

/// 备忘新建/编辑右侧抽屉（遮罩 + 面板，滑入/滑出），与列表浏览面分离。
/// 仅标题栏「×」或底栏「取消」可关；禁止点遮罩 / Esc（避免误关，也避免收起动画里表单 allocate 退进程）。
pub fn show_memo_form_drawer(
    ctx: &egui::Context,
    host: egui::Rect,
    title: &str,
    edit_form: &mut MemoFormState,
    mode: FormMode,
    id_salt: &str,
    data_dir: &str,
    drawer_w: &mut f32,
    drawer_dirty: &mut bool,
) -> MemoDrawerAction {
    let mut action = MemoDrawerAction::None;
    if host.width() < 40.0 || host.height() < 40.0 {
        return action;
    }
    if edit_form.data_dir.is_none() {
        edit_form.data_dir = Some(std::path::PathBuf::from(data_dir));
    }
    if edit_form.attach_id.trim().is_empty() {
        edit_form.attach_id = uuid::Uuid::new_v4().to_string().replace('-', "");
    }
    let anim = page_shell::drawer_anim_begin(ctx, id_salt);
    let mut request_close = false;
    page_shell::scale_drawer_w_with_host(ctx, id_salt, drawer_w, host.width());
    let (panel, dim, alpha) =
        page_shell::drawer_layout(host, *drawer_w, anim.t.max(0.001));

    if dim.width() > 1.0 && dim.height() > 1.0 {
        egui::Area::new(Id::new(("memo_form_drawer_bg", id_salt)))
            .order(Order::Middle)
            .fixed_pos(dim.min)
            .interactable(true)
            .show(ctx, |ui| {
                // 遮罩只挡点击，不关闭
                let (rect, _) = ui.allocate_exact_size(dim.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, Rounding::ZERO, Color32::from_black_alpha(alpha));
            });
    }

    // 紧凑抽屉：pad 12×8、头 32、底 44（对齐 memo-form-compact-focus）
    const PAD_X: f32 = 12.0;
    const PAD_Y: f32 = 8.0;
    let inner_h = (panel.height() - PAD_Y * 2.0 - 2.0).max(80.0);
    let header_h = 32.0;
    let footer_h = 44.0;
    let scroll_h = (inner_h - header_h - footer_h).max(80.0);
    // 收起动画全程跳过表单：滚动后 TextEdit/ScrollArea 在极窄宽下仍可能 allocate 退进程
    let skip_form = anim.closing
        || page_shell::drawer_panel_too_narrow(panel.width(), PAD_X);

    egui::Area::new(Id::new(("memo_form_drawer_panel", id_salt)))
        .order(Order::Foreground)
        .fixed_pos(panel.min)
        .interactable(true)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_max_size(panel.size());
            ui.set_clip_rect(panel);
            if skip_form {
                ui.painter()
                    .rect_filled(panel, Rounding::ZERO, theme::panel());
                return;
            }
            Frame::none()
                .fill(theme::panel())
                .stroke(Stroke::new(1.0, theme::border()))
                .inner_margin(Margin::symmetric(PAD_X, PAD_Y))
                .show(ui, |ui| {
                    // Frame 已含 PAD；勿再按外宽 drawer_w 设最小宽，否则右溢
                    let content_w = ui.available_width().max(1.0);
                    ui.set_min_height(inner_h);
                    ui.set_max_height(inner_h);
                    ui.set_max_width(content_w);
                    ui.allocate_ui_with_layout(
                        Vec2::new(content_w, header_h),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                RichText::new(title)
                                    .size(14.0)
                                    .strong()
                                    .color(theme::text()),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("×")
                                                .size(16.0)
                                                .color(theme::text_muted()),
                                        )
                                        .fill(Color32::TRANSPARENT)
                                        .frame(false)
                                        .min_size(Vec2::new(24.0, 24.0)),
                                    )
                                    .clicked()
                                    && !anim.closing
                                {
                                    request_close = true;
                                }
                            });
                        },
                    );

                    ctx.data_mut(|d| {
                        d.insert_temp(Id::new("form_viewport_h"), scroll_h);
                    });
                    // 关拖拽滚动，避免抢走备注 TextEdit 的拖选
                    let mut scroll = egui::ScrollArea::vertical()
                        .id_source(("memo_form_drawer_scroll", id_salt))
                        .auto_shrink([false, false])
                        .drag_to_scroll(false)
                        .max_height(scroll_h);
                    if edit_form.force_scroll_top() {
                        scroll = scroll.vertical_scroll_offset(0.0);
                        edit_form.tick_force_scroll_top();
                    }
                    scroll.show(ui, |ui| {
                        // 只限最大宽，勿 set_min_width 强迫内容撑破滚动区
                        let w = ui.available_width();
                        ui.set_max_width(w);
                        memo_form::show_fields(ui, edit_form, mode, id_salt);
                    });

                    ui.allocate_ui_with_layout(
                        Vec2::new(content_w, footer_h),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let save_label = match mode {
                                FormMode::Create => "保存备忘",
                                FormMode::Edit => "保存",
                            };
                            if theme::page_primary_button(ui, save_label).clicked() {
                                action = MemoDrawerAction::Save;
                            }
                            if theme::page_outline_button(ui, "取消").clicked() && !anim.closing {
                                request_close = true;
                            }
                        },
                    );
                });
        });

    if page_shell::drawer_resize_edge(
        ctx,
        id_salt,
        panel,
        drawer_w,
        host.width(),
        anim.t,
    ) {
        *drawer_dirty = true;
    }

    if action == MemoDrawerAction::Save {
        // 保存立即返回；清挂载态，下次打开从 0 再滑入
        ctx.data_mut(|d| {
            d.insert_temp(Id::new(("drawer_mounted", id_salt)), false);
        });
        return action;
    }
    if page_shell::drawer_anim_end(ctx, &anim, request_close) {
        MemoDrawerAction::Dismiss
    } else {
        MemoDrawerAction::None
    }
}

/// 备忘右栏：类型 / 标签 / 其它过滤 + 排序（文本搜索用顶栏）。
fn show_cred_filter_rail(
    ui: &mut egui::Ui,
    all_memos: &[MemoView],
    filter: &mut CredFilter,
    sort: &mut CredSort,
    tag_filter: &mut Option<String>,
) {
    ui.set_clip_rect(ui.max_rect());
    let rail_w = page_shell::fit_avail_width(ui, ui.max_rect().width() - 2.0);
    ui.set_max_width(rail_w);

    // 右栏整卡：白底 + 细边，与左列表区分
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_min_height((ui.available_height() - 4.0).max(80.0));
            ui.set_max_width(page_shell::fit_avail_width(ui, ui.available_width()));

            egui::ScrollArea::vertical()
                .id_source("cred_filter_rail")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(page_shell::fit_avail_width(ui, ui.available_width()));

                    cred_rail_section_label(ui, "类型");
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.spacing_mut().item_spacing.y = 6.0;
                        for (f, label) in [
                            (CredFilter::All, "全部"),
                            (CredFilter::TypeCredentials, "账号证件"),
                            (CredFilter::TypeWork, "工作学习"),
                            (CredFilter::TypeLife, "生活家庭"),
                            (CredFilter::TypeFinance, "财务订阅"),
                            (CredFilter::TypeInspiration, "灵感"),
                            (CredFilter::TypeEmergency, "应急"),
                            (CredFilter::TypeGeneral, "随手记"),
                        ] {
                            cred_filter_chip(ui, filter, f, label);
                        }
                    });
                    ui.add_space(14.0);

                    cred_rail_section_label(ui, "标签");
                    ui.add_space(6.0);
                    let mut tags: Vec<String> = all_memos
                        .iter()
                        .filter(|m| {
                            !is_private_hub(&m.id)
                                && !is_new_draft(&m.id)
                                && is_memo_archive_cat(m.category)
                        })
                        .flat_map(|m| m.tags.iter().cloned())
                        .filter(|t| !t.trim().is_empty())
                        .collect();
                    tags.sort();
                    tags.dedup();
                    if tags.is_empty() {
                        ui.label(
                            RichText::new("库内暂无标签")
                                .size(12.0)
                                .color(theme::text_muted()),
                        );
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.spacing_mut().item_spacing.y = 6.0;
                            for t in &tags {
                                let on = tag_filter.as_deref() == Some(t.as_str());
                                if cred_tag_chip(ui, t, on).clicked() {
                                    if on {
                                        *tag_filter = None;
                                    } else {
                                        *tag_filter = Some(t.clone());
                                    }
                                }
                            }
                        });
                    }
                    ui.add_space(14.0);

                    cred_rail_section_label(ui, "其它");
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.spacing_mut().item_spacing.y = 6.0;
                        for (f, label) in [
                            (CredFilter::Pinned, "置顶"),
                            (CredFilter::Tagged, "有标签"),
                            (CredFilter::PrivateOnly, "仅私密"),
                        ] {
                            cred_filter_chip(ui, filter, f, label);
                        }
                    });
                    ui.add_space(16.0);

                    // 底部分隔 + 排序
                    let sep = ui.max_rect();
                    ui.painter().hline(
                        sep.x_range(),
                        ui.cursor().top(),
                        Stroke::new(1.0, theme::border()),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("排序")
                                .size(12.0)
                                .strong()
                                .color(theme::text_muted()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            egui::ComboBox::from_id_source("cred_list_sort")
                                .selected_text(match *sort {
                                    CredSort::Updated => "最近更新",
                                    CredSort::Created => "创建时间",
                                    CredSort::Title => "标题",
                                })
                                .width(100.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(sort, CredSort::Updated, "最近更新");
                                    ui.selectable_value(sort, CredSort::Created, "创建时间");
                                    ui.selectable_value(sort, CredSort::Title, "标题");
                                });
                        });
                    });
                });
        });
}

fn cred_rail_section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(11.0)
            .strong()
            .color(theme::text_muted()),
    );
}

fn cred_filter_chip(ui: &mut egui::Ui, filter: &mut CredFilter, f: CredFilter, label: &str) {
    let on = *filter == f;
    if cred_rail_chip(ui, label, on).clicked() {
        *filter = f;
    }
}

fn cred_tag_chip(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    cred_rail_chip(ui, label, on)
}

fn cred_rail_chip(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    let (fg, fill, stroke) = if on {
        (
            theme::shell_accent(),
            theme::shell_accent_soft(),
            Stroke::new(1.0, Color32::from_rgb(184, 206, 248)),
        )
    } else {
        (
            theme::text_muted(),
            Color32::from_rgb(247, 249, 252),
            Stroke::new(1.0, theme::border()),
        )
    };
    ui.add(
        egui::Button::new(RichText::new(label).size(12.0).color(fg))
            .rounding(Rounding::same(99.0))
            .stroke(stroke)
            .fill(fill)
            .min_size(Vec2::new(0.0, 26.0)),
    )
}

fn archive_type_label(c: MemoCategory) -> &'static str {
    match c.canonical() {
        MemoCategory::Credentials => "账号证件",
        MemoCategory::Work | MemoCategory::Office => "工作学习",
        MemoCategory::Life => "生活家庭",
        MemoCategory::Finance => "财务订阅",
        MemoCategory::Inspiration => "灵感",
        MemoCategory::Emergency => "应急",
        MemoCategory::General => "随手记",
        other => other.label(),
    }
}

/// 备忘列表行：仅两行（对齐任务卡密度）——标题行 + 摘要/标签行。
fn show_cred_list_row_body(ui: &mut egui::Ui, m: &MemoView) {
    // 行 1：置顶点 · 标题 …… 类型 · 时间
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if m.priority == MemoPriority::High {
            ui.label(
                RichText::new("●")
                    .size(8.0)
                    .color(theme::shell_accent()),
            );
        }
        let mut title = RichText::new(if m.title.is_empty() {
            "(无标题)"
        } else {
            m.title.as_str()
        })
        .strong()
        .size(13.5)
        .color(if m.done {
            theme::text_muted()
        } else {
            theme::text()
        });
        if m.done {
            title = title.strikethrough();
        }
        ui.label(title);
        if m.visibility == MemoVisibility::Private {
            private_badge_pill(ui);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                RichText::new(format_relative_modified(&m.modified_at))
                    .size(11.0)
                    .color(theme::text_muted()),
            );
            Frame::none()
                .fill(theme::shell_accent_soft())
                .rounding(Rounding::same(4.0))
                .inner_margin(Margin::symmetric(5.0, 1.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(archive_type_label(m.category))
                            .size(10.0)
                            .strong()
                            .color(theme::shell_accent()),
                    );
                });
        });
    });
    // 行 2：正文摘录；无摘录则弱展示标签
    let sub = memo_excerpt(&m.content, 40).or_else(|| {
        if m.tags.is_empty() {
            None
        } else {
            Some(
                m.tags
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · "),
            )
        }
    });
    if let Some(line) = sub {
        ui.add_space(1.0);
        ui.label(RichText::new(line).size(12.0).color(theme::text_muted()));
    }
}

#[allow(dead_code)]
fn priority_mini_pill(ui: &mut egui::Ui, p: MemoPriority) {
    Frame::none()
        .fill(theme::priority_pill_bg(p))
        .rounding(Rounding::same(4.0))
        .inner_margin(Margin::symmetric(5.0, 1.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(p.label())
                    .size(10.5)
                    .strong()
                    .color(theme::priority_pill_fg(p)),
            );
        });
}

/// 备忘页跨栏顶栏（已改用左栏 page_shell 头；保留供对照）。
#[allow(dead_code, clippy::too_many_arguments)]
fn show_credentials_page_header(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    filtered: &[MemoView],
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
    _memo_doc: &mut doc_editor::Doc,
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
    open_new_memo: &mut bool,
    open_sticky: &mut Option<String>,
    list_n: usize,
    _tx: &Sender<BgMsg>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            RichText::new("备忘")
                .size(18.0)
                .strong()
                .color(theme::text()),
        );
        ui.label(
            RichText::new(format!("{list_n} 条 · 永久保存"))
                .size(13.0)
                .color(theme::text_muted()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let has_sel = selected
                .as_ref()
                .is_some_and(|id| !is_private_hub(id) && !is_new_draft(id));

            // RTL：更多 → 编辑/保存 → 新建
            let more_btn = ui.add_enabled(
                has_sel,
                egui::Button::new(RichText::new("更多 ▾").size(13.0).color(theme::text_muted()))
                    .fill(Color32::TRANSPARENT)
                    .frame(false),
            );
            let more_id = ui.make_persistent_id("cred_page_more");
            if more_btn.clicked() {
                ui.memory_mut(|m| m.toggle_popup(more_id));
            }
            egui::popup_below_widget(ui, more_id, &more_btn, |ui| {
                ui.set_min_width(148.0);
                if ui.button("复制全文").clicked() {
                    ui.output_mut(|o| {
                        o.copied_text = format!("{}\n\n{}", title_draft, body_draft)
                    });
                    *status_line = "已复制到剪贴板".into();
                    ui.memory_mut(|m| m.close_popup());
                }
                if ui.button("导出").clicked() {
                    if let Some(id) = selected.clone() {
                        let dir = std::path::PathBuf::from(data_dir).join("exports");
                        let path = dir.join(format!(
                            "memo-{}-{}.txt",
                            &id[..8.min(id.len())],
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                        ));
                        *export_ids = Some(vec![id]);
                        *export_path = path.display().to_string();
                        export_pw.clear();
                        *show_export = true;
                    }
                    ui.memory_mut(|m| m.close_popup());
                }
                if ui.button("历史").clicked() {
                    if let Some(id) = selected.clone() {
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
                    ui.memory_mut(|m| m.close_popup());
                }
                if ui.button("生成桌面便签").clicked() {
                    if let Some(id) = selected.clone() {
                        *open_sticky = Some(id);
                    }
                    ui.memory_mut(|m| m.close_popup());
                }
                if ui
                    .add(egui::Button::new(
                        RichText::new("移入回收站").color(theme::danger()),
                    ))
                    .clicked()
                {
                    *show_delete = true;
                    ui.memory_mut(|m| m.close_popup());
                }
            });

            // 编辑在右侧抽屉完成；页头只保留入口
            ui.add_enabled_ui(has_sel && !*editing, |ui| {
                    if theme::page_outline_button(ui, "编辑").clicked() {
                        begin_memo_edit(
                            filtered,
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
                            edit_form,
                            status_line,
                        );
                    }
                });
            if theme::page_primary_button(ui, "+ 新建").clicked() {
                *open_new_memo = true;
            }
        });
    });
}

pub(crate) fn begin_memo_edit(
    filtered: &[MemoView],
    selected: &Option<String>,
    title_draft: &str,
    body_draft: &str,
    visibility_draft: &MemoVisibility,
    category_draft: &MemoCategory,
    due_date_draft: &str,
    end_date_draft: &str,
    tags_draft: &str,
    done_draft: &bool,
    priority_draft: &MemoPriority,
    editing: &mut bool,
    edit_form: &mut MemoFormState,
    status_line: &mut String,
) {
    let Some(id) = selected.as_ref() else {
        return;
    };
    let meta = filtered.iter().find(|m| m.id == *id);
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
        id,
    );
    *status_line = "在右侧抽屉中编辑，保存后更新本机".into();
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn save_memo_edit(
    svc: &Arc<MemoService>,
    filtered: &[MemoView],
    selected: &Option<String>,
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
    tx: &Sender<BgMsg>,
) {
    let Some(id) = selected.clone() else {
        return;
    };
    let meta = filtered.iter().find(|m| m.id == id);
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn cancel_memo_edit(
    selected: &Option<String>,
    filtered: &[MemoView],
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
    *editing = false;
    let Some(id) = selected.as_ref() else {
        return;
    };
    let Some(m) = filtered.iter().find(|m| m.id == *id) else {
        return;
    };
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
        &m.id,
    );
}

/// 私密备忘只读详情（叠在关怀日历上的查看抽屉内）。
#[allow(clippy::too_many_arguments)]
fn show_private_memo_detail(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    filtered: &[MemoView],
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
) {
    let Some(id) = selected.clone() else {
        return;
    };
    if is_private_hub(&id) || is_new_draft(&id) {
        return;
    }
    let meta = filtered.iter().find(|m| m.id == id);

        ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(!*editing, |ui| {
                if theme::primary_button(ui, "编辑").clicked() {
                    begin_memo_edit(
                        filtered,
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
                        edit_form,
                        status_line,
                    );
                }
        });
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

                let title_show = if title_draft.is_empty() {
                    "(无标题)"
                } else {
                    title_draft.as_str()
                };
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
    let days_left = memo_core::days_until_due(due_date_draft, end_date_draft, *done_draft);
                if od || dt || soon {
                    ui.add_space(8.0);
                    let (bg, fg, border, line) = if od {
            let n = memo_core::overdue_days(due_date_draft, end_date_draft, *done_draft)
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
                        .size(12.5)
                        .strong()
                        .color(theme::text_muted()),
                );
                ui.add_space(6.0);

                let note_resp = Frame::none()
        .fill(theme::panel())
                    .rounding(Rounding::same(12.0))
                    .inner_margin(Margin::symmetric(16.0, 14.0))
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
                            let mid = selected.as_deref().unwrap_or("");
                            doc_editor::show_viewer_body_ex(
                                ui,
                                body_draft,
                                Some(std::path::Path::new(svc.config().data_dir.as_str())),
                                mid,
                            );
                        }
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                let (bar, _) = ui.allocate_exact_size(Vec2::new(3.0, 36.0), Sense::hover());
                ui.painter()
                    .rect_filled(bar, Rounding::same(1.5), theme::border());
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(
                        "经期日与体检日请在右栏关怀日历标记；系统提醒仅供参考，不能替代就医。",
                                    )
                                    .size(12.0)
                                    .color(theme::text_muted()),
                                );
                            });
                    });
                    theme::paint_dashed_rect(
                        ui.painter(),
                        note_resp.response.rect,
                        Stroke::new(1.2, theme::border()),
                    );
                }

/// 非备忘页右栏详情（备忘已改右栏过滤轨 + 抽屉查看）。
#[allow(clippy::too_many_arguments)]
fn show_detail(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    filtered: &[MemoView],
    _all_memos: &[MemoView],
    _gender_ui: &mut GenderPrivateUi,
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
    _memo_doc: &mut doc_editor::Doc,
    edit_form: &mut MemoFormState,
    status_line: &mut String,
    show_delete: &mut bool,
    _show_export: &mut bool,
    _export_pw: &mut String,
    _export_path: &mut String,
    _export_ids: &mut Option<Vec<String>>,
    _show_history: &mut bool,
    _history_entity: &mut String,
    _history_events: &mut Vec<memo_core::service::HistoryEvent>,
    _history_sel_a: &mut Option<usize>,
    _history_sel_b: &mut Option<usize>,
    _data_dir: &str,
    open_sticky: &mut Option<String>,
    _tx: &Sender<BgMsg>,
) {
    ui.set_clip_rect(ui.max_rect());
    ui.set_max_width(page_shell::fit_avail_width(ui, ui.max_rect().width() - 6.0));
    if selected.as_ref().is_some_and(|id| is_new_draft(id)) {
        *selected = None;
        *editing = false;
    }

    if selected.is_none() {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.label(
                theme::muted_label("点左侧列表查看详情\n或点「+ 新建」添加").size(14.0),
            );
        });
        return;
    }
    let id = selected.clone().unwrap();
    let meta = filtered.iter().find(|m| m.id == id);

    egui::ScrollArea::vertical()
        .id_source("shell_memo_body")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_max_width(page_shell::fit_avail_width(ui, ui.max_rect().width() - 6.0));
            show_cred_detail_readonly(
                ui,
                svc,
                meta,
                filtered,
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
                edit_form,
                status_line,
                show_delete,
                open_sticky,
            );
        });
}

#[allow(clippy::too_many_arguments)]
fn show_cred_detail_readonly(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    meta: Option<&MemoView>,
    filtered: &[MemoView],
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
    edit_form: &mut MemoFormState,
    status_line: &mut String,
    show_delete: &mut bool,
    open_sticky: &mut Option<String>,
) {
    let title_show = if title_draft.is_empty() {
        "(无标题)"
    } else {
        title_draft.as_str()
    };
    let updated = meta
        .map(|m| format_relative_modified(&m.modified_at))
        .unwrap_or_else(|| "—".into());
    let is_private = meta
        .map(|m| m.visibility == MemoVisibility::Private)
        .unwrap_or(true);
    let pri = *priority_draft;
    let ver = meta.map(|m| m.version).unwrap_or(0);
    let type_name = archive_type_label(*category_draft);

    // 阅读器顶栏：类型 · 永久保留 + 操作
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        Frame::none()
            .fill(theme::success_soft())
            .rounding(Rounding::same(99.0))
            .inner_margin(Margin::symmetric(10.0, 4.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("永久保留 · 不上日历")
                        .size(12.0)
                        .strong()
                        .color(theme::success()),
                );
            });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if theme::page_danger_outline_button(ui, "删除").clicked() {
                *show_delete = true;
            }
            if theme::page_outline_button(ui, "便签").clicked() {
                if let Some(id) = selected.clone() {
                    *open_sticky = Some(id);
                }
            }
            if theme::page_outline_button(ui, "复制").clicked() {
                ui.output_mut(|o| {
                    o.copied_text = format!("{}\n\n{}", title_draft, body_draft)
                });
                *status_line = "已复制到剪贴板".into();
            }
            ui.add_enabled_ui(!*editing, |ui| {
                if theme::page_primary_button(ui, "编辑").clicked() {
                    begin_memo_edit(
                        filtered,
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
                        edit_form,
                        status_line,
                    );
                }
            });
        });
    });
    ui.add_space(14.0);

    ui.label(
        RichText::new(format!("{type_name} · 更新于 {updated}"))
            .size(12.0)
            .color(theme::text_muted()),
    );
    ui.add_space(6.0);
    let mut heading = RichText::new(title_show)
        .size(22.0)
        .strong()
        .color(theme::text());
    if *done_draft {
        heading = heading.strikethrough().color(theme::text_muted());
    }
    ui.label(heading);
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        Frame::none()
            .fill(theme::shell_accent_soft())
            .rounding(Rounding::same(6.0))
            .inner_margin(Margin::symmetric(7.0, 2.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(type_name)
                        .size(11.5)
                        .strong()
                        .color(theme::shell_accent()),
                );
            });
        if is_private {
            private_badge_pill(ui);
        }
        for t in parse_tags(tags_draft) {
            tag_pill(ui, &t);
        }
    });

    ui.add_space(14.0);
    ui.separator();
    ui.add_space(12.0);
    if body_draft.trim().is_empty() {
        ui.label(
            RichText::new("（尚无正文）")
                .size(14.0)
                .italics()
                .color(theme::text_muted()),
        );
    } else {
        let mid = selected.as_deref().unwrap_or("");
        doc_editor::show_viewer_body_ex(
            ui,
            body_draft,
            Some(std::path::Path::new(svc.config().data_dir.as_str())),
            mid,
        );
    }

    ui.add_space(20.0);
    ui.separator();
    ui.add_space(10.0);
    let cell_w = page_shell::split_col_width(ui.available_width(), 3, 8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (k, v) in [
            ("类型", type_name.to_string()),
            (
                "可见性",
                if is_private {
                    "私密 · 本地加密".into()
                } else {
                    "节点可见".into()
                },
            ),
            ("版本", format!("v{ver} · {}", pri.label())),
        ] {
            Frame::none()
                .fill(Color32::from_rgb(250, 251, 252))
                .stroke(Stroke::new(1.0, theme::border()))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    let cw = page_shell::fit_avail_width(ui, (cell_w - 8.0).max(1.0));
                    ui.set_max_width(cw);
                    ui.set_min_width(cw.min(ui.available_width().max(1.0)));
                    ui.label(RichText::new(k).size(11.0).color(theme::text_muted()));
                    ui.label(
                        RichText::new(v)
                            .size(13.0)
                            .strong()
                            .color(theme::text()),
                    );
                });
        }
    });
}
