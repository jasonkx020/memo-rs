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
use crate::settings_view::{self, SettingsTab};
use crate::theme;
use chrono::{Datelike, Timelike};
use eframe::egui::{
    self, Color32, Frame, Id, Margin, Order, RichText, Rounding, Sense, Stroke, Vec2,
};
use memo_core::config::{Config, NodeRole};
use memo_core::disk::DiskSpace;
use memo_core::identity_keys::IdentityKeys;
use memo_core::service::{MemoService, MemoView};
use memo_core::store::{
    LanShare, MemoCategory, MemoLifecycle, MemoPriority, MemoSpace, MemoVisibility,
};
use memo_sync::{DiscoveredPeer, PeerStatus};
use std::sync::mpsc::Sender;
use std::sync::Arc;

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
    private_branch: PrivateBranch,
) {
    use crate::memo_form::CreateEntryKind;
    let entry = match nav {
        NavItem::Credentials => CreateEntryKind::Memo,
        NavItem::GenderPrivate => match private_branch {
            PrivateBranch::Credentials => CreateEntryKind::PrivateMemo,
            PrivateBranch::Todo | PrivateBranch::Care => CreateEntryKind::PrivateTask,
        },
        _ => CreateEntryKind::Task,
    };
    let stamp = today_ymd();
    let prefill = if matches!(
        entry,
        CreateEntryKind::Task | CreateEntryKind::PrivateTask
    ) && nav.is_task_space()
    {
        calendar_ymd.map(|d| format!("{d} 09:00"))
    } else {
        None
    };
    *edit_form = MemoFormState::begin_create(entry, &stamp, prefill.as_deref());
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
        content_loaded: true,
        deleted: false,
        version: 0,
        node_id: String::new(),
        visibility: MemoVisibility::Private,
        space: MemoSpace::Private,
        lan_share: LanShare::Off,
        owner_fp: String::new(),
        modified_at: String::new(),
        lifecycle: MemoLifecycle::Permanent,
        deleted_at: String::new(),
        category: MemoCategory::Todo,
        due_date: String::new(),
        end_date: String::new(),
        remind_before_days: 0,
        remind_seen_for: String::new(),
        done: false,
        tags: vec!["日历".into()],
        priority: MemoPriority::Normal,
    })
}

/// 日历产生的系统备忘不出现在私密列表（仍保留在关怀页内使用）。
pub(crate) fn is_private_calendar_memo(m: &MemoView) -> bool {
    if m.space != MemoSpace::Private && !m.category.is_gender_private() {
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

/// 私密空间分段（对齐 HTML：关怀 | 私密任务 | 私密备忘）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrivateBranch {
    #[default]
    Care,
    Todo,
    Credentials,
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
    /// 私密空间：任务 / 备忘子支
    pub private_branch: PrivateBranch,
    /// CentralPanel 内容区，供备忘新建/编辑抽屉锚定
    pub content_host: egui::Rect,
    /// 顶栏告警上次按自然日重算的日期（常开跨天检测）
    pub last_nav_warn_ymd: String,
    /// 表单 / 详情抽屉宽度（按身份持久化）
    pub drawer: page_shell::DrawerWidths,
    /// 关预览后仍高亮的列表条目（今日/日历/备忘/私密共用）
    pub list_focus: Option<String>,
    /// 解锁恢复选中后：下一帧按导航重置侧栏抽屉动画（与鼠标点开同为滑入）。
    pub pending_side_drawer_anim: bool,
    /// 设置页左栏分类
    pub settings_tab: SettingsTab,
    /// 关怀页「应用设置」入口：只弹关怀相关设置，不打开完整设置。
    pub show_care_settings: bool,
}

impl Default for ShellUi {
    fn default() -> Self {
        Self {
            nav: NavItem::Today,
            gender: GenderPrivateUi::default(),
            show_nodes: false,
            cal: CalendarUi::default(),
            cred_filter: CredFilter::All,
            cred_sort: CredSort::Updated,
            cred_tag: None,
            private_branch: PrivateBranch::Care,
            content_host: egui::Rect::NOTHING,
            last_nav_warn_ymd: String::new(),
            drawer: page_shell::DrawerWidths::default(),
            list_focus: None,
            pending_side_drawer_anim: false,
            settings_tab: SettingsTab::default(),
            show_care_settings: false,
        }
    }
}

/// 与壳层侧栏抽屉 id_salt 对齐（今日·日历 / 备忘 / 私密）。
pub fn side_drawer_salt(nav: NavItem) -> &'static str {
    if nav.is_credentials() {
        "cred_side_drawer"
    } else if nav.is_gender_private() {
        "priv_side_drawer"
    } else {
        "task_side_drawer"
    }
}

/// 备忘页：三类桶中的「备忘」。
fn is_memo_archive_cat(c: MemoCategory) -> bool {
    c.bucket() == MemoCategory::Credentials
}

