//! FlowEditor：缓冲 / 光标 / 选区 / 嵌入原子 / 键盘与 IME。

use eframe::egui::{
    self, Color32, CursorIcon, Event, EventFilter, Id, Key, Pos2, Rect, Sense, Stroke, Vec2,
};

use crate::embed::{
    embed_at_slot, move_left_over_embeds, move_right_over_embeds, resync_embed_slots, FlowEmbed,
    FlowEmbedKind, EMBED_CHAR,
};
use crate::style::{self, StyleDelta, StyleRun};

/// 撤销快照（忽略嵌入高宽等易抖动字段，避免 Undo 匹配失败）。
#[derive(Clone, Debug, PartialEq, Default)]
struct FlowUndoSnap {
    text: String,
    style_runs: Vec<StyleRun>,
    /// (id, kind, slot) — 不含 height/width
    embeds: Vec<(u64, FlowEmbedKind, usize)>,
    cursor: usize,
}

impl FlowUndoSnap {
    fn from_state(state: &FlowState) -> Self {
        Self {
            text: state.text.clone(),
            style_runs: state.style_runs.clone(),
            embeds: state
                .embeds
                .iter()
                .map(|e| (e.id, e.kind, e.slot))
                .collect(),
            cursor: state.cursor,
        }
    }

    fn apply_to(self, state: &mut FlowState) {
        let hw: std::collections::HashMap<u64, (f32, f32)> = state
            .embeds
            .iter()
            .map(|e| (e.id, (e.height, e.width)))
            .collect();
        state.text = self.text;
        state.style_runs = self.style_runs;
        state.embeds = self
            .embeds
            .into_iter()
            .map(|(id, kind, slot)| {
                let mut e = FlowEmbed::new(id, kind, slot);
                if let Some(&(h, w)) = hw.get(&id) {
                    e.height = h;
                    e.width = w;
                }
                e
            })
            .collect();
        state.cursor = self.cursor.min(state.text.chars().count());
        state.ime_anchor = state.cursor;
        state.prefer_next_row = prefer_next_row_for_index(&state.text, state.cursor);
        state.clear_selection();
        state.has_ime = false;
    }
}

#[derive(Clone, Default)]
struct FlowUndoStack {
    undos: Vec<FlowUndoSnap>,
    redos: Vec<FlowUndoSnap>,
    /// 上一动作是否为连续打字（用于合并撤销点）
    typing: bool,
}

impl FlowUndoStack {
    const MAX: usize = 100;

    fn push_before_edit(&mut self, state: &FlowState, from_typing: bool) {
        let snap = FlowUndoSnap::from_state(state);
        if self.undos.last() == Some(&snap) {
            self.typing = from_typing;
            return;
        }
        // 连续打字只保留字串起点一个撤销点
        if from_typing && self.typing {
            self.typing = true;
            self.redos.clear();
            return;
        }
        self.undos.push(snap);
        if self.undos.len() > Self::MAX {
            self.undos.remove(0);
        }
        self.redos.clear();
        self.typing = from_typing;
    }

    fn undo(&mut self, state: &mut FlowState) -> bool {
        let cur = FlowUndoSnap::from_state(state);
        let Some(prev) = self.undos.pop() else {
            return false;
        };
        if prev == cur {
            // 栈顶即当前：再弹一层
            let Some(prev2) = self.undos.pop() else {
                self.undos.push(prev);
                return false;
            };
            self.redos.push(cur);
            prev2.apply_to(state);
        } else {
            self.redos.push(cur);
            prev.apply_to(state);
        }
        self.typing = false;
        true
    }

