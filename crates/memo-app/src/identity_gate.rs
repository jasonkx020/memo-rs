//! 启动身份门：选择/生成/导入密钥对（macOS 风格设置助手）。

use crate::app_icon;
use crate::save_dialog;
use crate::theme;
use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Vec2};
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
    /// 用户主动退出后为 true：即使只有一个身份也不自动进入
    pub skip_auto_unlock: bool,
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
            skip_auto_unlock: false,
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
    /// 后台打开身份；若带 export，先写出 .memokey 再开库
    Unlock {
        cfg: Config,
        identity: IdentityKeys,
        export_path: Option<String>,
        export_pass: String,
    },
}

fn paint_flat_bg(ui: &egui::Ui) {
    let rect = ui.ctx().screen_rect();
    ui.painter().rect_filled(rect, 0.0, theme::mac_bg_top());
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(12.0)
            .color(theme::mac_text_secondary())
            .strong(),
    );
    ui.add_space(4.0);
}

fn mac_status(ui: &mut egui::Ui, status: &str, busy: bool) {
    if status.is_empty() {
        return;
    }
    ui.add_space(10.0);
    ui.label(
        RichText::new(status)
            .size(12.5)
            .color(if busy {
                theme::mac_text_secondary()
            } else {
                theme::mac_red()
            }),
    );
}

/// 路径输入 + 固定宽按钮，避免 horizontal 互相抢宽溢出。
fn path_picker_row(ui: &mut egui::Ui, path: &mut String, hint: &str, btn: &str) -> bool {
    const BTN_W: f32 = 88.0;
    const GAP: f32 = 8.0;
    let mut clicked = false;
    ui.horizontal(|ui| {
        let path_w = (ui.available_width() - BTN_W - GAP).max(80.0);
        ui.add(
            egui::TextEdit::singleline(path)
                .desired_width(path_w)
                .hint_text(theme::hint(hint))
                .margin(egui::Margin::symmetric(10.0, 8.0)),
        );
        ui.add_space(GAP);
        if ui
            .add_sized(
                [BTN_W, 32.0],
                egui::Button::new(RichText::new(btn).size(13.0).color(theme::mac_text()))
                    .fill(theme::mac_fill())
                    .rounding(egui::Rounding::same(theme::MAC_ROUND_CTRL)),
            )
            .clicked()
        {
            clicked = true;
        }
    });
    clicked
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
        .frame(egui::Frame::none().fill(theme::mac_bg_top()))
        .show(ui_ctx, |ui| {
            paint_flat_bg(ui);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(28.0);
                    ui.vertical_centered(|ui| {
                        // 单列内容，无浮层卡片
                        ui.set_max_width(400.0);
                        ui.set_min_width(ui.available_width().min(400.0));

                        app_icon::show(ui, 56.0);
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(theme::APP_NAME)
                                .size(22.0)
                                .strong()
                                .color(theme::mac_text()),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("每个身份独立数据 · 退出后可切换")
                                .size(13.0)
                                .color(theme::mac_text_secondary()),
                        );
                        ui.add_space(20.0);

                        // 左对齐表单列
                        ui.with_layout(Layout::top_down(Align::Min), |ui| {
                            ui.set_max_width(400.0);
                            ui.set_width(ui.available_width().min(400.0));

                            if st.legacy_wipe_confirm {
                                draw_legacy(ui, cfg, st);
                            } else if st.show_init || st.pending_identity.is_some() {
                                if st.pending_identity.is_none() {
                                    if let Some(a) = draw_init_create(ui, cfg, st) {
                                        action = a;
                                    }
                                } else if let Some(a) = draw_init_export(ui, cfg, st) {
                                    action = a;
                                }
                                ui.add_space(10.0);
                                ui.vertical_centered(|ui| {
                                    if ui
                                        .link(
                                            RichText::new("选择已有身份")
                                                .size(13.0)
                                                .color(theme::mac_blue()),
                                        )
                                        .clicked()
                                    {
                                        st.show_init = false;
                                        st.pending_identity = None;
                                        st.refresh_list(cfg);
                                    }
                                });
                            } else {
                                draw_select(ui, cfg, st, &mut action);
                            }

                            mac_status(ui, &st.status, st.busy);

                            ui.add_space(20.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(theme::APP_FILE_VERSION)
                                        .size(11.5)
                                        .color(theme::mac_text_tertiary()),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .link(
                                            RichText::new("使用说明")
                                                .size(12.5)
                                                .color(theme::mac_blue()),
                                        )
                                        .clicked()
                                    {
                                        show_help = true;
                                    }
                                });
                            });
                            ui.add_space(24.0);
                        });
                    });
                });
        });

    if show_help {
        GateAction::ShowHelp
    } else {
        action
    }
}

