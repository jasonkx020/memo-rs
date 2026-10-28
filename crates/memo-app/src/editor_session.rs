//! 锁定前后保留新建/编辑备忘表单（对齐 sticky_session）。

use crate::memo_form::{DateKind, FormScope, MemoFormState};
use crate::nav::NavItem;
use memo_core::store::{MemoCategory, MemoPriority, MemoVisibility};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorSession {
    pub person_fp: String,
    pub nav: String,
    pub selected: Option<String>,
    pub editing: bool,
    pub show_new_memo: bool,
    pub title_draft: String,
    pub body_draft: String,
    pub visibility_draft: MemoVisibility,
    pub category_draft: MemoCategory,
    pub due_date_draft: String,
    pub end_date_draft: String,
    pub tags_draft: String,
    pub done_draft: bool,
    pub priority_draft: MemoPriority,
    pub form: FormSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormSnapshot {
    pub category: MemoCategory,
    pub due_date: String,
    pub end_date: String,
    pub kind: String,
    pub remind_before_days: u32,
    pub priority: MemoPriority,
    pub tags: String,
    pub title: String,
    pub body: String,
    pub done: bool,
    pub visibility: MemoVisibility,
    pub attach_id: String,
    pub scope: String,
}

impl EditorSession {
    pub fn worth_keeping(&self) -> bool {
        self.show_new_memo || self.editing || self.selected.is_some()
    }
}

pub fn session_path_for(data_dir: &Path, person_fp: &str) -> PathBuf {
    data_dir
        .join("identities")
        .join(person_fp)
        .join("editor_session.json")
}

pub fn save_session(data_dir: &Path, session: &EditorSession) -> anyhow::Result<()> {
    let path = session_path_for(data_dir, &session.person_fp);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(session)?;
    std::fs::write(path, raw)?;
    Ok(())
}

pub fn load_session(data_dir: &Path, person_fp: &str) -> Option<EditorSession> {
    let path = session_path_for(data_dir, person_fp);
    let raw = std::fs::read_to_string(path).ok()?;
    let s: EditorSession = serde_json::from_str(&raw).ok()?;
    if s.person_fp != person_fp && !s.person_fp.is_empty() {
        return None;
    }
    Some(EditorSession {
        person_fp: person_fp.to_string(),
        ..s
    })
}

pub fn clear_session_file(data_dir: &Path, person_fp: &str) {
    let path = session_path_for(data_dir, person_fp);
    let _ = std::fs::remove_file(path);
}

pub fn nav_to_str(nav: NavItem) -> &'static str {
    match nav {
        NavItem::Calendar => "calendar",
        NavItem::Credentials => "credentials",
        NavItem::GenderPrivate => "gender_private",
        NavItem::Trash => "trash",
    }
}

pub fn nav_from_str(s: &str) -> NavItem {
    match s {
        "credentials" => NavItem::Credentials,
        "gender_private" => NavItem::GenderPrivate,
        "trash" => NavItem::Trash,
        _ => NavItem::Calendar,
    }
}

fn kind_to_str(k: DateKind) -> &'static str {
    match k {
        DateKind::Day => "day",
        DateKind::Span => "span",
        DateKind::Parked => "parked",
        DateKind::Credentials => "credentials",
    }
}

fn kind_from_str(s: &str) -> DateKind {
    match s {
        "day" => DateKind::Day,
        "span" => DateKind::Span,
        "credentials" => DateKind::Credentials,
        _ => DateKind::Parked,
    }
}

fn scope_to_str(s: FormScope) -> &'static str {
    match s {
        FormScope::Open => "open",
        FormScope::Calendar => "calendar",
        FormScope::Credentials => "credentials",
        FormScope::GenderPrivate => "gender_private",
    }
}

fn scope_from_str(s: &str) -> FormScope {
    match s {
        "calendar" => FormScope::Calendar,
        "credentials" => FormScope::Credentials,
        "gender_private" => FormScope::GenderPrivate,
        _ => FormScope::Open,
    }
}

pub fn form_snapshot(form: &MemoFormState) -> FormSnapshot {
    FormSnapshot {
        category: form.category,
        due_date: form.due_date.clone(),
        end_date: form.end_date.clone(),
        kind: kind_to_str(form.kind).to_string(),
        remind_before_days: form.remind_before_days,
        priority: form.priority,
        tags: form.tags.clone(),
        title: form.doc.title.clone(),
        body: form.doc.body.clone(),
        done: form.done,
        visibility: form.visibility,
        attach_id: form.attach_id.clone(),
        scope: scope_to_str(form.scope).to_string(),
    }
}

pub fn form_from_snapshot(snap: &FormSnapshot, data_dir: &Path) -> MemoFormState {
    let mut form = MemoFormState::load_from_drafts(
        snap.category,
        &snap.title,
        &snap.body,
        &snap.due_date,
        &snap.end_date,
        snap.priority,
        &snap.tags,
        snap.done,
        snap.visibility,
        snap.remind_before_days,
        &snap.attach_id,
    );
    form.kind = kind_from_str(&snap.kind);
    form.scope = scope_from_str(&snap.scope);
    form.data_dir = Some(data_dir.to_path_buf());
    if !snap.attach_id.trim().is_empty() {
        form.attach_id = snap.attach_id.clone();
    }
    form
}

#[allow(clippy::too_many_arguments)]
pub fn capture(
    person_fp: &str,
    nav: NavItem,
    selected: &Option<String>,
    editing: bool,
    show_new_memo: bool,
    title_draft: &str,
    body_draft: &str,
    visibility_draft: MemoVisibility,
    category_draft: MemoCategory,
    due_date_draft: &str,
    end_date_draft: &str,
    tags_draft: &str,
    done_draft: bool,
    priority_draft: MemoPriority,
    form: &MemoFormState,
) -> EditorSession {
    EditorSession {
        person_fp: person_fp.to_string(),
        nav: nav_to_str(nav).to_string(),
        selected: selected.clone(),
        editing,
        show_new_memo,
        title_draft: title_draft.to_string(),
        body_draft: body_draft.to_string(),
        visibility_draft,
        category_draft,
        due_date_draft: due_date_draft.to_string(),
        end_date_draft: end_date_draft.to_string(),
        tags_draft: tags_draft.to_string(),
        done_draft,
        priority_draft,
        form: form_snapshot(form),
    }
}
