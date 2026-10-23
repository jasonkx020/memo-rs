//! 新建/编辑备忘共用表单（分类下拉 + 标题/日期/优先级/可见性 + 标签 + 备注）。

use crate::date_field;
use crate::doc_editor::{self, Doc};
use crate::theme;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Sense, Stroke, Vec2};
use memo_core::store::{MemoCategory, MemoPriority, MemoVisibility};
use std::path::PathBuf;
use uuid::Uuid;

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
        // 备忘页：永久档案类型（无待办 / 无性别私密）；不上日历
        FormScope::Credentials => &[
            MemoCategory::Credentials,
            MemoCategory::Work,
            MemoCategory::Life,
            MemoCategory::Finance,
            MemoCategory::Inspiration,
            MemoCategory::Emergency,
        ],
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
        // 备忘作用域统一「不上日历」；分类由上面的类型选择决定
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

/// 表单「这是」：某一天 / 好几天 / 先记着 / 备忘。
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
            Self::Day => "某天",
            Self::Span => "时段",
            Self::Parked => "先记着",
            Self::Credentials => "不上日历",
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
    /// 附件目录用的备忘 id（新建时预生成，保存时沿用）
    pub attach_id: String,
    /// 数据目录（插图读写）；打开表单时填入
    pub data_dir: Option<PathBuf>,
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
            attach_id: String::new(),
            data_dir: None,
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
            attach_id: Uuid::new_v4().to_string().replace('-', ""),
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
        memo_id: &str,
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
            attach_id: if memo_id.trim().is_empty() {
                Uuid::new_v4().to_string().replace('-', "")
            } else {
                memo_id.to_string()
            },
            data_dir: None,
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
                // 备忘页作用域：保留用户选的类型，仅清日期；账号证件仍强制私密
                if self.scope != FormScope::Credentials {
                    self.category = MemoCategory::Credentials;
                    self.visibility = MemoVisibility::Private;
                } else if self.category.canonical() == MemoCategory::Credentials {
                    self.visibility = MemoVisibility::Private;
                }
                self.due_date.clear();
                self.end_date.clear();
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
        // 备忘页可多类型；其它作用域选「备忘」仍落到账号证件分类
        if kind == DateKind::Credentials
            && self.scope != FormScope::Credentials
            && self.category.canonical() != MemoCategory::Credentials
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
        MemoCategory::Todo => "📌",
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
    field_label_hint(ui, text, required, None);
}

fn field_label_hint(ui: &mut egui::Ui, text: &str, required: bool, hint: Option<&str>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(
            RichText::new(text)
                .size(12.0)
                .strong()
                .color(theme::text()),
        );
        if required {
            ui.label(RichText::new("*").size(12.0).color(theme::danger()));
        }
        if let Some(h) = hint {
            ui.label(
                RichText::new(h)
                    .size(11.0)
                    .color(theme::text_muted()),
            );
        }
    });
}

fn form_w(ui: &egui::Ui) -> f32 {
    // 给滚动条/描边留缝，避免子控件贴右缘或被裁切
    (ui.available_width() - 4.0).max(0.0)
}

/// 表单字段纵向间距（对齐紧凑焦点稿）。
const FIELD_GAP: f32 = 8.0;

fn split_cols(avail: f32, n: usize, gap: f32) -> f32 {
    crate::page_shell::split_col_width(avail, n, gap)
}

fn fit_col_width(ui: &egui::Ui, want: f32) -> f32 {
    crate::page_shell::fit_avail_width(ui, want)
}

/// 事件/备忘编号短码（完整 id 悬停「复制」可见）。
fn short_event_id(id: &str) -> String {
    let t = id.trim();
    if t.is_empty() {
        return String::new();
    }
    t.chars().take(8).collect()
}

fn show_due_priority(ui: &mut egui::Ui, state: &mut MemoFormState, id_salt: &str, col_w: f32) {
    let col_w = fit_col_width(ui, col_w);
    ui.set_max_width(col_w);
    ui.set_width(col_w);
    field_label(
        ui,
        if state.kind == DateKind::Span {
            "从哪天"
        } else {
            "日期"
        },
        false,
    );
    ui.add_space(2.0);
    date_field::show_datetime(ui, &format!("{id_salt}_due"), &mut state.due_date, true);
}