fn draw_legacy(ui: &mut egui::Ui, cfg: &mut Config, st: &mut IdentityGateState) {
    ui.label(
        RichText::new("需要重新初始化")
            .size(17.0)
            .strong()
            .color(theme::mac_text()),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new("检测到旧版数据目录，与当前身份模型不兼容。请先自行备份，再清空并继续。")
            .size(13.0)
            .color(theme::mac_text_secondary()),
    );
    ui.add_space(18.0);
    if theme::mac_primary_button(ui, "我已备份，清空并继续", !st.busy).clicked() {
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
}

fn draw_init_create(
    ui: &mut egui::Ui,
    cfg: &mut Config,
    st: &mut IdentityGateState,
) -> Option<GateAction> {
    ui.label(
        RichText::new("创建身份")
            .size(17.0)
            .strong()
            .color(theme::mac_text()),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(if st.init_mode_import {
            "从备份导入。名称取自密钥包；已有备份无需再导出。"
        } else {
            "生成新密钥对。新身份须导出备份后再进入。"
        })
        .size(13.0)
        .color(theme::mac_text_secondary()),
    );
    ui.add_space(14.0);

    theme::mac_segmented(ui, "生成新密钥", "导入备份", &mut st.init_mode_import);
    ui.add_space(16.0);

    if !st.init_mode_import {
        field_label(ui, "显示别名");
        ui.add(
            egui::TextEdit::singleline(&mut st.init_alias)
                .desired_width(ui.available_width())
                .hint_text(theme::hint("例如：张三"))
                .margin(egui::Margin::symmetric(12.0, 8.0)),
        );
        ui.add_space(12.0);
    }

    #[cfg(windows)]
    {
        field_label(ui, "数据盘");
        egui::ComboBox::from_id_source("mac_drive")
            .width(ui.available_width())
            .selected_text(st.init_drive.as_str())
            .show_ui(ui, |ui| {
                for d in config::list_drives() {
                    ui.selectable_value(&mut st.init_drive, d.clone(), d);
                }
            });
        ui.add_space(12.0);
    }

    if st.init_mode_import {
        field_label(ui, "密钥文件 (.memokey)");
        if path_picker_row(
            ui,
            &mut st.init_import_path,
            "选择 .memokey 文件…",
            "选择…",
        ) {
            if let Some(p) = save_dialog::open_memokey_dialog() {
                st.init_import_path = p.to_string_lossy().into_owned();
                st.status.clear();
            }
        }
        if let Some(name) = IdentityKeys::peek_alias(PathBuf::from(st.init_import_path.trim()).as_path())
        {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("将使用包内名称：{name}"))
                    .size(12.5)
                    .color(theme::mac_text_secondary()),
            );
        }
        ui.add_space(10.0);
        field_label(ui, "保险口令（可选，明文便于核对）");
        ui.add(
            egui::TextEdit::singleline(&mut st.init_import_pass)
                .desired_width(ui.available_width())
                .hint_text(theme::hint("若导出时设置过请填写；可留空")),
        );
        ui.label(
            RichText::new("明文显示便于确认；公共场合请勿久留屏幕。")
                .size(11.5)
                .color(theme::mac_text_secondary()),
        );
        ui.add_space(8.0);
    }

    ui.add_space(8.0);
    let btn = if st.init_mode_import {
        "导入并进入"
    } else {
        "继续"
    };
    if theme::mac_primary_button(ui, btn, !st.busy).clicked() {
        if st.init_mode_import {
            let path = st.init_import_path.trim();
            if path.is_empty() {
                st.status = "请选择密钥文件".into();
                return None;
            }
            match IdentityKeys::import_file(PathBuf::from(path).as_path(), &st.init_import_pass) {
                Ok(id) => {
                    cfg.data_dir = config::memo_data_path(&st.init_drive, &cfg.node_id)
                        .to_string_lossy()
                        .into_owned();
                    let _ = config::save_settings(cfg);
                    st.busy = true;
                    st.status = format!("正在以「{}」打开…", id.alias);
                    return Some(GateAction::Unlock {
                        cfg: cfg.clone(),
                        identity: id,
                        export_path: None,
                        export_pass: String::new(),
                    });
                }
                Err(e) => st.status = e.to_string(),
            }
        } else {
            let alias = st.init_alias.trim();
            if alias.is_empty() {
                st.status = "请填写显示别名".into();
            } else {
                let id = IdentityKeys::generate(alias);
                cfg.data_dir = config::memo_data_path(&st.init_drive, &cfg.node_id)
                    .to_string_lossy()
                    .into_owned();
                let _ = config::save_settings(cfg);
                st.pending_identity = Some(id);
                st.status = String::new();
            }
        }
    }
    None
}

