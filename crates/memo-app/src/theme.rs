#![allow(dead_code)]

//! 视觉令牌：主界面石板蓝 + 启动门 macOS 风格。

use eframe::egui::{
    self, Color32, Frame, Margin, Rounding, Stroke, Style, TextStyle, Vec2, Visuals,
};

// —— 色板（主界面） ——
pub const NAVY: Color32 = Color32::from_rgb(0x0F, 0x27, 0x44);
pub const NAVY_MID: Color32 = Color32::from_rgb(0x1B, 0x3A, 0x5C);
pub const BG: Color32 = Color32::from_rgb(0xF7, 0xF8, 0xFA);
pub const PANEL: Color32 = Color32::from_rgb(0xEE, 0xF1, 0xF5);
pub const CARD: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const BORDER: Color32 = Color32::from_rgb(0xE2, 0xE8, 0xF0);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0xCB, 0xD5, 0xE1);
pub const TEXT: Color32 = Color32::from_rgb(0x0F, 0x17, 0x2A);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x64, 0x74, 0x8B);
pub const ACCENT: Color32 = Color32::from_rgb(0x25, 0x63, 0xEB);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x1D, 0x4E, 0xD8);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0xDB, 0xEA, 0xFE);
pub const SUCCESS: Color32 = Color32::from_rgb(0x05, 0x96, 0x69);
pub const SUCCESS_SOFT: Color32 = Color32::from_rgb(0xD1, 0xFA, 0xE5);
pub const DANGER: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);
pub const DANGER_SOFT: Color32 = Color32::from_rgb(0xFE, 0xE2, 0xE2);
pub const WARN: Color32 = Color32::from_rgb(0xB4, 0x53, 0x09);

pub const ROUND_CARD: f32 = 8.0;
pub const ROUND_CTRL: f32 = 6.0;

// —— macOS 启动门色板 ——
pub const MAC_BG: Color32 = Color32::from_rgb(0xE8, 0xE8, 0xED);
pub const MAC_BG_TOP: Color32 = Color32::from_rgb(0xF5, 0xF5, 0xF7);
pub const MAC_CARD: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const MAC_SEPARATOR: Color32 = Color32::from_rgb(0xD1, 0xD1, 0xD6);
pub const MAC_FILL: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF7);
pub const MAC_FILL_HOVER: Color32 = Color32::from_rgb(0xE5, 0xE5, 0xEA);
pub const MAC_TEXT: Color32 = Color32::from_rgb(0x1D, 0x1D, 0x1F);
pub const MAC_TEXT_SECONDARY: Color32 = Color32::from_rgb(0x6E, 0x6E, 0x73);
pub const MAC_TEXT_TERTIARY: Color32 = Color32::from_rgb(0x8E, 0x8E, 0x93);
pub const MAC_BLUE: Color32 = Color32::from_rgb(0x00, 0x7A, 0xFF);
pub const MAC_BLUE_PRESSED: Color32 = Color32::from_rgb(0x00, 0x64, 0xD1);
pub const MAC_RED: Color32 = Color32::from_rgb(0xFF, 0x3B, 0x30);
pub const MAC_ROUND_SHEET: f32 = 14.0;
pub const MAC_ROUND_CTRL: f32 = 8.0;
pub const MAC_ROUND_PILL: f32 = 10.0;

pub fn apply(ctx: &egui::Context) {
    let mut visuals = Visuals::light();
    visuals.window_fill = CARD;
    visuals.panel_fill = PANEL;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = PANEL;
    visuals.code_bg_color = Color32::from_rgb(0xF1, 0xF5, 0xF9);
    visuals.override_text_color = Some(TEXT);
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARN;
    visuals.error_fg_color = DANGER;
    visuals.window_rounding = Rounding::same(ROUND_CARD);
    visuals.menu_rounding = Rounding::same(ROUND_CTRL);
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.bg_fill = CARD;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.rounding = Rounding::same(ROUND_CTRL);
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    visuals.widgets.inactive.rounding = Rounding::same(ROUND_CTRL);
    visuals.widgets.hovered.bg_fill = ACCENT_SOFT;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.hovered.rounding = Rounding::same(ROUND_CTRL);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0xBF, 0xDB, 0xFE);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, ACCENT);
    visuals.widgets.active.rounding = Rounding::same(ROUND_CTRL);
    visuals.widgets.open.bg_fill = ACCENT_SOFT;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.open.rounding = Rounding::same(ROUND_CTRL);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT_HOVER);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 7.0);
    style.spacing.window_margin = Margin::same(16.0);
    style.spacing.indent = 18.0;
    style.interaction.tooltip_delay = 0.35;
    polish_text_styles(&mut style);
    ctx.set_style(style);
}

fn polish_text_styles(style: &mut Style) {
    use egui::FontFamily;
    use egui::FontId;
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(22.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.5, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(12.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.0, FontFamily::Monospace),
    );
}

pub fn card_frame() -> Frame {
    Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .rounding(Rounding::same(ROUND_CARD))
        .inner_margin(Margin::same(16.0))
}

/// 启动门：大圆角白卡片（类似 macOS sheet）。
pub fn mac_sheet_frame() -> Frame {
    Frame::none()
        .fill(MAC_CARD)
        .stroke(Stroke::new(0.5, MAC_SEPARATOR))
        .rounding(Rounding::same(MAC_ROUND_SHEET))
        .inner_margin(Margin::symmetric(28.0, 26.0))
        .shadow(egui::epaint::Shadow {
            offset: Vec2::new(0.0, 8.0),
            blur: 24.0,
            spread: 0.0,
            color: Color32::from_black_alpha(32),
        })
}

