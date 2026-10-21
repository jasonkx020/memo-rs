//! 公文风所见即所得文档编辑器（备忘 / 任务计划共用）。
//! 正文格式：`memo-doc/v1`（颜色 / 链接 / 插图）；旧 Markdown 风格自动迁移。

use crate::save_dialog;
use crate::theme;
use eframe::egui::{
    self, Color32, Frame, Id, Margin, Order, RichText, Rounding, Sense, Stroke, TextBuffer, Vec2,
};
use memo_core::attachments;
use memo_core::memo_doc::{self, Block, ListKind, Span};
use memo_core::store::MemoCategory;
use std::path::Path;

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
    ui.add(
        egui::Button::new(label)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .min_size(Vec2::new(22.0, TOOL_H)),
    )
    .on_hover_text(tip)
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

                tool_group(ui, |ui| {
                    if tool_btn(ui, RichText::new("B").strong().size(12.5).color(fg), "加粗")
                        .clicked()
                    {
                        toggle_style_on_last_para(
                            doc,
                            StylePatch {
                                bold: Some(true),
                                ..Default::default()
                            },
                        );
                    }
                    if tool_btn(ui, RichText::new("I").italics().size(12.5).color(fg), "斜体")
                        .clicked()
                    {
                        toggle_style_on_last_para(
                            doc,
                            StylePatch {
                                italic: Some(true),
                                ..Default::default()
                            },
                        );
                    }
                    if tool_btn(
                        ui,
                        RichText::new("U").underline().size(12.5).color(fg),
                        "下划线",
                    )
                    .clicked()
                    {
                        toggle_style_on_last_para(
                            doc,
                            StylePatch {
                                underline: Some(true),
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
                                    toggle_style_on_last_para(
                                        doc,
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
                                        toggle_style_on_last_para(
                                            doc,
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
                    if tool_btn(ui, RichText::new("1.").size(12.0).color(fg), "编号列表").clicked()
                    {
                        ensure_body_paragraph(doc);
                        doc.blocks.push(Block::Items {
                            items: vec![String::new()],
                            list_kind: ListKind::Numbered,
                        });
                    }
                    if tool_btn(ui, RichText::new("•").size(13.0).color(fg), "项目符号").clicked()
                    {
                        ensure_body_paragraph(doc);
                        doc.blocks.push(Block::Items {
                            items: vec![String::new()],
                            list_kind: ListKind::Bullet,
                        });
                    }
                    if tool_btn(ui, RichText::new("▦").size(12.0).color(fg), "插入表格").clicked()
                    {
                        ensure_body_paragraph(doc);
                        doc.blocks.push(Block::Table {
                            headers: vec!["工作内容".into(), "完成情况".into()],
                            rows: vec![vec![String::new(), String::new()]; 2],
                        });
                    }
                });

                tool_group(ui, |ui| {
                    let link_label = RichText::new("链")
                        .size(12.0)
                        .color(link_style_color())
                        .strong();
                    if tool_btn(ui, link_label, "插入链接").clicked() {
                        *show_link = !*show_link;
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
                                        ensure_body_paragraph(doc);
                                        doc.blocks.push(Block::Image {
                                            file_id,
                                            alt: path
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or("图片")
                                                .to_string(),
                                        });
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

/// 插入链接副条（固定高度，不随窗口变高）。
fn toolbar_link_row(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    link_text: &mut String,
    link_url: &mut String,
    show_link: &mut bool,
) {
    let strip_w = fit_ui_width(ui);
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
                        ui.add(
                            egui::TextEdit::singleline(link_text)
                                .desired_width(100.0)
                                .hint_text(theme::hint("显示文字"))
                                .margin(egui::vec2(6.0, 3.0)),
                        );
                        ui.add(
                            egui::TextEdit::singleline(link_url)
                                .desired_width(140.0)
                                .hint_text(theme::hint("https://…"))
                                .margin(egui::vec2(6.0, 3.0)),
                        );
                        let can = !link_text.trim().is_empty() && !link_url.trim().is_empty();
                        if ui
                            .add_enabled(
                                can,
                                egui::Button::new(RichText::new("插入").size(12.0))
                                    .min_size(Vec2::new(0.0, TOOL_H)),
                            )
                            .clicked()
                        {
                            ensure_body_paragraph(doc);
                            if let Some(Block::Paragraph { spans }) = doc
                                .blocks
                                .iter_mut()
                                .rev()
                                .find(|b| matches!(b, Block::Paragraph { .. }))
                            {
                                insert_inline_link(spans, link_text, link_url);
                            }
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
                            *show_link = false;
                        }
                    });
                });
        },
    );
}

/// Word 式选区迷你工具条：松手后窄条；颜色/链接点开再展开。
/// `hide_for_drag`：仅正文内拖选时为 true（点迷你条时必须仍绘制，否则按钮吃不到点击）。
/// 返回浮层是否仍被指针悬停（用于保留选区快照）。
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
                            .on_hover_text("加粗")
                            .clicked()
                        {
                            bar_interacted = true;
                            apply_style_to_char_range(
                                spans,
                                state.start,
                                state.end,
                                StylePatch {
                                    bold: Some(true),
                                    ..Default::default()
                                },
                            );
                            dismiss = true;
                        }
                        if ui
                            .small_button(RichText::new("I").italics())
                            .on_hover_text("斜体")
                            .clicked()
                        {
                            bar_interacted = true;
                            apply_style_to_char_range(
                                spans,
                                state.start,
                                state.end,
                                StylePatch {
                                    italic: Some(true),
                                    ..Default::default()
                                },
                            );
                            dismiss = true;
                        }
                        if ui
                            .small_button(RichText::new("U").underline())
                            .on_hover_text("下划线")
                            .clicked()
                        {
                            bar_interacted = true;
                            apply_style_to_char_range(
                                spans,
                                state.start,
                                state.end,
                                StylePatch {
                                    underline: Some(true),
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

/// 多行正文：按 spans 着色（链接=蓝+下划线），折行避免撑破抽屉。
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

/// 段落编辑：颜色/链接/BIU/字号；选区迷你条。
fn edit_paragraph_spans(ui: &mut egui::Ui, spans: &mut Vec<Span>, id_salt: &str, body_rows: usize) {
    if spans.is_empty() {
        spans.push(Span::plain(""));
    }

    let sel_id = Id::new(("doc_body_sel", id_salt));
    let te_id = ui.make_persistent_id(("doc_body", id_salt));
    let mut sel = ui.ctx().data(|d| d.get_temp::<BodySelection>(sel_id));
    let edit_w = fit_ui_width(ui);

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
    if output.response.changed() {
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
                            Id::new(("doc_link_tip", id_salt)),
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

    if let Some(range) = output.cursor_range.as_ref() {
        if !range.is_empty() {
            let chars = range.as_sorted_char_range();
            let text = plain.char_range(chars.clone()).to_string();
            if !text.is_empty() {
                let anchor = selection_anchor(&output, range);
                let (keep_color, keep_link, keep_size, keep_url) = sel
                    .as_ref()
                    .map(|s| (s.color_mode, s.link_mode, s.size_mode, s.link_url.clone()))
                    .unwrap_or((false, false, false, String::new()));
                sel = Some(BodySelection {
                    start: chars.start,
                    end: chars.end,
                    text,
                    anchor,
                    color_mode: keep_color,
                    link_mode: keep_link,
                    size_mode: keep_size,
                    link_url: keep_url,
                });
            }
        }
    }

    let hide_for_drag = ui.input(|i| i.pointer.primary_down())
        && (output.response.is_pointer_button_down_on() || output.response.contains_pointer());
    let popup_hot = show_selection_toolbar(ui, spans, id_salt, &mut sel, hide_for_drag);

    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        if let Some(s) = sel.as_mut() {
            if s.color_mode || s.link_mode || s.size_mode {
                s.color_mode = false;
                s.link_mode = false;
                s.size_mode = false;
                s.link_url.clear();
            } else {
                sel = None;
            }
        }
    } else if ui.input(|i| i.pointer.any_click()) && !popup_hot {
        let te_has_sel = output
            .cursor_range
            .as_ref()
            .is_some_and(|r| !r.is_empty());
        let click_in_editor = output.response.clicked();
        if !(click_in_editor && te_has_sel) {
            sel = None;
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

/// 悬停块类型（互斥：同一时刻只亮一块）。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum HoverBlockKind {
    #[default]
    List,
    Table,
}

type HoverActive = (HoverBlockKind, usize);

/// 悬停工具条：浮层；仅 `allowed` 时显示；stick 严格限制在块∪工具条内。
/// 返回 tools_hot，供上层互斥状态机保持 active。
fn hover_block_tools(
    ui: &mut egui::Ui,
    id: Id,
    block_rect: egui::Rect,
    block_hot: bool,
    allowed: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    let stick_id = id.with("stick");
    let tools_rect_id = id.with("tools_rect");
    if !allowed {
        ui.ctx().data_mut(|d| {
            d.insert_temp(stick_id, false);
            d.insert_temp(tools_rect_id, egui::Rect::NOTHING);
        });
        return false;
    }

    let stick = ui.ctx().data(|d| d.get_temp::<bool>(stick_id)).unwrap_or(false);
    let last_tools = ui
        .ctx()
        .data(|d| d.get_temp::<egui::Rect>(tools_rect_id))
        .filter(|r| r.is_positive());

    let pointer = ui.ctx().pointer_interact_pos();
    let in_keep_zone = pointer.is_some_and(|p| {
        let mut zone = block_rect.expand(4.0);
        if let Some(tr) = last_tools {
            zone = zone.union(tr);
        }
        zone.contains(p)
    });

    // 未在块/工具条区域则绝不再画（防止表格 stick 串到下方列表）
    let show = block_hot || (stick && in_keep_zone);
    let mut tools_hot = false;
    let mut tools_rect = egui::Rect::NOTHING;

    if show && block_rect.is_finite() && block_rect.width() > 1.0 {
        // 右上角内锚：工具条向左展开，避免伸出抽屉右缘
        let pos = egui::pos2(
            (block_rect.right() - 2.0).max(block_rect.left() + 8.0),
            block_rect.top() + 1.0,
        );
        let area = egui::Area::new(id)
            .order(Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(pos)
            .constrain(true)
            .interactable(true)
            .show(ui.ctx(), |ui| {
                // 无描边外框：仅透明按钮横排
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.horizontal(|ui| add_contents(ui));
            });
        tools_hot = area.response.hovered() || area.response.contains_pointer();
        tools_rect = area.response.rect;
    }

    let next_stick = block_hot || tools_hot;
    ui.ctx().data_mut(|d| {
        d.insert_temp(stick_id, next_stick);
        d.insert_temp(
            tools_rect_id,
            if tools_rect.is_positive() {
                tools_rect
            } else {
                egui::Rect::NOTHING
            },
        );
    });
    tools_hot
}

/// Word 式列表。`tools_allowed`：全局互斥后才显示浮层。
/// 返回 (删除块, 指针是否在块上, 工具条是否热)。
fn edit_items_block(
    ui: &mut egui::Ui,
    items: &mut Vec<String>,
    list_kind: ListKind,
    id_salt: &str,
    block_idx: usize,
    tools_allowed: bool,
) -> (bool, bool, bool) {
    let mut delete_block = false;
    let mut drop_item: Option<usize> = None;
    let mut insert_after: Option<usize> = None;
    let mut add_item = false;
    let focus_id = Id::new(("doc_item_focus", id_salt, block_idx));
    let want_focus: Option<usize> = ui.ctx().data(|d| d.get_temp(focus_id));

    if items.is_empty() {
        items.push(String::new());
    }

    let block_r = ui
        .vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for ii in 0..items.len() {
                ui.horizontal_top(|ui| {
                    let marker = match list_kind {
                        ListKind::Numbered => format!("{}.", ii + 1),
                        ListKind::Bullet => "•".into(),
                    };
                    ui.add_sized(
                        egui::vec2(24.0, 20.0),
                        egui::Label::new(
                            RichText::new(marker).size(14.0).color(theme::text()),
                        ),
                    );
                    let iw = ui.available_width().max(40.0);
                    let te_id = ui.make_persistent_id(("doc_item", id_salt, block_idx, ii));
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut items[ii])
                            .id(te_id)
                            .desired_width(iw)
                            .frame(false)
                            .margin(egui::vec2(0.0, 2.0)),
                    );
                    if want_focus == Some(ii) {
                        resp.request_focus();
                        ui.ctx().data_mut(|d| d.remove_temp::<usize>(focus_id));
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if items[ii].trim().is_empty() {
                            if items.len() == 1 {
                                delete_block = true;
                            } else {
                                drop_item = Some(ii);
                            }
                        } else {
                            insert_after = Some(ii);
                        }
                    }
                    if resp.has_focus()
                        && items[ii].is_empty()
                        && ui.input(|i| i.key_pressed(egui::Key::Backspace))
                    {
                        if items.len() == 1 {
                            delete_block = true;
                        } else {
                            drop_item = Some(ii);
                            if ii > 0 {
                                ui.ctx().data_mut(|d| d.insert_temp(focus_id, ii - 1));
                            }
                        }
                    }
                });
            }
        })
        .response;

    let block_hot = block_r.hovered() || block_r.contains_pointer();
    let tools_hot = hover_block_tools(
        ui,
        Id::new(("doc_list_tools", id_salt, block_idx)),
        block_r.rect,
        block_hot,
        tools_allowed,
        |ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("+").size(11.0))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .min_size(Vec2::splat(18.0)),
                )
                .on_hover_text("添加一项")
                .clicked()
            {
                add_item = true;
            }
            if ui
                .add(
                    egui::Button::new(RichText::new("×").size(11.0).color(theme::text_muted()))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .min_size(Vec2::splat(18.0)),
                )
                .on_hover_text("删除列表")
                .clicked()
            {
                delete_block = true;
            }
        },
    );

    if add_item {
        items.push(String::new());
        ui.ctx()
            .data_mut(|d| d.insert_temp(focus_id, items.len() - 1));
    }
    if let Some(ii) = insert_after {
        let at = ii + 1;
        items.insert(at, String::new());
        ui.ctx().data_mut(|d| d.insert_temp(focus_id, at));
    }
    if let Some(ii) = drop_item {
        if ii < items.len() {
            items.remove(ii);
            if items.is_empty() {
                delete_block = true;
            }
        }
    }
    (delete_block, block_hot, tools_hot)
}

/// 表格列偏好宽（不按页宽均分，避免边框撑到右边界）。
const TABLE_CELL_W: f32 = 120.0;

/// Word 式细线表：内容定宽；浮层互斥。
fn edit_table_block(
    ui: &mut egui::Ui,
    headers: &mut Vec<String>,
    rows: &mut Vec<Vec<String>>,
    id_salt: &str,
    block_idx: usize,
    tools_allowed: bool,
) -> (bool, bool, bool) {
    let mut delete_block = false;
    let mut add_row = false;
    let mut add_col = false;
    let ncols = headers.len().max(1);
    let col_gap = 0.0;
    let page_w = ui.available_width();
    let cell = TABLE_CELL_W;
    let need_w = cell * ncols as f32;
    let table_w = need_w.min(page_w);

    let block_r = ui
        .allocate_ui_with_layout(
            egui::vec2(table_w, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(table_w);
                ui.set_max_width(table_w);
                Frame::none()
                    .stroke(table_hairline())
                    .rounding(Rounding::same(2.0))
                    .inner_margin(Margin::ZERO)
                    .show(ui, |ui| {
                        ui.set_width(table_w);
                        if need_w > page_w {
                            egui::ScrollArea::horizontal()
                                .id_source(format!("{id_salt}_tbl_h_{block_idx}"))
                                .max_width(table_w)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.set_min_width(need_w);
                                    paint_edit_table_grid(
                                        ui, headers, rows, ncols, cell, col_gap, id_salt,
                                        block_idx,
                                    );
                                });
                        } else {
                            paint_edit_table_grid(
                                ui, headers, rows, ncols, cell, col_gap, id_salt, block_idx,
                            );
                        }
                    });
            },
        )
        .response;

    let block_hot = block_r.hovered() || block_r.contains_pointer();
    let tools_hot = hover_block_tools(
        ui,
        Id::new(("doc_tbl_tools", id_salt, block_idx)),
        block_r.rect,
        block_hot,
        tools_allowed,
        |ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("+行").size(11.0))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE),
                )
                .clicked()
            {
                add_row = true;
            }
            if ui
                .add(
                    egui::Button::new(RichText::new("+列").size(11.0))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE),
                )
                .clicked()
            {
                add_col = true;
            }
            if ui
                .add(
                    egui::Button::new(RichText::new("×").size(11.0).color(theme::text_muted()))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE),
                )
                .on_hover_text("删除表格")
                .clicked()
            {
                delete_block = true;
            }
        },
    );

    if add_row {
        rows.push(vec![String::new(); ncols]);
    }
    if add_col {
        headers.push(format!("列{}", headers.len() + 1));
        for row in rows.iter_mut() {
            row.push(String::new());
        }
    }
    (delete_block, block_hot, tools_hot)
}