fn draw_init_export(
    ui: &mut egui::Ui,
    cfg: &Config,
    st: &mut IdentityGateState,
) -> Option<GateAction> {
    ui.label(
        RichText::new("备份密钥（必做）")
            .size(17.0)
            .strong()
            .color(theme::mac_text()),
    );
    ui.add_space(4.0);
    let name = st
        .pending_identity
        .as_ref()
        .map(|id| id.alias.as_str())
        .unwrap_or("");
    ui.label(
        RichText::new(format!(
            "新身份「{name}」须先导出密钥包到安全位置。丢失后无法恢复私人数据。"
        ))
        .size(13.0)
        .color(theme::mac_text_secondary()),
    );
    ui.add_space(16.0);

    field_label(ui, "导出路径");
    if path_picker_row(
        ui,
        &mut st.init_export_path,
        "选择导出位置…",
        "选择位置",
    ) {
        let default_name = default_memokey_name(st);
        if let Some(p) = save_dialog::save_memokey_dialog(&default_name) {
            st.init_export_path = p.to_string_lossy().into_owned();
            st.status.clear();
        }
    }
    ui.add_space(10.0);
    field_label(ui, "导出保险口令（可选，明文便于核对）");
    ui.add(
        egui::TextEdit::singleline(&mut st.init_export_pass)
            .desired_width(ui.available_width())
            .hint_text(theme::hint("可留空；建议设置并自行抄写保存")),
    );
    ui.label(
        RichText::new("明文显示便于核对，无需二次输入；公共场合请勿久留屏幕。")
            .size(11.5)
            .color(theme::mac_text_secondary()),
    );
    ui.add_space(16.0);

    let label = if st.busy {
        "正在打开…"
    } else if st.init_export_path.trim().is_empty() {
        "选择位置并进入"
    } else {
        "导出并进入"
    };
    if theme::mac_primary_button(ui, label, !st.busy).clicked() {
        let mut export_path = st.init_export_path.trim().to_string();
        if export_path.is_empty() {
            let default_name = default_memokey_name(st);
            match save_dialog::save_memokey_dialog(&default_name) {
                Some(p) => {
                    export_path = p.to_string_lossy().into_owned();
                    st.init_export_path = export_path.clone();
                }
                None => {
                    st.status = "请选择导出位置后再继续".into();
                    return None;
                }
            }
        }
        if let Some(id) = st.pending_identity.clone() {
            let p = PathBuf::from(&export_path);
            if p.is_dir() {
                st.status = "请选择具体文件名，而不是文件夹".into();
            } else {
                st.busy = true;
                st.status = "正在导出并打开…".into();
                return Some(GateAction::Unlock {
                    cfg: cfg.clone(),
                    identity: id,
                    export_path: Some(export_path),
                    export_pass: st.init_export_pass.clone(),
                });
            }
        }
    }
    None
}

