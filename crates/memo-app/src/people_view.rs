//! 任务负责人等人名目录（无密码；身份由密钥对决定）。

use eframe::egui::{self, RichText};
use memo_core::MemoService;

use crate::theme;

#[derive(Debug, Clone, Default)]
pub struct PeopleUi {
    pub show: bool,
    pub new_name: String,
    pub status: String,
    open_history_id: Option<String>,
}

impl PeopleUi {
    pub fn take_history_id(&mut self) -> Option<String> {
        self.open_history_id.take()
    }
}

/// 绘制人员目录窗口。返回是否有数据变更。
pub fn show_window(ctx: &egui::Context, ui_state: &mut PeopleUi, svc: &MemoService) -> bool {
    if !ui_state.show {
        return false;
    }
    let mut changed = false;
    let mut open = ui_state.show;
    egui::Window::new("人员目录")
        .collapsible(false)
        .resizable(true)
        .default_size([420.0, 360.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(
                theme::muted_label(
                    "此处姓名仅用于任务负责人等标记；登录身份由密钥对决定，无人员密码。",
                )
                .small(),
            );
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
                        .desired_width(200.0)
                        .hint_text("姓名"),
                );
            });
            if ui.button("添加").clicked() {
                match svc.add_person(&ui_state.new_name, "") {
                    Ok(_) => {
                        ui_state.new_name.clear();
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
                        .color(theme::TEXT_MUTED),
                );
            }
        });
    ui_state.show = open;
    changed
}
