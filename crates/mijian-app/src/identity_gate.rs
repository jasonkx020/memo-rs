//! 桌面终端用户身份门：选择/生成/导入密钥对。
//!
//! 视觉对齐 `docs/ui-mockup/login.html`：品牌先行 + 登录动作，雾绿氛围。
//! 与服务器「存储节点」身份不同：此处创建的是终端用户会话身份（含 Person、私人备忘）。
//! 无 UI 存储节点请用 CLI 自动 bootstrap / `--init`，勿走本模块。

use crate::app_icon;
use crate::autostart;
use crate::save_dialog;
use crate::theme;
use crate::tip_code;
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Frame, Layout, Margin, RichText, Sense, Stroke, Vec2,
    Visuals,
};
use mijian_core::config::{self, Config};
use mijian_core::identity_keys::{self, IdentityKeys, IdentityMeta};
use std::path::PathBuf;

pub struct IdentityGateState {
    pub identities: Vec<IdentityMeta>,
    pub selected: Option<String>,
    pub status: String,
    pub busy: bool,
    pub legacy_wipe_confirm: bool,
    pub show_init: bool,
    pub init_alias: String,
    pub init_drive: String,
    pub init_import_path: String,
    pub init_export_path: String,
    /// 旧版加密 .memokey 的解密口令（新明文包不需要）
    pub init_legacy_pack_pass: String,
    /// 生成/导入时设置的本机登录密码
    pub init_local_pass: String,
    pub init_local_pass2: String,
    /// 选择已有身份时的登录密码（设密时与 confirm 成对）
    pub select_pass: String,
    pub select_pass2: String,
    /// 选中身份后，下一帧让密码框获取焦点
    pub focus_select_pass: bool,
    pub init_mode_import: bool,
    pub pending_identity: Option<IdentityKeys>,
    /// 用户主动退出后为 true（保留字段；单身份不再自动进入）
    pub skip_auto_unlock: bool,
}

impl IdentityGateState {
    pub fn from_cfg(cfg: &Config) -> Self {
        let data = PathBuf::from(&cfg.data_dir);
        let legacy = identity_keys::looks_like_legacy_data(&data);
        let identities = IdentityKeys::list(&data).unwrap_or_default();
        let show_init = identities.is_empty() && !legacy;
        Self {
            identities,
            selected: None,
            status: if legacy {
                "检测到旧版数据，与当前版本不兼容。请备份后确认清空并重新初始化。".into()
            } else {
                String::new()
            },
            busy: false,
            legacy_wipe_confirm: legacy,
            show_init,
            init_alias: String::new(),
            init_drive: config::default_data_drive(),
            init_import_path: String::new(),
            init_export_path: String::new(),
            init_legacy_pack_pass: String::new(),
            init_local_pass: String::new(),
            init_local_pass2: String::new(),
            select_pass: String::new(),
            select_pass2: String::new(),
            focus_select_pass: false,
            init_mode_import: false,
            pending_identity: None,
            skip_auto_unlock: false,
        }
    }

    pub fn refresh_list(&mut self, cfg: &Config) {
        let data = PathBuf::from(&cfg.data_dir);
        self.identities = IdentityKeys::list(&data).unwrap_or_default();
    }
}

pub enum GateAction {
    None,
    ShowHelp,
    /// 打开打赏二维码大图
    ShowTip,
    /// 后台打开身份；若带 export，先写出 .memokey 再开库
    Unlock {
        cfg: Config,
        identity: IdentityKeys,
        export_path: Option<String>,
        export_pass: String,
        /// 本机登录密码（密封 secret.key / 迁移设密）
        local_password: String,
    },
}

/// 绘制密码框；若在密码框上回车则返回 `true`（用于提交登录）。
fn local_password_fields(
    ui: &mut egui::Ui,
    pass: &mut String,
    pass2: &mut String,
    setup: bool,
    focus_first: bool,
) -> bool {
    let w = ui.available_width();
    let enter = |ui: &egui::Ui, r: &egui::Response| {
        r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
    };
    if setup {
        field_label(ui, "登录密码（至少 6 位）");
        let r = gate_password_field(ui, pass, "设置登录密码", w, 36.0);
        if focus_first {
            r.request_focus();
        }
        let mut submit = enter(ui, &r);
        ui.add_space(6.0);
        field_label(ui, "确认登录密码");
        let r2 = gate_password_field(ui, pass2, "再输入一次", w, 36.0);
        submit |= enter(ui, &r2);
        ui.add_space(4.0);
        ui.label(
            RichText::new("用于进入本机并加密数据备份。密钥包请妥善保管。")
                .size(11.0)
                .color(gate_muted()),
        );
        submit
    } else {
        field_label(ui, "登录密码");
        let r = gate_password_field(ui, pass, "输入登录密码", w, 36.0);
        if focus_first {
            r.request_focus();
        }
        enter(ui, &r)
    }
}

