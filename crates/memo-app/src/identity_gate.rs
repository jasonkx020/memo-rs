//! 启动身份门：选择/生成/导入密钥对（macOS 风格设置助手）。

use crate::app_icon;
use crate::autostart;
use crate::save_dialog;
use crate::theme;
use crate::tip_code;
use eframe::egui::{self, Align, Color32, Frame, Layout, RichText, Sense, Stroke, Vec2};
use memo_core::config::{self, Config};
use memo_core::identity_keys::{self, IdentityKeys, IdentityMeta};
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
    /// 登录主按钮下方：开机自启动 / 生成·导入身份
    pub show_more_options: bool,
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
            show_more_options: false,
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
        let r = theme::password_field(ui, pass, "设置登录密码", w, 34.0);
        if focus_first {
            r.request_focus();
        }
        let mut submit = enter(ui, &r);
        ui.add_space(8.0);
        field_label(ui, "确认登录密码");
        let r2 = theme::password_field(ui, pass2, "再输入一次", w, 34.0);
        submit |= enter(ui, &r2);
        ui.add_space(4.0);
        ui.label(
            RichText::new("用于进入本机并加密数据备份；.memokey 密钥包本身无口令（靠签名验真，请妥善保管文件）。")
                .size(11.5)
                .color(theme::mac_text_secondary()),
        );
        submit
    } else {
        field_label(ui, "登录密码");
        let r = theme::password_field(ui, pass, "输入登录密码", w, 34.0);
        if focus_first {
            r.request_focus();
        }
        enter(ui, &r)
    }
}

/// 登录门全屏黑底（广告/能力展示面）。
fn gate_black() -> Color32 {
    Color32::from_rgb(0x08, 0x08, 0x0A)
}

fn paint_gate_bg(ui: &egui::Ui) {
    let rect = ui.ctx().screen_rect();
    ui.painter().rect_filled(rect, 0.0, gate_black());
    // 左侧微光，避免死黑平板
    let glow = egui::Rect::from_min_max(
        rect.left_top(),
        egui::pos2(rect.left() + rect.width() * 0.55, rect.bottom()),
    );
    ui.painter().rect_filled(
        glow,
        0.0,
        Color32::from_rgba_unmultiplied(0x1A, 0x2A, 0x44, 40),
    );
}

/// 特色能力（登录页左侧营销）。
const CAPABILITY_ROWS: &[(&str, &str)] = &[
    ("任务日历", "日/周安排、到期提醒与过期摘要"),
    ("备忘归档", "账号、证件等长期资料本地保存"),
    ("私密空间", "独立加密隔离，不进普通列表"),
    ("局域网同步", "授权节点密文托管与公开同步"),
    ("桌面便签", "置顶小窗，即改即存回本条"),
    ("身份密钥", "换机导入 .memokey 即可认领"),
    ("关怀健康", "经期推算与体检提醒（可选）"),
    ("加密备份", "登录密码密封私钥与数据备份"),
];

/// 信任卖点：补足左侧空白。
const TRUST_LINES: &[&str] = &[
    "数据默认落本机，只有你授权才上局域网",
    "无云账号绑架：绿色单文件，开箱即用",
    "Ed25519 身份与审计链，换机不丢数据",
    "空闲自动锁定，Esc 一键回到选人页",
];

fn draw_feature_row(ui: &mut egui::Ui, title: &str, desc: &str, compact: bool) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 3.0, theme::shell_accent());
        ui.add_space(8.0);
        ui.label(
            RichText::new(title)
                .size(if compact { 12.5 } else { 13.5 })
                .strong()
                .color(Color32::WHITE),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(desc)
                .size(if compact { 11.5 } else { 12.5 })
                .color(Color32::from_rgb(0x8E, 0x8E, 0x93)),
        );
    });
}