    fn redo(&mut self, state: &mut FlowState) -> bool {
        let Some(next) = self.redos.pop() else {
            return false;
        };
        self.undos.push(FlowUndoSnap::from_state(state));
        if self.undos.len() > Self::MAX {
            self.undos.remove(0);
        }
        next.apply_to(state);
        self.typing = false;
        true
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlowSelection {
    pub start: usize,
    pub end: usize,
}

impl FlowSelection {
    pub fn is_empty(self) -> bool {
        let (a, b) = self.sorted();
        a >= b
    }

    pub fn sorted(self) -> (usize, usize) {
        if self.start <= self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FlowState {
    pub text: String,
    pub style_runs: Vec<StyleRun>,
    pub embeds: Vec<FlowEmbed>,
    pub cursor: usize,
    pub selection: Option<FlowSelection>,
    pub scroll: f32,
    pub has_ime: bool,
    pub ime_anchor: usize,
    /// 与 egui `CCursor::prefer_next_row` 一致：软折行边界上决定光标画在上一行末还是下一行首。
    /// 插入仍用 `cursor` 下标；绘制必须用本标志，否则「点在下一行行首」会看起来对、输入却像在上一行。
    pub prefer_next_row: bool,
}

impl FlowState {
    pub fn from_plain(text: impl Into<String>) -> Self {
        let text = text.into();
        let len = text.chars().count();
        Self {
            text,
            style_runs: if len == 0 {
                Vec::new()
            } else {
                vec![StyleRun::plain(0, len)]
            },
            embeds: Vec::new(),
            cursor: len,
            selection: None,
            scroll: 0.0,
            has_ime: false,
            ime_anchor: len,
            prefer_next_row: true,
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn selected_text(&self) -> String {
        let Some(sel) = self.selection else {
            return String::new();
        };
        let (a, b) = sel.sorted();
        self.text
            .chars()
            .skip(a)
            .take(b.saturating_sub(a))
            .filter(|c| *c != EMBED_CHAR)
            .collect()
    }

    pub fn set_embed_height(&mut self, id: u64, height: f32) {
        if let Some(e) = self.embeds.iter_mut().find(|e| e.id == id) {
            e.height = height.max(24.0);
        }
    }

    pub fn set_embed_width(&mut self, id: u64, width: f32) {
        if let Some(e) = self.embeds.iter_mut().find(|e| e.id == id) {
            e.width = width.max(1.0);
        }
    }
}

#[derive(Debug)]
pub struct FlowOutput {
    pub changed: bool,
    pub response: egui::Response,
    pub selection: Option<FlowSelection>,
    pub cursor: usize,
    pub enter_pressed: bool,
    pub soft_enter: bool,
    /// 本帧绘制用的光标矩形（屏幕坐标）；宿主竖滚据此滚入。
    pub cursor_rect: Rect,
    /// 回车/上下键等本帧改了纵向光标位置 → 宿主应在竖滚层 ensure 可见。
    /// 不含鼠标滚轮；横向 ScrollArea 会吞掉内部 scroll_to_rect 的 Y 目标。
    pub ensure_caret_visible: bool,
    /// 本帧点击到的嵌入 id（表编辑态入口）。
    pub clicked_embed: Option<u64>,
    /// 本帧被删掉的嵌入 id。
    pub deleted_embeds: Vec<u64>,
    /// 嵌入占位屏幕矩形（宿主绘制表/图）。
    pub embed_rects: Vec<(u64, Rect)>,
    /// 指针下带链接的 URL（宿主 Ctrl+单击打开）。
    pub hovered_link: Option<String>,
}

/// 可视化片段。
enum VisSeg {
    Text {
        char_start: usize,
        galley: std::sync::Arc<egui::Galley>,
        rect: Rect,
    },
    Embed {
        id: u64,
        slot: usize,
        rect: Rect,
    },
}

pub struct FlowEditor;

impl FlowEditor {
    pub fn apply_patch(state: &mut FlowState, start: usize, end: usize, patch: StyleDelta) {
        let len = state.char_len();
        // 跳过嵌入槽上的样式
        style::apply_delta(&mut state.style_runs, len, start, end, &patch);
    }

    pub fn insert_embed(state: &mut FlowState, kind: FlowEmbedKind, id: u64) {
        let at = state.cursor.min(state.char_len());
        // 保证嵌入前后有换行，便于独占行排版
        let mut insert = String::new();
        let chars: Vec<char> = state.text.chars().collect();
        if at > 0 && chars.get(at - 1).copied() != Some('\n') {
            insert.push('\n');
        }
        let embed_local = insert.chars().count();
        insert.push(EMBED_CHAR);
        if at < chars.len() && chars.get(at).copied() != Some('\n') {
            insert.push('\n');
        }
        insert_text_at(state, at, &insert);
        let slot = at + embed_local;
        state.embeds.push(FlowEmbed::new(id, kind, slot));
        state.embeds.sort_by_key(|e| e.slot);
        state.cursor = slot + 1;
        if state.cursor < state.char_len()
            && state.text.chars().nth(state.cursor) == Some('\n')
        {
            state.cursor += 1;
        }
        state.clear_selection();
    }

    pub fn show(
        ui: &mut egui::Ui,
        id: Id,
        state: &mut FlowState,
        width: f32,
        min_rows: usize,
        hint: &str,
        toolbar_hit: bool,
    ) -> FlowOutput {
        let width = width.max(1.0);
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let row_h = ui.fonts(|f| f.row_height(&font_id));
        let min_h = row_h * min_rows.max(1) as f32;

        resync_embed_slots(&state.text, &mut state.embeds);

        let segs = build_vis_segs(ui, state, width);
        let total_h = segs
            .last()
            .map(|s| match s {
                VisSeg::Text { rect, .. } | VisSeg::Embed { rect, .. } => rect.bottom(),
            })
            .unwrap_or(0.0)
            .max(min_h);

        // 宽表占位可能超过正文折行宽，分配区要跟最宽嵌入对齐，否则右缘被裁
        let layout_w = segs
            .iter()
            .map(|s| match s {
                VisSeg::Text { rect, .. } | VisSeg::Embed { rect, .. } => rect.width(),
            })
            .fold(width, f32::max);
        let desired = Vec2::new(layout_w, total_h);
        let (full_id, rect) = ui.allocate_space(desired);
        let _ = full_id;
        // 把 seg 从局部坐标平移到 rect
        let segs: Vec<VisSeg> = segs
            .into_iter()
            .map(|s| match s {
                VisSeg::Text {
                    char_start,
                    galley,
                    rect: r,
                } => VisSeg::Text {
                    char_start,
                    galley,
                    rect: r.translate(rect.min.to_vec2()),
                },
                VisSeg::Embed { id, slot, rect: r } => VisSeg::Embed {
                    id,
                    slot,
                    rect: r.translate(rect.min.to_vec2()),
                },
            })
            .collect();

        let response = ui
            .interact(rect, id, Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::Text);

        let event_filter = EventFilter {
            tab: false,
            horizontal_arrows: true,
            vertical_arrows: true,
            escape: true,
        };

        let mut pointer_interacted = false;
        let mut clicked_embed: Option<u64> = None;
        let mut hovered_link: Option<String> = None;
        let drag_anchor_id = id.with("drag_anchor");
        let did_drag_id = id.with("did_drag");
        let preferred_x_id = id.with("preferred_x");
        let preferred_col_id = id.with("preferred_col");
        // 回车增高后同帧勿二次 interact 同 id（会丢焦点）。
        // sticky：短抖动；armed：点进编辑器后一直收键，直到点到别处或其它控件获焦。
        // 仅靠 2 帧 sticky 时，第一次回车后若丢焦，第二次回车（人手间隔）就不会进 handle_enter。
        let sticky_focus_id = id.with("sticky_kb_focus");
        let armed_id = id.with("kb_armed");
        let sticky_focus = ui
            .ctx()
            .data_mut(|d| d.get_temp::<u8>(sticky_focus_id))
            .unwrap_or(0);
        let mut armed = ui
            .ctx()
            .data(|d| d.get_temp::<bool>(armed_id))
            .unwrap_or(false);

        if !toolbar_hit {
            // 本帧若在打字/回车：禁止指针用「鼠标悬停旧点击位」覆盖键盘光标。
            // 实机 trace：restore session=18 后 kb begin=12，Enter 被钉回初次点击处。
            let key_owns_caret = ui.input(|i| {
                i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        Event::Key {
                            key: Key::Enter,
                            pressed: true,
                            ..
                        } | Event::Text(_)
                            | Event::Paste(_)
                    )
                })
            });
            // 松手必须清拖选，否则 is_being_dragged/did_drag 残留会每帧按悬停改光标
            if ui.input(|i| i.pointer.primary_released()) {
                ui.ctx().data_mut(|d| d.insert_temp(did_drag_id, false));
            }

            if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
                let (raw_idx, prefer_next, embed_hit) = cursor_from_pointer(&segs, pointer_pos);
                if embed_hit.is_none() {
                    hovered_link = link_at_char(state, raw_idx);
                }
                let primary_down = ui.input(|i| i.pointer.primary_down());
                let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
                let is_being_dragged =
                    ui.ctx().is_being_dragged(response.id) && primary_down;
                let shift = ui.input(|i| i.modifiers.shift);
                let did_drag = ui
                    .ctx()
                    .data(|d| d.get_temp::<bool>(did_drag_id))
                    .unwrap_or(false);

                // 点在嵌入上：单击交给宿主实表；拖选穿过时仍用 slot 扩展选区
                let on_embed = embed_hit.is_some();
                // 键盘占用插入点时：只更新悬停链接，不 place_caret
                if !key_owns_caret {
                    if on_embed {
                        if let Some(eid) = embed_hit {
                            clicked_embed = Some(eid);
                        }
                        if (is_being_dragged || did_drag) && primary_down {
                            ui.ctx().data_mut(|d| d.insert_temp(did_drag_id, true));
                            let anchor = ui
                                .ctx()
                                .data(|d| d.get_temp::<usize>(drag_anchor_id))
                                .unwrap_or(state.cursor);
                            if raw_idx != anchor {
                                state.selection = Some(FlowSelection {
                                    start: anchor,
                                    end: raw_idx,
                                });
                            }
                            let before = state.cursor;
                            place_caret(state, raw_idx, prefer_next, false);
                            pointer_interacted = true;
                            if before != state.cursor {
                                flow_caret_trace(format!(
                                    "ptr embed-drag {} -> {} raw={}",
                                    before, state.cursor, raw_idx
                                ));
                            }
                        } else if response.hovered() && primary_pressed {
                            // 单击表区：不抢焦点；记下槽旁光标便于 Backspace 删表
                            place_caret(state, raw_idx, prefer_next, true);
                            state.clear_selection();
                            ui.ctx().data_mut(|d| d.remove_temp::<f32>(preferred_x_id));
                        }
                    } else if response.hovered()
                        && (primary_pressed || response.drag_started())
                    {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(did_drag_id, false);
                            d.remove_temp::<f32>(preferred_x_id);
                            d.remove_temp::<usize>(preferred_col_id);
                        });
                        // Word：按下即定插入点（含 prefer_next_row）；列表前缀夹到正文
                        let idx = crate::list::clamp_cursor_to_list_body(&state.text, raw_idx);
                        let before = state.cursor;
                        if shift {
                            let anchor = state.selection.map(|s| s.start).unwrap_or(state.cursor);
                            state.selection = Some(FlowSelection {
                                start: anchor,
                                end: idx,
                            });
                            place_caret(state, idx, prefer_next, true);
                            ui.ctx()
                                .data_mut(|d| d.insert_temp(drag_anchor_id, anchor));
                        } else {
                            place_caret(state, idx, prefer_next, true);
                            state.clear_selection();
                            ui.ctx()
                                .data_mut(|d| d.insert_temp(drag_anchor_id, idx));
                        }
                        pointer_interacted = true;
                        flow_caret_trace(format!(
                            "ptr press {} -> {} raw={}",
                            before, state.cursor, raw_idx
                        ));
                    }

                    if !on_embed && is_being_dragged && primary_down {
                        ui.ctx().data_mut(|d| d.insert_temp(did_drag_id, true));
                        let anchor = ui
                            .ctx()
                            .data(|d| d.get_temp::<usize>(drag_anchor_id))
                            .unwrap_or(state.cursor);
                        // 拖选也必须夹紧并同步 ime_anchor，否则按下已夹紧、拖动用裸下标，
                        // 绘制/回车会落到不同位置。
                        let idx = crate::list::clamp_cursor_to_list_body(&state.text, raw_idx);
                        if idx != anchor {
                            state.selection = Some(FlowSelection {
                                start: anchor,
                                end: idx,
                            });
                        } else if !shift {
                            state.clear_selection();
                        }
                        let before = state.cursor;
                        place_caret(state, idx, prefer_next, true);
                        pointer_interacted = true;
                        if before != state.cursor {
                            flow_caret_trace(format!(
                                "ptr drag {} -> {} raw={}",
                                before, state.cursor, raw_idx
                            ));
                        }
                    }

                    if !on_embed && response.clicked() && !shift {
                        let did_drag = ui
                            .ctx()
                            .data(|d| d.get_temp::<bool>(did_drag_id))
                            .unwrap_or(false);
                        if !did_drag {
                            let idx = crate::list::clamp_cursor_to_list_body(&state.text, raw_idx);
                            let before = state.cursor;
                            place_caret(state, idx, prefer_next, true);
                            state.clear_selection();
                            ui.ctx().data_mut(|d| d.remove_temp::<f32>(preferred_x_id));
                            pointer_interacted = true;
                            flow_caret_trace(format!(
                                "ptr click {} -> {} raw={}",
                                before, state.cursor, raw_idx
                            ));
                        }
                    }

                    if !on_embed && response.double_clicked() {
                        let (a, b) = select_word_char_range(&state.text, raw_idx);
                        state.selection = Some(FlowSelection { start: a, end: b });
                        place_caret(state, b, true, true);
                        ui.ctx().data_mut(|d| d.remove_temp::<f32>(preferred_x_id));
                        pointer_interacted = true;
                    }
                } else if on_embed {
                    if let Some(eid) = embed_hit {
                        clicked_embed = Some(eid);
                    }
                }

                if pointer_interacted {
                    armed = true;
                    response.request_focus();
                }
            }
        }

        // 指针落点已定：在收键前同步插入点（唯一权威 = state.cursor）
        if pointer_interacted {
            sync_insert_caret(state);
        }

        // 点在编辑器外：解除武装，避免抢走标题等单行框的 Enter
        if ui.input(|i| i.pointer.primary_clicked()) && !response.hovered() {
            armed = false;
        }
        let other_focused = ui.memory(|m| m.focused().is_some_and(|f| f != id));
        // 本帧刚点进编辑器时旧焦点可能尚未卸掉，勿清 armed
        if other_focused && !pointer_interacted {
            armed = false;
        }
        // 主窗口失焦（点到其它 OS 窗口）：去焦点，清 armed/sticky，停收键与 IME
        let viewport_focused = ui.input(|i| i.viewport().focused.unwrap_or(true));
        if !viewport_focused {
            armed = false;
            ui.ctx().data_mut(|d| {
                d.insert_temp(sticky_focus_id, 0u8);
            });
            if ui.memory(|m| m.has_focus(id)) {
                ui.memory_mut(|m| m.surrender_focus(id));
            }
        }

        let focused = response.has_focus() || pointer_interacted;
        let kb_focus = viewport_focused
            && !toolbar_hit
            && !other_focused
            && (focused || armed || sticky_focus > 0 || ui.memory(|m| m.has_focus(id)));
        if kb_focus {
            armed = true;
            ui.memory_mut(|m| m.set_focus_lock_filter(id, event_filter));
            // 已有焦点时勿每帧 request_focus，否则回车会闪、像光标被拽回行头
            if !ui.memory(|m| m.has_focus(id)) {
                response.request_focus();
                ui.memory_mut(|m| m.request_focus(id));
            }
        }
        // 窗口失焦时上面已 surrender，且 kb_focus=false，不会再 request_focus
        // 粘性焦点每帧减 1，回车后约 2 帧内即使 has_focus 抖动仍收键
        if sticky_focus > 0 && viewport_focused {
            ui.ctx().data_mut(|d| {
                d.insert_temp(sticky_focus_id, sticky_focus.saturating_sub(1));
            });
        }
        ui.ctx().data_mut(|d| d.insert_temp(armed_id, armed));

        // 先处理键盘再绘制，保证光标显示与插入位置同一帧一致
        let mut segs = segs;
        let mut changed = false;
        let mut enter_pressed = false;
        let mut soft_enter = false;
        // 仅回车/上下键触发「离开可视区则滚入」；鼠标滚轮不置位、不抢滚动
        let mut ensure_caret_visible = false;
        let mut deleted_embeds = Vec::new();
        let embeds_before: Vec<u64> = state.embeds.iter().map(|e| e.id).collect();

        let undo_id = id.with("flow_undo_stack");
        let mut undo_stack: FlowUndoStack = ui
            .ctx()
            .data(|d| d.get_temp(undo_id))
            .unwrap_or_default();

        // kb_focus 已在上方计算（memory 与 response 取或）
        if kb_focus && !toolbar_hit {
            let (want_undo, want_redo) = ui.input(|i| {
                let undo = i.modifiers.command
                    && !i.modifiers.shift
                    && i.key_pressed(Key::Z);
                let redo = (i.modifiers.command && i.key_pressed(Key::Y))
                    || (i.modifiers.command && i.modifiers.shift && i.key_pressed(Key::Z));
                (undo, redo)
            });
            if want_undo && undo_stack.undo(state) {
                changed = true;
            } else if want_redo && undo_stack.redo(state) {
                changed = true;
            }
        }

        if kb_focus && !toolbar_hit {
            // 插入点唯一权威 = state.cursor
            let events = ui.input(|i| i.filtered_events(&event_filter));
            // Windows IME：Commit/Preedit 与 Key::Enter 同帧时，必须先消化合成再处理回车，
            // 否则可能先换行再被 CompositionEnd 拽回旧锚点。
            let (ime_events, other_events): (Vec<_>, Vec<_>) =
                events.into_iter().partition(|ev| {
                    matches!(
                        ev,
                        Event::CompositionStart
                            | Event::CompositionUpdate(_)
                            | Event::CompositionEnd(_)
                    )
                });
            // 本帧是否已处理「真·合成提交」（空 End 且无预编辑不拦回车）
            let mut ime_committed_this_frame = false;
            // 同帧内 Enter 只处理一次（egui 多 pass / 重复 Key 事件）
            let frame_t = ui.input(|i| i.time);
            let enter_once_id = id.with("enter_once_t");
            let enter_already = ui
                .ctx()
                .data(|d| d.get_temp::<f64>(enter_once_id))
                .is_some_and(|t| (t - frame_t).abs() < 1e-9);
            flow_caret_trace(format!(
                "kb begin cursor={} ime={} has_ime={} events={} enter_already={}",
                state.cursor,
                state.ime_anchor,
                state.has_ime,
                ime_events.len() + other_events.len(),
                enter_already
            ));
            for ev in ime_events.into_iter().chain(other_events) {
                match ev {
                    Event::Key {
                        key: Key::Z | Key::Y,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.command => {}
                    Event::Text(t) => {
                        // 真合成中（有预编辑选区）：忽略 Text，避免与 Composition 双写。
                        // 残留 has_ime / 锚点过期：清标志后照常插入，绝不 continue 吞字。
                        if state.has_ime {
                            if ime_preedit_range_active(state) {
                                continue;
                            }
                            state.has_ime = false;
                            state.ime_anchor = state.cursor;
                            state.clear_selection();
                        }
                        if !t.is_empty() && t != "\n" && t != "\r" && !t.contains(EMBED_CHAR) {
                            let before = state.cursor;
                            undo_stack.push_before_edit(state, true);
                            insert_text(state, &t);
                            state.ime_anchor = state.cursor;
                            ui.ctx().data_mut(|d| d.remove_temp::<usize>(preferred_col_id));
                            changed = true;
                            flow_caret_trace(format!(
                                "Text {:?} at {} -> {}",
                                t.chars().take(8).collect::<String>(),
                                before,
                                state.cursor
                            ));
                        }
                    }
                    Event::Paste(t) => {
                        if state.has_ime {
                            clear_ime_preedit(state);
                        }
                        undo_stack.push_before_edit(state, false);
                        insert_text(state, &t.replace('\r', "").replace(EMBED_CHAR, ""));
                        changed = true;
                    }
                    Event::Copy => {
                        let t = state.selected_text();
                        if !t.is_empty() {
                            ui.ctx().copy_text(t);
                        }
                    }
                    Event::Cut => {
                        if let Some(sel) = state.selection {
                            if !sel.is_empty() {
                                let (a, b) = sel.sorted();
                                ui.ctx().copy_text(state.selected_text());
                                undo_stack.push_before_edit(state, false);
                                delete_range(state, a, b);
                                changed = true;
                            }
                        }
                    }
                    Event::Key {
                        key: Key::Enter,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        // 合成中回车只确认输入法，不插换行（避免清空预编辑后再 Enter 造成上方空行）
                        let gate = enter_gate(state, ime_committed_this_frame);
                        flow_caret_trace(format!(
                            "Enter gate={:?} cursor={} already={}",
                            gate, state.cursor, enter_already
                        ));
                        if enter_already {
                            // 同帧已处理过回车
                        } else if matches!(gate, EnterGate::Newline) {
                            let before = state.cursor;
                            apply_enter_key(
                                state,
                                &mut undo_stack,
                                ui,
                                sticky_focus_id,
                                armed_id,
                                preferred_x_id,
                                preferred_col_id,
                                modifiers.shift,
                                &mut enter_pressed,
                                &mut soft_enter,
                                &mut changed,
                            );
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(enter_once_id, frame_t);
                                d.insert_temp(did_drag_id, false);
                            });
                            // 回车后强制夺回焦点（CJK IME 确定后 egui 可能丢焦，看起来像光标跳走）
                            response.request_focus();
                            ui.memory_mut(|m| m.request_focus(id));
                            flow_caret_trace(format!(
                                "Enter applied {} -> {}",
                                before, state.cursor
                            ));
                        }
                    }
                    Event::Key {
                        key: Key::Backspace,
                        pressed: true,
                        ..
                    } => {
                        if state.has_ime {
                            clear_ime_preedit(state);
                        }
                        if state.selection.is_some_and(|s| !s.is_empty()) || state.cursor > 0 {
                            undo_stack.push_before_edit(state, false);
                            if crate::list::handle_backspace_list(state) || backspace(state) {
                                changed = true;
                            }
                        }
                    }
                    Event::Key {
                        key: Key::Delete,
                        pressed: true,
                        ..
                    } => {
                        if state.has_ime {
                            clear_ime_preedit(state);
                        }
                        if state.selection.is_some_and(|s| !s.is_empty())
                            || state.cursor < state.char_len()
                        {
                            undo_stack.push_before_edit(state, false);
                            if delete_forward(state) {
                                changed = true;
                            }
                        }
                    }
                    Event::Key {
                        key: Key::A,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.command => {
                        let len = state.char_len();
                        state.selection = Some(FlowSelection { start: 0, end: len });
                        state.cursor = len;
                        state.has_ime = false;
                        state.ime_anchor = len;
                    }
                    Event::Key {
                        key: Key::ArrowLeft,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if state.has_ime {
                            clear_ime_preedit(state);
                        }
                        // 方向键始终移动逻辑光标，并结束 IME（见 set_cursor）；清拖选残留
                        ui.ctx().data_mut(|d| {
                            d.remove_temp::<f32>(preferred_x_id);
                            d.remove_temp::<usize>(preferred_col_id);
                            d.insert_temp(did_drag_id, false);
                        });
                        let body = crate::list::line_body_start(&state.text, state.cursor);
                        let next = if state.cursor <= body {
                            // 正文起点再左：跳过前缀，落到上一行行末
                            let (ls, _) = crate::list::line_at_cursor(&state.text, state.cursor);
                            if ls == 0 {
                                body
                            } else {
                                crate::list::clamp_cursor_to_list_body(&state.text, ls - 1)
                            }
                        } else {
                            let n = move_left_over_embeds(&state.embeds, state.cursor);
                            crate::list::clamp_cursor_to_list_body(&state.text, n)
                        };
                        set_cursor(state, next, modifiers.shift);
                    }
                    Event::Key {
                        key: Key::ArrowRight,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if state.has_ime {
                            clear_ime_preedit(state);
                        }
                        ui.ctx().data_mut(|d| {
                            d.remove_temp::<f32>(preferred_x_id);
                            d.remove_temp::<usize>(preferred_col_id);
                            d.insert_temp(did_drag_id, false);
                        });
                        let next =
                            move_right_over_embeds(&state.embeds, state.cursor, state.char_len());
                        let next = crate::list::clamp_cursor_to_list_body(&state.text, next);
                        set_cursor(state, next, modifiers.shift);
                    }
                    Event::Key {
                        key: Key::ArrowUp,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if state.has_ime {
                            clear_ime_preedit(state);
                        }
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(did_drag_id, false));
                        let mut pref = ui.ctx().data(|d| d.get_temp::<usize>(preferred_col_id));
                        let next = crate::list::move_vert_logical(
                            &state.text,
                            state.cursor,
                            -1,
                            &mut pref,
                        );
                        ui.ctx().data_mut(|d| {
                            if let Some(c) = pref {
                                d.insert_temp(preferred_col_id, c);
                            }
                            d.remove_temp::<f32>(preferred_x_id);
                        });
                        set_cursor(state, next, modifiers.shift);
                        ensure_caret_visible = true;
                    }
                    Event::Key {
                        key: Key::ArrowDown,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if state.has_ime {
                            clear_ime_preedit(state);
                        }
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(did_drag_id, false));
                        let mut pref = ui.ctx().data(|d| d.get_temp::<usize>(preferred_col_id));
                        let next = crate::list::move_vert_logical(
                            &state.text,
                            state.cursor,
                            1,
                            &mut pref,
                        );
                        ui.ctx().data_mut(|d| {
                            if let Some(c) = pref {
                                d.insert_temp(preferred_col_id, c);
                            }
                            d.remove_temp::<f32>(preferred_x_id);
                        });
                        set_cursor(state, next, modifiers.shift);
                        ensure_caret_visible = true;
                    }
                    Event::Key {
                        key: Key::Home,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if state.has_ime {
                            clear_ime_preedit(state);
                        }
                        ui.ctx().data_mut(|d| {
                            d.remove_temp::<f32>(preferred_x_id);
                            d.remove_temp::<usize>(preferred_col_id);
                        });
                        let body = crate::list::line_body_start(&state.text, state.cursor);
                        set_cursor(state, body, modifiers.shift);
                    }
                    Event::Key {
                        key: Key::End,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if state.has_ime {
                            clear_ime_preedit(state);
                        }
                        ui.ctx().data_mut(|d| {
                            d.remove_temp::<f32>(preferred_x_id);
                            d.remove_temp::<usize>(preferred_col_id);
                        });
                        let (_, le) = crate::list::line_at_cursor(&state.text, state.cursor);
                        set_cursor(state, le, modifiers.shift);
                    }
                    Event::CompositionStart => {
                        // 只认当前 cursor 为合成锚点；清选区高亮，不 delete_range（避免 cursor→sel.start）
                        undo_stack.push_before_edit(state, false);
                        state.clear_selection();
                        state.has_ime = true;
                        state.ime_anchor = state.cursor.min(state.char_len());
                    }
                    Event::CompositionUpdate(preedit) => {
                        if apply_composition_update(state, &preedit) {
                            changed = true;
                        }
                    }
                    Event::CompositionEnd(committed) => {
                        // 仅「真有合成」时才拦同帧 Enter；裸空 End 不拦（否则方向键后回车被吞）
                        // 中文热路径经 sanitize 已变成 Text，此处主要服务未 sanitize 环境。
                        let had_composition = ime_preedit_range_active(state)
                            || (!committed.is_empty() && state.has_ime);
                        if apply_composition_end(state, &committed) {
                            changed = true;
                        }
                        if had_composition {
                            ime_committed_this_frame = true;
                        }
                    }
                    _ => {}
                }
            }
            // 不再用 key_pressed(Enter) 兜底：合成中若事件循环 Ignore 了 Enter，
            // 兜底会无视 has_ime 再插一次换行，把预编辑挤到下一行。
        }

        // IME 区：回车增高后扩大几何；切勿对同一 id 再 interact（会丢焦点，第二次回车光标卡住）
        let mut ime_rect = rect;
        let mut response = response;

        if changed || enter_pressed {
            resync_embed_slots(&state.text, &mut state.embeds);
            let keep: std::collections::HashSet<u64> =
                state.embeds.iter().map(|e| e.id).collect();
            for id in embeds_before {
                if !keep.contains(&id) {
                    deleted_embeds.push(id);
                }
            }
            // 文本变了：按新内容重排，光标绘制才跟得上
            let rebuilt = build_vis_segs(ui, state, width);
            segs = rebuilt
                .into_iter()
                .map(|s| match s {
                    VisSeg::Text {
                        char_start,
                        galley,
                        rect: r,
                    } => VisSeg::Text {
                        char_start,
                        galley,
                        rect: r.translate(rect.min.to_vec2()),
                    },
                    VisSeg::Embed { id, slot, rect: r } => VisSeg::Embed {
                        id,
                        slot,
                        rect: r.translate(rect.min.to_vec2()),
                    },
                })
                .collect();
            // 回车等会增高：本帧补占位，避免新行落在已分配 rect 外被裁切、光标点不中
            let new_bottom = segs
                .last()
                .map(|s| match s {
                    VisSeg::Text { rect: r, .. } | VisSeg::Embed { rect: r, .. } => r.bottom(),
                })
                .unwrap_or(rect.top());
            let extra_h = (new_bottom - rect.bottom()).max(0.0);
            if extra_h > 0.5 {
                let _ = ui.allocate_space(Vec2::new(0.0, extra_h));
            }
            ime_rect = Rect::from_min_max(
                rect.min,
                Pos2::new(rect.max.x.max(ime_rect.max.x), new_bottom.max(rect.max.y)),
            );
            // 只扩大 IME 几何；勿在此 request_focus（回车增高时会闪）
        }

        let len = state.char_len();
        if let Some(sel) = state.selection.as_mut() {
            sel.start = sel.start.min(len);
            sel.end = sel.end.min(len);
        }
        if state.selection.is_some_and(|s| s.is_empty()) {
            state.selection = None;
        }
        // 最终光标勿停在列表前缀内（与绘制/下一帧插入一致）
        let before = state.cursor;
        sync_insert_caret(state);
        if state.cursor != before {
            // 夹紧挪了下标时，按新位置重算软折行偏好
            state.prefer_next_row = prefer_next_row_for_index(&state.text, state.cursor);
        }

        ui.ctx()
            .data_mut(|d| d.insert_temp(undo_id, undo_stack));

        // 绘制（光标已是本帧最终位置）
        if state.text.is_empty() && state.embeds.is_empty() && !state.has_ime && !hint.is_empty() {
            ui.painter().text(
                rect.left_top() + Vec2::new(0.0, 2.0),
                egui::Align2::LEFT_TOP,
                hint,
                font_id.clone(),
                ui.visuals().weak_text_color(),
            );
        }

        let mut embed_rects = Vec::new();
        for seg in &segs {
            match seg {
                VisSeg::Text { galley, rect: r, .. } => {
                    ui.painter()
                        .galley(r.min, galley.clone(), ui.visuals().text_color());
                }
                VisSeg::Embed { id, rect: r, .. } => {
                    embed_rects.push((*id, *r));
                }
            }
        }

        if let Some(sel) = state.selection {
            let (a, b) = sel.sorted();
            if a < b {
                paint_selection_segs(ui, &segs, a, b);
            }
        }

        let primary_cursor_rect =
            cursor_rect_from_segs(&segs, state.cursor, state.prefer_next_row, row_h);
        // 与收键一致用 kb_focus：回车后 has_focus 抖动时仍发布 IME，否则只能鼠标再点才能输入
        if kb_focus && !toolbar_hit {
            let blink = ui.input(|i| (i.time * 2.0) as i64 % 2 == 0);
            if blink || state.has_ime {
                ui.painter().line_segment(
                    [
                        Pos2::new(primary_cursor_rect.left(), primary_cursor_rect.top()),
                        Pos2::new(primary_cursor_rect.left(), primary_cursor_rect.bottom()),
                    ],
                    Stroke::new(1.0, ui.visuals().strong_text_color()),
                );
            }
            ui.ctx().output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect: ime_rect,
                    cursor_rect: primary_cursor_rect,
                });
            });
        }

        // 回车也算纵向移动；真正 scroll 由宿主在竖滚层做（内层横滚会吞掉 Y 目标）
        ensure_caret_visible |= enter_pressed || soft_enter;

        if changed {
            response.mark_changed();
        }
        // 光标闪烁：定时重绘即可，勿每帧 request_repaint（与回车叠加会闪屏）
        if kb_focus {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }

        // 带半行 padding，宿主滚入时不贴边
        let pad = row_h * 0.5;
        let cursor_rect = Rect::from_min_max(
            Pos2::new(
                primary_cursor_rect.left(),
                primary_cursor_rect.top() - pad,
            ),
            Pos2::new(
                primary_cursor_rect
                    .right()
                    .max(primary_cursor_rect.left() + 1.0),
                primary_cursor_rect.bottom() + pad,
            ),
        );

        FlowOutput {
            changed: changed || enter_pressed || soft_enter,
            response,
            selection: state.selection,
            cursor: state.cursor,
            enter_pressed,
            soft_enter,
            cursor_rect,
            ensure_caret_visible,
            clicked_embed,
            deleted_embeds,
            embed_rects,
            hovered_link,
        }
    }
}