/// 登录门固定浅色品牌色（对齐 login.html，不受会话主题切换影响）。
fn gate_paper() -> Color32 {
    Color32::from_rgb(0xF3, 0xF6, 0xF2)
}
fn gate_grad_top() -> Color32 {
    Color32::from_rgb(0xEE, 0xF4, 0xEF)
}
fn gate_grad_mid() -> Color32 {
    Color32::from_rgb(0xDF, 0xEA, 0xE3)
}
fn gate_grad_bot() -> Color32 {
    Color32::from_rgb(0xC9, 0xDB, 0xD0)
}
fn gate_ink() -> Color32 {
    Color32::from_rgb(0x14, 0x22, 0x1C)
}
fn gate_ink_soft() -> Color32 {
    Color32::from_rgb(0x2A, 0x3D, 0x34)
}
fn gate_muted() -> Color32 {
    Color32::from_rgb(0x5C, 0x6F, 0x66)
}
fn gate_accent() -> Color32 {
    Color32::from_rgb(0x2F, 0x5D, 0x4A)
}
fn gate_accent_mid() -> Color32 {
    Color32::from_rgb(0x3A, 0x6F, 0x59)
}
fn gate_accent_deep() -> Color32 {
    Color32::from_rgb(0x1E, 0x3F, 0x32)
}
fn gate_accent_soft() -> Color32 {
    Color32::from_rgb(0xD8, 0xEB, 0xE1)
}
fn gate_line() -> Color32 {
    Color32::from_rgba_unmultiplied(0x14, 0x22, 0x1C, 31) // ~0.12
}
fn gate_warn() -> Color32 {
    Color32::from_rgb(0xB4, 0x23, 0x18)
}
fn gate_field_bg() -> Color32 {
    Color32::from_rgb(0xFB, 0xFC, 0xFB)
}
fn gate_user_idle() -> Color32 {
    Color32::from_rgb(0xFA, 0xFC, 0xFB)
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        lerp_u8(a.r(), b.r(), t),
        lerp_u8(a.g(), b.g(), t),
        lerp_u8(a.b(), b.b(), t),
        lerp_u8(a.a(), b.a(), t),
    )
}

/// 强制登录门浅色松针绿控件色，避免深色/腮红主题污染密码框与勾选。
fn apply_gate_visuals(ctx: &egui::Context) {
    let mut visuals = Visuals::light();
    visuals.dark_mode = false;
    visuals.window_fill = gate_paper();
    visuals.panel_fill = gate_paper();
    visuals.extreme_bg_color = gate_field_bg();
    visuals.faint_bg_color = gate_accent_soft();
    visuals.override_text_color = None;
    visuals.hyperlink_color = gate_accent();
    visuals.warn_fg_color = gate_warn();
    visuals.error_fg_color = gate_warn();
    visuals.window_stroke = Stroke::new(1.0, gate_line());
    visuals.selection.bg_fill = gate_accent_soft();
    visuals.selection.stroke = Stroke::new(1.0, gate_accent());

    let round = egui::Rounding::same(10.0);
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.rounding = round;
        w.fg_stroke = Stroke::new(1.0, gate_ink());
    }
    visuals.widgets.noninteractive.bg_fill = Color32::WHITE;
    visuals.widgets.noninteractive.weak_bg_fill = gate_paper();
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, gate_line());
    visuals.widgets.inactive.bg_fill = gate_field_bg();
    visuals.widgets.inactive.weak_bg_fill = gate_paper();
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, gate_line());
    visuals.widgets.hovered.bg_fill = gate_accent_soft();
    visuals.widgets.hovered.weak_bg_fill = gate_paper();
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, gate_accent());
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, gate_ink());
    visuals.widgets.active.bg_fill = gate_accent();
    visuals.widgets.active.weak_bg_fill = gate_accent_soft();
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, gate_accent_deep());
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.open.bg_fill = gate_accent_soft();
    visuals.widgets.open.weak_bg_fill = gate_paper();
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, gate_accent());
    ctx.set_visuals(visuals);
}

fn paint_soft_glow(
    p: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    rgb: (u8, u8, u8),
    peak_alpha: u8,
) {
    const LAYERS: i32 = 14;
    for i in (0..LAYERS).rev() {
        let t = i as f32 / (LAYERS - 1) as f32;
        let r = radius * (0.18 + 0.82 * t);
        let a = ((1.0 - t).powf(1.35) * peak_alpha as f32) as u8;
        if a < 2 {
            continue;
        }
        p.circle_filled(
            center,
            r,
            Color32::from_rgba_unmultiplied(rgb.0, rgb.1, rgb.2, a),
        );
    }
}

