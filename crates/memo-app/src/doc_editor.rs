//! 备忘 / 任务计划共用的纯 TXT 正文编辑器。
//! 落盘为 UTF-8 字符串；旧 `memo-doc/v1` 在装入时拍平。

use crate::theme;
use eframe::egui::{self, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::memo_doc;
use memo_core::store::MemoCategory;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub title: String,
    pub body: String,
}

impl Doc {
    pub fn empty() -> Self {
        Self {
            title: String::new(),
            body: String::new(),
        }
    }

    /// 从已拆分的 title/content 装入（旧富文本会拍成纯 TXT）。
    pub fn from_store(title: &str, content: &str) -> Self {
        Self {
            title: title.to_string(),
            body: memo_doc::to_plain(content),
        }
    }

    pub fn to_store(&self, default_title: &str) -> (String, String) {
        let title = if self.title.trim().is_empty() {
            default_title.to_string()
        } else {
            self.title.trim().to_string()
        };
        (title, self.body.clone())
    }
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

/// 序列化：标题一行 + 纯 TXT 正文（供模板 pristine 比较）。
pub fn serialize(doc: &Doc) -> String {
    let title = doc.title.trim();
    let body = doc.body.as_str();
    if body.trim().is_empty() {
        title.to_string()
    } else if title.is_empty() {
        body.to_string()
    } else {
        format!("{title}\n{body}")
    }
}

fn fit_ui_width(ui: &egui::Ui) -> f32 {
    crate::page_shell::fit_avail_width(ui, ui.available_width())
}

/// 编辑器。`min_h` 为整卡最小高度；正文可无限撑开，由外层滚动。
/// `data_dir` / `memo_id` 保留签名兼容调用方（纯 TXT 不再插图）。
pub fn show_editor(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    id_salt: &str,
    _data_dir: Option<&Path>,
    _memo_id: &str,
    min_h: f32,
) {
    let min_h = min_h.max(140.0);
    let avail = fit_ui_width(ui);
    if avail < 24.0 {
        ui.allocate_exact_size(Vec2::new(avail, min_h.min(48.0)), Sense::hover());
        return;
    }

    let editor_w = avail;
    Frame::none()
        .fill(theme::card())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::ZERO)
        .show(ui, |ui| {
            let editor_w = fit_ui_width(ui).min(editor_w);
            ui.set_max_width(editor_w);
            ui.set_min_height(min_h);
            // 只限最大宽，避免描边后 set_width(满宽) 超父级退进程
            ui.set_width(editor_w.min(ui.available_width().max(1.0)));

            Frame::none()
                .fill(theme::card())
                .inner_margin(Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    let inner_w = fit_ui_width(ui);
                    ui.set_max_width(inner_w);
                    let edit_w = (inner_w - 1.0).max(1.0).min(ui.available_width().max(1.0));
                    let rows = body_desired_rows(&doc.body, min_h);
                    ui.add(
                        egui::TextEdit::multiline(&mut doc.body)
                            .id_source(("doc_plain_body", id_salt))
                            .desired_width(edit_w)
                            .desired_rows(rows)
                            .frame(false)
                            .hint_text(theme::hint("在此填写备注…")),
                    );
                });
        });
}

fn body_desired_rows(body: &str, min_h: f32) -> usize {
    let lines = body.lines().count().max(1);
    // ~18px/行，保证至少铺满纸面最小高度
    let min_rows = ((min_h - 24.0) / 18.0).ceil() as usize;
    lines.max(min_rows).max(4).min(200)
}

/// 只读正文（标题由详情顶栏展示）。
#[allow(dead_code)]
pub fn show_viewer_body(ui: &mut egui::Ui, content: &str) {
    show_viewer_body_ex(ui, content, None, "");
}

pub fn show_viewer_body_ex(
    ui: &mut egui::Ui,
    content: &str,
    _data_dir: Option<&Path>,
    _memo_id: &str,
) {
    let plain = memo_doc::to_plain(content);
    if plain.trim().is_empty() {
        ui.label(theme::muted_label("（正文为空）").size(15.0));
        return;
    }
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_max_width(ui.available_width());
            ui.add(
                egui::Label::new(RichText::new(plain).size(15.0).color(theme::text())).wrap(true),
            );
        },
    );
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
    format!("{t}\n\n（工作说明）\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use memo_core::memo_doc::{self, Block, ListKind, Span};
    use memo_core::store::MemoCategory;

    #[test]
    fn memo_template_is_title_only() {
        let note = memo_template("2026-09-30");
        let (title, body) = split_note(&note, "未命名备忘");
        let doc = Doc::from_store(&title, &body);
        assert_eq!(doc.title, "工作学习 2026-09-30");
        assert!(doc.body.is_empty());
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
    fn plain_roundtrip_store() {
        let mut doc = Doc::empty();
        doc.title = "标题".into();
        doc.body = "第一行\n第二行".into();
        let (t, c) = doc.to_store("未命名");
        assert_eq!(t, "标题");
        assert_eq!(c, "第一行\n第二行");
        let again = Doc::from_store(&t, &c);
        assert_eq!(again.title, "标题");
        assert_eq!(again.body, "第一行\n第二行");
        assert!(!c.starts_with(memo_doc::MAGIC));
    }

    #[test]
    fn from_store_flattens_memo_doc() {
        let blocks = vec![
            Block::Paragraph {
                spans: vec![Span {
                    text: "你好".into(),
                    color: Some("#C62828".into()),
                    link: Some("https://x".into()),
                    bold: true,
                    italic: false,
                    underline: false,
                    size: None,
                }],
            },
            Block::Items {
                items: vec!["一".into()],
                list_kind: ListKind::Bullet,
            },
        ];
        let rich = memo_doc::serialize_body(&blocks);
        let doc = Doc::from_store("T", &rich);
        assert_eq!(doc.body, "你好\n\n- 一");
        let (_t, saved) = doc.to_store("T");
        assert_eq!(saved, "你好\n\n- 一");
    }
}
