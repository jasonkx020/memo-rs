#![allow(dead_code)]

//! 视觉令牌：Lumen 风浅灰 chrome + 深色变体；启动门 mac 灰阶对齐主 UI。

use crate::nav::NavItem;
use eframe::egui::{
    self, Color32, Frame, Margin, Rounding, Stroke, Style, TextStyle, Vec2, Visuals,
};
use memo_core::store::MemoCategory;
use memo_core::ThemePreference;
use parking_lot::RwLock;
use std::sync::OnceLock;

/// 解析后的主题模式（非用户偏好）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
    /// 柔美粉：浅色基底 + 玫瑰雾面配色
    Blush,
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub navy: Color32,
    pub navy_mid: Color32,
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub success: Color32,
    pub success_soft: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub warn: Color32,
    pub list_hover: Color32,
    pub widget_active: Color32,
    pub code_bg: Color32,
    pub mac_bg: Color32,
    pub mac_bg_top: Color32,
    pub mac_card: Color32,
    pub mac_separator: Color32,
    pub mac_fill: Color32,
    pub mac_fill_hover: Color32,
    pub mac_text: Color32,
    pub mac_text_secondary: Color32,
    pub mac_text_tertiary: Color32,
    pub mac_blue: Color32,
    pub mac_blue_pressed: Color32,
    pub mac_red: Color32,
    /// 顶栏/菜单文字色（浅色主题为深字）
    pub chrome_fg: Color32,
    pub chrome_stroke: Color32,
    pub remind_bg: Color32,
    pub remind_fg: Color32,
    /// 分类壳层主色（蓝）
    pub shell_accent: Color32,
    pub shell_accent_soft: Color32,
    pub shell_nav_selected: Color32,
    pub shell_chip_green: Color32,
    pub shell_chip_green_fg: Color32,
    pub shell_chip_gray: Color32,
    pub shell_chip_gray_fg: Color32,
    pub shell_warn_bg: Color32,
    pub shell_warn_fg: Color32,
    pub shell_tag_bg: Color32,
    pub shell_tag_fg: Color32,
    pub shell_women_pill: Color32,
    pub shell_women_pill_fg: Color32,
    pub shell_men_pill: Color32,
    pub shell_men_pill_fg: Color32,
    pub period_pink_bg: Color32,
    pub period_day: Color32,
    pub period_fertile: Color32,
}

pub const ROUND_CARD: f32 = 4.0;
pub const ROUND_CTRL: f32 = 4.0;
pub const MAC_ROUND_SHEET: f32 = 10.0;
pub const MAC_ROUND_CTRL: f32 = 6.0;
pub const MAC_ROUND_PILL: f32 = 8.0;

fn light_palette() -> Palette {
    Palette {
        // chrome：浅灰顶栏（不再用海军蓝块）
        navy: Color32::from_rgb(0xF5, 0xF5, 0xF7),
        navy_mid: Color32::from_rgb(0xEB, 0xEB, 0xEF),
        bg: Color32::from_rgb(0xF7, 0xF7, 0xF8),
        panel: Color32::from_rgb(0xF0, 0xF0, 0xF2),
        card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        border: Color32::from_rgb(0xE5, 0xE5, 0xEA),
        border_strong: Color32::from_rgb(0xD2, 0xD2, 0xD7),
        text: Color32::from_rgb(0x1D, 0x1D, 0x1F),
        text_muted: Color32::from_rgb(0x6E, 0x6E, 0x73),
        accent: Color32::from_rgb(0x00, 0x7A, 0xFF),
        accent_hover: Color32::from_rgb(0x00, 0x64, 0xD1),
        accent_soft: Color32::from_rgb(0xE8, 0xF1, 0xFF),
        success: Color32::from_rgb(0x05, 0x96, 0x69),
        success_soft: Color32::from_rgb(0xD1, 0xFA, 0xE5),
        danger: Color32::from_rgb(0xDC, 0x26, 0x26),
        danger_soft: Color32::from_rgb(0xFE, 0xE2, 0xE2),
        warn: Color32::from_rgb(0xB4, 0x53, 0x09),
        list_hover: Color32::from_rgb(0xE8, 0xE8, 0xED),
        widget_active: Color32::from_rgb(0xD6, 0xE8, 0xFF),
        code_bg: Color32::from_rgb(0xF2, 0xF2, 0xF7),
        mac_bg: Color32::from_rgb(0xF0, 0xF0, 0xF2),
        mac_bg_top: Color32::from_rgb(0xF5, 0xF5, 0xF7),
        mac_card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        mac_separator: Color32::from_rgb(0xE5, 0xE5, 0xEA),
        mac_fill: Color32::from_rgb(0xF2, 0xF2, 0xF7),
        mac_fill_hover: Color32::from_rgb(0xE5, 0xE5, 0xEA),
        mac_text: Color32::from_rgb(0x1D, 0x1D, 0x1F),
        mac_text_secondary: Color32::from_rgb(0x6E, 0x6E, 0x73),
        mac_text_tertiary: Color32::from_rgb(0x8E, 0x8E, 0x93),
        mac_blue: Color32::from_rgb(0x00, 0x7A, 0xFF),
        mac_blue_pressed: Color32::from_rgb(0x00, 0x64, 0xD1),
        mac_red: Color32::from_rgb(0xFF, 0x3B, 0x30),
        chrome_fg: Color32::from_rgb(0x1D, 0x1D, 0x1F),
        chrome_stroke: Color32::from_rgb(0xE5, 0xE5, 0xEA),
        remind_bg: Color32::from_rgb(0xFE, 0xF3, 0xC7),
        remind_fg: Color32::from_rgb(0xB4, 0x53, 0x09),
        shell_accent: Color32::from_rgb(0x3B, 0x82, 0xF6),
        shell_accent_soft: Color32::from_rgb(0xEF, 0xF6, 0xFF),
        shell_nav_selected: Color32::from_rgb(0xDB, 0xEA, 0xFE),
        shell_chip_green: Color32::from_rgb(0xD1, 0xFA, 0xE5),
        shell_chip_green_fg: Color32::from_rgb(0x05, 0x96, 0x69),
        shell_chip_gray: Color32::from_rgb(0xF3, 0xF4, 0xF6),
        shell_chip_gray_fg: Color32::from_rgb(0x6B, 0x72, 0x80),
        shell_warn_bg: Color32::from_rgb(0xFF, 0xF7, 0xED),
        shell_warn_fg: Color32::from_rgb(0xC2, 0x41, 0x0C),
        shell_tag_bg: Color32::from_rgb(0xDB, 0xEA, 0xFE),
        shell_tag_fg: Color32::from_rgb(0x1D, 0x4E, 0xD8),
        shell_women_pill: Color32::from_rgb(0xFD, 0xE8, 0xF0),
        shell_women_pill_fg: Color32::from_rgb(0xDB, 0x27, 0x7A),
        shell_men_pill: Color32::from_rgb(0xE0, 0xF2, 0xFE),
        shell_men_pill_fg: Color32::from_rgb(0x03, 0x67, 0xA1),
        // 预测浅填：够粉可辨、仍浅于手标实填；易孕：描边/角标用色，需与白底高对比
        period_pink_bg: Color32::from_rgb(0xFB, 0xCF, 0xE8),
        period_day: Color32::from_rgb(0xDB, 0x27, 0x7A),
        period_fertile: Color32::from_rgb(0x0F, 0x76, 0x6E),
    }
}

