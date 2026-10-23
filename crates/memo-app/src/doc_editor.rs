//! 公文风所见即所得文档编辑器（备忘 / 任务计划共用）。
//! 正文格式：`memo-doc/v1`（颜色 / 链接 / 插图）；旧 Markdown 风格自动迁移。

use crate::save_dialog;
use crate::theme;
use eframe::egui::{
    self, text::CCursor, text::CCursorRange, Color32, Frame, Id, Key, Margin, Order, Rect,
    RichText, Rounding, Sense, Stroke, TextBuffer, Vec2,
};
use memo_core::attachments;
use memo_core::memo_doc::{self, Block, Span};
use memo_core::store::MemoCategory;
use memo_text::{
    apply_list_style_to_lines, line_at_cursor, parse_list_prefix, FlowEditor, FlowEmbed,
    FlowEmbedKind, FlowSelection, FlowState, ListStyle, StyleDelta, StyleRun, EMBED_CHAR,
};
use std::collections::HashMap;
use std::path::Path;

/// 流内嵌入载荷（与 FlowEmbed.id 对应；不进 text）。
#[derive(Clone)]
enum EmbedPayload {
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Image {
        file_id: String,
        alt: String,
    },
}

fn doc_embed_heights_id(id_salt: &str) -> Id {
    Id::new(("doc_embed_heights", id_salt))
}

fn doc_embed_widths_id(id_salt: &str) -> Id {
    Id::new(("doc_embed_widths", id_salt))
}

/// 上一帧表内控件获焦 / 右键菜单打开 → 本帧 Flow 不抢键盘。
fn doc_table_ui_lock_id(id_salt: &str) -> Id {
    Id::new(("doc_table_ui_lock", id_salt))
}

/// 正文选区快照（点浮层按钮前须缓存，否则会丢掉选区）。
#[derive(Clone, Default)]
struct BodySelection {
    start: usize,
    end: usize,
    text: String,
    anchor: egui::Pos2,
    /// 迷你条展开「字体颜色」色板
    color_mode: bool,
    /// 迷你条展开「设为链接」网址框
    link_mode: bool,
    /// 迷你条展开字号
    size_mode: bool,
    link_url: String,
}

/// 供顶栏工具条使用的选区：落在某一 flow run 内的字符下标。
#[derive(Clone, Default)]
struct DocTextSelection {
    /// 该 flow run 在 `doc.blocks` 中的起始块下标。
    run_start: usize,
    /// 兼容旧字段名：同 `run_start`。
    para_idx: usize,
    start: usize,
    end: usize,
    #[allow(dead_code)]
    text: String,
}

fn doc_flow_cursor_id(id_salt: &str, run_key: u64) -> Id {
    Id::new(("doc_flow_cursor", id_salt, run_key))
}

fn doc_flow_cursor_line_id(id_salt: &str, run_key: u64) -> Id {
    Id::new(("doc_flow_cursor_line", id_salt, run_key))
}

/// 光标快照：绝对下标 + 软折行偏好（与 FlowState / egui CCursor 一致）。
#[derive(Clone, Copy, Default)]
struct FlowCaretSnap {
    index: usize,
    prefer_next_row: bool,
}

/// 恢复光标：只用绝对下标 + prefer_next_row。
/// 曾用「行号+列」优先恢复，连续回车时会把光标钉回旧行行末（落在 `\n` 上），
/// 下一帧再插入的 `\n` 就像加在光标后面，光标看起来不动。
fn restore_flow_cursor(state: &mut FlowState, snap: FlowCaretSnap) {
    state.cursor = snap.index.min(state.char_len());
    state.cursor = memo_text::clamp_cursor_to_list_body(&state.text, state.cursor);
    state.prefer_next_row = snap.prefer_next_row;
    if !state.has_ime {
        state.ime_anchor = state.cursor;
    }
}

fn save_flow_cursor(ctx: &egui::Context, id_salt: &str, run_key: u64, state: &FlowState) {
    ctx.data_mut(|d| {
        // 清掉旧的行号快照 / 裸 usize，避免其它路径误用
        d.remove_temp::<(usize, usize)>(doc_flow_cursor_line_id(id_salt, run_key));
        d.remove_temp::<usize>(doc_flow_cursor_id(id_salt, run_key));
        d.insert_temp(
            doc_flow_cursor_id(id_salt, run_key),
            FlowCaretSnap {
                index: state.cursor,
                prefer_next_row: state.prefer_next_row,
            },
        );
    });
}

fn read_flow_caret(ctx: &egui::Context, id_salt: &str, run_key: u64) -> Option<FlowCaretSnap> {
    ctx.data(|d| {
        d.get_temp::<FlowCaretSnap>(doc_flow_cursor_id(id_salt, run_key))
            .or_else(|| {
                d.get_temp::<usize>(doc_flow_cursor_id(id_salt, run_key))
                    .map(|index| FlowCaretSnap {
                        index,
                        prefer_next_row: true,
                    })
            })
    })
}

fn doc_flow_sel_id(id_salt: &str, run_key: u64) -> Id {
    Id::new(("doc_flow_sel", id_salt, run_key))
}

fn doc_flow_session_id(id_salt: &str, run_key: u64) -> Id {
    Id::new(("doc_flow_session", id_salt, run_key))
}

/// 待在竖滚层应用的光标滚入目标（横滚会吞掉内部 scroll_to_rect 的 Y）。
fn doc_caret_scroll_id(id_salt: &str) -> Id {
    Id::new(("doc_caret_scroll", id_salt))
}

#[derive(Clone, Copy)]
struct CaretScrollTarget(Rect);

impl Default for CaretScrollTarget {
    fn default() -> Self {
        Self(Rect::NOTHING)
    }
}

fn table_cell_id(id_salt: &str, block_idx: usize, row: usize, col: usize) -> Id {
    Id::new(("tbl_cell", id_salt, block_idx, row, col))
}

fn table_focus_pending_id(id_salt: &str) -> Id {
    Id::new(("tbl_focus_pending", id_salt))
}

/// 下帧要聚焦的表格格；`cursor_at_end` 控制落点（左跳→末尾，右/Tab→开头）。
#[derive(Clone, Copy, Default)]
struct TableFocusPending {
    block_idx: usize,
    row: usize,
    col: usize,
    cursor_at_end: bool,
}

#[derive(Clone, Copy)]
enum TableNavDir {
    Left,
    Right,
    Up,
    Down,
    Tab,
    ShiftTab,
}

fn table_cursor_at_bounds(text: &str, idx: usize) -> (bool, bool, bool, bool) {
    let len = text.chars().count();
    let idx = idx.min(len);
    let at_start = idx == 0;
    let at_end = idx >= len;
    let before: String = text.chars().take(idx).collect();
    let after: String = text.chars().skip(idx).collect();
    let at_first_line = !before.contains('\n');
    let at_last_line = !after.contains('\n');
    (at_start, at_end, at_first_line, at_last_line)
}

fn table_nav_target(
    dir: TableNavDir,
    row: usize,
    col: usize,
    nrows: usize,
    ncols: usize,
    at_start: bool,
    at_end: bool,
    at_first_line: bool,
    at_last_line: bool,
) -> Option<(usize, usize, bool)> {
    // 返回 (nrow, ncol, cursor_at_end)
    let ncols = ncols.max(1);
    let nrows = nrows.max(1);
    match dir {
        TableNavDir::Left if at_start && col > 0 => Some((row, col - 1, true)),
        TableNavDir::Right if at_end && col + 1 < ncols => Some((row, col + 1, false)),
        TableNavDir::Up if at_first_line && row > 0 => Some((row - 1, col, false)),
        TableNavDir::Down if at_last_line && row + 1 < nrows => Some((row + 1, col, false)),
        TableNavDir::Tab => {
            if col + 1 < ncols {
                Some((row, col + 1, false))
            } else if row + 1 < nrows {
                Some((row + 1, 0, false))
            } else {
                Some((0, 0, false))
            }
        }
        TableNavDir::ShiftTab => {
            if col > 0 {
                Some((row, col - 1, true))
            } else if row > 0 {
                Some((row - 1, ncols - 1, true))
            } else {
                Some((nrows - 1, ncols - 1, true))
            }
        }
        _ => None,
    }
}

fn peek_table_nav_keys(ui: &egui::Ui) -> Option<(TableNavDir, bool)> {
    ui.input(|i| {
        if i.key_pressed(Key::Tab) {
            return Some((
                if i.modifiers.shift {
                    TableNavDir::ShiftTab
                } else {
                    TableNavDir::Tab
                },
                i.modifiers.shift,
            ));
        }
        if i.key_pressed(Key::ArrowLeft) {
            return Some((TableNavDir::Left, false));
        }
        if i.key_pressed(Key::ArrowRight) {
            return Some((TableNavDir::Right, false));
        }
        if i.key_pressed(Key::ArrowUp) {
            return Some((TableNavDir::Up, false));
        }
        if i.key_pressed(Key::ArrowDown) {
            return Some((TableNavDir::Down, false));
        }
        None
    })
}

fn consume_table_nav_key(ui: &mut egui::Ui, dir: TableNavDir) {
    match dir {
        TableNavDir::Tab | TableNavDir::ShiftTab => {
            ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, Key::Tab));
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Tab));
        }
        TableNavDir::Left => {
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft));
        }
        TableNavDir::Right => {
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowRight));
        }
        TableNavDir::Up => {
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowUp));
        }
        TableNavDir::Down => {
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowDown));
        }
    }
}

fn edit_table_cell(
    ui: &mut egui::Ui,
    text: &mut String,
    id_salt: &str,
    block_idx: usize,
    row: usize,
    col: usize,
    nrows: usize,
    ncols: usize,
    cell_w: f32,
) -> (egui::Response, bool) {
    let cell_id = table_cell_id(id_salt, block_idx, row, col);
    let pending_id = table_focus_pending_id(id_salt);
    let was_focused = ui.memory(|m| m.has_focus(cell_id));

    // TextEdit 前：若已聚焦且边界键应跳格，先 consume，避免格内光标被挪走
    if was_focused {
        if let Some((dir, _)) = peek_table_nav_keys(ui) {
            let idx = egui::text_edit::TextEditState::load(ui.ctx(), cell_id)
                .and_then(|s| s.cursor.char_range().map(|r| r.primary.index))
                .unwrap_or(0);
            let (at_start, at_end, at_first, at_last) = table_cursor_at_bounds(text, idx);
            if let Some((nr, nc, at_end_cur)) =
                table_nav_target(dir, row, col, nrows, ncols, at_start, at_end, at_first, at_last)
            {
                consume_table_nav_key(ui, dir);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        pending_id,
                        TableFocusPending {
                            block_idx,
                            row: nr,
                            col: nc,
                            cursor_at_end: at_end_cur,
                        },
                    );
                });
            }
        }
    }

    let r = ui.add(
        egui::TextEdit::multiline(text)
            .id_source(("tbl_cell", id_salt, block_idx, row, col))
            .desired_width((cell_w - 12.0).max(1.0))
            .desired_rows(1)
            .frame(false)
            .font(egui::TextStyle::Body),
    );

    // 本帧 pending 指向本格：夺焦并设插入点，写入滚入目标
    let take = ui.ctx().data(|d| {
        d.get_temp::<TableFocusPending>(pending_id).is_some_and(|p| {
            p.block_idx == block_idx && p.row == row && p.col == col
        })
    });
    if take {
        let at_end = ui
            .ctx()
            .data(|d| {
                d.get_temp::<TableFocusPending>(pending_id)
                    .map(|p| p.cursor_at_end)
            })
            .unwrap_or(false);
        r.request_focus();
        let idx = if at_end {
            text.chars().count()
        } else {
            0
        };
        let mut st = egui::text_edit::TextEditState::load(ui.ctx(), r.id).unwrap_or_default();
        st.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(idx))));
        st.store(ui.ctx(), r.id);
        ui.ctx().data_mut(|d| {
            d.remove_temp::<TableFocusPending>(pending_id);
            d.insert_temp(
                doc_caret_scroll_id(id_salt),
                CaretScrollTarget(r.rect),
            );
        });
    }

    let focused = r.has_focus();
    (r, focused)
}

/// 跨帧编辑会话：正文与光标同存。仅当 `text` 与本帧 `blocks_to_flow` 一致时复用光标，
/// 避免「每帧重建临时 FlowState + 侧信道 snap」在方向键/IME/回车之间分叉。
#[derive(Clone, Default)]
struct FlowSession {
    text: String,
    cursor: usize,
    prefer_next_row: bool,
    selection: Option<FlowSelection>,
    has_ime: bool,
    ime_anchor: usize,
}

fn caret_trace(msg: impl AsRef<str>) {
    if std::env::var_os("MEMO_CARET_TRACE").is_none() {
        return;
    }
    let path = std::env::temp_dir().join("memo_caret_trace.log");
    let line = format!("{}\n", msg.as_ref());
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.as_bytes())
        });
}

fn doc_text_sel_id(id_salt: &str) -> Id {
    Id::new(("doc_text_sel", id_salt))
}

fn doc_toolbar_hit_id(id_salt: &str) -> Id {
    Id::new(("doc_toolbar_hit", id_salt))
}

fn doc_pending_style_id(id_salt: &str) -> Id {
    Id::new(("doc_pending_style", id_salt))
}

fn doc_select_all_id(id_salt: &str) -> Id {
    Id::new(("doc_select_all", id_salt))
}

fn doc_select_all_active(ctx: &egui::Context, id_salt: &str) -> bool {
    ctx.data(|d| d.get_temp::<bool>(doc_select_all_id(id_salt)))
        .unwrap_or(false)
}

/// 是否有可用于「设为链接」的正文选区（含整篇选中）。
fn doc_has_text_selection(ctx: &egui::Context, id_salt: &str) -> bool {
    if doc_select_all_active(ctx, id_salt) {
        return true;
    }
    ctx.data(|d| d.get_temp::<DocTextSelection>(doc_text_sel_id(id_salt)))
        .is_some_and(|s| s.start < s.end)
}

fn set_doc_select_all(ctx: &egui::Context, id_salt: &str, on: bool) {
    ctx.data_mut(|d| {
        if on {
            d.insert_temp(doc_select_all_id(id_salt), true);
            d.remove_temp::<DocTextSelection>(doc_text_sel_id(id_salt));
        } else {
            d.insert_temp(doc_select_all_id(id_salt), false);
        }
    });
}

/// 整篇纯文本（复制用）：段落 / 列表 / 表格 / 图片说明。
fn doc_plain_export(doc: &Doc) -> String {
    let mut out = String::new();
    for (bi, b) in doc.blocks.iter().enumerate() {
        if bi > 0 && !out.ends_with('\n') {
            out.push('\n');
        }
        match b {
            Block::Paragraph { spans } => {
                out.push_str(&paragraph_full_text(spans));
                out.push('\n');
            }
            Block::Items { .. } => {
                // 读入时已迁成段落；防御性跳过
            }
            Block::Table { headers, rows } => {
                if !headers.is_empty() {
                    out.push_str(&headers.join("\t"));
                    out.push('\n');
                }
                for row in rows {
                    out.push_str(&row.join("\t"));
                    out.push('\n');
                }
            }
            Block::Image { alt, .. } => {
                let label = if alt.trim().is_empty() {
                    "[图片]"
                } else {
                    alt.trim()
                };
                out.push_str(&format!("[{label}]\n"));
            }
        }
    }
    out
}

#[allow(dead_code)]
fn paint_select_all_bg(ui: &mut egui::Ui, rect: egui::Rect) {
    if !rect.is_positive() {
        return;
    }
    ui.painter().rect_filled(
        rect.expand(2.0),
        Rounding::same(4.0),
        Color32::from_rgba_unmultiplied(0x2F, 0x6F, 0xED, 48),
    );
}

/// 与正文选区同色系的单元格底（不透明，避免盖住格线后看不清）。
fn table_cell_selection_fill(base: Color32) -> Color32 {
    // 往选区蓝 (0x2F, 0x6F, 0xED) 靠拢
    Color32::from_rgb(
        ((base.r() as u16 * 2 + 0x2F) / 3) as u8,
        ((base.g() as u16 * 2 + 0x6F) / 3) as u8,
        ((base.b() as u16 * 2 + 0xED) / 3) as u8,
    )
}

fn flow_embed_in_selection(state: &FlowState, embed_id: u64) -> bool {
    let Some(sel) = state.selection else {
        return false;
    };
    let (a, b) = sel.sorted();
    if a >= b {
        return false;
    }
    state
        .embeds
        .iter()
        .any(|e| e.id == embed_id && a <= e.slot && e.slot < b)
}

