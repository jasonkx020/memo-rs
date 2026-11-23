//! 设置页通用排版助手（不改视觉，仅从 lib 外提）。

use crate::theme;
use eframe::egui::{self, RichText};

pub fn settings_item_header(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(
        RichText::new(title)
            .size(13.5)
            .strong()
            .color(theme::text()),
    );
    if !description.is_empty() {
        ui.add_space(3.0);
        ui.label(
            RichText::new(description)
                .size(12.0)
                .color(theme::text_muted()),
        );
    }
    ui.add_space(6.0);
}

pub fn settings_item_gap(ui: &mut egui::Ui) {
    ui.add_space(14.0);
}

pub fn settings_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(title)
            .size(16.0)
            .strong()
            .color(theme::text()),
    );
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(10.0);
}

pub fn settings_text_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> bool {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(width)
            .hint_text(theme::hint(hint)),
    )
    .changed()
}
