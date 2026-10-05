//! 公文风所见即所得文档编辑器（备忘 / 任务计划共用）。
//! 始终第一行（标题块）为标题；保存时 split 写入 store。不向用户暴露 Markdown。

use crate::theme;
use eframe::egui::{self, Frame, Margin, RichText, Rounding, Stroke};
use memo_core::store::MemoCategory;

#[derive(Debug, Clone)]
pub enum Block {
    Paragraph { text: String },
    Items { items: Vec<String> },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

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
            blocks: vec![Block::Paragraph {
                text: String::new(),
            }],
        }
    }

    pub fn from_store(title: &str, content: &str) -> Self {
        parse(&join_note(title, content))
    }

    pub fn to_store(&self, default_title: &str) -> (String, String) {
        split_note(&serialize(self), default_title)
    }
}

/// 合并为编辑用全文：第一行标题，其余正文。
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

pub fn parse(note: &str) -> Doc {
    let (title, content) = split_note(note, "");
    let mut blocks = parse_blocks(&content);
    if blocks.is_empty() {
        blocks.push(Block::Paragraph {
            text: String::new(),
        });
    }
    Doc { title, blocks }
}

pub fn serialize(doc: &Doc) -> String {
    let title = doc.title.trim();
    let body = serialize_blocks(&doc.blocks);
    if body.trim().is_empty() {
        title.to_string()
    } else if title.is_empty() {
        body
    } else {
        format!("{title}\n{body}")
    }
}