pub fn panel_frame() -> Frame {
    Frame::none()
        .fill(PANEL)
        .inner_margin(Margin::same(12.0))
}

pub fn top_bar_frame() -> Frame {
    Frame::none()
        .fill(NAVY)
        .inner_margin(Margin::symmetric(16.0, 11.0))
}

pub fn bottom_bar_frame() -> Frame {
    Frame::none()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(Margin::symmetric(14.0, 7.0))
}

pub fn list_row_fill(selected: bool, hovered: bool) -> Color32 {
    if selected {
        ACCENT_SOFT
    } else if hovered {
        Color32::from_rgb(0xF1, 0xF5, 0xF9)
    } else {
        CARD
    }
}

pub fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).strong()).fill(ACCENT),
    )
}

/// macOS 风格主按钮（系统蓝、大圆角）；宽度不超过当前列可用宽度。
pub fn mac_primary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    let text = egui::RichText::new(label)
        .color(Color32::WHITE)
        .strong()
        .size(15.0);
    let w = ui.available_width().clamp(120.0, 400.0);
    ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .fill(MAC_BLUE)
            .rounding(Rounding::same(MAC_ROUND_PILL))
            .min_size(Vec2::new(w, 36.0)),
    )
}

/// macOS 次要按钮（浅灰底）。
pub fn mac_secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(MAC_TEXT).size(14.0))
            .fill(MAC_FILL)
            .rounding(Rounding::same(MAC_ROUND_CTRL))
            .min_size(Vec2::new(0.0, 32.0)),
    )
}

/// 两段式选择（类似 NSSegmentedControl）。
pub fn mac_segmented(ui: &mut egui::Ui, left: &str, right: &str, right_selected: &mut bool) {
    let full = ui.available_width().min(360.0);
    let h = 32.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(full, h), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        Rounding::same(MAC_ROUND_CTRL),
        MAC_FILL,
        Stroke::NONE,
    );
    let mid = rect.center().x;
    let left_rect = egui::Rect::from_min_max(rect.min, egui::pos2(mid, rect.max.y));
    let right_rect = egui::Rect::from_min_max(egui::pos2(mid, rect.min.y), rect.max);

    let paint_seg = |r: egui::Rect, selected: bool, label: &str| {
        if selected {
            painter.rect(
                r.shrink(2.0),
                Rounding::same(MAC_ROUND_CTRL - 1.0),
                MAC_CARD,
                Stroke::new(0.5, MAC_SEPARATOR),
            );
        }
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(13.0),
            if selected {
                MAC_TEXT
            } else {
                MAC_TEXT_SECONDARY
            },
        );
    };
    paint_seg(left_rect, !*right_selected, left);
    paint_seg(right_rect, *right_selected, right);

    let id = ui.id().with("mac_seg");
    let resp = ui.interact(rect, id, egui::Sense::click());
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            *right_selected = pos.x >= mid;
        }
    }
}

/// 步骤圆点指示（1-based current）。
pub fn mac_step_dots(ui: &mut egui::Ui, total: usize, current: usize) {
    ui.horizontal(|ui| {
        let w = (total as f32) * 14.0;
        ui.add_space(((ui.available_width() - w) * 0.5).max(0.0));
        for i in 1..=total {
            let active = i == current;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
            ui.painter().circle_filled(
                rect.center(),
                if active { 4.0 } else { 3.0 },
                if active { MAC_BLUE } else { MAC_SEPARATOR },
            );
            ui.add_space(6.0);
        }
    });
}

pub fn success_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).strong()).fill(SUCCESS),
    )
}

pub fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).strong()).fill(DANGER),
    )
}

pub fn ghost_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(label).color(TEXT)))
}

/// HTML 风格密码框：文字与灰色提示均垂直居中、水平靠左。
pub fn password_field(
    ui: &mut egui::Ui,
    password: &mut String,
    hint: &str,
    width: f32,
    height: f32,
) -> egui::Response {
    use egui::{Align, Align2, FontId, Sense, TextEdit};

    let size = Vec2::new(width, height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let pad_x = 12.0_f32;

    let response = ui.put(
        rect,
        TextEdit::singleline(password)
            .password(true)
            .frame(true)
            .vertical_align(Align::Center)
            .horizontal_align(Align::LEFT)
            .margin(Margin::symmetric(pad_x, 0.0))
            .min_size(size)
            .desired_width(width),
    );

    if password.is_empty() && !hint.is_empty() {
        let font = FontId::proportional(14.5);
        let galley = ui.fonts(|f| f.layout_no_wrap(hint.to_owned(), font, TEXT_MUTED));
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + pad_x, rect.top()),
            Vec2::new((rect.width() - pad_x * 2.0).max(1.0), rect.height()),
        );
        let pos = Align2::LEFT_CENTER
            .align_size_within_rect(galley.size(), text_rect)
            .min;
        ui.painter().galley(pos, galley, TEXT_MUTED);
    }

    response
}

pub const APP_NAME: &str = "分布式备忘录";
pub const APP_COPYRIGHT: &str = "Copyright (C) 2026 DistributedMemo. All rights reserved.";
pub const APP_FILE_VERSION: &str = "0.3.0.0";
pub const APP_DESCRIPTION: &str = "本地加密 · 局域网同步 · 绿色单文件分布式备忘录";

pub fn muted_label(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).color(TEXT_MUTED)
}

pub fn brand_title(size: f32) -> egui::RichText {
    egui::RichText::new("分布式备忘录")
        .strong()
        .size(size)
        .color(TEXT)
}

pub fn brand_title_on_navy(size: f32) -> egui::RichText {
    egui::RichText::new("分布式备忘录")
        .strong()
        .size(size)
        .color(Color32::WHITE)
}