fn link_at_char(state: &FlowState, idx: usize) -> Option<String> {
    let idx = idx.min(state.char_len().saturating_sub(1));
    state
        .style_runs
        .iter()
        .find(|r| r.start <= idx && idx < r.end)
        .and_then(|r| r.link.clone())
}

fn build_vis_segs(ui: &egui::Ui, state: &FlowState, width: f32) -> Vec<VisSeg> {
    let chars: Vec<char> = state.text.chars().collect();
    let mut segs = Vec::new();
    let mut y = 0.0f32;
    let mut i = 0usize;

    while i < chars.len() {
        if chars[i] == EMBED_CHAR {
            let emb = embed_at_slot(&state.embeds, i);
            let h = emb.map(|e| e.height).unwrap_or(100.0);
            let w = emb
                .map(|e| if e.width > 0.0 { e.width } else { width })
                .unwrap_or(width);
            let id = emb.map(|e| e.id).unwrap_or(0);
            let r = Rect::from_min_size(Pos2::new(0.0, y), Vec2::new(w, h));
            segs.push(VisSeg::Embed {
                id,
                slot: i,
                rect: r,
            });
            y += h + 2.0;
            i += 1;
            // 吃掉嵌入后紧跟的单个换行，避免双空隙
            if i < chars.len() && chars[i] == '\n' {
                i += 1;
            }
            continue;
        }

        // 文本直到下一个嵌入或文末
        let start = i;
        while i < chars.len() && chars[i] != EMBED_CHAR {
            i += 1;
        }
        let slice: String = chars[start..i].iter().collect();
        // 嵌入前的分隔 `\n` 不画（嵌入自己占行）；文末尾部 `\n` 必须保留，
        // 否则空最后一行没有视觉行，上下键/光标会停在旧位置。
        let draw = if i < chars.len() && chars[i] == EMBED_CHAR {
            slice.trim_end_matches('\n')
        } else {
            slice.as_str()
        };
        let job = build_layout_job_range(ui, state, start, start + draw.chars().count(), width);
        let galley = ui.fonts(|f| f.layout_job(job));
        let gh = galley.size().y.max(ui.spacing().interact_size.y * 0.5);
        let r = Rect::from_min_size(Pos2::new(0.0, y), Vec2::new(width, gh));
        segs.push(VisSeg::Text {
            char_start: start,
            galley,
            rect: r,
        });
        y += gh;
        if i < chars.len() && chars[i] == EMBED_CHAR {
            let nl_count = slice.chars().count() - draw.chars().count();
            if nl_count > 0 {
                y += 1.0;
            }
        }
    }

    if segs.is_empty() {
        let job = build_layout_job_range(ui, state, 0, 0, width);
        let galley = ui.fonts(|f| f.layout_job(job));
        let gh = galley.size().y.max(20.0);
        segs.push(VisSeg::Text {
            char_start: 0,
            galley,
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(width, gh)),
        });
    }
    segs
}