fn paint_gate_bg(ui: &egui::Ui, wide: bool) {
    let rect = ui.ctx().screen_rect();
    let p = ui.painter();

    // 145° 雾绿底：纵向色带逼近 HTML linear-gradient
    const BANDS: i32 = 40;
    for i in 0..BANDS {
        let t0 = i as f32 / BANDS as f32;
        let t1 = (i + 1) as f32 / BANDS as f32;
        let c = if t0 < 0.42 {
            lerp_color(gate_grad_top(), gate_grad_mid(), t0 / 0.42)
        } else {
            lerp_color(gate_grad_mid(), gate_grad_bot(), (t0 - 0.42) / 0.58)
        };
        let y0 = rect.top() + rect.height() * t0;
        let y1 = rect.top() + rect.height() * t1 + 0.5;
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), y0),
                egui::pos2(rect.right(), y1),
            ),
            0.0,
            c,
        );
    }

    // 左上 / 右下径向光晕（对齐 HTML radial-gradient）
    paint_soft_glow(
        p,
        egui::pos2(
            rect.left() + rect.width() * 0.12,
            rect.top() + rect.height() * 0.18,
        ),
        rect.width().max(rect.height()) * 0.42,
        (0x2F, 0x5D, 0x4A),
        58,
    );
    paint_soft_glow(
        p,
        egui::pos2(
            rect.left() + rect.width() * 0.78,
            rect.top() + rect.height() * 0.88,
        ),
        rect.width().max(rect.height()) * 0.36,
        (0x1E, 0x3F, 0x32),
        42,
    );

    // 左侧淡网格（48px，低透明）
    let grid = Color32::from_rgba_unmultiplied(0x14, 0x22, 0x1C, 9);
    let step = 48.0;
    let grid_right = rect.left() + rect.width() * if wide { 0.58 } else { 1.0 };
    let mut x = rect.left();
    while x < grid_right {
        let fade = 1.0 - ((x - rect.left()) / (grid_right - rect.left() + 1.0)).clamp(0.0, 1.0);
        let a = (9.0 * fade * 0.9) as u8;
        if a >= 2 {
            p.vline(
                x,
                rect.y_range(),
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x14, 0x22, 0x1C, a)),
            );
        }
        x += step;
        let _ = grid;
    }
    let mut y = rect.top();
    while y < rect.bottom() {
        p.hline(
            rect.left()..=grid_right,
            y,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x14, 0x22, 0x1C, 7)),
        );
        y += step;
    }

    // 宽屏右侧磨砂面板（白 → 纸绿）
    if wide {
        let split = rect.left() + rect.width() * 0.55;
        const RB: i32 = 24;
        for i in 0..RB {
            let t0 = i as f32 / RB as f32;
            let t1 = (i + 1) as f32 / RB as f32;
            let c = lerp_color(
                Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 184),
                Color32::from_rgba_unmultiplied(0xF3, 0xF6, 0xF2, 235),
                t0,
            );
            let y0 = rect.top() + rect.height() * t0;
            let y1 = rect.top() + rect.height() * t1 + 0.5;
            p.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(split, y0),
                    egui::pos2(rect.right(), y1),
                ),
                0.0,
                c,
            );
        }
        p.vline(
            split,
            rect.y_range(),
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x14, 0x22, 0x1C, 15)),
        );
    }
}

/// 登录卡内密码框：纸白底 + 松针描边/聚焦光环。
fn gate_password_field(
    ui: &mut egui::Ui,
    password: &mut String,
    hint: &str,
    width: f32,
    height: f32,
) -> egui::Response {
    let size = Vec2::new(width, height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let pad_x = 14.0_f32;
    let round = egui::Rounding::same(11.0);
    let id = ui.make_persistent_id(("gate_pw", hint));
    let focused = ui.memory(|m| m.has_focus(id));

    ui.painter().rect(
        rect,
        round,
        if focused {
            Color32::WHITE
        } else {
            gate_field_bg()
        },
        Stroke::new(
            if focused { 1.25 } else { 1.0 },
            if focused { gate_accent() } else { gate_line() },
        ),
    );
    if focused {
        ui.painter().rect_stroke(
            rect.expand(2.5),
            egui::Rounding::same(13.0),
            Stroke::new(3.0, Color32::from_rgba_unmultiplied(0x2F, 0x5D, 0x4A, 40)),
        );
    }

    let response = ui.put(
        rect,
        egui::TextEdit::singleline(password)
            .id(id)
            .password(true)
            .frame(false)
            .vertical_align(Align::Center)
            .horizontal_align(Align::LEFT)
            .margin(Margin::symmetric(pad_x, 0.0))
            .min_size(size)
            .desired_width(width)
            .text_color(gate_ink()),
    );
    if password.is_empty() && !hint.is_empty() && !response.has_focus() {
        let font = FontId::proportional(14.0);
        let galley = ui.fonts(|f| f.layout_no_wrap(hint.to_owned(), font, gate_muted()));
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + pad_x, rect.top()),
            Vec2::new((rect.width() - pad_x * 2.0).max(1.0), rect.height()),
        );
        let pos = Align2::LEFT_CENTER
            .align_size_within_rect(galley.size(), text_rect)
            .min;
        ui.painter().galley(pos, galley, gate_muted());
    }
    response
}

const PROMISE_CHIPS: &[&str] = &["本地加密", "绿色单文件", "换机可认领"];