/// 备忘页细筛：旧 Type* 改为按标签匹配（TypeCredentials = 桶为备忘）。
fn cred_filter_tag(filter: CredFilter) -> Option<&'static str> {
    match filter {
        CredFilter::TypeWork => Some("工作"),
        CredFilter::TypeLife => Some("生活"),
        CredFilter::TypeFinance => Some("财务"),
        CredFilter::TypeInspiration => Some("灵感"),
        CredFilter::TypeEmergency => Some("应急"),
        CredFilter::TypeGeneral => Some("随手记"),
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

/// 今日·日历 / 备忘 / 私密：预览与编辑共用同一侧栏抽屉（勿再叠第二层表单抽屉）。
pub fn nav_uses_unified_side_drawer(nav: NavItem) -> bool {
    nav.is_task_space() || nav.is_credentials() || nav.is_gender_private()
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
        let type_ok = match filter {
            CredFilter::All => true,
            CredFilter::TypeCredentials => is_memo_archive_cat(m.category),
            CredFilter::Pinned => m.priority == MemoPriority::High,
            CredFilter::Tagged => !m.tags.is_empty(),
            CredFilter::PrivateOnly => m.lan_share == LanShare::Off,
            other => {
                if let Some(need) = cred_filter_tag(other) {
                    m.tags.iter().any(|x| x == need)
                        || m.category.legacy_tag() == Some(need)
                } else {
                    true
                }
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

/// 是否在私密页（顶栏或置顶入口）。锁定时勿改 `nav`：须先快照再进身份门，否则重进会丢页签。
pub fn leave_private_for_lock(
    shell: &ShellUi,
    selected: &Option<String>,
    _editing: &bool,
) -> bool {
    shell.nav.is_gender_private() || selected.as_deref() == Some(HUB_GENDER_PRIVATE)
}

/// 顶栏「锁定」与 Esc 共用：私密页延后一帧再锁（便于本帧 flush），否则立即切身份门。
pub fn request_lock(
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

/// 在线用鲜艳绿（深色 success 在白底上看不清）；闪烁由绘制处叠加。
fn status_dot_color(st: PeerStatus) -> Color32 {
    match st {
        PeerStatus::Online => Color32::from_rgb(0x22, 0xC5, 0x5E),
        PeerStatus::Stale => theme::warn(),
        PeerStatus::Offline | PeerStatus::Undiscovered => theme::text_muted(),
    }
}

fn paint_peer_status_dot(ui: &mut egui::Ui, center: egui::Pos2, st: PeerStatus) {
    let base = status_dot_color(st);
    let (fill, glow) = if st == PeerStatus::Online {
        let t = ui.ctx().input(|i| i.time);
        let u = (t * 2.8).sin() as f32 * 0.5 + 0.5;
        ui.ctx().request_repaint();
        // 亮暗交替，并带一圈半透明光晕
        let fill = Color32::from_rgba_unmultiplied(
            0x22,
            0xC5,
            0x5E,
            (180.0 + u * 75.0) as u8,
        );
        let glow = Color32::from_rgba_unmultiplied(0x22, 0xC5, 0x5E, (40.0 + u * 70.0) as u8);
        (fill, Some((4.0 + u * 2.5, glow)))
    } else {
        (base, None)
    };
    if let Some((r, c)) = glow {
        ui.painter().circle_filled(center, r, c);
    }
    ui.painter().circle_filled(center, 4.5, fill);
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
    listen_error: Option<&str>,
    action: &mut ShellAction,
) {
    // 对齐 HTML：nodes-head
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new("发现与托管")
                    .size(22.0)
                    .strong()
                    .color(theme::text()),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("同步与节点状态集中在此，不再挤占业务顶栏。")
                    .size(13.0)
                    .color(theme::text_muted()),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("同步")
                            .size(13.5)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(theme::shell_accent())
                    .rounding(Rounding::same(9.0))
                    .min_size(Vec2::new(72.0, 36.0)),
                )
                .clicked()
            {
                action.sync = true;
            }
        });
    });
    ui.add_space(16.0);

    let vis = if node_role.is_master() {
        if node_visible {
            "主机 · 显示中"
        } else {
            "主机 · 已隐藏"
        }
    } else {
        "从机 · 不广播"
    };
    let disco = if lan_discovery {
        "局域网发现已开"
    } else {
        "局域网发现已关"
    };
    let disk = if local_disk.total_bytes > 0 {
        format!(" · 磁盘 {}", local_disk.format_pair())
    } else {
        String::new()
    };

    // local-card
    Frame::none()
        .fill(theme::shell_accent_soft())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(local_node_id)
                            .size(14.0)
                            .strong()
                            .color(theme::text()),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("{vis} · {disco}{disk}"))
                            .size(12.5)
                            .color(theme::text_muted()),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let _ = pill(
                        ui,
                        theme::shell_chip_green(),
                        theme::shell_chip_green_fg(),
                        &format!("{online_count} 在线"),
                        22.0,
                    );
                });
            });
        });

    if let Some(err) = listen_error.filter(|e| !e.is_empty()) {
        ui.add_space(8.0);
        ui.label(RichText::new(err).size(12.0).color(theme::warn()));
    }

    ui.add_space(18.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("对端")
                .size(11.0)
                .strong()
                .color(theme::mac_text_tertiary()),
        );
        ui.label(
            RichText::new("空勾选 = 推送到所有可托管节点")
                .size(11.0)
                .color(theme::mac_text_tertiary()),
        );
    });
    ui.add_space(8.0);

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
                let role = if p.role.is_master() { "主机" } else { "从机" };
                let host_tag = if p.accept_backup {
                    " · 接受托管"
                } else {
                    ""
                };
                Frame::none()
                    .fill(theme::card())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(14.0, 12.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (dot, _) =
                                ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                            paint_peer_status_dot(ui, dot.center(), p.status);
                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&name)
                                        .size(14.0)
                                        .strong()
                                        .color(theme::text()),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{role}{host_tag} · {} · {}",
                                        p.status.label(),
                                        p.node_id
                                    ))
                                    .size(12.0)
                                    .color(theme::mac_text_tertiary()),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let mut on = peer_backup_checked(backup_targets, &p.node_id);
                                    ui.add_enabled_ui(p.accept_backup, |ui| {
                                        if ui
                                            .checkbox(&mut on, "托管")
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
                                },
                            );
                        });
                    });
                ui.add_space(8.0);
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
const TOP_BAR_CTRL_H: f32 = 36.0;
/// 分段内项高（页签、图标钮）。
const TOP_BAR_ITEM_H: f32 = 28.0;
/// 分段底内边距：外高 = ITEM + 2*PAD。
const TOP_BAR_SEG_PAD: f32 = 2.0;

