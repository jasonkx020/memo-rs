//! Personnel management dialog.

use eframe::egui::{self, RichText};
use memo_core::MemoService;

use crate::theme;

#[derive(Debug, Clone, Default)]
pub struct PeopleUi {
    pub show: bool,
    pub new_name: String,
    pub new_pw: String,
    pub new_pw2: String,
    pub reset_id: Option<String>,
    pub reset_pw: String,
    pub reset_pw2: String,
    pub status: String,
    /// 请求打开某人员的变更历史
    open_history_id: Option<String>,
}

impl PeopleUi {
    pub fn take_history_id(&mut self) -> Option<String> {
        self.open_history_id.take()
    }
}

/// Draw people management window. Returns true if data changed.
pub fn show_window(
    ctx: &egui::Context,
    ui_state: &mut PeopleUi,
    svc: &MemoService,
) -> bool {
    if !ui_state.show {
        return false;
    }
    let mut changed = false;
    let mut open = ui_state.show;
    egui::Window::new("人员管理")
        .collapsible(false)
        .resizable(true)
        .default_size([440.0, 420.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(theme::muted_label("主密码已解锁即可管理；每人有独立登录密码。").small());
            ui.add_space(8.0);

            let persons = svc.list_persons();
            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                if persons.is_empty() {
                    ui.label(theme::muted_label("暂无人员"));
                }
                for p in &persons {
                    ui.horizontal(|ui| {
                        let st = if p.disabled { "已禁用" } else { "启用" };
                        ui.label(
                            RichText::new(format!("{}  ({})", p.name, st)).color(theme::TEXT),
                        );
                        if ui.small_button("改密").clicked() {
                            ui_state.reset_id = Some(p.id.clone());
                            ui_state.reset_pw.clear();
                            ui_state.reset_pw2.clear();
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
                    });
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.label(RichText::new("新增人员").strong().color(theme::TEXT));
            ui.horizontal(|ui| {
                ui.label("姓名");
                ui.add(
                    egui::TextEdit::singleline(&mut ui_state.new_name)
                        .desired_width(160.0)
                        .hint_text("姓名"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("密码");
                theme::password_field(ui, &mut ui_state.new_pw, "人员密码", 160.0, 28.0);
            });
            ui.horizontal(|ui| {
                ui.label("确认");
                theme::password_field(ui, &mut ui_state.new_pw2, "再次输入", 160.0, 28.0);
            });
            if ui.button("添加").clicked() {
                if ui_state.new_pw != ui_state.new_pw2 {
                    ui_state.status = "两次密码不一致".into();
                } else {
                    match svc.add_person(&ui_state.new_name, &ui_state.new_pw) {
                        Ok(_) => {
                            ui_state.new_name.clear();
                            ui_state.new_pw.clear();
                            ui_state.new_pw2.clear();
                            ui_state.status = "已添加人员".into();
                            changed = true;
                        }
                        Err(e) => ui_state.status = e.to_string(),
                    }
                }
            }

            if let Some(id) = ui_state.reset_id.clone() {
                ui.add_space(10.0);
                ui.separator();
                let name = persons
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.as_str())
                    .unwrap_or("?");
                ui.label(RichText::new(format!("重设密码 · {name}")).strong());
                theme::password_field(ui, &mut ui_state.reset_pw, "新密码", 200.0, 28.0);
                theme::password_field(ui, &mut ui_state.reset_pw2, "确认新密码", 200.0, 28.0);
                ui.horizontal(|ui| {
                    if ui.button("保存密码").clicked() {
                        if ui_state.reset_pw != ui_state.reset_pw2 {
                            ui_state.status = "两次密码不一致".into();
                        } else {
                            match svc.set_person_password(&id, &ui_state.reset_pw) {
                                Ok(()) => {
                                    ui_state.reset_id = None;
                                    ui_state.reset_pw.clear();
                                    ui_state.reset_pw2.clear();
                                    ui_state.status = "密码已更新".into();
                                    changed = true;
                                }
                                Err(e) => ui_state.status = e.to_string(),
                            }
                        }
                    }
                    if ui.button("取消").clicked() {
                        ui_state.reset_id = None;
                    }
                });
            }

            if !ui_state.status.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(&ui_state.status).small().color(theme::TEXT_MUTED));
            }
        });
    ui_state.show = open;
    changed
}