/// 按 Flow 字符范围导出明文；`EMBED_CHAR` 展开为表 TSV / 图片说明。
fn flow_range_plain(
    state: &FlowState,
    store: &HashMap<u64, EmbedPayload>,
    start: usize,
    end: usize,
) -> String {
    let chars: Vec<char> = state.text.chars().collect();
    let a = start.min(chars.len());
    let b = end.min(chars.len()).max(a);
    let mut out = String::new();
    let mut i = a;
    while i < b {
        if chars[i] == EMBED_CHAR {
            if let Some(emb) = state.embeds.iter().find(|e| e.slot == i) {
                match store.get(&emb.id) {
                    Some(EmbedPayload::Table { headers, rows }) => {
                        if !headers.is_empty() {
                            out.push_str(&headers.join("\t"));
                            out.push('\n');
                        }
                        for row in rows {
                            out.push_str(&row.join("\t"));
                            out.push('\n');
                        }
                    }
                    Some(EmbedPayload::Image { alt, .. }) => {
                        let label = if alt.trim().is_empty() {
                            "[图片]"
                        } else {
                            alt.trim()
                        };
                        out.push_str(&format!("[{label}]"));
                    }
                    None => {}
                }
            }
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// 整篇选中时：样式打到所有段落全文。
fn apply_style_to_all_paragraphs(doc: &mut Doc, patch: StylePatch) {
    for b in doc.blocks.iter_mut() {
        if let Block::Paragraph { spans } = b {
            let len = paragraph_full_text(spans).chars().count();
            if len == 0 {
                if spans.is_empty() {
                    spans.push(Span::plain(""));
                }
                if let Some(sp) = spans.last_mut() {
                    if let Some(v) = patch.bold {
                        sp.bold = v;
                    }
                    if let Some(v) = patch.italic {
                        sp.italic = v;
                    }
                    if let Some(v) = patch.underline {
                        sp.underline = v;
                    }
                    if let Some(sz) = patch.size {
                        sp.size = sz;
                    }
                    if let Some(c) = &patch.color {
                        sp.color = c.clone();
                    }
                }
            } else {
                apply_style_to_char_range(spans, 0, len, patch.clone());
            }
        }
    }
}

/// 选区内（或目标范围）是否全部已带某种 BIU 样式。
fn range_all_flag(spans: &[Span], start: usize, end: usize, flag: impl Fn(&Span) -> bool) -> bool {
    if start >= end {
        return false;
    }
    let mut any = false;
    let mut char_i = 0usize;
    for sp in spans {
        let len = sp.text.chars().count();
        let span_start = char_i;
        let span_end = char_i + len;
        char_i = span_end;
        if len == 0 {
            continue;
        }
        if span_end <= start || span_start >= end {
            continue;
        }
        any = true;
        if !flag(sp) {
            return false;
        }
    }
    any
}

/// 根据当前目标（整篇 / 选区 / 末段）决定 BIU 开关：已全开则关，否则开。
fn biu_toggle_on(
    ctx: &egui::Context,
    doc: &Doc,
    id_salt: &str,
    flag: impl Fn(&Span) -> bool,
) -> bool {
    if doc_select_all_active(ctx, id_salt) {
        let mut any = false;
        for b in &doc.blocks {
            if let Block::Paragraph { spans } = b {
                let len = paragraph_full_text(spans).chars().count();
                if len == 0 {
                    if let Some(sp) = spans.last() {
                        any = true;
                        if !flag(sp) {
                            return true;
                        }
                    }
                    continue;
                }
                any = true;
                if !range_all_flag(spans, 0, len, &flag) {
                    return true;
                }
            }
        }
        return !any;
    }
    if let Some(sel) = ctx.data(|d| d.get_temp::<DocTextSelection>(doc_text_sel_id(id_salt))) {
        if sel.start < sel.end {
            let state = blocks_to_flow(&doc.blocks).0;
            return !flow_range_all_flag(&state, sel.start, sel.end, &flag);
        }
    }
    if let Some(Block::Paragraph { spans }) = doc
        .blocks
        .iter()
        .rev()
        .find(|b| matches!(b, Block::Paragraph { .. }))
    {
        let len = paragraph_full_text(spans).chars().count();
        if len == 0 {
            return !spans.last().map(&flag).unwrap_or(false);
        }
        return !range_all_flag(spans, 0, len, flag);
    }
    true
}

/// 顶栏点击：记下样式补丁，由段落编辑在本帧消费（避免焦点切换丢选区后改不到字）。
fn apply_toolbar_style(ctx: &egui::Context, doc: &mut Doc, id_salt: &str, patch: StylePatch) {
    ctx.data_mut(|d| {
        d.insert_temp(doc_toolbar_hit_id(id_salt), true);
        d.insert_temp(doc_pending_style_id(id_salt), patch.clone());
    });
    // 整篇选中：格式作用于所有正文段落
    if doc_select_all_active(ctx, id_salt) {
        apply_style_to_all_paragraphs(doc, patch);
        return;
    }
    let sel = ctx.data(|d| d.get_temp::<DocTextSelection>(doc_text_sel_id(id_salt)));
    if let Some(sel) = sel {
        if sel.start < sel.end {
            if apply_style_to_flow_range(doc, sel.run_start, sel.start, sel.end, patch.clone()) {
                return;
            }
        }
    }
    // 无选区快照时仍立即改最后一段，pending 留给 flow 侧再补一次
    toggle_style_on_last_para(doc, patch);
}

#[allow(dead_code)]
fn flow_run_end(doc: &Doc, run_start: usize) -> usize {
    let mut end = run_start;
    while end < doc.blocks.len() && matches!(doc.blocks[end], Block::Paragraph { .. }) {
        end += 1;
    }
    end
}

fn flow_range_all_flag(
    state: &FlowState,
    start: usize,
    end: usize,
    flag: &impl Fn(&Span) -> bool,
) -> bool {
    if start >= end {
        return false;
    }
    let mut any = false;
    for r in &state.style_runs {
        if r.end <= start || r.start >= end {
            continue;
        }
        any = true;
        let probe = Span {
            text: String::new(),
            color: r.color.clone(),
            link: r.link.clone(),
            bold: r.bold,
            italic: r.italic,
            underline: r.underline,
            size: r.size,
        };
        if !flag(&probe) {
            return false;
        }
    }
    any
}

fn style_patch_to_delta(patch: StylePatch) -> StyleDelta {
    StyleDelta {
        color: patch.color,
        link: patch.link,
        bold: patch.bold,
        italic: patch.italic,
        underline: patch.underline,
        size: patch.size,
    }
}

fn apply_style_to_flow_range(
    doc: &mut Doc,
    _run_start: usize,
    start: usize,
    end: usize,
    patch: StylePatch,
) -> bool {
    let (mut state, store) = blocks_to_flow(&doc.blocks);
    if state.char_len() == 0 && start >= end {
        return false;
    }
    FlowEditor::apply_patch(&mut state, start, end, style_patch_to_delta(patch));
    doc.blocks = flow_to_blocks(&state, &store);
    ensure_body_paragraph(doc);
    true
}

/// 整篇 blocks → 单一 Flow（表/图为 FFFC 嵌入）。
fn blocks_to_flow(blocks: &[Block]) -> (FlowState, HashMap<u64, EmbedPayload>) {
    let mut text = String::new();
    let mut runs: Vec<StyleRun> = Vec::new();
    let mut embeds: Vec<FlowEmbed> = Vec::new();
    let mut store: HashMap<u64, EmbedPayload> = HashMap::new();
    let mut next_id = 1u64;
    let mut need_para_sep = false;

    for b in blocks {
        match b {
            Block::Paragraph { spans } => {
                // 段落之间始终插入一个 \n（含空段），否则尾部空行 / 连续空段会丢
                if need_para_sep && !text.is_empty() {
                    text.push('\n');
                }
                need_para_sep = true;
                let base = text.chars().count();
                let mut off = 0usize;
                for sp in spans {
                    let n = sp.text.chars().count();
                    if n == 0 {
                        continue;
                    }
                    runs.push(StyleRun {
                        start: base + off,
                        end: base + off + n,
                        bold: sp.bold,
                        italic: sp.italic,
                        underline: sp.underline,
                        size: sp.size,
                        color: sp.color.clone(),
                        link: sp.link.clone(),
                    });
                    text.push_str(&sp.text);
                    off += n;
                }
            }
            Block::Table { headers, rows } => {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                let slot = text.chars().count();
                text.push(EMBED_CHAR);
                text.push('\n');
                let id = next_id;
                next_id += 1;
                embeds.push(FlowEmbed::new(id, FlowEmbedKind::Table, slot));
                store.insert(
                    id,
                    EmbedPayload::Table {
                        headers: headers.clone(),
                        rows: rows.clone(),
                    },
                );
                need_para_sep = false;
            }
            Block::Image { file_id, alt } => {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                let slot = text.chars().count();
                text.push(EMBED_CHAR);
                text.push('\n');
                let id = next_id;
                next_id += 1;
                embeds.push(FlowEmbed::new(id, FlowEmbedKind::Image, slot));
                store.insert(
                    id,
                    EmbedPayload::Image {
                        file_id: file_id.clone(),
                        alt: alt.clone(),
                    },
                );
                need_para_sep = false;
            }
            Block::Items { .. } => {}
        }
    }

    (
        FlowState {
            text,
            style_runs: runs,
            embeds,
            cursor: 0,
            selection: None,
            scroll: 0.0,
            has_ime: false,
            ime_anchor: 0,
            prefer_next_row: true,
        },
        store,
    )
}

/// 单一 Flow → blocks（嵌入还原为 Table/Image）。
fn flow_to_blocks(state: &FlowState, store: &HashMap<u64, EmbedPayload>) -> Vec<Block> {
    let chars: Vec<char> = state.text.chars().collect();
    let mut out: Vec<Block> = Vec::new();
    let mut i = 0usize;

    // `keep_trailing_empty`：区间以 \n 结尾时是否保留尾部空段。
    // 文末为 true（连续回车）；嵌入前为 false（嵌入前的 \n 只是分隔，不另开空段）。
    let flush_para =
        |out: &mut Vec<Block>,
         state: &FlowState,
         chars: &[char],
         a: usize,
         b: usize,
         keep_trailing_empty: bool| {
            if a >= b {
                return;
            }
            let mut start = a;
            for j in a..b {
                if chars[j] == '\n' {
                    push_para_spans(out, state, chars, start, j);
                    start = j + 1;
                }
            }
            if start < b {
                push_para_spans(out, state, chars, start, b);
            } else if keep_trailing_empty {
                push_para_spans(out, state, chars, start, start);
            }
        };

    let mut seg_start = 0usize;
    while i < chars.len() {
        if chars[i] == EMBED_CHAR {
            flush_para(&mut out, state, &chars, seg_start, i, false);
            if let Some(emb) = state.embeds.iter().find(|e| e.slot == i) {
                match store.get(&emb.id) {
                    Some(EmbedPayload::Table { headers, rows }) => {
                        out.push(Block::Table {
                            headers: headers.clone(),
                            rows: rows.clone(),
                        });
                    }
                    Some(EmbedPayload::Image { file_id, alt }) => {
                        out.push(Block::Image {
                            file_id: file_id.clone(),
                            alt: alt.clone(),
                        });
                    }
                    None => {}
                }
            }
            i += 1;
            if i < chars.len() && chars[i] == '\n' {
                i += 1;
            }
            seg_start = i;
            continue;
        }
        i += 1;
    }
    flush_para(&mut out, state, &chars, seg_start, chars.len(), true);

    if out.is_empty() {
        out.push(Block::Paragraph {
            spans: vec![Span::plain("")],
        });
    }
    out
}

fn push_para_spans(
    out: &mut Vec<Block>,
    state: &FlowState,
    chars: &[char],
    ls: usize,
    le: usize,
) {
    if ls > le {
        return;
    }
    // 纯空段落：仅当需要占位时保留
    if ls == le {
        out.push(Block::Paragraph {
            spans: vec![Span::plain("")],
        });
        return;
    }
    let mut spans = Vec::new();
    let mut i = ls;
    while i < le {
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
        let end = run.end.min(le).max(i + 1);
        let end = {
            let mut e = end;
            while e > i && e <= le && chars.get(e - 1) == Some(&EMBED_CHAR) {
                e -= 1;
            }
            // 不要跨过嵌入
            let mut e2 = i + 1;
            while e2 < end && e2 < le && chars[e2] != EMBED_CHAR {
                e2 += 1;
            }
            e2
        };
        let slice: String = chars[i..end].iter().collect();
        if !slice.is_empty() {
            spans.push(Span {
                text: slice,
                color: run.color.clone(),
                link: run.link.clone(),
                bold: run.bold,
                italic: run.italic,
                underline: run.underline,
                size: run.size,
            });
        }
        i = end;
    }
    if spans.is_empty() {
        spans.push(Span::plain(""));
    }
    out.push(Block::Paragraph {
        spans: merge_adjacent_spans(spans),
    });
}

/// 测试/旧路径：仅段落 → Flow（无嵌入）。
#[allow(dead_code)]
fn paragraphs_to_flow(blocks: &[Block]) -> FlowState {
    blocks_to_flow(blocks).0
}

#[allow(dead_code)]
fn flow_to_paragraphs(state: &FlowState) -> Vec<Vec<Span>> {
    flow_to_blocks(state, &HashMap::new())
        .into_iter()
        .filter_map(|b| match b {
            Block::Paragraph { spans } => Some(spans),
            _ => None,
        })
        .collect()
}

/// 选区样式补丁（字段为 Some 才改）。
#[derive(Clone, Default)]
struct StylePatch {
    color: Option<Option<String>>,
    link: Option<Option<String>>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    size: Option<Option<u8>>,
}

fn span_with_text(sp: &Span, text: String) -> Span {
    Span {
        text,
        color: sp.color.clone(),
        link: sp.link.clone(),
        bold: sp.bold,
        italic: sp.italic,
        underline: sp.underline,
        size: sp.size,
    }
}

fn same_span_style(a: &Span, b: &Span) -> bool {
    a.color == b.color
        && a.link == b.link
        && a.bold == b.bold
        && a.italic == b.italic
        && a.underline == b.underline
        && a.size == b.size
}

const FONT_SIZES: &[u8] = &[10, 12, 14, 16, 18, 20];

#[derive(Debug, Clone)]
pub struct Doc {
    pub title: String,
    pub blocks: Vec<Block>,
}

impl Default for Doc {
    fn default() -> Self {
        Self::empty()
    }
}

impl Doc {
    pub fn empty() -> Self {
        Self {
            title: String::new(),
            blocks: memo_doc::DocBody::empty().blocks,
        }
    }

    /// 从已拆分的 title/content 装入。
    pub fn from_store(title: &str, content: &str) -> Self {
        let body = memo_doc::parse_body(content);
        Self {
            title: title.to_string(),
            blocks: body.blocks,
        }
    }

    pub fn to_store(&self, default_title: &str) -> (String, String) {
        let title = if self.title.trim().is_empty() {
            default_title.to_string()
        } else {
            self.title.trim().to_string()
        };
        (title, serialize_body_only(self))
    }
}

fn serialize_body_only(doc: &Doc) -> String {
    memo_doc::serialize_body(&doc.blocks)
}

/// 合并为编辑用全文：第一行标题，其余正文（测试 / 模板用）。
#[allow(dead_code)]
pub fn join_note(title: &str, content: &str) -> String {
    let title = title.trim_end_matches('\r').trim_end_matches('\n');
    let content = content.trim_start_matches('\r').trim_start_matches('\n');
    if content.is_empty() {
        title.to_string()
    } else if title.is_empty() {
        content.to_string()
    } else {
        format!("{title}\n{content}")
    }
}

/// 拆分：首个非空行 = 标题，其余 = 正文。
pub fn split_note(note: &str, default_title: &str) -> (String, String) {
    let mut lines = note.lines();
    let mut title = String::new();
    while let Some(line) = lines.next() {
        let t = line.trim();
        if !t.is_empty() {
            title = t.to_string();
            break;
        }
    }
    let content: String = lines.collect::<Vec<_>>().join("\n");
    let content = content.trim_end_matches('\n').to_string();
    if title.is_empty() {
        (default_title.to_string(), content)
    } else {
        (title, content)
    }
}

#[allow(dead_code)]
pub fn parse(note: &str) -> Doc {
    let (title, content) = split_note(note, "");
    Doc::from_store(&title, &content)
}

/// 序列化：标题一行 + memo-doc 正文（供模板 pristine 比较）。
pub fn serialize(doc: &Doc) -> String {
    let title = doc.title.trim();
    let body = serialize_body_only(doc);
    if body.trim().is_empty() {
        title.to_string()
    } else if title.is_empty() {
        body
    } else {
        format!("{title}\n{body}")
    }
}

fn ensure_body_paragraph(doc: &mut Doc) {
    if !doc
        .blocks
        .iter()
        .any(|b| matches!(b, Block::Paragraph { .. }))
    {
        doc.blocks.insert(
            0,
            Block::Paragraph {
                spans: vec![Span::plain("")],
            },
        );
    }
}

const ORDERED_LIST_STYLES: &[ListStyle] = &[
    ListStyle::Decimal,
    ListStyle::LowerAlpha,
    ListStyle::UpperAlpha,
    ListStyle::LowerRoman,
    ListStyle::UpperRoman,
];

const BULLET_LIST_STYLES: &[ListStyle] = &[
    ListStyle::Disc,
    ListStyle::Dash,
    ListStyle::Circle,
    ListStyle::Asterisk,
];

#[allow(dead_code)]
fn apply_list_mark_to_spans(spans: &mut Vec<Span>, style: Option<ListStyle>, number: usize) {
    let plain = paragraph_full_text(spans);
    let body = parse_list_prefix(&plain)
        .map(|(_, _, b, _)| b.to_string())
        .unwrap_or(plain);
    let new_text = match style {
        Some(s) => format!("{}{body}", s.prefix(number)),
        None => body,
    };
    sync_spans_from_plain(spans, &new_text);
}

/// 目标行：整篇 / 选区覆盖行 / 光标行。
fn list_target_line_indices(ctx: &egui::Context, state: &FlowState, id_salt: &str) -> Vec<usize> {
    if doc_select_all_active(ctx, id_salt) {
        return (0..state.text.split('\n').count().max(1)).collect();
    }
    if let Some(sel) = ctx.data(|d| d.get_temp::<DocTextSelection>(doc_text_sel_id(id_salt))) {
        if sel.start < sel.end {
            let (ls, _) = line_at_cursor(&state.text, sel.start);
            let (le_start, _) = line_at_cursor(&state.text, sel.end.saturating_sub(1).max(sel.start));
            return flow_line_indices_covering(&state.text, ls, le_start);
        }
    }
    let (ls, _) = line_at_cursor(&state.text, state.cursor);
    let mut char_at = 0usize;
    let mut line_i = 0usize;
    for (i, line) in state.text.split('\n').enumerate() {
        if char_at == ls {
            line_i = i;
            break;
        }
        char_at += line.chars().count() + 1;
        line_i = i;
    }
    vec![line_i]
}

fn flow_cursor_line_style(ctx: &egui::Context, doc: &Doc, id_salt: &str) -> Option<ListStyle> {
    let (state, _) = blocks_to_flow(&doc.blocks);
    let mut state = state;
    if let Some(c) = read_flow_caret(ctx, id_salt, 0) {
        restore_flow_cursor(&mut state, c);
    }
    let indices = list_target_line_indices(ctx, &state, id_salt);
    let lines: Vec<&str> = state.text.split('\n').collect();
    indices.into_iter().find_map(|i| {
        lines
            .get(i)
            .and_then(|l| parse_list_prefix(l).map(|(s, _, _, _)| s))
    })
}

/// 目标：整篇 / 选区覆盖行 / 光标行。
/// `want = Some(s)`：已全是 `s` 则去掉，否则加上/切换；`None`：去掉任意列表前缀。
fn toggle_list_style(ctx: &egui::Context, doc: &mut Doc, id_salt: &str, want: Option<ListStyle>) {
    ctx.data_mut(|d| {
        d.insert_temp(doc_toolbar_hit_id(id_salt), true);
    });
    ensure_body_paragraph(doc);

    let (mut state, store) = blocks_to_flow(&doc.blocks);
    if let Some(c) = read_flow_caret(ctx, id_salt, 0) {
        restore_flow_cursor(&mut state, c);
    }

    let line_indices = list_target_line_indices(ctx, &state, id_salt);
    apply_list_style_to_lines(&mut state, &line_indices, want);
    save_flow_cursor(ctx, id_salt, 0, &state);
    // 正文被工具栏改写：丢掉会话，下一帧按 snap 恢复
    ctx.data_mut(|d| {
        d.remove_temp::<FlowSession>(doc_flow_session_id(id_salt, 0));
    });
    doc.blocks = flow_to_blocks(&state, &store);
    ensure_body_paragraph(doc);
}

fn flow_line_indices_covering(text: &str, first_line_start: usize, last_line_start: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut char_at = 0usize;
    for (i, line) in text.split('\n').enumerate() {
        if char_at >= first_line_start && char_at <= last_line_start {
            out.push(i);
        }
        char_at += line.chars().count() + 1;
    }
    if out.is_empty() {
        out.push(0);
    }
    out
}

fn paragraph_full_text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// Word 式：链接在正文里蓝字+下划线，不拆成外置芯片。
fn link_style_color() -> Color32 {
    // 接近 Word 默认超链接蓝
    Color32::from_rgb(0x05, 0x63, 0xC1)
}

fn span_text_format(ui: &egui::Ui, sp: &Span, default_color: Color32) -> egui::TextFormat {
    let mut font_id = egui::TextStyle::Body.resolve(ui.style());
    if let Some(sz) = sp.size {
        font_id.size = sz as f32;
    }
    // 加粗：切到 bold 字族（fonts.rs 注入），避免只改颜色看不出效果
    if sp.bold {
        font_id.family = egui::FontFamily::Name("bold".into());
    }
    let (color, force_underline) = if sp.link.is_some() {
        (link_style_color(), true)
    } else {
        (
            sp.color
                .as_deref()
                .and_then(parse_hex_color)
                .unwrap_or(if sp.bold {
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
        italics: sp.italic,
        ..Default::default()
    };
    if force_underline || sp.underline {
        format.underline = Stroke::new(1.0, color);
    }
    format
}

fn spans_layout_job(ui: &egui::Ui, text: &str, spans: &[Span], wrap_width: f32) -> egui::text::LayoutJob {
    let font_id = egui::TextStyle::Body.resolve(ui.style());
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
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        job.append(
            "",
            0.0,
            egui::TextFormat {
                font_id,
                color: default_color,
                ..Default::default()
            },
        );
        return job;
    }
    let mut idx = 0usize;
    for sp in spans {
        if idx >= chars.len() {
            break;
        }
        let n = sp.text.chars().count();
        if n == 0 {
            continue;
        }
        let take = n.min(chars.len() - idx);
        let slice: String = chars[idx..idx + take].iter().collect();
        idx += take;
        job.append(&slice, 0.0, span_text_format(ui, sp, default_color));
    }
    if idx < chars.len() {
        let rest: String = chars[idx..].iter().collect();
        job.append(
            &rest,
            0.0,
            egui::TextFormat {
                font_id,
                color: default_color,
                ..Default::default()
            },
        );
    }
    job
}

/// 编辑后按公共前后缀回写各 span，尽量保留颜色/链接。
fn sync_spans_from_plain(spans: &mut Vec<Span>, new_text: &str) {
    let old = paragraph_full_text(spans);
    if old == new_text {
        return;
    }
    if spans.len() <= 1 {
        if let Some(s) = spans.first_mut() {
            s.text = new_text.to_string();
        } else {
            *spans = vec![Span::plain(new_text.to_string())];
        }
        return;
    }

    let old_c: Vec<char> = old.chars().collect();
    let new_c: Vec<char> = new_text.chars().collect();
    let mut pre = 0usize;
    while pre < old_c.len() && pre < new_c.len() && old_c[pre] == new_c[pre] {
        pre += 1;
    }
    let mut suf = 0usize;
    while suf < old_c.len().saturating_sub(pre)
        && suf < new_c.len().saturating_sub(pre)
        && old_c[old_c.len() - 1 - suf] == new_c[new_c.len() - 1 - suf]
    {
        suf += 1;
    }
    let suf_start_old = old_c.len() - suf;
    let mid_new: String = new_c[pre..new_c.len() - suf].iter().collect();

    let mut offset = 0usize;
    let mut out = Vec::new();
    let mut mid_done = false;
    for sp in spans.iter() {
        let len = sp.text.chars().count();
        let a = offset;
        let b = offset + len;
        offset = b;

        if a < pre {
            let end = b.min(pre);
            let t: String = sp.text.chars().take(end - a).collect();
            if !t.is_empty() {
                out.push(span_with_text(sp, t));
            }
        }
        if !mid_done && a < suf_start_old && b > pre {
            if !mid_new.is_empty() {
                out.push(span_with_text(sp, mid_new.clone()));
            }
            mid_done = true;
        }
        if b > suf_start_old {
            let local = suf_start_old.saturating_sub(a).min(len);
            let t: String = sp.text.chars().skip(local).collect();
            if !t.is_empty() {
                out.push(span_with_text(sp, t));
            }
        }
    }
    if !mid_done && !mid_new.is_empty() {
        out.push(Span::plain(mid_new));
    }
    *spans = merge_adjacent_spans(out);
}

/// 独立插入链接芯片（已不用：链接只能套到选区）。测试仍覆盖此路径。
#[allow(dead_code)]
fn insert_inline_link(spans: &mut Vec<Span>, display: &str, url: &str) {
    let display = display.trim();
    let url = normalize_url(url);
    if display.is_empty() || url.is_empty() {
        return;
    }
    while spans.last().is_some_and(|s| {
        s.link.is_none()
            && s.text.is_empty()
            && s.color.is_none()
            && !s.bold
            && !s.italic
            && !s.underline
            && s.size.is_none()
    }) {
        spans.pop();
    }
    spans.push(Span {
        text: display.to_string(),
        color: None,
        link: Some(url),
        bold: false,
        italic: false,
        underline: false,
        size: None,
    });
    spans.push(Span::plain(""));
}

fn merge_adjacent_spans(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for sp in spans {
        if sp.text.is_empty()
            && sp.link.is_none()
            && sp.color.is_none()
            && !sp.bold
            && !sp.italic
            && !sp.underline
            && sp.size.is_none()
        {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if same_span_style(last, &sp) {
                last.text.push_str(&sp.text);
                continue;
            }
        }
        out.push(sp);
    }
    if out.is_empty() {
        out.push(Span::plain(""));
    }
    out
}

/// 按字符下标给选区上样式（颜色 / 链接 / BIU / 字号）。
fn apply_style_to_char_range(spans: &mut Vec<Span>, start: usize, end: usize, patch: StylePatch) {
    if start >= end {
        return;
    }
    let mut char_i = 0usize;
    let mut out = Vec::new();
    for sp in spans.iter() {
        let len = sp.text.chars().count();
        let span_start = char_i;
        let span_end = char_i + len;
        char_i = span_end;

        if len == 0 {
            if sp.link.is_some()
                || sp.color.is_some()
                || sp.bold
                || sp.italic
                || sp.underline
                || sp.size.is_some()
            {
                out.push(sp.clone());
            }
            continue;
        }
        if span_end <= start || span_start >= end {
            out.push(sp.clone());
            continue;
        }

        let local_start = start.saturating_sub(span_start).min(len);
        let local_end = end.saturating_sub(span_start).min(len);
        let before: String = sp.text.chars().take(local_start).collect();
        let mid: String = sp
            .text
            .chars()
            .skip(local_start)
            .take(local_end.saturating_sub(local_start))
            .collect();
        let after: String = sp.text.chars().skip(local_end).collect();

        if !before.is_empty() {
            out.push(span_with_text(sp, before));
        }
        if !mid.is_empty() {
            let mut n = span_with_text(sp, mid);
            if let Some(c) = &patch.color {
                n.color = c.clone();
            }
            if let Some(l) = &patch.link {
                n.link = l.clone();
                if l.is_some() {
                    n.color = None;
                }
            }
            if let Some(b) = patch.bold {
                n.bold = b;
            }
            if let Some(i) = patch.italic {
                n.italic = i;
            }
            if let Some(u) = patch.underline {
                n.underline = u;
            }
            if let Some(sz) = &patch.size {
                n.size = *sz;
            }
            out.push(n);
        }
        if !after.is_empty() {
            out.push(span_with_text(sp, after));
        }
    }
    *spans = merge_adjacent_spans(out);
}

fn selection_anchor(output: &egui::text_edit::TextEditOutput, range: &egui::text::CursorRange) -> egui::Pos2 {
    let cursors = range.sorted_cursors();
    let r0 = output.galley.pos_from_cursor(&cursors[0]);
    let r1 = output.galley.pos_from_cursor(&cursors[1]);
    let mid_x = (r0.center().x + r1.center().x) * 0.5;
    let top_y = r0.top().min(r1.top());
    egui::pos2(
        output.galley_pos.x + mid_x,
        (output.galley_pos.y + top_y - 40.0).max(output.response.rect.top() - 40.0),
    )
}

fn normalize_url(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.contains("://") || t.starts_with("mailto:") {
        t.to_string()
    } else {
        format!("https://{t}")
    }
}

fn parse_hex_color(s: &str) -> Option<Color32> {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&t[0..2], 16).ok()?;
    let g = u8::from_str_radix(&t[2..4], 16).ok()?;
    let b = u8::from_str_radix(&t[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

fn open_url(url: &str) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

fn toggle_style_on_last_para(doc: &mut Doc, patch: StylePatch) {
    if let Some(Block::Paragraph { spans }) = doc
        .blocks
        .iter_mut()
        .rev()
        .find(|b| matches!(b, Block::Paragraph { .. }))
    {
        let len = paragraph_full_text(spans).chars().count();
        if len == 0 {
            if spans.is_empty() {
                spans.push(Span::plain(""));
            }
            if let Some(sp) = spans.last_mut() {
                if let Some(b) = patch.bold {
                    sp.bold = b;
                }
                if let Some(i) = patch.italic {
                    sp.italic = i;
                }
                if let Some(u) = patch.underline {
                    sp.underline = u;
                }
                if let Some(sz) = patch.size {
                    sp.size = sz;
                }
                if let Some(c) = patch.color {
                    sp.color = c;
                }
            }
        } else {
            apply_style_to_char_range(spans, 0, len, patch);
        }
    }
}

/// 工具条芯片内按钮高度（对齐参考图紧凑顶栏）。
const TOOL_H: f32 = 22.0;
/// 工具栏整条固定高度（含内边距）；拉高窗口时不跟着变。
const TOOLBAR_STRIP_H: f32 = 30.0;
/// 插入链接副条固定高度。
const LINK_STRIP_H: f32 = 30.0;

fn tool_btn(ui: &mut egui::Ui, label: RichText, tip: &str) -> egui::Response {
    let desired = Vec2::new(26.0, TOOL_H);
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());
    let hovered = resp.hovered();
    let fill = if hovered {
        theme::shell_accent_soft()
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if hovered {
        Stroke::new(1.0, theme::shell_accent().linear_multiply(0.4))
    } else {
        Stroke::NONE
    };
    ui.painter()
        .rect(rect, Rounding::same(4.0), fill, stroke);
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // 用 painter 画字，勿再 put Label（会盖住 allocate 的 click）
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(false),
        f32::INFINITY,
        egui::FontSelection::Default,
    );
    let g = galley.size();
    let pos = egui::pos2(rect.center().x - g.x * 0.5, rect.center().y - g.y * 0.5);
    ui.painter().galley(pos, galley, theme::text());
    resp.on_hover_text(tip)
}

/// 白底描边圆角小组（垂直中线对齐）。
fn tool_group(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(5.0))
        .inner_margin(Margin::symmetric(3.0, 1.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.spacing_mut().button_padding = Vec2::new(4.0, 0.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_height(TOOL_H);
                add_contents(ui);
            });
        });
}

/// 紧凑弹出按钮（字号 / 颜色），外观与 tool_btn 一致。
fn tool_popup_btn(
    ui: &mut egui::Ui,
    id: Id,
    label: RichText,
    tip: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let resp = tool_btn(ui, label, tip);
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup_below_widget(ui, id, &resp, |ui| {
        ui.set_min_width(96.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        add_contents(ui);
    });
    resp
}

fn table_hairline() -> Stroke {
    Stroke::new(1.0, theme::border())
}

fn table_header_fill() -> Color32 {
    theme::panel()
}

/// Word 式插入表格选格上限。
const TABLE_INSERT_MAX: usize = 8;

#[allow(dead_code)]
fn push_empty_table(doc: &mut Doc, cols: usize, rows: usize) {
    let cols = cols.clamp(1, TABLE_INSERT_MAX);
    let rows = rows.clamp(1, TABLE_INSERT_MAX);
    ensure_body_paragraph(doc);
    // 追加到 blocks 末尾；单 Flow 同步后会出现在文末（光标处插入由顶栏调用方可扩展）
    doc.blocks.push(Block::Table {
        headers: vec![String::new(); cols],
        rows: vec![vec![String::new(); cols]; rows.saturating_sub(1)],
    });
}

fn insert_image_at_cursor(
    ctx: &egui::Context,
    doc: &mut Doc,
    id_salt: &str,
    file_id: String,
    alt: String,
) {
    ensure_body_paragraph(doc);
    let (mut state, mut store) = blocks_to_flow(&doc.blocks);
    if let Some(c) = read_flow_caret(ctx, id_salt, 0) {
        restore_flow_cursor(&mut state, c);
    }
    let id = store.keys().max().copied().unwrap_or(0) + 1;
    FlowEditor::insert_embed(&mut state, FlowEmbedKind::Image, id);
    store.insert(id, EmbedPayload::Image { file_id, alt });
    save_flow_cursor(ctx, id_salt, 0, &state);
    ctx.data_mut(|d| {
        d.remove_temp::<FlowSession>(doc_flow_session_id(id_salt, 0));
    });
    doc.blocks = flow_to_blocks(&state, &store);
}

/// 在当前光标处插入空表（单 Flow）。
fn insert_table_at_cursor(
    ctx: &egui::Context,
    doc: &mut Doc,
    id_salt: &str,
    cols: usize,
    rows: usize,
) {
    let cols = cols.clamp(1, TABLE_INSERT_MAX);
    let rows = rows.clamp(1, TABLE_INSERT_MAX);
    ensure_body_paragraph(doc);
    let (mut state, mut store) = blocks_to_flow(&doc.blocks);
    if let Some(c) = read_flow_caret(ctx, id_salt, 0) {
        restore_flow_cursor(&mut state, c);
    }
    let id = store.keys().max().copied().unwrap_or(0) + 1;
    FlowEditor::insert_embed(&mut state, FlowEmbedKind::Table, id);
    store.insert(
        id,
        EmbedPayload::Table {
            headers: vec![String::new(); cols],
            rows: vec![vec![String::new(); cols]; rows.saturating_sub(1)],
        },
    );
    save_flow_cursor(ctx, id_salt, 0, &state);
    ctx.data_mut(|d| {
        d.remove_temp::<FlowSession>(doc_flow_session_id(id_salt, 0));
    });
    doc.blocks = flow_to_blocks(&state, &store);
}

/// Word 式选格：悬停高亮左上 `c×r`，单击返回 `(cols, rows)`（含表头行）。
fn table_size_picker(ui: &mut egui::Ui, popup_id: Id) -> Option<(usize, usize)> {
    let hover_id = Id::new(("tbl_pick_hover", popup_id));
    let mut hover = ui
        .ctx()
        .data(|d| d.get_temp::<(usize, usize)>(hover_id))
        .unwrap_or((0, 0));
    let mut picked: Option<(usize, usize)> = None;
    let cell = 14.0;
    let gap = 2.0;
    ui.spacing_mut().item_spacing = Vec2::splat(gap);
    for r in 1..=TABLE_INSERT_MAX {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for c in 1..=TABLE_INSERT_MAX {
                let on = hover.0 > 0 && c <= hover.0 && r <= hover.1;
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(cell), Sense::click());
                let fill = if on {
                    theme::shell_accent_soft()
                } else {
                    theme::card()
                };
                let stroke = Stroke::new(
                    1.0,
                    if on {
                        theme::shell_accent()
                    } else {
                        theme::border()
                    },
                );
                ui.painter()
                    .rect(rect, Rounding::same(2.0), fill, stroke);
                if resp.hovered() {
                    hover = (c, r);
                }
                if resp.clicked() {
                    picked = Some((c, r));
                }
            }
        });
    }
    ui.ctx().data_mut(|d| d.insert_temp(hover_id, hover));
    let tip = if hover.0 > 0 {
        format!("{} × {} 表格", hover.0, hover.1)
    } else {
        "选择行列".into()
    };
    ui.add_space(4.0);
    ui.label(RichText::new(tip).size(11.0).color(theme::text_muted()));
    picked
}

fn tbl_ctx_id(id_salt: &str, block_idx: usize) -> Id {
    Id::new(("doc_tbl_ctx", id_salt, block_idx))
}

#[derive(Clone, Default)]
struct TblCtxMenu {
    row: usize,
    col: usize,
    pos: egui::Pos2,
}

#[derive(Clone, Copy)]
enum TblCtxAction {
    RowAbove,
    RowBelow,
    ColLeft,
    ColRight,
    DelRow,
    DelTable,
}

fn insert_table_col(headers: &mut Vec<String>, rows: &mut Vec<Vec<String>>, at: usize) {
    let at = at.min(headers.len());
    headers.insert(at, String::new());
    for row in rows.iter_mut() {
        let i = at.min(row.len());
        row.insert(i, String::new());
    }
}

fn apply_tbl_ctx_action(
    headers: &mut Vec<String>,
    rows: &mut Vec<Vec<String>>,
    cell: (usize, usize),
    action: TblCtxAction,
) -> bool {
    let (row, col) = cell;
    let ncols = headers.len().max(1);
    match action {
        TblCtxAction::RowAbove => {
            let at = if row == 0 {
                0
            } else {
                (row - 1).min(rows.len())
            };
            rows.insert(at, vec![String::new(); ncols]);
            false
        }
        TblCtxAction::RowBelow => {
            let at = if row == 0 { 0 } else { row.min(rows.len()) };
            rows.insert(at, vec![String::new(); ncols]);
            false
        }
        TblCtxAction::ColLeft => {
            insert_table_col(headers, rows, col.min(headers.len()));
            false
        }
        TblCtxAction::ColRight => {
            insert_table_col(headers, rows, (col + 1).min(headers.len()));
            false
        }
        TblCtxAction::DelRow => {
            if row >= 1 {
                let i = row - 1;
                if i < rows.len() {
                    rows.remove(i);
                }
            }
            false
        }
        TblCtxAction::DelTable => true,
    }
}

/// 表格右键菜单；点项返回 `(动作, 格坐标)` 并关闭，点空白（左键）关闭。
fn show_table_ctx_menu(
    ui: &mut egui::Ui,
    id_salt: &str,
    block_idx: usize,
) -> Option<(TblCtxAction, usize, usize)> {
    let ctx_id = tbl_ctx_id(id_salt, block_idx);
    let Some(menu) = ui.ctx().data(|d| d.get_temp::<TblCtxMenu>(ctx_id)) else {
        return None;
    };
    let cell = (menu.row, menu.col);
    let area_id = Id::new(("doc_tbl_ctx_area", id_salt, block_idx));
    let mut action: Option<TblCtxAction> = None;
    let mut close = false;

    let area = egui::Area::new(area_id)
        .order(Order::Foreground)
        .fixed_pos(menu.pos)
        .constrain(true)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            Frame::none()
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::border_strong()))
                .rounding(Rounding::same(6.0))
                .inner_margin(Margin::symmetric(6.0, 4.0))
                .shadow(egui::epaint::Shadow {
                    offset: Vec2::new(0.0, 2.0),
                    blur: 8.0,
                    spread: 0.0,
                    color: Color32::from_black_alpha(40),
                })
                .show(ui, |ui| {
                    ui.set_min_width(148.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let item = |ui: &mut egui::Ui, label: &str| {
                        ui.add(
                            egui::Button::new(RichText::new(label).size(12.5).color(theme::text()))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE)
                                .min_size(Vec2::new(140.0, 22.0)),
                        )
                    };
                    if item(ui, "在上方插入行").clicked() {
                        action = Some(TblCtxAction::RowAbove);
                        close = true;
                    }
                    if item(ui, "在下方插入行").clicked() {
                        action = Some(TblCtxAction::RowBelow);
                        close = true;
                    }
                    ui.separator();
                    if item(ui, "在左侧插入列").clicked() {
                        action = Some(TblCtxAction::ColLeft);
                        close = true;
                    }
                    if item(ui, "在右侧插入列").clicked() {
                        action = Some(TblCtxAction::ColRight);
                        close = true;
                    }
                    ui.separator();
                    let del_row = ui.add_enabled(
                        menu.row > 0,
                        egui::Button::new(
                            RichText::new("删除行")
                                .size(12.5)
                                .color(theme::text()),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .min_size(Vec2::new(140.0, 22.0)),
                    );
                    if del_row.clicked() {
                        action = Some(TblCtxAction::DelRow);
                        close = true;
                    }
                    if item(ui, "删除表格").clicked() {
                        action = Some(TblCtxAction::DelTable);
                        close = true;
                    }
                });
        });

    let menu_hot = area.response.hovered() || area.response.contains_pointer();
    // 左键点菜单外关闭（避开本帧右键打开）
    if !menu_hot
        && ui.input(|i| i.pointer.primary_clicked())
        && !ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary))
    {
        close = true;
    }
    if close {
        ui.ctx()
            .data_mut(|d| d.remove_temp::<TblCtxMenu>(ctx_id));
    }
    action.map(|a| (a, cell.0, cell.1))
}

