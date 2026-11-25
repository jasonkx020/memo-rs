//! 无边框、可拖动、置顶的桌面便签（deferred 子 Viewport）。
//! 只读展示来源路径与正文；打开列表与位置仅本机落盘（按身份隔离，不同步）。

use crate::theme;
use eframe::egui::{
    self, Color32, Frame, Margin, Pos2, RichText, Sense, Stroke, Vec2, ViewportBuilder,
    ViewportClass, ViewportCommand, ViewportId, WindowLevel,
};
use mijian_core::service::MemoView;
use mijian_core::store::{MemoCategory, MemoSpace};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 桌面便签纸色（本机会话，不同步）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StickyColor {
    #[default]
    Yellow,
    Peach,
    Mint,
    Sky,
    Lilac,
    Blush,
}

impl StickyColor {
    pub const ALL: [StickyColor; 6] = [
        StickyColor::Yellow,
        StickyColor::Peach,
        StickyColor::Mint,
        StickyColor::Sky,
        StickyColor::Lilac,
        StickyColor::Blush,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StickyColor::Yellow => "淡黄",
            StickyColor::Peach => "蜜桃",
            StickyColor::Mint => "薄荷绿",
            StickyColor::Sky => "天空蓝",
            StickyColor::Lilac => "淡紫",
            StickyColor::Blush => "雾粉",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct StickyPalette {
    bg: Color32,
    border: Color32,
    title: Color32,
    body: Color32,
    chip_bg: Color32,
    chip_fg: Color32,
}

fn palette(c: StickyColor) -> StickyPalette {
    // 高对比：纸底 / 描边 / 标题+正文（深字）
    match c {
        StickyColor::Yellow => StickyPalette {
            bg: Color32::from_rgb(0xFF, 0xF3, 0xA0),
            border: Color32::from_rgb(0xC4, 0xA0, 0x00),
            title: Color32::from_rgb(0x3D, 0x32, 0x00),
            body: Color32::from_rgb(0x4F, 0x42, 0x08),
            chip_bg: Color32::from_rgba_unmultiplied(0x3D, 0x32, 0x00, 36),
            chip_fg: Color32::from_rgb(0x3D, 0x32, 0x00),
        },
        StickyColor::Peach => StickyPalette {
            bg: Color32::from_rgb(0xFF, 0xD4, 0xC2),
            border: Color32::from_rgb(0xC4, 0x5A, 0x32),
            title: Color32::from_rgb(0x4A, 0x18, 0x08),
            body: Color32::from_rgb(0x5C, 0x28, 0x18),
            chip_bg: Color32::from_rgba_unmultiplied(0x4A, 0x18, 0x08, 36),
            chip_fg: Color32::from_rgb(0x4A, 0x18, 0x08),
        },
        StickyColor::Mint => StickyPalette {
            bg: Color32::from_rgb(0xC8, 0xEF, 0xD4),
            border: Color32::from_rgb(0x2F, 0x8A, 0x4E),
            title: Color32::from_rgb(0x0E, 0x3B, 0x22),
            body: Color32::from_rgb(0x1A, 0x4F, 0x30),
            chip_bg: Color32::from_rgba_unmultiplied(0x0E, 0x3B, 0x22, 36),
            chip_fg: Color32::from_rgb(0x0E, 0x3B, 0x22),
        },
        StickyColor::Sky => StickyPalette {
            bg: Color32::from_rgb(0xBF, 0xE4, 0xFF),
            border: Color32::from_rgb(0x2A, 0x7A, 0xB8),
            title: Color32::from_rgb(0x0A, 0x30, 0x50),
            body: Color32::from_rgb(0x15, 0x42, 0x62),
            chip_bg: Color32::from_rgba_unmultiplied(0x0A, 0x30, 0x50, 36),
            chip_fg: Color32::from_rgb(0x0A, 0x30, 0x50),
        },
        StickyColor::Lilac => StickyPalette {
            bg: Color32::from_rgb(0xE0, 0xD2, 0xF5),
            border: Color32::from_rgb(0x6B, 0x4A, 0xA8),
            title: Color32::from_rgb(0x2A, 0x18, 0x50),
            body: Color32::from_rgb(0x3A, 0x28, 0x68),
            chip_bg: Color32::from_rgba_unmultiplied(0x2A, 0x18, 0x50, 36),
            chip_fg: Color32::from_rgb(0x2A, 0x18, 0x50),
        },
        StickyColor::Blush => StickyPalette {
            bg: Color32::from_rgb(0xFF, 0xD0, 0xDC),
            border: Color32::from_rgb(0xC4, 0x48, 0x6A),
            title: Color32::from_rgb(0x4A, 0x10, 0x28),
            body: Color32::from_rgb(0x5C, 0x20, 0x38),
            chip_bg: Color32::from_rgba_unmultiplied(0x4A, 0x10, 0x28, 36),
            chip_fg: Color32::from_rgb(0x4A, 0x10, 0x28),
        },
    }
}

#[derive(Debug, Clone)]
pub struct StickyNote {
    pub id: String,
    pub title: String,
    pub body: String,
    pub space: MemoSpace,
    pub category: MemoCategory,
    pub color: StickyColor,
    /// 是否置顶（默认 false：返回桌面可见、可被其它窗盖住；可点「钉」置顶）
    pub pinned: bool,
    pub dirty: bool,
    pub dirty_since: Option<Instant>,
    pub request_focus: bool,
    pub closed: bool,
    /// 恢复用几何；运行中由视口 outer_rect 更新
    pub pos: Option<[f32; 2]>,
    pub size: Option<[f32; 2]>,
    pub flush_now: bool,
    /// 从会话恢复后需强制 OuterPosition/InnerSize，直到与目标接近
    pub apply_geom: bool,
    /// 切换置顶后需发 WindowLevel
    pub apply_pin: bool,
}

/// 新建便签默认不置顶：返回桌面即可看见，可被其它窗覆盖；需要时再点「钉」。
fn default_pinned() -> bool {
    false
}

pub type StickyHandle = Arc<Mutex<StickyNote>>;

const DEBOUNCE: Duration = Duration::from_millis(800);
const DEFAULT_SIZE: [f32; 2] = [280.0, 180.0];

fn origin_label(space: MemoSpace, category: MemoCategory) -> &'static str {
    let is_memo = category.bucket() == MemoCategory::Credentials;
    match (space, is_memo) {
        (MemoSpace::Normal, false) => "任务",
        (MemoSpace::Normal, true) => "备忘",
        (MemoSpace::Private, false) => "私密任务",
        (MemoSpace::Private, true) => "私密备忘",
    }
}

impl StickyNote {
    pub fn from_memo(m: &MemoView) -> StickyHandle {
        Self::from_memo_geom(
            m,
            None,
            None,
            false,
            StickyColor::default(),
            default_pinned(),
        )
    }