/// 柔美：雾面浅粉底 + 玫瑰强调，低对比柔和，适合长时间阅读与女性私密场景。
fn blush_palette() -> Palette {
    Palette {
        navy: Color32::from_rgb(0xFD, 0xF2, 0xF8),
        navy_mid: Color32::from_rgb(0xFC, 0xE7, 0xF3),
        bg: Color32::from_rgb(0xFF, 0xF7, 0xFA),
        panel: Color32::from_rgb(0xFD, 0xF2, 0xF8),
        card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        border: Color32::from_rgb(0xFB, 0xE4, 0xEF),
        border_strong: Color32::from_rgb(0xF9, 0xC8, 0xDC),
        text: Color32::from_rgb(0x4A, 0x2C, 0x3D),
        text_muted: Color32::from_rgb(0x9D, 0x71, 0x88),
        accent: Color32::from_rgb(0xE8, 0x5A, 0x9B),
        accent_hover: Color32::from_rgb(0xDB, 0x27, 0x7A),
        accent_soft: Color32::from_rgb(0xFC, 0xE7, 0xF3),
        success: Color32::from_rgb(0x0D, 0x94, 0x88),
        success_soft: Color32::from_rgb(0xCC, 0xFB, 0xF1),
        danger: Color32::from_rgb(0xE1, 0x1D, 0x48),
        danger_soft: Color32::from_rgb(0xFF, 0xE4, 0xE6),
        warn: Color32::from_rgb(0xC2, 0x4B, 0x6E),
        list_hover: Color32::from_rgb(0xFC, 0xE7, 0xF3),
        widget_active: Color32::from_rgb(0xFB, 0xD0, 0xE8),
        code_bg: Color32::from_rgb(0xFD, 0xF2, 0xF8),
        mac_bg: Color32::from_rgb(0xFD, 0xF2, 0xF8),
        mac_bg_top: Color32::from_rgb(0xFF, 0xF7, 0xFA),
        mac_card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        mac_separator: Color32::from_rgb(0xFB, 0xE4, 0xEF),
        mac_fill: Color32::from_rgb(0xFC, 0xE7, 0xF3),
        mac_fill_hover: Color32::from_rgb(0xFB, 0xD0, 0xE8),
        mac_text: Color32::from_rgb(0x4A, 0x2C, 0x3D),
        mac_text_secondary: Color32::from_rgb(0x9D, 0x71, 0x88),
        mac_text_tertiary: Color32::from_rgb(0xB8, 0x8F, 0xA3),
        mac_blue: Color32::from_rgb(0xE8, 0x5A, 0x9B),
        mac_blue_pressed: Color32::from_rgb(0xDB, 0x27, 0x7A),
        mac_red: Color32::from_rgb(0xE1, 0x1D, 0x48),
        chrome_fg: Color32::from_rgb(0x4A, 0x2C, 0x3D),
        chrome_stroke: Color32::from_rgb(0xFB, 0xE4, 0xEF),
        remind_bg: Color32::from_rgb(0xFF, 0xED, 0xD5),
        remind_fg: Color32::from_rgb(0xC2, 0x41, 0x0C),
        shell_accent: Color32::from_rgb(0xE8, 0x5A, 0x9B),
        shell_accent_soft: Color32::from_rgb(0xFC, 0xE7, 0xF3),
        shell_nav_selected: Color32::from_rgb(0xFB, 0xD0, 0xE8),
        shell_chip_green: Color32::from_rgb(0xCC, 0xFB, 0xF1),
        shell_chip_green_fg: Color32::from_rgb(0x0D, 0x94, 0x88),
        shell_chip_gray: Color32::from_rgb(0xFD, 0xF2, 0xF8),
        shell_chip_gray_fg: Color32::from_rgb(0x9D, 0x71, 0x88),
        shell_warn_bg: Color32::from_rgb(0xFF, 0xF1, 0xF2),
        shell_warn_fg: Color32::from_rgb(0xBE, 0x12, 0x3C),
        shell_tag_bg: Color32::from_rgb(0xFC, 0xE7, 0xF3),
        shell_tag_fg: Color32::from_rgb(0xBE, 0x18, 0x5D),
        shell_women_pill: Color32::from_rgb(0xFB, 0xD0, 0xE8),
        shell_women_pill_fg: Color32::from_rgb(0xBE, 0x18, 0x5D),
        shell_men_pill: Color32::from_rgb(0xE0, 0xF2, 0xFE),
        shell_men_pill_fg: Color32::from_rgb(0x0C, 0x4A, 0x6E),
        period_pink_bg: Color32::from_rgb(0xFB, 0xD0, 0xE8),
        period_day: Color32::from_rgb(0xBE, 0x18, 0x5D),
        period_fertile: Color32::from_rgb(0x0F, 0x76, 0x6E),
    }
}