/// 指针 → (绝对字符下标, prefer_next_row, 命中的嵌入 id)。
/// `prefer_next_row` 原样取自 egui galley，与 Word/TextEdit 软折行落点一致。
fn cursor_from_pointer(segs: &[VisSeg], pos: Pos2) -> (usize, bool, Option<u64>) {
    for seg in segs {
        if let VisSeg::Embed { id, slot, rect } = seg {
            if rect.contains(pos) {
                // 左半 = 槽前(=slot)，右半 = 槽后
                let idx = if pos.x < rect.center().x {
                    *slot
                } else {
                    *slot + 1
                };
                return (idx, true, Some(*id));
            }
        }
    }

    let mut best: Option<(f32, usize, bool)> = None;
    for seg in segs {
        if let VisSeg::Text {
            char_start,
            galley,
            rect,
        } = seg
        {
            let within = pos.y >= rect.top() - 1.0 && pos.y <= rect.bottom() + 1.0;
            let y_dist = if pos.y < rect.top() {
                rect.top() - pos.y
            } else if pos.y > rect.bottom() {
                pos.y - rect.bottom()
            } else {
                0.0
            };
            if within || best.as_ref().map_or(true, |(d, ..)| y_dist < *d) {
                let local = pos - rect.min;
                let cur = galley.cursor_from_pos(local);
                let idx = char_start + cur.ccursor.index;
                let prefer = cur.ccursor.prefer_next_row;
                if within {
                    return (idx, prefer, None);
                }
                best = Some((y_dist, idx, prefer));
            }
        }
    }
    if let Some((_, idx, prefer)) = best {
        return (idx, prefer, None);
    }
    // 落在空隙：文末
    if let Some(last) = segs.last() {
        match last {
            VisSeg::Text {
                char_start, galley, ..
            } => {
                let n = galley.text().chars().count();
                return (
                    char_start + n,
                    prefer_next_row_for_index(galley.text(), n),
                    None,
                );
            }
            VisSeg::Embed { slot, .. } => return (*slot + 1, true, None),
        }
    }
    (0, true, None)
}

