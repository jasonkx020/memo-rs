//! 应用品牌图标（PNG）加载与绘制。

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Vec2};

pub const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app_icon.png");

fn rgba_image() -> ColorImage {
    let img = image::load_from_memory(APP_ICON_PNG)
        .expect("app_icon.png")
        .into_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, img.as_raw())
}

/// 窗口 / 任务栏图标（egui IconData）。
pub fn window_icon() -> egui::IconData {
    let color = rgba_image();
    egui::IconData {
        rgba: color.as_raw().to_vec(),
        width: color.width() as u32,
        height: color.height() as u32,
    }
}

fn texture(ctx: &egui::Context) -> TextureHandle {
    ctx.load_texture("app_logo", rgba_image(), TextureOptions::LINEAR)
}

/// 绘制方形品牌图（身份门 / 关于 / 顶栏）。
pub fn show(ui: &mut egui::Ui, size: f32) {
    let tex = texture(ui.ctx());
    let side = size.max(8.0);
    ui.add(egui::Image::new(egui::load::SizedTexture {
        id: tex.id(),
        size: Vec2::splat(side),
    }));
}