/// 深色：浅灰顶栏条（略亮于 bg），避免旧海军蓝块。
fn dark_palette() -> Palette {
    Palette {
        navy: Color32::from_rgb(0x32, 0x32, 0x36),
        navy_mid: Color32::from_rgb(0x3A, 0x3A, 0x3E),
        bg: Color32::from_rgb(0x28, 0x28, 0x2C),
        panel: Color32::from_rgb(0x2E, 0x2E, 0x32),
        card: Color32::from_rgb(0x36, 0x36, 0x3A),
        border: Color32::from_rgb(0x48, 0x48, 0x4C),
        border_strong: Color32::from_rgb(0x5A, 0x5A, 0x5E),
        text: Color32::from_rgb(0xF5, 0xF5, 0xF7),
        text_muted: Color32::from_rgb(0xA1, 0xA1, 0xA6),
        accent: Color32::from_rgb(0x0A, 0x84, 0xFF),
        accent_hover: Color32::from_rgb(0x40, 0x9C, 0xFF),
        accent_soft: Color32::from_rgb(0x1C, 0x3A, 0x5C),
        success: Color32::from_rgb(0x3D, 0xA8, 0x7C),
        success_soft: Color32::from_rgb(0x1F, 0x3A, 0x30),
        danger: Color32::from_rgb(0xE0, 0x5C, 0x5C),
        danger_soft: Color32::from_rgb(0x4A, 0x2A, 0x2A),
        warn: Color32::from_rgb(0xD4, 0xA0, 0x3C),
        list_hover: Color32::from_rgb(0x3E, 0x3E, 0x42),
        widget_active: Color32::from_rgb(0x2F, 0x4A, 0x6E),
        code_bg: Color32::from_rgb(0x2A, 0x2A, 0x2E),
        mac_bg: Color32::from_rgb(0x2E, 0x2E, 0x32),
        mac_bg_top: Color32::from_rgb(0x32, 0x32, 0x36),
        mac_card: Color32::from_rgb(0x3A, 0x3A, 0x3E),
        mac_separator: Color32::from_rgb(0x48, 0x48, 0x4C),
        mac_fill: Color32::from_rgb(0x3A, 0x3A, 0x3E),
        mac_fill_hover: Color32::from_rgb(0x48, 0x48, 0x4C),
        mac_text: Color32::from_rgb(0xF5, 0xF5, 0xF7),
        mac_text_secondary: Color32::from_rgb(0xA1, 0xA1, 0xA6),
        mac_text_tertiary: Color32::from_rgb(0x8E, 0x8E, 0x93),
        mac_blue: Color32::from_rgb(0x0A, 0x84, 0xFF),
        mac_blue_pressed: Color32::from_rgb(0x00, 0x6E, 0xD8),
        mac_red: Color32::from_rgb(0xFF, 0x45, 0x3A),
        chrome_fg: Color32::from_rgb(0xF5, 0xF5, 0xF7),
        chrome_stroke: Color32::from_rgb(0x48, 0x48, 0x4C),
        remind_bg: Color32::from_rgb(0x4A, 0x3C, 0x1A),
        remind_fg: Color32::from_rgb(0xE8, 0xC4, 0x6A),
        shell_accent: Color32::from_rgb(0x60, 0xA5, 0xFA),
        shell_accent_soft: Color32::from_rgb(0x1E, 0x3A, 0x5F),
        shell_nav_selected: Color32::from_rgb(0x1E, 0x3A, 0x5F),
        shell_chip_green: Color32::from_rgb(0x1F, 0x3A, 0x30),
        shell_chip_green_fg: Color32::from_rgb(0x6E, 0xE7, 0xB7),
        shell_chip_gray: Color32::from_rgb(0x3A, 0x3A, 0x3E),
        shell_chip_gray_fg: Color32::from_rgb(0xA1, 0xA1, 0xA6),
        shell_warn_bg: Color32::from_rgb(0x4A, 0x3C, 0x1A),
        shell_warn_fg: Color32::from_rgb(0xFB, 0xBF, 0x24),
        shell_tag_bg: Color32::from_rgb(0x1E, 0x3A, 0x5F),
        shell_tag_fg: Color32::from_rgb(0x93, 0xC5, 0xFD),
        shell_women_pill: Color32::from_rgb(0x4A, 0x2A, 0x38),
        shell_women_pill_fg: Color32::from_rgb(0xF4, 0x72, 0xB6),
        shell_men_pill: Color32::from_rgb(0x1E, 0x3A, 0x5F),
        shell_men_pill_fg: Color32::from_rgb(0x7D, 0xD3, 0xFC),
        period_pink_bg: Color32::from_rgb(0x5C, 0x2E, 0x42),
        period_day: Color32::from_rgb(0xF4, 0x72, 0xB6),
        period_fertile: Color32::from_rgb(0x5E, 0xEA, 0xD4),
    }
}

