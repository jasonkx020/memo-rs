//! 应用品牌图标（PNG）加载与绘制。

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Vec2};
use image::imageops::FilterType;

pub const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app_icon.png");

fn load_rgba() -> image::RgbaImage {
    image::load_from_memory(APP_ICON_PNG)
        .expect("app_icon.png")
        .into_rgba8()
}

fn color_image_from_rgba(img: &image::RgbaImage) -> ColorImage {
    let size = [img.width() as usize, img.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, img.as_raw())
}

/// 窗口 / 任务栏图标（egui IconData）。
pub fn window_icon() -> egui::IconData {
    let src = load_rgba();
    // 任务栏用 256，边角经 Lanczos 抗锯齿
    let img = image::imageops::resize(&src, 256, 256, FilterType::Lanczos3);
    let color = color_image_from_rgba(&img);
    egui::IconData {
        rgba: color.as_raw().to_vec(),
        width: color.width() as u32,
        height: color.height() as u32,
    }
}

fn texture_for_physical(ctx: &egui::Context, phys: u32) -> TextureHandle {
    let src = load_rgba();
    let img = if src.width() == phys && src.height() == phys {
        src
    } else {
        image::imageops::resize(&src, phys, phys, FilterType::Lanczos3)
    };
    // 已按物理像素生成，1:1 贴图用 Nearest，避免 GPU 二次缩放出锯齿
    ctx.load_texture(
        format!("app_logo_px_{phys}"),
        color_image_from_rgba(&img),
        TextureOptions::NEAREST,
    )
}

/// 绘制方形品牌图（身份门 / 关于 / 顶栏）。
pub fn show(ui: &mut egui::Ui, size: f32) {
    let ppp = ui.ctx().pixels_per_point().max(1.0);
    let logical = size.max(8.0);
    // 对齐物理像素，消除半像素采样毛边
    let phys = (logical * ppp).round().clamp(16.0, 1024.0) as u32;
    let tex = texture_for_physical(ui.ctx(), phys);
    let draw = phys as f32 / ppp;
    ui.add(egui::Image::new(egui::load::SizedTexture {
        id: tex.id(),
        size: Vec2::splat(draw),
    }));
}
