//! 无边框、可拖动、置顶的桌面便签（子 Viewport）。
//!
//! 使用 **deferred** viewport：主窗口最小化后便签仍可独立重绘并 `StartDrag`。
//! （immediate 会把重绘转给父窗口，最小化时被 eframe 丢弃，导致拖不动。）

use crate::theme;
use eframe::egui::{
    self, Color32, Frame, Margin, RichText, Sense, Stroke, Vec2, ViewportBuilder, ViewportClass,
    ViewportCommand, ViewportId,
};
use memo_core::service::MemoView;
use parking_lot::Mutex;
use std::sync::Arc;

/// 打开时从备忘拷贝的只读快照。
#[derive(Debug, Clone)]
pub struct StickyNote {
    pub id: String,
    pub title: String,
    pub body_preview: String,
    /// 再次右键时请求聚焦已有窗口
    pub request_focus: bool,
    /// 用户关闭后置位；父侧下一帧移除（主窗最小化期间也可在回调里置位）
    pub closed: bool,
}

pub type StickyHandle = Arc<Mutex<StickyNote>>;

impl StickyNote {
    pub fn from_memo(m: &MemoView) -> StickyHandle {
        Arc::new(Mutex::new(Self {
            id: m.id.clone(),
            title: if m.title.trim().is_empty() {
                "(无标题)".into()
            } else {
                m.title.clone()
            },
            body_preview: body_preview(&m.content),
            request_focus: false,
            closed: false,
        }))
    }
}

fn body_preview(content: &str) -> String {
    let t = content.trim();
    const MAX: usize = 280;
    let count = t.chars().count();
    if count <= MAX {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(MAX).collect::<String>())
    }
}

/// 每帧注册 deferred 便签视口；已关闭的会跳过（由调用方随后移除）。
pub fn show_viewport(ctx: &egui::Context, note: &StickyHandle) {
    let (id_str, title, request_focus, closed) = {
        let n = note.lock();
        (
            n.id.clone(),
            n.title.clone(),
            n.request_focus,
            n.closed,
        )
    };
    if closed {
        return;
    }

    let vid = ViewportId::from_hash_of(("sticky", id_str.as_str()));
    let builder = ViewportBuilder::default()
        .with_title(format!("便签 · {title}"))
        .with_decorations(false)
        .with_always_on_top()
        .with_inner_size([280.0, 180.0])
        .with_min_inner_size([200.0, 120.0])
        .with_resizable(true);

    if request_focus {
        ctx.send_viewport_cmd_to(vid, ViewportCommand::Focus);
        note.lock().request_focus = false;
    }

    let note_cb = note.clone();
    ctx.show_viewport_deferred(vid, builder, move |ctx, class| {
        let mut note = note_cb.lock();
        if note.closed {
            return;
        }

        if class == ViewportClass::Embedded {
            let mut open = true;
            let title = note.title.clone();
            let id = note.id.clone();
            drop(note);
            egui::Window::new(format!("便签 · {title}"))
                .id(egui::Id::new(("sticky_embed", id.as_str())))
                .open(&mut open)
                .resizable(true)
                .default_size([280.0, 180.0])
                .show(ctx, |ui| {
                    let n = note_cb.lock();
                    paint_content(ui, &n, false);
                });
            if !open {
                note_cb.lock().closed = true;
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
                close_btn = paint_content(ui, &note, true);
            });

        if close_os || close_btn {
            note.closed = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    });
}

fn sticky_bg() -> Color32 {
    Color32::from_rgb(0xFF, 0xF6, 0xC8)
}

fn sticky_border() -> Color32 {
    Color32::from_rgb(0xE8, 0xD4, 0x6A)
}

/// 返回是否点了关闭。
fn paint_content(ui: &mut egui::Ui, note: &StickyNote, enable_drag: bool) -> bool {
    let panel = ui.max_rect();
    let hovered = ui.rect_contains_pointer(panel);
    if hovered {
        // 悬停进出时刷新，以便显示/隐藏关闭钮
        ui.ctx().request_repaint();
    }

    // 右侧预留关闭钮占位，避免标题被盖住
    const CLOSE_W: f32 = 28.0;
    ui.horizontal(|ui| {
        let max_w = (ui.available_width() - CLOSE_W).max(40.0);
        ui.set_max_width(max_w);
        ui.label(
            RichText::new(&note.title)
                .size(14.0)
                .strong()
                .color(Color32::from_rgb(0x3A, 0x32, 0x1A)),
        );
    });
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .id_source(("sticky_scroll", note.id.as_str()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(if note.body_preview.is_empty() {
                        "（无正文）"
                    } else {
                        note.body_preview.as_str()
                    })
                    .size(12.5)
                    .color(Color32::from_rgb(0x4A, 0x40, 0x28)),
                )
                .wrap(true),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new("按住任意处拖动 · 置顶提醒")
                    .size(10.5)
                    .color(theme::text_muted()),
            );
            let leftover = ui.available_height().max(24.0);
            ui.allocate_exact_size(Vec2::new(ui.available_width(), leftover), Sense::hover());
        });

    // 整窗拖动层（关闭钮后画、叠在上层，避免抢点击）
    let mut close = false;
    let mut on_close = false;
    if enable_drag {
        let drag = ui.interact(
            panel,
            ui.id().with("sticky_drag").with(note.id.as_str()),
            Sense::click_and_drag(),
        );
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
        if drag.drag_started() && !on_close && !close {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

    close
}