/// 顶栏身份入口：头像 + 名称，对齐效果稿胶囊。
fn show_identity_chip(ui: &mut egui::Ui, alias: &str) -> egui::Response {
    let letter = avatar_letter(alias);
    let av_color = theme::shell_accent();
    let hover_id = ui.id().with("identity_chip_hover");
    let hovered = ui.memory(|m| m.data.get_temp::<bool>(hover_id).unwrap_or(false));
    let fill = if hovered {
        theme::shell_accent_soft()
    } else {
        theme::panel()
    };
    let stroke_c = theme::border();
    let pad_v = ((TOP_BAR_CTRL_H - TOP_BAR_ITEM_H) * 0.5).max(0.0);
    let inner = Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke_c))
        .rounding(Rounding::same(999.0))
        .inner_margin(Margin {
            left: 5.0,
            right: 12.0,
            top: pad_v,
            bottom: pad_v,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.set_min_height(TOP_BAR_ITEM_H);
                ui.set_max_height(TOP_BAR_ITEM_H);
                let av_sz = 26.0;
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
            let undated_legacy_archive = m.category.legacy_tag().is_some()
                && m.due_date.trim().is_empty()
                && m.space == MemoSpace::Normal
                && !m.category.is_gender_private();
            let cat_ok = match nav {
                NavItem::Today | NavItem::Calendar => {
                    m.space == MemoSpace::Normal
                        && m.category.bucket() == MemoCategory::Todo
                        && !undated_legacy_archive
                }
                NavItem::Credentials => {
                    m.space == MemoSpace::Normal
                        && (is_memo_archive_cat(m.category) || undated_legacy_archive)
                }
                NavItem::Trash | NavItem::Nodes | NavItem::Settings => false,
                NavItem::GenderPrivate => m.space == MemoSpace::Private,
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
    svc: &MemoService,
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
                                svc,
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
    settings_draft: &mut Config,
    live_cfg: &mut Config,
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
    empty_trash_confirm: &mut bool,
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
    tx: &Sender<BgMsg>,
    // 有新版本时顶栏提示（版本号）；None 不显示
    update_hint_ver: Option<&str>,
    update_hint_url: Option<&str>,
    // TCP/UDP 监听异常（监督重绑中）
    listen_error: Option<&str>,
) -> ShellAction {
    if shell.pending_side_drawer_anim {
        if show_new_memo {
            page_shell::drawer_anim_reset(ctx, NEW_MEMO_DRAWER_SALT);
        } else if shell.nav.is_task_space()
            || shell.nav.is_credentials()
            || shell.nav.is_gender_private()
        {
            // 恢复预览/编辑：直接全开，勿 anim_reset 再滑入
            page_shell::drawer_anim_open_immediate(ctx, side_drawer_salt(shell.nav));
        }
        shell.pending_side_drawer_anim = false;
    }
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

    // 经期 / 体检关怀 + 任务/私密过期（顶栏用标签提示，不再脉冲红框）
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
            m.space == MemoSpace::Private
                && !is_private_hub(&m.id)
                && !is_private_calendar_memo(m)
                && memo_is_overdue(m)
        })
        .count();
    let care_needs = women_care
        .as_ref()
        .is_some_and(|r| r.needs_attention())
        || male_care.as_ref().is_some_and(|r| r.needs_attention());

    // —— 效果稿壳：暖纸底 + 左轨卡片 + 右侧主卡片（顶栏+内容）——
    const RAIL_W: f32 = 200.0;
    const SHELL_GAP: f32 = 14.0;
    const TOP_H: f32 = 58.0;

    egui::CentralPanel::default()
        .frame(
            Frame::none()
                .fill(theme::bg())
                .inner_margin(Margin::same(14.0)),
        )
        .show(ctx, |ui| {
            let full = ui.max_rect();
            ui.set_clip_rect(full);
            ui.allocate_rect(full, Sense::hover());

            let gap = SHELL_GAP;
            let rail_w = RAIL_W.min((full.width() - gap - 280.0).max(160.0));
            let main_w = (full.width() - rail_w - gap).max(200.0);
            let h = full.height().max(120.0);

            let rail_rect =
                egui::Rect::from_min_size(full.min, Vec2::new(rail_w, h));
            let main_rect = egui::Rect::from_min_size(
                egui::pos2(full.min.x + rail_w + gap, full.min.y),
                Vec2::new(main_w, h),
            );

            // 左轨：整卡裁剪，底栏（回收站/锁定）固定在卡片内底部
            {
                let mut rail_ui = ui.child_ui(rail_rect, egui::Layout::top_down(egui::Align::Min));
                rail_ui.set_clip_rect(rail_rect);
                // 描边内缩 0.5，避免 clip 吃掉圆角边线（尤其底部）
                let rail_shell = rail_rect.shrink(0.5);
                rail_ui.painter().rect(
                    rail_shell,
                    Rounding::same(theme::ROUND_SHELL),
                    theme::panel(),
                    Stroke::new(1.0, theme::border()),
                );
                let content = rail_rect.shrink2(Vec2::new(12.0, 14.0));
                let mut content_ui =
                    rail_ui.child_ui(content, egui::Layout::top_down(egui::Align::Min));
                content_ui.set_clip_rect(content);
                show_left_rail(
                    &mut content_ui,
                    shell,
                    &all,
                    svc.list_trash().len(),
                    task_overdue_n,
                    private_overdue_n,
                    care_needs,
                    online_count,
                    selected,
                    editing,
                    nav_locked,
                    show_settings,
                    &alias_show,
                    show_change_pw,
                    show_help,
                    show_about,
                    show_export,
                    export_pw,
                    export_path,
                    export_ids,
                    data_dir,
                    &mut action,
                );
            }

            // 主卡片
            {
                let mut main_ui = ui.child_ui(main_rect, egui::Layout::top_down(egui::Align::Min));
                main_ui.set_clip_rect(main_rect);
                theme::shell_card_frame().show(&mut main_ui, |ui| {
                    let inner = ui.max_rect();
                    ui.set_clip_rect(inner);
                    ui.set_min_size(Vec2::new(inner.width(), inner.height().max(80.0)));

                    // 顶栏
                    let top_rect =
                        egui::Rect::from_min_size(inner.min, Vec2::new(inner.width(), TOP_H));
                    {
                        let mut top = ui.child_ui(top_rect, egui::Layout::left_to_right(egui::Align::Center));
                        top.set_clip_rect(top_rect);
                        Frame::none()
                            .inner_margin(Margin::symmetric(18.0, 10.0))
                            .show(&mut top, |ui| {
                                show_shell_topbar(
                                    ui,
                                    shell,
                                    search,
                                    nav_locked,
                                    update_hint_ver,
                                    update_hint_url,
                                    &mut action,
                                );
                            });
                        // 底部分割线
                        ui.painter().hline(
                            top_rect.x_range(),
                            top_rect.bottom() - 0.5,
                            Stroke::new(1.0, theme::border()),
                        );
                    }

                    // 内容区
                    let content_rect = egui::Rect::from_min_max(
                        egui::pos2(inner.min.x, top_rect.bottom()),
                        inner.max,
                    );
                    shell.content_host = content_rect;
                    let mut body =
                        ui.child_ui(content_rect, egui::Layout::top_down(egui::Align::Min));
                    body.set_clip_rect(content_rect);
                    Frame::none()
                        .inner_margin(Margin::same(18.0))
                        .show(&mut body, |ui| {
                            if let Some(r) = women_care.as_ref().filter(|r| r.needs_attention()) {
                                show_period_care_banner(
                                    ui,
                                    r,
                                    &mut shell.nav,
                                    selected,
                                    editing,
                                    nav_locked,
                                );
                                ui.add_space(8.0);
                            }
                            if let Some(r) = male_care.as_ref().filter(|r| r.needs_attention()) {
                                show_male_care_banner(
                                    ui,
                                    r,
                                    &mut shell.nav,
                                    selected,
                                    editing,
                                    nav_locked,
                                );
                                ui.add_space(8.0);
                            }

                            let body_h = ui.available_height().max(60.0);
                            let body_w = ui.available_width();
                            let body_rect = egui::Rect::from_min_size(
                                ui.cursor().min,
                                Vec2::new(body_w, body_h),
                            );
                            let mut inner_body =
                                ui.child_ui(body_rect, egui::Layout::top_down(egui::Align::Min));
                            inner_body.set_clip_rect(body_rect);

                            if shell.nav.is_nodes() {
                                egui::ScrollArea::vertical()
                                    .id_source("shell_nodes_page")
                                    .auto_shrink([false, false])
                                    .show(&mut inner_body, |ui| {
                                        show_nodes_panel(
                                            ui,
                                            discovered,
                                            local_node_id,
                                            node_role,
                                            node_visible,
                                            lan_discovery,
                                            local_disk,
                                            &mut settings_draft.backup_targets,
                                            online_count,
                                            listen_error,
                                            &mut action,
                                        );
                                    });
                            } else if shell.nav.is_settings() {
                                settings_view::show_page(
                                    &mut inner_body,
                                    &mut shell.settings_tab,
                                    settings_draft,
                                    live_cfg,
                                    &alias_show,
                                    svc,
                                    status_line,
                                );
                            } else {
                                show_shell_body(
                                    &mut inner_body,
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
                                    empty_trash_confirm,
                                    &mut action,
                                    tx,
                                );
                            }
                            ui.allocate_rect(body_rect, Sense::hover());
                        });
                });
            }
        });

    if shell.gender.request_open_settings {
        shell.gender.request_open_settings = false;
        shell.show_care_settings = true;
    }
    if shell.cal.request_open_settings {
        shell.cal.request_open_settings = false;
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
    empty_trash_confirm: &mut bool,
    action: &mut ShellAction,
    tx: &Sender<BgMsg>,
) {
            if shell.nav.is_trash() {
                show_trash(
                    ui,
                    svc,
                    trash,
                    purge_confirm_id,
                    empty_trash_confirm,
                    status_line,
                    tx,
                );
            } else if shell.nav.is_today() || shell.nav.is_calendar() {
                let detail_open = selected
                    .as_ref()
                    .map(|id| !is_new_draft(id) && all.iter().any(|m| m.id == *id))
                    .unwrap_or(false);
                    let mut picked = None;
                let mut pick_edit = false;
                let mut pick_delete = false;
                let host = ui.available_rect_before_wrap();
                if shell.nav.is_today() {
                    calendar_view::show_today(
                        ui,
                        svc,
                        &mut shell.cal,
                        &all,
                        search,
                        selected,
                        &mut shell.list_focus,
                        editing,
                        &mut action.open_new_memo,
                        &mut picked,
                        &mut pick_edit,
                        &mut pick_delete,
                        &mut action.open_sticky,
                        tx,
                    );
                } else {
                calendar_view::show(
                    ui,
                    svc,
                                    &mut shell.cal,
                                    &all,
                    search,
                                    selected,
                                    &mut shell.list_focus,
                                    editing,
                                    &mut action.open_new_memo,
                                    &mut picked,
                    &mut pick_edit,
                    &mut pick_delete,
                    &mut action.open_sticky,
                    tx,
                );
                }
                if let Some(id) = picked {
                    if let Some(m) = all.iter().find(|m| m.id == id) {
                        load_selection(
                            svc,
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
                        shell.list_focus = Some(id);
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
                    // list() 懒解密无正文：优先用已加载的 body_draft，否则单条 load
                    let mut m_owned = selected
                        .as_ref()
                        .and_then(|id| all.iter().find(|x| x.id == *id))
                        .cloned();
                    if let Some(ref mut m) = m_owned {
                        if !m.content_loaded {
                            if selected.as_ref() == Some(&m.id) && !body_draft.is_empty() {
                                m.content = body_draft.clone();
                                m.content_loaded = true;
                            } else if let Ok(c) = svc.load_content(&m.id) {
                                m.content = c.clone();
                                m.content_loaded = true;
                                *title_draft = m.title.clone();
                                *body_draft = c;
                                *visibility_draft = m.visibility;
                                *category_draft = m.category.canonical();
                                *due_date_draft = m.due_date.clone();
                                *end_date_draft = m.end_date.clone();
                                *tags_draft = format_tags(&m.tags);
                                *done_draft = m.done;
                                *priority_draft = m.priority;
                                *memo_doc = doc_editor::Doc::from_store(&m.title, &m.content);
                            }
                        }
                    }
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
                                    &mut action.open_sticky,
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
                            svc,
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
                                svc,
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
                                    svc,
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
                    &mut shell.list_focus,
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
                    &mut shell.private_branch,
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
                                svc,
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
                                    svc,
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
                                svc,
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
                                    svc,
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

/// 主卡片顶栏：标题 · 搜索 · 新建（身份已迁至左栏顶部）。
#[allow(clippy::too_many_arguments)]
fn show_shell_topbar(
    ui: &mut egui::Ui,
    shell: &mut ShellUi,
    search: &mut String,
    nav_locked: bool,
    update_hint_ver: Option<&str>,
    update_hint_url: Option<&str>,
    action: &mut ShellAction,
) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_height(ui.available_height());
        ui.label(
            RichText::new(shell.nav.label())
                .size(22.0)
                .strong()
                .color(theme::text()),
        );
        if let (Some(ver), Some(url)) = (update_hint_ver, update_hint_url) {
            ui.add_space(10.0);
            show_update_hint(ui, ver, url);
        }

        // 搜索 + 新建：固定 TOP_BAR_CTRL_H，垂直中线对齐
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.set_height(TOP_BAR_CTRL_H);
            ui.spacing_mut().item_spacing.x = 10.0;

            let new_enabled = !nav_locked
                && (shell.nav.is_task_space()
                    || shell.nav.is_credentials()
                    || shell.nav.is_gender_private());
            let new_resp = ui
                .add_enabled_ui(new_enabled, |ui| {
                    ui.add_sized(
                        Vec2::new(68.0, TOP_BAR_CTRL_H),
                        egui::Button::new(
                            RichText::new("新建")
                                .size(13.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(theme::shell_accent())
                        .rounding(Rounding::same(9.0)),
                    )
                })
                .inner
                .on_hover_text(if new_enabled {
                    "新建条目"
                } else {
                    "当前页不支持新建，或请先关闭编辑"
                });
            if new_resp.clicked() {
                action.open_new_memo = true;
            }

            let search_w = page_shell::fit_avail_width(
                ui,
                (ui.available_width() - 10.0).min(320.0).max(140.0),
            );
            let (search_rect, search_click) =
                ui.allocate_exact_size(Vec2::new(search_w, TOP_BAR_CTRL_H), Sense::click());
            ui.painter().rect(
                search_rect,
                Rounding::same(9.0),
                theme::panel(),
                Stroke::new(1.0, theme::border()),
            );
            let font = egui::FontId::proportional(13.0);
            let line_h = ui.fonts(|f| f.row_height(&font));
            let text_rect = egui::Rect::from_min_size(
                egui::pos2(
                    search_rect.left() + 12.0,
                    search_rect.center().y - line_h * 0.5,
                ),
                egui::vec2((search_rect.width() - 24.0).max(40.0), line_h),
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
                    "搜索标题、标签…",
                    font,
                    theme::mac_text_tertiary(),
                );
            }
            if search_click.clicked() {
                te.request_focus();
            }
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn show_left_rail(
    ui: &mut egui::Ui,
    shell: &mut ShellUi,
    all: &[MemoView],
    trash_n: usize,
    task_overdue_n: usize,
    private_overdue_n: usize,
    care_needs: bool,
    online_count: usize,
    selected: &mut Option<String>,
    editing: &mut bool,
    nav_locked: bool,
    show_settings: &mut bool,
    alias_show: &str,
    show_change_pw: &mut bool,
    show_help: &mut bool,
    show_about: &mut bool,
    show_export: &mut bool,
    export_pw: &mut String,
    export_path: &mut String,
    export_ids: &mut Option<Vec<String>>,
    data_dir: &str,
    action: &mut ShellAction,
) {
    let _ = private_overdue_n;
    let full = ui.max_rect();
    ui.set_clip_rect(full);
    ui.allocate_rect(full, Sense::hover());

    // 底栏固定高度：分隔 + 回收站(≈40) + 锁定(≈34)（必须整段落在卡片内）
    const FOOT_H: f32 = 92.0;
    let foot_h = FOOT_H.min(full.height() * 0.4).max(80.0);
    let top_h = (full.height() - foot_h).max(40.0);
    let top_rect = egui::Rect::from_min_size(full.min, Vec2::new(full.width(), top_h));
    let foot_rect = egui::Rect::from_min_max(
        egui::pos2(full.min.x, full.max.y - foot_h),
        full.max,
    );

    // —— 上：身份（取代品牌行）+ 主导航 + 系统（可裁剪）——
    {
        let mut top = ui.child_ui(top_rect, egui::Layout::top_down(egui::Align::Min));
        top.set_clip_rect(top_rect);
        let identity_resp = show_identity_chip(&mut top, alias_show);
        let identity_menu_id = top.make_persistent_id("shell_identity_menu");
        if identity_resp.clicked() {
            top.memory_mut(|m| m.toggle_popup(identity_menu_id));
        }
        egui::popup_below_widget(&top, identity_menu_id, &identity_resp, |ui| {
            ui.set_min_width(148.0);
            if ui.button("设置").clicked() {
                shell.nav = NavItem::Settings;
                ui.memory_mut(|m| m.close_popup());
            }
            if ui.button("切换身份 / 锁定").clicked() {
                request_lock(shell, selected, editing, action);
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
        top.add_space(12.0);

        let care_pulse = if care_needs {
            Some(((top.input(|i| i.time) * 2.2).sin() * 0.5 + 0.5) as f32)
        } else {
            None
        };
        let cred_n = count_category(all, MemoCategory::Credentials);
        let priv_n = all.iter().filter(|m| m.space == MemoSpace::Private).count();

        for item in NavItem::PRIMARY {
            let badge = if matches!(item, NavItem::Today) && task_overdue_n > 0 {
                Some((task_overdue_n, BadgeKind::Blue))
            } else if matches!(item, NavItem::Credentials) && cred_n > 0 {
                Some((cred_n, BadgeKind::Soft))
            } else if matches!(item, NavItem::GenderPrivate) && priv_n > 0 {
                Some((priv_n, BadgeKind::Soft))
            } else {
                None
            };
            nav_row(
                &mut top,
                &mut shell.nav,
                *item,
                badge,
                care_pulse,
                nav_locked,
                show_settings,
                selected,
                editing,
                &mut shell.list_focus,
            );
        }

        top.add_space(8.0);
        top.separator();
        top.add_space(8.0);

        for item in NavItem::SYSTEM {
            let badge = if matches!(item, NavItem::Nodes) && online_count > 0 {
                Some((online_count, BadgeKind::Soft))
            } else {
                None
            };
            nav_row(
                &mut top,
                &mut shell.nav,
                *item,
                badge,
                None,
                nav_locked,
                show_settings,
                selected,
                editing,
                &mut shell.list_focus,
            );
        }
    }

    // —— 下：回收站 + 锁定（绝对贴底，不再参与上方流式排版）——
    {
        let mut foot = ui.child_ui(foot_rect, egui::Layout::top_down(egui::Align::Min));
        foot.set_clip_rect(foot_rect);
        foot.separator();
        foot.add_space(4.0);
        nav_row(
            &mut foot,
            &mut shell.nav,
            NavItem::Trash,
            if trash_n > 0 {
                Some((trash_n, BadgeKind::Gray))
            } else {
                None
            },
            None,
            nav_locked,
            show_settings,
            selected,
            editing,
            &mut shell.list_focus,
        );
        let lock_resp = foot.add(
            egui::Button::new(
                RichText::new("🔒  锁定")
                    .size(13.0)
                    .color(theme::text_muted()),
            )
            .fill(Color32::TRANSPARENT)
            .frame(false)
            .min_size(Vec2::new(foot.available_width(), 34.0)),
        );
        if lock_resp.clicked() {
            request_lock(shell, selected, editing, action);
        }
    }
}

#[allow(dead_code)]
fn show_top_tabs(
    ui: &mut egui::Ui,
    shell: &mut ShellUi,
    all: &[MemoView],
    trash_n: usize,
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
        .filter(|m| m.space == MemoSpace::Private && !is_private_calendar_memo(m))
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
            NavItem::Today => task_overdue_n,
            NavItem::Calendar | NavItem::Nodes | NavItem::Settings => 0,
        };
                    // 过期只用标签提示，页签本身不再红框/脉冲底；选中态保持原样
                    let overdue_tag = if (item.is_calendar() || item.is_today()) && task_overdue_n > 0 {
                        Some(if task_overdue_n > 99 {
                            "过期".into()
                        } else {
                            format!("过期{task_overdue_n}")
                        })
                    } else if item.is_gender_private()
                        && (care_needs || private_overdue_n > 0)
                    {
                        Some(if private_overdue_n > 0 {
                            if private_overdue_n > 99 {
                                "过期".into()
                            } else {
                                format!("过期{private_overdue_n}")
                            }
                        } else {
                            "关怀".into()
                        })
                    } else {
                        None
                    };
                    let icon_fg = if tab_locked {
                        theme::text_muted().linear_multiply(0.55)
                    } else if sel {
                        theme::shell_accent()
                    } else {
                        theme::text_muted()
                    };
                    let label_fg = if tab_locked {
                        theme::text_muted().linear_multiply(0.55)
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
                    let overdue_g = overdue_tag.as_ref().map(|t| {
                        theme::layout_galley(
                            ui,
                            t,
                            egui::FontId::proportional(10.0),
                            theme::warn_overdue_fg(),
                        )
                    });
                    let icon_slot = 16.0_f32.max(icon_g.mesh_bounds.width());
                    let text_w = label_g.as_ref().map(|g| g.size().x).unwrap_or(0.0);
                    let badge_w = if badge > 0 { 20.0 } else { 0.0 };
                    let overdue_w = overdue_g
                        .as_ref()
                        .map(|g| (g.size().x + 10.0).max(28.0))
                        .unwrap_or(0.0);
                    let w = if trash {
                        (8.0 + icon_slot + badge_w + 8.0).max(32.0)
                    } else {
                        (10.0
                            + icon_slot
                            + 6.0
                            + text_w
                            + if badge > 0 { 6.0 + badge_w } else { 0.0 }
                            + if overdue_w > 0.0 {
                                6.0 + overdue_w
                            } else {
                                0.0
                            }
                            + 10.0)
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
                    // 选中 / 悬停（过期不再改整页签底色与边框）
                    if sel {
                        ui.painter().rect(
                            rect,
                            Rounding::same(8.0),
                            theme::card(),
                            Stroke::new(1.0, Color32::from_black_alpha(12)),
                        );
                    } else if tab_locked {
                        // 禁用：无悬停高亮
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
                        let (bbg, bfg) = if sel {
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
                        x += badge_w;
                    }
                    if let Some(og) = overdue_g {
                        x += 6.0;
                        let tr = egui::Rect::from_center_size(
                            egui::pos2(x + overdue_w * 0.5, cy),
                            Vec2::new(overdue_w, 16.0),
                        );
                        ui.painter().rect_filled(
                            tr,
                            Rounding::same(8.0),
                            theme::warn_overdue_bg(),
                        );
                        ui.painter().galley(
                            theme::galley_pos_center(tr.center(), &og),
                            og,
                            theme::warn_overdue_fg(),
                        );
                    }
        if !tab_locked && resp.clicked() {
            shell.nav = *item;
            shell.list_focus = None;
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
    list_focus: &mut Option<String>,
) {
    let sel = *current == item && !locked;
    ui.add_space(2.0);
    let height = 38.0;
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::click(),
    );
    // 整行一体交互：不用 Label 子控件，避免抢走悬停并把光标变成 I 型
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = if locked {
        resp.on_hover_text("请先关闭新建或编辑")
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
        .rect_filled(rect, Rounding::same(9.0), fill);

    if let Some(p) = care_pulse.filter(|_| item.is_gender_private()) {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            Rounding::same(9.0),
            Stroke::new(1.2, theme::period_day().linear_multiply(p)),
        );
    }

    let muted = theme::mac_text_tertiary();
    let icon_fg = if locked {
        muted
    } else if sel {
        theme::shell_accent()
    } else {
        theme::text_muted()
    };
    let label_fg = if locked {
        muted
    } else if sel {
        theme::shell_accent()
    } else if care_pulse.is_some() && item.is_gender_private() {
        theme::shell_women_pill_fg()
    } else {
        theme::text_muted()
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
            return;
        }
        *current = item;
        *list_focus = None;
        // 设置走中央页，不再强制弹窗
        if item.is_gender_private() {
            // 私密默认进关怀卡，不强制 hub
            *selected = None;
            *editing = false;
        } else if !item.is_settings() {
            *selected = None;
            *editing = false;
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
    let category = form.category.for_store();
    let space = form.space;
    let lan_share = form.lan_share;
    let due = form.due_date.clone();
    let end = form.end_date.clone();
    let tags = memo_form::tags_vec(form);
    let priority = form.priority;
    let remind = form.remind_before_days;
    let attach_id = form.attach_id.clone();
    let svc = svc.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        match svc.add_full_with_id(
            &attach_id,
            &title,
            &body,
            space,
            lan_share,
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
    empty_trash_confirm: &mut bool,
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
        if !trash.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::danger_button(ui, "清空回收站").clicked() {
                    *empty_trash_confirm = true;
                }
            });
        }
    });
    ui.label(
        theme::muted_label(format!(
            "已删除备忘保留 {} 天，可恢复或彻底清除；可一键清空。",
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

fn load_memo_body(svc: &MemoService, m: &MemoView) -> String {
    if m.content_loaded {
        m.content.clone()
    } else {
        svc.load_content(&m.id).unwrap_or_default()
    }
}

fn load_selection(
    svc: &MemoService,
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
    let body = load_memo_body(svc, m);
    *selected = Some(m.id.clone());
    *title_draft = m.title.clone();
    *body_draft = body.clone();
    *visibility_draft = m.visibility;
    *category_draft = m.category.canonical();
    *due_date_draft = m.due_date.clone();
    *end_date_draft = m.end_date.clone();
    *tags_draft = format_tags(&m.tags);
    *done_draft = m.done;
    *priority_draft = m.priority;
    *editing = false;
    *memo_doc = doc_editor::Doc::from_store(&m.title, &body);
    *edit_form = MemoFormState::load_from_drafts(
        m.category.canonical(),
        &m.title,
        &body,
        &m.due_date,
        &m.end_date,
        m.priority,
        tags_draft,
        m.done,
        m.visibility,
        m.space,
        m.lan_share,
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
    list_focus: &mut Option<String>,
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
    _cred_sort: &mut CredSort,
    cred_tag: &mut Option<String>,
    private_branch: &mut PrivateBranch,
    _open_new_memo: &mut bool,
    open_sticky: &mut Option<String>,
    tx: &Sender<BgMsg>,
) {
    let avail = ui.available_width();
    // 必须在 horizontal 之前取高度：horizontal 内 available_height 不可靠
    let full_h = ui.available_height().max(120.0);
    let gap = page_shell::PAGE_GAP;
    // 备忘 / 私密任务备忘：单栏列表 + 叠层抽屉（对齐 HTML）
    // 其它：左列表 · 右详情
    let (list_w, detail_w) = if nav.is_credentials() || nav.is_gender_private() {
        (avail.max(1.0), 0.0)
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
    let private_cred_list =
        private_page && matches!(*private_branch, PrivateBranch::Credentials);
    let card_list = credentials_page || private_cred_list;
    // 备忘 / 私密：页头进左栏，右栏顶对齐上移

    // 私密：顶部分段对齐 HTML（关怀 | 私密任务 | 私密备忘）
    if private_page {
        let prev_branch = *private_branch;
        show_private_seg(ui, private_branch);
        if *private_branch != prev_branch {
            // 切分段时收起关怀日历 / 详情选中，回到对应列表入口
            if selected.as_deref() == Some(HUB_GENDER_PRIVATE) || is_new_draft(selected.as_deref().unwrap_or("")) {
                *selected = None;
            }
            *editing = false;
            if matches!(*private_branch, PrivateBranch::Care) {
                *list_focus = None;
            }
        }
        ui.add_space(16.0);
        if matches!(*private_branch, PrivateBranch::Care) {
            if selected.as_deref() == Some(HUB_GENDER_PRIVATE) {
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("← 关怀入口")
                                    .size(12.5)
                                    .strong()
                                    .color(theme::shell_accent()),
                            )
                            .fill(Color32::TRANSPARENT)
                            .frame(false),
                        )
                        .clicked()
                    {
                        *selected = None;
                        *editing = false;
                    }
                });
                ui.add_space(10.0);
                gender_private_view::show(ui, svc, gender_ui, all_memos, status_line);
            } else {
                ui.label(
                    RichText::new("本机私密 · 经期与体检。点卡片进入关怀日历。")
                        .size(13.0)
                        .color(theme::mac_text_tertiary()),
                );
                ui.add_space(14.0);
                show_private_care_cards(ui, svc, gender_ui, selected, editing, status_line);
            }
            return;
        }
    }

    // 备忘：提示 + 顶栏 chip（全部 / 账号 / 证件 / 网络）对齐 HTML
    if credentials_page {
        ui.label(
            RichText::new("账号、证件等不上日历。列表扫一眼，详情里完成复制与核对。")
                .size(13.0)
                .color(theme::mac_text_tertiary()),
        );
        ui.add_space(10.0);
        show_cred_top_chips(ui, cred_filter, cred_tag);
        ui.add_space(12.0);
    }

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
                    );
                    ui.add_space(page_shell::PAGE_GAP);
                } else if private_page {
                    // 分段在外层全宽绘制；左栏仅列表时用
                    if !matches!(*private_branch, PrivateBranch::Care) {
                        page_shell::show_page_header(
                            ui,
                            match *private_branch {
                                PrivateBranch::Todo => "私密任务",
                                PrivateBranch::Credentials => "私密备忘",
                                PrivateBranch::Care => "关怀",
                            },
                            &format!("{list_n} 条"),
                        );
                        ui.add_space(page_shell::PAGE_GAP);
                    }
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
                    });
                    ui.add_space(8.0);
                }

                // 私密页：过期 / 今天到期 / 即将到期(3天内) / 其它
                let private_warn = nav.is_gender_private();
                let mut overdue_ids = std::collections::HashSet::new();
                let mut due_today_ids = std::collections::HashSet::new();
                let mut due_soon_ids = std::collections::HashSet::new();
                let display: Vec<&MemoView> = if private_warn {
                    let branch_cat = match *private_branch {
                        PrivateBranch::Todo | PrivateBranch::Care => MemoCategory::Todo,
                        PrivateBranch::Credentials => MemoCategory::Credentials,
                    };
                    let rest: Vec<_> = filtered
                        .iter()
                        .filter(|m| {
                            !is_private_hub(&m.id) && m.category.bucket() == branch_cat
                        })
                        .collect();
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
                            svc,
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
                    // 不插「关怀与健康」虚拟卡；关怀入口只在「关怀」分段
                    let mut out = Vec::with_capacity(filtered.len());
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
                            let empty = if private_page {
                                match *private_branch {
                                    PrivateBranch::Todo => "还没有私密任务",
                                    PrivateBranch::Credentials => "还没有私密备忘",
                                    PrivateBranch::Care => "暂无内容",
                                }
                            } else if credentials_page {
                                "还没有备忘"
                            } else {
                                "暂无备忘"
                            };
                            ui.add_space(24.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(empty)
                                        .size(13.5)
                                        .color(theme::text_muted()),
                                );
                            });
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
                            let is_sel = !is_hub
                                && (selected.as_deref() == Some(m.id.as_str())
                                    || list_focus.as_deref() == Some(m.id.as_str()));
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
                            // 选中主信号：强调软底；描边保持普通边框
                            let border = if is_hub {
                                theme::period_day()
                            } else if let Some(c) = stripe {
                                if is_sel {
                                    theme::border()
                                } else {
                                    c
                                }
                            } else {
                                theme::border()
                            };
                            let card_fill = if is_sel {
                                theme::shell_accent_soft()
                            } else if is_hub {
                                theme::shell_women_pill().linear_multiply(0.45)
                            } else {
                                theme::card()
                            };
                            let mut toggled_done = false;
                            let resp = Frame::none()
                                .fill(card_fill)
                                .stroke(Stroke::new(1.0, border))
                                .rounding(Rounding::same(if card_list { 10.0 } else { 12.0 }))
                                .inner_margin(Margin::symmetric(
                                    if card_list { 14.0 } else { 12.0 },
                                    if card_list { 12.0 } else { 10.0 },
                                ))
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    if card_list && !is_hub {
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
                                    *list_focus = None;
                                } else {
                                    load_selection(
                                        svc,
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
                                    *list_focus = Some(m.id.clone());
                                }
                            }
                            if !is_hub {
                                resp.context_menu(|ui| {
                                    theme::style_context_menu(ui);
                                    if theme::menu_item_mark(ui, "开", "打开").clicked() {
                                        load_selection(
                                            svc,
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
                                        *list_focus = Some(m.id.clone());
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
                            ui.add_space(if card_list { 8.0 } else { 6.0 });
                        }
                    });
            },
        );

        // 备忘 / 私密列表无右栏（详情走叠层抽屉）
        if !credentials_page && !private_page && detail_w > 1.0 {
            ui.allocate_ui_with_layout(
                Vec2::new(detail_w, body_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(body_h);
                    ui.set_max_height(body_h);
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
                        open_sticky,
                        tx,
                    );
                },
            );
        }
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
                    let mut scroll = egui::ScrollArea::vertical()
                        .id_source(("memo_form_drawer_scroll", id_salt))
                        .auto_shrink([false, false])
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
/// 备忘顶栏 chip：全部 / 账号 / 证件 / 网络（对齐 HTML）。
fn show_cred_top_chips(
    ui: &mut egui::Ui,
    filter: &mut CredFilter,
    tag_filter: &mut Option<String>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().item_spacing.y = 6.0;
        for (label, tag) in [
            ("全部", None),
            ("账号", Some("账号")),
            ("证件", Some("证件")),
            ("网络", Some("网络")),
        ] {
            let on = match tag {
                None => *filter == CredFilter::All && tag_filter.is_none(),
                Some(t) => tag_filter.as_deref() == Some(t),
            };
            if cred_rail_chip(ui, label, on).clicked() {
                *filter = CredFilter::All;
                *tag_filter = tag.map(str::to_string);
            }
        }
    });
}

#[allow(dead_code)]
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

                    cred_rail_section_label(ui, "标签筛选");
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.spacing_mut().item_spacing.y = 6.0;
                        for (f, label) in [
                            (CredFilter::All, "全部"),
                            (CredFilter::TypeCredentials, "账号证件"),
                            (CredFilter::TypeWork, "工作"),
                            (CredFilter::TypeLife, "生活"),
                            (CredFilter::TypeFinance, "财务"),
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

fn archive_type_label(c: MemoCategory, tags: &[String]) -> &'static str {
    const LEGACY: &[&str] = &["工作", "生活", "财务", "灵感", "应急", "随手记"];
    for need in LEGACY {
        if tags.iter().any(|t| t == *need) {
            return need;
        }
    }
    if let Some(tag) = c.legacy_tag() {
        return tag;
    }
    match c.bucket() {
        MemoCategory::Credentials => "账号证件",
        MemoCategory::GenderPrivate => "私密",
        _ => "任务",
    }
}

fn show_private_seg(ui: &mut egui::Ui, private_branch: &mut PrivateBranch) {
    // 浅底轨 + 选中松绿软底（相对 panel 有对比）+ accent 字
    Frame::none()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::same(3.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (b, label) in [
                    (PrivateBranch::Care, "关怀"),
                    (PrivateBranch::Todo, "私密任务"),
                    (PrivateBranch::Credentials, "私密备忘"),
                ] {
                    let on = *private_branch == b;
                    let fill = if on {
                        theme::shell_accent_soft()
                    } else {
                        Color32::TRANSPARENT
                    };
                    let fg = if on {
                        theme::shell_accent()
                    } else {
                        theme::text_muted()
                    };
                    let btn = egui::Button::new(
                        RichText::new(label).size(13.0).strong().color(fg),
                    )
                    .fill(fill)
                    .rounding(Rounding::same(8.0))
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::new(0.0, 32.0));
                    let resp = ui.add(btn);
                    if on {
                        let r = resp.rect.expand(0.5);
                        ui.painter().rect(
                            r.translate(Vec2::new(0.0, 1.0)),
                            Rounding::same(8.0),
                            Color32::from_black_alpha(18),
                            Stroke::NONE,
                        );
                    }
                    if resp.clicked() {
                        *private_branch = b;
                    }
                }
            });
        });
}

