//! 任务 / 私密共用页面壳：左栏页头 + 分栏宽度。
//! 统一壳：右轨约 420 固定 · 左列表吃满剩余。

use crate::theme;
use eframe::egui::{
    self, Color32, CursorIcon, Id, LayerId, Order, RichText, Rounding, Sense, Vec2,
};
use serde::{Deserialize, Serialize};
use std::hash::Hash;
use std::path::{Path, PathBuf};

/// 分栏间距。
pub const PAGE_GAP: f32 = 8.0;

/// 右栏（日历 / 过滤轨）目标宽度（窗口放大时保持，左栏伸展）。
pub const PAGE_RAIL_TARGET: f32 = 420.0;
pub const PAGE_RAIL_MIN: f32 = 280.0;
/// 左列表最小宽度。
pub const PAGE_LIST_MIN: f32 = 280.0;

/// 右侧抽屉滑入/滑出时长（秒）。
pub const DRAWER_ANIM_SEC: f32 = 0.22;

/// 表单 / 详情抽屉默认与钳制宽度。
pub const DRAWER_FORM_DEFAULT: f32 = 440.0;
pub const DRAWER_DETAIL_DEFAULT: f32 = 420.0;
pub const DRAWER_W_MIN: f32 = 320.0;

/// 抽屉左右 padding 之外，再留一点给描边/滚动条；低于此总宽则勿渲染表单内容。
pub const DRAWER_CONTENT_MIN: f32 = 48.0;

/// 滑入/收起动画中面板过窄（含 pad）时返回 true，应只画壳不 allocate 表单。
pub fn drawer_panel_too_narrow(panel_w: f32, pad_x: f32) -> bool {
    !panel_w.is_finite() || panel_w < pad_x * 2.0 + DRAWER_CONTENT_MIN
}

/// 将期望宽度钳到当前 `available_width`（有限且 ≥1），杜绝 allocate/set_width 超父级退进程。
pub fn fit_avail_width(ui: &egui::Ui, want: f32) -> f32 {
    let a = ui.available_width();
    if !a.is_finite() || a <= 0.0 {
        1.0
    } else if !want.is_finite() || want <= 0.0 {
        a.min(1.0).max(1.0)
    } else {
        want.min(a).max(1.0)
    }
}

/// 均分 `n` 列，列宽之和不超过 `avail`（不做 `.max(80)` 一类硬底）。
pub fn split_col_width(avail: f32, n: usize, gap: f32) -> f32 {
    let n = n.max(1) as f32;
    let gaps = gap * (n - 1.0);
    if !avail.is_finite() || avail <= gaps + 1.0 {
        1.0
    } else {
        ((avail - gaps) / n).floor().max(1.0)
    }
}

fn default_form_w() -> f32 {
    DRAWER_FORM_DEFAULT
}
fn default_detail_w() -> f32 {
    DRAWER_DETAIL_DEFAULT
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DrawerUiPrefs {
    #[serde(default = "default_form_w")]
    form_w: f32,
    #[serde(default = "default_detail_w")]
    detail_w: f32,
}

/// 按身份持久化的抽屉宽度。
pub struct DrawerWidths {
    pub form_w: f32,
    pub detail_w: f32,
    prefs_dirty: bool,
}

impl Default for DrawerWidths {
    fn default() -> Self {
        Self {
            form_w: DRAWER_FORM_DEFAULT,
            detail_w: DRAWER_DETAIL_DEFAULT,
            prefs_dirty: false,
        }
    }
}

fn drawer_prefs_path(data_dir: &Path, person_fp: &str) -> PathBuf {
    data_dir
        .join("identities")
        .join(person_fp)
        .join("drawer_ui.json")
}

/// 相对当前内容区宽度钳制。
pub fn clamp_drawer_w(w: f32, host_w: f32) -> f32 {
    let host_w = host_w.max(DRAWER_W_MIN);
    let max_w = (host_w * 0.85).min(900.0).max(DRAWER_W_MIN);
    w.clamp(DRAWER_W_MIN, max_w.min(host_w))
}

impl DrawerWidths {
    pub fn load(&mut self, data_dir: &Path, person_fp: &str) {
        let path = drawer_prefs_path(data_dir, person_fp);
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(p) = serde_json::from_str::<DrawerUiPrefs>(&raw) else {
            return;
        };
        self.form_w = p.form_w.max(DRAWER_W_MIN);
        self.detail_w = p.detail_w.max(DRAWER_W_MIN);
        self.prefs_dirty = false;
    }

    pub fn save_if_dirty(&mut self, data_dir: &Path, person_fp: &str) {
        if !self.prefs_dirty {
            return;
        }
        let path = drawer_prefs_path(data_dir, person_fp);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let p = DrawerUiPrefs {
            form_w: self.form_w,
            detail_w: self.detail_w,
        };
        if let Ok(raw) = serde_json::to_string_pretty(&p) {
            let _ = std::fs::write(path, raw);
            self.prefs_dirty = false;
        }
    }

    pub fn mark_dirty(&mut self) {
        self.prefs_dirty = true;
    }
}

/// 左缘命中宽度（落在面板内侧，避免被左侧遮罩抢走）。
const DRAWER_RESIZE_HIT: f32 = 12.0;

/// 面板左缘拖拽调宽；向左拖变宽。动画未完成时不响应。返回是否改变宽度。
pub fn drawer_resize_edge(
    ctx: &egui::Context,
    id_salt: impl Hash,
    panel: egui::Rect,
    width: &mut f32,
    host_w: f32,
    anim_t: f32,
) -> bool {
    if anim_t < 0.95 || panel.height() < 40.0 || panel.width() < 40.0 {
        return false;
    }
    let hit_w = DRAWER_RESIZE_HIT.min(panel.width() * 0.25).max(8.0);
    let handle = egui::Rect::from_min_size(
        egui::pos2(panel.left(), panel.top()),
        Vec2::new(hit_w, panel.height()),
    );

    let area_id = Id::new(("drawer_resize", &id_salt));
    let drag_id = Id::new(("drawer_resize_drag", &id_salt));
    let layer = LayerId::new(Order::Foreground, area_id);

    // 指针命中（不依赖 Area 首帧尺寸）；拖拽中即使移出仍继续
    let pointer = ctx.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()));
    let primary_down = ctx.input(|i| i.pointer.primary_down());
    let primary_pressed = ctx.input(|i| i.pointer.primary_pressed());
    let over = pointer.map(|p| handle.contains(p)).unwrap_or(false);
    let mut dragging = ctx.data(|d| d.get_temp::<bool>(drag_id)).unwrap_or(false);
    if primary_pressed && over {
        dragging = true;
    }
    if !primary_down {
        dragging = false;
    }
    ctx.data_mut(|d| d.insert_temp(drag_id, dragging));

    let hot = over || dragging;
    if hot {
        ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
    }

    // 可交互 Area 置顶，挡住面板内容抢事件
    ctx.move_to_top(layer);
    let mut changed = false;
    egui::Area::new(area_id)
        .order(Order::Foreground)
        .fixed_pos(handle.min)
        .interactable(true)
        .constrain(false)
        .show(ctx, |ui| {
            let (rect, resp) =
                ui.allocate_exact_size(handle.size(), Sense::click_and_drag());
            if resp.hovered() || resp.dragged() || hot {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            let line_w = if hot { 3.0 } else { 2.0 };
            let line = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 1.0, rect.top() + 8.0),
                Vec2::new(line_w, (rect.height() - 16.0).max(24.0)),
            );
            let accent = theme::shell_accent();
            let fill = if dragging || resp.dragged() {
                accent
            } else if hot {
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 200)
            } else {
                theme::border_strong()
            };
            ui.painter().rect_filled(line, Rounding::same(1.0), fill);
            if hot {
                ui.painter().rect_filled(
                    rect,
                    Rounding::ZERO,
                    Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 32),
                );
            }
            let drag_now = dragging || resp.dragged();
            if drag_now {
                let dx = ui.input(|i| i.pointer.delta().x);
                let new_w = clamp_drawer_w(*width - dx, host_w);
                if (new_w - *width).abs() > 0.5 {
                    *width = new_w;
                    changed = true;
                }
                ui.ctx().request_repaint();
            }
        });
    changed
}

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