fn parse_blocks(md: &str) -> Vec<Block> {
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    let mut blocks: Vec<Block> = Vec::new();
    let mut para = String::new();

    let flush_para = |para: &mut String, blocks: &mut Vec<Block>| {
        let t = para.trim_end();
        if !t.is_empty() {
            blocks.push(Block::Paragraph {
                text: t.to_string(),
            });
        }
        para.clear();
    };

    while i < lines.len() {
        let line = lines[i].trim_end();

        if line.trim().is_empty() {
            flush_para(&mut para, &mut blocks);
            i += 1;
            continue;
        }

        if looks_like_table_header(line) && i + 1 < lines.len() && is_table_sep(lines[i + 1]) {
            flush_para(&mut para, &mut blocks);
            let headers = split_table_row(line);
            i += 2;
            let mut rows = Vec::new();
            while i < lines.len() {
                let row = lines[i].trim_end();
                if !row.trim_start().starts_with('|') {
                    break;
                }
                rows.push(split_table_row(row));
                i += 1;
            }
            let ncols = headers
                .len()
                .max(rows.iter().map(|r| r.len()).max().unwrap_or(0))
                .max(1);
            blocks.push(Block::Table {
                headers: pad_row(headers, ncols),
                rows: rows.into_iter().map(|r| pad_row(r, ncols)).collect(),
            });
            continue;
        }

        if let Some((_n, item)) = ordered_item(line) {
            flush_para(&mut para, &mut blocks);
            let mut items = vec![item.to_string()];
            i += 1;
            while i < lines.len() {
                if let Some((_n, it)) = ordered_item(lines[i].trim_end()) {
                    items.push(it.to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(Block::Items { items });
            continue;
        }

        if let Some(item) = unordered_item(line) {
            flush_para(&mut para, &mut blocks);
            let mut items = vec![item.to_string()];
            i += 1;
            while i < lines.len() {
                if let Some(it) = unordered_item(lines[i].trim_end()) {
                    items.push(it.to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(Block::Items { items });
            continue;
        }

        if !para.is_empty() {
            para.push('\n');
        }
        para.push_str(line);
        i += 1;
    }
    flush_para(&mut para, &mut blocks);
    blocks
}

fn serialize_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    for (bi, block) in blocks.iter().enumerate() {
        if bi > 0 {
            out.push('\n');
        }
        match block {
            Block::Paragraph { text } => {
                out.push_str(text.trim_end());
                out.push('\n');
            }
            Block::Items { items } => {
                for (i, it) in items.iter().enumerate() {
                    out.push_str(&format!("{}. ", i + 1));
                    out.push_str(it);
                    out.push('\n');
                }
            }
            Block::Table { headers, rows } => {
                out.push_str(&format_table_row(headers));
                out.push('\n');
                out.push('|');
                for _ in headers {
                    out.push_str(" --- |");
                }
                out.push('\n');
                for row in rows {
                    let mut padded = row.clone();
                    if padded.len() < headers.len() {
                        padded.resize(headers.len(), String::new());
                    }
                    out.push_str(&format_table_row(&padded[..headers.len()]));
                    out.push('\n');
                }
            }
        }
    }
    out
}

fn format_table_row(cells: &[String]) -> String {
    let mut s = String::from("|");
    for c in cells {
        s.push(' ');
        s.push_str(c);
        s.push_str(" |");
    }
    s
}

fn pad_row(mut row: Vec<String>, ncols: usize) -> Vec<String> {
    if row.len() < ncols {
        row.resize(ncols, String::new());
    } else if row.len() > ncols {
        row.truncate(ncols);
    }
    row
}

fn unordered_item(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let rest = t.strip_prefix('-').or_else(|| t.strip_prefix('*'))?;
    // 允许 "-"/ "*" 或 "- "/"* "；trim_end 后空事项会变成 "-"。
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn ordered_item(line: &str) -> Option<(u32, &str)> {
    let t = line.trim_start();
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let (num_s, rest) = t.split_at(digits);
    // 允许 "1. item" / "1."；parse_blocks 的 trim_end 会把空事项 "1. " 收成 "1."。
    let item = match rest.strip_prefix('.') {
        Some(after) if after.is_empty() || after.starts_with(char::is_whitespace) => {
            after.trim_start()
        }
        _ => return None,
    };
    let num: u32 = num_s.parse().ok()?;
    Some((num, item))
}

fn looks_like_table_header(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.matches('|').count() >= 2
}

fn is_table_sep(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|')
        && t.chars()
            .all(|c| c == '|' || c == '-' || c == ':' || c == ' ')
}

fn split_table_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|s| s.trim().to_string()).collect()
}

fn toolbar(ui: &mut egui::Ui, doc: &mut Doc) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if ui.small_button("事项").on_hover_text("添加编号事项").clicked() {
            ensure_body_paragraph(doc);
            doc.blocks.push(Block::Items {
                items: vec![String::new(), String::new()],
            });
        }
        if ui.small_button("表格").on_hover_text("添加工作台账表").clicked() {
            ensure_body_paragraph(doc);
            doc.blocks.push(Block::Table {
                headers: vec!["工作内容".into(), "完成情况".into()],
                rows: vec![vec![String::new(), String::new()]; 2],
            });
        }
        ui.label(
            RichText::new("默认正文 · 需要时可添加事项或表格")
                .small()
                .color(theme::text_muted()),
        );
    });
}

fn ensure_body_paragraph(doc: &mut Doc) {
    if !doc.blocks.iter().any(|b| matches!(b, Block::Paragraph { .. })) {
        doc.blocks.insert(
            0,
            Block::Paragraph {
                text: String::new(),
            },
        );
    }
}

/// 将多个 Paragraph 合并为一段纯文本（同一正文框）。
fn coalesce_paragraphs(doc: &mut Doc) {
    let mut body = String::new();
    let mut rest = Vec::new();
    for block in doc.blocks.drain(..) {
        match block {
            Block::Paragraph { text } => {
                if body.is_empty() {
                    body = text;
                } else if !text.is_empty() {
                    if !body.ends_with('\n') {
                        body.push('\n');
                    }
                    body.push_str(&text);
                }
            }
            other => rest.push(other),
        }
    }
    doc.blocks.push(Block::Paragraph { text: body });
    doc.blocks.append(&mut rest);
}

fn block_chrome(
    ui: &mut egui::Ui,
    label: &str,
    can_delete: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut delete = false;
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(6.0))
        .inner_margin(Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(label)
                        .small()
                        .strong()
                        .color(theme::text_muted()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if can_delete
                        && ui
                            .small_button("删除本段")
                            .on_hover_text("删除此段")
                            .clicked()
                    {
                        delete = true;
                    }
                });
            });
            ui.add_space(4.0);
            add_contents(ui);
        });
    ui.add_space(6.0);
    delete
}