fn open_table_ctx_menu(
    ui: &egui::Ui,
    id_salt: &str,
    block_idx: usize,
    row: usize,
    col: usize,
    fallback: egui::Pos2,
) {
    let pos = ui
        .ctx()
        .pointer_interact_pos()
        .unwrap_or(fallback);
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            tbl_ctx_id(id_salt, block_idx),
            TblCtxMenu { row, col, pos },
        );
    });
}

/// 格式工具条：固定高度浅灰顶条 + BIU | 字号/色 | 列表/表 | 链/图。
fn toolbar(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    data_dir: Option<&Path>,
    memo_id: &str,
    show_link: &mut bool,
    id_salt: &str,
) {
    let size_id = Id::new(("doc_toolbar_size", id_salt));
    let mut cur_size: u8 = ui.ctx().data(|d| d.get_temp(size_id)).unwrap_or(14);
    let fg = theme::text();
    let strip_w = fit_ui_width(ui);

    ui.allocate_ui_with_layout(
        Vec2::new(strip_w, TOOLBAR_STRIP_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            Frame::none()
                .fill(theme::panel())
                .inner_margin(Margin::symmetric(6.0, 0.0))
                .show(ui, |ui| {
                    let strip_w = fit_ui_width(ui).min(strip_w);
                    ui.set_max_width(strip_w);
                    ui.set_max_height(TOOLBAR_STRIP_H);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.set_height(TOOLBAR_STRIP_H);

                let select_all = doc_select_all_active(ui.ctx(), id_salt);
                let style_tip = if select_all {
                    "整篇选中时样式主要作用于正文段落；再点一次可取消"
                } else {
                    "有选区则切换选中文字；再点一次可取消"
                };
                tool_group(ui, |ui| {
                    if tool_btn(
                        ui,
                        RichText::new("B").strong().size(12.5).color(fg),
                        &format!("加粗（{style_tip}）"),
                    )
                    .clicked()
                    {
                        let on = biu_toggle_on(ui.ctx(), doc, id_salt, |s| s.bold);
                        apply_toolbar_style(
                            ui.ctx(),
                            doc,
                            id_salt,
                            StylePatch {
                                bold: Some(on),
                                ..Default::default()
                            },
                        );
                    }
                    if tool_btn(
                        ui,
                        RichText::new("I").italics().size(12.5).color(fg),
                        &format!("斜体（{style_tip}）"),
                    )
                    .clicked()
                    {
                        let on = biu_toggle_on(ui.ctx(), doc, id_salt, |s| s.italic);
                        apply_toolbar_style(
                            ui.ctx(),
                            doc,
                            id_salt,
                            StylePatch {
                                italic: Some(on),
                                ..Default::default()
                            },
                        );
                    }
                    if tool_btn(
                        ui,
                        RichText::new("U").underline().size(12.5).color(fg),
                        &format!("下划线（{style_tip}）"),
                    )
                    .clicked()
                    {
                        let on = biu_toggle_on(ui.ctx(), doc, id_salt, |s| s.underline);
                        apply_toolbar_style(
                            ui.ctx(),
                            doc,
                            id_salt,
                            StylePatch {
                                underline: Some(on),
                                ..Default::default()
                            },
                        );
                    }
                });

                tool_group(ui, |ui| {
                    let size_popup = Id::new(("doc_fs_popup", id_salt));
                    tool_popup_btn(
                        ui,
                        size_popup,
                        RichText::new(format!("{cur_size}")).size(12.0).color(fg),
                        "字号",
                        |ui| {
                            for &sz in FONT_SIZES {
                                if ui
                                    .selectable_label(
                                        cur_size == sz,
                                        RichText::new(format!("{sz}")).size(13.0),
                                    )
                                    .clicked()
                                {
                                    cur_size = sz;
                                    apply_toolbar_style(
                                        ui.ctx(),
                                        doc,
                                        id_salt,
                                        StylePatch {
                                            size: Some(Some(sz)),
                                            ..Default::default()
                                        },
                                    );
                                    ui.memory_mut(|m| m.close_popup());
                                }
                            }
                        },
                    );
                    let color_popup = Id::new(("doc_color_popup", id_salt));
                    tool_popup_btn(
                        ui,
                        color_popup,
                        RichText::new("A").underline().size(12.5).color(fg),
                        "字体颜色",
                        |ui| {
                            ui.label(
                                RichText::new("字体颜色")
                                    .size(11.0)
                                    .color(theme::text_muted()),
                            );
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                                for (name, hex) in memo_doc::palette() {
                                    let fill = if hex.is_empty() {
                                        theme::panel()
                                    } else {
                                        parse_hex_color(hex).unwrap_or(theme::panel())
                                    };
                                    let swatch = ui.add(
                                        egui::Button::new(RichText::new("●").color(
                                            if hex.is_empty() {
                                                theme::text_muted()
                                            } else {
                                                fill
                                            },
                                        ))
                                        .fill(theme::card())
                                        .stroke(Stroke::new(1.0, theme::border()))
                                        .min_size(Vec2::splat(18.0)),
                                    );
                                    if swatch.on_hover_text(*name).clicked() {
                                        let color = if hex.is_empty() {
                                            None
                                        } else {
                                            Some((*hex).to_string())
                                        };
                                        apply_toolbar_style(
                                            ui.ctx(),
                                            doc,
                                            id_salt,
                                            StylePatch {
                                                color: Some(color),
                                                ..Default::default()
                                            },
                                        );
                                        ui.memory_mut(|m| m.close_popup());
                                    }
                                }
                            });
                        },
                    );
                });

                tool_group(ui, |ui| {
                    let num_popup = Id::new(("doc_list_num_popup", id_salt));
                    let cur_list = flow_cursor_line_style(ui.ctx(), doc, id_salt);
                    let on_ordered = cur_list.is_some_and(|s| s.is_ordered());
                    let on_bullet = cur_list.is_some_and(|s| !s.is_ordered());
                    let num_resp = tool_btn(
                        ui,
                        RichText::new("1.").size(12.0).color(fg),
                        "编号列表：已编号时再点可取消；否则打开样式菜单",
                    );
                    if num_resp.clicked() {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(doc_toolbar_hit_id(id_salt), true);
                        });
                        if on_ordered {
                            toggle_list_style(ui.ctx(), doc, id_salt, None);
                        } else {
                            ui.memory_mut(|m| m.toggle_popup(num_popup));
                        }
                    }
                    egui::popup_below_widget(ui, num_popup, &num_resp, |ui| {
                        ui.set_min_width(120.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if ui
                            .selectable_label(false, RichText::new("取消编号").size(13.0))
                            .clicked()
                        {
                            toggle_list_style(ui.ctx(), doc, id_salt, None);
                            ui.memory_mut(|m| m.close_popup());
                        }
                        ui.separator();
                        for &st in ORDERED_LIST_STYLES {
                            if ui
                                .selectable_label(
                                    cur_list == Some(st),
                                    RichText::new(st.toolbar_label()).size(13.0),
                                )
                                .clicked()
                            {
                                toggle_list_style(ui.ctx(), doc, id_salt, Some(st));
                                ui.memory_mut(|m| m.close_popup());
                            }
                        }
                    });
                    let bullet_popup = Id::new(("doc_list_bullet_popup", id_salt));
                    let bullet_resp = tool_btn(
                        ui,
                        RichText::new("•").size(13.0).color(fg),
                        "项目符号：已是符号列表时再点可取消；否则打开样式菜单",
                    );
                    if bullet_resp.clicked() {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(doc_toolbar_hit_id(id_salt), true);
                        });
                        if on_bullet {
                            toggle_list_style(ui.ctx(), doc, id_salt, None);
                        } else {
                            ui.memory_mut(|m| m.toggle_popup(bullet_popup));
                        }
                    }
                    egui::popup_below_widget(ui, bullet_popup, &bullet_resp, |ui| {
                        ui.set_min_width(120.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if ui
                            .selectable_label(false, RichText::new("取消项目符号").size(13.0))
                            .clicked()
                        {
                            toggle_list_style(ui.ctx(), doc, id_salt, None);
                            ui.memory_mut(|m| m.close_popup());
                        }
                        ui.separator();
                        for &st in BULLET_LIST_STYLES {
                            if ui
                                .selectable_label(
                                    cur_list == Some(st),
                                    RichText::new(st.toolbar_label()).size(13.0),
                                )
                                .clicked()
                            {
                                toggle_list_style(ui.ctx(), doc, id_salt, Some(st));
                                ui.memory_mut(|m| m.close_popup());
                            }
                        }
                    });
                    let tbl_popup = Id::new(("doc_tbl_insert", id_salt));
                    tool_popup_btn(
                        ui,
                        tbl_popup,
                        RichText::new("▦").size(12.0).color(fg),
                        "插入表格（拖选行列）",
                        |ui| {
                            ui.set_min_width(8.0 * 16.0);
                            if let Some((cols, rows)) = table_size_picker(ui, tbl_popup) {
                                insert_table_at_cursor(ui.ctx(), doc, id_salt, cols, rows);
                                ui.memory_mut(|m| m.close_popup());
                            }
                        },
                    );
                });

                tool_group(ui, |ui| {
                    let has_sel = doc_has_text_selection(ui.ctx(), id_salt);
                    let link_label = RichText::new("链")
                        .size(12.0)
                        .color(if has_sel {
                            link_style_color()
                        } else {
                            theme::text_muted()
                        })
                        .strong();
                    let link_out = ui.add_enabled_ui(has_sel, |ui| {
                        tool_btn(ui, link_label, "为选中文字设为链接")
                    });
                    if has_sel {
                        if link_out.inner.clicked() {
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(doc_toolbar_hit_id(id_salt), true);
                            });
                            *show_link = !*show_link;
                        }
                    } else {
                        link_out
                            .response
                            .on_disabled_hover_text("请先选中一段文字");
                        *show_link = false;
                    }
                    if tool_btn(ui, RichText::new("图").size(12.0).color(fg), "插入图片").clicked()
                    {
                        if let (Some(dir), true) = (data_dir, !memo_id.is_empty()) {
                            if let Some(path) = save_dialog::open_image_dialog() {
                                if let Ok(bytes) = std::fs::read(&path) {
                                    let ext = path
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("png");
                                    if let Ok(file_id) =
                                        attachments::add_image(dir, memo_id, &bytes, ext)
                                    {
                                        let alt = path
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("图片")
                                            .to_string();
                                        insert_image_at_cursor(
                                            ui.ctx(),
                                            doc,
                                            id_salt,
                                            file_id,
                                            alt,
                                        );
                                    }
                                }
                            }
                        }
                    }
                });
                    });
                });
        },
    );

    ui.ctx().data_mut(|d| d.insert_temp(size_id, cur_size));
}