fn cursor_rect_from_segs(segs: &[VisSeg], cursor: usize, prefer_next_row: bool, row_h: f32) -> Rect {
    for seg in segs {
        match seg {
            VisSeg::Embed { slot, rect, .. } => {
                // 槽旁光标用行高，避免整表高竖线
                if cursor == *slot {
                    return Rect::from_min_size(
                        Pos2::new(rect.left(), rect.top()),
                        Vec2::new(1.0, row_h),
                    );
                }
                if cursor == *slot + 1 {
                    return Rect::from_min_size(
                        Pos2::new(rect.right(), rect.top()),
                        Vec2::new(1.0, row_h),
                    );
                }
            }
            VisSeg::Text {
                char_start,
                galley,
                rect,
            } => {
                let local_len = galley.text().chars().count();
                if cursor >= *char_start && cursor <= *char_start + local_len {
                    let local = cursor - char_start;
                    // Word：用 CCursor→RCursor 取几何。勿走 pos_from_cursor（内部用
                    // pcursor，空段 `|| ends_with_newline` 会把靠后的下标画到靠前的空行）。
                    let ccursor = egui::text::CCursor {
                        index: local,
                        prefer_next_row,
                    };
                    let c = galley.from_ccursor(ccursor);
                    let r = galley.pos_from_rcursor(c.rcursor);
                    return Rect::from_min_max(
                        rect.min + r.min.to_vec2(),
                        rect.min + r.max.to_vec2(),
                    );
                }
            }
        }
    }
    segs.last()
        .map(|s| match s {
            VisSeg::Text { rect, .. } | VisSeg::Embed { rect, .. } => Rect::from_min_size(
                Pos2::new(rect.left(), rect.bottom() - row_h),
                Vec2::new(1.0, row_h),
            ),
        })
        .unwrap_or(Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, row_h)))
}

fn paint_selection_segs(ui: &egui::Ui, segs: &[VisSeg], start: usize, end: usize) {
    let color = Color32::from_rgba_unmultiplied(0x2F, 0x6F, 0xED, 48);
    for seg in segs {
        match seg {
            // 表/图原子：不铺整块背景
            VisSeg::Embed { .. } => {}
            VisSeg::Text {
                char_start,
                galley,
                rect,
            } => {
                let local_len = galley.text().chars().count();
                let seg_end = char_start + local_len;
                if end <= *char_start || start >= seg_end {
                    continue;
                }
                let sel_a = start.max(*char_start) - char_start;
                let sel_b = end.min(seg_end) - char_start;
                if sel_a >= sel_b {
                    continue;
                }
                // 按视觉行高亮，避免跨行变成整块包围盒
                let mut off = 0usize;
                for row in galley.rows.iter() {
                    let n_incl = row.char_count_including_newline();
                    // 高亮不含行尾换行本身
                    let n_text = if n_incl > 0 && row.ends_with_newline {
                        n_incl - 1
                    } else {
                        n_incl
                    };
                    let row_a = off;
                    let row_b = off + n_text;
                    let a = sel_a.max(row_a);
                    let b = sel_b.min(row_b);
                    if a < b {
                        let c0 = galley.from_ccursor(egui::text::CCursor::new(a));
                        let c1 = galley.from_ccursor(egui::text::CCursor::new(b));
                        let r0 = galley.pos_from_cursor(&c0);
                        let r1 = galley.pos_from_cursor(&c1);
                        let y0 = rect.min.y + r0.min.y.min(r1.min.y);
                        let y1 = rect.min.y + r0.max.y.max(r1.max.y);
                        let x0 = rect.min.x + r0.min.x.min(r1.min.x);
                        let x1 = rect.min.x + r0.max.x.max(r1.max.x);
                        if x1 > x0 && y1 > y0 {
                            ui.painter().rect_filled(
                                Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1)),
                                0.0,
                                color,
                            );
                        }
                    }
                    off += n_incl;
                }
            }
        }
    }
}

