//! 任务负责人等人名目录（无密码；身份由密钥对决定）。

use eframe::egui::{self, RichText};
use memo_core::person::Gender;
use memo_core::MemoService;

use crate::theme;

#[derive(Debug, Clone)]
pub struct PeopleUi {
    pub show: bool,
    pub new_name: String,
    pub new_gender: Gender,
    pub status: String,
    /// 正在编辑的人员 id
    edit_id: Option<String>,
    edit_name: String,
    edit_gender: Gender,
    open_history_id: Option<String>,
}

impl Default for PeopleUi {
    fn default() -> Self {
        Self {
            show: false,
            new_name: String::new(),
            new_gender: Gender::Female,
            status: String::new(),
            edit_id: None,
            edit_name: String::new(),
            edit_gender: Gender::Female,
            open_history_id: None,
        }
    }
}

impl PeopleUi {
    pub fn take_history_id(&mut self) -> Option<String> {
        self.open_history_id.take()
    }

    fn begin_edit(&mut self, id: &str, name: &str, gender: Gender) {
        self.edit_id = Some(id.to_string());
        self.edit_name = name.to_string();
        self.edit_gender = if matches!(gender, Gender::Unknown) {
            Gender::Female
        } else {
            gender
        };
        self.status.clear();
    }

    fn cancel_edit(&mut self) {
        self.edit_id = None;
        self.edit_name.clear();
    }
}

fn gender_badge(ui: &mut egui::Ui, g: Gender) {
    let (label, color) = match g {
        Gender::Male => ("男", theme::accent()),
        Gender::Female => ("女", egui::Color32::from_rgb(0xDB, 0x27, 0x77)),
        Gender::Unknown => ("?", theme::text_muted()),
    };
    egui::Frame::none()
        .fill(theme::panel())
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).small().strong().color(color));
        });
}

/// 绘制人员目录窗口。返回是否有数据变更。
pub fn show_window(ctx: &egui::Context, ui_state: &mut PeopleUi, svc: &MemoService) -> bool {
    if !ui_state.show {
        return false;
    }
    let mut changed = false;
    let mut open = ui_state.show;
    let current = svc.current_person_id();
    let modal = theme::begin_modal(ctx, "people_modal");
    theme::modal_fixed(ctx, "人员目录", [480.0, 460.0])
        .id(modal.window_id)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(
                theme::muted_label(
                    "姓名用于任务负责人；性别用于节点标签与生理期（仅女性）。身份由密钥对决定。",
                )
                .small(),
            );
            ui.add_space(8.0);

            let persons = svc.list_persons();
            let list_h = (ui.available_height() * 0.45).clamp(120.0, 280.0);
            egui::ScrollArea::vertical().max_height(list_h).show(ui, |ui| {
                if persons.is_empty() {
                    ui.label(theme::muted_label("暂无人员"));
                }
                for p in &persons {
                    let editing = ui_state.edit_id.as_deref() == Some(p.id.as_str());
                    ui.horizontal(|ui| {
                        gender_badge(ui, p.gender);
                        let cur = current.as_deref() == Some(p.id.as_str());
                        let st = if p.disabled {
                            "已禁用"
                        } else if cur {
                            "当前·启用"
                        } else {
                            "启用"
                        };
                        ui.label(
                            RichText::new(format!("{}  ({})", p.name, st)).color(theme::text()),
                        );
                        if !editing {
                            if ui.small_button("修改").clicked() {
                                ui_state.begin_edit(&p.id, &p.name, p.gender);
                            }
                            if !p.disabled
                                && !cur
                                && ui.small_button("设为当前").clicked()
                            {
                                match svc.select_person(&p.id) {
                                    Ok(()) => {
                                        ui_state.status = format!("当前人员：{}", p.name);
                                        changed = true;
                                    }
                                    Err(e) => ui_state.status = e.to_string(),
                                }
                            }
                            if ui.small_button("历史").clicked() {
                                ui_state.open_history_id = Some(p.id.clone());
                            }
                            let toggle = if p.disabled { "启用" } else { "禁用" };
                            if ui.small_button(toggle).clicked() {
                                match svc.disable_person(&p.id, !p.disabled) {
                                    Ok(()) => {
                                        ui_state.status = "已更新".into();
                                        changed = true;
                                    }
                                    Err(e) => ui_state.status = e.to_string(),
                                }
                            }
                        } else {
                            ui.label(
                                RichText::new("编辑中…")
                                    .small()
                                    .color(theme::warn()),
                            );
                        }
                    });
                }
            });

            if ui_state.edit_id.is_some() {
                ui.add_space(10.0);
                ui.separator();
                ui.label(RichText::new("修改人员").strong().color(theme::text()));
                ui.horizontal(|ui| {
                    ui.label("姓名");
                    ui.add(
                        egui::TextEdit::singleline(&mut ui_state.edit_name)
                            .desired_width(180.0)
                            .hint_text(theme::hint("姓名")),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("性别");
                    ui.selectable_value(&mut ui_state.edit_gender, Gender::Male, "男");
                    ui.selectable_value(&mut ui_state.edit_gender, Gender::Female, "女");
                });
                ui.horizontal(|ui| {
                    if theme::success_button(ui, "保存修改").clicked() {
                        if let Some(id) = ui_state.edit_id.clone() {
                            let disabled = persons
                                .iter()
                                .find(|p| p.id == id)
                                .map(|p| p.disabled)
                                .unwrap_or(false);
                            match svc.update_person(
                                &id,
                                &ui_state.edit_name,
                                ui_state.edit_gender,
                                disabled,
                            ) {
                                Ok(()) => {
                                    ui_state.status = "人员信息已保存".into();
                                    ui_state.cancel_edit();
                                    changed = true;
                                }
                                Err(e) => ui_state.status = e.to_string(),
                            }
                        }
                    }
                    if ui.button("取消").clicked() {
                        ui_state.cancel_edit();
                    }
                });
            }

            ui.add_space(10.0);
            ui.separator();
            ui.label(RichText::new("新增人员").strong().color(theme::text()));
            ui.horizontal(|ui| {
                ui.label("姓名");
                ui.add(
                    egui::TextEdit::singleline(&mut ui_state.new_name)
                        .desired_width(180.0)
                        .hint_text(theme::hint("姓名")),
                );
            });
            ui.horizontal(|ui| {
                ui.label("性别");
                ui.selectable_value(&mut ui_state.new_gender, Gender::Male, "男");
                ui.selectable_value(&mut ui_state.new_gender, Gender::Female, "女");
                ui.label(
                    theme::muted_label("生理期提醒仅对女性开放").small(),
                );
            });
            if ui.button("添加").clicked() {
                match svc.add_person_with_gender(&ui_state.new_name, ui_state.new_gender) {
                    Ok(id) => {
                        ui_state.new_name.clear();
                        if svc.current_person_id().is_none() {
                            let _ = svc.select_person(&id);
                        }
                        ui_state.status = "已添加".into();
                        changed = true;
                    }
                    Err(e) => ui_state.status = e.to_string(),
                }
            }

            if !ui_state.status.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(&ui_state.status)
                        .small()
                        .color(theme::text_muted()),
                );
            }
        });
    if modal.end(ctx, open) {
        ui_state.show = false;
        ui_state.cancel_edit();
    } else {
        ui_state.show = open;
    }
    changed
}
