//! 为 egui 注入系统 CJK 字体，避免中文显示为方框。

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
use std::fs;
use std::path::PathBuf;

fn candidate_fonts() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(windows)]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let fonts = PathBuf::from(windir).join("Fonts");
        // 优先单文件 TTF（egui 对 TTC 字形度量不稳定，易导致中文输入框异常）
        for name in [
            "simhei.ttf",
            "simkai.ttf",
            "msyh.ttf",
            "msyhbd.ttf",
            "simsun.ttc",
            "msyh.ttc",
            "msyhbd.ttc",
        ] {
            paths.push(fonts.join(name));
        }
    }

    #[cfg(target_os = "macos")]
    {
        for p in [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
        ] {
            paths.push(PathBuf::from(p));
        }
    }

    #[cfg(target_os = "linux")]
    {
        for p in [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ] {
            paths.push(PathBuf::from(p));
        }
    }

    paths
}

pub fn configure_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut loaded = false;

    for path in candidate_fonts() {
        if !path.exists() {
            continue;
        }
        if let Ok(data) = fs::read(&path) {
            let font_data = FontData::from_owned(data);
            fonts.font_data.insert("cjk".to_owned(), font_data);
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".to_owned());
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .insert(0, "cjk".to_owned());
            loaded = true;
            break;
        }
    }

    let _ = loaded;
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(22.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();
    ctx.set_style(style);
}