/// 设为链接副条：仅在已有选区时使用，把网址套到选中文字上。
fn toolbar_link_row(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    link_text: &mut String,
    link_url: &mut String,
    show_link: &mut bool,
    id_salt: &str,
) {
    if !doc_has_text_selection(ui.ctx(), id_salt) {
        link_text.clear();
        link_url.clear();
        *show_link = false;
        return;
    }

    let strip_w = fit_ui_width(ui);
    let flow_sel = ui
        .ctx()
        .data(|d| d.get_temp::<DocTextSelection>(doc_text_sel_id(id_salt)));
    if let Some(sel) = &flow_sel {
        if link_text.is_empty() && !sel.text.is_empty() {
            *link_text = sel.text.clone();
        }
    } else if doc_select_all_active(ui.ctx(), id_salt) && link_text.is_empty() {
        *link_text = doc_plain_export(doc);
    }

    ui.allocate_ui_with_layout(
        Vec2::new(strip_w, LINK_STRIP_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            Frame::none()
                .fill(theme::panel())
                .inner_margin(Margin::symmetric(6.0, 0.0))
                .show(ui, |ui| {
                    let strip_w = fit_ui_width(ui).min(strip_w);
                    ui.set_max_width(strip_w);
                    ui.set_max_height(LINK_STRIP_H);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.set_height(LINK_STRIP_H);
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(doc_toolbar_hit_id(id_salt), true);
                        });
                        let preview = if link_text.chars().count() > 16 {
                            format!("{}…", link_text.chars().take(16).collect::<String>())
                        } else {
                            link_text.clone()
                        };
                        ui.label(
                            RichText::new(format!("选区「{preview}」"))
                                .size(12.0)
                                .color(theme::text_muted()),
                        );
                        ui.add(
                            egui::TextEdit::singleline(link_url)
                                .desired_width(140.0)
                                .hint_text(theme::hint("https://…"))
                                .margin(egui::vec2(6.0, 3.0)),
                        );
                        let can = !link_url.trim().is_empty();
                        if ui
                            .add_enabled(
                                can,
                                egui::Button::new(RichText::new("设为链接").size(12.0))
                                    .min_size(Vec2::new(0.0, TOOL_H)),
                            )
                            .clicked()
                        {
                            let url = normalize_url(link_url);
                            apply_toolbar_style(
                                ui.ctx(),
                                doc,
                                id_salt,
                                StylePatch {
                                    link: Some(Some(url)),
                                    ..Default::default()
                                },
                            );
                            link_text.clear();
                            link_url.clear();
                            *show_link = false;
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("取消")
                                        .size(12.0)
                                        .color(theme::text_muted()),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE)
                                .min_size(Vec2::new(0.0, TOOL_H)),
                            )
                            .clicked()
                        {
                            link_text.clear();
                            link_url.clear();
                            *show_link = false;
                        }
                    });
                });
        },
    );
}