/// 左侧品牌主区：产品名是首屏主信号，承诺三词，无能力清单。
fn draw_brand_showcase(ui: &mut egui::Ui, wide: bool, compact: bool) {
    let title_size = if wide && !compact {
        42.0
    } else if wide {
        34.0
    } else {
        28.0
    };
    // 完整品牌 PNG（含字），需足够大；勿再套绿底小徽标
    let icon = if wide && !compact {
        120.0
    } else if wide {
        96.0
    } else {
        80.0
    };

    let draw_body = |ui: &mut egui::Ui| {
        Frame::none()
            .shadow(egui::epaint::Shadow {
                offset: Vec2::new(0.0, 10.0),
                blur: 22.0,
                spread: 0.0,
                color: Color32::from_rgba_unmultiplied(0x2F, 0x5D, 0x4A, 70),
            })
            .show(ui, |ui| {
                app_icon::show(ui, icon);
            });
        ui.add_space(if compact { 18.0 } else { 28.0 });
        ui.label(
            RichText::new(theme::APP_NAME)
                .size(title_size)
                .strong()
                .color(gate_ink()),
        );
        ui.add_space(12.0);
        ui.label(
            RichText::new("任务、备忘与私密资料，本机加密存放。需要时再授权局域网同步。")
                .size(if compact { 13.5 } else { 16.0 })
                .color(gate_ink_soft()),
        );
        ui.add_space(if compact { 16.0 } else { 24.0 });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.spacing_mut().item_spacing.y = 8.0;
            for chip in PROMISE_CHIPS {
                Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 148))
                    .stroke(Stroke::new(
                        1.0,
                        Color32::from_rgba_unmultiplied(0x2F, 0x5D, 0x4A, 41),
                    ))
                    .rounding(egui::Rounding::same(99.0))
                    .inner_margin(Margin::symmetric(12.0, 7.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(*chip)
                                .size(12.5)
                                .strong()
                                .color(gate_accent_deep()),
                        );
                    });
            }
        });
    };

    if wide {
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(if compact { 12.0 } else { 20.0 });
            ui.label(
                RichText::new("当前版本功能全开 · 无云账号绑架")
                    .size(12.5)
                    .color(gate_muted()),
            );
            ui.with_layout(Layout::top_down(Align::Min), draw_body);
        });
    } else {
        draw_body(ui);
        ui.add_space(12.0);
        ui.label(
            RichText::new("当前版本功能全开 · 无云账号绑架")
                .size(12.0)
                .color(gate_muted()),
        );
    }
}

/// 打赏码块尺寸（含白边与文案），供右下角绝对定位。
fn tip_corner_size(compact: bool) -> (f32, f32, f32) {
    let qr: f32 = if compact { 96.0 } else { 112.0 };
    let frame_pad = 10.0;
    let w = qr + frame_pad * 2.0;
    let h = qr + frame_pad * 2.0 + 8.0 + 36.0;
    (qr, w, h)
}

/// 登录页右下角打赏码；点击打开大图弹窗。
fn draw_gate_tip_corner(ui: &mut egui::Ui, compact: bool, show_tip: &mut bool) {
    let (qr, _, _) = tip_corner_size(compact);
    let frame = Frame::none()
        .fill(Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 235))
        .stroke(Stroke::new(1.0, gate_line()))
        .rounding(egui::Rounding::same(14.0))
        .inner_margin(egui::Margin::same(10.0))
        .shadow(egui::epaint::Shadow {
            offset: Vec2::new(0.0, 8.0),
            blur: 20.0,
            spread: 0.0,
            color: Color32::from_black_alpha(28),
        });
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        let r = frame
            .show(ui, |ui| {
                tip_code::show(ui, qr);
                ui.add_space(8.0);
                ui.label(
                    RichText::new("打赏作者")
                        .size(12.5)
                        .strong()
                        .color(gate_ink_soft()),
                );
                ui.label(
                    RichText::new("自愿 · 请喝杯咖啡")
                        .size(11.0)
                        .color(gate_muted()),
                );
            })
            .response
            .interact(Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("点击放大");
        if r.clicked() {
            *show_tip = true;
        }
    });
}

/// 贴在面板真正的右下角绘制打赏码。
fn place_tip_bottom_right(ui: &mut egui::Ui, compact: bool, show_tip: &mut bool) {
    let (_, tip_w, tip_h) = tip_corner_size(compact);
    let margin = 24.0;
    let panel = ui.max_rect();
    let tip_rect = egui::Rect::from_min_size(
        egui::pos2(
            panel.max.x - tip_w - margin,
            panel.max.y - tip_h - margin,
        ),
        Vec2::new(tip_w, tip_h),
    );
    ui.allocate_ui_at_rect(tip_rect, |ui| {
        ui.set_clip_rect(tip_rect);
        draw_gate_tip_corner(ui, compact, show_tip);
    });
}

/// 登录卡下方：版本 + 使用说明。
fn draw_right_meta(ui: &mut egui::Ui, show_help: &mut bool) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("版本 {}", theme::APP_FILE_VERSION))
                .size(12.0)
                .color(gate_muted()),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .link(
                    RichText::new("使用说明")
                        .size(12.5)
                        .strong()
                        .color(gate_accent_deep()),
                )
                .clicked()
            {
                *show_help = true;
            }
        });
    });
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(11.5)
            .color(gate_ink_soft())
            .strong(),
    );
    ui.add_space(3.0);
}

fn mac_status(ui: &mut egui::Ui, status: &str, busy: bool) {
    if status.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new(status)
            .size(12.5)
            .color(if busy { gate_muted() } else { gate_warn() }),
    );
}

/// 路径输入 + 固定宽按钮，避免 horizontal 互相抢宽溢出。
fn path_picker_row(ui: &mut egui::Ui, path: &mut String, hint: &str, btn: &str) -> bool {
    const BTN_W: f32 = 88.0;
    const GAP: f32 = 8.0;
    let mut clicked = false;
    ui.horizontal(|ui| {
        let path_w = (ui.available_width() - BTN_W - GAP).max(1.0);
        ui.add(
            egui::TextEdit::singleline(path)
                .desired_width(path_w)
                .hint_text(theme::hint(hint))
                .margin(egui::Margin::symmetric(10.0, 8.0)),
        );
        ui.add_space(GAP);
        if ui
            .add_sized(
                [BTN_W, 32.0],
                egui::Button::new(RichText::new(btn).size(13.0).color(theme::mac_text()))
                    .fill(theme::mac_fill())
                    .rounding(egui::Rounding::same(theme::MAC_ROUND_CTRL)),
            )
            .clicked()
        {
            clicked = true;
        }
    });
    clicked
}

