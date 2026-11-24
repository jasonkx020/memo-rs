//! 设置页：对齐 HTML 效果稿的左分类 + 右表单。

use crate::autostart;
use crate::gender_private_view;
use crate::theme;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::config::{self, Config, NodeRole};
use memo_core::service::MemoService;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Look,
    Local,
    Care,
    Net,
    Backup,
}

pub fn settings_item_header(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(
        RichText::new(title)
            .size(12.0)
            .strong()
            .color(theme::text_muted()),
    );
    if !description.is_empty() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(description)
                .size(12.0)
                .color(theme::mac_text_tertiary()),
        );
    }
    ui.add_space(6.0);
}

pub fn settings_item_gap(ui: &mut egui::Ui) {
    ui.add_space(14.0);
}

pub fn settings_section(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .size(18.0)
            .strong()
            .color(theme::text()),
    );
    ui.add_space(14.0);
}

pub fn settings_text_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> bool {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(width)
            .hint_text(theme::hint(hint)),
    )
    .changed()
}

fn theme_pref_label(p: memo_core::ThemePreference) -> &'static str {
    p.label()
}

fn auto_lock_label(minutes: u32) -> String {
    match minutes {
        0 => "不自动锁定".into(),
        1 => "1 分钟".into(),
        n => format!("{n} 分钟"),
    }
}

fn switch_row(ui: &mut egui::Ui, lbl: &str, desc: &str, on: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(lbl)
                    .size(14.0)
                    .strong()
                    .color(theme::text()),
            );
            ui.label(
                RichText::new(desc)
                    .size(12.0)
                    .color(theme::mac_text_tertiary()),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed = ui.checkbox(on, "").changed();
        });
    });
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(8.0);
    changed
}