fn paint_edit_table_grid(
    ui: &mut egui::Ui,
    headers: &mut Vec<String>,
    rows: &mut Vec<Vec<String>>,
    ncols: usize,
    cell: f32,
    col_gap: f32,
    id_salt: &str,
    block_idx: usize,
) {
    let line = table_hairline();
    egui::Grid::new(format!("{id_salt}_tbl_{block_idx}"))
        .num_columns(ncols)
        .striped(false)
        .spacing([col_gap, 0.0])
        .min_col_width(cell)
        .show(ui, |ui| {
            for h in headers.iter_mut() {
                Frame::none()
                    .fill(table_header_fill())
                    .stroke(line)
                    .inner_margin(Margin::symmetric(6.0, 4.0))
                    .show(ui, |ui| {
                        ui.set_min_width(cell - 2.0);
                        ui.add(
                            egui::TextEdit::singleline(h)
                                .desired_width(cell - 12.0)
                                .frame(false)
                                .font(egui::TextStyle::Body),
                        );
                    });
            }
            ui.end_row();
            for row in rows.iter_mut() {
                if row.len() < ncols {
                    row.resize(ncols, String::new());
                }
                for c in 0..ncols {
                    Frame::none()
                        .fill(theme::card())
                        .stroke(line)
                        .inner_margin(Margin::symmetric(6.0, 4.0))
                        .show(ui, |ui| {
                            ui.set_min_width(cell - 2.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut row[c])
                                    .desired_width(cell - 12.0)
                                    .frame(false),
                            );
                        });
                }
                ui.end_row();
            }
        });
}