fn show_private_care_cards(
    ui: &mut egui::Ui,
    svc: &Arc<MemoService>,
    gender_ui: &mut GenderPrivateUi,
    selected: &mut Option<String>,
    editing: &mut bool,
    status_line: &mut String,
) {
    let women = period_view::care_remind(svc, &svc.list());
    let male = male_view::care_remind(svc);
    let period_tag = women
        .as_ref()
        .map(|r| {
            if r.days_until >= 0 {
                format!("预计 {} 天后", r.days_until)
            } else {
                "请关注周期".into()
            }
        })
        .unwrap_or_else(|| "开通后可预测".into());
    let check_tag = male
        .as_ref()
        .map(|r| {
            if r.days_until >= 0 {
                format!("还有 {} 天", r.days_until)
            } else {
                "可标注体检日".into()
            }
        })
        .unwrap_or_else(|| "可标注体检日".into());

    let avail = ui.available_width();
    let gap = 12.0;
    let col = if avail > 420.0 {
        ((avail - gap) * 0.5).max(180.0)
    } else {
        avail
    };
    let use_row = avail > 420.0;

    let cards = [
        (
            "经期",
            "记录与预测周期，仅本机私密空间。",
            period_tag.as_str(),
        ),
        (
            "体检",
            "下次体检提醒与当日备注。",
            check_tag.as_str(),
        ),
    ];

    let mut draw_card = |ui: &mut egui::Ui, title: &str, desc: &str, tag: &str, width: f32| {
        let w = (width - 4.0).max(120.0);
        let id = ui.make_persistent_id(("priv_care_card", title));
        let hovered = ui.memory(|m| m.data.get_temp::<bool>(id).unwrap_or(false));
        let border = if hovered {
            theme::shell_accent()
        } else {
            theme::border()
        };
        // 近似 HTML：暖白底 + 悬停描边变绿
        let fill = Color32::from_rgb(252, 250, 247);
        let resp = Frame::none()
            .fill(fill)
            .stroke(Stroke::new(if hovered { 1.5 } else { 1.0 }, border))
            .rounding(Rounding::same(12.0))
            .inner_margin(Margin::same(18.0))
            .show(ui, |ui| {
                ui.set_width(w);
                ui.set_min_height(118.0);
                ui.label(
                    RichText::new(title)
                        .size(18.0)
                        .strong()
                        .color(theme::text()),
                );
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(desc).size(13.0).color(theme::text_muted()),
                    )
                    .wrap(true),
                );
                ui.add_space(14.0);
                ui.label(
                    RichText::new(tag)
                        .size(11.5)
                        .strong()
                        .color(theme::shell_accent()),
                );
            })
            .response
            .interact(Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.memory_mut(|m| m.data.insert_temp(id, resp.hovered()));
        if resp.hovered() {
            ui.ctx().request_repaint();
        }
        if resp.clicked() {
            select_gender_hub(selected, editing);
            gender_ui.tip.clear();
            *status_line = format!("已打开{title}");
        }
    };

    if use_row {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (title, desc, tag) in cards {
                draw_card(ui, title, desc, tag, col);
            }
        });
    } else {
        for (title, desc, tag) in cards {
            draw_card(ui, title, desc, tag, avail);
            ui.add_space(gap);
        }
    }
}