/// 选区迷你条（已停用：改用顶栏工具条编辑选区）。保留实现便于日后开关。
#[allow(dead_code)]
fn show_selection_toolbar(
    ui: &mut egui::Ui,
    spans: &mut Vec<Span>,
    id_salt: &str,
    sel: &mut Option<BodySelection>,
    hide_for_drag: bool,
) -> bool {
    let Some(state) = sel.as_mut() else {
        return false;
    };
    if state.text.trim().is_empty() || state.start >= state.end {
        *sel = None;
        return false;
    }

    // 仅正文拖选时隐藏；点迷你条时 primary_down 也必须继续画，否则会闪一下且点不到
    if hide_for_drag {
        return false;
    }

    let popup_id = Id::new(("doc_sel_popup", id_salt));
    let mut dismiss = false;
    let mut bar_interacted = false;
    // 迷你条浮于选区上方，避免盖住正编辑的字
    let pos = egui::pos2(state.anchor.x - 8.0, state.anchor.y - 32.0);

    let area = egui::Area::new(popup_id)
        .order(Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            Frame::none()
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::border_strong()))
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::symmetric(8.0, 6.0))
                .shadow(egui::epaint::Shadow {
                    offset: Vec2::new(0.0, 3.0),
                    blur: 12.0,
                    spread: 0.0,
                    color: Color32::from_black_alpha(24),
                })
                .show(ui, |ui| {
                    ui.set_max_width(300.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if ui
                            .small_button(RichText::new("B").strong())
                            .on_hover_text("加粗（再点取消）")
                            .clicked()
                        {
                            bar_interacted = true;
                            let on = !range_all_flag(spans, state.start, state.end, |s| s.bold);
                            apply_style_to_char_range(
                                spans,
                                state.start,
                                state.end,
                                StylePatch {
                                    bold: Some(on),
                                    ..Default::default()
                                },
                            );
                            dismiss = true;
                        }
                        if ui
                            .small_button(RichText::new("I").italics())
                            .on_hover_text("斜体（再点取消）")
                            .clicked()
                        {
                            bar_interacted = true;
                            let on = !range_all_flag(spans, state.start, state.end, |s| s.italic);
                            apply_style_to_char_range(
                                spans,
                                state.start,
                                state.end,
                                StylePatch {
                                    italic: Some(on),
                                    ..Default::default()
                                },
                            );
                            dismiss = true;
                        }
                        if ui
                            .small_button(RichText::new("U").underline())
                            .on_hover_text("下划线（再点取消）")
                            .clicked()
                        {
                            bar_interacted = true;
                            let on =
                                !range_all_flag(spans, state.start, state.end, |s| s.underline);
                            apply_style_to_char_range(
                                spans,
                                state.start,
                                state.end,
                                StylePatch {
                                    underline: Some(on),
                                    ..Default::default()
                                },
                            );
                            dismiss = true;
                        }
                        let size_on = state.size_mode;
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("字号")
                                        .size(12.0)
                                        .color(if size_on {
                                            theme::shell_accent()
                                        } else {
                                            theme::text()
                                        }),
                                )
                                .fill(if size_on {
                                    theme::shell_accent_soft()
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .stroke(Stroke::NONE),
                            )
                            .on_hover_text("字号")
                            .clicked()
                        {
                            bar_interacted = true;
                            state.size_mode = !state.size_mode;
                            if state.size_mode {
                                state.color_mode = false;
                                state.link_mode = false;
                            }
                        }
                        let color_on = state.color_mode;
                        let color_r = ui
                            .add(
                                egui::Button::new(
                                    RichText::new("颜色")
                                        .size(12.0)
                                        .color(if color_on {
                                            theme::shell_accent()
                                        } else {
                                            theme::text()
                                        }),
                                )
                                .fill(if color_on {
                                    theme::shell_accent_soft()
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .stroke(Stroke::NONE),
                            )
                            .on_hover_text("字体颜色");
                        if color_r.clicked() {
                            bar_interacted = true;
                            state.color_mode = !state.color_mode;
                            if state.color_mode {
                                state.link_mode = false;
                                state.size_mode = false;
                            }
                        }
                        let link_on = state.link_mode;
                        let link_r = ui
                            .add(
                                egui::Button::new(
                                    RichText::new("链接")
                                        .size(12.0)
                                        .color(if link_on {
                                            theme::shell_accent()
                                        } else {
                                            theme::text()
                                        }),
                                )
                                .fill(if link_on {
                                    theme::shell_accent_soft()
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .stroke(Stroke::NONE),
                            )
                            .on_hover_text("把选中文字做成行内超链接");
                        if link_r.clicked() {
                            bar_interacted = true;
                            state.link_mode = !state.link_mode;
                            if state.link_mode {
                                state.color_mode = false;
                                state.size_mode = false;
                                if state.link_url.is_empty() {
                                    let t = state.text.trim();
                                    if t.contains('.') || t.contains("://") {
                                        state.link_url = t.to_string();
                                    }
                                }
                            }
                        }
                        let copy_r = ui.small_button("复制").on_hover_text("复制选中文字");
                        if copy_r.clicked() {
                            bar_interacted = true;
                            ui.ctx().copy_text(state.text.clone());
                        }
                        let clear_r = ui
                            .small_button("清除")
                            .on_hover_text("去掉选区颜色与链接");
                        if clear_r.clicked() {
                            bar_interacted = true;
                            apply_style_to_char_range(
                                spans,
                                state.start,
                                state.end,
                                StylePatch {
                                    color: Some(None),
                                    link: Some(None),
                                    bold: Some(false),
                                    italic: Some(false),
                                    underline: Some(false),
                                    size: Some(None),
                                },
                            );
                            dismiss = true;
                        }
                        let close_r = ui.small_button("×").on_hover_text("关闭");
                        if close_r.clicked() {
                            bar_interacted = true;
                            dismiss = true;
                        }
                    });

                    if state.size_mode {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for &sz in FONT_SIZES {
                                if ui.small_button(format!("{sz}")).clicked() {
                                    bar_interacted = true;
                                    apply_style_to_char_range(
                                        spans,
                                        state.start,
                                        state.end,
                                        StylePatch {
                                            size: Some(Some(sz)),
                                            ..Default::default()
                                        },
                                    );
                                    dismiss = true;
                                }
                            }
                        });
                    }

                    if state.color_mode {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for (name, hex) in memo_doc::palette() {
                                let fill = if hex.is_empty() {
                                    theme::panel()
                                } else {
                                    parse_hex_color(hex).unwrap_or(theme::panel())
                                };
                                let btn = ui.add(
                                    egui::Button::new(RichText::new("●").color(
                                        if hex.is_empty() {
                                            theme::text_muted()
                                        } else {
                                            fill
                                        },
                                    ))
                                    .fill(theme::card())
                                    .stroke(Stroke::new(1.0, theme::border()))
                                    .min_size(Vec2::splat(22.0)),
                                );
                                if btn.on_hover_text(*name).clicked() {
                                    bar_interacted = true;
                                    let color = if hex.is_empty() {
                                        None
                                    } else {
                                        Some((*hex).to_string())
                                    };
                                    apply_style_to_char_range(
                                        spans,
                                        state.start,
                                        state.end,
                                        StylePatch {
                                            color: Some(color),
                                            ..Default::default()
                                        },
                                    );
                                    dismiss = true;
                                }
                            }
                        });
                    }

                    if state.link_mode {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("网址")
                                    .size(12.0)
                                    .color(theme::text_muted()),
                            );
                            let url_r = ui.add(
                                egui::TextEdit::singleline(&mut state.link_url)
                                    .id_source(("sel_link_url", id_salt))
                                    .desired_width(180.0)
                                    .hint_text(theme::hint("https://…")),
                            );
                            if url_r.has_focus() || url_r.changed() {
                                bar_interacted = true;
                            }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let can = !state.link_url.trim().is_empty();
                            if ui
                                .add_enabled(can, egui::Button::new("设为链接"))
                                .clicked()
                            {
                                bar_interacted = true;
                                let url = normalize_url(&state.link_url);
                                apply_style_to_char_range(
                                    spans,
                                    state.start,
                                    state.end,
                                    StylePatch {
                                        link: Some(Some(url)),
                                        ..Default::default()
                                    },
                                );
                                dismiss = true;
                            }
                            if ui.small_button("取消").clicked() {
                                bar_interacted = true;
                                state.link_mode = false;
                                state.link_url.clear();
                            }
                        });
                    }
                });
        });

    // 点了条上控件也算 hot，避免同帧被外侧点击逻辑清掉
    let popup_hot = bar_interacted
        || area.response.hovered()
        || area.response.contains_pointer();
    if dismiss {
        *sel = None;
    }
    popup_hot && !dismiss
}

/// 整篇单一 Flow：表/图为嵌入原子；表槽内始终实表可编。
/// `wrap_w`：正文折行宽（纸面可视宽），勿用横滚撑开后的 available_width。
fn edit_body_segments(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    data_dir: Option<&Path>,
    memo_id: &str,
    id_salt: &str,
    select_all: bool,
    wrap_w: f32,
) {
    ensure_body_paragraph(doc);
    if doc.blocks.iter().any(|b| matches!(b, Block::Items { .. })) {
        doc.blocks = memo_doc::normalize_blocks(std::mem::take(&mut doc.blocks));
    }

    let (mut state, mut store) = blocks_to_flow(&doc.blocks);
    let run_key = 0u64;

    // 恢复嵌入高宽（贴合实表，避免全纸面宽外框）
    if let Some(heights) = ui
        .ctx()
        .data(|d| d.get_temp::<HashMap<u64, f32>>(doc_embed_heights_id(id_salt)))
    {
        for e in &mut state.embeds {
            if let Some(&h) = heights.get(&e.id) {
                e.height = h;
            }
        }
    }
    if let Some(widths) = ui
        .ctx()
        .data(|d| d.get_temp::<HashMap<u64, f32>>(doc_embed_widths_id(id_salt)))
    {
        for e in &mut state.embeds {
            if let Some(&w) = widths.get(&e.id) {
                e.width = w;
            }
        }
    }

    // 优先复用会话光标（与正文绑定）；仅正文被外部改写时才退回 FlowCaretSnap
    let session = ui
        .ctx()
        .data(|d| d.get_temp::<FlowSession>(doc_flow_session_id(id_salt, run_key)));
    let session_hit = session
        .as_ref()
        .is_some_and(|s| s.text == state.text);
    if let Some(sess) = session.filter(|s| s.text == state.text) {
        state.cursor = sess.cursor.min(state.char_len());
        state.cursor = memo_text::clamp_cursor_to_list_body(&state.text, state.cursor);
        state.prefer_next_row = sess.prefer_next_row;
        state.selection = sess.selection.filter(|s| !s.is_empty());
        state.has_ime = sess.has_ime
            && state.cursor > sess.ime_anchor
            && state.selection.is_some_and(|sel| {
                let (a, b) = sel.sorted();
                a == sess.ime_anchor.min(state.char_len()) && b == state.cursor
            });
        state.ime_anchor = if state.has_ime {
            sess.ime_anchor.min(state.char_len())
        } else {
            state.cursor
        };
        caret_trace(format!(
            "restore session cursor={} ime={} has_ime={} len={}",
            state.cursor, state.ime_anchor, state.has_ime, state.char_len()
        ));
    } else if let Some(c) = read_flow_caret(ui.ctx(), id_salt, run_key) {
        restore_flow_cursor(&mut state, c);
        state.has_ime = false;
        state.ime_anchor = state.cursor;
        caret_trace(format!(
            "restore snap cursor={} prefer_next={} session_miss len={}",
            state.cursor, state.prefer_next_row, state.char_len()
        ));
    } else {
        state.cursor = state.char_len();
        state.prefer_next_row = true;
        state.ime_anchor = state.cursor;
        state.has_ime = false;
        caret_trace(format!("restore eof cursor={}", state.cursor));
    }
    let _ = session_hit;
    if select_all {
        let len = state.char_len();
        state.selection = Some(FlowSelection { start: 0, end: len });
        state.cursor = len;
        state.prefer_next_row = true;
        state.ime_anchor = len;
        state.has_ime = false;
    } else if !state.has_ime {
        // 非合成：选区仅在光标仍落在端点时保留（会话已带上则不动；否则从 snap 补）
        if state.selection.is_none() {
            if let Some(sel) = ui
                .ctx()
                .data(|d| d.get_temp::<FlowSelection>(doc_flow_sel_id(id_salt, run_key)))
            {
                if !sel.is_empty()
                    && (state.cursor == sel.start || state.cursor == sel.end)
                {
                    state.selection = Some(sel);
                } else {
                    ui.ctx().data_mut(|d| {
                        d.remove_temp::<FlowSelection>(doc_flow_sel_id(id_salt, run_key));
                    });
                }
            }
        }
    }

    let toolbar_hit = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(doc_toolbar_hit_id(id_salt)))
        .unwrap_or(false);
    let table_ui_lock = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(doc_table_ui_lock_id(id_salt)))
        .unwrap_or(false);

    // 消费顶栏挂起样式
    if let Some(patch) = ui
        .ctx()
        .data_mut(|d| d.get_temp::<StylePatch>(doc_pending_style_id(id_salt)))
    {
        let sel = ui
            .ctx()
            .data(|d| d.get_temp::<DocTextSelection>(doc_text_sel_id(id_salt)));
        let applied = if let Some(sel) = sel {
            if sel.start < sel.end {
                FlowEditor::apply_patch(
                    &mut state,
                    sel.start,
                    sel.end,
                    style_patch_to_delta(patch),
                );
                true
            } else {
                false
            }
        } else {
            false
        };
        if applied {
            ui.ctx().data_mut(|d| {
                d.remove_temp::<StylePatch>(doc_pending_style_id(id_salt));
            });
        }
    }

    let edit_w = wrap_w.max(1.0);
    let te_id = ui.make_persistent_id(("doc_flow_single", id_salt));
    // 表内获焦时勿让 Flow 抢走键盘
    let flow_toolbar = toolbar_hit || table_ui_lock;
    let out = FlowEditor::show(
        ui,
        te_id,
        &mut state,
        edit_w,
        3,
        "在此输入正文…",
        flow_toolbar,
    );

    // 回车/上下键：把光标矩形交给竖滚层（本层在横滚内，scroll_to_rect 的 Y 会被吃掉）
    if out.ensure_caret_visible {
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                doc_caret_scroll_id(id_salt),
                CaretScrollTarget(out.cursor_rect),
            );
        });
    } else {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<CaretScrollTarget>(doc_caret_scroll_id(id_salt));
        });
    }

    // Enter 已在 FlowEditor 内 handle_enter；此处不再二次调用

    // Ctrl+单击打开链接（与旧段落路径一致）
    if let Some(url) = out.hovered_link.as_ref() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        egui::show_tooltip_at_pointer(
            ui.ctx(),
            Id::new(("doc_flow_link_tip", id_salt)),
            |ui| {
                ui.label(format!("Ctrl+单击打开\n{url}"));
            },
        );
        if ui.input(|i| i.modifiers.command && i.pointer.primary_released()) {
            open_url(url);
        }
    }

    // 嵌入区：表始终实表；高宽由测量回写
    let mut heights: HashMap<u64, f32> = ui
        .ctx()
        .data(|d| d.get_temp(doc_embed_heights_id(id_salt)))
        .unwrap_or_default();
    let mut widths_map: HashMap<u64, f32> = ui
        .ctx()
        .data(|d| d.get_temp(doc_embed_widths_id(id_salt)))
        .unwrap_or_default();
    let mut delete_image_files: Vec<String> = Vec::new();
    let mut table_ui_lock_next = false;
    let mut delete_table_ids: Vec<u64> = Vec::new();

    for (eid, rect) in &out.embed_rects {
        let Some(payload) = store.get_mut(eid) else {
            continue;
        };
        match payload {
            EmbedPayload::Table { headers, rows } => {
                // 子 UI 按列外宽撑开，避免嵌在过窄 embed 槽里量不到真宽
                let est_w = table_outer_width(&sync_tbl_widths(
                    ui.ctx(),
                    id_salt,
                    *eid as usize,
                    headers.len().max(1),
                ));
                let child_rect = egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(rect.width().max(est_w), rect.height().max(24.0)),
                );
                let mut child =
                    ui.child_ui(child_rect, egui::Layout::top_down(egui::Align::Min));
                let cells_selected =
                    select_all || flow_embed_in_selection(&state, *eid);
                let (del, cell_focus, content_w) = edit_table_block(
                    &mut child,
                    headers,
                    rows,
                    id_salt,
                    *eid as usize,
                    cells_selected,
                );
                let used_h = child.min_rect().height().max(24.0);
                let content_w = content_w
                    .max(est_w)
                    .max(child.min_rect().width());
                heights.insert(*eid, used_h);
                widths_map.insert(*eid, content_w);
                state.set_embed_height(*eid, used_h);
                state.set_embed_width(*eid, content_w);
                if cell_focus {
                    table_ui_lock_next = true;
                }
                if del {
                    delete_table_ids.push(*eid);
                }
            }
            EmbedPayload::Image { file_id, alt } => {
                let mut child = ui.child_ui(*rect, egui::Layout::top_down(egui::Align::Min));
                if !alt.is_empty() {
                    child.label(RichText::new(alt.as_str()).small().color(theme::text_muted()));
                }
                show_image_preview(&mut child, data_dir, memo_id, file_id);
                if child
                    .add(
                        egui::Button::new(
                            RichText::new("× 删除图")
                                .size(11.0)
                                .color(theme::text_muted()),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE),
                    )
                    .clicked()
                {
                    delete_image_files.push(file_id.clone());
                    if let Some(emb) = state.embeds.iter().find(|e| e.id == *eid).cloned() {
                        let mut s = state.text.chars().collect::<Vec<_>>();
                        if emb.slot < s.len() && s[emb.slot] == EMBED_CHAR {
                            s.remove(emb.slot);
                            if emb.slot < s.len() && s[emb.slot] == '\n' {
                                s.remove(emb.slot);
                            }
                            state.text = s.into_iter().collect();
                            state.embeds.retain(|e| e.id != *eid);
                        }
                    }
                    store.remove(eid);
                }
                let used_h = child.min_rect().height().max(40.0);
                heights.insert(*eid, used_h);
                state.set_embed_height(*eid, used_h);
            }
        }
    }

    for eid in delete_table_ids {
        store.remove(&eid);
        if let Some(emb) = state.embeds.iter().find(|e| e.id == eid).cloned() {
            let mut s = state.text.chars().collect::<Vec<_>>();
            if emb.slot < s.len() && s[emb.slot] == EMBED_CHAR {
                s.remove(emb.slot);
                if emb.slot < s.len() && s[emb.slot] == '\n' {
                    s.remove(emb.slot);
                }
                state.text = s.into_iter().collect();
                state.embeds.retain(|e| e.id != eid);
                for e in &mut state.embeds {
                    if e.slot > emb.slot {
                        e.slot = e.slot.saturating_sub(1);
                    }
                }
            }
        }
    }

    for id in &out.deleted_embeds {
        if let Some(EmbedPayload::Image { file_id, .. }) = store.remove(id) {
            delete_image_files.push(file_id);
        } else {
            store.remove(id);
        }
    }

    // 点在表区也锁一帧，避免 Flow 抢焦点
    if out.clicked_embed.is_some_and(|eid| {
        matches!(store.get(&eid), Some(EmbedPayload::Table { .. }))
    }) {
        table_ui_lock_next = true;
    }

    // 含表选区复制：覆盖 Flow 剥掉 EMBED 的剪贴板（表内单元格获焦时不抢）
    let copy_hit = ui.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::Copy))
            || (i.modifiers.command && i.key_pressed(egui::Key::C))
    });
    if copy_hit && !table_ui_lock_next {
        if select_all {
            ui.ctx()
                .copy_text(flow_range_plain(&state, &store, 0, state.char_len()));
        } else if let Some(sel) = out.selection.filter(|s| !s.is_empty()) {
            let (a, b) = sel.sorted();
            ui.ctx()
                .copy_text(flow_range_plain(&state, &store, a, b));
        }
    }

    // 选区快照（流选，含跨嵌入）
    if let Some(sel) = out.selection.filter(|s| !s.is_empty()).or_else(|| {
        select_all.then(|| FlowSelection {
            start: 0,
            end: state.char_len(),
        })
    }) {
        let (a, b) = sel.sorted();
        let text = flow_range_plain(&state, &store, a, b);
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                doc_flow_sel_id(id_salt, run_key),
                FlowSelection { start: a, end: b },
            );
            d.insert_temp(
                doc_text_sel_id(id_salt),
                DocTextSelection {
                    run_start: 0,
                    para_idx: 0,
                    start: a,
                    end: b,
                    text,
                },
            );
        });
    } else if !toolbar_hit && !table_ui_lock_next {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<FlowSelection>(doc_flow_sel_id(id_salt, run_key));
            d.remove_temp::<DocTextSelection>(doc_text_sel_id(id_salt));
        });
    }

    // 帧末：会话（正文+光标）与 snap 一并保存；IME 仅真预编辑时持久化
    debug_assert_eq!(state.cursor, out.cursor);
    state.cursor = out.cursor;
    let ime_persist = state.has_ime
        && state.cursor > state.ime_anchor
        && state.selection.is_some_and(|s| {
            let (a, b) = s.sorted();
            a == state.ime_anchor && b == state.cursor
        });
    if !ime_persist {
        state.has_ime = false;
        state.ime_anchor = state.cursor;
    }
    caret_trace(format!(
        "save cursor={} enter={} changed={} text_len={} has_ime={}",
        state.cursor, out.enter_pressed, out.changed, state.char_len(), state.has_ime
    ));
    save_flow_cursor(ui.ctx(), id_salt, run_key, &state);
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            doc_flow_session_id(id_salt, run_key),
            FlowSession {
                text: state.text.clone(),
                cursor: state.cursor,
                prefer_next_row: state.prefer_next_row,
                selection: state.selection,
                has_ime: ime_persist,
                ime_anchor: state.ime_anchor,
            },
        );
        d.insert_temp(doc_table_ui_lock_id(id_salt), table_ui_lock_next);
        d.insert_temp(doc_embed_heights_id(id_salt), heights);
        d.insert_temp(doc_embed_widths_id(id_salt), widths_map);
    });

    // Esc：只清 Flow 选区（表无点进模式，不退出「编辑态」）
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.clear_selection();
        state.has_ime = false;
        set_doc_select_all(ui.ctx(), id_salt, false);
        ui.ctx().data_mut(|d| {
            d.remove_temp::<DocTextSelection>(doc_text_sel_id(id_salt));
            d.remove_temp::<FlowSelection>(doc_flow_sel_id(id_salt, run_key));
        });
    }

    doc.blocks = flow_to_blocks(&state, &store);
    ensure_body_paragraph(doc);

    for file_id in delete_image_files {
        if let Some(dir) = data_dir {
            let p = attachments::path_of(dir, memo_id, &file_id);
            let _ = std::fs::remove_file(p);
        }
    }
}

