//! 任务 / 私密共用页面壳：左栏页头 + 分栏宽度。
//! 统一壳：右轨约 420 固定 · 左列表吃满剩余。

use crate::theme;
use eframe::egui::{self, Color32, Id, RichText, Rounding, Vec2};
use std::hash::Hash;

/// 分栏间距。
pub const PAGE_GAP: f32 = 8.0;

/// 右栏（日历 / 过滤轨）目标宽度（窗口放大时保持，左栏伸展）。
pub const PAGE_RAIL_TARGET: f32 = 420.0;
pub const PAGE_RAIL_MIN: f32 = 280.0;
/// 左列表最小宽度。
pub const PAGE_LIST_MIN: f32 = 280.0;

/// 右侧抽屉滑入/滑出时长（秒）。
pub const DRAWER_ANIM_SEC: f32 = 0.22;

/// 手动插值状态（不用 `animate_bool`：其对 true 首帧会直接跳到 1）。
#[derive(Clone, Copy)]
struct DrawerTween {
    /// 是否朝打开方向
    want_open: bool,
    /// 本段动画起点值
    from: f32,
    /// 本段开始时间（`ctx.input.time`）
    start: f64,
}

/// 本帧抽屉动画状态（每帧只 `begin` 一次推进进度）。
pub struct DrawerAnim {
    pub t: f32,
    /// 已在收起过程中（忽略重复关闭输入）。
    pub closing: bool,
    tween_id: Id,
    mounted_id: Id,
}

fn drawer_ease(p: f32) -> f32 {
    // ease-out cubic
    let p = p.clamp(0.0, 1.0);
    1.0 - (1.0 - p).powi(3)
}

fn drawer_eval(tw: DrawerTween, now: f64) -> f32 {
    let target = if tw.want_open { 1.0 } else { 0.0 };
    let elapsed = (now - tw.start).max(0.0) as f32;
    let p = (elapsed / DRAWER_ANIM_SEC).clamp(0.0, 1.0);
    let t = tw.from + (target - tw.from) * drawer_ease(p);
    t.clamp(0.0, 1.0)
}

/// 开始本帧抽屉动画：挂载时从 0 滑入。
pub fn drawer_anim_begin(ctx: &egui::Context, id_salt: impl Hash) -> DrawerAnim {
    let tween_id = Id::new(("drawer_tween", &id_salt));
    let mounted_id = Id::new(("drawer_mounted", &id_salt));
    let now = ctx.input(|i| i.time);

    let mounted = ctx.data(|d| d.get_temp::<bool>(mounted_id)).unwrap_or(false);
    if !mounted {
        ctx.data_mut(|d| {
            d.insert_temp(mounted_id, true);
            d.insert_temp(
                tween_id,
                DrawerTween {
                    want_open: true,
                    from: 0.0,
                    start: now,
                },
            );
        });
    }

    let tw = ctx
        .data(|d| d.get_temp::<DrawerTween>(tween_id))
        .unwrap_or(DrawerTween {
            want_open: true,
            from: 0.0,
            start: now,
        });
    let t = drawer_eval(tw, now);
    let target = if tw.want_open { 1.0 } else { 0.0 };
    if (t - target).abs() > 0.001 {
        ctx.request_repaint();
    }
    DrawerAnim {
        t,
        closing: !tw.want_open,
        tween_id,
        mounted_id,
    }
}

/// 结束本帧：若 `request_close` 则开始收起；收起完成返回 `true`（可卸载）。
pub fn drawer_anim_end(ctx: &egui::Context, anim: &DrawerAnim, request_close: bool) -> bool {
    let now = ctx.input(|i| i.time);
    if request_close && !anim.closing {
        ctx.data_mut(|d| {
            d.insert_temp(
                anim.tween_id,
                DrawerTween {
                    want_open: false,
                    from: anim.t,
                    start: now,
                },
            );
        });
        ctx.request_repaint();
        return false;
    }
    if anim.closing && anim.t < 0.01 {
        // 卸挂载；下次 begin 会重新插入 from=0 的 tween
        ctx.data_mut(|d| d.insert_temp(anim.mounted_id, false));
        return true;
    }
    false
}

/// 由动画进度得到面板与遮罩矩形，以及遮罩透明度。
pub fn drawer_layout(
    host: egui::Rect,
    drawer_w: f32,
    t: f32,
) -> (egui::Rect, egui::Rect, u8) {
    let t = t.clamp(0.0, 1.0);
    let panel_left = host.right() - drawer_w * t;
    let panel = egui::Rect::from_min_max(
        egui::pos2(panel_left, host.top()),
        host.right_bottom(),
    );
    let dim = egui::Rect::from_min_max(host.min, egui::pos2(panel_left, host.bottom()));
    let alpha = (70.0 * t).round().clamp(0.0, 70.0) as u8;
    (panel, dim, alpha)
}

/// 返回 `(list_w, rail_w)`：右轨约 420 固定，左列表吃满剩余。
pub fn page_rail_split(avail: f32) -> (f32, f32) {
    let gap = PAGE_GAP;
    let avail = avail.max(gap + PAGE_LIST_MIN + PAGE_RAIL_MIN);
    let mut rail = PAGE_RAIL_TARGET;
    if rail + PAGE_LIST_MIN + gap > avail {
        rail = (avail - PAGE_LIST_MIN - gap).max(PAGE_RAIL_MIN);
    }
    let list = (avail - rail - gap).max(PAGE_LIST_MIN);
    (list, rail)
}

/// 左栏页头：标题 · 计数 · + 新建（写操作入口，编辑进抽屉）。
/// 仅画在左栏，右栏日历可顶对齐上移。
pub fn show_page_header(
    ui: &mut egui::Ui,
    title: &str,
    count_text: &str,
    open_new: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            RichText::new(title)
                .size(18.0)
                .strong()
                .color(theme::text()),
        );
        if !count_text.is_empty() {
            ui.label(
                RichText::new(count_text)
                    .size(13.0)
                    .color(theme::text_muted()),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("+ 新建")
                            .color(Color32::WHITE)
                            .strong()
                            .size(13.0),
                    )
                    .fill(theme::shell_accent())
                    .rounding(Rounding::same(7.0))
                    .min_size(Vec2::new(64.0, 28.0)),
                )
                .clicked()
            {
                *open_new = true;
            }
        });
    });
}