/// 编辑器：默认一个正文框；事项/表格按需添加。标题由表单字段维护，此处不画。
pub fn show_editor(ui: &mut egui::Ui, doc: &mut Doc, id_salt: &str) {
    coalesce_paragraphs(doc);
    ensure_body_paragraph(doc);

    toolbar(ui, doc);
    ui.add_space(4.0);
    // ScrollArea 内容区 available_height 常为 ∞；勿贪占剩余高度，留给外层表单滚动。
    let has_structure = doc
        .blocks
        .iter()
        .any(|b| matches!(b, Block::Items { .. } | Block::Table { .. }));
    let body_h = if has_structure { 260.0 } else { 148.0 };
    Frame::none()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width());
            ui.set_max_height(body_h);
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(body_h)
                .id_source(format!("{id_salt}_doc_scroll"))
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width());

                    // 正文框（合并后的第一段 Paragraph）
                    let body_rows = if has_structure { 5 } else { 7 };
                    if let Some(Block::Paragraph { text }) = doc.blocks.first_mut() {
                        let empty = text.is_empty();
                        let resp = ui.add(
                            egui::TextEdit::multiline(text)
                                .desired_width(ui.available_width())
                                .desired_rows(body_rows),
                        );
                        if empty {
                            ui.painter().text(
                                egui::pos2(resp.rect.left() + 4.0, resp.rect.top() + 4.0),
                                egui::Align2::LEFT_TOP,
                                "自由书写备注，换行即可编排…",
                                egui::FontId::proportional(14.5),
                                theme::text_muted(),
                            );
                        }
                    }
                    ui.add_space(8.0);

                    let mut remove_at: Option<usize> = None;
                    // 跳过索引 0 的正文段，只编辑事项/表格
                    for idx in 1..doc.blocks.len() {
                        let del = match &mut doc.blocks[idx] {
                            Block::Paragraph { .. } => false,
                            Block::Items { items } => block_chrome(ui, "事项", true, |ui| {
                                let mut drop_item = None;
                                let n_items = items.len();
                                for (ii, item) in items.iter_mut().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!("{}.", ii + 1))
                                                .strong()
                                                .color(theme::accent()),
                                        );
                                        ui.add(
                                            egui::TextEdit::singleline(item)
                                                .desired_width(
                                                    (ui.available_width() - 70.0).max(40.0),
                                                )
                                                .hint_text(theme::hint("落实要点")),
                                        );
                                        if n_items > 1 && ui.small_button("×").clicked() {
                                            drop_item = Some(ii);
                                        }
                                    });
                                }
                                if let Some(ii) = drop_item {
                                    items.remove(ii);
                                }
                                if ui.small_button("+ 事项").clicked() {
                                    items.push(String::new());
                                }
                            }),
                            Block::Table { headers, rows } => {
                                block_chrome(ui, "表格", true, |ui| {
                                    let ncols = headers.len().max(1);
                                    ui.horizontal(|ui| {
                                        if ui.small_button("加行").clicked() {
                                            rows.push(vec![String::new(); ncols]);
                                        }
                                        if ui.small_button("加列").clicked() {
                                            headers.push(format!("列{}", headers.len() + 1));
                                            for row in rows.iter_mut() {
                                                row.push(String::new());
                                            }
                                        }
                                    });
                                    ui.add_space(4.0);
                                    let ncols = headers.len().max(1);
                                    let col_gap = 8.0;
                                    let inner_w = ui.available_width();
                                    let cell = ((inner_w
                                        - col_gap * (ncols.saturating_sub(1) as f32))
                                        / ncols as f32)
                                        .clamp(72.0, 140.0);
                                    let need_w = cell * ncols as f32
                                        + col_gap * (ncols.saturating_sub(1) as f32);
                                    egui::ScrollArea::horizontal()
                                        .id_source(format!("{id_salt}_tbl_h_{idx}"))
                                        .max_width(inner_w)
                                        .auto_shrink([false, true])
                                        .show(ui, |ui| {
                                            ui.set_min_width(need_w);
                                            egui::Grid::new(format!("{id_salt}_tbl_{idx}"))
                                                .num_columns(ncols)
                                                .striped(true)
                                                .spacing([col_gap, 4.0])
                                                .show(ui, |ui| {
                                                    for h in headers.iter_mut() {
                                                        ui.add(
                                                            egui::TextEdit::singleline(h)
                                                                .desired_width(cell),
                                                        );
                                                    }
                                                    ui.end_row();
                                                    for row in rows.iter_mut() {
                                                        if row.len() < ncols {
                                                            row.resize(ncols, String::new());
                                                        }
                                                        for c in 0..ncols {
                                                            ui.add(
                                                                egui::TextEdit::singleline(
                                                                    &mut row[c],
                                                                )
                                                                .desired_width(cell),
                                                            );
                                                        }
                                                        ui.end_row();
                                                    }
                                                });
                                        });
                                })
                            }
                        };
                        if del {
                            remove_at = Some(idx);
                        }
                    }
                    if let Some(i) = remove_at {
                        if i > 0 && i < doc.blocks.len() {
                            doc.blocks.remove(i);
                        }
                    }
                });
        });
}

/// 只读：正文/事项/表格版式（标题由详情顶栏展示）。
pub fn show_viewer_body(ui: &mut egui::Ui, content: &str) {
    let blocks = parse_blocks(content);
    if blocks.is_empty()
        || (blocks.len() == 1
            && matches!(&blocks[0], Block::Paragraph { text } if text.trim().is_empty()))
    {
        ui.label(theme::muted_label("（正文为空）").size(15.0));
        return;
    }
    render_blocks_readonly(ui, &blocks);
}