fn show_end_date(ui: &mut egui::Ui, state: &mut MemoFormState, id_salt: &str, col_w: f32) {
    let col_w = fit_col_width(ui, col_w);
    ui.set_max_width(col_w);
    ui.set_width(col_w);
    field_label(ui, "到哪天", false);
    ui.add_space(2.0);
    date_field::show(ui, &format!("{id_salt}_end"), &mut state.end_date, true);
}

fn kind_hint(kind: DateKind) -> &'static str {
    match kind {
        DateKind::Day => "出现在选中的那一天。",
        DateKind::Span => "月历上画一条横跨多天的色条。",
        DateKind::Parked => "进右侧「未安排」，以后再放到某一天。",
        DateKind::Credentials => "永久档案：不上日历；账号证件类强制私密。",
    }
}

/// 与分类下拉同高，保证「分类 | 这是」两框中线对齐。
const KIND_SEG_H: f32 = 30.0;

/// 紧凑分段控件「这是」的选项条（不含标签；标签由调用方与分类列对称绘制）。
fn show_kinds_seg(ui: &mut egui::Ui, state: &mut MemoFormState, stamp: &str) {
    let kinds = kinds_for(state.scope);
    if kinds.len() <= 1 {
        let k = kinds.first().copied().unwrap_or(state.kind);
        if state.kind != k {
            state.kind = k;
            state.sync_kind_fields();
        }
    }
    let seg_w = fit_col_width(ui, ui.available_width());
    // inner 2+2 + 按钮 26 = 30，与分类下拉 h 一致
    Frame::none()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(2.0, 2.0))
        .show(ui, |ui| {
            let seg_w = fit_col_width(ui, seg_w);
            ui.set_max_width(seg_w);
            ui.set_width(seg_w);
            ui.set_min_height(KIND_SEG_H - 4.0);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let locked = kinds.len() <= 1;
                let n = kinds.len().max(1) as f32;
                let btn_w = ((ui.available_width() - 2.0 * (n - 1.0)) / n)
                    .floor()
                    .max(36.0);
                for &k in kinds {
                    let on = state.kind == k;
                    let fill = if on {
                        theme::card()
                    } else {
                        Color32::TRANSPARENT
                    };
                    let fg = if on {
                        theme::text()
                    } else {
                        theme::text_muted()
                    };
                    // 窄列用短标签，完整说明放 hover
                    let label = if btn_w < 72.0 && k == DateKind::Credentials {
                        "备忘"
                    } else {
                        k.label()
                    };
                    let btn = ui.add_enabled(
                        !locked || on,
                        egui::Button::new(
                            RichText::new(label)
                                .size(12.0)
                                .color(fg)
                                .strong(),
                        )
                        .fill(fill)
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(6.0))
                        .min_size(Vec2::new(btn_w, 26.0)),
                    );
                    let btn = btn.on_hover_text(kind_hint(k));
                    if !locked && btn.clicked() {
                        state.set_kind(k, stamp);
                    }
                }
            });
        });
}

fn show_priority_field(ui: &mut egui::Ui, state: &mut MemoFormState, id_salt: &str, col_w: f32) {
    let col_w = fit_col_width(ui, col_w);
    ui.set_max_width(col_w);
    ui.set_width(col_w);
    field_label(ui, "优先级", false);
    ui.add_space(2.0);
    egui::ComboBox::from_id_source(format!("{id_salt}_prio"))
        .width((col_w - 2.0).max(1.0))
        .selected_text(state.priority.label())
        .show_ui(ui, |ui| {
            for &p in MemoPriority::ALL {
                ui.selectable_value(&mut state.priority, p, p.label());
            }
        });
}

