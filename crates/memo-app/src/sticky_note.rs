//! 无边框、可拖动、置顶的桌面便签（deferred 子 Viewport）。
//! 可编辑并回写备忘；打开列表与位置仅本机落盘（按身份隔离，不同步）。

use crate::theme;
use eframe::egui::{
    self, Color32, Frame, Margin, Pos2, RichText, Sense, Stroke, TextEdit, Vec2, ViewportBuilder,
    ViewportClass, ViewportCommand, ViewportId,
};
use memo_core::service::MemoView;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct StickyNote {
    pub id: String,
    pub title: String,
    pub body: String,
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
}

pub type StickyHandle = Arc<Mutex<StickyNote>>;

const DEBOUNCE: Duration = Duration::from_millis(800);
const DEFAULT_SIZE: [f32; 2] = [280.0, 180.0];

impl StickyNote {
    pub fn from_memo(m: &MemoView) -> StickyHandle {
        Self::from_memo_geom(m, None, None, false)
    }

    pub fn from_memo_geom(
        m: &MemoView,
        pos: Option<[f32; 2]>,
        size: Option<[f32; 2]>,
        apply_geom: bool,
    ) -> StickyHandle {
        Arc::new(Mutex::new(Self {
            id: m.id.clone(),
            title: if m.title.trim().is_empty() {
                String::new()
            } else {
                m.title.clone()
            },
            body: m.content.clone(),
            dirty: false,
            dirty_since: None,
            request_focus: false,
            closed: false,
            pos,
            size: size.or(Some(DEFAULT_SIZE)),
            flush_now: false,
            apply_geom: apply_geom && pos.is_some(),
        }))
    }

    pub fn apply_memo_if_clean(&mut self, m: &MemoView) {
        if self.dirty {
            return;
        }
        self.title = m.title.clone();
        self.body = m.content.clone();
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
        });
    }
    StickySession {
        person_fp: person_fp.to_string(),
        notes,
    }
}

/// 每帧注册 deferred 便签视口。
pub fn show_viewport(ctx: &egui::Context, note: &StickyHandle) {
    let (id_str, title, request_focus, closed, pos, size) = {
        let n = note.lock();
        (
            n.id.clone(),
            n.title.clone(),
            n.request_focus,
            n.closed,
            n.pos,
            n.size.unwrap_or(DEFAULT_SIZE),
        )
    };
    if closed {
        return;
    }

    let vid = ViewportId::from_hash_of(("sticky", id_str.as_str()));
    let mut builder = ViewportBuilder::default()
        .with_title(format!("便签 · {}", display_title(&title)))
        .with_decorations(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_inner_size(size)
        .with_min_inner_size([200.0, 120.0])
        .with_resizable(true);
    if let Some(p) = pos {
        builder = builder.with_position(Pos2::new(p[0], p[1]));
    }

    if request_focus {
        ctx.send_viewport_cmd_to(vid, ViewportCommand::Focus);
        note.lock().request_focus = false;
    }

    let note_cb = note.clone();
    ctx.show_viewport_deferred(vid, builder, move |ctx, class| {
        let mut note = note_cb.lock();
        // Close 不会立刻拆窗；停画后 GL 会留下黑框。先 Visible(false) 藏掉。
        if note.closed {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

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
            let mut open = true;
            let title = note.title.clone();
            let id = note.id.clone();
            drop(note);
            egui::Window::new(format!("便签 · {}", display_title(&title)))
                .id(egui::Id::new(("sticky_embed", id.as_str())))
                .open(&mut open)
                .resizable(true)
                .default_size(DEFAULT_SIZE)
                .show(ctx, |ui| {
                    let mut n = note_cb.lock();
                    paint_editable(ui, &mut n, false);
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
        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(sticky_bg())
                    .stroke(Stroke::new(1.0, sticky_border()))
                    .inner_margin(Margin::symmetric(10.0, 8.0)),
            )
            .show(ctx, |ui| {
                close_btn = paint_editable(ui, &mut note, true);
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

fn sticky_bg() -> Color32 {
    Color32::from_rgb(0xFF, 0xF6, 0xC8)
}

fn sticky_border() -> Color32 {
    Color32::from_rgb(0xE8, 0xD4, 0x6A)
}

/// 返回是否点了关闭。
fn paint_editable(ui: &mut egui::Ui, note: &mut StickyNote, enable_drag: bool) -> bool {
    let panel = ui.max_rect();
    let hovered = ui.rect_contains_pointer(panel);
    if hovered {
        ui.ctx().request_repaint();
    }

    const CLOSE_W: f32 = 28.0;
    let mut close = false;
    let mut on_close = false;
    let mut title_bar = egui::Rect::NOTHING;

    ui.horizontal(|ui| {
        let max_w = (ui.available_width() - CLOSE_W).max(40.0);
        ui.set_max_width(max_w);
        let r = ui.add(
            TextEdit::singleline(&mut note.title)
                .desired_width(max_w - 8.0)
                .font(egui::TextStyle::Body)
                .text_color(Color32::from_rgb(0x3A, 0x32, 0x1A))
                .frame(false)
                .hint_text("标题"),
        );
        title_bar = r.rect;
        if r.changed() {
            note.mark_dirty();
        }
    });
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(4.0);

    let body_h = (ui.available_height() - 28.0).max(48.0);
    egui::ScrollArea::vertical()
        .id_source(("sticky_scroll", note.id.as_str()))
        .auto_shrink([false, false])
        .max_height(body_h)
        .show(ui, |ui| {
            let r = ui.add(
                TextEdit::multiline(&mut note.body)
                    .desired_width(ui.available_width())
                    .desired_rows(6)
                    .frame(false)
                    .text_color(Color32::from_rgb(0x4A, 0x40, 0x28))
                    .hint_text("正文…"),
            );
            if r.changed() {
                note.mark_dirty();
            }
        });

    ui.label(
        RichText::new(if note.dirty {
            "未保存 · 停顿后自动写回"
        } else {
            "已同步 · 拖动标题栏移动"
        })
        .size(10.5)
        .color(theme::text_muted()),
    );

    if enable_drag {
        if hovered {
            let close_rect = egui::Rect::from_min_size(
                egui::pos2(panel.right() - CLOSE_W - 2.0, panel.top() + 2.0),
                Vec2::new(CLOSE_W, 24.0),
            );
            let r = ui
                .put(
                    close_rect,
                    egui::Button::new(
                        RichText::new("×")
                            .size(16.0)
                            .color(Color32::from_rgb(0x6B, 0x5C, 0x3A)),
                    )
                    .fill(Color32::from_white_alpha(160))
                    .frame(false)
                    .min_size(Vec2::new(CLOSE_W, 24.0)),
                )
                .on_hover_text("关闭便签");
            on_close = r.hovered() || r.is_pointer_button_down_on();
            if r.clicked() {
                close = true;
            }
        }
        // 仅标题栏空白处拖动（避免与 TextEdit 抢手势）
        if title_bar == egui::Rect::NOTHING {
            title_bar = egui::Rect::from_min_size(panel.min, Vec2::new(panel.width(), 28.0));
        }
        let drag_rect = egui::Rect::from_min_max(
            title_bar.min,
            egui::pos2(panel.right() - CLOSE_W - 4.0, title_bar.bottom()),
        );
        let drag = ui.interact(
            drag_rect,
            ui.id().with("sticky_drag").with(note.id.as_str()),
            Sense::click_and_drag(),
        );
        if drag.drag_started() && !on_close && !close {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

    close
}
