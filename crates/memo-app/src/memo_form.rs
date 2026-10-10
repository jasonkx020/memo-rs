//! 新建/编辑备忘共用表单（分类下拉 + 标题/日期/优先级/可见性 + 标签 + 备注）。

use crate::date_field;
use crate::doc_editor::{self, Doc};
use crate::theme;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::store::{MemoCategory, MemoPriority, MemoVisibility};

/// 新建弹窗作用域：随主导航收窄分类/类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormScope {
    /// 编辑：不限制
    #[default]
    Open,
    Calendar,
    Credentials,
    GenderPrivate,
}

pub fn categories_for(scope: FormScope) -> &'static [MemoCategory] {
    match scope {
        FormScope::Open => &[
            MemoCategory::Todo,
            MemoCategory::Work,
            MemoCategory::Credentials,
            MemoCategory::Life,
            MemoCategory::Finance,
            MemoCategory::Emergency,
            MemoCategory::Inspiration,
            MemoCategory::GenderPrivate,
        ],
        FormScope::Calendar => &[
            MemoCategory::Todo,
            MemoCategory::Work,
            MemoCategory::Life,
            MemoCategory::Finance,
            MemoCategory::Emergency,
            MemoCategory::Inspiration,
        ],
        FormScope::Credentials => &[MemoCategory::Credentials],
        FormScope::GenderPrivate => &[MemoCategory::GenderPrivate],
    }
}

pub fn kinds_for(scope: FormScope) -> &'static [DateKind] {
    match scope {
        FormScope::Open => &[
            DateKind::Day,
            DateKind::Span,
            DateKind::Parked,
            DateKind::Credentials,
        ],
        FormScope::Calendar | FormScope::GenderPrivate => {
            &[DateKind::Day, DateKind::Span, DateKind::Parked]
        }
        FormScope::Credentials => &[DateKind::Credentials],
    }
}

fn category_allowed(scope: FormScope, cat: MemoCategory) -> bool {
    let c = if cat.is_gender_private() {
        MemoCategory::GenderPrivate
    } else {
        cat.canonical()
    };
    categories_for(scope).iter().any(|&x| {
        if x.is_gender_private() {
            c.is_gender_private()
        } else {
            x.canonical() == c
        }
    })
}

fn kind_allowed(scope: FormScope, kind: DateKind) -> bool {
    kinds_for(scope).contains(&kind)
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

/// 表单「这是」：某一天 / 好几天 / 先记着 / 账号密码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateKind {
    Day,
    Span,
    Parked,
    Credentials,
}

