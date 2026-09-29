//! 启动身份门：选择/生成/导入密钥对。

use crate::theme;
use eframe::egui::{self, Color32, Frame, RichText, Vec2};
use memo_core::config::{self, Config};
use memo_core::identity_keys::{self, IdentityKeys, IdentityMeta};
use std::path::PathBuf;

pub struct IdentityGateState {
    pub identities: Vec<IdentityMeta>,
    pub selected: Option<String>,
    pub status: String,
    pub busy: bool,
    pub legacy_wipe_confirm: bool,
    pub show_init: bool,
    pub init_alias: String,
    pub init_drive: String,
    pub init_import_path: String,
    pub init_import_pass: String,
    pub init_export_path: String,
    pub init_export_pass: String,
    pub init_mode_import: bool,
    pub pending_identity: Option<IdentityKeys>,
}

impl IdentityGateState {
    pub fn from_cfg(cfg: &Config) -> Self {
        let data = PathBuf::from(&cfg.data_dir);
        let legacy = identity_keys::looks_like_legacy_data(&data);
        let identities = IdentityKeys::list(&data).unwrap_or_default();
        let show_init = identities.is_empty() && !legacy;
        Self {
            identities,
            selected: None,
            status: if legacy {
                "检测到旧版数据，与当前版本不兼容。请备份后确认清空并重新初始化。".into()
            } else {
                String::new()
            },
            busy: false,
            legacy_wipe_confirm: legacy,
            show_init,
            init_alias: String::new(),
            init_drive: config::default_data_drive(),
            init_import_path: String::new(),
            init_import_pass: String::new(),
            init_export_path: String::new(),
            init_export_pass: String::new(),
            init_mode_import: false,
            pending_identity: None,
        }
    }

    pub fn refresh_list(&mut self, cfg: &Config) {
        let data = PathBuf::from(&cfg.data_dir);
        self.identities = IdentityKeys::list(&data).unwrap_or_default();
    }
}

pub enum GateAction {
    None,
    ShowHelp,
    /// 后台打开身份；若带 export，先写出 .memokey 再开库（勿在 UI 线程做文件 IO）
    Unlock {
        cfg: Config,
        identity: IdentityKeys,
        export_path: Option<String>,
        export_pass: String,
    },
}