/// 绘制身份门；若需后台解锁返回 `Unlock`。
pub fn show(
    ui_ctx: &egui::Context,
    cfg: &mut Config,
    st: &mut IdentityGateState,
) -> GateAction {
    let mut action = GateAction::None;
    let mut show_help = false;
    let mut show_tip = false;

    apply_gate_visuals(ui_ctx);

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(gate_grad_mid()))
        .show(ui_ctx, |ui| {
            let select_mode =
                !st.legacy_wipe_confirm && !st.show_init && st.pending_identity.is_none();
            let avail = ui.available_size();
            // 默认最小窗 960×720：走宽屏紧凑
            let wide = avail.x >= 900.0;
            let compact = avail.y <= 680.0;
            paint_gate_bg(ui, wide);

            if wide {
                let pad = 40.0;
                // 品牌略宽，登录卡收窄（对齐 login.html 约 340 内容宽）
                let left_w = (avail.x * 0.52).clamp(340.0, 560.0);
                let gap = 28.0;
                let right_w = (avail.x - left_w - gap - pad * 2.0).max(300.0);
                let col_h = (avail.y - 12.0).max(200.0);
                const CARD_MAX: f32 = 340.0;
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_w, col_h),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.add_space(if compact { 28.0 } else { 48.0 });
                            draw_brand_showcase(ui, true, compact);
                        },
                    );
                    ui.add_space(gap);
                    ui.allocate_ui_with_layout(
                        Vec2::new(right_w, col_h),
                        Layout::top_down(Align::Center),
                        |ui| {
                            ui.set_max_width(right_w);
                            let top = if compact { 40.0 } else { 72.0 };
                            ui.add_space(top);
                            ui.set_max_width(right_w.min(CARD_MAX));
                            draw_gate_card(ui, cfg, st, &mut action, select_mode);
                            ui.set_max_width(right_w.min(CARD_MAX));
                            draw_right_meta(ui, &mut show_help);
                        },
                    );
                });
                place_tip_bottom_right(ui, compact, &mut show_tip);
            } else {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| {
                            ui.set_max_width(avail.x.min(340.0).max(280.0));
                            draw_brand_showcase(ui, false, true);
                            ui.add_space(16.0);
                            draw_gate_card(ui, cfg, st, &mut action, select_mode);
                            draw_right_meta(ui, &mut show_help);
                            let (_, _, tip_h) = tip_corner_size(true);
                            ui.add_space(tip_h + 28.0);
                        });
                    });
                place_tip_bottom_right(ui, true, &mut show_tip);
            }
        });

    if show_help {
        GateAction::ShowHelp
    } else if show_tip {
        GateAction::ShowTip
    } else {
        action
    }
}

/// 右侧白卡片：选择用户 / 创建导入 / 旧版清理。
fn draw_gate_card(
    ui: &mut egui::Ui,
    cfg: &mut Config,
    st: &mut IdentityGateState,
    action: &mut GateAction,
    select_mode: bool,
) {
    let card_w = ui.available_width().min(340.0);
    Frame::none()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x14, 0x22, 0x1C, 15)))
        .rounding(egui::Rounding::same(20.0))
        .inner_margin(Margin::symmetric(22.0, 20.0))
        .shadow(egui::epaint::Shadow {
            offset: Vec2::new(0.0, 22.0),
            blur: 48.0,
            spread: 0.0,
            // HTML: 0 28px 64px rgba(20,34,28,0.14)
            color: Color32::from_rgba_unmultiplied(0x14, 0x22, 0x1C, 36),
        })
        .show(ui, |ui| {
            ui.set_width(card_w);
            if select_mode {
                draw_select(ui, cfg, st, action);
                return;
            }
            if st.legacy_wipe_confirm {
                draw_legacy(ui, cfg, st);
            } else if st.show_init || st.pending_identity.is_some() {
                if st.pending_identity.is_none() {
                    if let Some(a) = draw_init_create(ui, cfg, st) {
                        *action = a;
                    }
                } else if let Some(a) = draw_init_export(ui, cfg, st) {
                    *action = a;
                }
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    if ui
                        .link(
                            RichText::new("选择已有身份")
                                .size(13.0)
                                .strong()
                                .color(gate_accent()),
                        )
                        .clicked()
                    {
                        st.show_init = false;
                        st.pending_identity = None;
                        st.refresh_list(cfg);
                    }
                });
            }
            if !select_mode {
                mac_status(ui, &st.status, st.busy);
            }
        });
}

fn draw_legacy(ui: &mut egui::Ui, cfg: &mut Config, st: &mut IdentityGateState) {
    ui.label(
        RichText::new("需要重新初始化")
            .size(17.0)
            .strong()
            .color(theme::mac_text()),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new("检测到旧版数据目录，与当前身份模型不兼容。请先自行备份，再清空并继续。")
            .size(13.0)
            .color(theme::mac_text_secondary()),
    );
    ui.add_space(18.0);
    if theme::mac_primary_button(ui, "我已备份，清空并继续", !st.busy).clicked() {
        let dir = PathBuf::from(&cfg.data_dir);
        match identity_keys::wipe_data_dir(&dir) {
            Ok(()) => {
                st.legacy_wipe_confirm = false;
                st.show_init = true;
                st.status = "已清空，请生成或导入密钥对".into();
            }
            Err(e) => st.status = e.to_string(),
        }
    }
}

