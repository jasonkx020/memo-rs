//! 为 egui 注入系统 CJK / 符号字体，避免中文与 ☐✓◀ 等显示为方框。

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
use std::fs;
use std::path::PathBuf;

fn candidate_cjk_fonts() -> Vec<PathBuf> {
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

/// 符号 / emoji 回退（排在 CJK 之后），补齐 ☐✓◀⚠ 等 CJK 常缺字形。
fn candidate_symbol_fonts() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();

    #[cfg(windows)]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let fonts = PathBuf::from(windir).join("Fonts");
        for (key, name) in [
            ("segoe_emoji", "seguiemj.ttf"),
            ("segoe_symbol", "seguisym.ttf"),
        ] {
            out.push((key.to_owned(), fonts.join(name)));
        }
    }

    #[cfg(target_os = "macos")]
    {
        out.push((
            "apple_emoji".into(),
            PathBuf::from("/System/Library/Fonts/Apple Color Emoji.ttc"),
        ));
        out.push((
            "apple_symbol".into(),
            PathBuf::from("/System/Library/Fonts/Apple Symbols.ttf"),
        ));
    }

    #[cfg(target_os = "linux")]
    {
        for (key, p) in [
            (
                "noto_emoji",
                "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
            ),
            (
                "noto_symbols",
                "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            ),
        ] {
            out.push((key.into(), PathBuf::from(p)));
        }
    }

    out
}

fn push_family_font(fonts: &mut FontDefinitions, family: FontFamily, name: &str, at: usize) {
    let list = fonts.families.entry(family).or_default();
    if !list.iter().any(|n| n == name) {
        list.insert(at.min(list.len()), name.to_owned());
    }
}

pub fn configure_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    for path in candidate_cjk_fonts() {
        if !path.exists() {
            continue;
        }
        if let Ok(data) = fs::read(&path) {
            fonts
                .font_data
                .insert("cjk".to_owned(), FontData::from_owned(data));
            push_family_font(&mut fonts, FontFamily::Proportional, "cjk", 0);
            push_family_font(&mut fonts, FontFamily::Monospace, "cjk", 0);
            break;
        }
    }

    // CJK 之后插入系统符号字体，避免 ☐✓◀ 等落到 CJK 缺字方框且无法回退。
    let mut insert_at = if fonts.font_data.contains_key("cjk") {
        1
    } else {
        0
    };
    for (key, path) in candidate_symbol_fonts() {
        if !path.exists() {
            continue;
        }
        if let Ok(data) = fs::read(&path) {
            fonts
                .font_data
                .insert(key.clone(), FontData::from_owned(data));
            push_family_font(&mut fonts, FontFamily::Proportional, &key, insert_at);
            push_family_font(&mut fonts, FontFamily::Monospace, &key, insert_at);
            insert_at += 1;
        }
    }

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