fn render_blocks_readonly(ui: &mut egui::Ui, blocks: &[Block]) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let mut table_seq = 0u32;
    for block in blocks {
        match block {
            Block::Paragraph { text } => {
                if text.trim().is_empty() {
                    continue;
                }
                ui.label(RichText::new(text).size(15.5).color(theme::text()));
                ui.add_space(4.0);
            }
            Block::Items { items } => {
                for (i, it) in items.iter().enumerate() {
                    if it.trim().is_empty() {
                        continue;
                    }
                    ui.horizontal_top(|ui| {
                        ui.label(
                            RichText::new(format!("{}.", i + 1))
                                .strong()
                                .size(15.0)
                                .color(theme::accent()),
                        );
                        ui.label(RichText::new(it).size(15.0).color(theme::text()));
                    });
                }
                ui.add_space(4.0);
            }
            Block::Table { headers, rows } => {
                table_seq += 1;
                let ncols = headers.len().max(1);
                Frame::none()
                    .fill(theme::card())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .rounding(Rounding::same(6.0))
                    .inner_margin(Margin::same(8.0))
                    .show(ui, |ui| {
                        egui::Grid::new(format!("view_tbl_{table_seq}"))
                            .num_columns(ncols)
                            .striped(true)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                for h in headers {
                                    ui.label(
                                        RichText::new(h).strong().size(13.0).color(theme::text()),
                                    );
                                }
                                ui.end_row();
                                for row in rows {
                                    for c in 0..ncols {
                                        let cell = row.get(c).map(|s| s.as_str()).unwrap_or("");
                                        ui.label(
                                            RichText::new(cell).size(13.0).color(theme::text()),
                                        );
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                ui.add_space(4.0);
            }
        }
    }
}

/// 备忘新建模板（第一行标题）。兼容旧调用，等同工作学习模板。
#[allow(dead_code)]
pub fn memo_template(stamp: &str) -> String {
    memo_template_for(MemoCategory::Work, stamp)
}

/// 按分类套用新建模板，便于直接填写（纯文本进正文框，不预置事项/表格）。
pub fn memo_template_for(category: MemoCategory, stamp: &str) -> String {
    match category {
        MemoCategory::Todo => format!(
            "待办 {stamp}\n\n\
             （一句话说明要做什么；需要时可点上方「事项」或「表格」）"
        ),
        MemoCategory::Credentials => format!(
            "账号证件 {stamp}\n\n\
             （用途：某网站 / 银行卡 / 证件）\n\
             名称或机构：\n\
             账号或卡号：\n\
             密码或口令：\n\
             有效期：\n\
             其它备注："
        ),
        MemoCategory::Work | MemoCategory::Office => format!(
            "工作学习 {stamp}\n\n\
             （情况说明，自由书写即可）"
        ),
        MemoCategory::Life => format!(
            "生活家庭 {stamp}\n\n\
             （发生了什么 / 想记下来的事）"
        ),
        MemoCategory::Finance => format!(
            "财务订阅 {stamp}\n\n\
             （账单 / 订阅 / 收支说明）\n\
             名称：\n\
             金额：\n\
             周期：\n\
             下次扣款或到期：\n\
             备注："
        ),
        MemoCategory::GenderPrivate | MemoCategory::WomenPrivate | MemoCategory::MalePrivate => {
            format!(
                "性别私密备注 {stamp}\n\n\
             （仅本人可见。可记经期感受、体检、用药保健、情绪，或为伴侣留下的笔记）\n\n\
             经期日与体检日请在「性别私密」专属页设置；系统提醒仅供参考，不能替代就医。"
            )
        }
        MemoCategory::Emergency => format!(
            "应急 {stamp}\n\n\
             （紧急情况一句话描述）\n\
             联系人：\n\
             电话：\n\
             地址或集合点：\n\
             立即要做："
        ),
        MemoCategory::Inspiration => format!(
            "灵感 {stamp}\n\n\
             （先写下那一点想法，不用完美）"
        ),
        MemoCategory::General => format!(
            "备忘 {stamp}\n\n\
             （在此填写）"
        ),
    }
}

/// 任务计划新建默认（第一行与 task_title 对齐时可再用 split）。
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
        let n_items = doc
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Items { .. }))
            .count();
        let n_table = doc
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Table { .. }))
            .count();
        assert_eq!(n_para, 1, "应只有一段正文");
        assert_eq!(n_items, 0, "模板不应预置事项");
        assert_eq!(n_table, 0, "模板不应预置表格");
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
    fn empty_ordered_items_survive_trim_end() {
        assert_eq!(ordered_item("1."), Some((1, "")));
        assert_eq!(ordered_item("1. "), Some((1, "")));
        assert_eq!(ordered_item("2. 落实"), Some((2, "落实")));
        assert_eq!(ordered_item("3.foo"), None);
        assert_eq!(unordered_item("-"), Some(""));
        assert_eq!(unordered_item("- "), Some(""));
        assert_eq!(unordered_item("-x"), None);
    }
}