fn draw_init_create(
    ui: &mut egui::Ui,
    cfg: &mut Config,
    st: &mut IdentityGateState,
) -> Option<GateAction> {
    ui.label(
        RichText::new("创建身份")
            .size(17.0)
            .strong()
            .color(theme::mac_text()),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(if st.init_mode_import {
            "从备份导入。名称取自密钥包；已有备份无需再导出。"
        } else {
            "生成新密钥对。新身份须导出备份后再进入。"
        })
        .size(13.0)
        .color(theme::mac_text_secondary()),
    );
    ui.add_space(14.0);

    theme::mac_segmented(ui, "生成新密钥", "导入备份", &mut st.init_mode_import);
    ui.add_space(16.0);

    if !st.init_mode_import {
        field_label(ui, "显示别名");
        ui.add(
            egui::TextEdit::singleline(&mut st.init_alias)
                .desired_width(ui.available_width())
                .hint_text(theme::hint("例如：张三"))
                .margin(egui::Margin::symmetric(12.0, 8.0)),
        );
        ui.add_space(12.0);
    }

    #[cfg(windows)]
    {
        field_label(ui, "数据盘");
        egui::ComboBox::from_id_source("mac_drive")
            .width(ui.available_width())
            .selected_text(st.init_drive.as_str())
            .show_ui(ui, |ui| {
                for d in config::list_drives() {
                    ui.selectable_value(&mut st.init_drive, d.clone(), d);
                }
            });
        ui.add_space(12.0);
    }

    if st.init_mode_import {
        field_label(ui, "密钥文件 (.memokey)");
        if path_picker_row(
            ui,
            &mut st.init_import_path,
            "选择 .memokey 文件…",
            "选择…",
        ) {
            if let Some(p) = save_dialog::open_memokey_dialog() {
                st.init_import_path = p.to_string_lossy().into_owned();
                st.status.clear();
            }
        }
        let import_path = PathBuf::from(st.init_import_path.trim());
        if let Some(name) = IdentityKeys::peek_alias(import_path.as_path()) {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("将使用包内名称：{name}"))
                    .size(12.5)
                    .color(theme::mac_text_secondary()),
            );
        }
        let enc = IdentityKeys::peek_encrypted(import_path.as_path()).unwrap_or(false);
        ui.add_space(10.0);
        if enc {
            field_label(ui, "旧版加密包口令");
            theme::password_field(
                ui,
                &mut st.init_legacy_pack_pass,
                "仅旧版加密 .memokey 需要",
                ui.available_width(),
                34.0,
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("新版密钥包无导出密码；此框仅用于兼容旧版加密包。")
                    .size(11.5)
                    .color(theme::mac_text_secondary()),
            );
            ui.add_space(10.0);
        }
        local_password_fields(ui, &mut st.init_local_pass, &mut st.init_local_pass2, true, false);
        ui.add_space(8.0);
    }

    ui.add_space(8.0);
    let btn = if st.init_mode_import {
        "导入并进入"
    } else {
        "继续"
    };
    if theme::mac_primary_button(ui, btn, !st.busy).clicked() {
        if st.init_mode_import {
            let path = st.init_import_path.trim();
            if path.is_empty() {
                st.status = "请选择密钥文件".into();
                return None;
            }
            let p = PathBuf::from(path);
            let enc = IdentityKeys::peek_encrypted(p.as_path()).unwrap_or(false);
            if let Err(e) = identity_keys::validate_local_password(
                &st.init_local_pass,
                &st.init_local_pass2,
            ) {
                st.status = e.to_string();
                return None;
            }
            let local_pw = st.init_local_pass.trim().to_string();
            let import_pass = if enc {
                let legacy = st.init_legacy_pack_pass.trim();
                if legacy.is_empty() {
                    st.status = "请输入旧版加密包口令".into();
                    return None;
                }
                legacy
            } else {
                ""
            };
            match IdentityKeys::import_file(p.as_path(), import_pass) {
                Ok(id) => {
                    cfg.data_dir = config::memo_data_path(&st.init_drive, &cfg.node_id)
                        .to_string_lossy()
                        .into_owned();
                    let _ = config::save_settings(cfg);
                    st.busy = true;
                    st.status = format!("正在以「{}」打开…", id.alias);
                    return Some(GateAction::Unlock {
                        cfg: cfg.clone(),
                        identity: id,
                        export_path: None,
                        export_pass: String::new(),
                        local_password: local_pw,
                    });
                }
                Err(e) => st.status = e.to_string(),
            }
        } else {
            let alias = st.init_alias.trim();
            if alias.is_empty() {
                st.status = "请填写显示别名".into();
            } else {
                let id = IdentityKeys::generate(alias);
                cfg.data_dir = config::memo_data_path(&st.init_drive, &cfg.node_id)
                    .to_string_lossy()
                    .into_owned();
                let _ = config::save_settings(cfg);
                st.pending_identity = Some(id);
                st.init_local_pass.clear();
                st.init_local_pass2.clear();
                st.status = String::new();
            }
        }
    }
    None
}