impl DateKind {
    fn label(self) -> &'static str {
        match self {
            Self::Day => "某一天",
            Self::Span => "好几天",
            Self::Parked => "先记着",
            Self::Credentials => "账号密码",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoFormState {
    pub category: MemoCategory,
    pub due_date: String,
    pub end_date: String,
    pub kind: DateKind,
    pub remind_before_days: u32,
    pub priority: MemoPriority,
    pub tags: String,
    pub doc: Doc,
    pub done: bool,
    pub visibility: MemoVisibility,
    /// 新建弹窗导航作用域；编辑为 Open
    pub scope: FormScope,
    /// 下一帧聚焦标题框（校验失败时）
    focus_title: bool,
    /// 外层 ScrollArea 强制滚到顶部的剩余帧数（标题在表单上部）
    force_scroll_top: u8,
    /// 标题输入框闪烁提示剩余帧数
    flash_title_frames: u8,
    /// 上次自动套用的标题/正文，用于判断是否可随分类刷新模板
    auto_title: String,
    auto_body: String,
}

impl Default for MemoFormState {
    fn default() -> Self {
        Self {
            category: MemoCategory::Todo,
            due_date: String::new(),
            end_date: String::new(),
            kind: DateKind::Parked,
            remind_before_days: 0,
            priority: MemoPriority::Normal,
            tags: String::new(),
            doc: Doc::empty(),
            done: false,
            visibility: MemoVisibility::Private,
            scope: FormScope::Open,
            focus_title: false,
            force_scroll_top: 0,
            flash_title_frames: 0,
            auto_title: String::new(),
            auto_body: String::new(),
        }
    }
}

impl MemoFormState {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// 按导航作用域打开新建草稿并套用模板。
    /// `prefill_due` 有值时为「某一天」；账号分类强制无日期。
    pub fn begin_create(
        category: MemoCategory,
        stamp: &str,
        prefill_due: Option<&str>,
        scope: FormScope,
    ) -> Self {
        let category = if category_allowed(scope, category) {
            category
        } else {
            categories_for(scope)
                .first()
                .copied()
                .unwrap_or(MemoCategory::Todo)
        };
        let mut s = Self {
            category,
            due_date: prefill_due.unwrap_or("").to_string(),
            scope,
            ..Self::default()
        };
        s.kind = infer_kind(s.category, &s.due_date, &s.end_date);
        if !kind_allowed(scope, s.kind) {
            s.kind = kinds_for(scope)
                .first()
                .copied()
                .unwrap_or(DateKind::Parked);
        }
        s.sync_kind_fields();
        s.apply_template(stamp, true);
        s
    }

    pub fn load_from_drafts(
        category: MemoCategory,
        title: &str,
        body: &str,
        due: &str,
        end: &str,
        priority: MemoPriority,
        tags: &str,
        done: bool,
        visibility: MemoVisibility,
        remind_before_days: u32,
    ) -> Self {
        let mut s = Self {
            category,
            due_date: due.to_string(),
            end_date: end.to_string(),
            kind: infer_kind(category, due, end),
            remind_before_days,
            priority,
            tags: tags.to_string(),
            doc: Doc::from_store(title, body),
            done,
            visibility,
            scope: FormScope::Open,
            focus_title: false,
            force_scroll_top: 0,
            flash_title_frames: 0,
            auto_title: title.to_string(),
            auto_body: body.to_string(),
        };
        s.sync_kind_fields();
        s
    }

    fn sync_kind_fields(&mut self) {
        match self.kind {
            DateKind::Credentials => {
                self.category = MemoCategory::Credentials;
                self.due_date.clear();
                self.end_date.clear();
                self.visibility = MemoVisibility::Private;
                self.remind_before_days = 0;
            }
            DateKind::Parked => {
                self.due_date.clear();
                self.end_date.clear();
                self.remind_before_days = 0;
                // 勿在私密/账号作用域把分类改成 Todo
                if self.category.canonical() == MemoCategory::Credentials
                    && self.scope != FormScope::Credentials
                    && self.scope != FormScope::GenderPrivate
                {
                    self.category = MemoCategory::Todo;
                }
            }
            DateKind::Day => {
                self.end_date.clear();
                if self.due_date.trim().is_empty() {
                    self.due_date = format!("{} 09:00", today_ymd());
                }
                if self.category.canonical() == MemoCategory::Credentials
                    && self.scope != FormScope::Credentials
                    && self.scope != FormScope::GenderPrivate
                {
                    self.category = MemoCategory::Todo;
                }
            }
            DateKind::Span => {
                if self.due_date.trim().is_empty() {
                    self.due_date = format!("{} 09:00", today_ymd());
                }
                if self.end_date.trim().is_empty() {
                    if let Some(d) = date_field::parse_ymd(&self.due_date) {
                        self.end_date = date_field::format_ymd(d);
                    }
                }
                if self.category.canonical() == MemoCategory::Credentials
                    && self.scope != FormScope::Credentials
                    && self.scope != FormScope::GenderPrivate
                {
                    self.category = MemoCategory::Todo;
                }
            }
        }
        // 作用域内强制合法分类
        if !category_allowed(self.scope, self.category) {
            if let Some(&c) = categories_for(self.scope).first() {
                self.category = c;
            }
        }
    }

    pub fn set_kind(&mut self, kind: DateKind, stamp: &str) {
        if self.kind == kind {
            return;
        }
        if !kind_allowed(self.scope, kind) {
            return;
        }
        self.kind = kind;
        if kind == DateKind::Credentials && self.category.canonical() != MemoCategory::Credentials
        {
            self.set_category(MemoCategory::Credentials, stamp);
            return;
        }
        self.sync_kind_fields();
    }

    fn current_store(&self) -> (String, String) {
        self.doc.to_store("未命名备忘")
    }

    fn is_pristine_template(&self) -> bool {
        let title = self.doc.title.trim();
        let serialized = doc_editor::serialize(&self.doc);
        // 标题为空时 serialize 即为正文，不可再 split（否则会把正文首行当成标题）
        let body = if title.is_empty() {
            serialized.trim().to_string()
        } else {
            doc_editor::split_note(&serialized, "")
                .1
                .trim()
                .to_string()
        };
        let auto_body = self.auto_body.trim();
        (title.is_empty() && body.is_empty())
            || (title == self.auto_title.trim() && body == auto_body)
    }

    pub fn apply_template(&mut self, stamp: &str, force: bool) {
        if !force && !self.is_pristine_template() {
            return;
        }
        let note = doc_editor::memo_template_for(self.category, stamp);
        // 模板首行不再写入标题，留给用户填写；仅套用正文
        let (_tpl_title, body) = doc_editor::split_note(&note, "");
        // 必须用 from_store("", body)：旧 join+parse 会把正文首行当成标题
        self.doc = Doc::from_store("", &body);
        self.auto_title = String::new();
        self.auto_body = body;
    }

    /// 保存校验失败时：滚到标题、聚焦，并闪烁提示。
    pub fn request_title_focus(&mut self) {
        self.focus_title = true;
        // 多帧重试：外层 ScrollArea 在下一帧才能应用 offset；动画也可能要几帧
        self.force_scroll_top = 12;
        // ~3 次亮暗（每相约 8 帧切换），约 0.8s@60fps
        self.flash_title_frames = 48;
    }

    /// 外层 ScrollArea 是否应强制滚到顶部（标题附近）。
    pub fn force_scroll_top(&self) -> bool {
        self.force_scroll_top > 0
    }

    /// 消耗一帧强制滚顶计数。
    pub fn tick_force_scroll_top(&mut self) {
        self.force_scroll_top = self.force_scroll_top.saturating_sub(1);
    }

    fn take_focus_title(&mut self) -> bool {
        let f = self.focus_title;
        self.focus_title = false;
        f
    }

    pub fn set_category(&mut self, cat: MemoCategory, stamp: &str) {
        if self.category == cat {
            return;
        }
        if !category_allowed(self.scope, cat) {
            return;
        }
        self.category = cat;
        self.apply_template(stamp, false);
        if cat.is_gender_private() || cat.canonical() == MemoCategory::Credentials {
            self.visibility = MemoVisibility::Private;
        }
        if cat.canonical() == MemoCategory::Credentials {
            if kind_allowed(self.scope, DateKind::Credentials) {
                self.kind = DateKind::Credentials;
            }
        } else if self.kind == DateKind::Credentials {
            self.kind = if self.due_date.trim().is_empty() {
                DateKind::Parked
            } else {
                DateKind::Day
            };
            if !kind_allowed(self.scope, self.kind) {
                self.kind = kinds_for(self.scope)
                    .first()
                    .copied()
                    .unwrap_or(DateKind::Parked);
            }
        }
        self.sync_kind_fields();
    }
}

fn infer_kind(category: MemoCategory, due: &str, end: &str) -> DateKind {
    if category.canonical() == MemoCategory::Credentials {
        return DateKind::Credentials;
    }
    if due.trim().is_empty() {
        return DateKind::Parked;
    }
    match (
        memo_core::due_date_part(due),
        memo_core::event_end_date(due, end),
    ) {
        (Some(a), Some(b)) if b > a => DateKind::Span,
        _ => DateKind::Day,
    }
}

fn today_ymd() -> String {
    chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string()
}

pub fn category_icon(cat: MemoCategory) -> &'static str {
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
#[allow(dead_code)]
pub fn default_category_for_nav(nav_category: Option<MemoCategory>) -> MemoCategory {
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

fn form_w(ui: &egui::Ui) -> f32 {
    (ui.available_width() - 8.0).max(0.0)
}

fn show_due_priority(ui: &mut egui::Ui, state: &mut MemoFormState, id_salt: &str, col_w: f32) {
    ui.set_width(col_w);
    ui.set_max_width(col_w);
    field_label(ui, if state.kind == DateKind::Span { "从哪天" } else { "哪一天" }, false);
    ui.add_space(3.0);
    date_field::show_datetime(ui, &format!("{id_salt}_due"), &mut state.due_date, true);
}

fn show_end_date(ui: &mut egui::Ui, state: &mut MemoFormState, id_salt: &str, col_w: f32) {
    ui.set_width(col_w);
    ui.set_max_width(col_w);
    field_label(ui, "到哪天", false);
    ui.add_space(3.0);
    date_field::show(ui, &format!("{id_salt}_end"), &mut state.end_date, true);
}

fn kind_hint(kind: DateKind) -> &'static str {
    match kind {
        DateKind::Day => "出现在选中的那一天。",
        DateKind::Span => "月历上画一条横跨多天的色条。",
        DateKind::Parked => "进右侧「未安排」，以后再放到某一天。",
        DateKind::Credentials => "会放进「账号证件」，只本人可见，不上日历。",
    }
}

fn show_kinds(ui: &mut egui::Ui, state: &mut MemoFormState, stamp: &str) {
    let kinds = kinds_for(state.scope);
    field_label(ui, "这是", true);
    ui.add_space(3.0);
    if kinds.len() <= 1 {
        let k = kinds.first().copied().unwrap_or(state.kind);
        if state.kind != k {
            state.kind = k;
            state.sync_kind_fields();
        }
        ui.horizontal(|ui| {
            let fill = theme::shell_nav_selected();
            ui.add(
                egui::Button::new(
                    RichText::new(k.label())
                        .size(12.5)
                        .color(theme::shell_accent())
                        .strong(),
                )
                .fill(fill)
                .stroke(Stroke::new(1.0, Color32::from_rgb(0xBF, 0xDB, 0xFE)))
                .rounding(Rounding::same(99.0))
                .min_size(Vec2::new(64.0, 26.0)),
            );
            ui.label(
                RichText::new("（当前页固定）")
                    .size(11.0)
                    .color(theme::text_muted()),
            );
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for &k in kinds {
                let on = state.kind == k;
                let fill = if on {
                    theme::shell_nav_selected()
                } else {
                    theme::card()
                };
                let fg = if on {
                    theme::shell_accent()
                } else {
                    theme::text()
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(k.label()).size(12.5).color(fg).strong())
                            .fill(fill)
                            .stroke(Stroke::new(
                                1.0,
                                if on {
                                    Color32::from_rgb(0xBF, 0xDB, 0xFE)
                                } else {
                                    theme::border()
                                },
                            ))
                            .rounding(Rounding::same(99.0))
                            .min_size(Vec2::new(64.0, 26.0)),
                    )
                    .clicked()
                {
                    state.set_kind(k, stamp);
                }
            }
        });
    }
    ui.label(
        RichText::new(kind_hint(state.kind))
            .size(11.0)
            .color(if state.kind == DateKind::Credentials {
                theme::shell_accent()
            } else {
                theme::text_muted()
            }),
    );
}

fn show_priority_field(ui: &mut egui::Ui, state: &mut MemoFormState, id_salt: &str, col_w: f32) {
    ui.set_width(col_w);
    ui.set_max_width(col_w);
    field_label(ui, "优先级", false);
    ui.add_space(3.0);
    egui::ComboBox::from_id_source(format!("{id_salt}_prio"))
        .width((col_w - 6.0).max(80.0))
        .selected_text(state.priority.label())
        .show_ui(ui, |ui| {
            for &p in MemoPriority::ALL {
                ui.selectable_value(&mut state.priority, p, p.label());
            }
        });
}

fn show_category_combo(
    ui: &mut egui::Ui,
    id_salt: &str,
    cats: &[MemoCategory],
    selected_cat: MemoCategory,
    stamp: &str,
    state: &mut MemoFormState,
) {
    let locked = cats.len() <= 1;
    let popup_id = ui.make_persistent_id(("memo_cat_combo", id_salt));
    let w = form_w(ui);
    let h = 34.0;
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(w, h),
        if locked {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    if !locked && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fill = if !locked && resp.hovered() {
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
    if locked {
        ui.painter().text(
            egui::pos2(rect.right() - 14.0, cy),
            egui::Align2::CENTER_CENTER,
            "·",
            egui::FontId::proportional(11.0),
            theme::text_muted(),
        );
    } else {
        ui.painter().text(
            egui::pos2(rect.right() - 14.0, cy),
            egui::Align2::CENTER_CENTER,
            "v",
            egui::FontId::proportional(11.0),
            theme::text_muted(),
        );
    }

    if locked {
        return;
    }
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
    // 编辑用完整列表；新建用 state.scope
    let scope = if mode == FormMode::Edit {
        FormScope::Open
    } else {
        state.scope
    };
    let cats = categories_for(scope);
    let gender_locked = state.category.is_gender_private();
    let cred_locked = state.kind == DateKind::Credentials
        || state.category.canonical() == MemoCategory::Credentials;
    if gender_locked || cred_locked {
        state.visibility = MemoVisibility::Private;
    }
    let w = form_w(ui);
    ui.set_max_width(w);

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
    let need_focus = state.take_focus_title();
    let need_scroll = state.force_scroll_top > 0;
    let flash_on = state.flash_title_frames > 0 && ((state.flash_title_frames / 8) % 2 == 0);
    if state.flash_title_frames > 0 {
        state.flash_title_frames = state.flash_title_frames.saturating_sub(1);
        ui.ctx().request_repaint();
    }
    let title_stroke = if flash_on {
        Stroke::new(2.0, theme::danger())
    } else {
        Stroke::new(1.0, theme::border())
    };
    let title_block = ui
        .vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("标题")
                        .size(13.0)
                        .strong()
                        .color(theme::text()),
                );
                ui.label(RichText::new("*").size(13.0).color(theme::danger()));
                ui.label(
                    RichText::new("标题是必填字段，一句话说清楚要做什么")
                        .size(12.0)
                        .color(theme::danger()),
                );
            });
            ui.add_space(3.0);
            let title_id = ui.make_persistent_id((id_salt, "memo_title"));
            let title_resp = Frame::none()
                .stroke(title_stroke)
                .rounding(Rounding::same(8.0))
                .inner_margin(Margin::same(1.0))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.doc.title)
                            .id(title_id)
                            .desired_width((w - 2.0).max(40.0))
                            .hint_text(theme::hint(title_hint(state.category)))
                            .margin(egui::vec2(10.0, 7.0)),
                    )
                })
                .inner;
            if need_focus {
                title_resp.request_focus();
            }
            title_resp
        })
        .response;
    if need_focus || need_scroll {
        // 滚到标题块；外层还会用 vertical_scroll_offset(0) 兜底
        title_block.scroll_to_me(Some(egui::Align::TOP));
        ui.scroll_to_rect(title_block.rect, Some(egui::Align::TOP));
        ui.ctx().request_repaint();
    }
    ui.add_space(8.0);

    // —— 备注 ——
    field_label(ui, "备注（可选）", false);
    ui.add_space(3.0);
    let notes_w = (ui.available_width() - 10.0).max(48.0);
    ui.scope(|ui| {
        ui.set_width(notes_w);
        ui.set_max_width(notes_w);
        Frame::none()
            .stroke(Stroke::new(1.0, theme::border()))
            .rounding(Rounding::same(10.0))
            .inner_margin(Margin::same(8.0))
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                doc_editor::show_editor(ui, &mut state.doc, id_salt);
            });
    });
    ui.add_space(8.0);

    // —— 类型 ——
    show_kinds(ui, state, &stamp);
    ui.add_space(8.0);

    // —— 日期 | 优先级 ——
    let show_dates = matches!(state.kind, DateKind::Day | DateKind::Span);
    let show_end = state.kind == DateKind::Span;
    if show_dates {
        if w >= 360.0 {
            let col = ((w - 12.0) * 0.5).max(80.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    show_due_priority(ui, state, id_salt, col);
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    if show_end {
                        show_end_date(ui, state, id_salt, col);
                    } else {
                        show_priority_field(ui, state, id_salt, col);
                    }
                });
            });
            if show_end {
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    show_priority_field(ui, state, id_salt, w);
                });
            }
        } else {
            ui.vertical(|ui| {
                show_due_priority(ui, state, id_salt, w);
            });
            if show_end {
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    show_end_date(ui, state, id_salt, w);
                });
            }
            ui.add_space(8.0);
            ui.vertical(|ui| {
                show_priority_field(ui, state, id_salt, w);
            });
        }
    } else {
        ui.vertical(|ui| {
            show_priority_field(ui, state, id_salt, w);
        });
    }
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
            .width(w.min(220.0).max(80.0))
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
                ui.add_enabled_ui(!(gender_locked || cred_locked), |ui| {
                    ui.radio_value(&mut state.visibility, MemoVisibility::Private, "私密");
                    ui.radio_value(&mut state.visibility, MemoVisibility::Public, "公开");
                });
                if gender_locked {
                    ui.label(
                        RichText::new("性别私密固定为私密")
                            .size(11.5)
                            .color(theme::text_muted()),
                    );
                } else if cred_locked {
                    ui.label(
                        RichText::new("账号证件固定为私密，不上日历")
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
            .desired_width(w)
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
}

pub fn tags_vec(state: &MemoFormState) -> Vec<String> {
    parse_tags(&state.tags)
}

pub fn validate_title(state: &MemoFormState) -> Result<(String, String), String> {
    // 必须看 doc.title：to_store 在标题为空时会把正文首行当成标题
    if state.doc.title.trim().is_empty() {
        Err("请填写标题".into())
    } else {
        Ok(state.current_store())
    }
}