fn current_lock() -> &'static RwLock<Palette> {
    static LOCK: OnceLock<RwLock<Palette>> = OnceLock::new();
    LOCK.get_or_init(|| RwLock::new(light_palette()))
}

pub fn c() -> Palette {
    *current_lock().read()
}

pub fn set_current(p: Palette) {
    *current_lock().write() = p;
}

pub fn palette_for(mode: ThemeMode) -> Palette {
    match mode {
        ThemeMode::Light => light_palette(),
        ThemeMode::Dark => dark_palette(),
        ThemeMode::Blush => blush_palette(),
    }
}

/// 将用户偏好解析为具体模式。`system_dark=None` 时按浅色回退。
pub fn resolve(pref: ThemePreference, system_dark: Option<bool>) -> ThemeMode {
    match pref {
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
        ThemePreference::Blush => ThemeMode::Blush,
        ThemePreference::System => {
            if system_dark.unwrap_or(false) {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            }
        }
    }
}

/// 读取操作系统是否偏好深色（egui 0.27 无 system_theme，自检）。
pub fn system_prefers_dark() -> Option<bool> {
    #[cfg(windows)]
    {
        system_prefers_dark_windows()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn system_prefers_dark_windows() -> Option<bool> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ, REG_DWORD,
    };

    // Software\Microsoft\Windows\CurrentVersion\Themes\Personalize
    const KEY: &[u16] = &[
        b'S' as u16, b'o' as u16, b'f' as u16, b't' as u16, b'w' as u16, b'a' as u16, b'r' as u16,
        b'e' as u16, b'\\' as u16, b'M' as u16, b'i' as u16, b'c' as u16, b'r' as u16, b'o' as u16,
        b's' as u16, b'o' as u16, b'f' as u16, b't' as u16, b'\\' as u16, b'W' as u16, b'i' as u16,
        b'n' as u16, b'd' as u16, b'o' as u16, b'w' as u16, b's' as u16, b'\\' as u16, b'C' as u16,
        b'u' as u16, b'r' as u16, b'r' as u16, b'e' as u16, b'n' as u16, b't' as u16, b'V' as u16,
        b'e' as u16, b'r' as u16, b's' as u16, b'i' as u16, b'o' as u16, b'n' as u16, b'\\' as u16,
        b'T' as u16, b'h' as u16, b'e' as u16, b'm' as u16, b'e' as u16, b's' as u16, b'\\' as u16,
        b'P' as u16, b'e' as u16, b'r' as u16, b's' as u16, b'o' as u16, b'n' as u16, b'a' as u16,
        b'l' as u16, b'i' as u16, b'z' as u16, b'e' as u16, 0,
    ];
    const VAL: &[u16] = &[
        b'A' as u16, b'p' as u16, b'p' as u16, b's' as u16, b'U' as u16, b's' as u16, b'e' as u16,
        b'L' as u16, b'i' as u16, b'g' as u16, b'h' as u16, b't' as u16, b'T' as u16, b'h' as u16,
        b'e' as u16, b'm' as u16, b'e' as u16, 0,
    ];

    unsafe {
        let mut hkey = 0;
        if RegOpenKeyExW(HKEY_CURRENT_USER, KEY.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return None;
        }
        let mut typ = 0u32;
        let mut data = 0u32;
        let mut data_bytes = 4u32;
        let rc = RegQueryValueExW(
            hkey,
            VAL.as_ptr(),
            std::ptr::null_mut(),
            &mut typ,
            &mut data as *mut u32 as *mut u8,
            &mut data_bytes,
        );
        RegCloseKey(hkey);
        if rc != 0 || typ != REG_DWORD {
            return None;
        }
        // AppsUseLightTheme: 1 = 浅色, 0 = 深色
        Some(data == 0)
    }
}