fn draw_init_export(
    ui: &mut egui::Ui,
    cfg: &Config,
    st: &mut IdentityGateState,
) -> Option<GateAction> {
    ui.label(
        RichText::new("备份密钥（必做）")
            .size(17.0)
            .strong()
            .color(theme::mac_text()),
    );
    ui.add_space(4.0);
    let name = st
        .pending_identity
        .as_ref()
        .map(|id| id.alias.as_str())
        .unwrap_or("");
    ui.label(
        RichText::new(format!(
            "新身份「{name}」须先导出密钥包到安全位置。丢失后无法恢复私人数据。"
        ))
        .size(13.0)
        .color(theme::mac_text_secondary()),
    );
    ui.add_space(16.0);

    field_label(ui, "导出路径");
    if path_picker_row(
        ui,
        &mut st.init_export_path,
        "选择导出位置…",
        "选择位置",
    ) {
        let default_name = default_memokey_name(st);
        if let Some(p) = save_dialog::save_memokey_dialog(&default_name) {
            st.init_export_path = p.to_string_lossy().into_owned();
            st.status.clear();
        }
    }
    ui.add_space(12.0);
    local_password_fields(ui, &mut st.init_local_pass, &mut st.init_local_pass2, true, false);
    ui.label(
        RichText::new("密钥包无导出密码（含签名防篡改）；请把文件存到安全位置。上方密码仅用于本机登录。")
            .size(11.5)
            .color(theme::mac_text_secondary()),
    );
    ui.add_space(16.0);

    let label = if st.busy {
        "正在打开…"
    } else if st.init_export_path.trim().is_empty() {
        "选择位置并进入"
    } else {
        "导出并进入"
    };
    if theme::mac_primary_button(ui, label, !st.busy).clicked() {
        if let Err(e) =
            identity_keys::validate_local_password(&st.init_local_pass, &st.init_local_pass2)
        {
            st.status = e.to_string();
            return None;
        }
        let local_pw = st.init_local_pass.trim().to_string();
        let mut export_path = st.init_export_path.trim().to_string();
        if export_path.is_empty() {
            let default_name = default_memokey_name(st);
            match save_dialog::save_memokey_dialog(&default_name) {
                Some(p) => {
                    export_path = p.to_string_lossy().into_owned();
                    st.init_export_path = export_path.clone();
                }
                None => {
                    st.status = "请选择导出位置后再继续".into();
                    return None;
                }
            }
        }
        if let Some(id) = st.pending_identity.clone() {
            let p = PathBuf::from(&export_path);
            if p.is_dir() {
                st.status = "请选择具体文件名，而不是文件夹".into();
            } else {
                st.busy = true;
                st.status = "正在导出并打开…".into();
                return Some(GateAction::Unlock {
                    cfg: cfg.clone(),
                    identity: id,
                    export_path: Some(export_path),
                    export_pass: String::new(),
                    local_password: local_pw,
                });
            }
        }
    }
    None
}