/// 中央设置页（HTML settings-layout）。
pub fn show_page(
    ui: &mut egui::Ui,
    tab: &mut SettingsTab,
    draft: &mut Config,
    live_cfg: &mut Config,
    alias: &str,
    svc: &Arc<MemoService>,
    status_line: &mut String,
) {
    let nav_w = 140.0_f32.min(ui.available_width() * 0.28).max(110.0);
    let gap = 16.0;
    let h = ui.available_height().max(200.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        // 左分类：自绘行（勿用 Button 默认 hover 描边，贴边会被 clip 裁掉左边）
        ui.allocate_ui_with_layout(
            Vec2::new(nav_w, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(nav_w);
                let inset = 2.0_f32;
                for (t, label) in [
                    (SettingsTab::Look, "外观"),
                    (SettingsTab::Local, "本机"),
                    (SettingsTab::Care, "关怀"),
                    (SettingsTab::Net, "网络"),
                    (SettingsTab::Backup, "备份"),
                ] {
                    let on = *tab == t;
                    let care = matches!(t, SettingsTab::Care);
                    let (outer, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 38.0),
                        Sense::hover(),
                    );
                    let rect = outer.shrink2(Vec2::new(inset, 1.0));
                    let resp = ui.interact(
                        rect,
                        ui.id().with(("settings_nav", label)),
                        Sense::click(),
                    );
                    let hovered = resp.hovered();
                    if hovered {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let fill = if on {
                        if care {
                            theme::shell_women_pill()
                        } else {
                            theme::shell_accent_soft()
                        }
                    } else if hovered {
                        theme::c().list_hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    let fg = if on || hovered {
                        if care {
                            theme::shell_women_pill_fg()
                        } else {
                            theme::shell_accent()
                        }
                    } else {
                        theme::text_muted()
                    };
                    if fill != Color32::TRANSPARENT {
                        ui.painter()
                            .rect_filled(rect, Rounding::same(8.0), fill);
                    }
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        label,
                        egui::FontId::proportional(13.5),
                        fg,
                    );
                    if resp.clicked() {
                        *tab = t;
                    }
                }
            },
        );

        // 右表单：高度钳在分配区内，避免 Frame 边距把底边撑出 clip
        let pane_w = (ui.available_width() - 4.0).max(200.0);
        ui.allocate_ui_with_layout(
            Vec2::new(pane_w, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                const FRAME_PAD: f32 = 20.0;
                let frame_h = ui.available_height().max(160.0);
                let stroke_h = 2.0;
                let inner_budget =
                    (frame_h - FRAME_PAD * 2.0 - stroke_h).max(80.0);
                Frame::none()
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .rounding(Rounding::same(12.0))
                    .inner_margin(Margin::same(FRAME_PAD))
                    .show(ui, |ui| {
                        ui.set_min_height(inner_budget);
                        ui.set_max_height(inner_budget);
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_source("settings_page_scroll")
                            .auto_shrink([false, false])
                            .max_height(inner_budget)
                            .show(ui, |ui| {
                                let field_w = ui.available_width().min(420.0);
                                match *tab {
                                    SettingsTab::Look => {
                                        settings_section(ui, "外观");
                                        settings_item_header(ui, "主题", "");
                                        egui::ComboBox::from_id_source("settings_page_theme")
                                            .selected_text(theme_pref_label(draft.theme))
                                            .width(field_w)
                                            .show_ui(ui, |ui| {
                                                for p in [
                                                    memo_core::ThemePreference::System,
                                                    memo_core::ThemePreference::Light,
                                                    memo_core::ThemePreference::Dark,
                                                    memo_core::ThemePreference::Blush,
                                                ] {
                                                    ui.selectable_value(
                                                        &mut draft.theme,
                                                        p,
                                                        theme_pref_label(p),
                                                    );
                                                }
                                            });
                                        if draft.theme != live_cfg.theme {
                                            live_cfg.theme = draft.theme;
                                            let mut persist = live_cfg.clone();
                                            persist.theme = draft.theme;
                                            let _ = config::save_settings(&persist);
                                        }
                                        settings_item_gap(ui);
                                        settings_item_header(ui, "自动锁定", "");
                                        egui::ComboBox::from_id_source("settings_page_autolock")
                                            .selected_text(auto_lock_label(
                                                draft.auto_lock_minutes,
                                            ))
                                            .width(field_w)
                                            .show_ui(ui, |ui| {
                                                for m in [0, 1, 3, 5, 10, 15, 30, 60] {
                                                    ui.selectable_value(
                                                        &mut draft.auto_lock_minutes,
                                                        m,
                                                        auto_lock_label(m),
                                                    );
                                                }
                                            });
                                        if draft.auto_lock_minutes != live_cfg.auto_lock_minutes {
                                            live_cfg.auto_lock_minutes = draft.auto_lock_minutes;
                                            let mut persist = live_cfg.clone();
                                            persist.auto_lock_minutes = draft.auto_lock_minutes;
                                            let _ = config::save_settings(&persist);
                                        }
                                        settings_item_gap(ui);
                                        let mut boot = draft.start_on_boot;
                                        if switch_row(
                                            ui,
                                            "开机自启动",
                                            if autostart::supported() {
                                                "登录后以托盘方式启动"
                                            } else {
                                                "当前系统不支持"
                                            },
                                            &mut boot,
                                        ) && autostart::supported()
                                        {
                                            draft.start_on_boot = boot;
                                            let _ = autostart::apply(boot);
                                            live_cfg.start_on_boot = boot;
                                            let mut persist = live_cfg.clone();
                                            persist.start_on_boot = boot;
                                            let _ = config::save_settings(&persist);
                                        }
                                    }
                                    SettingsTab::Local => {
                                        settings_section(ui, "本机");
                                        settings_item_header(ui, "当前身份", "");
                                        let mut alias_buf = alias.to_string();
                                        ui.add_enabled(
                                            false,
                                            egui::TextEdit::singleline(&mut alias_buf)
                                                .desired_width(field_w),
                                        );
                                        settings_item_gap(ui);
                                        settings_item_header(
                                            ui,
                                            "数据目录",
                                            "改路径后需重启。应用配置仍在系统用户目录。",
                                        );
                                        if settings_text_field(
                                            ui,
                                            &mut draft.data_dir,
                                            "数据目录",
                                            field_w,
                                        ) {
                                            // 仅写入 draft，保存走底栏/关闭逻辑；此处即时落盘关键字段
                                        }
                                        settings_item_gap(ui);
                                        settings_item_header(
                                            ui,
                                            "修改登录密码",
                                            "请在身份菜单「重置密码」中更换。",
                                        );
                                    }
                                    SettingsTab::Care => {
                                        settings_section(ui, "关怀与健康");
                                        ui.label(
                                            RichText::new(
                                                "经期与体检图层开关；关闭后标记与日期仍保留。",
                                            )
                                            .size(12.5)
                                            .color(theme::shell_women_pill_fg()),
                                        );
                                        ui.add_space(12.0);
                                        gender_private_view::show_care_health_settings(
                                            ui,
                                            svc,
                                            status_line,
                                        );
                                    }
                                    SettingsTab::Net => {
                                        settings_section(ui, "网络");
                                        settings_item_header(
                                            ui,
                                            "节点角色",
                                            "主机每 10s 广播并受理登记；从机不广播，发现主机后主动连接。更改后需重启。",
                                        );
                                        egui::ComboBox::from_id_source("settings_page_node_role")
                                            .selected_text(draft.node_role.label())
                                            .width(field_w.min(220.0))
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(
                                                    &mut draft.node_role,
                                                    NodeRole::Master,
                                                    "主机",
                                                );
                                                ui.selectable_value(
                                                    &mut draft.node_role,
                                                    NodeRole::Slave,
                                                    "从机",
                                                );
                                            });
                                        if draft.node_role != live_cfg.node_role {
                                            live_cfg.node_role = draft.node_role;
                                            let mut persist = live_cfg.clone();
                                            persist.node_role = draft.node_role;
                                            let _ = config::save_settings(&persist);
                                            ui.add_space(4.0);
                                            ui.label(
                                                RichText::new("已保存。重启后按新角色生效。")
                                                    .size(12.0)
                                                    .color(theme::warn()),
                                            );
                                        }
                                        settings_item_gap(ui);
                                        if switch_row(
                                            ui,
                                            "局域网自动发现",
                                            "同网段自动寻找主机",
                                            &mut draft.lan_discovery,
                                        ) {
                                            live_cfg.lan_discovery = draft.lan_discovery;
                                            let mut persist = live_cfg.clone();
                                            persist.lan_discovery = draft.lan_discovery;
                                            let _ = config::save_settings(&persist);
                                        }
                                        ui.add_enabled_ui(draft.node_role.is_master(), |ui| {
                                            if switch_row(
                                                ui,
                                                "显示本节点",
                                                "显示=对外广播，可被同网段发现；隐藏=不广播",
                                                &mut draft.node_visible,
                                            ) {
                                                live_cfg.node_visible = draft.node_visible;
                                                let mut persist = live_cfg.clone();
                                                persist.node_visible = draft.node_visible;
                                                let _ = config::save_settings(&persist);
                                            }
                                        });
                                        if !draft.node_role.is_master() {
                                            ui.label(
                                                RichText::new("当前为从机，不会对外广播。")
                                                    .size(12.0)
                                                    .color(theme::text_muted()),
                                            );
                                            ui.add_space(8.0);
                                        }
                                        settings_item_header(ui, "监听端口", "更改后需重启。");
                                        let mut port = draft.listen_port.to_string();
                                        if settings_text_field(ui, &mut port, "例如 7000", field_w) {
                                            if let Ok(p) = port.parse::<u16>() {
                                                if p > 0 {
                                                    draft.listen_port = p;
                                                }
                                            }
                                        }
                                        settings_item_gap(ui);
                                        settings_item_header(
                                            ui,
                                            "集群盐",
                                            "同集群各机必须相同；与登录密码无关。修改后需重启。",
                                        );
                                        let _ = settings_text_field(
                                            ui,
                                            &mut draft.cluster_salt_hex,
                                            "各机填写相同内容",
                                            field_w,
                                        );
                                        settings_item_gap(ui);
                                        settings_item_header(ui, "手填对端", "每行一个地址");
                                        let mut peers = draft.peers.join("\n");
                                        if ui
                                            .add(
                                                egui::TextEdit::multiline(&mut peers)
                                                    .desired_width(field_w)
                                                    .desired_rows(3),
                                            )
                                            .changed()
                                        {
                                            draft.peers = peers
                                                .lines()
                                                .map(|s| s.trim().to_string())
                                                .filter(|s| !s.is_empty())
                                                .collect();
                                        }
                                    }
                                    SettingsTab::Backup => {
                                        settings_section(ui, "备份");
                                        if switch_row(
                                            ui,
                                            "推送私人密文备份",
                                            "保存私密条目时推到可托管节点",
                                            &mut draft.backup_enabled,
                                        ) {
                                            live_cfg.backup_enabled = draft.backup_enabled;
                                            let mut persist = live_cfg.clone();
                                            persist.backup_enabled = draft.backup_enabled;
                                            let _ = config::save_settings(&persist);
                                        }
                                        if switch_row(
                                            ui,
                                            "受理他节点托管",
                                            "本机作为主机时收密文",
                                            &mut draft.accept_foreign_backup,
                                        ) {
                                            live_cfg.accept_foreign_backup =
                                                draft.accept_foreign_backup;
                                            let mut persist = live_cfg.clone();
                                            persist.accept_foreign_backup =
                                                draft.accept_foreign_backup;
                                            let _ = config::save_settings(&persist);
                                        }
                                        ui.add_space(8.0);
                                        ui.label(
                                            RichText::new("导出加密备份请用身份菜单「导出」。")
                                                .size(12.5)
                                                .color(theme::text_muted()),
                                        );
                                    }
                                }
                                ui.add_space(16.0);
                            });
                    });
            },
        );
    });
    let _ = Sense::hover();
}