    pub fn from_memo_geom(
        m: &MemoView,
        pos: Option<[f32; 2]>,
        size: Option<[f32; 2]>,
        apply_geom: bool,
        color: StickyColor,
        pinned: bool,
    ) -> StickyHandle {
        Arc::new(Mutex::new(Self {
            id: m.id.clone(),
            title: if m.title.trim().is_empty() {
                String::new()
            } else {
                m.title.clone()
            },
            body: m.content.clone(),
            space: m.space,
            category: m.category,
            color,
            pinned,
            dirty: false,
            dirty_since: None,
            request_focus: false,
            closed: false,
            pos,
            size: size.or(Some(DEFAULT_SIZE)),
            flush_now: false,
            apply_geom: apply_geom && pos.is_some(),
            apply_pin: false,
        }))
    }

    pub fn apply_memo_if_clean(&mut self, m: &MemoView) {
        if self.dirty {
            return;
        }
        self.title = m.title.clone();
        self.body = m.content.clone();
        self.space = m.space;
        self.category = m.category;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        if self.dirty_since.is_none() {
            self.dirty_since = Some(Instant::now());
        }
    }

    pub fn needs_save(&self) -> bool {
        if !self.dirty {
            return false;
        }
        if self.flush_now || self.closed {
            return true;
        }
        self.dirty_since
            .map(|t| t.elapsed() >= DEBOUNCE)
            .unwrap_or(false)
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
        self.dirty_since = None;
        self.flush_now = false;
    }

