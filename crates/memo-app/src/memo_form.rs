//! 新建/编辑备忘共用表单（分类下拉 + 标题/日期/优先级/可见性 + 标签 + 备注）。

use crate::date_field;
use crate::doc_editor::{self, Doc};
use crate::theme;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::store::{MemoCategory, MemoPriority, MemoVisibility};

/// 表单可选分类（含统一「性别私密」）。
pub fn form_categories() -> [MemoCategory; 8] {
    [
        MemoCategory::Todo,
        MemoCategory::Work,
        MemoCategory::Credentials,
        MemoCategory::Life,
        MemoCategory::Finance,
        MemoCategory::Emergency,
        MemoCategory::Inspiration,
        MemoCategory::GenderPrivate,
    ]
}

/// 通用跨分类快捷标签。
const COMMON_TAGS: &[&str] = &["重要", "稍后", "已办", "跟进"];

/// 当前分类下的预设标签。
fn preset_tags_for(cat: MemoCategory) -> &'static [&'static str] {
    match cat {
        MemoCategory::Todo => &["待办", "提醒", "截止", "复盘", "清单"],
        MemoCategory::Work | MemoCategory::Office => {
            &["工作", "学习", "会议", "项目", "汇报", "培训", "周报"]
        }
        MemoCategory::Credentials => &["账号", "密码", "证件", "卡证", "证书", "到期", "双因素"],
        MemoCategory::Life => &["家庭", "生活", "购物", "出行", "社交", "家务", "孩子"],
        MemoCategory::Finance => &["财务", "订阅", "账单", "报销", "理财", "税费", "工资"],
        MemoCategory::Emergency => &["应急", "紧急", "联系人", "备用", "急救", "疏散"],
        MemoCategory::Inspiration => &["灵感", "想法", "收藏", "草稿", "摘录", "读书"],
        MemoCategory::GenderPrivate | MemoCategory::WomenPrivate | MemoCategory::MalePrivate => {
            &["健康", "经期", "体检", "用药", "日记", "私密", "伴侣"]
        }
        MemoCategory::General => &["备忘", "随手记", "杂项"],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormMode {
    Create,
    Edit,
}

#[derive(Debug, Clone)]
pub struct MemoFormState {
    pub category: MemoCategory,
    pub due_date: String,
    pub remind_before_days: u32,
    pub priority: MemoPriority,
    pub tags: String,
    pub doc: Doc,
    pub done: bool,
    pub visibility: MemoVisibility,
    /// 上次自动套用的标题/正文，用于判断是否可随分类刷新模板
    auto_title: String,
    auto_body: String,
}

impl Default for MemoFormState {
    fn default() -> Self {
        Self {
            category: MemoCategory::Todo,
            due_date: String::new(),
            remind_before_days: 0,
            priority: MemoPriority::Normal,
            tags: String::new(),
            doc: Doc::empty(),
            done: false,
            visibility: MemoVisibility::Private,
            auto_title: String::new(),
            auto_body: String::new(),
        }
    }
}

impl MemoFormState {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// 按导航默认分类打开新建草稿并套用模板。
    /// `due_date` 默认空（永久）；仅「今天到期」导航传入预填到期。
    pub fn begin_create(category: MemoCategory, stamp: &str, prefill_due: Option<&str>) -> Self {
        let mut s = Self {
            category,
            due_date: prefill_due.unwrap_or("").to_string(),
            ..Self::default()
        };
        s.apply_template(stamp, true);
        s
    }

    pub fn load_from_drafts(
        category: MemoCategory,
        title: &str,
        body: &str,
        due: &str,
        priority: MemoPriority,
        tags: &str,
        done: bool,
        visibility: MemoVisibility,
        remind_before_days: u32,
    ) -> Self {
        Self {
            category,
            due_date: due.to_string(),
            remind_before_days,
            priority,
            tags: tags.to_string(),
            doc: Doc::from_store(title, body),
            done,
            visibility,
            auto_title: title.to_string(),
            auto_body: body.to_string(),
        }
    }

    fn current_store(&self) -> (String, String) {
        self.doc.to_store("未命名备忘")
    }

    fn is_pristine_template(&self) -> bool {
        let (t, b) = self.current_store();
        (t.trim().is_empty() && b.trim().is_empty())
            || (t.trim() == self.auto_title.trim() && b.trim() == self.auto_body.trim())
    }

    pub fn apply_template(&mut self, stamp: &str, force: bool) {
        if !force && !self.is_pristine_template() {
            return;
        }
        let note = doc_editor::memo_template_for(self.category, stamp);
        let (title, body) = doc_editor::split_note(&note, "未命名备忘");
        self.doc = Doc::from_store(&title, &body);
        self.auto_title = title;
        self.auto_body = body;
        // 到期：新建默认永久；编辑保留用户原值，套模板不改动。
    }

    pub fn set_category(&mut self, cat: MemoCategory, stamp: &str) {
        if self.category == cat {
            return;
        }
        self.category = cat;
        self.apply_template(stamp, false);
        if cat.is_gender_private() {
            self.visibility = MemoVisibility::Private;
        }
    }
}

fn today_ymd() -> String {
    chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string()
}

fn category_icon(cat: MemoCategory) -> &'static str {
    match cat {
        MemoCategory::Todo => "✅",
        MemoCategory::Work | MemoCategory::Office => "💼",
        MemoCategory::Credentials => "🔑",
        MemoCategory::Life => "🏠",
        MemoCategory::Finance => "💰",
        MemoCategory::Emergency => "🚨",
        MemoCategory::Inspiration => "💡",
        MemoCategory::GenderPrivate | MemoCategory::WomenPrivate | MemoCategory::MalePrivate => "🔒",
        MemoCategory::General => "📋",
    }
}

fn category_desc(cat: MemoCategory) -> &'static str {
    match cat {
        MemoCategory::Todo => "待办、提醒与截止日期",
        MemoCategory::Work | MemoCategory::Office => "工作、学习与项目记录",
        MemoCategory::Credentials => "账号、密码、证件与卡证",
        MemoCategory::Life => "家庭、出行与日常记事",
        MemoCategory::Finance => "账单、订阅与收支",
        MemoCategory::Emergency => "应急联系人与预案",
        MemoCategory::Inspiration => "想法、摘录与草稿",
        MemoCategory::GenderPrivate | MemoCategory::WomenPrivate | MemoCategory::MalePrivate => {
            "加密私密：健康与日记"
        }
        MemoCategory::General => "未归类随手记",
    }
}

