//! 轻量 Markdown 子集渲染（使用说明与备忘预览共用；无图片）。

use crate::theme;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Stroke, Vec2};

pub fn show(ui: &mut egui::Ui, md: &str) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    let mut table_seq = 0u32;
    while i < lines.len() {
        let raw = lines[i];
        let line = raw.trim_end();

        if line.trim().is_empty() {
            ui.add_space(8.0);
            i += 1;
            continue;
        }

        if is_hr(line) {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            i += 1;
            continue;
        }

        if let Some(level) = heading_level(line) {
            let text = line[level + 1..].trim();
            ui.add_space(if level == 1 { 4.0 } else { 10.0 });
            let size = match level {
                1 => 22.0,
                2 => 17.0,
                _ => 15.0,
            };
            ui.label(
                RichText::new(text)
                    .strong()
                    .size(size)
                    .color(theme::text()),
            );
            if level == 1 {
                ui.add_space(2.0);
            }
            i += 1;
            continue;
        }

        if line.trim_start().starts_with('>') {
            let body = line.trim_start().trim_start_matches('>').trim();
            Frame::none()
                .fill(theme::accent_soft())
                .rounding(Rounding::same(6.0))
                .inner_margin(Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (bar, _) =
                            ui.allocate_exact_size(Vec2::new(3.0, 18.0), egui::Sense::hover());
                        ui.painter().rect_filled(bar, Rounding::same(1.0), theme::accent());
                        ui.add_space(8.0);
                        rich_line(ui, body, 13.5, theme::text());
                    });
                });
            ui.add_space(4.0);
            i += 1;
            continue;
        }

        if looks_like_table_header(line) && i + 1 < lines.len() && is_table_sep(lines[i + 1]) {
            let headers = split_table_row(line);
            i += 2; // skip header + sep
            let mut rows: Vec<Vec<String>> = Vec::new();
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
            table_seq += 1;
            let table_id = table_seq;
            Frame::none()
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::border()))
                .rounding(Rounding::same(6.0))
                .inner_margin(Margin::same(8.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    egui::Grid::new(format!("md_table_{table_id}"))
                        .num_columns(ncols)
                        .striped(true)
                        .min_col_width(48.0)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            for c in 0..ncols {
                                let h = headers.get(c).map(|s| s.as_str()).unwrap_or("");
                                ui.label(
                                    RichText::new(h).strong().size(13.0).color(theme::text()),
                                );
                            }
                            ui.end_row();
                            for row in &rows {
                                for c in 0..ncols {
                                    let cell = row.get(c).map(|s| s.as_str()).unwrap_or("");
                                    rich_line(ui, cell, 13.0, theme::text());
                                }
                                ui.end_row();
                            }
                        });
                });
            ui.add_space(4.0);
            continue;
        }

        if line.trim_start().starts_with("```") {
            let mut code = String::new();
            i += 1;
            while i < lines.len() {
                if lines[i].trim_start().starts_with("```") {
                    i += 1;
                    break;
                }
                if !code.is_empty() {
                    code.push('\n');
                }
                code.push_str(lines[i]);
                i += 1;
            }
            Frame::none()
                .fill(theme::panel())
                .stroke(Stroke::new(1.0, theme::border()))
                .rounding(Rounding::same(6.0))
                .inner_margin(Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        RichText::new(if code.is_empty() { " " } else { &code })
                            .monospace()
                            .size(13.0)
                            .color(theme::navy_mid()),
                    );
                });
            ui.add_space(4.0);
            continue;
        }

        if let Some(item) = unordered_item(line) {
            list_item(ui, "•", item);
            i += 1;
            continue;
        }

        if let Some((num, item)) = ordered_item(line) {
            list_item(ui, &format!("{num}."), item);
            i += 1;
            continue;
        }

        // 普通段落（可含行末两个空格换行的续行，本说明较少见，按单行处理）
        rich_line(ui, line.trim(), 14.0, theme::text());
        i += 1;
    }
}

fn is_hr(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && t.chars().all(|c| c == '-' || c == '*' || c == '_')
}

fn heading_level(line: &str) -> Option<usize> {
    let t = line.trim_start();
    if !t.starts_with('#') {
        return None;
    }
    let level = t.chars().take_while(|c| *c == '#').count();
    if level >= 1 && level <= 3 && t.as_bytes().get(level) == Some(&b' ') {
        Some(level)
    } else {
        None
    }
}

fn unordered_item(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix("- ") {
        return Some(rest);
    }
    if let Some(rest) = t.strip_prefix("* ") {
        return Some(rest);
    }
    None
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

fn list_item(ui: &mut egui::Ui, bullet: &str, text: &str) {
    ui.horizontal_top(|ui| {
        ui.add_space(8.0);
        ui.label(
            RichText::new(bullet)
                .strong()
                .size(14.0)
                .color(theme::accent()),
        );
        ui.add_space(6.0);
        // 列表正文可换行
        ui.allocate_ui_with_layout(
            Vec2::new((ui.available_width() - 4.0).max(40.0), 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                rich_line(ui, text, 14.0, theme::text());
            },
        );
    });
}

/// 渲染含 **粗体**、*斜体* 与 `代码` 的一行。
fn rich_line(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(after_bold) = rest.strip_prefix("**") {
                if let Some(end) = after_bold.find("**") {
                    let (bold, tail) = after_bold.split_at(end);
                    ui.label(RichText::new(bold).strong().size(size).color(color));
                    rest = &tail[2..];
                    continue;
                }
            }
            if let Some(after_em) = rest.strip_prefix('*') {
                if !after_em.starts_with('*') {
                    if let Some(end) = after_em.find('*') {
                        let (em, tail) = after_em.split_at(end);
                        ui.label(RichText::new(em).italics().size(size).color(color));
                        rest = &tail[1..];
                        continue;
                    }
                }
            }
            if let Some(after_code) = rest.strip_prefix('`') {
                if let Some(end) = after_code.find('`') {
                    let (code, tail) = after_code.split_at(end);
                    ui.label(
                        RichText::new(code)
                            .monospace()
                            .size(size * 0.95)
                            .color(theme::navy_mid())
                            .background_color(theme::panel()),
                    );
                    rest = &tail[1..];
                    continue;
                }
            }

            // 取到下一个特殊标记前的普通文本
            let next_bold = rest.find("**").unwrap_or(rest.len());
            let next_em = find_single_star(rest).unwrap_or(rest.len());
            let next_code = rest.find('`').unwrap_or(rest.len());
            let cut = next_bold.min(next_em).min(next_code).max(1);
            // 若以 * 或 ` 开头但不成对，当作普通字符吃掉一个
            let cut = if rest.starts_with("**") || rest.starts_with('*') || rest.starts_with('`')
            {
                1
            } else {
                cut
            };
            let (chunk, tail) = rest.split_at(cut);
            ui.label(RichText::new(chunk).size(size).color(color));
            rest = tail;
        }
    });
}

fn find_single_star(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            let prev_star = i > 0 && bytes[i - 1] == b'*';
            let next_star = i + 1 < bytes.len() && bytes[i + 1] == b'*';
            if !prev_star && !next_star {
                return Some(i);
            }
            if next_star {
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    None
}
