//! 公文风所见即所得文档编辑器（备忘 / 任务计划共用）。
//! 始终第一行（标题块）为标题；保存时 split 写入 store。不向用户暴露 Markdown。

use crate::theme;
use eframe::egui::{self, Frame, Margin, RichText, Rounding, Stroke, Vec2};

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
    t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))
}

fn ordered_item(line: &str) -> Option<(u32, &str)> {
    let t = line.trim_start();
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let (num_s, rest) = t.split_at(digits);
    let rest = rest.strip_prefix(". ")?;
    let num: u32 = num_s.parse().ok()?;
    Some((num, rest))
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
        if ui.small_button("正文").on_hover_text("添加一段情况说明").clicked() {
            doc.blocks.push(Block::Paragraph {
                text: String::new(),
            });
        }
        if ui.small_button("事项").on_hover_text("添加编号事项").clicked() {
            doc.blocks.push(Block::Items {
                items: vec![String::new(), String::new()],
            });
        }
        if ui.small_button("表格").on_hover_text("添加工作台账表").clicked() {
            doc.blocks.push(Block::Table {
                headers: vec!["工作内容".into(), "完成情况".into()],
                rows: vec![vec![String::new(), String::new()]; 2],
            });
        }
        ui.label(
            RichText::new("第一行是标题 · 保存时自动识别")
                .small()
                .color(theme::TEXT_MUTED),
        );
    });
}

fn block_chrome(
    ui: &mut egui::Ui,
    label: &str,
    can_delete: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut delete = false;
    Frame::none()
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .rounding(Rounding::same(6.0))
        .inner_margin(Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(label)
                        .small()
                        .strong()
                        .color(theme::TEXT_MUTED),
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

/// 编辑器：标题块 + 正文/事项/表格。
pub fn show_editor(ui: &mut egui::Ui, doc: &mut Doc, id_salt: &str) {
    toolbar(ui, doc);
    ui.add_space(4.0);
    let body_h = (ui.available_height() - 4.0).max(180.0);
    let body_w = ui.available_width();
    Frame::none()
        .fill(theme::PANEL)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.set_min_size(Vec2::new(body_w - 2.0, body_h));
            ui.set_max_height(body_h);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_source(format!("{id_salt}_doc_scroll"))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());

                    // 标题（第一行，不可删除）
                    Frame::none()
                        .fill(theme::CARD)
                        .stroke(Stroke::new(1.0, theme::BORDER_STRONG))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(Margin::symmetric(10.0, 8.0))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("标题（第一行）")
                                    .small()
                                    .strong()
                                    .color(theme::TEXT_MUTED),
                            );
                            ui.add_space(4.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut doc.title)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Heading)
                                    .hint_text("输入标题…"),
                            );
                        });
                    ui.add_space(8.0);

                    if doc.blocks.is_empty() {
                        doc.blocks.push(Block::Paragraph {
                            text: String::new(),
                        });
                    }
                    let can_delete = doc.blocks.len() > 1;
                    let mut remove_at: Option<usize> = None;
                    for (idx, block) in doc.blocks.iter_mut().enumerate() {
                        let del = match block {
                            Block::Paragraph { text } => block_chrome(
                                ui,
                                "正文",
                                can_delete,
                                |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(text)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(3)
                                            .hint_text("情况说明、意见…"),
                                    );
                                },
                            ),
                            Block::Items { items } => block_chrome(
                                ui,
                                "事项",
                                can_delete,
                                |ui| {
                                    let mut drop_item = None;
                                    let n_items = items.len();
                                    for (ii, item) in items.iter_mut().enumerate() {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(format!("{}.", ii + 1))
                                                    .strong()
                                                    .color(theme::ACCENT),
                                            );
                                            ui.add(
                                                egui::TextEdit::singleline(item)
                                                    .desired_width(
                                                        (ui.available_width() - 70.0).max(80.0),
                                                    )
                                                    .hint_text("落实要点"),
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
                                },
                            ),
                            Block::Table { headers, rows } => block_chrome(
                                ui,
                                "表格",
                                can_delete,
                                |ui| {
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
                                    egui::Grid::new(format!("{id_salt}_tbl_{idx}"))
                                        .num_columns(ncols)
                                        .striped(true)
                                        .spacing([8.0, 4.0])
                                        .show(ui, |ui| {
                                            for h in headers.iter_mut() {
                                                ui.add(
                                                    egui::TextEdit::singleline(h)
                                                        .desired_width(120.0),
                                                );
                                            }
                                            ui.end_row();
                                            for row in rows.iter_mut() {
                                                if row.len() < ncols {
                                                    row.resize(ncols, String::new());
                                                }
                                                for c in 0..ncols {
                                                    ui.add(
                                                        egui::TextEdit::singleline(&mut row[c])
                                                            .desired_width(120.0),
                                                    );
                                                }
                                                ui.end_row();
                                            }
                                        });
                                },
                            ),
                        };
                        if del {
                            remove_at = Some(idx);
                        }
                    }
                    if let Some(i) = remove_at {
                        if doc.blocks.len() > 1 {
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
                ui.label(RichText::new(text).size(15.5).color(theme::TEXT));
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
                                .color(theme::ACCENT),
                        );
                        ui.label(RichText::new(it).size(15.0).color(theme::TEXT));
                    });
                }
                ui.add_space(4.0);
            }
            Block::Table { headers, rows } => {
                table_seq += 1;
                let ncols = headers.len().max(1);
                Frame::none()
                    .fill(theme::CARD)
                    .stroke(Stroke::new(1.0, theme::BORDER))
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
                                        RichText::new(h).strong().size(13.0).color(theme::TEXT),
                                    );
                                }
                                ui.end_row();
                                for row in rows {
                                    for c in 0..ncols {
                                        let cell = row.get(c).map(|s| s.as_str()).unwrap_or("");
                                        ui.label(
                                            RichText::new(cell).size(13.0).color(theme::TEXT),
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

/// 备忘新建模板（第一行标题）。
pub fn memo_template(stamp: &str) -> String {
    format!(
        "工作备忘 {stamp}\n\n\
         （在此填写情况说明）\n\n\
         1. \n\
         2. \n\n\
         | 工作内容 | 完成情况 |\n\
         | --- | --- |\n\
         |  |  |\n"
    )
}

/// 任务计划新建默认（第一行与 task_title 对齐时可再用 split）。
pub fn task_plan_template(title: &str) -> String {
    let t = if title.trim().is_empty() {
        "未命名任务"
    } else {
        title.trim()
    };
    format!("{t}\n\n（工作说明）\n\n1. \n")
}