    pub fn origin_label(&self) -> &'static str {
        origin_label(self.space, self.category)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickySession {
    pub person_fp: String,
    pub notes: Vec<StickyGeom>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickyGeom {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub color: StickyColor,
    #[serde(default = "default_pinned")]
    pub pinned: bool,
}

/// 按身份隔离的会话路径（仅本机）。
pub fn session_path_for(data_dir: &Path, person_fp: &str) -> PathBuf {
    data_dir
        .join("identities")
        .join(person_fp)
        .join("sticky_session.json")
}

/// 旧版全局路径（迁移用）。
fn legacy_session_path(data_dir: &Path) -> PathBuf {
    data_dir.join("sticky_session.json")
}

pub fn load_session(data_dir: &Path, person_fp: &str) -> Option<StickySession> {
    let path = session_path_for(data_dir, person_fp);
    if let Ok(raw) = std::fs::read_to_string(&path) {
        if let Ok(s) = serde_json::from_str::<StickySession>(&raw) {
            if s.person_fp == person_fp || s.person_fp.is_empty() {
                return Some(StickySession {
                    person_fp: person_fp.to_string(),
                    notes: s.notes,
                });
            }
        }
    }
    // 兼容旧版根目录文件
    let raw = std::fs::read_to_string(legacy_session_path(data_dir)).ok()?;
    let s: StickySession = serde_json::from_str(&raw).ok()?;
    if s.person_fp != person_fp {
        return None;
    }
    Some(s)
}

pub fn save_session(data_dir: &Path, session: &StickySession) -> anyhow::Result<()> {
    let path = session_path_for(data_dir, &session.person_fp);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(session)?;
    std::fs::write(path, raw)?;
    Ok(())
}

pub fn session_from_handles(person_fp: &str, stickies: &[(String, StickyHandle)]) -> StickySession {
    let mut notes = Vec::new();
    for (id, h) in stickies {
        let n = h.lock();
        if n.closed {
            continue;
        }
        let size = n.size.unwrap_or(DEFAULT_SIZE);
        let pos = n.pos.unwrap_or([100.0, 100.0]);
        notes.push(StickyGeom {
            id: id.clone(),
            x: pos[0],
            y: pos[1],
            w: size[0].max(200.0),
            h: size[1].max(120.0),
            color: n.color,
            pinned: n.pinned,
        });
    }
    StickySession {
        person_fp: person_fp.to_string(),
        notes,
    }
}

/// 每帧注册 deferred 便签视口。
pub fn show_viewport(ctx: &egui::Context, note: &StickyHandle) {
    let (id_str, title, origin, request_focus, closed, pos, size, pinned) = {
        let n = note.lock();
        (
            n.id.clone(),
            n.title.clone(),
            n.origin_label(),
            n.request_focus,
            n.closed,
            n.pos,
            n.size.unwrap_or(DEFAULT_SIZE),
            n.pinned,
        )
    };
    if closed {
        return;
    }

    let window_title = format!("{} · {}", origin, display_title(&title));
    let vid = ViewportId::from_hash_of(("sticky", id_str.as_str()));
    let mut builder = ViewportBuilder::default()
        .with_title(window_title.clone())
        .with_decorations(false)
        .with_taskbar(false)
        .with_inner_size(size)
        .with_min_inner_size([200.0, 120.0])
        .with_resizable(true);
    if pinned {
        builder = builder.with_always_on_top();
    }
    if let Some(p) = pos {
        builder = builder.with_position(Pos2::new(p[0], p[1]));
    }

    if request_focus {
        ctx.send_viewport_cmd_to(vid, ViewportCommand::Focus);
        note.lock().request_focus = false;
    }

    // 「显示桌面」后 OS 会藏掉视口；主窗未聚焦时也要持续拉回可见，勿等用户点主程序。
    ctx.send_viewport_cmd_to(vid, ViewportCommand::Visible(true));
    ctx.send_viewport_cmd_to(
        vid,
        ViewportCommand::WindowLevel(if pinned {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        }),
    );

    let note_cb = note.clone();
    ctx.show_viewport_deferred(vid, builder, move |ctx, class| {
        let mut note = note_cb.lock();
        // Close 不会立刻拆窗；停画后 GL 会留下黑框。先 Visible(false) 藏掉。
        if note.closed {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        // 子视口内再确认一次：从「显示桌面」回来时无需主窗抢焦点
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        let level = if note.pinned {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
        note.apply_pin = false;

        // 恢复历史位置：强制 OuterPosition，避免 winit 忽略 builder.with_position
        if note.apply_geom {
            if let Some(p) = note.pos {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new(p[0], p[1])));
            }
            if let Some(s) = note.size {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(s[0], s[1])));
            }
            if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                if let Some(p) = note.pos {
                    let ok_x = (rect.min.x - p[0]).abs() < 8.0;
                    let ok_y = (rect.min.y - p[1]).abs() < 8.0;
                    if ok_x && ok_y {
                        note.apply_geom = false;
                        note.pos = Some([rect.min.x, rect.min.y]);
                        note.size = Some([rect.width().max(200.0), rect.height().max(120.0)]);
                    }
                }
            }
            ctx.request_repaint();
        } else if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            note.pos = Some([rect.min.x, rect.min.y]);
            note.size = Some([rect.width().max(200.0), rect.height().max(120.0)]);
        }