/// 卸挂载并清 tween，下次 `drawer_anim_begin` 从 0 重新滑入。
/// 用于重复点「新建」或切换作用域后避免脏动画状态。
pub fn drawer_anim_reset(ctx: &egui::Context, id_salt: impl Hash) {
    let mounted_id = Id::new(("drawer_mounted", &id_salt));
    // 仅卸挂载；下次 begin 会写入 from=0 的新 tween
    ctx.data_mut(|d| d.insert_temp(mounted_id, false));
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

/// 返回 `(list_w, rail_w)`：右轨约 420 优先，左列表吃满剩余。
/// 窄窗时按真实 `avail` 收缩，**绝不**返回之和超过 `avail`（避免 allocate 退进程）。
pub fn page_rail_split(avail: f32) -> (f32, f32) {
    let gap = PAGE_GAP;
    if !avail.is_finite() || avail <= gap + 2.0 {
        return (1.0, 1.0);
    }
    let usable = avail - gap;
    let mut rail = PAGE_RAIL_TARGET.min(usable - 1.0).max(1.0);
    if usable >= PAGE_LIST_MIN + PAGE_RAIL_MIN {
        if rail + PAGE_LIST_MIN > usable {
            rail = (usable - PAGE_LIST_MIN).clamp(PAGE_RAIL_MIN, PAGE_RAIL_TARGET);
        }
    } else {
        // 不够双栏软底：按比例拆，不抬高虚拟 avail
        rail = (usable * 0.45).clamp(1.0, PAGE_RAIL_TARGET);
    }
    let list = (usable - rail).max(1.0);
    debug_assert!(list + rail + gap <= avail + 0.5);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_rail_split_never_exceeds_avail() {
        // avail ≤ gap+2 时只能返回占位宽；其余必须 list+rail+gap ≤ avail
        for &avail in &[1.0_f32, 10.0, 40.0, 120.0, 200.0, 400.0, 568.0, 900.0, 1400.0] {
            let (list, rail) = page_rail_split(avail);
            assert!(list >= 1.0 && rail >= 1.0, "avail={avail}");
            if avail > PAGE_GAP + 2.0 {
                assert!(
                    list + rail + PAGE_GAP <= avail + 0.51,
                    "avail={avail} list={list} rail={rail}"
                );
            }
        }
        // 回归：旧实现会把 avail 抬到 ≥568，窄窗双栏之和远超真实宽度
        let (list, rail) = page_rail_split(200.0);
        assert!(list + rail + PAGE_GAP <= 200.5);
        assert!(list < 200.0 && rail < 200.0);
    }

    #[test]
    fn split_col_width_fits() {
        let w = split_col_width(100.0, 3, 8.0);
        assert!(w * 3.0 + 16.0 <= 100.5);
        assert_eq!(split_col_width(10.0, 3, 8.0), 1.0);
    }

    #[test]
    fn drawer_narrow_threshold() {
        assert!(drawer_panel_too_narrow(70.0, 12.0));
        assert!(!drawer_panel_too_narrow(120.0, 12.0));
    }
}