/// 左侧品牌 + 特色功能 + 信任文案。宽屏钉住底部标语以吃满高度。
fn draw_capability_showcase(ui: &mut egui::Ui, wide: bool, compact: bool) {
    let title_size = if wide && !compact {
        28.0
    } else if wide {
        24.0
    } else {
        20.0
    };
    let icon = if wide && !compact {
        48.0
    } else if wide {
        40.0
    } else {
        32.0
    };
    let row_gap = if compact { 4.0 } else { 8.0 };
    let section_gap = if compact { 12.0 } else { 18.0 };

    let draw_body = |ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            app_icon::show(ui, icon);
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(theme::APP_NAME)
                        .size(title_size)
                        .strong()
                        .color(Color32::from_rgb(0xF5, 0xF5, 0xF7)),
                );
                ui.label(
                    RichText::new(theme::APP_DESCRIPTION)
                        .size(12.5)
                        .color(Color32::from_rgb(0xA1, 0xA1, 0xA6)),
                );
            });
        });
        ui.add_space(section_gap);
        if compact {
            ui.label(
                RichText::new("个人备忘与任务，本机加密存放；需要时再授权局域网同步。")
                    .size(13.0)
                    .color(Color32::from_rgb(0xD1, 0xD1, 0xD6)),
            );
        } else {
            ui.label(
                RichText::new("把任务、备忘与私密资料收进一个本地加密壳层——")
                    .size(14.5)
                    .color(Color32::from_rgb(0xD1, 0xD1, 0xD6)),
            );
            ui.label(
                RichText::new("需要同步时再授权节点，换机导入密钥即可认领数据。")
                    .size(14.5)
                    .color(Color32::from_rgb(0xD1, 0xD1, 0xD6)),
            );
        }
        ui.add_space(section_gap);
        ui.label(
            RichText::new("特色能力")
                .size(12.0)
                .strong()
                .color(Color32::from_rgb(0x64, 0xB5, 0xF6)),
        );
        ui.add_space(if compact { 6.0 } else { 10.0 });

        // 窄屏少列几条；宽屏（含 640 紧凑）打赏已在右侧，左侧可列全
        let n = if wide {
            CAPABILITY_ROWS.len()
        } else {
            5.min(CAPABILITY_ROWS.len())
        };
        for (title, desc) in &CAPABILITY_ROWS[..n] {
            draw_feature_row(ui, title, desc, compact);
            ui.add_space(row_gap);
        }

        ui.add_space(if compact { 6.0 } else { 10.0 });
        ui.label(
            RichText::new("为什么放心")
                .size(12.0)
                .strong()
                .color(Color32::from_rgb(0x64, 0xB5, 0xF6)),
        );
        ui.add_space(if compact { 6.0 } else { 8.0 });
        let trust_n = if !wide {
            2
        } else if compact {
            3
        } else {
            TRUST_LINES.len()
        };
        for line in &TRUST_LINES[..trust_n.min(TRUST_LINES.len())] {
            ui.label(
                RichText::new(format!("· {line}"))
                    .size(if compact { 11.5 } else { 12.5 })
                    .color(Color32::from_rgb(0xA1, 0xA1, 0xA6)),
            );
            ui.add_space(if compact { 3.0 } else { 5.0 });
        }
    };

    if wide {
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(if compact { 10.0 } else { 18.0 });
            ui.label(
                RichText::new("绿色单文件 · 开箱即用 · 当前版本功能全开")
                    .size(12.0)
                    .color(Color32::from_rgb(0x6E, 0x6E, 0x73)),
            );
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                draw_body(ui);
            });
        });
    } else {
        draw_body(ui);
    }
}

/// 打赏码块尺寸（含白边与文案），供右下角绝对定位。
fn tip_corner_size(compact: bool) -> (f32, f32, f32) {
    let qr: f32 = if compact { 144.0 } else { 192.0 };
    let frame_pad = 8.0;
    let w = qr + frame_pad * 2.0;
    // 码 + 间距 + 三行文案
    let h = qr + frame_pad * 2.0 + 6.0 + 48.0;
    (qr, w, h)
}