fn flow_caret_trace(msg: impl AsRef<str>) {
    if std::env::var_os("MEMO_CARET_TRACE").is_none() {
        return;
    }
    let path = std::env::temp_dir().join("memo_caret_trace.log");
    let line = format!("{}\n", msg.as_ref());
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.as_bytes())
        });
}

/// 回车门控：仅「真预编辑」或同帧真提交 → Ignore。
/// 残留 `has_ime`（无匹配预编辑选区）一律清掉后 Newline——不依赖易脏的合成标志挡换行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnterGate {
    Ignore,
    Newline,
}

fn enter_gate(state: &mut FlowState, ime_committed_this_frame: bool) -> EnterGate {
    if ime_committed_this_frame {
        return EnterGate::Ignore;
    }
    if ime_preedit_range_active(state) {
        // 真合成中：回车只确认输入法，等 CompositionEnd（sanitize 中文路径不会走到这里）
        return EnterGate::Ignore;
    }
    // 残留 has_ime / 过期锚点：清掉后允许换行，插入点仍是 state.cursor
    if state.has_ime {
        state.has_ime = false;
        state.ime_anchor = state.cursor;
        state.clear_selection();
    }
    EnterGate::Newline
}

/// 是否存在「锚点..光标」且选区与之重合的预编辑区间（真·合成中）。
fn ime_preedit_range_active(state: &FlowState) -> bool {
    if !state.has_ime {
        return false;
    }
    let anchor = state.ime_anchor.min(state.char_len());
    let end = state.cursor.min(state.char_len());
    if end <= anchor {
        return false;
    }
    state.selection.is_some_and(|s| {
        let (a, b) = s.sorted();
        a == anchor && b == end
    })
}

/// 处理 CompositionUpdate：用预编辑替换 [ime_anchor, cursor)。
/// 返回是否改动了正文。
fn apply_composition_update(state: &mut FlowState, preedit: &str) -> bool {
    if preedit == "\n" || preedit == "\r" {
        return false;
    }
    // 空预编辑：只结束合成标志，不删字、不拽光标。
    if preedit.is_empty() {
        if !state.has_ime {
            return false;
        }
        state.has_ime = false;
        state.ime_anchor = state.cursor.min(state.char_len());
        state.clear_selection();
        return true;
    }
    // 仅在「已有匹配选区的预编辑」时替换；否则从当前光标新开，绝不误删正文
    if ime_preedit_range_active(state) {
        let anchor = state.ime_anchor.min(state.char_len());
        let end = state.cursor.min(state.char_len()).max(anchor);
        delete_range(state, anchor, end);
        insert_text_at(state, anchor, preedit);
        state.cursor = anchor + preedit.chars().count();
        state.selection = Some(FlowSelection {
            start: anchor,
            end: state.cursor,
        });
        state.ime_anchor = anchor;
    } else {
        let at = state.cursor.min(state.char_len());
        state.has_ime = true;
        state.ime_anchor = at;
        insert_text_at(state, at, preedit);
        state.cursor = at + preedit.chars().count();
        state.selection = Some(FlowSelection {
            start: at,
            end: state.cursor,
        });
        state.ime_anchor = at;
    }
    state.prefer_next_row = prefer_next_row_for_index(&state.text, state.cursor);
    true
}

/// 处理 CompositionEnd：用提交文字替换预编辑区间并结束合成。
/// 返回是否改动了正文（含仅清合成态时为 false）。
fn apply_composition_end(state: &mut FlowState, committed: &str) -> bool {
    if committed == "\n" || committed == "\r" {
        state.has_ime = false;
        return false;
    }
    let mut changed = false;
    if ime_preedit_range_active(state) {
        let anchor = state.ime_anchor.min(state.char_len());
        let end = state.cursor.min(state.char_len()).max(anchor);
        delete_range(state, anchor, end);
        changed = end > anchor;
        if !committed.is_empty() {
            insert_text_at(state, anchor, committed);
            state.cursor = anchor + committed.chars().count();
            changed = true;
        } else {
            state.cursor = anchor;
        }
    } else if !committed.is_empty() {
        // 无有效预编辑：当作普通插入，不删任何已有字符
        let at = state.cursor.min(state.char_len());
        insert_text_at(state, at, committed);
        state.cursor = at + committed.chars().count();
        changed = true;
    }
    state.clear_selection();
    state.has_ime = false;
    state.ime_anchor = state.cursor;
    state.prefer_next_row = prefer_next_row_for_index(&state.text, state.cursor);
    changed
}

/// 处理 Enter / Shift+Enter：列表续项或软换行，并续上 sticky/armed 焦点。
/// 调用前须已通过 [`enter_gate`]，保证此时无正在合成的预编辑。
fn apply_enter_key(
    state: &mut FlowState,
    undo_stack: &mut FlowUndoStack,
    ui: &egui::Ui,
    sticky_focus_id: Id,
    armed_id: Id,
    preferred_x_id: Id,
    preferred_col_id: Id,
    shift: bool,
    enter_pressed: &mut bool,
    soft_enter: &mut bool,
    changed: &mut bool,
) {
    undo_stack.push_before_edit(state, false);
    // 输入只认当前光标：只消选区，绝不 delete_range（那会把 cursor 拽到选区起点）
    state.clear_selection();
    state.has_ime = false;
    let at = state.cursor.min(state.char_len());
    state.cursor = at;
    state.ime_anchor = at;

    if shift {
        insert_text(state, "\n");
        state.has_ime = false;
        state.ime_anchor = state.cursor;
        state.prefer_next_row = true;
        *soft_enter = true;
        *changed = true;
        ui.ctx().data_mut(|d| {
            d.insert_temp(sticky_focus_id, 2u8);
            d.insert_temp(armed_id, true);
        });
    } else {
        crate::list::handle_enter(state);
        // 回车后必须停在 `\n` 之后（新行行首），否则会画回旧行行末
        debug_assert!(
            state.cursor > 0
                && state.text.chars().nth(state.cursor - 1) == Some('\n'),
            "回车后光标须在换行符之后"
        );
        state.has_ime = false;
        state.ime_anchor = state.cursor;
        state.prefer_next_row = true;
        ui.ctx().data_mut(|d| {
            d.remove_temp::<f32>(preferred_x_id);
            d.remove_temp::<usize>(preferred_col_id);
            d.insert_temp(sticky_focus_id, 2u8);
            d.insert_temp(armed_id, true);
        });
        *enter_pressed = true;
        *changed = true;
    }
}

fn set_cursor(state: &mut FlowState, idx: usize, extend: bool) {
    let idx = idx.min(state.char_len());
    if extend {
        let anchor = state.selection.map(|s| s.start).unwrap_or(state.cursor);
        state.selection = Some(FlowSelection {
            start: anchor,
            end: idx,
        });
    } else {
        state.clear_selection();
    }
    // 方向键/Home/End：结束 IME，插入点与绘制光标同行
    place_caret(state, idx, prefer_next_row_for_index(&state.text, idx), true);
}

/// Word 语义落点：逻辑下标 + 软折行偏好；非合成时插入锚点必须等于光标。
fn place_caret(state: &mut FlowState, idx: usize, prefer_next_row: bool, end_ime: bool) {
    let idx = idx.min(state.char_len());
    state.cursor = idx;
    state.prefer_next_row = prefer_next_row;
    if end_ime {
        state.has_ime = false;
        state.ime_anchor = idx;
    } else if !state.has_ime {
        state.ime_anchor = idx;
    }
}

/// 收键前：夹紧列表前缀，并保证非合成态 ime_anchor == cursor。
fn sync_insert_caret(state: &mut FlowState) {
    let len = state.char_len();
    state.cursor = crate::list::clamp_cursor_to_list_body(&state.text, state.cursor.min(len));
    if state.has_ime {
        state.ime_anchor =
            crate::list::clamp_cursor_to_list_body(&state.text, state.ime_anchor.min(len));
    } else {
        state.ime_anchor = state.cursor;
    }
}

/// 无 galley 时的启发式：越过 `\n` 后偏好下一视觉行行首（与回车后一致）。
fn prefer_next_row_for_index(text: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    text.chars().nth(idx - 1) == Some('\n')
}

fn backspace(state: &mut FlowState) -> bool {
    if let Some(sel) = state.selection {
        if !sel.is_empty() {
            let (a, b) = sel.sorted();
            delete_range(state, a, b);
            return true;
        }
    }
    if state.cursor > 0 {
        delete_range(state, state.cursor - 1, state.cursor);
        return true;
    }
    false
}

fn delete_forward(state: &mut FlowState) -> bool {
    if let Some(sel) = state.selection {
        if !sel.is_empty() {
            let (a, b) = sel.sorted();
            delete_range(state, a, b);
            return true;
        }
    }
    if state.cursor < state.char_len() {
        delete_range(state, state.cursor, state.cursor + 1);
        return true;
    }
    false
}

fn insert_text(state: &mut FlowState, t: &str) {
    // 输入只认当前光标：清选区但不删选区内容（删选区会把 cursor 设成选区起点）
    state.clear_selection();
    let at = state.cursor.min(state.char_len());
    insert_text_at(state, at, t);
}