fn default_memokey_name(st: &IdentityGateState) -> String {
    let alias = st
        .pending_identity
        .as_ref()
        .map(|id| id.alias.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("memo");
    let safe: String = alias
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!("{safe}.memokey")
}

fn draw_select(
    ui: &mut egui::Ui,
    cfg: &Config,
    st: &mut IdentityGateState,
    action: &mut GateAction,
) {
    // 冷启动仅一个身份：自动进入（退出后再进则 skip_auto_unlock 阻止）
    if !st.busy && !st.skip_auto_unlock && st.identities.len() == 1 {
        let fp = st.identities[0].fingerprint.clone();
        st.selected = Some(fp.clone());
        match IdentityKeys::load(PathBuf::from(&cfg.data_dir).as_path(), &fp) {
            Ok(id) => {
                st.busy = true;
                st.status = "正在打开…".into();
                *action = GateAction::Unlock {
                    cfg: cfg.clone(),
                    identity: id,
                    export_path: None,
                    export_pass: String::new(),
                };
            }
            Err(e) => {
                st.status = e.to_string();
                st.skip_auto_unlock = true;
            }
        }
    }

    ui.label(
        RichText::new("选择身份")
            .size(17.0)
            .strong()
            .color(theme::mac_text()),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new("每个身份独立数据。选定后进入；退出可再切换其他身份。")
            .size(13.0)
            .color(theme::mac_text_secondary()),
    );
    ui.add_space(12.0);

    egui::ScrollArea::vertical()
        .max_height(220.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if st.identities.is_empty() {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("暂无已保存的身份").color(theme::mac_text_tertiary()),
                    );
                });
                ui.add_space(16.0);
                return;
            }
            for m in st.identities.iter() {
                let sel = st.selected.as_deref() == Some(m.fingerprint.as_str());
                let fill = if sel {
                    Color32::from_rgb(0xE5, 0xF1, 0xFF)
                } else {
                    theme::mac_fill()
                };
                let stroke = if sel {
                    egui::Stroke::new(1.0, theme::mac_blue())
                } else {
                    egui::Stroke::new(0.5, theme::mac_separator())
                };
                egui::Frame::none()
                    .fill(fill)
                    .stroke(stroke)
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                    .show(ui, |ui| {
                        let resp = ui.allocate_response(
                            Vec2::new(ui.available_width(), 36.0),
                            Sense::click(),
                        );
                        let painter = ui.painter();
                        let r = resp.rect;
                        painter.circle_filled(
                            egui::pos2(r.left() + 10.0, r.center().y),
                            6.0,
                            if sel {
                                theme::mac_blue()
                            } else {
                                theme::mac_separator()
                            },
                        );
                        painter.text(
                            egui::pos2(r.left() + 28.0, r.center().y - 7.0),
                            egui::Align2::LEFT_CENTER,
                            if m.alias.is_empty() {
                                "未命名"
                            } else {
                                m.alias.as_str()
                            },
                            egui::FontId::proportional(14.5),
                            theme::mac_text(),
                        );
                        painter.text(
                            egui::pos2(r.left() + 28.0, r.center().y + 9.0),
                            egui::Align2::LEFT_CENTER,
                            IdentityKeys::short_fp(&m.fingerprint),
                            egui::FontId::proportional(11.5),
                            theme::mac_text_tertiary(),
                        );
                        if resp.clicked() {
                            st.selected = Some(m.fingerprint.clone());
                        }
                    });
                ui.add_space(6.0);
            }
        });

    ui.add_space(14.0);
    let can = st.selected.is_some() && !st.busy;
    if theme::mac_primary_button(ui, if st.busy { "打开中…" } else { "继续" }, can).clicked() {
        if let Some(fp) = st.selected.clone() {
            match IdentityKeys::load(PathBuf::from(&cfg.data_dir).as_path(), &fp) {
                Ok(id) => {
                    st.busy = true;
                    st.status = "正在打开…".into();
                    *action = GateAction::Unlock {
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
    if theme::mac_secondary_button(ui, "生成或导入新身份…").clicked() {
        st.show_init = true;
        st.status.clear();
    }
}