/// 绘制身份门；若需后台解锁返回 `Unlock`。
pub fn show(
    ui_ctx: &egui::Context,
    cfg: &mut Config,
    st: &mut IdentityGateState,
) -> GateAction {
    let mut action = GateAction::None;
    let mut show_help = false;

    egui::CentralPanel::default()
        .frame(Frame::none().fill(theme::BG))
        .show(ui_ctx, |ui| {
            let avail = ui.available_size();
            let card_w = 460.0_f32;
            let card_h = 520.0_f32;
            let left = ((avail.x - card_w) * 0.5).max(16.0);
            let top = ((avail.y - card_h) * 0.28).max(24.0);
            let card_rect = egui::Rect::from_min_size(
                egui::pos2(ui.min_rect().left() + left, ui.min_rect().top() + top),
                Vec2::new(card_w, card_h),
            );
            ui.allocate_ui_at_rect(card_rect, |ui| {
                theme::card_frame().show(ui, |ui| {
                    ui.set_min_width(card_w - 24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(theme::brand_title(24.0));
                        ui.label(
                            theme::muted_label("密钥对 = 身份 · 姓名仅为别名").size(13.0),
                        );
                    });
                    ui.add_space(10.0);

                    if st.legacy_wipe_confirm {
                        ui.label(
                            RichText::new("旧版数据不兼容")
                                .strong()
                                .color(theme::DANGER),
                        );
                        ui.label(
                            theme::muted_label(
                                "请先自行备份原数据目录，确认后将删除并重新初始化。",
                            )
                            .small(),
                        );
                        if theme::primary_button(ui, "我已备份，清空并重装").clicked() {
                            let dir = PathBuf::from(&cfg.data_dir);
                            match identity_keys::wipe_data_dir(&dir) {
                                Ok(()) => {
                                    st.legacy_wipe_confirm = false;
                                    st.show_init = true;
                                    st.status = "已清空，请生成或导入密钥对".into();
                                }
                                Err(e) => st.status = e.to_string(),
                            }
                        }
                    } else if st.show_init || st.pending_identity.is_some() {
                        if st.pending_identity.is_none() {
                            ui.label(RichText::new("初始化身份").strong().color(theme::TEXT));
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(!st.init_mode_import, "生成新密钥对")
                                    .clicked()
                                {
                                    st.init_mode_import = false;
                                }
                                if ui
                                    .selectable_label(st.init_mode_import, "导入 .memokey")
                                    .clicked()
                                {
                                    st.init_mode_import = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("别名");
                                ui.add(
                                    egui::TextEdit::singleline(&mut st.init_alias)
                                        .desired_width(220.0),
                                );
                            });
                            #[cfg(windows)]
                            {
                                ui.horizontal(|ui| {
                                    ui.label("数据盘");
                                    egui::ComboBox::from_id_source("drive")
                                        .selected_text(st.init_drive.as_str())
                                        .show_ui(ui, |ui| {
                                            for d in config::list_drives() {
                                                ui.selectable_value(
                                                    &mut st.init_drive,
                                                    d.clone(),
                                                    d,
                                                );
                                            }
                                        });
                                });
                            }
                            if st.init_mode_import {
                                ui.horizontal(|ui| {
                                    ui.label("密钥文件");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut st.init_import_path)
                                            .desired_width(240.0),
                                    );
                                });
                                theme::password_field(
                                    ui,
                                    &mut st.init_import_pass,
                                    "保险口令（可空）",
                                    280.0,
                                    32.0,
                                );
                            }
                            if theme::primary_button(ui, "下一步：导出备份").clicked() {
                                let alias = st.init_alias.trim();
                                if alias.is_empty() {
                                    st.status = "请填写别名".into();
                                } else {
                                    let id_res = if st.init_mode_import {
                                        IdentityKeys::import_file(
                                            PathBuf::from(st.init_import_path.trim()).as_path(),
                                            &st.init_import_pass,
                                        )
                                        .map(|mut id| {
                                            if id.alias.is_empty() {
                                                id.alias = alias.to_string();
                                            }
                                            id
                                        })
                                    } else {
                                        Ok(IdentityKeys::generate(alias))
                                    };
                                    match id_res {
                                        Ok(id) => {
                                            cfg.data_dir = config::memo_data_path(
                                                &st.init_drive,
                                                &cfg.node_id,
                                            )
                                            .to_string_lossy()
                                            .into_owned();
                                            let _ = config::save_settings(cfg);
                                            st.pending_identity = Some(id);
                                            st.status =
                                                "请导出密钥到安全位置（可用云盘目录）".into();
                                        }
                                        Err(e) => st.status = e.to_string(),
                                    }
                                }
                            }
                        } else {
                            ui.label(
                                RichText::new("强制导出密钥备份")
                                    .strong()
                                    .color(theme::TEXT),
                            );
                            ui.label(
                                theme::muted_label(
                                    "丢失密钥文件将无法恢复私人数据。可导出到云盘同步文件夹。",
                                )
                                .small(),
                            );
                            ui.horizontal(|ui| {
                                ui.label("导出路径");
                                ui.add(
                                    egui::TextEdit::singleline(&mut st.init_export_path)
                                        .desired_width(260.0)
                                        .hint_text(r"D:\OneDrive\memo.memokey"),
                                );
                            });
                            theme::password_field(
                                ui,
                                &mut st.init_export_pass,
                                "导出保险口令（可空）",
                                280.0,
                                32.0,
                            );
                            if theme::primary_button(ui, "导出并进入").clicked() && !st.busy {
                                if st.init_export_path.trim().is_empty() {
                                    st.status = "请指定导出路径".into();
                                } else if let Some(id) = st.pending_identity.clone() {
                                    let export_path = st.init_export_path.trim().to_string();
                                    let p = PathBuf::from(&export_path);
                                    if p.is_dir() {
                                        st.status = "导出路径必须是文件（例如 D:\\backup\\memo.memokey）".into();
                                    } else {
                                        st.busy = true;
                                        st.status = "正在导出并打开…".into();
                                        action = GateAction::Unlock {
                                            cfg: cfg.clone(),
                                            identity: id,
                                            export_path: Some(export_path),
                                            export_pass: st.init_export_pass.clone(),
                                        };
                                    }
                                }
                            }
                        }
                        if ui.link("返回选择已有身份").clicked() {
                            st.show_init = false;
                            st.pending_identity = None;
                            st.refresh_list(cfg);
                        }
                    } else {
                        ui.label(
                            RichText::new("选择身份密钥对")
                                .strong()
                                .color(theme::TEXT)
                                .size(18.0),
                        );
                        ui.label(
                            theme::muted_label("选定后会话锁定，中途不可切换").small(),
                        );
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                            for m in st.identities.iter() {
                                let label = format!(
                                    "{}  ({})",
                                    m.alias,
                                    IdentityKeys::short_fp(&m.fingerprint)
                                );
                                let sel =
                                    st.selected.as_deref() == Some(m.fingerprint.as_str());
                                if ui.selectable_label(sel, label).clicked() {
                                    st.selected = Some(m.fingerprint.clone());
                                }
                            }
                        });
                        ui.add_space(8.0);
                        let can = st.selected.is_some() && !st.busy;
                        if ui
                            .add_enabled(
                                can,
                                egui::Button::new(
                                    RichText::new(if st.busy { "打开中…" } else { "进入" })
                                        .color(Color32::WHITE)
                                        .strong(),
                                )
                                .fill(theme::ACCENT),
                            )
                            .clicked()
                        {
                            if let Some(fp) = st.selected.clone() {
                                match IdentityKeys::load(
                                    PathBuf::from(&cfg.data_dir).as_path(),
                                    &fp,
                                ) {
                                    Ok(id) => {
                                        st.busy = true;
                                        st.status = "正在打开…".into();
                                        action = GateAction::Unlock {
                                            cfg: cfg.clone(),
                                            identity: id,
                                            export_path: None,
                                            export_pass: String::new(),
                                        };
                                    }
                                    Err(e) => st.status = e.to_string(),
                                }
                            }
                        }
                        ui.add_space(8.0);
                        if ui.button("生成 / 导入新身份…").clicked() {
                            st.show_init = true;
                        }
                    }

                    if !st.status.is_empty() {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(st.status.as_str())
                                .color(if st.busy {
                                    theme::TEXT_MUTED
                                } else {
                                    theme::DANGER
                                })
                                .small(),
                        );
                    }
                    ui.add_space(12.0);
                    if ui
                        .link(RichText::new("使用说明").color(theme::ACCENT))
                        .clicked()
                    {
                        show_help = true;
                    }
                });
            });
        });

    if show_help {
        GateAction::ShowHelp
    } else {
        action
    }
}