fn insert_text_at(state: &mut FlowState, at: usize, t: &str) {
    let at = at.min(state.char_len());
    let chars: Vec<char> = state.text.chars().collect();
    let before: String = chars[..at].iter().collect();
    let after: String = chars[at..].iter().collect();
    let n = t.chars().count();
    state.text = format!("{before}{t}{after}");
    style::adjust_runs_for_edit(&mut state.style_runs, at, 0, n);
    for e in &mut state.embeds {
        if e.slot >= at {
            e.slot += n;
        }
    }
    // 插入字符必须落在光标之前：光标推进到插入段之后（含不可见 `\n`）
    // 勿在此结束 IME：CompositionUpdate 会走本函数，需保持 has_ime。
    let new_cursor = at + n;
    state.cursor = new_cursor;
    state.prefer_next_row = prefer_next_row_for_index(&state.text, new_cursor);
    if !state.has_ime {
        state.ime_anchor = new_cursor;
    }
    state.clear_selection();
}

fn delete_range(state: &mut FlowState, start: usize, end: usize) {
    let chars: Vec<char> = state.text.chars().collect();
    let start = start.min(chars.len());
    let end = end.min(chars.len()).max(start);
    if start == end {
        return;
    }
    let before: String = chars[..start].iter().collect();
    let after: String = chars[end..].iter().collect();
    state.text = format!("{before}{after}");
    style::adjust_runs_for_edit(&mut state.style_runs, start, end - start, 0);
    state.embeds.retain(|e| e.slot < start || e.slot >= end);
    let del = end - start;
    for e in &mut state.embeds {
        if e.slot >= end {
            e.slot -= del;
        }
    }
    state.cursor = start;
    state.prefer_next_row = prefer_next_row_for_index(&state.text, start);
    if !state.has_ime {
        state.ime_anchor = start;
    }
    state.clear_selection();
}

fn select_word_char_range(text: &str, at: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let is_word = |c: char| c != EMBED_CHAR && (c.is_alphanumeric() || c == '_' || c > '\u{7f}');
    let at = at.min(chars.len());
    if at < chars.len() && is_word(chars[at]) {
        let mut start = at;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = start + 1;
        while end < chars.len() && is_word(chars[end]) {
            end += 1;
        }
        return (start, end);
    }
    if at > 0 && is_word(chars[at - 1]) {
        let mut start = at - 1;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        return (start, at);
    }
    (at, at)
}

fn clear_ime_preedit(state: &mut FlowState) {
    if !state.has_ime {
        return;
    }
    // 仅删真正的预编辑选区；残留 has_ime 只清标志，绝不删正文
    if ime_preedit_range_active(state) {
        let anchor = state.ime_anchor.min(state.char_len());
        let end = state.cursor.min(state.char_len()).max(anchor);
        delete_range(state, anchor, end);
    }
    state.has_ime = false;
    state.ime_anchor = state.cursor.min(state.char_len());
    state.clear_selection();
}

fn build_layout_job_range(
    ui: &egui::Ui,
    state: &FlowState,
    start: usize,
    end: usize,
    wrap_width: f32,
) -> egui::text::LayoutJob {
    let default_font = egui::TextStyle::Body.resolve(ui.style());
    let default_color = ui.visuals().widgets.inactive.text_color();
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: wrap_width,
            max_rows: usize::MAX,
            break_anywhere: true,
            overflow_character: None,
        },
        ..Default::default()
    };
    let chars: Vec<char> = state.text.chars().collect();
    let end = end.min(chars.len());
    let start = start.min(end);
    if start >= end {
        job.append(
            "",
            0.0,
            egui::TextFormat {
                font_id: default_font,
                color: default_color,
                ..Default::default()
            },
        );
        return job;
    }

    let mut i = start;
    while i < end {
        if chars[i] == EMBED_CHAR {
            i += 1;
            continue;
        }
        let run = state
            .style_runs
            .iter()
            .find(|r| r.start <= i && i < r.end)
            .cloned()
            .unwrap_or_else(|| StyleRun::plain(i, i + 1));
        let mut j = i + 1;
        while j < end && j < run.end && chars[j] != EMBED_CHAR {
            let same = state
                .style_runs
                .iter()
                .find(|r| r.start <= j && j < r.end)
                .map(|r| r.same_style(&run))
                .unwrap_or(true);
            if !same {
                break;
            }
            j += 1;
        }
        let slice: String = chars[i..j].iter().collect();
        let mut font_id = default_font.clone();
        if let Some(sz) = run.size {
            font_id.size = sz as f32;
        }
        if run.bold {
            font_id.family = egui::FontFamily::Name("bold".into());
        }
        let (color, force_ul) = if run.link.is_some() {
            (Color32::from_rgb(0x05, 0x63, 0xC1), true)
        } else {
            (
                run.color
                    .as_deref()
                    .and_then(parse_hex)
                    .unwrap_or(if run.bold {
                        ui.visuals().strong_text_color()
                    } else {
                        default_color
                    }),
                false,
            )
        };
        let mut format = egui::TextFormat {
            font_id,
            color,
            italics: run.italic,
            ..Default::default()
        };
        if force_ul || run.underline {
            format.underline = Stroke::new(1.0, color);
        }
        job.append(&slice, 0.0, format);
        i = j;
    }
    job
}