fn title_hint(cat: MemoCategory) -> &'static str {
    match cat {
        MemoCategory::Todo => "一句话说清要做什么",
        MemoCategory::Credentials => "例如：某网站账号 / 身份证",
        MemoCategory::Work | MemoCategory::Office => "工作或学习主题",
        MemoCategory::Life => "生活记事标题",
        MemoCategory::Finance => "账单 / 订阅名称",
        MemoCategory::GenderPrivate | MemoCategory::WomenPrivate | MemoCategory::MalePrivate => {
            "私密备注标题"
        }
        MemoCategory::Emergency => "紧急情况一句话",
        MemoCategory::Inspiration => "灵感一句话",
        MemoCategory::General => "备忘标题",
    }
}

fn parse_tags(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c == '，' || c == ';' || c == '；' || c.is_whitespace())
        .map(|t| t.trim().trim_start_matches('#').trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn format_tags(tags: &[String]) -> String {
    tags.iter()
        .map(|t| {
            if t.starts_with('#') {
                t.clone()
            } else {
                format!("#{t}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn append_tag(tags_str: &mut String, tag: &str) {
    let mut tags = parse_tags(tags_str);
    let t = tag.trim().trim_start_matches('#').to_string();
    if t.is_empty() {
        return;
    }
    if !tags.iter().any(|x| x == &t) {
        tags.push(t);
    }
    *tags_str = format_tags(&tags);
}

/// 导航 → 新建默认分类。
pub fn default_category_for_nav(nav_is_due_today: bool, nav_category: Option<MemoCategory>) -> MemoCategory {
    if nav_is_due_today {
        return MemoCategory::Todo;
    }
    match nav_category {
        Some(MemoCategory::General) | None => MemoCategory::Todo,
        Some(c) if c.is_gender_private() => MemoCategory::GenderPrivate,
        Some(c) => c.canonical(),
    }
}

fn field_label(ui: &mut egui::Ui, text: &str, required: bool) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(text)
                .size(13.0)
                .strong()
                .color(theme::text()),
        );
        if required {
            ui.label(RichText::new("*").size(13.0).color(theme::danger()));
        }
    });
}

fn show_category_combo(
    ui: &mut egui::Ui,
    id_salt: &str,
    cats: &[MemoCategory; 8],
    selected_cat: MemoCategory,
    stamp: &str,
    state: &mut MemoFormState,
) {
    let popup_id = ui.make_persistent_id(("memo_cat_combo", id_salt));
    let w = ui.available_width();
    let h = 34.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fill = if resp.hovered() {
        theme::c().list_hover
    } else {
        theme::card()
    };
    ui.painter().rect(
        rect,
        Rounding::same(8.0),
        fill,
        Stroke::new(1.0, theme::border()),
    );

    let pad = 10.0;
    let icon_slot = 28.0;
    let cy = rect.center().y;
    let icon_g = theme::layout_galley(
        ui,
        category_icon(selected_cat),
        egui::FontId::proportional(19.5),
        theme::category_icon_color(selected_cat),
    );
    let label_g = theme::layout_galley(
        ui,
        selected_cat.label(),
        egui::FontId::proportional(13.0),
        theme::text(),
    );
    ui.painter().galley(
        theme::galley_pos_center(
            egui::pos2(rect.left() + pad + icon_slot * 0.5, cy),
            &icon_g,
        ),
        icon_g,
        theme::category_icon_color(selected_cat),
    );
    ui.painter().galley(
        theme::galley_pos_left_center(
            egui::pos2(rect.left() + pad + icon_slot + 8.0, cy),
            &label_g,
        ),
        label_g,
        theme::text(),
    );
    ui.painter().text(
        egui::pos2(rect.right() - 14.0, cy),
        egui::Align2::CENTER_CENTER,
        "▾",
        egui::FontId::proportional(11.0),
        theme::text_muted(),
    );

    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }
    egui::popup_below_widget(ui, popup_id, &resp, |ui| {
        ui.set_min_width(w);
        ui.spacing_mut().item_spacing.y = 2.0;
        for &cat in cats {
            if category_option_row(ui, cat, selected_cat == cat).clicked() {
                state.set_category(cat, stamp);
                ui.memory_mut(|m| m.close_popup());
            }
        }
    });
}