/// 多行正文：按 spans 着色（链接=蓝+下划线），折行避免撑破抽屉。
#[allow(dead_code)]
fn multiline_body_edit(
    ui: &mut egui::Ui,
    text: &mut String,
    spans: &[Span],
    te_id: egui::Id,
    width: f32,
    rows: usize,
    hint: &str,
) -> egui::text_edit::TextEditOutput {
    let w = width.max(1.0).min(fit_ui_width(ui).max(1.0));
    let spans = spans.to_vec();
    let mut out: Option<egui::text_edit::TextEditOutput> = None;
    ui.allocate_ui_with_layout(
        egui::vec2(w, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_max_width(w);
            ui.set_width(w);
            let mut layouter = |ui: &egui::Ui, s: &str, wrap_width: f32| {
                let job = spans_layout_job(ui, s, &spans, wrap_width);
                ui.fonts(|f| f.layout_job(job))
            };
            out = Some(
                egui::TextEdit::multiline(text)
                    .id(te_id)
                    .desired_width(w)
                    .desired_rows(rows)
                    .frame(false)
                    .hint_text(theme::hint(hint))
                    .layouter(&mut layouter)
                    .show(ui),
            );
        },
    );
    out.expect("multiline_body_edit allocated")
}

/// 段落编辑（旧 TextEdit 路径；正文已改走 FlowEditor，保留供对照）。
#[allow(dead_code)]
fn edit_paragraph_spans(
    ui: &mut egui::Ui,
    spans: &mut Vec<Span>,
    doc_id_salt: &str,
    para_idx: usize,
    body_rows: usize,
    select_all: bool,
) {
    if spans.is_empty() {
        spans.push(Span::plain(""));
    }

    let para_salt = format!("{doc_id_salt}_p{para_idx}");
    let sel_id = Id::new(("doc_body_sel", para_salt.as_str()));
    let te_id = ui.make_persistent_id(("doc_body", para_salt.as_str()));
    let mut sel = ui.ctx().data(|d| d.get_temp::<BodySelection>(sel_id));
    let edit_w = fit_ui_width(ui);
    let toolbar_hit = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(doc_toolbar_hit_id(doc_id_salt)))
        .unwrap_or(false);

    // 消费顶栏挂起的样式：优先打到本段选区
    if let Some(patch) = ui.ctx().data_mut(|d| {
        d.get_temp::<StylePatch>(doc_pending_style_id(doc_id_salt))
    }) {
        let sel = ui
            .ctx()
            .data(|d| d.get_temp::<DocTextSelection>(doc_text_sel_id(doc_id_salt)));
        let applied = if let Some(sel) = sel {
            if sel.para_idx == para_idx && sel.start < sel.end {
                apply_style_to_char_range(spans, sel.start, sel.end, patch);
                true
            } else {
                false
            }
        } else {
            false
        };
        if applied {
            ui.ctx().data_mut(|d| {
                d.remove_temp::<StylePatch>(doc_pending_style_id(doc_id_salt));
            });
        }
    }

    let mut plain = paragraph_full_text(spans);
    let output = multiline_body_edit(
        ui,
        &mut plain,
        spans,
        te_id,
        edit_w,
        body_rows,
        "在此输入正文…",
    );
    if select_all {
        paint_select_all_bg(ui, output.response.rect);
    }
    // 点顶栏时勿用 TextEdit 回写冲掉刚加的样式
    if output.response.changed() && !toolbar_hit {
        sync_spans_from_plain(spans, &plain);
    }

    if output.response.hovered() {
        if let Some(pointer) = output.response.hover_pos() {
            let local = pointer - output.galley_pos;
            let cursor = output.galley.cursor_from_pos(local);
            let ci = cursor.ccursor.index;
            let mut at = 0usize;
            for sp in spans.iter() {
                let n = sp.text.chars().count();
                if n > 0 && ci < at + n {
                    if let Some(url) = &sp.link {
                        egui::show_tooltip_at_pointer(
                            ui.ctx(),
                            Id::new(("doc_link_tip", para_salt.as_str())),
                            |ui| {
                                ui.label(format!("Ctrl+单击打开\n{url}"));
                            },
                        );
                        if ui.input(|i| i.modifiers.command && i.pointer.primary_released()) {
                            open_url(url);
                        }
                    }
                    break;
                }
                at += n;
            }
        }
    }

    let mut has_live_sel = false;
    if let Some(range) = output.cursor_range.as_ref() {
        if !range.is_empty() {
            let chars = range.as_sorted_char_range();
            let text = plain.char_range(chars.clone()).to_string();
            if !text.is_empty() {
                has_live_sel = true;
                let anchor = selection_anchor(&output, range);
                let (keep_color, keep_link, keep_size, keep_url) = sel
                    .as_ref()
                    .map(|s| (s.color_mode, s.link_mode, s.size_mode, s.link_url.clone()))
                    .unwrap_or((false, false, false, String::new()));
                sel = Some(BodySelection {
                    start: chars.start,
                    end: chars.end,
                    text: text.clone(),
                    anchor,
                    color_mode: keep_color,
                    link_mode: keep_link,
                    size_mode: keep_size,
                    link_url: keep_url,
                });
                // 供顶栏：选中后即可用栏上 B/I/U/字号/颜色 改选区
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        doc_text_sel_id(doc_id_salt),
                        DocTextSelection {
                            run_start: para_idx,
                            para_idx,
                            start: chars.start,
                            end: chars.end,
                            text,
                        },
                    );
                });
            }
        }
    }

    // 选区格式化改走顶栏工具条，不再弹迷你条

    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        sel = None;
        ui.ctx().data_mut(|d| {
            d.remove_temp::<DocTextSelection>(doc_text_sel_id(doc_id_salt));
        });
    } else if !toolbar_hit && ui.input(|i| i.pointer.any_click()) {
        let te_has_sel = output
            .cursor_range
            .as_ref()
            .is_some_and(|r| !r.is_empty());
        let click_in_editor = output.response.clicked() || output.response.contains_pointer();
        // 点顶栏工具时已用 toolbar_hit 跳过；点正文外才清选区
        if !(click_in_editor && te_has_sel) && !has_live_sel {
            let clear_doc_sel = ui.ctx().data(|d| {
                d.get_temp::<DocTextSelection>(doc_text_sel_id(doc_id_salt))
                    .map(|s| s.para_idx == para_idx)
                    .unwrap_or(false)
            });
            sel = None;
            if clear_doc_sel {
                ui.ctx().data_mut(|d| {
                    d.remove_temp::<DocTextSelection>(doc_text_sel_id(doc_id_salt));
                });
            }
        }
    }

    ui.ctx().data_mut(|d| {
        if let Some(s) = sel {
            d.insert_temp(sel_id, s);
        } else {
            d.remove_temp::<BodySelection>(sel_id);
        }
    });
}

/// 表格列默认宽（不按页宽均分，避免边框撑到右边界）。
const TABLE_CELL_W: f32 = 120.0;
const TABLE_COL_MIN_W: f32 = 48.0;
const TABLE_COL_RESIZE_HIT: f32 = 6.0;
/// 单元格 Frame：左右 inner_margin 6+6，描边约 1×2；内层还按 `w` 要宽 → 列外宽 ≈ w+12。
const TABLE_CELL_X_CHROME: f32 = 12.0;
/// 外表框描边。
const TABLE_FRAME_X_CHROME: f32 = 2.0;

/// 列内容宽 → 整表外宽（含单元格边距/描边），供嵌入占位与纸面横滚。
fn table_outer_width(col_widths: &[f32]) -> f32 {
    let n = col_widths.len().max(1) as f32;
    col_widths
        .iter()
        .copied()
        .sum::<f32>()
        .max(TABLE_COL_MIN_W)
        + TABLE_CELL_X_CHROME * n
        + TABLE_FRAME_X_CHROME
}

fn tbl_widths_id(id_salt: &str, block_idx: usize) -> Id {
    Id::new(("doc_tbl_widths", id_salt, block_idx))
}

fn tbl_col_rights_id(id_salt: &str, block_idx: usize) -> Id {
    Id::new(("doc_tbl_col_rights", id_salt, block_idx))
}

/// 与 headers 列数对齐的列宽（会话 temp）。
fn sync_tbl_widths(ctx: &egui::Context, id_salt: &str, block_idx: usize, ncols: usize) -> Vec<f32> {
    let id = tbl_widths_id(id_salt, block_idx);
    let mut widths = ctx
        .data(|d| d.get_temp::<Vec<f32>>(id))
        .unwrap_or_default();
    if widths.len() < ncols {
        widths.resize(ncols, TABLE_CELL_W);
    } else if widths.len() > ncols {
        widths.truncate(ncols);
    }
    for w in widths.iter_mut() {
        if !w.is_finite() || *w < TABLE_COL_MIN_W {
            *w = TABLE_CELL_W;
        }
    }
    ctx.data_mut(|d| d.insert_temp(id, widths.clone()));
    widths
}

fn store_tbl_widths(ctx: &egui::Context, id_salt: &str, block_idx: usize, widths: Vec<f32>) {
    ctx.data_mut(|d| d.insert_temp(tbl_widths_id(id_salt, block_idx), widths));
}

/// Word 式细线表：列宽可拖；单元格右键菜单插行/列、删行、删表。
/// 无表内横滚；溢出由纸面底栏统一横滚。返回 `(删除整表, 获焦, 表宽 need_w)`。
/// `cells_selected`：流选区覆盖本表（或整篇选中）时，按单元格铺选中底色。
fn edit_table_block(
    ui: &mut egui::Ui,
    headers: &mut Vec<String>,
    rows: &mut Vec<Vec<String>>,
    id_salt: &str,
    block_idx: usize,
    cells_selected: bool,
) -> (bool, bool, f32) {
    let mut delete_block = false;
    if headers.is_empty() {
        headers.push(String::new());
    }
    let ncols = headers.len();
    let col_gap = 0.0;
    let mut widths = sync_tbl_widths(ui.ctx(), id_salt, block_idx, ncols);
    let need_w = table_outer_width(&widths);

    let mut cell_focused = false;
    ui.set_min_width(need_w);
    let frame_out = Frame::none()
        .stroke(table_hairline())
        .rounding(Rounding::same(2.0))
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            ui.set_min_width(need_w);
            ui.set_width(need_w);
            cell_focused = paint_edit_table_grid(
                ui,
                headers,
                rows,
                &widths,
                col_gap,
                id_salt,
                block_idx,
                cells_selected,
            );
        });
    let block_r = frame_out.response.rect;

    // 列右缘拖拽调左侧列宽（相对表框）
    let col_rights: Vec<f32> = ui
        .ctx()
        .data(|d| d.get_temp::<Vec<f32>>(tbl_col_rights_id(id_salt, block_idx)))
        .unwrap_or_default();
    let resize_clip = block_r;
    let mut resizing = false;
    if col_rights.len() == widths.len() && resize_clip.height() > 4.0 {
        let hit_w = TABLE_COL_RESIZE_HIT;
        let top = resize_clip.top();
        let h = resize_clip.height();
        let mut changed = false;
        for (i, &right_x) in col_rights.iter().enumerate() {
            if right_x < resize_clip.left() - hit_w || right_x > resize_clip.right() + hit_w {
                continue;
            }
            let strip = egui::Rect::from_min_size(
                egui::pos2(right_x - hit_w * 0.5, top),
                egui::vec2(hit_w, h),
            )
            .intersect(resize_clip.expand(hit_w));
            if !strip.is_positive() {
                continue;
            }
            let resp = ui.interact(
                strip,
                Id::new(("doc_tbl_resize", id_salt, block_idx, i)),
                Sense::click_and_drag(),
            );
            if resp.hovered() || resp.dragged() {
                ui.ctx()
                    .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if resp.dragged() {
                let dx = ui.input(|inp| inp.pointer.delta().x);
                widths[i] = (widths[i] + dx).max(TABLE_COL_MIN_W);
                changed = true;
                resizing = true;
            }
        }
        if changed {
            store_tbl_widths(ui.ctx(), id_salt, block_idx, widths);
            ui.ctx().request_repaint();
        }
    }

    let ctx_open = ui
        .ctx()
        .data(|d| d.get_temp::<TblCtxMenu>(tbl_ctx_id(id_salt, block_idx)))
        .is_some();
    if let Some((action, row, col)) = show_table_ctx_menu(ui, id_salt, block_idx) {
        let mut w_before = sync_tbl_widths(ui.ctx(), id_salt, block_idx, headers.len());
        if apply_tbl_ctx_action(headers, rows, (row, col), action) {
            delete_block = true;
        } else {
            match action {
                TblCtxAction::ColLeft => {
                    w_before.insert(col.min(w_before.len()), TABLE_CELL_W);
                    store_tbl_widths(ui.ctx(), id_salt, block_idx, w_before);
                }
                TblCtxAction::ColRight => {
                    w_before.insert((col + 1).min(w_before.len()), TABLE_CELL_W);
                    store_tbl_widths(ui.ctx(), id_salt, block_idx, w_before);
                }
                _ => {
                    let _ = sync_tbl_widths(ui.ctx(), id_salt, block_idx, headers.len().max(1));
                }
            }
        }
    }

    let ui_active = cell_focused || ctx_open || resizing;
    let widths_now = sync_tbl_widths(ui.ctx(), id_salt, block_idx, headers.len().max(1));
    // 取「估算外宽」与实绘宽度较大者，避免边距/描边被裁掉后横滚仍顶不到表尾
    let need_w_now = table_outer_width(&widths_now).max(block_r.width());
    (delete_block, ui_active, need_w_now)
}