fn show_remind_field(ui: &mut egui::Ui, state: &mut MemoFormState, id_salt: &str, col_w: f32) {
    let col_w = fit_col_width(ui, col_w);
    ui.set_max_width(col_w);
    ui.set_width(col_w);
    field_label(ui, "提醒", false);
    ui.add_space(2.0);
    let has_due = !state.due_date.trim().is_empty();
    if !has_due {
        state.remind_before_days = 0;
    }
    if ![0, 1, 3, 7].contains(&state.remind_before_days) {
        state.remind_before_days = 0;
    }
    let remind_label = match state.remind_before_days {
        1 => "提前 1 天",
        3 => "提前 3 天",
        7 => "提前 7 天",
        _ => "到期时",
    };
    ui.add_enabled_ui(has_due, |ui| {
        egui::ComboBox::from_id_source(format!("{id_salt}_remind"))
            .width((col_w - 2.0).max(1.0))
            .selected_text(if has_due { remind_label } else { "—" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.remind_before_days, 0, "到期时");
                ui.selectable_value(&mut state.remind_before_days, 1, "提前 1 天");
                ui.selectable_value(&mut state.remind_before_days, 3, "提前 3 天");
                ui.selectable_value(&mut state.remind_before_days, 7, "提前 7 天");
            });
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
    // 绝不能 .max(80) 再 allocate：双列/抽屉滑入首帧可用宽常 <80，会直接 abort 退进程
    let w = fit_col_width(ui, form_w(ui));
    let h = 30.0;
    if w < 8.0 {
        ui.allocate_exact_size(Vec2::new(w.max(1.0), h), Sense::hover());
        return;
    }
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
    // 弹层内也勿 .max(200) 强扩，父级 set_min_width 已决定宽度
    let w = fit_col_width(ui, ui.available_width());
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
/// 布局对齐 `docs/memo-form-compact-focus.html`：编号→标题→备注→分类|类型→日期网格→可见性→标签。
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
    let w = form_w(ui).max(1.0);
    // 抽屉动画极窄时勿强行 set_width 超过 available；双列至少约 100 才画完整表单
    let w = w.min(ui.available_width().max(1.0));
    if w < 24.0 {
        ui.allocate_exact_size(egui::vec2(w.max(1.0), 1.0), Sense::hover());
        return;
    }
    ui.set_max_width(w);
    ui.set_width(w);
    ui.spacing_mut().item_spacing.y = FIELD_GAP;

    // —— 事件编号 ——
    if !state.attach_id.trim().is_empty() {
        let short = short_event_id(&state.attach_id);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(
                RichText::new("事件编号")
                    .size(11.5)
                    .color(theme::text_muted()),
            );
            ui.label(
                RichText::new(&short)
                    .size(12.0)
                    .strong()
                    .color(theme::text())
                    .monospace(),
            );
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("复制")
                            .size(11.0)
                            .color(theme::text_muted()),
                    )
                    .fill(theme::card())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .rounding(Rounding::same(5.0))
                    .min_size(Vec2::new(0.0, 20.0)),
                )
                .on_hover_text(format!("完整编号：{}", state.attach_id.trim()))
                .clicked()
            {
                ui.ctx().copy_text(state.attach_id.trim().to_string());
            }
        });
    }

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
            ui.spacing_mut().item_spacing.y = 3.0;
            field_label_hint(ui, "标题", true, Some("一句话说清楚"));
            let title_id = ui.make_persistent_id((id_salt, "memo_title"));
            let title_resp = Frame::none()
                .stroke(title_stroke)
                .rounding(Rounding::same(7.0))
                .inner_margin(Margin::same(1.0))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.doc.title)
                            .id(title_id)
                            .desired_width((w - 2.0).clamp(1.0, w.max(1.0)))
                            .hint_text(theme::hint(title_hint(state.category)))
                            .margin(egui::vec2(9.0, 5.0)),
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
        title_block.scroll_to_me(Some(egui::Align::TOP));
        ui.scroll_to_rect(title_block.rect, Some(egui::Align::TOP));
        ui.ctx().request_repaint();
    }

    // —— 备注：整卡约 280（顶栏固定，正文卡内竖滚，不撑开抽屉外层）——
    const NOTES_MIN_H: f32 = 280.0;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        field_label_hint(ui, "备注", false, Some("可选"));
        let notes_w = fit_col_width(ui, ui.available_width());
        ui.set_max_width(notes_w);
        ui.set_width(notes_w);
        doc_editor::show_editor(
            ui,
            &mut state.doc,
            id_salt,
            state.data_dir.as_deref(),
            &state.attach_id,
            NOTES_MIN_H,
        );
    });

    // —— 分类 | 这是 ——（两列同结构：标签 + 固定间隙 + 等高控件，中线对齐）
    let selected_cat = if state.category.is_gender_private() {
        MemoCategory::GenderPrivate
    } else {
        state.category.canonical()
    };
    let col_gap = 8.0;
    let row_w = fit_col_width(ui, w);
    let half = split_cols(row_w, 2, col_gap);
    ui.horizontal(|ui| {
        ui.set_max_width(row_w);
        ui.spacing_mut().item_spacing.x = col_gap;
        ui.vertical(|ui| {
            let half = fit_col_width(ui, half);
            ui.set_max_width(half);
            ui.set_width(half);
            ui.spacing_mut().item_spacing.y = 0.0;
            field_label(ui, "分类", false);
            ui.add_space(2.0);
            show_category_combo(ui, id_salt, cats, selected_cat, &stamp, state);
        });
        ui.vertical(|ui| {
            let half = fit_col_width(ui, half);
            ui.set_max_width(half);
            ui.set_width(half);
            ui.spacing_mut().item_spacing.y = 0.0;
            field_label(ui, "这是", false);
            ui.add_space(2.0);
            show_kinds_seg(ui, state, &stamp);
        });
    });

    // —— 日期 | 优先级 | 提醒 ——
    let show_dates = matches!(state.kind, DateKind::Day | DateKind::Span);
    let show_end = state.kind == DateKind::Span;
    let third = split_cols(row_w, 3, col_gap);
    if show_dates && show_end {
        // 时段：从 / 到 占两列，优先级第三列；提醒下行
        ui.horizontal(|ui| {
            ui.set_max_width(row_w);
            ui.spacing_mut().item_spacing.x = col_gap;
            ui.vertical(|ui| {
                show_due_priority(ui, state, id_salt, third);
            });
            ui.vertical(|ui| {
                show_end_date(ui, state, id_salt, third);
            });
            ui.vertical(|ui| {
                show_priority_field(ui, state, id_salt, third);
            });
        });
        ui.horizontal(|ui| {
            ui.set_max_width(row_w);
            ui.spacing_mut().item_spacing.x = col_gap;
            ui.vertical(|ui| {
                show_remind_field(ui, state, id_salt, third);
            });
        });
    } else {
        ui.horizontal(|ui| {
            ui.set_max_width(row_w);
            ui.spacing_mut().item_spacing.x = col_gap;
            ui.vertical(|ui| {
                let third = fit_col_width(ui, third);
                ui.set_max_width(third);
                ui.set_width(third);
                if show_dates {
                    show_due_priority(ui, state, id_salt, third);
                } else {
                    field_label(ui, "日期", false);
                    ui.add_space(2.0);
                    ui.add_enabled_ui(false, |ui| {
                        let mut placeholder = String::from("— 不上日历");
                        ui.add(
                            egui::TextEdit::singleline(&mut placeholder)
                                .desired_width((third - 2.0).max(1.0))
                                .margin(egui::vec2(9.0, 5.0)),
                        );
                    });
                }
            });
            ui.vertical(|ui| {
                show_priority_field(ui, state, id_salt, third);
            });
            ui.vertical(|ui| {
                show_remind_field(ui, state, id_salt, third);
            });
        });
    }

    // —— 可见性（说明另起一行，避免右缘裁切）——
    ui.vertical(|ui| {
        ui.set_max_width(row_w);
        ui.spacing_mut().item_spacing.y = 2.0;
        field_label(ui, "可见性", false);
        Frame::none()
            .fill(theme::panel())
            .stroke(Stroke::new(1.0, theme::border()))
            .rounding(Rounding::same(7.0))
            .inner_margin(Margin::symmetric(10.0, 4.0))
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.add_enabled_ui(!(gender_locked || cred_locked), |ui| {
                        ui.radio_value(&mut state.visibility, MemoVisibility::Private, "私密");
                        ui.radio_value(&mut state.visibility, MemoVisibility::Public, "公开");
                    });
                });
                let note = if gender_locked {
                    Some("性别私密固定私密")
                } else if cred_locked {
                    Some("备忘固定私密")
                } else {
                    None
                };
                if let Some(n) = note {
                    ui.label(
                        RichText::new(n)
                            .size(11.0)
                            .color(theme::text_muted()),
                    );
                }
            });
    });

    // —— 标签 + 预设同行 ——
    ui.vertical(|ui| {
        ui.set_max_width(row_w);
        ui.spacing_mut().item_spacing.y = 2.0;
        field_label(ui, "标签", false);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(row_w);
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            let tag_w = (ui.available_width() * 0.42).clamp(80.0, row_w.min(180.0));
            let tag_resp = ui.add(
                egui::TextEdit::singleline(&mut state.tags)
                    .desired_width(tag_w)
                    .hint_text(theme::hint("逗号或空格分隔"))
                    .margin(egui::vec2(8.0, 4.0)),
            );
            if tag_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let tags = parse_tags(&state.tags);
                state.tags = format_tags(&tags);
            }
            ui.spacing_mut().button_padding = egui::vec2(7.0, 1.5);
            let cat_tags = preset_tags_for(state.category);
            for t in cat_tags.iter().chain(COMMON_TAGS.iter()).take(6) {
                let chip = ui.add(
                    egui::Button::new(
                        RichText::new(*t)
                            .size(11.0)
                            .color(theme::shell_accent()),
                    )
                    .fill(theme::shell_tag_bg())
                    .stroke(Stroke::NONE)
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(0.0, 22.0)),
                );
                if chip.clicked() {
                    append_tag(&mut state.tags, t);
                }
            }
        });
    });

    if mode == FormMode::Edit {
        ui.checkbox(&mut state.done, "已完成");
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

#[cfg(test)]
mod tests {
    use super::*;
    use memo_core::store::MemoCategory;

    /// 回归：新建/编辑表单在抽屉滑入·收起窄宽下不得 allocate>available 崩进程。
    #[test]
    fn create_form_ui_survives_narrow_drawer_widths() {
        let ctx = egui::Context::default();
        let stamp = "2026-10-21";
        // 模拟动画 t→0 收起时的面板内宽序列
        let widths = [
            1.0_f32, 12.0, 24.0, 40.0, 71.0, 72.0, 79.0, 80.0, 100.0, 200.0, 420.0,
        ];
        for scope in [
            FormScope::Credentials,
            FormScope::Calendar,
            FormScope::GenderPrivate,
            FormScope::Open,
        ] {
            let cat = match scope {
                FormScope::Credentials => MemoCategory::Credentials,
                FormScope::GenderPrivate => MemoCategory::GenderPrivate,
                _ => MemoCategory::Todo,
            };
            let mut create = MemoFormState::begin_create(cat, stamp, None, scope);
            let mut edit = MemoFormState::load_from_drafts(
                cat,
                "历史标题",
                "历史正文",
                "",
                "",
                MemoPriority::Normal,
                "标签",
                false,
                MemoVisibility::Private,
                0,
                "deadbeefcafe",
            );
            for &w in &widths {
                for (form, mode, salt) in [
                    (&mut create, FormMode::Create, "smoke_new"),
                    (&mut edit, FormMode::Edit, "smoke_edit"),
                ] {
                    let mut input = egui::RawInput::default();
                    input.screen_rect = Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(900.0, 700.0),
                    ));
                    let _ = ctx.begin_frame(input);
                    egui::CentralPanel::default().show(&ctx, |ui| {
                        ui.set_max_width(w);
                        ui.set_width(w);
                        show_fields(ui, form, mode, salt);
                    });
                    let _ = ctx.end_frame();
                }
            }
        }
        assert!(crate::page_shell::drawer_panel_too_narrow(70.0, 12.0));
        assert!(!crate::page_shell::drawer_panel_too_narrow(120.0, 12.0));
    }
}