fn category_option_row(ui: &mut egui::Ui, cat: MemoCategory, selected: bool) -> egui::Response {
    let w = ui.available_width().max(200.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 50.0), Sense::click());
    let hovered = resp.hovered();
    if hovered || selected {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fill = if selected {
        theme::shell_accent_soft()
    } else if hovered {
        theme::c().list_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect(rect, Rounding::same(6.0), fill, Stroke::NONE);

    let fg = if selected {
        theme::shell_accent()
    } else {
        theme::text()
    };
    let icon_font = egui::FontId::proportional(24.0);
    let title_font = egui::FontId::proportional(13.0);
    let desc_font = egui::FontId::proportional(11.0);
    let icon_g = theme::layout_galley(
        ui,
        category_icon(cat),
        icon_font,
        theme::category_icon_color(cat),
    );
    let title_g = theme::layout_galley(ui, cat.label(), title_font, fg);
    let desc_g = theme::layout_galley(ui, category_desc(cat), desc_font, theme::text_muted());
    let icon_slot = 32.0_f32.max(icon_g.mesh_bounds.width());
    let title_h = title_g.mesh_bounds.height().max(title_g.size().y);
    let desc_h = desc_g.mesh_bounds.height().max(desc_g.size().y);
    let gap = 2.0;
    let text_h = title_h + gap + desc_h;
    let cy = rect.center().y;
    let text_x = rect.left() + 10.0 + icon_slot + 8.0;
    let text_top = cy - text_h * 0.5;
    ui.painter().galley(
        theme::galley_pos_center(
            egui::pos2(rect.left() + 10.0 + icon_slot * 0.5, cy),
            &icon_g,
        ),
        icon_g,
        theme::category_icon_color(cat),
    );
    ui.painter().galley(
        theme::galley_pos_left_center(
            egui::pos2(text_x, text_top + title_h * 0.5),
            &title_g,
        ),
        title_g,
        fg,
    );
    ui.painter().galley(
        theme::galley_pos_left_center(
            egui::pos2(text_x, text_top + title_h + gap + desc_h * 0.5),
            &desc_g,
        ),
        desc_g,
        theme::text_muted(),
    );
    resp
}

/// 绘制表单主体（不含底部取消/保存按钮）。
pub fn show_fields(
    ui: &mut egui::Ui,
    state: &mut MemoFormState,
    mode: FormMode,
    id_salt: &str,
) {
    let stamp = today_ymd();
    let cats = form_categories();
    let gender_locked = state.category.is_gender_private();
    if gender_locked {
        state.visibility = MemoVisibility::Private;
    }

    // —— 头 ——
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
        ui.painter()
            .circle_filled(icon_rect.center(), 13.0, theme::shell_accent());
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            if mode == FormMode::Create { "+" } else { "✎" },
            egui::FontId::proportional(15.0),
            Color32::WHITE,
        );
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(
                RichText::new(if mode == FormMode::Create {
                    "新建备忘"
                } else {
                    "编辑备忘"
                })
                .size(17.0)
                .strong()
                .color(theme::text()),
            );
            ui.label(
                RichText::new(if mode == FormMode::Create {
                    "填写后保存，将出现在对应分类下"
                } else {
                    "修改后保存即可更新本条备忘"
                })
                .size(12.0)
                .color(theme::text_muted()),
            );
        });
    });
    ui.add_space(10.0);

    // —— 分类 ——
    field_label(ui, "选择分类", true);
    ui.add_space(3.0);
    let selected_cat = if state.category.is_gender_private() {
        MemoCategory::GenderPrivate
    } else {
        state.category.canonical()
    };
    show_category_combo(ui, id_salt, &cats, selected_cat, &stamp, state);
    ui.label(
        RichText::new(category_desc(selected_cat))
            .size(11.0)
            .color(theme::text_muted()),
    );
    if gender_locked {
        ui.label(
            RichText::new("ⓘ 性别私密强制加密，仅本人可见")
                .size(11.5)
                .color(theme::text_muted()),
        );
    }
    ui.add_space(8.0);

    // —— 标题 ——
    field_label(ui, "标题", true);
    ui.add_space(3.0);
    ui.add(
        egui::TextEdit::singleline(&mut state.doc.title)
            .desired_width(ui.available_width())
            .hint_text(theme::hint(title_hint(state.category)))
            .margin(egui::vec2(10.0, 7.0)),
    );
    ui.add_space(8.0);

    // —— 到期时间 | 优先级 ——
    let half = (ui.available_width() - 12.0) * 0.5;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(half);
            field_label(ui, "到期时间", false);
            ui.add_space(3.0);
            date_field::show_datetime(ui, &format!("{id_salt}_due"), &mut state.due_date, true);
            ui.label(
                RichText::new("不填=永久有效")
                    .size(11.0)
                    .color(theme::text_muted()),
            );
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(half);
            field_label(ui, "优先级", false);
            ui.add_space(3.0);
            egui::ComboBox::from_id_source(format!("{id_salt}_prio"))
                .width(ui.available_width().max(80.0))
                .selected_text(state.priority.label())
                .show_ui(ui, |ui| {
                    for &p in MemoPriority::ALL {
                        ui.selectable_value(&mut state.priority, p, p.label());
                    }
                });
        });
    });
    if !state.due_date.trim().is_empty() {
        ui.add_space(6.0);
        field_label(ui, "提前提醒", false);
        ui.add_space(3.0);
        let remind_label = match state.remind_before_days {
            0 => "到期时",
            1 => "提前 1 天",
            3 => "提前 3 天",
            7 => "提前 7 天",
            n => {
                // keep custom
                let _ = n;
                "到期时"
            }
        };
        if ![0, 1, 3, 7].contains(&state.remind_before_days) {
            state.remind_before_days = 0;
        }
        egui::ComboBox::from_id_source(format!("{id_salt}_remind"))
            .width(ui.available_width().min(220.0).max(120.0))
            .selected_text(remind_label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.remind_before_days, 0, "到期时");
                ui.selectable_value(&mut state.remind_before_days, 1, "提前 1 天");
                ui.selectable_value(&mut state.remind_before_days, 3, "提前 3 天");
                ui.selectable_value(&mut state.remind_before_days, 7, "提前 7 天");
            });
        ui.label(
            RichText::new("需保持程序运行；完全退出后无法弹系统通知")
                .size(11.0)
                .color(theme::text_muted()),
        );
    } else {
        state.remind_before_days = 0;
    }
    ui.add_space(8.0);

    // —— 可见性 ——
    field_label(ui, "可见性", false);
    ui.add_space(3.0);
    Frame::none()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!gender_locked, |ui| {
                    ui.radio_value(&mut state.visibility, MemoVisibility::Private, "私密");
                    ui.radio_value(&mut state.visibility, MemoVisibility::Public, "公开");
                });
                if gender_locked {
                    ui.label(
                        RichText::new("性别私密固定为私密")
                            .size(11.5)
                            .color(theme::text_muted()),
                    );
                }
            });
        });
    ui.add_space(8.0);

    // —— 标签 ——
    field_label(ui, "标签", false);
    ui.add_space(3.0);
    let tag_resp = ui.add(
        egui::TextEdit::singleline(&mut state.tags)
            .desired_width(ui.available_width())
            .hint_text(theme::hint("逗号或空格分隔；可点下方预设"))
            .margin(egui::vec2(10.0, 7.0)),
    );
    if tag_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        let tags = parse_tags(&state.tags);
        state.tags = format_tags(&tags);
    }
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
        ui.spacing_mut().button_padding = egui::vec2(6.0, 1.5);
        ui.label(
            RichText::new("预设")
                .size(11.0)
                .color(theme::text_muted()),
        );
        let cat_tags = preset_tags_for(state.category);
        for t in cat_tags.iter().chain(COMMON_TAGS.iter()) {
            let chip = ui.add(
                egui::Button::new(
                    RichText::new(*t)
                        .size(11.0)
                        .color(theme::shell_accent()),
                )
                .fill(theme::shell_tag_bg())
                .stroke(Stroke::NONE)
                .rounding(Rounding::same(6.0))
                .min_size(Vec2::ZERO),
            );
            if chip.clicked() {
                append_tag(&mut state.tags, t);
            }
        }
    });
    ui.add_space(6.0);

    if mode == FormMode::Edit {
        ui.checkbox(&mut state.done, "已完成");
        ui.add_space(6.0);
    }

    // —— 备注（放最后，高度克制，不挡住上方选项）——
    field_label(ui, "备注（可选）", false);
    ui.add_space(3.0);
    Frame::none()
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::same(8.0))
        .show(ui, |ui| {
            doc_editor::show_editor(ui, &mut state.doc, id_salt);
        });
}

pub fn tags_vec(state: &MemoFormState) -> Vec<String> {
    parse_tags(&state.tags)
}

pub fn validate_title(state: &MemoFormState) -> Result<(String, String), String> {
    let (title, body) = state.current_store();
    if title.trim().is_empty() {
        Err("请填写标题".into())
    } else {
        Ok((title, body))
    }
}