pub fn navy() -> Color32 {
    c().navy
}
pub fn navy_mid() -> Color32 {
    c().navy_mid
}
pub fn bg() -> Color32 {
    c().bg
}
pub fn panel() -> Color32 {
    c().panel
}
pub fn card() -> Color32 {
    c().card
}
pub fn border() -> Color32 {
    c().border
}
pub fn border_strong() -> Color32 {
    c().border_strong
}
pub fn text() -> Color32 {
    c().text
}
pub fn text_muted() -> Color32 {
    c().text_muted
}
pub fn accent() -> Color32 {
    c().accent
}
pub fn accent_hover() -> Color32 {
    c().accent_hover
}
pub fn accent_soft() -> Color32 {
    c().accent_soft
}
pub fn success() -> Color32 {
    c().success
}
pub fn success_soft() -> Color32 {
    c().success_soft
}
pub fn danger() -> Color32 {
    c().danger
}
pub fn danger_soft() -> Color32 {
    c().danger_soft
}
pub fn warn() -> Color32 {
    c().warn
}
pub fn mac_bg() -> Color32 {
    c().mac_bg
}
pub fn mac_bg_top() -> Color32 {
    c().mac_bg_top
}
pub fn mac_card() -> Color32 {
    c().mac_card
}
pub fn mac_separator() -> Color32 {
    c().mac_separator
}
pub fn mac_fill() -> Color32 {
    c().mac_fill
}
pub fn mac_fill_hover() -> Color32 {
    c().mac_fill_hover
}
pub fn mac_text() -> Color32 {
    c().mac_text
}
pub fn mac_text_secondary() -> Color32 {
    c().mac_text_secondary
}
pub fn mac_text_tertiary() -> Color32 {
    c().mac_text_tertiary
}
pub fn mac_blue() -> Color32 {
    c().mac_blue
}
pub fn mac_blue_pressed() -> Color32 {
    c().mac_blue_pressed
}
pub fn mac_red() -> Color32 {
    c().mac_red
}
pub fn chrome_fg() -> Color32 {
    c().chrome_fg
}
pub fn chrome_stroke() -> Color32 {
    c().chrome_stroke
}
pub fn remind_bg() -> Color32 {
    c().remind_bg
}
pub fn remind_fg() -> Color32 {
    c().remind_fg
}
pub fn shell_accent() -> Color32 {
    c().shell_accent
}

fn dark_icons() -> bool {
    let [r, g, b, _] = c().bg.to_array();
    (r as u16) + (g as u16) + (b as u16) < 380
}

/// 分类图标语义色（待办绿、应急红等）。
pub fn category_icon_color(cat: MemoCategory) -> Color32 {
    let dark = dark_icons();
    match cat {
        MemoCategory::Todo => success(),
        MemoCategory::Work | MemoCategory::Office => shell_accent(),
        MemoCategory::Credentials => {
            if dark {
                Color32::from_rgb(0x7E, 0xB6, 0xD9)
            } else {
                Color32::from_rgb(0x3D, 0x7A, 0xA8)
            }
        }
        MemoCategory::Life => {
            if dark {
                Color32::from_rgb(0x4E, 0xC9, 0xB0)
            } else {
                Color32::from_rgb(0x1F, 0xA0, 0x7A)
            }
        }
        MemoCategory::Finance => {
            if dark {
                Color32::from_rgb(0xE8, 0xC3, 0x4A)
            } else {
                Color32::from_rgb(0xC4, 0x8E, 0x14)
            }
        }
        MemoCategory::Emergency => danger(),
        MemoCategory::Inspiration => {
            if dark {
                Color32::from_rgb(0xC4, 0xA0, 0xF8)
            } else {
                Color32::from_rgb(0x7C, 0x4D, 0xDE)
            }
        }
        MemoCategory::GenderPrivate | MemoCategory::WomenPrivate => shell_women_pill_fg(),
        MemoCategory::MalePrivate => shell_men_pill_fg(),
        MemoCategory::General => text(),
    }
}

/// 左侧菜单图标语义色。
pub fn nav_icon_color(item: NavItem) -> Color32 {
    match item {
        NavItem::All => {
            if dark_icons() {
                Color32::from_rgb(0x8E, 0xA4, 0xC8)
            } else {
                Color32::from_rgb(0x3D, 0x5A, 0x80)
            }
        }
        NavItem::DueToday => warn(),
        NavItem::Trash => text_muted(),
        other => other
            .category()
            .map(category_icon_color)
            .unwrap_or_else(text),
    }
}

/// 图标与标签分色排版。
pub fn icon_label_job(
    icon: &str,
    label: &str,
    icon_color: Color32,
    label_color: Color32,
    icon_size: f32,
    label_size: f32,
) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let icon_font = egui::FontId::proportional(icon_size);
    let label_font = egui::FontId::proportional(label_size);
    let row_h = icon_size.max(label_size);
    let mut job = LayoutJob::default();
    job.first_row_min_height = row_h;
    let icon_fmt = TextFormat {
        font_id: icon_font,
        color: icon_color,
        valign: egui::Align::Center,
        line_height: Some(row_h),
        ..Default::default()
    };
    let label_fmt = TextFormat {
        font_id: label_font,
        color: label_color,
        valign: egui::Align::Center,
        line_height: Some(row_h),
        ..Default::default()
    };
    job.append(icon, 0.0, icon_fmt);
    job.append("  ", 0.0, label_fmt.clone());
    job.append(label, 0.0, label_fmt);
    job
}