        if class == ViewportClass::Embedded {
            let title = format!("{} · {}", note.origin_label(), display_title(&note.title));
            let id = note.id.clone();
            let emb_pal = palette(note.color);
            drop(note);
            let mut open = true;
            egui::Window::new(title)
                .id(egui::Id::new(("sticky_embed", id.as_str())))
                .open(&mut open)
                .resizable(true)
                .default_size(DEFAULT_SIZE)
                .frame(
                    Frame::none()
                        .fill(emb_pal.bg)
                        .stroke(Stroke::new(1.0, emb_pal.border))
                        .inner_margin(Margin::symmetric(10.0, 8.0))
                        .rounding(8.0),
                )
                .show(ctx, |ui| {
                    let mut n = note_cb.lock();
                    paint_readonly(ui, &mut n, false);
                });
            if !open {
                let mut n = note_cb.lock();
                n.flush_now = true;
                n.closed = true;
            }
            return;
        }

        let close_os = ctx.input(|i| i.viewport().close_requested());
        let mut close_btn = false;
        let pal = palette(note.color);
        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(pal.bg)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(Margin::symmetric(10.0, 8.0)),
            )
            .show(ctx, |ui| {
                close_btn = paint_readonly(ui, &mut note, true);
            });

        if close_os || close_btn {
            note.flush_now = true;
            note.closed = true;
            // 立刻隐藏，避免主窗在托盘时 GC 延迟留下黑框
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    });
}

/// 主循环在移除便签前再催一次关窗（防止 deferred 回调未跑到）。
pub fn force_hide_viewport(ctx: &egui::Context, memo_id: &str) {
    let vid = ViewportId::from_hash_of(("sticky", memo_id));
    ctx.send_viewport_cmd_to(vid, ViewportCommand::Visible(false));
    ctx.send_viewport_cmd_to(vid, ViewportCommand::Close);
}

fn display_title(title: &str) -> &str {
    let t = title.trim();
    if t.is_empty() {
        "(无标题)"
    } else {
        t
    }
}

fn paint_origin_chip(ui: &mut egui::Ui, label: &str, pal: StickyPalette) {
    Frame::none()
        .fill(pal.chip_bg)
        .rounding(4.0)
        .inner_margin(Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(11.0).color(pal.chip_fg));
        });
}

/// 菜单里用的饱和色点（六色可分）。
fn color_swatch(c: StickyColor) -> Color32 {
    match c {
        StickyColor::Yellow => Color32::from_rgb(0xE6, 0xC2, 0x00),
        StickyColor::Peach => Color32::from_rgb(0xE8, 0x70, 0x50),
        StickyColor::Mint => Color32::from_rgb(0x3C, 0xB8, 0x6A),
        StickyColor::Sky => Color32::from_rgb(0x3B, 0x9F, 0xE0),
        StickyColor::Lilac => Color32::from_rgb(0x8B, 0x6A, 0xD4),
        StickyColor::Blush => Color32::from_rgb(0xE8, 0x5A, 0x82),
    }
}

const MENU_DOT: f32 = 16.0;

/// 右键菜单内一行色点；点中改色并关菜单。
fn paint_menu_color_dots(ui: &mut egui::Ui, note: &mut StickyNote) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(4.0);
        for &c in &StickyColor::ALL {
            let selected = note.color == c;
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(MENU_DOT), Sense::click());
            let resp = resp.on_hover_text(c.label());
            let center = rect.center();
            let r = MENU_DOT * 0.5 - 1.0;
            ui.painter().circle_filled(center, r, color_swatch(c));
            ui.painter().circle_stroke(
                center,
                r,
                Stroke::new(
                    if selected { 2.0 } else { 1.0 },
                    if selected {
                        Color32::from_rgb(0x1F, 0x29, 0x37)
                    } else {
                        Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 55)
                    },
                ),
            );
            if resp.clicked() {
                note.color = c;
                ui.close_menu();
            }
        }
    });
}

