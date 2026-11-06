//! 打赏二维码（JPEG）加载与绘制。

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Vec2};

const TIP_CODE_JPG: &[u8] = include_bytes!("../assets/tip_code.jpg");

fn rgba_image() -> ColorImage {
    let img = image::load_from_memory(TIP_CODE_JPG)
        .expect("tip_code.jpg")
        .into_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, img.as_raw())
}

fn texture(ctx: &egui::Context) -> TextureHandle {
    ctx.load_texture("tip_code", rgba_image(), TextureOptions::LINEAR)
}

/// 绘制打赏码；`size` 为显示边长（建议 ≥180，便于手机扫）。
pub fn show(ui: &mut egui::Ui, size: f32) {
    let tex = texture(ui.ctx());
    let side = size.max(64.0);
    ui.add(egui::Image::new(egui::load::SizedTexture {
        id: tex.id(),
        size: Vec2::splat(side),
    }));
}