pub fn layout_galley(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font, color))
}

/// 按字形墨水包围盒对齐，避免 emoji 与汉字视觉中心错位。
pub fn galley_pos_center(center: egui::Pos2, g: &egui::Galley) -> egui::Pos2 {
    let m = g.mesh_bounds;
    if m.width() > 0.5 && m.height() > 0.5 {
        egui::pos2(center.x - m.center().x, center.y - m.center().y)
    } else {
        egui::pos2(center.x - g.size().x * 0.5, center.y - g.size().y * 0.5)
    }
}

pub fn galley_pos_left_center(left_center: egui::Pos2, g: &egui::Galley) -> egui::Pos2 {
    let m = g.mesh_bounds;
    if m.height() > 0.5 {
        egui::pos2(left_center.x - m.left(), left_center.y - m.center().y)
    } else {
        egui::pos2(left_center.x, left_center.y - g.size().y * 0.5)
    }
}

/// 列表/回收站标题：图标与文字按墨水中心线对齐。
pub fn icon_label_heading(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    icon_color: Color32,
    label_color: Color32,
    icon_size: f32,
    label_size: f32,
) {
    let icon_g = layout_galley(
        ui,
        icon,
        egui::FontId::proportional(icon_size),
        icon_color,
    );
    let label_g = layout_galley(
        ui,
        label,
        egui::FontId::proportional(label_size),
        label_color,
    );
    let icon_slot = (icon_size * 1.35).max(icon_g.mesh_bounds.width());
    let h = icon_size.max(label_size) * 1.2;
    let w = icon_slot + 8.0 + label_g.size().x + 2.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::hover());
    let cy = rect.center().y;
    ui.painter().galley(
        galley_pos_center(egui::pos2(rect.left() + icon_slot * 0.5, cy), &icon_g),
        icon_g,
        icon_color,
    );
    ui.painter().galley(
        galley_pos_left_center(egui::pos2(rect.left() + icon_slot + 8.0, cy), &label_g),
        label_g,
        label_color,
    );
}
pub fn shell_accent_soft() -> Color32 {
    c().shell_accent_soft
}
pub fn shell_nav_selected() -> Color32 {
    c().shell_nav_selected
}
pub fn shell_chip_green() -> Color32 {
    c().shell_chip_green
}
pub fn shell_chip_green_fg() -> Color32 {
    c().shell_chip_green_fg
}
pub fn shell_chip_gray() -> Color32 {
    c().shell_chip_gray
}
pub fn shell_chip_gray_fg() -> Color32 {
    c().shell_chip_gray_fg
}
pub fn shell_warn_bg() -> Color32 {
    c().shell_warn_bg
}
pub fn shell_warn_fg() -> Color32 {
    c().shell_warn_fg
}
pub fn shell_tag_bg() -> Color32 {
    c().shell_tag_bg
}
pub fn shell_tag_fg() -> Color32 {
    c().shell_tag_fg
}
pub fn shell_women_pill() -> Color32 {
    c().shell_women_pill
}
pub fn shell_women_pill_fg() -> Color32 {
    c().shell_women_pill_fg
}
pub fn shell_men_pill() -> Color32 {
    c().shell_men_pill
}
pub fn shell_men_pill_fg() -> Color32 {
    c().shell_men_pill_fg
}
pub fn period_pink_bg() -> Color32 {
    c().period_pink_bg
}
pub fn period_day() -> Color32 {
    c().period_day
}
pub fn period_fertile() -> Color32 {
    c().period_fertile
}

/// 头像底色：按索引循环。
pub fn avatar_color(index: usize) -> Color32 {
    const COLORS: [Color32; 8] = [
        Color32::from_rgb(0x3B, 0x82, 0xF6),
        Color32::from_rgb(0x0E, 0xA5, 0xE9),
        Color32::from_rgb(0x10, 0xB9, 0x81),
        Color32::from_rgb(0xF5, 0x9E, 0x0B),
        Color32::from_rgb(0xEF, 0x44, 0x44),
        Color32::from_rgb(0xEC, 0x48, 0x99),
        Color32::from_rgb(0x8B, 0x5C, 0xF6),
        Color32::from_rgb(0x14, 0xB8, 0xA6),
    ];
    COLORS[index % COLORS.len()]
}