fn fit_ui_width(ui: &egui::Ui) -> f32 {
    crate::page_shell::fit_avail_width(ui, ui.available_width())
}

/// 编辑器。`min_h` 为整卡最小高度；正文可无限撑开，由外层滚动。
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
    let paper_min = (min_h - chrome_h).max(96.0);

    // 外框：固定顶栏 + 纸面随内容增高（无上限）
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            let editor_w = fit_ui_width(ui).min(editor_w);
            ui.set_max_width(editor_w);
            ui.set_min_height(min_h);
            ui.set_width(editor_w);

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

            let paper_top = ui.cursor().min.y;
            Frame::none()
                .fill(theme::card())
                .inner_margin(Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    let inner_w = fit_ui_width(ui);
                    ui.set_max_width(inner_w);
                    ui.set_width(inner_w);
                    ui.spacing_mut().item_spacing.y = 6.0;

                            let active_id = Id::new(("doc_hover_active", id_salt));
                            let prev_active: Option<HoverActive> =
                                ui.ctx().data(|d| d.get_temp(active_id));
                            let mut hit_block: Option<HoverActive> = None;
                            let mut keep_via_tools: Option<HoverActive> = None;

                            let mut remove_at: Option<usize> = None;
                            let n_blocks = doc.blocks.len();
                            for idx in 0..n_blocks {
                                let del = match &mut doc.blocks[idx] {
                                    Block::Paragraph { spans } => {
                                        edit_paragraph_spans(
                                            ui,
                                            spans,
                                            &format!("{id_salt}_p{idx}"),
                                            2,
                                        );
                                        false
                                    }
                                    Block::Items { items, list_kind } => {
                                        let kind = *list_kind;
                                        let key = (HoverBlockKind::List, idx);
                                        // 无 active 时允许本帧探测；有 active 则仅该块可亮
                                        let allowed =
                                            prev_active.map(|a| a == key).unwrap_or(true);
                                        let (del, block_hot, tools_hot) = edit_items_block(
                                            ui, items, kind, id_salt, idx, allowed,
                                        );
                                        if block_hot {
                                            hit_block = Some(key);
                                        }
                                        if tools_hot && allowed {
                                            keep_via_tools = Some(key);
                                        }
                                        del
                                    }
                                    Block::Table { headers, rows } => {
                                        let key = (HoverBlockKind::Table, idx);
                                        let allowed =
                                            prev_active.map(|a| a == key).unwrap_or(true);
                                        let (del, block_hot, tools_hot) = edit_table_block(
                                            ui, headers, rows, id_salt, idx, allowed,
                                        );
                                        if block_hot {
                                            hit_block = Some(key);
                                        }
                                        if tools_hot && allowed {
                                            keep_via_tools = Some(key);
                                        }
                                        del
                                    }
                                    Block::Image { file_id, alt } => {
                                        let fid = file_id.clone();
                                        let alt_s = alt.clone();
                                        let mut del = false;
                                        let img_r = ui
                                            .vertical(|ui| {
                                                if !alt_s.is_empty() {
                                                    ui.label(
                                                        RichText::new(&alt_s)
                                                            .small()
                                                            .color(theme::text_muted()),
                                                    );
                                                }
                                                show_image_preview(
                                                    ui, data_dir, memo_id, &fid,
                                                );
                                            })
                                            .response;
                                        if img_r.hovered()
                                            && ui
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
                                            del = true;
                                        }
                                        del
                                    }
                                };
                                if del {
                                    remove_at = Some(idx);
                                }
                                if idx + 1 < n_blocks {
                                    ui.add_space(6.0);
                                }
                            }

                            // 互斥：指针所在块优先；否则保持仍热的工具条所属块
                            let next_active = hit_block.or(keep_via_tools);
                            ui.ctx().data_mut(|d| {
                                if let Some(a) = next_active {
                                    d.insert_temp(active_id, a);
                                } else {
                                    d.remove_temp::<HoverActive>(active_id);
                                }
                            });
                            // 页末留白，像 Word 可继续往下写
                            ui.add_space(48.0);

                            if let Some(i) = remove_at {
                                if i < doc.blocks.len() {
                                    let only_para = doc.blocks.len() == 1
                                        && matches!(
                                            doc.blocks[0],
                                            Block::Paragraph { .. }
                                        );
                                    if !only_para {
                                        if let Block::Image { file_id, .. } =
                                            &doc.blocks[i]
                                        {
                                            if let Some(dir) = data_dir {
                                                let p = attachments::path_of(
                                                    dir, memo_id, file_id,
                                                );
                                                let _ = std::fs::remove_file(p);
                                            }
                                        }
                                        doc.blocks.remove(i);
                                        ensure_body_paragraph(doc);
                                    }
                                }
                            }
                });
            // 内容不足最小纸面高度时补白，保证空备注仍有可点区域
            let used = (ui.cursor().min.y - paper_top).max(0.0);
            if used < paper_min {
                ui.add_space(paper_min - used);
            }
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
            Block::Items { items, list_kind } => {
                let mut n = 0usize;
                for it in items {
                    if it.trim().is_empty() {
                        continue;
                    }
                    n += 1;
                    let marker = match list_kind {
                        ListKind::Numbered => format!("{n}."),
                        ListKind::Bullet => "•".into(),
                    };
                    ui.horizontal_top(|ui| {
                        ui.add_sized(
                            egui::vec2(28.0, 20.0),
                            egui::Label::new(
                                RichText::new(marker)
                                    .strong()
                                    .size(14.0)
                                    .color(theme::text_muted()),
                            ),
                        );
                        let w = (ui.available_width() - 4.0).max(40.0);
                        ui.allocate_ui(egui::vec2(w, 0.0), |ui| {
                            ui.set_max_width(w);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(it).size(15.0).color(theme::text()),
                                )
                                .wrap(true),
                            );
                        });
                    });
                    ui.add_space(2.0);
                }
                ui.add_space(6.0);
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
    fn apply_bold_and_list_kind_roundtrip() {
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
        doc.blocks.push(Block::Items {
            items: vec!["a".into()],
            list_kind: ListKind::Bullet,
        });
        let (title, body) = doc.to_store("未命名");
        let again = Doc::from_store(&title, &body);
        assert!(matches!(
            &again.blocks[1],
            Block::Items {
                list_kind: ListKind::Bullet,
                ..
            }
        ));
    }
}