fn default_memokey_name(st: &IdentityGateState) -> String {
    let alias = st
        .pending_identity
        .as_ref()
        .map(|id| id.alias.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("memo");
    let safe: String = alias
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!("{safe}.memokey")
}

fn draw_select(
    ui: &mut egui::Ui,
    cfg: &mut Config,
    st: &mut IdentityGateState,
    action: &mut GateAction,
) {
    let accent = gate_accent();
    let data = PathBuf::from(&cfg.data_dir);
    let needs_setup = st
        .selected
        .as_ref()
        .map(|fp| IdentityKeys::needs_password_setup(&data, fp))
        .unwrap_or(false);

    ui.label(
        RichText::new("选择用户")
            .size(20.0)
            .strong()
            .color(gate_ink()),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new("每位用户拥有独立的备忘与私密空间")
            .size(12.5)
            .color(gate_muted()),
    );
    ui.add_space(12.0);

    egui::ScrollArea::vertical()
        .max_height(132.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if st.identities.is_empty() {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("暂无已保存的用户").size(12.5).color(gate_muted()));
                });
                ui.add_space(10.0);
                return;
            }
            for (idx, m) in st.identities.iter().enumerate() {
                let sel = st.selected.as_deref() == Some(m.fingerprint.as_str());
                let fill = if sel {
                    gate_accent_soft()
                } else {
                    gate_user_idle()
                };
                let stroke = if sel {
                    Stroke::new(1.5, accent)
                } else {
                    Stroke::new(1.0, gate_line())
                };
                let alias = if m.alias.is_empty() {
                    "未命名"
                } else {
                    m.alias.as_str()
                };
                let letter = alias
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_else(|| "?".into());
                let sub = if idx == 0 {
                    "本机身份 · 最近使用"
                } else {
                    "本机身份"
                };

                Frame::none()
                    .fill(fill)
                    .stroke(stroke)
                    .rounding(egui::Rounding::same(10.0))
                    .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                    .show(ui, |ui| {
                        let resp = ui.allocate_response(
                            Vec2::new(ui.available_width(), 36.0),
                            Sense::click(),
                        );
                        let painter = ui.painter();
                        let r = resp.rect;
                        let av_r = 13.0;
                        let av_c = egui::pos2(r.left() + av_r + 1.0, r.center().y);
                        painter.circle_filled(av_c, av_r, Color32::WHITE);
                        painter.circle_stroke(
                            av_c,
                            av_r,
                            Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x2F, 0x5D, 0x4A, 46)),
                        );
                        painter.text(
                            av_c,
                            egui::Align2::CENTER_CENTER,
                            &letter,
                            egui::FontId::proportional(12.5),
                            gate_accent_deep(),
                        );
                        let text_x = r.left() + av_r * 2.0 + 12.0;
                        painter.text(
                            egui::pos2(text_x, r.center().y - 7.0),
                            egui::Align2::LEFT_CENTER,
                            alias,
                            egui::FontId::proportional(13.5),
                            gate_ink(),
                        );
                        painter.text(
                            egui::pos2(text_x, r.center().y + 9.0),
                            egui::Align2::LEFT_CENTER,
                            sub,
                            egui::FontId::proportional(11.0),
                            gate_muted(),
                        );
                        if resp.clicked() {
                            st.selected = Some(m.fingerprint.clone());
                            st.select_pass.clear();
                            st.select_pass2.clear();
                            st.status.clear();
                            st.focus_select_pass = true;
                        }
                    });
                ui.add_space(6.0);
            }
        });

    ui.add_space(10.0);
    if st.selected.is_some() {
        if needs_setup {
            ui.label(
                RichText::new("此身份尚未设置登录密码，请先设置后再进入。")
                    .size(12.0)
                    .color(Color32::from_rgb(0xB4, 0x23, 0x18)),
            );
            ui.add_space(6.0);
        }
        let focus = st.focus_select_pass;
        let enter_submit = local_password_fields(
            ui,
            &mut st.select_pass,
            &mut st.select_pass2,
            needs_setup,
            focus,
        );
        if focus {
            st.focus_select_pass = false;
        }

        ui.add_space(8.0);
        ui.add_enabled_ui(autostart::supported(), |ui| {
            let tip = if autostart::supported() {
                "与设置中的「开机自启动」同步；开机后托盘启动，仍需解锁身份"
            } else {
                "当前系统不支持写入开机启动项"
            };
            if ui
                .checkbox(&mut cfg.start_on_boot, "开机自启动")
                .on_hover_text(tip)
                .changed()
            {
                let want = cfg.start_on_boot;
                match autostart::apply_and_persist(cfg, want) {
                    Ok(msg) => st.status = msg.into(),
                    Err(e) => {
                        cfg.start_on_boot = !want;
                        st.status = format!("开机自启动设置失败: {e}");
                    }
                }
            }
        });

        ui.add_space(10.0);
        let can = !st.busy;
        let btn_label = if st.busy {
            "打开中…"
        } else if needs_setup {
            "设置密码并进入"
        } else {
            "进入"
        };
        let text = RichText::new(btn_label)
            .color(Color32::WHITE)
            .strong()
            .size(14.0);
        let w = ui.available_width().clamp(120.0, 340.0);
        let btn_clicked = ui
            .add_enabled(
                can,
                egui::Button::new(text)
                    .fill(gate_accent_mid())
                    .stroke(Stroke::new(1.0, gate_accent_deep()))
                    .rounding(egui::Rounding::same(12.0))
                    .min_size(Vec2::new(w, 40.0)),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked();
        if (btn_clicked || enter_submit) && can {
            if let Some(fp) = st.selected.clone() {
                let local_pw = if needs_setup {
                    match identity_keys::validate_local_password(
                        &st.select_pass,
                        &st.select_pass2,
                    ) {
                        Ok(()) => st.select_pass.trim().to_string(),
                        Err(e) => {
                            st.status = e.to_string();
                            return;
                        }
                    }
                } else {
                    let p = st.select_pass.trim().to_string();
                    if p.is_empty() {
                        st.status = "请输入登录密码".into();
                        return;
                    }
                    p
                };
                let load_pw = if needs_setup { "" } else { local_pw.as_str() };
                match IdentityKeys::load(data.as_path(), &fp, load_pw) {
                    Ok(id) => {
                        st.busy = true;
                        st.status = "正在打开…".into();
                        *action = GateAction::Unlock {
                            cfg: cfg.clone(),
                            identity: id,
                            export_path: None,
                            export_pass: String::new(),
                            local_password: local_pw,
                        };
                    }
                    Err(e) => st.status = e.to_string(),
                }
            }
        }
    } else {
        ui.label(
            RichText::new("请先选择一位用户")
                .size(12.5)
                .color(gate_muted()),
        );
    }

    if !st.status.is_empty() {
        ui.add_space(6.0);
        mac_status(ui, &st.status, st.busy);
    }

    ui.add_space(10.0);
    ui.vertical_centered(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if ui
                .link(
                    RichText::new("生成身份")
                        .size(12.5)
                        .strong()
                        .color(gate_accent()),
                )
                .clicked()
            {
                st.show_init = true;
                st.init_mode_import = false;
                st.status.clear();
            }
            if ui
                .link(
                    RichText::new("导入密钥")
                        .size(12.5)
                        .strong()
                        .color(gate_accent()),
                )
                .clicked()
            {
                st.show_init = true;
                st.init_mode_import = true;
                st.status.clear();
            }
        });
    });
}