pub fn apply(ctx: &egui::Context, mode: ThemeMode) {
    let p = palette_for(mode);
    set_current(p);

    let mut visuals = match mode {
        ThemeMode::Light | ThemeMode::Blush => Visuals::light(),
        ThemeMode::Dark => Visuals::dark(),
    };
    visuals.dark_mode = matches!(mode, ThemeMode::Dark);
    visuals.window_fill = p.card;
    visuals.panel_fill = p.panel;
    visuals.extreme_bg_color = p.bg;
    visuals.faint_bg_color = p.panel;
    visuals.code_bg_color = p.code_bg;
    // 勿设 override_text_color：否则 TextEdit hint 会在 layout 时被烘焙成正文字色，
    // painter 的 weak_text_color 无法生效。正文色走 widgets.*.fg_stroke。
    visuals.override_text_color = None;
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.warn;
    visuals.error_fg_color = p.danger;
    visuals.window_rounding = Rounding::same(ROUND_CARD);
    visuals.menu_rounding = Rounding::same(ROUND_CTRL);
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.noninteractive.bg_fill = p.card;
    visuals.widgets.noninteractive.weak_bg_fill = p.panel;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.noninteractive.rounding = Rounding::same(ROUND_CTRL);
    visuals.widgets.inactive.bg_fill = p.card;
    visuals.widgets.inactive.weak_bg_fill = p.panel;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border_strong);
    visuals.widgets.inactive.rounding = Rounding::same(ROUND_CTRL);
    visuals.widgets.hovered.bg_fill = p.accent_soft;
    visuals.widgets.hovered.weak_bg_fill = p.panel;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.hovered.rounding = Rounding::same(ROUND_CTRL);
    visuals.widgets.active.bg_fill = p.widget_active;
    visuals.widgets.active.weak_bg_fill = p.panel;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, p.accent);
    visuals.widgets.active.rounding = Rounding::same(ROUND_CTRL);
    visuals.widgets.open.bg_fill = p.accent_soft;
    visuals.widgets.open.weak_bg_fill = p.panel;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.open.rounding = Rounding::same(ROUND_CTRL);
    visuals.selection.bg_fill = p.accent;
    visuals.selection.stroke = Stroke::new(1.0, p.accent_hover);
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

/// 若模式变化则应用并返回新模式。
pub fn sync(ctx: &egui::Context, pref: ThemePreference, last: &mut Option<ThemeMode>) {
    let mode = resolve(pref, system_prefers_dark());
    if last.map(|m| m != mode).unwrap_or(true) {
        apply(ctx, mode);
        *last = Some(mode);
    }
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
        .fill(card())
        .stroke(Stroke::new(1.0, border()))
        .rounding(Rounding::same(ROUND_CARD))
        .inner_margin(Margin::same(16.0))
}

/// 中央内容区：白底、极淡边或无边，贴近「白纸」。
pub fn content_frame() -> Frame {
    Frame::none()
        .fill(card())
        .inner_margin(Margin::same(16.0))
}

/// 启动门：大圆角白卡片（类似 macOS sheet）。
pub fn mac_sheet_frame() -> Frame {
    Frame::none()
        .fill(mac_card())
        .stroke(Stroke::new(0.5, mac_separator()))
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
        .fill(panel())
        .inner_margin(Margin::same(12.0))
}

/// 左侧栏：浅灰 + 右侧细分割线。
pub fn left_panel_frame() -> Frame {
    Frame::none()
        .fill(panel())
        .stroke(Stroke::new(1.0, border()))
        .inner_margin(Margin::same(12.0))
}

/// 右侧栏：浅灰 + 左侧细分割线（用 stroke 整框，视觉足够）。
pub fn right_panel_frame() -> Frame {
    Frame::none()
        .fill(panel())
        .stroke(Stroke::new(1.0, border()))
        .inner_margin(Margin::same(12.0))
}

pub fn top_bar_frame() -> Frame {
    Frame::none()
        .fill(navy())
        .stroke(Stroke::new(1.0, border()))
        .inner_margin(Margin::symmetric(16.0, 10.0))
}

pub fn bottom_bar_frame() -> Frame {
    Frame::none()
        .fill(navy())
        .stroke(Stroke::new(1.0, border()))
        .inner_margin(Margin::symmetric(14.0, 7.0))
}

pub fn list_row_fill(selected: bool, hovered: bool) -> Color32 {
    if selected {
        accent_soft()
    } else if hovered {
        c().list_hover
    } else {
        panel()
    }
}

/// 顶栏文字菜单项（无填充，疏朗）。
pub fn menu_text_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(14.0).color(chrome_fg()))
            .frame(false)
            .min_size(Vec2::new(0.0, 26.0)),
    )
}

pub fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).strong()).fill(accent()),
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
            .fill(mac_blue())
            .rounding(Rounding::same(MAC_ROUND_PILL))
            .min_size(Vec2::new(w, 36.0)),
    )
}

/// macOS 次要按钮（浅灰底）。
pub fn mac_secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(mac_text()).size(14.0))
            .fill(mac_fill())
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
        mac_fill(),
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
                mac_card(),
                Stroke::new(0.5, mac_separator()),
            );
        }
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(13.0),
            if selected {
                mac_text()
            } else {
                mac_text_secondary()
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
                if active { mac_blue() } else { mac_separator() },
            );
            ui.add_space(6.0);
        }
    });
}

pub fn success_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).strong()).fill(success()),
    )
}

pub fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).strong()).fill(danger()),
    )
}

pub fn ghost_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(label).color(text())))
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
        let muted = text_muted();
        let galley = ui.fonts(|f| f.layout_no_wrap(hint.to_owned(), font, muted));
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + pad_x, rect.top()),
            Vec2::new((rect.width() - pad_x * 2.0).max(1.0), rect.height()),
        );
        let pos = Align2::LEFT_CENTER
            .align_size_within_rect(galley.size(), text_rect)
            .min;
        ui.painter().galley(pos, galley, muted);
    }

    response
}