/// 备忘列表行：对齐 HTML memo-card（图标块 · 标题/摘要 · 类型 pill）。
fn show_cred_list_row_body(ui: &mut egui::Ui, m: &MemoView) {
    let letter = m
        .title
        .chars()
        .find(|c| !c.is_whitespace())
        .unwrap_or('备')
        .to_string();
    let type_name = archive_type_label(m.category, &m.tags);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (ico, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
        ui.painter()
            .rect_filled(ico, Rounding::same(11.0), theme::shell_accent_soft());
        ui.painter().text(
            ico.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            egui::FontId::proportional(16.0),
            theme::shell_accent(),
        );
        ui.vertical(|ui| {
            let mut title = RichText::new(if m.title.is_empty() {
                "(无标题)"
            } else {
                m.title.as_str()
            })
            .strong()
            .size(15.0)
            .color(if m.done {
                theme::text_muted()
            } else {
                theme::text()
            });
            if m.done {
                title = title.strikethrough();
            }
            ui.label(title);
            ui.label(
                RichText::new(format!(
                    "{} · 更新 {}",
                    type_name,
                    format_relative_modified(&m.modified_at)
                ))
                .size(12.5)
                .color(theme::mac_text_tertiary()),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let _ = pill(
                ui,
                theme::shell_chip_gray(),
                theme::shell_chip_gray_fg(),
                type_name,
                22.0,
            );
        });
    });
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

            // RTL：更多 → 编辑（新建统一走主顶栏）
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

            // 编辑在右侧抽屉完成；页头只保留入口（新建统一走主顶栏）
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
        meta.map(|m| m.space).unwrap_or(MemoSpace::Normal),
        meta.map(|m| m.lan_share).unwrap_or(LanShare::Off),
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
            *category_draft = edit_form.category.for_store();
            *due_date_draft = edit_form.due_date.clone();
            *end_date_draft = edit_form.end_date.clone();
            *priority_draft = edit_form.priority;
            *tags_draft = edit_form.tags.clone();
            *done_draft = edit_form.done;
            *visibility_draft = if edit_form.lan_share.is_on() {
                MemoVisibility::Public
            } else {
                MemoVisibility::Private
            };
            *memo_doc = edit_form.doc.clone();
            let space = edit_form.space;
            let lan = edit_form.lan_share;
            let cat = edit_form.category.for_store();
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
                    space,
                    lan,
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
    svc: &MemoService,
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
    let body = load_memo_body(svc, m);
    *title_draft = m.title.clone();
    *body_draft = body.clone();
    *visibility_draft = m.visibility;
    *category_draft = m.category.canonical();
    *due_date_draft = m.due_date.clone();
    *end_date_draft = m.end_date.clone();
    *tags_draft = format_tags(&m.tags);
    *done_draft = m.done;
    *priority_draft = m.priority;
    *memo_doc = doc_editor::Doc::from_store(&m.title, &body);
    *edit_form = MemoFormState::load_from_drafts(
        m.category.canonical(),
        &m.title,
        &body,
        &m.due_date,
        &m.end_date,
        m.priority,
        tags_draft,
        m.done,
        m.visibility,
        m.space,
        m.lan_share,
        m.remind_before_days,
        &m.id,
    );
}