/// 登录页右下角打赏码；点击打开大图弹窗。
fn draw_gate_tip_corner(ui: &mut egui::Ui, compact: bool, show_tip: &mut bool) {
    let (qr, _, _) = tip_corner_size(compact);
    let frame = Frame::none()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, Color32::from_rgb(0x2A, 0x2A, 0x30)))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(8.0));
    ui.with_layout(Layout::top_down(Align::Max), |ui| {
        let r = frame
            .show(ui, |ui| {
                tip_code::show(ui, qr);
            })
            .response
            .interact(Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("点击放大");
        if r.clicked() {
            *show_tip = true;
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new("打赏作者")
                .size(12.5)
                .strong()
                .color(Color32::from_rgb(0xD1, 0xD1, 0xD6)),
        );
        ui.label(
            RichText::new("扫码请喝杯咖啡 · 纯属自愿")
                .size(11.0)
                .color(Color32::from_rgb(0x8E, 0x8E, 0x93)),
        );
        if ui
            .link(
                RichText::new("查看大图")
                    .size(11.5)
                    .color(Color32::from_rgb(0x64, 0xB5, 0xF6)),
            )
            .clicked()
        {
            *show_tip = true;
        }
    });
}

/// 贴在面板真正的右下角绘制打赏码。
fn place_tip_bottom_right(ui: &mut egui::Ui, compact: bool, show_tip: &mut bool) {
    let (_, tip_w, tip_h) = tip_corner_size(compact);
    let margin = 20.0;
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

/// 登录卡下方黑底区：能力芯片 + 完整版本号 / 说明，吃掉右侧空白。
fn draw_right_black_footer(ui: &mut egui::Ui, show_help: &mut bool) {
    ui.add_space(18.0);
    // 宽屏紧凑时左侧只列了前 4 条，这里补后两条；窄屏则展示前 4 条摘要
    let chips: &[&str] = if ui.available_width() >= 360.0 {
        &["桌面便签", "身份密钥", "本地加密", "可换机"]
    } else {
        &["任务日历", "备忘归档", "私密空间", "局域网同步"]
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.spacing_mut().item_spacing.y = 6.0;
        for title in chips {
            Frame::none()
                .fill(Color32::from_rgb(0x16, 0x16, 0x1A))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x2A, 0x2A, 0x30)))
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::symmetric(10.0, 5.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(*title)
                            .size(12.0)
                            .color(Color32::from_rgb(0xD1, 0xD1, 0xD6)),
                    );
                });
        }
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("版本 {}", theme::APP_FILE_VERSION))
                .size(12.5)
                .color(Color32::from_rgb(0xA1, 0xA1, 0xA6)),
        );
        ui.add_space(16.0);
        if ui
            .link(
                RichText::new("使用说明")
                    .size(12.5)
                    .color(Color32::from_rgb(0x64, 0xB5, 0xF6)),
            )
            .clicked()
        {
            *show_help = true;
        }
    });
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(12.0)
            .color(theme::mac_text_secondary())
            .strong(),
    );
    ui.add_space(4.0);
}

