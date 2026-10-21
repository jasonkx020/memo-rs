//! 备忘正文自有格式 `memo-doc/v1`（块 + 行内样式 + 插图引用）。

use serde::{Deserialize, Serialize};

pub const MAGIC: &str = "memo-doc/v1";

fn is_false(b: &bool) -> bool {
    !*b
}

/// 列表样式（Word 编号 / 项目符号）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    #[default]
    Numbered,
    Bullet,
}

/// 行内片段：文字 + 颜色/链接 + Word 式字符格式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    /// 字号（磅感，如 10/12/14/16）；缺省跟界面 Body。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u8>,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: None,
            link: None,
            bold: false,
            italic: false,
            underline: false,
            size: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph {
        spans: Vec<Span>,
    },
    Items {
        items: Vec<String>,
        #[serde(default)]
        list_kind: ListKind,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Image {
        file_id: String,
        #[serde(default)]
        alt: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocBody {
    pub blocks: Vec<Block>,
}

impl DocBody {
    pub fn empty() -> Self {
        Self {
            blocks: vec![Block::Paragraph {
                spans: vec![Span::plain("")],
            }],
        }
    }
}

/// 序列化正文（不含标题）。
pub fn serialize_body(blocks: &[Block]) -> String {
    let body = DocBody {
        blocks: blocks.to_vec(),
    };
    match serde_json::to_string(&body) {
        Ok(json) => format!("{MAGIC}\n{json}"),
        Err(_) => String::new(),
    }
}

/// 解析正文：新格式或旧 Markdown 风格。
pub fn parse_body(content: &str) -> DocBody {
    let t = content.trim_start();
    if let Some(rest) = t.strip_prefix(MAGIC) {
        let json = rest.trim_start_matches(['\r', '\n']);
        if let Ok(body) = serde_json::from_str::<DocBody>(json) {
            if body.blocks.is_empty() {
                return DocBody::empty();
            }
            return body;
        }
    }
    // 旧正文：迁成块（段落 / 事项 / 表格），无颜色链接
    let blocks = parse_legacy_blocks(content);
    if blocks.is_empty() {
        DocBody::empty()
    } else {
        DocBody { blocks }
    }
}

/// 收集正文中引用的附件 file_id。
pub fn collect_file_ids(content: &str) -> Vec<String> {
    let body = parse_body(content);
    let mut ids = Vec::new();
    for b in &body.blocks {
        if let Block::Image { file_id, .. } = b {
            if !file_id.is_empty() && !ids.contains(file_id) {
                ids.push(file_id.clone());
            }
        }
    }
    ids
}

fn parse_legacy_blocks(md: &str) -> Vec<Block> {
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    let mut blocks: Vec<Block> = Vec::new();
    let mut para = String::new();

    let flush_para = |para: &mut String, blocks: &mut Vec<Block>| {
        let t = para.trim_end();
        if !t.is_empty() {
            blocks.push(Block::Paragraph {
                spans: vec![Span::plain(t)],
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
            blocks.push(Block::Items {
                items,
                list_kind: ListKind::Numbered,
            });
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
            blocks.push(Block::Items {
                items,
                list_kind: ListKind::Bullet,
            });
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

/// 固定色板（存储为 #RRGGBB）。
pub fn palette() -> &'static [(&'static str, &'static str)] {
    &[
        ("默认", ""),
        ("红", "#C62828"),
        ("橙", "#EF6C00"),
        ("绿", "#2E7D32"),
        ("蓝", "#1565C0"),
        ("紫", "#6A1B9A"),
        ("灰", "#546E7A"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_rich() {
        let blocks = vec![
            Block::Paragraph {
                spans: vec![Span {
                    text: "你好".into(),
                    color: Some("#C62828".into()),
                    link: None,
                    bold: true,
                    italic: false,
                    underline: false,
                    size: Some(14),
                }],
            },
            Block::Image {
                file_id: "abc123.png".into(),
                alt: "图".into(),
            },
        ];
        let s = serialize_body(&blocks);
        assert!(s.starts_with(MAGIC));
        let body = parse_body(&s);
        assert_eq!(body.blocks, blocks);
        assert_eq!(collect_file_ids(&s), vec!["abc123.png".to_string()]);
    }

    #[test]
    fn legacy_paragraph() {
        let body = parse_body("hello\n\n1. a\n2. b\n");
        assert!(matches!(body.blocks[0], Block::Paragraph { .. }));
        assert!(matches!(body.blocks[1], Block::Items { .. }));
    }
}