/// 绘制可编表格网格；返回是否有单元格获焦。
fn paint_edit_table_grid(
    ui: &mut egui::Ui,
    headers: &mut Vec<String>,
    rows: &mut Vec<Vec<String>>,
    widths: &[f32],
    col_gap: f32,
    id_salt: &str,
    block_idx: usize,
    cells_selected: bool,
) -> bool {
    let line = table_hairline();
    let ncols = headers.len().max(1);
    let nrows = 1 + rows.len(); // 表头 + 正文
    let min_col = widths
        .iter()
        .copied()
        .fold(TABLE_COL_MIN_W, f32::min)
        .max(1.0);
    let header_fill = if cells_selected {
        table_cell_selection_fill(table_header_fill())
    } else {
        table_header_fill()
    };
    let body_fill = if cells_selected {
        table_cell_selection_fill(theme::card())
    } else {
        theme::card()
    };
    let mut col_rights = Vec::with_capacity(ncols);
    let mut any_focused = false;
    egui::Grid::new(format!("{id_salt}_tbl_{block_idx}"))
        .num_columns(ncols)
        .striped(false)
        .spacing([col_gap, 0.0])
        .min_col_width(min_col)
        .show(ui, |ui| {
            for (ci, h) in headers.iter_mut().enumerate() {
                let w = widths.get(ci).copied().unwrap_or(TABLE_CELL_W);
                let fr = Frame::none()
                    .fill(header_fill)
                    .stroke(line)
                    .inner_margin(Margin::symmetric(6.0, 4.0))
                    .show(ui, |ui| {
                        ui.set_min_width((w - 2.0).max(1.0));
                        ui.set_max_width(w.max(1.0));
                        let (r, focused) = edit_table_cell(
                            ui, h, id_salt, block_idx, 0, ci, nrows, ncols, w,
                        );
                        if focused {
                            any_focused = true;
                        }
                        r
                    });
                col_rights.push(fr.response.rect.right());
                if fr.response.secondary_clicked() || fr.inner.secondary_clicked() {
                    open_table_ctx_menu(
                        ui,
                        id_salt,
                        block_idx,
                        0,
                        ci,
                        fr.response.rect.left_top(),
                    );
                }
            }
            ui.end_row();
            for (ri, row) in rows.iter_mut().enumerate() {
                if row.len() < ncols {
                    row.resize(ncols, String::new());
                }
                for c in 0..ncols {
                    let w = widths.get(c).copied().unwrap_or(TABLE_CELL_W);
                    let fr = Frame::none()
                        .fill(body_fill)
                        .stroke(line)
                        .inner_margin(Margin::symmetric(6.0, 4.0))
                        .show(ui, |ui| {
                            ui.set_min_width((w - 2.0).max(1.0));
                            ui.set_max_width(w.max(1.0));
                            let (r, focused) = edit_table_cell(
                                ui,
                                &mut row[c],
                                id_salt,
                                block_idx,
                                ri + 1,
                                c,
                                nrows,
                                ncols,
                                w,
                            );
                            if focused {
                                any_focused = true;
                            }
                            r
                        });
                    if fr.response.secondary_clicked() || fr.inner.secondary_clicked() {
                        open_table_ctx_menu(
                            ui,
                            id_salt,
                            block_idx,
                            ri + 1,
                            c,
                            fr.response.rect.left_top(),
                        );
                    }
                }
                ui.end_row();
            }
        });
    ui.ctx().data_mut(|d| {
        d.insert_temp(tbl_col_rights_id(id_salt, block_idx), col_rights);
    });
    any_focused
}

fn fit_ui_width(ui: &egui::Ui) -> f32 {
    crate::page_shell::fit_avail_width(ui, ui.available_width())
}

/// 编辑器。`min_h` 为整卡高度（顶栏固定 + 纸面限高竖滚），长正文在卡内滚、不撑开外层。
pub fn show_editor(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    id_salt: &str,
    data_dir: Option<&Path>,
    memo_id: &str,
    min_h: f32,
) {
    ensure_body_paragraph(doc);

    let link_text_id = egui::Id::new(("doc_link_text", id_salt));
    let link_url_id = egui::Id::new(("doc_link_url", id_salt));
    let show_id = egui::Id::new(("doc_show_link", id_salt));
    let mut link_text = ui
        .ctx()
        .data(|d| d.get_temp::<String>(link_text_id))
        .unwrap_or_default();
    let mut link_url = ui
        .ctx()
        .data(|d| d.get_temp::<String>(link_url_id))
        .unwrap_or_default();
    let mut show_link = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(show_id))
        .unwrap_or(false);

    let min_h = min_h.max(140.0);
    let avail = fit_ui_width(ui);
    // 抽屉滑入首帧极窄：只占位，避免 set_width 超过 available 直接 abort
    if avail < 24.0 {
        ui.allocate_exact_size(Vec2::new(avail, min_h.min(48.0)), Sense::hover());
        ui.ctx().data_mut(|d| {
            d.insert_temp(link_text_id, link_text);
            d.insert_temp(link_url_id, link_url);
            d.insert_temp(show_id, show_link);
        });
        return;
    }

    let editor_w = avail;
    let chrome_h =
        TOOLBAR_STRIP_H + if show_link { LINK_STRIP_H } else { 0.0 } + 1.0;
    let paper_view_h = (min_h - chrome_h).max(96.0);

    // 外框：顶栏固定；纸面限高并独立竖滚（宽表另有横滚）
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            let editor_w = fit_ui_width(ui).min(editor_w);
            ui.set_max_width(editor_w);
            ui.set_min_height(min_h);
            ui.set_max_height(min_h);
            ui.set_width(editor_w);
            ui.set_height(min_h);

            toolbar(
                ui,
                doc,
                data_dir,
                memo_id,
                &mut show_link,
                id_salt,
            );
            if show_link {
                toolbar_link_row(
                    ui,
                    doc,
                    &mut link_text,
                    &mut link_url,
                    &mut show_link,
                    id_salt,
                );
            }
            ui.ctx().data_mut(|d| {
                d.insert_temp(link_text_id, link_text);
                d.insert_temp(link_url_id, link_url);
                d.insert_temp(show_id, show_link);
            });

            let y = ui.cursor().min.y;
            let left = ui.max_rect().left();
            ui.painter().hline(
                left..=left + editor_w,
                y,
                Stroke::new(1.0, theme::border()),
            );
            ui.allocate_exact_size(Vec2::new(editor_w, 1.0), Sense::hover());

            let viewport_w = fit_ui_width(ui).max(1.0);
            // 上一帧表宽 → 撑开横滚内容；正文折行仍用 viewport_w
            let max_embed_w = ui
                .ctx()
                .data(|d| d.get_temp::<HashMap<u64, f32>>(doc_embed_widths_id(id_salt)))
                .map(|m| m.values().copied().fold(0.0f32, f32::max))
                .unwrap_or(0.0);
            // 表外宽 + 纸面左右内边距(10+10)；略加余量避免滚到头仍贴边裁切
            let paper_content_w = viewport_w.max(max_embed_w + 24.0);

            egui::ScrollArea::vertical()
                .id_source(("doc_paper_v", id_salt))
                .max_height(paper_view_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::ScrollArea::horizontal()
                        .id_source(("doc_paper_h", id_salt))
                        .max_width(viewport_w)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.set_min_width(paper_content_w);
                            Frame::none()
                                .fill(theme::card())
                                .inner_margin(Margin::symmetric(10.0, 8.0))
                                .show(ui, |ui| {
                                    // 折行宽 = 可视宽减左右内边距
                                    let wrap_w = (viewport_w - 20.0).max(1.0);
                                    ui.spacing_mut().item_spacing.y = 6.0;

                                    let paper_focused = ui.ui_contains_pointer()
                                        || ui.memory(|m| m.focused().is_some());
                                    if paper_focused
                                        && ui.input(|i| {
                                            i.modifiers.command && i.key_pressed(egui::Key::A)
                                        })
                                    {
                                        set_doc_select_all(ui.ctx(), id_salt, true);
                                        ui.input_mut(|i| {
                                            i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)
                                        });
                                    }
                                    let select_all = doc_select_all_active(ui.ctx(), id_salt);
                                    if select_all
                                        && ui.input(|i| {
                                            i.modifiers.command && i.key_pressed(egui::Key::C)
                                        })
                                    {
                                        ui.ctx().copy_text(doc_plain_export(doc));
                                        ui.input_mut(|i| {
                                            i.consume_key(egui::Modifiers::COMMAND, egui::Key::C)
                                        });
                                    }
                                    if select_all && ui.input(|i| i.key_pressed(egui::Key::Escape))
                                    {
                                        set_doc_select_all(ui.ctx(), id_salt, false);
                                    }
                                    if select_all
                                        && ui.input(|i| i.pointer.primary_pressed())
                                        && ui.memory(|m| m.focused().is_some())
                                    {
                                        set_doc_select_all(ui.ctx(), id_salt, false);
                                    }

                                    if doc.blocks.iter().any(|b| matches!(b, Block::Items { .. }))
                                    {
                                        doc.blocks = memo_doc::normalize_blocks(std::mem::take(
                                            &mut doc.blocks,
                                        ));
                                    }

                                    let select_all = doc_select_all_active(ui.ctx(), id_salt);
                                    edit_body_segments(
                                        ui,
                                        doc,
                                        data_dir,
                                        memo_id,
                                        id_salt,
                                        select_all,
                                        wrap_w,
                                    );

                                    ui.ctx().data_mut(|d| {
                                        d.remove_temp::<bool>(doc_toolbar_hit_id(id_salt));
                                        d.remove_temp::<StylePatch>(doc_pending_style_id(id_salt));
                                    });
                                    let blank = ui.allocate_response(
                                        Vec2::new(wrap_w.max(1.0), 48.0),
                                        Sense::click(),
                                    );
                                    if select_all && blank.clicked() {
                                        set_doc_select_all(ui.ctx(), id_salt, false);
                                    }
                                });

                            // 横滚层内：滚入目标的 X（Y 会被本层吞掉，留给竖滚层）
                            if let Some(CaretScrollTarget(target)) = ui.ctx().data(|d| {
                                d.get_temp::<CaretScrollTarget>(doc_caret_scroll_id(id_salt))
                            }) {
                                let clip = ui.clip_rect();
                                let x_ok = target.left() >= clip.left()
                                    && target.right() <= clip.right();
                                if !x_ok {
                                    let align = if target.left() < clip.left() {
                                        egui::Align::Min
                                    } else {
                                        egui::Align::Max
                                    };
                                    ui.scroll_to_rect(target, Some(align));
                                }
                            }
                        });

                    // 横滚已结束：在竖滚层根据回车/上下键/跳格请求滚入（鼠标滚轮不强制拽回）
                    let wheel = ui.input(|i| i.smooth_scroll_delta);
                    if wheel.x.abs() > 0.1 || wheel.y.abs() > 0.1 {
                        ui.ctx().data_mut(|d| {
                            d.remove_temp::<CaretScrollTarget>(doc_caret_scroll_id(id_salt));
                        });
                    } else if let Some(CaretScrollTarget(target)) = ui.ctx().data_mut(|d| {
                        d.remove_temp::<CaretScrollTarget>(doc_caret_scroll_id(id_salt))
                    }) {
                        let clip = ui.clip_rect();
                        let fully_visible =
                            target.top() >= clip.top() && target.bottom() <= clip.bottom();
                        if !fully_visible {
                            // 禁止 Center，避免按住回车整页闪
                            let align = if target.top() < clip.top() {
                                egui::Align::Min
                            } else {
                                egui::Align::Max
                            };
                            ui.scroll_to_rect(target, Some(align));
                        }
                    }
                });
        });
}

fn texture_id_for(memo_id: &str, file_id: &str) -> String {
    format!("att:{memo_id}:{file_id}")
}

fn show_image_preview(ui: &mut egui::Ui, data_dir: Option<&Path>, memo_id: &str, file_id: &str) {
    let Some(dir) = data_dir else {
        ui.label(RichText::new("(无数据目录)").small().color(theme::text_muted()));
        return;
    };
    if memo_id.is_empty() || file_id.is_empty() {
        return;
    }
    let path = attachments::path_of(dir, memo_id, file_id);
    if !path.is_file() {
        ui.label(
            RichText::new(format!("缺失文件 {file_id}"))
                .small()
                .color(theme::warn_overdue_fg()),
        );
        return;
    }
    let tid = texture_id_for(memo_id, file_id);
    let tex = ui.ctx().load_texture(
        tid,
        load_color_image(&path).unwrap_or_else(|| {
            egui::ColorImage::new([2, 2], Color32::from_gray(200))
        }),
        Default::default(),
    );
    let max_w = ui.available_width().min(320.0);
    let size = tex.size_vec2();
    let scale = (max_w / size.x.max(1.0)).min(1.0);
    let disp = size * scale;
    ui.image((tex.id(), disp));
}

fn load_color_image(path: &Path) -> Option<egui::ColorImage> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.into_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let pixels = img.into_raw();
    Some(egui::ColorImage::from_rgba_unmultiplied(size, &pixels))
}

/// 只读正文（标题由详情顶栏展示；无附件目录时不显示插图）。
#[allow(dead_code)]
pub fn show_viewer_body(ui: &mut egui::Ui, content: &str) {
    show_viewer_body_ex(ui, content, None, "");
}

pub fn show_viewer_body_ex(
    ui: &mut egui::Ui,
    content: &str,
    data_dir: Option<&Path>,
    memo_id: &str,
) {
    let body = memo_doc::parse_body(content);
    if body.blocks.is_empty()
        || (body.blocks.len() == 1
            && matches!(
                &body.blocks[0],
                Block::Paragraph { spans }
                    if spans.iter().all(|s| s.text.trim().is_empty())
            ))
    {
        ui.label(theme::muted_label("（正文为空）").size(15.0));
        return;
    }
    render_blocks_readonly(ui, &body.blocks, data_dir, memo_id);
}