/// 私密条目只读预览（对齐 HTML：标题 → 摘要 → 正文 → 行动）。
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
    let type_name = archive_type_label(*category_draft, &parse_tags(tags_draft));

    // 状态徽章
    let (badge_bg, badge_fg, badge_text) = if *done_draft {
        (theme::success_soft(), theme::success(), "已完成")
    } else if memo_core::is_overdue(due_date_draft, end_date_draft, *done_draft) {
        (
            theme::warn_overdue_bg(),
            theme::warn_overdue_fg(),
            "已过期",
        )
    } else if memo_core::is_due_today(due_date_draft, end_date_draft, *done_draft) {
        (
            theme::warn_due_today_bg(),
            theme::warn_due_today_fg(),
            "今天到期",
        )
    } else {
        (
            theme::shell_accent_soft(),
            theme::shell_accent(),
            "私密",
        )
    };
    Frame::none()
        .fill(badge_bg)
        .rounding(Rounding::same(99.0))
        .inner_margin(Margin::symmetric(10.0, 4.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(badge_text)
                    .size(12.0)
                    .strong()
                    .color(badge_fg),
            );
        });
    ui.add_space(12.0);

    let title_show = if title_draft.is_empty() {
        "(无标题)"
    } else {
        title_draft.as_str()
    };
    let mut heading = RichText::new(title_show)
        .size(24.0)
        .strong()
        .color(theme::text());
    if *done_draft {
        heading = heading.strikethrough().color(theme::text_muted());
    }
    ui.label(heading);

    let mut sub_bits: Vec<String> = vec![type_name.to_string()];
    if let Some(m) = meta {
        sub_bits.push(format!("更新 {}", format_relative_modified(&m.modified_at)));
    }
    if !due_date_draft.is_empty() {
        let d = memo_core::display_due(due_date_draft);
        if !end_date_draft.trim().is_empty() {
            sub_bits.push(format!("{d} ～ {}", end_date_draft.trim()));
        } else {
            sub_bits.push(format!("到期 {d}"));
        }
    }
    let tags = parse_tags(tags_draft);
    if !tags.is_empty() {
        sub_bits.push(
            tags.iter()
                .map(|t| format!("#{t}"))
                .collect::<Vec<_>>()
                .join("  "),
        );
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new(sub_bits.join(" · "))
            .size(13.0)
            .color(theme::text_muted()),
    );

    // 到期警示（仅必要时）
    let od = memo_core::is_overdue(due_date_draft, end_date_draft, *done_draft);
    let dt = memo_core::is_due_today(due_date_draft, end_date_draft, *done_draft);
    let soon = memo_core::is_due_soon(
        due_date_draft,
        end_date_draft,
        *done_draft,
        PRIVATE_DUE_SOON_DAYS,
    );
    if od || dt || soon {
        ui.add_space(12.0);
        let days_left = memo_core::days_until_due(due_date_draft, end_date_draft, *done_draft);
        let (bg, fg, border, line) = if od {
            let n = memo_core::overdue_days(due_date_draft, end_date_draft, *done_draft)
                .unwrap_or(1);
            (
                theme::warn_overdue_bg(),
                theme::warn_overdue_fg(),
                theme::warn_overdue_border(),
                format!("已逾期 {n} 天"),
            )
        } else if dt {
            (
                theme::warn_due_today_bg(),
                theme::warn_due_today_fg(),
                theme::warn_due_today_border(),
                "今天到期".into(),
            )
        } else {
            let n = days_left.unwrap_or(1);
            (
                theme::warn_due_soon_bg(),
                theme::warn_due_soon_fg(),
                theme::warn_due_soon_border(),
                format!("还有 {n} 天"),
            )
        };
        Frame::none()
            .fill(bg)
            .stroke(Stroke::new(1.0, border))
            .rounding(Rounding::same(8.0))
            .inner_margin(Margin::symmetric(12.0, 8.0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(line).size(13.0).strong().color(fg));
            });
    }

    ui.add_space(18.0);
    if body_draft.trim().is_empty() {
        ui.label(
            RichText::new("暂无正文")
                .size(14.5)
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
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
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
        if theme::page_outline_button(ui, "复制").clicked() {
            ui.output_mut(|o| {
                o.copied_text = format!("{}\n\n{}", title_draft, body_draft)
            });
            *status_line = "已复制到剪贴板".into();
        }
        if theme::page_outline_button(ui, "删除").clicked() {
            *show_delete = true;
        }
        // 次要：导出 / 历史收在菜单感更弱的文字按钮后
        if theme::page_outline_button(ui, "导出").clicked() {
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
        if theme::page_outline_button(ui, "历史").clicked() {
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
    });
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
                theme::muted_label("点左侧列表查看详情\n或点右上角「新建」添加").size(14.0),
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
    let detail_tags = meta.map(|m| m.tags.as_slice()).unwrap_or(&[]);
    let type_name = archive_type_label(*category_draft, detail_tags);

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

    // 可复制字段：从正文解析「键：值」行（对齐 HTML copy-row）
    let kv_fields = parse_body_kv_fields(body_draft);
    if !kv_fields.is_empty() {
        Frame::none()
            .fill(theme::panel())
            .stroke(Stroke::new(1.0, theme::border()))
            .rounding(Rounding::same(12.0))
            .inner_margin(Margin::same(14.0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(
                    RichText::new("可复制字段")
                        .size(11.0)
                        .strong()
                        .color(theme::text_muted()),
                );
                ui.add_space(10.0);
                for (lab, val) in &kv_fields {
                    Frame::none()
                        .fill(Color32::WHITE)
                        .stroke(Stroke::new(1.0, theme::border()))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::symmetric(12.0, 10.0))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(lab)
                                        .size(11.5)
                                        .strong()
                                        .color(theme::text_muted()),
                                );
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(val).size(13.5).color(theme::text()),
                                    )
                                    .wrap(true),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("复制")
                                                        .size(12.0)
                                                        .strong()
                                                        .color(theme::shell_accent()),
                                                )
                                                .fill(theme::shell_accent_soft())
                                                .rounding(Rounding::same(7.0))
                                                .min_size(Vec2::new(48.0, 28.0)),
                                            )
                                            .clicked()
                                        {
                                            ui.output_mut(|o| o.copied_text = val.clone());
                                            *status_line = "已复制到剪贴板".into();
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(8.0);
                }
            });
        ui.add_space(12.0);
    }

    Frame::none()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new("正文")
                    .size(11.0)
                    .strong()
                    .color(theme::text_muted()),
            );
            ui.add_space(10.0);
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
        });

    ui.add_space(16.0);
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

/// 从正文提取短「键：值」行，供备忘详情一键复制。
fn parse_body_kv_fields(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let split = line
            .split_once('：')
            .or_else(|| line.split_once(':'));
        let Some((k, v)) = split else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        if k.is_empty() || v.is_empty() {
            continue;
        }
        if k.chars().count() > 12 || k.contains(' ') && k.chars().count() > 8 {
            continue;
        }
        // 跳过像 URL 协议头之类
        if k.eq_ignore_ascii_case("http") || k.eq_ignore_ascii_case("https") {
            continue;
        }
        out.push((k.to_string(), v.to_string()));
        if out.len() >= 8 {
            break;
        }
    }
    out
}