fn mac_status(ui: &mut egui::Ui, status: &str, busy: bool) {
    if status.is_empty() {
        return;
    }
    ui.add_space(10.0);
    ui.label(
        RichText::new(status)
            .size(12.5)
            .color(if busy {
                theme::mac_text_secondary()
            } else {
                theme::mac_red()
            }),
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

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(gate_black()))
        .show(ui_ctx, |ui| {
            paint_gate_bg(ui);
            let select_mode =
                !st.legacy_wipe_confirm && !st.show_init && st.pending_identity.is_none();
            let avail = ui.available_size();
            // 默认最小窗 960×640：走宽屏紧凑，保证版本号完整可见
            let wide = avail.x >= 900.0;
            let compact = avail.y <= 680.0;

            if wide {
                let pad = 28.0;
                let left_w = (avail.x * 0.42).clamp(300.0, 420.0);
                let gap = 24.0;
                let right_w = (avail.x - left_w - gap - pad * 2.0).max(380.0);
                let col_h = (avail.y - 16.0).max(200.0);
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_w, col_h),
                        Layout::top_down(Align::Min),
                        |ui| {
                            // 顶部留白交给 showcase 内 bottom_up，整列吃满营销文案
                            ui.add_space(if compact { 24.0 } else { 40.0 });
                            draw_capability_showcase(ui, true, compact);
                        },
                    );
                    ui.add_space(gap);
                    // 右侧：登录卡 + 页脚；二维码另贴整页右下角
                    ui.allocate_ui_with_layout(
                        Vec2::new(right_w, col_h),
                        Layout::top_down(Align::Center),
                        |ui| {
                            ui.set_max_width(right_w);
                            let top = if compact { 28.0 } else { 48.0 };
                            ui.add_space(top);
                            ui.set_max_width(right_w.min(440.0));
                            draw_gate_card(ui, cfg, st, &mut action, select_mode);
                            ui.set_max_width(right_w.min(480.0));
                            draw_right_black_footer(ui, &mut show_help);
                        },
                    );
                });
                place_tip_bottom_right(ui, compact, &mut show_tip);
            } else {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(16.0);
                        ui.vertical_centered(|ui| {
                            ui.set_max_width(avail.x.min(440.0).max(280.0));
                            draw_capability_showcase(ui, false, true);
                            ui.add_space(14.0);
                            draw_gate_card(ui, cfg, st, &mut action, select_mode);
                            draw_right_black_footer(ui, &mut show_help);
                            // 为右下角叠加码留空，避免挡住页脚
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
    let card_w = ui.available_width().min(420.0);
    theme::mac_sheet_frame()
        .fill(Color32::WHITE)
        .inner_margin(egui::Margin::symmetric(28.0, 24.0))
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
                                .color(theme::mac_blue()),
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
    let accent = theme::shell_accent();
    let data = PathBuf::from(&cfg.data_dir);
    let needs_setup = st
        .selected
        .as_ref()
        .map(|fp| IdentityKeys::needs_password_setup(&data, fp))
        .unwrap_or(false);

    ui.label(
        RichText::new("选择用户")
            .size(22.0)
            .strong()
            .color(Color32::from_rgb(0x1D, 0x1D, 0x1F)),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new("每位用户拥有独立的备忘录和私密空间")
            .size(13.5)
            .color(Color32::from_rgb(0x6E, 0x6E, 0x73)),
    );
    ui.add_space(16.0);

    egui::ScrollArea::vertical()
        .max_height(260.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if st.identities.is_empty() {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("暂无已保存的用户")
                            .color(Color32::from_rgb(0x8E, 0x8E, 0x93)),
                    );
                });
                ui.add_space(16.0);
                return;
            }
            for (idx, m) in st.identities.iter().enumerate() {
                let sel = st.selected.as_deref() == Some(m.fingerprint.as_str());
                let fill = if sel {
                    theme::shell_accent_soft()
                } else {
                    Color32::from_rgb(0xF5, 0xF5, 0xF7)
                };
                let stroke = if sel {
                    egui::Stroke::new(2.0, accent)
                } else {
                    egui::Stroke::new(1.0, Color32::from_rgb(0xE5, 0xE5, 0xEA))
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

                egui::Frame::none()
                    .fill(fill)
                    .stroke(stroke)
                    .rounding(egui::Rounding::same(10.0))
                    .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                    .show(ui, |ui| {
                        let resp = ui.allocate_response(
                            Vec2::new(ui.available_width(), 48.0),
                            Sense::click(),
                        );
                        let painter = ui.painter();
                        let r = resp.rect;
                        let av = theme::avatar_color(idx);
                        painter.circle_filled(
                            egui::pos2(r.left() + 18.0, r.center().y),
                            16.0,
                            av,
                        );
                        painter.text(
                            egui::pos2(r.left() + 18.0, r.center().y),
                            egui::Align2::CENTER_CENTER,
                            &letter,
                            egui::FontId::proportional(14.0),
                            Color32::WHITE,
                        );
                        painter.text(
                            egui::pos2(r.left() + 44.0, r.center().y - 8.0),
                            egui::Align2::LEFT_CENTER,
                            alias,
                            egui::FontId::proportional(15.0),
                            Color32::from_rgb(0x1D, 0x1D, 0x1F),
                        );
                        let name_w = ui.fonts(|f| {
                            f.layout_no_wrap(
                                alias.to_owned(),
                                egui::FontId::proportional(15.0),
                                Color32::WHITE,
                            )
                            .size()
                            .x
                        });
                        let br = egui::Rect::from_min_size(
                            egui::pos2(r.left() + 44.0 + name_w + 8.0, r.center().y - 16.0),
                            Vec2::new(36.0, 16.0),
                        );
                        painter.rect_filled(br, 4.0, theme::shell_accent_soft());
                        painter.text(
                            br.center(),
                            egui::Align2::CENTER_CENTER,
                            "私密",
                            egui::FontId::proportional(10.0),
                            accent,
                        );
                        painter.text(
                            egui::pos2(r.left() + 44.0, r.center().y + 10.0),
                            egui::Align2::LEFT_CENTER,
                            IdentityKeys::short_fp(&m.fingerprint),
                            egui::FontId::proportional(11.5),
                            Color32::from_rgb(0x8E, 0x8E, 0x93),
                        );
                        if resp.clicked() {
                            st.selected = Some(m.fingerprint.clone());
                            st.select_pass.clear();
                            st.select_pass2.clear();
                            st.status.clear();
                            st.focus_select_pass = true;
                        }
                    });
                ui.add_space(8.0);
            }
        });

    ui.add_space(12.0);
    if st.selected.is_some() {
        if needs_setup {
            ui.label(
                RichText::new("此身份尚未设置登录密码，请先设置后再进入。")
                    .size(12.5)
                    .color(Color32::from_rgb(0xC9, 0x5A, 0x2C)),
            );
            ui.add_space(8.0);
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
        ui.add_space(12.0);

        let can = !st.busy;
        let btn_label = if st.busy {
            "打开中…"
        } else if needs_setup {
            "设置密码并进入"
        } else {
            "进入备忘录"
        };
        let text = RichText::new(btn_label)
            .color(Color32::WHITE)
            .strong()
            .size(15.0);
        let w = ui.available_width().clamp(120.0, 400.0);
        let btn_clicked = ui
            .add_enabled(
                can,
                egui::Button::new(text)
                    .fill(accent)
                    .rounding(egui::Rounding::same(theme::MAC_ROUND_PILL))
                    .min_size(Vec2::new(w, 38.0)),
            )
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
    }

    if !st.status.is_empty() {
        ui.add_space(8.0);
        mac_status(ui, &st.status, st.busy);
    }
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        let label = if st.show_more_options {
            "收起 ▴"
        } else {
            "更多选项 ▾"
        };
        let resp = ui.add(
            egui::Label::new(
                RichText::new(label)
                    .size(12.5)
                    .color(theme::mac_text_secondary()),
            )
            .sense(Sense::click()),
        );
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            st.show_more_options = !st.show_more_options;
        }
    });

    if st.show_more_options {
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(8.0);
        ui.add_enabled_ui(autostart::supported(), |ui| {
            let tip = if autostart::supported() {
                "与设置中的「开机自启动」同步；开机后托盘启动，仍需解锁身份"
            } else {
                "当前系统不支持写入开机启动项"
            };
            if ui
                .checkbox(&mut cfg.start_on_boot, "开机时启动 Memo")
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
        ui.horizontal(|ui| {
            if theme::mac_secondary_button(ui, "生成新身份").clicked() {
                st.show_init = true;
                st.init_mode_import = false;
                st.status.clear();
            }
            if theme::mac_secondary_button(ui, "导入身份").clicked() {
                st.show_init = true;
                st.init_mode_import = true;
                st.status.clear();
            }
        });
    }
}