fn render_blocks_readonly(
    ui: &mut egui::Ui,
    blocks: &[Block],
    data_dir: Option<&Path>,
    memo_id: &str,
) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let mut table_seq = 0u32;
    for block in blocks {
        match block {
            Block::Paragraph { spans } => {
                if spans.iter().all(|s| s.text.trim().is_empty()) {
                    continue;
                }
                // 横向布局默认不换行；限定宽度并强制 wrap，避免长文/中文溢出
                let max_w = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(max_w, 0.0),
                    egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
                    |ui| {
                        ui.set_max_width(max_w);
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for sp in spans {
                            if sp.text.is_empty() {
                                continue;
                            }
                            let size = sp.size.map(|s| s as f32).unwrap_or(15.5);
                            if let Some(url) = &sp.link {
                                let mut rt = RichText::new(&sp.text)
                                    .size(size)
                                    .color(link_style_color())
                                    .underline();
                                if sp.bold {
                                    rt = rt.strong();
                                }
                                if sp.italic {
                                    rt = rt.italics();
                                }
                                let r = ui
                                    .add(
                                        egui::Label::new(rt)
                                            .wrap(true)
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text(url);
                                if r.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                if r.clicked() {
                                    open_url(url);
                                }
                            } else {
                                let color = sp
                                    .color
                                    .as_deref()
                                    .and_then(parse_hex_color)
                                    .unwrap_or_else(theme::text);
                                let mut rt = RichText::new(&sp.text).size(size).color(color);
                                if sp.bold {
                                    rt = rt.strong();
                                }
                                if sp.italic {
                                    rt = rt.italics();
                                }
                                if sp.underline {
                                    rt = rt.underline();
                                }
                                ui.add(egui::Label::new(rt).wrap(true));
                            }
                        }
                    },
                );
                ui.add_space(4.0);
            }
            Block::Items { .. } => {
                // 读入时已迁成段落
            }
            Block::Table { headers, rows } => {
                table_seq += 1;
                let ncols = headers.len().max(1);
                let cell = TABLE_CELL_W;
                let need_w = cell * ncols as f32;
                let page_w = ui.available_width();
                let table_w = need_w.min(page_w);
                let line = table_hairline();
                ui.allocate_ui_with_layout(
                    egui::vec2(table_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_max_width(table_w);
                        Frame::none()
                            .stroke(line)
                            .rounding(Rounding::same(2.0))
                            .inner_margin(Margin::ZERO)
                            .show(ui, |ui| {
                                egui::Grid::new(format!("view_tbl_{table_seq}"))
                                    .num_columns(ncols)
                                    .striped(false)
                                    .spacing([0.0, 0.0])
                                    .min_col_width(cell)
                                    .show(ui, |ui| {
                                        for h in headers {
                                            Frame::none()
                                                .fill(table_header_fill())
                                                .stroke(line)
                                                .inner_margin(Margin::symmetric(6.0, 4.0))
                                                .show(ui, |ui| {
                                                    ui.set_min_width(cell - 4.0);
                                                    ui.label(
                                                        RichText::new(h)
                                                            .strong()
                                                            .size(13.0)
                                                            .color(theme::text()),
                                                    );
                                                });
                                        }
                                        ui.end_row();
                                        for row in rows {
                                            for c in 0..ncols {
                                                let cell_s =
                                                    row.get(c).map(|s| s.as_str()).unwrap_or("");
                                                Frame::none()
                                                    .fill(theme::card())
                                                    .stroke(line)
                                                    .inner_margin(Margin::symmetric(6.0, 4.0))
                                                    .show(ui, |ui| {
                                                        ui.set_min_width(cell - 4.0);
                                                        ui.label(
                                                            RichText::new(cell_s)
                                                                .size(13.0)
                                                                .color(theme::text()),
                                                        );
                                                    });
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    },
                );
                ui.add_space(6.0);
            }
            Block::Image { file_id, alt } => {
                if !alt.is_empty() {
                    ui.label(
                        RichText::new(alt)
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                }
                show_image_preview(ui, data_dir, memo_id, file_id);
                ui.add_space(4.0);
            }
        }
    }
}

/// 备忘新建模板（第一行标题）。
#[allow(dead_code)]
pub fn memo_template(stamp: &str) -> String {
    memo_template_for(MemoCategory::Work, stamp)
}

pub fn memo_template_for(category: MemoCategory, stamp: &str) -> String {
    match category {
        MemoCategory::Todo => format!("待办 {stamp}"),
        MemoCategory::Credentials => format!("备忘 {stamp}"),
        MemoCategory::Work | MemoCategory::Office => format!("工作学习 {stamp}"),
        MemoCategory::Life => format!("生活家庭 {stamp}"),
        MemoCategory::Finance => format!("财务订阅 {stamp}"),
        MemoCategory::GenderPrivate | MemoCategory::WomenPrivate | MemoCategory::MalePrivate => {
            format!("性别私密备注 {stamp}")
        }
        MemoCategory::Emergency => format!("应急 {stamp}"),
        MemoCategory::Inspiration => format!("灵感 {stamp}"),
        MemoCategory::General => format!("备忘 {stamp}"),
    }
}

#[allow(dead_code)]
pub fn task_plan_template(title: &str) -> String {
    let t = if title.trim().is_empty() {
        "未命名任务"
    } else {
        title.trim()
    };
    format!("{t}\n\n（工作说明）\n\n1. \n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use memo_core::store::MemoCategory;

    #[test]
    fn memo_template_is_plain_paragraph() {
        let note = memo_template("2026-09-30");
        let (title, body) = split_note(&note, "未命名备忘");
        let doc = Doc::from_store(&title, &body);
        let n_para = doc
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Paragraph { .. }))
            .count();
        assert_eq!(n_para, 1, "应只有一段正文");
        assert_eq!(doc.title, "工作学习 2026-09-30");
    }

    #[test]
    fn each_category_template_parses() {
        for cat in MemoCategory::ALL {
            let note = memo_template_for(*cat, "2026-10-02");
            let (title, body) = split_note(&note, "未命名");
            assert!(!title.is_empty(), "{cat:?} 标题为空");
            let doc = Doc::from_store(&title, &body);
            assert!(!doc.title.is_empty());
        }
    }

    #[test]
    fn rich_roundtrip_via_store() {
        let mut doc = Doc::empty();
        doc.title = "T".into();
        if let Some(Block::Paragraph { spans }) = doc.blocks.first_mut() {
            *spans = vec![
                Span {
                    text: "见".into(),
                    color: Some("#C62828".into()),
                    link: None,
                    bold: true,
                    italic: false,
                    underline: false,
                    size: Some(14),
                },
                Span {
                    text: "官网".into(),
                    color: None,
                    link: Some("https://example.com".into()),
                    bold: false,
                    italic: false,
                    underline: false,
                    size: None,
                },
                Span::plain("了解更多"),
            ];
        }
        doc.blocks.push(Block::Image {
            file_id: "x.png".into(),
            alt: "a".into(),
        });
        let (title, body) = doc.to_store("未命名");
        assert_eq!(title, "T");
        let again = Doc::from_store(&title, &body);
        assert!(matches!(
            &again.blocks[0],
            Block::Paragraph { spans }
                if spans.len() == 3
                    && spans[0].color.as_deref() == Some("#C62828")
                    && spans[1].link.as_deref() == Some("https://example.com")
                    && spans[1].text == "官网"
        ));
        assert!(matches!(&again.blocks[1], Block::Image { .. }));
    }

    #[test]
    fn insert_inline_link_appends_chip() {
        let mut spans = vec![Span::plain("前缀")];
        insert_inline_link(&mut spans, "文档", "example.com");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].text, "文档");
        assert_eq!(spans[1].link.as_deref(), Some("https://example.com"));
        assert!(spans[2].text.is_empty() && spans[2].link.is_none());
    }

    #[test]
    fn apply_color_to_middle_chars() {
        let mut spans = vec![Span::plain("你好世界")];
        apply_style_to_char_range(
            &mut spans,
            2,
            4,
            StylePatch {
                color: Some(Some("#C62828".into())),
                ..Default::default()
            },
        );
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].text, "你好");
        assert!(spans[0].color.is_none());
        assert_eq!(spans[1].text, "世界");
        assert_eq!(spans[1].color.as_deref(), Some("#C62828"));
    }

    #[test]
    fn apply_link_to_selection() {
        let mut spans = vec![Span::plain("访问官网了解")];
        apply_style_to_char_range(
            &mut spans,
            2,
            4,
            StylePatch {
                link: Some(Some("https://example.com".into())),
                ..Default::default()
            },
        );
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].text, "访问");
        assert_eq!(spans[1].text, "官网");
        assert_eq!(spans[1].link.as_deref(), Some("https://example.com"));
        assert_eq!(spans[2].text, "了解");
    }

    #[test]
    fn apply_bold_and_list_prefix_paragraph_roundtrip() {
        let mut spans = vec![Span::plain("粗体字")];
        apply_style_to_char_range(
            &mut spans,
            0,
            3,
            StylePatch {
                bold: Some(true),
                size: Some(Some(16)),
                ..Default::default()
            },
        );
        assert!(spans[0].bold);
        assert_eq!(spans[0].size, Some(16));

        let mut doc = Doc::empty();
        doc.blocks.push(Block::Paragraph {
            spans: vec![Span::plain("- a")],
        });
        let (title, body) = doc.to_store("未命名");
        let again = Doc::from_store(&title, &body);
        assert!(matches!(&again.blocks[1], Block::Paragraph { .. }));
        if let Block::Paragraph { spans } = &again.blocks[1] {
            assert_eq!(spans[0].text, "- a");
        }
        assert!(!body.contains("\"items\""));
    }

    #[test]
    fn biu_toggle_off_when_already_on() {
        let mut spans = vec![Span {
            text: "粗斜下".into(),
            color: None,
            link: None,
            bold: true,
            italic: true,
            underline: true,
            size: None,
        }];
        assert!(range_all_flag(&spans, 0, 3, |s| s.bold));
        assert!(range_all_flag(&spans, 0, 3, |s| s.italic));
        assert!(range_all_flag(&spans, 0, 3, |s| s.underline));

        apply_style_to_char_range(
            &mut spans,
            0,
            3,
            StylePatch {
                bold: Some(false),
                ..Default::default()
            },
        );
        assert!(!spans[0].bold);
        assert!(spans[0].italic);
        assert!(spans[0].underline);

        apply_style_to_char_range(
            &mut spans,
            0,
            3,
            StylePatch {
                italic: Some(false),
                underline: Some(false),
                ..Default::default()
            },
        );
        assert!(!spans[0].italic);
        assert!(!spans[0].underline);

        // 混合选区：未全粗 → 应视为需要打开
        let mixed = vec![
            Span {
                text: "粗".into(),
                color: None,
                link: None,
                bold: true,
                italic: false,
                underline: false,
                size: None,
            },
            Span::plain("普"),
        ];
        assert!(!range_all_flag(&mixed, 0, 2, |s| s.bold));
    }

    #[test]
    fn list_prefix_toggle_strip_and_switch() {
        let (st, _, body, _) = parse_list_prefix("1. hello").unwrap();
        assert_eq!(st, ListStyle::Decimal);
        assert_eq!(body, "hello");
        let (st, _, body, _) = parse_list_prefix("- a").unwrap();
        assert_eq!(st, ListStyle::Dash);
        assert_eq!(body, "a");
        let (st, _, body, _) = parse_list_prefix("• b").unwrap();
        assert_eq!(st, ListStyle::Disc);
        assert_eq!(body, "b");

        let mut spans = vec![Span::plain("hello")];
        apply_list_mark_to_spans(&mut spans, Some(ListStyle::Decimal), 1);
        assert_eq!(paragraph_full_text(&spans), "1. hello");
        apply_list_mark_to_spans(&mut spans, Some(ListStyle::Dash), 1);
        assert_eq!(paragraph_full_text(&spans), "- hello");
        apply_list_mark_to_spans(&mut spans, None, 1);
        assert_eq!(paragraph_full_text(&spans), "hello");
    }

    #[test]
    fn flow_roundtrip_preserves_styles_and_anchors() {
        let blocks = vec![
            Block::Paragraph {
                spans: vec![Span {
                    text: "你好".into(),
                    color: None,
                    link: None,
                    bold: true,
                    italic: false,
                    underline: false,
                    size: Some(16),
                }],
            },
            Block::Paragraph {
                spans: vec![Span::plain("1. 项")],
            },
            Block::Table {
                headers: vec!["A".into()],
                rows: vec![vec!["1".into()]],
            },
            Block::Paragraph {
                spans: vec![Span::plain("后")],
            },
        ];
        let (flow, store) = blocks_to_flow(&blocks);
        assert!(flow.text.contains(EMBED_CHAR));
        assert_eq!(flow.embeds.len(), 1);
        assert!(flow.style_runs.iter().any(|r| r.bold));
        let back = flow_to_blocks(&flow, &store);
        assert!(matches!(&back[0], Block::Paragraph { spans } if spans[0].bold));
        assert!(matches!(&back[1], Block::Paragraph { .. }));
        assert!(matches!(&back[2], Block::Table { headers, .. } if headers[0] == "A"));
        assert!(matches!(&back[3], Block::Paragraph { spans } if spans[0].text == "后"));
    }

    #[test]
    fn flow_roundtrip_keeps_trailing_newlines() {
        let flow = FlowState::from_plain("hello\n");
        let back = flow_to_blocks(&flow, &HashMap::new());
        assert_eq!(back.len(), 2);
        assert!(matches!(&back[0], Block::Paragraph { spans } if spans[0].text == "hello"));
        assert!(matches!(&back[1], Block::Paragraph { spans } if spans[0].text.is_empty()));
        let (again, _) = blocks_to_flow(&back);
        assert_eq!(again.text, "hello\n");

        let flow2 = FlowState::from_plain("a\n\n");
        let back2 = flow_to_blocks(&flow2, &HashMap::new());
        let (again2, _) = blocks_to_flow(&back2);
        assert_eq!(again2.text, "a\n\n");
    }

    #[test]
    fn list_enter_plain_survives_block_roundtrip() {
        // 列表行回车只插 \\n，不续编号；往返后文本与光标仍合法
        let mut flow = FlowState::from_plain("1. hello");
        flow.cursor = flow.text.chars().count();
        memo_text::handle_enter(&mut flow);
        assert_eq!(flow.text, "1. hello\n");
        let at = flow.cursor;
        assert_eq!(at, flow.text.chars().count());
        let blocks = flow_to_blocks(&flow, &HashMap::new());
        let (again, _) = blocks_to_flow(&blocks);
        assert_eq!(again.text, "1. hello\n");
        assert!(at <= again.text.chars().count());
        assert_eq!(again.text.chars().count(), at);
    }

    #[test]
    fn enter_amid_list_plain_survives_roundtrip() {
        let mut lines: Vec<String> = (1..=3).map(|i| format!("{i}. x")).collect();
        lines.push("4. 234".into());
        lines.push("5. 234".into());
        for i in 6..=11 {
            lines.push(format!("{i}. "));
        }
        let mut flow = FlowState::from_plain(lines.join("\n"));
        let line5 = flow.text.find("5. 234").unwrap();
        let line5 = flow.text[..line5].chars().count();
        flow.cursor = line5 + "5. 234".chars().count();
        memo_text::handle_enter(&mut flow);
        let at = flow.cursor;
        // 普通插入：光标落在空行（原 5. 与 6. 之间）
        let (ls, le) = memo_text::line_at_cursor(&flow.text, at);
        let cur: String = flow.text.chars().skip(ls).take(le - ls).collect();
        assert_eq!(cur, "");

        let blocks = flow_to_blocks(&flow, &HashMap::new());
        let (again, _) = blocks_to_flow(&blocks);
        assert_eq!(again.text, flow.text, "往返不得改文本");
        let (ls2, le2) = memo_text::line_at_cursor(&again.text, at.min(again.text.chars().count()));
        let cur2: String = again.text.chars().skip(ls2).take(le2 - ls2).collect();
        assert_eq!(cur2, "");
    }

    #[test]
    fn consecutive_plain_enters_survive_host_roundtrip() {
        let mut flow = FlowState::from_plain("1. hello");
        flow.cursor = flow.text.chars().count();
        let mut prev = flow.cursor;
        for n in 1..=4 {
            memo_text::handle_enter(&mut flow);
            assert!(
                flow.cursor > prev,
                "第 {n} 次回车后光标应前进，prev={} now={}",
                prev,
                flow.cursor
            );
            let at = flow.cursor;
            let blocks = flow_to_blocks(&flow, &HashMap::new());
            let (mut again, _) = blocks_to_flow(&blocks);
            assert_eq!(again.text, flow.text, "往返不得改文本 n={n}");
            // 模拟宿主只按绝对下标恢复（勿用行号+列钉回旧行）
            restore_flow_cursor(
                &mut again,
                FlowCaretSnap {
                    index: at,
                    prefer_next_row: true,
                },
            );
            assert_eq!(again.cursor, at.min(again.char_len()));
            let before: String = again.text.chars().take(again.cursor).collect();
            assert!(
                before.ends_with('\n'),
                "\\n 须在光标前：before={before:?} cursor={}",
                again.cursor
            );
            flow = again;
            prev = flow.cursor;
        }
        assert_eq!(flow.text, "1. hello\n\n\n\n");
    }

    #[test]
    fn restore_abs_cursor_not_stuck_on_newline_after_enters() {
        // 回归：若误用旧「行号+列」会钉在首行行末，连续回车光标不动
        let mut flow = FlowState::from_plain("hello");
        flow.cursor = 5;
        for _ in 0..3 {
            memo_text::handle_enter(&mut flow);
        }
        assert_eq!(flow.text, "hello\n\n\n");
        assert_eq!(flow.cursor, 8);
        let blocks = flow_to_blocks(&flow, &HashMap::new());
        let (mut again, _) = blocks_to_flow(&blocks);
        restore_flow_cursor(
            &mut again,
            FlowCaretSnap {
                index: flow.cursor,
                prefer_next_row: true,
            },
        );
        assert_eq!(again.cursor, 8);
        assert_ne!(again.text.chars().nth(again.cursor), Some('\n'));
    }

    /// 回归：在「房间|按到」间连续回车，光标须始终紧贴「按到」前（与 Word 一致）。
    #[test]
    fn mid_han_consecutive_enters_cursor_stays_before_suffix() {
        let mut flow = FlowState::from_plain("（在此填写情况说明）房间按到");
        let at = flow
            .text
            .char_indices()
            .find(|(_, c)| *c == '按')
            .map(|(i, _)| flow.text[..i].chars().count())
            .expect("按");
        flow.cursor = at;
        for n in 1..=5 {
            memo_text::handle_enter(&mut flow);
            let after: String = flow.text.chars().skip(flow.cursor).collect();
            assert!(
                after.starts_with("按到"),
                "第 {n} 次回车后光标应在「按到」前，after={after:?} cursor={} text={:?}",
                flow.cursor,
                flow.text
            );
            let blocks = flow_to_blocks(&flow, &HashMap::new());
            let (mut again, _) = blocks_to_flow(&blocks);
            assert_eq!(again.text, flow.text, "往返不得改文本 n={n}");
            restore_flow_cursor(
                &mut again,
                FlowCaretSnap {
                    index: flow.cursor,
                    prefer_next_row: true,
                },
            );
            let after2: String = again.text.chars().skip(again.cursor).collect();
            assert!(
                after2.starts_with("按到"),
                "往返恢复后仍应在「按到」前 n={n} after={after2:?}"
            );
            flow = again;
        }
        assert!(flow.text.contains("房间\n\n\n\n\n按到") || flow.text.matches('\n').count() >= 5);
    }

    #[test]
    fn table_nav_boundary_and_tab_wrap() {
        // 3 列 × 2 行（含表头）
        let nrows = 2;
        let ncols = 3;
        // 格内未到边界：不跳
        assert!(table_nav_target(
            TableNavDir::Left,
            0,
            1,
            nrows,
            ncols,
            false,
            false,
            true,
            true
        )
        .is_none());
        assert!(table_nav_target(
            TableNavDir::Right,
            0,
            1,
            nrows,
            ncols,
            false,
            false,
            true,
            true
        )
        .is_none());
        // 边界跳格
        assert_eq!(
            table_nav_target(TableNavDir::Left, 0, 1, nrows, ncols, true, false, true, true),
            Some((0, 0, true))
        );
        assert_eq!(
            table_nav_target(TableNavDir::Right, 0, 1, nrows, ncols, false, true, true, true),
            Some((0, 2, false))
        );
        assert_eq!(
            table_nav_target(TableNavDir::Up, 1, 1, nrows, ncols, false, false, true, false),
            Some((0, 1, false))
        );
        assert_eq!(
            table_nav_target(TableNavDir::Down, 0, 1, nrows, ncols, false, false, false, true),
            Some((1, 1, false))
        );
        // 首末不动
        assert!(table_nav_target(
            TableNavDir::Left,
            0,
            0,
            nrows,
            ncols,
            true,
            false,
            true,
            true
        )
        .is_none());
        assert!(table_nav_target(
            TableNavDir::Up,
            0,
            1,
            nrows,
            ncols,
            false,
            false,
            true,
            true
        )
        .is_none());
        // Tab 行优先 + 绕回
        assert_eq!(
            table_nav_target(TableNavDir::Tab, 0, 2, nrows, ncols, false, false, true, true),
            Some((1, 0, false))
        );
        assert_eq!(
            table_nav_target(TableNavDir::Tab, 1, 2, nrows, ncols, false, false, true, true),
            Some((0, 0, false))
        );
        assert_eq!(
            table_nav_target(
                TableNavDir::ShiftTab,
                0,
                0,
                nrows,
                ncols,
                false,
                false,
                true,
                true
            ),
            Some((1, 2, true))
        );
        let (at_s, at_e, at_f, at_l) = table_cursor_at_bounds("a\nb", 0);
        assert!(at_s && !at_e && at_f && !at_l);
        let (at_s, at_e, at_f, at_l) = table_cursor_at_bounds("a\nb", 3);
        assert!(!at_s && at_e && !at_f && at_l);
    }
}