fn parse_hex(s: &str) -> Option<Color32> {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&t[0..2], 16).ok()?;
    let g = u8::from_str_radix(&t[2..4], 16).ok()?;
    let b = u8::from_str_radix(&t[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_restores_deleted_text() {
        let mut state = FlowState::from_plain("hello");
        state.cursor = 5;
        let mut stack = FlowUndoStack::default();
        stack.push_before_edit(&state, false);
        assert!(backspace(&mut state));
        assert_eq!(state.text, "hell");
        assert!(stack.undo(&mut state));
        assert_eq!(state.text, "hello");
        assert!(stack.redo(&mut state));
        assert_eq!(state.text, "hell");
    }

    #[test]
    fn undo_ignores_embed_size_jitter() {
        let mut state = FlowState::from_plain("ab");
        state.embeds.push(FlowEmbed::new(1, FlowEmbedKind::Table, 0));
        // 文本含嵌入符才合理；这里只测 snap 相等性
        let mut a = FlowUndoSnap::from_state(&state);
        state.embeds[0].height = 999.0;
        state.embeds[0].width = 888.0;
        let b = FlowUndoSnap::from_state(&state);
        a.cursor = b.cursor;
        assert_eq!(a, b);
    }

    /// 回车先于 CompositionEnd：合成中 Ignore，提交后正文正确，再回车才换行。
    #[test]
    fn enter_during_ime_preedit_ignored_then_commit_then_newline() {
        // AAA|BBB → 先拆行
        let mut state = FlowState::from_plain("AAABBB");
        state.cursor = 3;
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "AAA\nBBB");
        assert_eq!(state.cursor, 4);

        // 合成预编辑 xy
        assert!(apply_composition_update(&mut state, "xy"));
        assert_eq!(state.text, "AAA\nxyBBB");
        assert!(state.has_ime);
        assert_eq!(state.ime_anchor, 4);
        assert_eq!(state.cursor, 6);

        // 回车先到：应 Ignore，正文不动
        assert_eq!(enter_gate(&mut state, false), EnterGate::Ignore);
        assert_eq!(state.text, "AAA\nxyBBB");
        assert!(state.has_ime);

        // CompositionEnd 提交
        assert!(apply_composition_end(&mut state, "xy"));
        assert_eq!(state.text, "AAA\nxyBBB");
        assert!(!state.has_ime);
        assert_eq!(state.cursor, 6);

        // 再回车才换行
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "AAA\nxy\nBBB");
        assert_eq!(state.cursor, 7);
    }

    /// 同帧先 CompositionEnd 后 Enter：门控 Ignore，不插空行。
    #[test]
    fn enter_after_ime_commit_same_frame_ignored() {
        let mut state = FlowState::from_plain("AAA\nBBB");
        state.cursor = 4;
        assert!(apply_composition_update(&mut state, "xy"));
        assert!(apply_composition_end(&mut state, "xy"));
        assert_eq!(state.text, "AAA\nxyBBB");
        assert_eq!(state.cursor, 6);

        // 同帧已提交 → Enter Ignore
        assert_eq!(enter_gate(&mut state, true), EnterGate::Ignore);
        assert_eq!(state.text, "AAA\nxyBBB");
        assert!(!state.text.contains("\n\n"));
    }

    /// 残留 has_ime（预编辑为空）：清掉后允许换行，\\n 在光标前。
    #[test]
    fn enter_with_residual_has_ime_clears_and_newlines() {
        let mut state = FlowState::from_plain("hello");
        state.cursor = 5;
        state.has_ime = true;
        state.ime_anchor = 5; // 预编辑为空
        assert_eq!(enter_gate(&mut state, false), EnterGate::Newline);
        assert!(!state.has_ime);
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "hello\n");
        assert_eq!(state.cursor, 6);
        let before: String = state.text.chars().take(state.cursor).collect();
        assert!(before.ends_with('\n'));
    }

    /// 无 IME 时普通行中回车：cursor+1。
    #[test]
    fn plain_midline_enter_advances_cursor() {
        let mut state = FlowState::from_plain("AAABBB");
        state.cursor = 3;
        assert_eq!(enter_gate(&mut state, false), EnterGate::Newline);
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "AAA\nBBB");
        assert_eq!(state.cursor, 4);
    }

    /// 残留 has_ime + 过期锚点：CompositionUpdate 不得删掉中间正文。
    #[test]
    fn stale_ime_update_does_not_delete_body() {
        let mut state = FlowState::from_plain("房间按到");
        state.cursor = 4; // 已在「按到」之后
        state.has_ime = true;
        state.ime_anchor = 0; // 过期锚点指到文首
        state.clear_selection(); // 无预编辑选区
        assert!(apply_composition_update(&mut state, "啊"));
        assert!(
            state.text.contains("房间按到"),
            "不得删正文：got {:?}",
            state.text
        );
        assert!(state.text.contains('啊'));
    }

    /// 残留 has_ime：CompositionEnd 只插入，不删已有字符。
    #[test]
    fn stale_ime_end_inserts_without_delete() {
        let mut state = FlowState::from_plain("hello");
        state.cursor = 5;
        state.has_ime = true;
        state.ime_anchor = 0;
        state.clear_selection();
        assert!(apply_composition_end(&mut state, "!"));
        assert_eq!(state.text, "hello!");
        assert!(!state.has_ime);
    }

    /// 空 Preedit 不得把光标拽回旧锚点，也不得删掉已插入的换行。
    #[test]
    fn empty_composition_update_does_not_rewind_caret_after_enter() {
        let mut state = FlowState::from_plain("房间按到");
        state.cursor = 2; // 间|按
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "房间\n按到");
        assert_eq!(state.cursor, 3);
        // 模拟次帧残留：旧锚点落在已变成 \\n 的位置
        state.has_ime = true;
        state.ime_anchor = 2;
        let cursor_before = state.cursor;
        assert!(apply_composition_update(&mut state, ""));
        assert!(!state.has_ime);
        assert_eq!(state.text, "房间\n按到", "不得删掉回车插入的换行");
        assert_eq!(state.cursor, cursor_before, "不得把光标拽回旧锚点");
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "房间\n\n按到");
        assert_eq!(state.cursor, 4);
        assert!(state.text.chars().skip(state.cursor).collect::<String>().starts_with("按到"));
        // 非合成时空更新必须 no-op
        let c = state.cursor;
        assert!(!apply_composition_update(&mut state, ""));
        assert_eq!(state.cursor, c);
        assert!(!state.has_ime);
    }

    /// 方向键移到行中再回车：光标必须在新行（`\n` 之后），不能停在旧行行末。
    #[test]
    fn mid_line_enter_cursor_after_newline_not_on_it() {
        let mut state = FlowState::from_plain("房间按到");
        state.cursor = 2; // 间|按（模拟方向键到位）
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "房间\n按到");
        assert_eq!(state.cursor, 3);
        assert_eq!(state.text.chars().nth(2), Some('\n'));
        assert_ne!(
            state.text.chars().nth(state.cursor),
            Some('\n'),
            "光标若停在 \\n 上会被画回旧行"
        );
        assert!(state.prefer_next_row);
        assert_eq!(state.ime_anchor, state.cursor);
    }

    /// 空 CompositionEnd 不得把已换行的光标拽回去。
    #[test]
    fn empty_composition_end_after_enter_keeps_new_line_caret() {
        let mut state = FlowState::from_plain("AB");
        state.cursor = 1;
        crate::list::handle_enter(&mut state);
        assert_eq!(state.cursor, 2);
        assert!(!apply_composition_end(&mut state, ""));
        assert_eq!(state.text, "A\nB");
        assert_eq!(state.cursor, 2);
    }

    /// 有选区时回车：只消选区，在当前光标换行，不删选区正文、不拽回选区起点。
    #[test]
    fn enter_with_selection_inserts_at_cursor_only() {
        let mut state = FlowState::from_plain("ABCDEF");
        state.cursor = 4; // |E
        state.selection = Some(FlowSelection { start: 0, end: 2 }); // 「AB」
        state.clear_selection(); // 与 apply_enter_key 一致
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "ABCD\nEF");
        assert_eq!(state.cursor, 5);
        assert!(state.text.contains("AB"));
    }

    /// 端点选区 + 回车：同样不删选区，只在光标处换行。
    #[test]
    fn enter_with_anchored_selection_still_at_cursor() {
        let mut state = FlowState::from_plain("ABCDEF");
        state.selection = Some(FlowSelection { start: 2, end: 4 }); // CD
        state.cursor = 4;
        state.clear_selection();
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "ABCD\nEF");
        assert_eq!(state.cursor, 5);
        assert!(state.text.contains("CD"), "不得因选区删除正文");
    }

    /// 有选区时插入：不删正文，只在光标处插入。
    #[test]
    fn insert_with_selection_does_not_delete() {
        let mut state = FlowState::from_plain("ABCDEF");
        state.cursor = 4;
        state.selection = Some(FlowSelection { start: 0, end: 2 });
        insert_text(&mut state, "X");
        assert_eq!(state.text, "ABCDXEF");
        assert_eq!(state.cursor, 5);
    }

    /// Word：点击落点后 cursor == ime_anchor，回车插在同一下标。
    #[test]
    fn place_caret_syncs_insert_point_for_enter() {
        let mut state = FlowState::from_plain("hello world");
        // 点在 "hello|" 与旧锚点（文末）不同
        place_caret(&mut state, 5, false, true);
        sync_insert_caret(&mut state);
        assert_eq!(state.cursor, 5);
        assert_eq!(state.ime_anchor, 5);
        assert!(!state.has_ime);
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "hello\n world");
        assert_eq!(state.cursor, 6);
        assert_eq!(state.ime_anchor, 6);
    }

    /// 拖选路径曾用未夹紧下标覆盖光标；夹紧后插入点须在列表正文内。
    #[test]
    fn clamp_after_pointer_keeps_insert_in_list_body() {
        let mut state = FlowState::from_plain("1. hello");
        // 误点在编号 "1" 上
        place_caret(&mut state, 0, false, true);
        sync_insert_caret(&mut state);
        let body = crate::list::line_body_start(&state.text, 0);
        assert_eq!(state.cursor, body);
        assert_eq!(state.ime_anchor, body);
        crate::list::handle_enter(&mut state);
        assert!(state.text.starts_with("1. "));
        assert_eq!(state.cursor, body + 1);
    }

    /// 单源契约：方向键到 B → Text（模拟 sanitize 中文）→ Enter，全程不回到 A。
    #[test]
    fn arrow_then_text_then_enter_stays_at_new_caret() {
        let mut state = FlowState::from_plain("房间按到世界");
        // 原点击落点 A=0；方向键模拟移到「按|到」=3
        let a = 0usize;
        let b = 3usize;
        place_caret(&mut state, a, false, true);
        set_cursor(&mut state, b, false);
        sync_insert_caret(&mut state);
        assert_eq!(state.cursor, b);
        assert_eq!(state.ime_anchor, b);

        // sanitize 后中文走 Text
        insert_text(&mut state, "啊");
        assert!(state.text.contains("按啊到"));
        assert_eq!(state.cursor, b + 1);
        assert_ne!(state.cursor, a);

        // 再回车：须在新位置之后，竖线不跳回 A
        assert_eq!(enter_gate(&mut state, false), EnterGate::Newline);
        state.clear_selection();
        let at = state.cursor;
        crate::list::handle_enter(&mut state);
        assert_eq!(state.cursor, at + 1);
        assert_ne!(state.cursor, a);
        assert_eq!(state.text.chars().nth(at), Some('\n'));
        assert!(state.text.chars().take(state.cursor).collect::<String>().ends_with('\n'));
    }

    /// 陈旧选区 [A,B]、cursor=B：Text/Enter 不删区间、不把 cursor 拽回 A。
    #[test]
    fn stale_selection_text_and_enter_keep_cursor_end() {
        let mut state = FlowState::from_plain("ABCDEF");
        let a = 1usize;
        let b = 4usize;
        state.selection = Some(FlowSelection { start: a, end: b });
        state.cursor = b;
        state.ime_anchor = b;

        insert_text(&mut state, "中");
        assert_eq!(state.text, "ABCD中EF");
        assert_eq!(state.cursor, b + 1);
        assert!(state.text.contains("BCD"), "不得删选区正文");

        // 再挂上陈旧选区端点（模拟宿主误恢复），Enter 仍只认 cursor
        state.selection = Some(FlowSelection { start: a, end: state.cursor });
        let c = state.cursor;
        state.clear_selection();
        crate::list::handle_enter(&mut state);
        assert_eq!(state.cursor, c + 1);
        assert_ne!(state.cursor, a);
        assert!(state.text.contains("BCD"));
    }

    /// 残留 has_ime 不得挡住 Enter（sanitize 中文后常见脏标志）。
    #[test]
    fn residual_has_ime_without_preedit_does_not_block_enter() {
        let mut state = FlowState::from_plain("hello");
        set_cursor(&mut state, 2, false);
        state.has_ime = true;
        state.ime_anchor = 0; // 过期
        state.clear_selection();
        assert_eq!(enter_gate(&mut state, false), EnterGate::Newline);
        assert!(!state.has_ime);
        assert_eq!(state.ime_anchor, 2);
        crate::list::handle_enter(&mut state);
        assert_eq!(state.text, "he\nllo");
        assert_eq!(state.cursor, 3);
    }
}