pub const APP_NAME: &str = "分布式备忘录";
pub const APP_COPYRIGHT: &str = "Copyright (C) 2026 DistributedMemo. All rights reserved.";
pub const APP_FILE_VERSION: &str = "0.3.0.0";
pub const APP_DESCRIPTION: &str = "本地加密 · 局域网同步 · 绿色单文件分布式备忘录";

pub fn muted_label(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).color(text_muted())
}

/// 输入框占位提示：淡色，随当前主题 `text_muted` 变化。
pub fn hint(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).color(text_muted())
}

pub fn brand_title(size: f32) -> egui::RichText {
    egui::RichText::new("分布式备忘录")
        .strong()
        .size(size)
        .color(text())
}

pub fn brand_title_on_navy(size: f32) -> egui::RichText {
    egui::RichText::new("分布式备忘录")
        .strong()
        .size(size)
        .color(chrome_fg())
}

/// 贴屏约束的模态窗：居中、可缩放、不超过屏幕。
pub fn modal_window<'a>(
    ctx: &egui::Context,
    title: impl Into<egui::WidgetText>,
) -> egui::Window<'a> {
    let screen = ctx.screen_rect();
    let max = Vec2::new(
        (screen.width() - 24.0).max(320.0),
        (screen.height() - 24.0).max(240.0),
    );
    // 默认尺寸适中，避免仅改 width 时仍继承「接近全屏高」导致内容反向撑窗
    let default_h = 420.0_f32.min(max.y);
    let default_w = 520.0_f32.min(max.x);
    egui::Window::new(title)
        .collapsible(false)
        .resizable(true)
        .constrain(true)
        .constrain_to(screen)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .default_size([default_w, default_h])
        .min_size([320.0, 200.0])
        .max_size(max)
}

/// 固定尺寸表单弹窗：禁止内容反向撑开（每帧缓慢延伸）。
pub fn modal_fixed<'a>(
    ctx: &egui::Context,
    title: impl Into<egui::WidgetText>,
    size: [f32; 2],
) -> egui::Window<'a> {
    let screen = ctx.screen_rect();
    let w = size[0].min((screen.width() - 24.0).max(280.0));
    let h = size[1].min((screen.height() - 24.0).max(180.0));
    modal_window(ctx, title)
        .fixed_size([w, h])
        .resizable(false)
}

/// 短确认框：不可缩放、居中贴屏。
pub fn modal_confirm<'a>(
    ctx: &egui::Context,
    title: impl Into<egui::WidgetText>,
) -> egui::Window<'a> {
    modal_fixed(ctx, title, [420.0, 180.0])
}

/// 一次模态会话：遮罩 + 窗口 Id。用法：
/// ```ignore
/// let modal = theme::begin_modal(ctx, "settings");
/// let mut open = true;
/// theme::modal_fixed(...).id(modal.window_id).open(&mut open).show(...);
/// if modal.end(ctx, open) { /* 仅标题栏 X 关闭；点遮罩不关 */ }
/// ```
pub struct ModalSession {
    pub window_id: egui::Id,
}

/// 绘制遮罩并分配窗口 Id（请用于 `.id(modal.window_id)`）。
pub fn begin_modal(ctx: &egui::Context, key: &'static str) -> ModalSession {
    let window_id = egui::Id::new(key);
    modal_dimmer(ctx, egui::Id::new((key, "dimmer")));
    ModalSession { window_id }
}

impl ModalSession {
    /// 窗体绘完后调用：抬升窗口；仅当标题栏 X 关掉时返回 `true`。
    /// `still_open`：标题栏关闭按钮对应的 `open`；无关闭按钮时传 `true`。
    /// 点遮罩**不会**关闭（遮罩只拦截底层交互）。
    pub fn end(self, ctx: &egui::Context, still_open: bool) -> bool {
        raise_modal(ctx, self.window_id);
        !still_open
    }
}

/// 半透明遮罩：拦截底层点击，**不**用于关闭弹窗。须在对应 Window **之前**调用。
///
/// egui 0.27 的 Window 与 Area 同属 `Order::Middle`：点击遮罩会 `move_to_top`，
/// 若不再把窗口抬回顶层，遮罩会盖住窗口并吞掉所有点击（界面假死）。
/// 因此 Window 绘制后务必调用 [`raise_modal`]（或 [`ModalSession::end`]）。
pub fn modal_dimmer(ctx: &egui::Context, id: impl Into<egui::Id>) {
    let screen = ctx.screen_rect();
    egui::Area::new(id.into())
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .interactable(true)
        .show(ctx, |ui| {
            // 先占位再绘制，避免 painter 裁剪到 Area 初始空矩形导致遮罩不可见
            let response = ui.allocate_response(screen.size(), egui::Sense::click());
            ui.painter().rect_filled(
                response.rect,
                0.0,
                Color32::from_rgba_unmultiplied(0, 0, 0, 110),
            );
        });
}

/// 将模态 Window 抬到同层最顶（须与 Window 的 `.id(...)` 一致）。
pub fn raise_modal(ctx: &egui::Context, id: egui::Id) {
    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
}