/// 便签右键菜单；点了关闭返回 true。
fn sticky_context_menu(ui: &mut egui::Ui, note: &mut StickyNote, panel_h: f32) -> bool {
    let mut close = false;
    theme::style_context_menu(ui);
    // 便签视口很小，菜单过高会被窗体裁掉
    let max_h = (panel_h - 8.0).clamp(120.0, 280.0);
    egui::ScrollArea::vertical()
        .max_height(max_h)
        .show(ui, |ui| {
            let pin_label = if note.pinned {
                "取消置顶"
            } else {
                "置顶"
            };
            let pin_mark = if note.pinned { "浮" } else { "钉" };
            if theme::menu_item_mark(ui, pin_mark, pin_label).clicked() {
                note.pinned = !note.pinned;
                note.apply_pin = true;
                ui.close_menu();
            }
            theme::menu_sep(ui);
            theme::menu_section(ui, "纸色");
            paint_menu_color_dots(ui, note);
            ui.add_space(4.0);
            theme::menu_sep(ui);
            if theme::menu_item_danger_mark(ui, "关", "关闭便签").clicked() {
                close = true;
                ui.close_menu();
            }
        });
    close
}

/// 返回是否点了关闭。
fn paint_readonly(ui: &mut egui::Ui, note: &mut StickyNote, enable_drag: bool) -> bool {
    let pal = palette(note.color);
    let mut close = false;
    let panel = ui.max_rect();
    let header_top = ui.cursor().min.y;
    let row_h = ui.spacing().interact_size.y.max(22.0);

    // 标题栏：chip + 限宽标题（控件改右键菜单，无悬停工具）
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        paint_origin_chip(ui, note.origin_label(), pal);
        let title = if note.title.trim().is_empty() {
            "(无标题)"
        } else {
            note.title.as_str()
        };
        let title_w = ui.available_width().max(40.0);
        ui.allocate_ui_with_layout(
            Vec2::new(title_w, row_h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(RichText::new(title).size(14.0).color(pal.title))
                        .truncate(true),
                );
            },
        );
    });
    let row1_bottom = ui.cursor().min.y.max(header_top + row_h);
    // 整行标题栏（含文字右侧空白）均可拖
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(panel.left(), header_top),
        egui::pos2(panel.right(), row1_bottom),
    );

    ui.add_space(2.0);
    ui.separator();
    ui.add_space(4.0);

    let body_h = (ui.available_height() - 22.0).max(48.0);
    egui::ScrollArea::vertical()
        .id_source(("sticky_scroll", note.id.as_str()))
        .auto_shrink([false, false])
        .max_height(body_h)
        .show(ui, |ui| {
            let body = if note.body.trim().is_empty() {
                "（无正文）"
            } else {
                note.body.as_str()
            };
            ui.add(egui::Label::new(RichText::new(body).size(13.0).color(pal.body)).wrap(true));
        });

    ui.label(
        RichText::new("只读 · 拖动标题栏 · 右键菜单")
            .size(10.5)
            .color(pal.chip_fg),
    );

    // 正文区右键（不盖住标题栏，避免抢走拖动）
    let body_rect = egui::Rect::from_min_max(
        egui::pos2(panel.left(), row1_bottom),
        panel.max,
    );
    let body_ctx = ui.interact(
        body_rect,
        egui::Id::new(("sticky_ctx_body", note.id.as_str())),
        Sense::click(),
    );
    body_ctx.context_menu(|ui| {
        if sticky_context_menu(ui, note, panel.height()) {
            close = true;
        }
    });

    if enable_drag {
        let drag = ui
            .interact(
                drag_rect,
                egui::Id::new(("sticky_drag", note.id.as_str())),
                Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Grab);
        // 标题栏也可右键
        drag.context_menu(|ui| {
            if sticky_context_menu(ui, note, panel.height()) {
                close = true;
            }
        });
        // 右键不拖动
        if drag.drag_started()
            && ui.input(|i| i.pointer.primary_down())
            && !close
        {
            ui.ctx()
                .set_cursor_icon(egui::CursorIcon::Grabbing);
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
    } else {
        let title_ctx = ui.interact(
            drag_rect,
            egui::Id::new(("sticky_ctx_title", note.id.as_str())),
            Sense::click(),
        );
        title_ctx.context_menu(|ui| {
            if sticky_context_menu(ui, note, panel.height()) {
                close = true;
            }
        });
    }

    close
}
