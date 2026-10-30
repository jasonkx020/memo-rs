mod calendar_view;
mod app_icon;
mod autostart;
mod date_field;
mod editor_session;
mod fortune;
mod huangli;
mod tray;
mod doc_editor;
mod fonts;
mod gender_private_view;
mod help_view;
mod history_view;
mod identity_gate;
mod male_view;
mod memo_form;
mod msg;
mod nav;
mod notify_sys;
mod page_shell;
mod period_view;
mod runtime;
mod save_dialog;
mod shell;
mod single_instance;
mod sticky_note;
mod theme;
mod update_check;

use eframe::egui::{self, Color32, Frame, Margin, RichText};
use identity_gate::{GateAction, IdentityGateState};
use memo_core::config::{self, Config, NodeRole};
use memo_core::identity_keys::IdentityKeys;
use memo_core::person::Gender;
use memo_core::service::HistoryEvent;
use memo_core::store::{MemoCategory, MemoLifecycle, MemoPriority, MemoVisibility};
use memo_form::{FormMode, MemoFormState};
use memo_sync::{EngineBroadcaster, SyncEngine};
use msg::BgMsg;
use runtime::AppRuntime;
use shell::{MemoDrawerAction, ShellAction, ShellUi};
use sticky_note::StickyHandle;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

fn auto_lock_after(cfg: &Config) -> Option<Duration> {
    let m = cfg.auto_lock_minutes;
    if m == 0 {
        None
    } else {
        Some(Duration::from_secs(u64::from(m) * 60))
    }
}

fn auto_lock_label(minutes: u32) -> String {
    match minutes {
        0 => "不自动锁定".into(),
        1 => "1 分钟".into(),
        n => format!("{n} 分钟"),
    }
}

fn settings_item_header(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(
        RichText::new(title)
            .size(13.5)
            .strong()
            .color(theme::text()),
    );
    if !description.is_empty() {
        ui.add_space(3.0);
        ui.label(
            RichText::new(description)
                .size(12.0)
                .color(theme::text_muted()),
        );
    }
    ui.add_space(6.0);
}

fn settings_item_gap(ui: &mut egui::Ui) {
    ui.add_space(14.0);
}

fn settings_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(title)
            .size(16.0)
            .strong()
            .color(theme::text()),
    );
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(10.0);
}

fn settings_text_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> bool {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(width)
            .hint_text(theme::hint(hint)),
    )
    .changed()
}

fn theme_pref_label(p: memo_core::ThemePreference) -> &'static str {
    p.label()
}

/// egui 0.27 在 Windows 中文 IME 下：CompositionUpdate 会写入临时字，
/// CompositionEnd 又常因光标与 ime_cursor_range 不一致而丢弃上屏（表现为只能输入约 1～2 个汉字）。
/// 将提交转为普通 Text，并忽略预编辑，交给系统候选框显示拼音过程。
fn sanitize_cjk_ime_events(ctx: &egui::Context) {
    ctx.input_mut(|i| {
        let events = std::mem::take(&mut i.events);
        i.events = events
            .into_iter()
            .filter_map(|e| match e {
                egui::Event::CompositionStart => None,
                egui::Event::CompositionUpdate(_) => None,
                egui::Event::CompositionEnd(text) => {
                    if text.is_empty() || text == "\n" || text == "\r" {
                        None
                    } else {
                        Some(egui::Event::Text(text))
                    }
                }
                other => Some(other),
            })
            .collect();
    });
}

enum Screen {
    IdentityGate(IdentityGateState),
    Main {
        rt: Arc<AppRuntime>,
        search: String,
        selected: Option<String>,
        title_draft: String,
        body_draft: String,
        visibility_draft: MemoVisibility,
        category_draft: MemoCategory,
        due_date_draft: String,
        end_date_draft: String,
        tags_draft: String,
        done_draft: bool,
        priority_draft: MemoPriority,
        editing: bool,
        show_settings: bool,
        settings_draft: Config,
        show_delete: bool,
        /// 头像菜单「重置密码」弹窗
        show_change_pw: bool,
        /// 新建备忘弹窗
        show_new_memo: bool,
        purge_confirm_id: Option<String>,
        show_export: bool,
        export_pw: String,
        export_path: String,
        export_ids: Option<Vec<String>>,
        show_history: bool,
        history_entity: String,
        history_events: Vec<HistoryEvent>,
        history_sel_a: Option<usize>,
        history_sel_b: Option<usize>,
        show_backup: bool,
        backup_import: bool,
        backup_pw: String,
        backup_path: String,
        status_line: String,
        peers: Vec<String>,
        discovered: Vec<memo_sync::DiscoveredPeer>,
        audit_ok: bool,
        audit_detail: String,
    },
}

pub struct MemoApp {
    cfg: Config,
    screen: Screen,
    tx: Sender<BgMsg>,
    rx: Receiver<BgMsg>,
    last_theme_mode: Option<theme::ThemeMode>,
    last_peer_refresh: Instant,
    show_help: bool,
    show_about: bool,
    pending_switch_person: bool,
    /// 私密页锁定：已切任务页，下一帧再真正锁定
    pending_lock_after_task: bool,
    shell: ShellUi,
    memo_doc: doc_editor::Doc,
    edit_form: MemoFormState,
    last_input_at: Instant,
    pending_sync_hint: usize,
    last_remind_scan: Instant,
    /// 桌面便签（memo_id → 共享句柄；deferred viewport 需 Send+Sync）
    stickies: HashMap<String, StickyHandle>,
    /// 系统托盘（退出时 take + dispose）
    tray: Option<tray::TrayHandle>,
    /// 真正退出（托盘菜单）；否则关主窗只隐藏
    quit_requested: bool,
    /// 主窗已藏到托盘
    main_hidden: bool,
    /// 开机 `--tray`：首帧隐藏主窗
    start_hidden: bool,
    last_sticky_session_save: Instant,
    /// 下一帧强制落盘便签会话（新建便签等）
    sticky_session_dirty: bool,
    /// 当前会话登录密码（锁定/切换身份时清空）
    session_login_password: Option<String>,
    /// 设置中改密草稿
    change_pw_old: String,
    change_pw_new: String,
    change_pw_new2: String,
    /// 保存需重启的设置后，确认是否立即重启
    show_restart_confirm: bool,
    /// 版本检查：远端最新 / 下载页 / 是否有新版本
    update_latest: Option<String>,
    update_download_url: Option<String>,
    update_newer: bool,
    update_check_busy: bool,
    update_check_msg: String,
    /// 本会话是否已做过静默检查
    update_silent_done: bool,
    /// 登录会话运行时（与 Screen 解耦：锁定 UI 时仍保持 LAN 监听）
    session_rt: Option<Arc<AppRuntime>>,
    /// 锁定前备忘编辑快照（同进程解锁优先；亦落盘）
    editor_resume: Option<editor_session::EditorSession>,
}

const HELP_DOC: &str = include_str!("HELP.md");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

impl MemoApp {
    pub fn new(cfg: Config, start_hidden: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let gate = IdentityGateState::from_cfg(&cfg);
        let tray = tray::create();
        Self {
            cfg,
            screen: Screen::IdentityGate(gate),
            tx,
            rx,
            last_theme_mode: None,
            last_peer_refresh: Instant::now() - Duration::from_secs(10),
            show_help: false,
            show_about: false,
            pending_switch_person: false,
            pending_lock_after_task: false,
            shell: ShellUi::default(),
            memo_doc: doc_editor::Doc::empty(),
            edit_form: MemoFormState::default(),
            last_input_at: Instant::now(),
            pending_sync_hint: 0,
            last_remind_scan: Instant::now() - Duration::from_secs(30),
            stickies: HashMap::new(),
            tray,
            quit_requested: false,
            main_hidden: start_hidden,
            start_hidden,
            last_sticky_session_save: Instant::now() - Duration::from_secs(60),
            sticky_session_dirty: false,
            session_login_password: None,
            change_pw_old: String::new(),
            change_pw_new: String::new(),
            change_pw_new2: String::new(),
            show_restart_confirm: false,
            update_latest: None,
            update_download_url: None,
            update_newer: false,
            update_check_busy: false,
            update_check_msg: String::new(),
            update_silent_done: false,
            session_rt: None,
            editor_resume: None,
        }
    }

    fn stop_session_rt(&mut self) {
        if let Some(rt) = self.session_rt.take() {
            rt.engine.stop();
            drop(rt);
        }
    }

    fn spawn_update_check(&mut self, silent: bool) {
        if self.update_check_busy {
            return;
        }
        self.update_check_busy = true;
        if !silent {
            self.update_check_msg = "正在检查更新…".into();
        }
        let tx = self.tx.clone();
        let local = APP_VERSION.to_string();
        std::thread::spawn(move || {
            let result = update_check::check_against_local(&local);
            let _ = tx.send(BgMsg::UpdateCheckResult { silent, result });
        });
    }

    fn apply_update_check_result(
        &mut self,
        silent: bool,
        result: Result<update_check::UpdateInfo, String>,
    ) {
        self.update_check_busy = false;
        match result {
            Ok(info) => {
                self.update_latest = Some(info.latest.clone());
                self.update_download_url = Some(info.download_url.clone());
                self.update_newer = info.newer;
                if info.newer {
                    self.update_check_msg =
                        format!("发现新版本 {}，可前往官网下载。", info.latest);
                } else if !silent {
                    self.update_check_msg = "已是最新版本。".into();
                } else {
                    self.update_check_msg.clear();
                }
            }
            Err(e) => {
                self.update_newer = false;
                if !silent {
                    self.update_check_msg = e;
                }
            }
        }
    }

    /// 拉起新进程后由调用方设置 `quit_requested` 退出当前实例。
    fn spawn_self_restart() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| format!("无法解析程序路径: {e}"))?;
        // 从设置确认重启：去掉 --tray，让用户看到主窗
        let args: Vec<std::ffi::OsString> = std::env::args_os()
            .skip(1)
            .filter(|a| a != "--tray")
            .collect();
        std::process::Command::new(exe)
            .args(&args)
            .spawn()
            .map_err(|e| format!("无法启动新进程: {e}"))?;
        Ok(())
    }

    fn save_sticky_session_now(&mut self) {
        // 仅在主界面落盘；锁定后 stickies 已空，勿覆盖会话文件
        let Screen::Main { rt, .. } = &self.screen else {
            return;
        };
        let fp = rt.svc.session_fp().to_string();
        let pairs: Vec<_> = self
            .stickies
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let session = sticky_note::session_from_handles(&fp, &pairs);
        let _ = sticky_note::save_session(Path::new(&self.cfg.data_dir), &session);
        self.last_sticky_session_save = Instant::now();
    }

    fn close_all_sticky_viewports(&mut self, ctx: &egui::Context) {
        for (id, h) in &self.stickies {
            h.lock().closed = true;
            sticky_note::force_hide_viewport(ctx, id);
        }
    }

    /// 解锁后按本机会话自动打开便签，并尽量恢复历史位置。
    fn restore_stickies_from_session(&mut self, rt: &AppRuntime) {
        let fp = rt.svc.session_fp().to_string();
        let Some(session) = sticky_note::load_session(Path::new(&self.cfg.data_dir), &fp) else {
            return;
        };
        let list = rt.svc.list();
        for g in session.notes {
            if self.stickies.contains_key(&g.id) {
                continue;
            }
            let Some(m) = list.iter().find(|m| m.id == g.id) else {
                continue;
            };
            self.stickies.insert(
                g.id.clone(),
                sticky_note::StickyNote::from_memo_geom(
                    m,
                    Some([g.x, g.y]),
                    Some([g.w.max(200.0), g.h.max(120.0)]),
                    true,
                ),
            );
        }
    }

    fn flush_dirty_stickies(&mut self, rt: &AppRuntime, status_line: &mut String) {
        let mut to_save = Vec::new();
        for (id, h) in &self.stickies {
            let n = h.lock();
            if n.needs_save() {
                to_save.push((id.clone(), n.title.clone(), n.body.clone()));
            }
        }
        for (id, title, body) in to_save {
            match rt.svc.edit(&id, &title, &body) {
                Ok(()) => {
                    if let Some(h) = self.stickies.get(&id) {
                        h.lock().clear_dirty();
                    }
                }
                Err(e) => {
                    *status_line = format!("便签保存失败: {e}");
                }
            }
        }
    }

    /// 锁定前：同步写回正在编辑的已有备忘，并快照新建/编辑 UI。
    fn flush_and_snapshot_editor_on_lock(&mut self, rt: &AppRuntime) {
        let fp = rt.svc.session_fp().to_string();
        let (editing, show_new, selected_id) = match &self.screen {
            Screen::Main {
                selected,
                editing,
                show_new_memo,
                ..
            } => (*editing, *show_new_memo, selected.clone()),
            _ => return,
        };

        let mut lock_status: Option<String> = None;
        if editing {
            if let Some(id) = selected_id.as_ref() {
                if !shell::is_private_hub(id) && !shell::is_new_draft(id) {
                    match memo_form::validate_title(&self.edit_form) {
                        Ok((t, b)) => {
                            let due = self.edit_form.due_date.clone();
                            let end = self.edit_form.end_date.clone();
                            let vis = self.edit_form.visibility;
                            let cat = self.edit_form.category.canonical();
                            let done = self.edit_form.done;
                            let prio = self.edit_form.priority;
                            let remind = self.edit_form.remind_before_days;
                            let tags = memo_form::tags_vec(&self.edit_form);
                            let tags_str = self.edit_form.tags.clone();
                            let doc = self.edit_form.doc.clone();
                            let meta = rt.svc.list().into_iter().find(|m| m.id == *id);
                            let seen = meta
                                .as_ref()
                                .map(|m| {
                                    if m.due_date.trim() == due.trim() {
                                        m.remind_seen_for.clone()
                                    } else {
                                        String::new()
                                    }
                                })
                                .unwrap_or_default();
                            match rt.svc.edit_full(
                                id,
                                &t,
                                &b,
                                vis,
                                MemoLifecycle::Permanent,
                                cat,
                                &due,
                                &end,
                                done,
                                &tags,
                                prio,
                                remind,
                                &seen,
                            ) {
                                Ok(()) => {
                                    if let Screen::Main {
                                        title_draft,
                                        body_draft,
                                        visibility_draft,
                                        category_draft,
                                        due_date_draft,
                                        end_date_draft,
                                        tags_draft,
                                        done_draft,
                                        priority_draft,
                                        ..
                                    } = &mut self.screen
                                    {
                                        *title_draft = t;
                                        *body_draft = b;
                                        *category_draft = cat;
                                        *due_date_draft = due;
                                        *end_date_draft = end;
                                        *priority_draft = prio;
                                        *tags_draft = tags_str;
                                        *done_draft = done;
                                        *visibility_draft = vis;
                                    }
                                    self.memo_doc = doc;
                                }
                                Err(e) => {
                                    lock_status = Some(format!("锁定前保存失败: {e}"));
                                }
                            }
                        }
                        Err(e) => {
                            lock_status = Some(format!("锁定前未保存（{e}），将恢复草稿"));
                        }
                    }
                }
            }
        }

        if let (Some(msg), Screen::Main { status_line, .. }) =
            (lock_status, &mut self.screen)
        {
            *status_line = msg;
        }

        let session = match &self.screen {
            Screen::Main {
                selected,
                title_draft,
                body_draft,
                visibility_draft,
                category_draft,
                due_date_draft,
                end_date_draft,
                tags_draft,
                done_draft,
                priority_draft,
                ..
            } => editor_session::capture(
                &fp,
                self.shell.nav,
                selected,
                editing,
                show_new,
                title_draft,
                body_draft,
                *visibility_draft,
                *category_draft,
                due_date_draft,
                end_date_draft,
                tags_draft,
                *done_draft,
                *priority_draft,
                &self.edit_form,
            ),
            _ => return,
        };
        if session.worth_keeping() {
            let _ = editor_session::save_session(Path::new(&self.cfg.data_dir), &session);
            self.editor_resume = Some(session);
        } else {
            editor_session::clear_session_file(Path::new(&self.cfg.data_dir), &fp);
            self.editor_resume = None;
        }
    }

    fn clear_editor_session_at(
        data_dir: &Path,
        resume: &mut Option<editor_session::EditorSession>,
        person_fp: &str,
    ) {
        *resume = None;
        editor_session::clear_session_file(data_dir, person_fp);
    }

    fn clear_editor_session(&mut self, person_fp: Option<&str>) {
        let fp = person_fp
            .map(|s| s.to_string())
            .or_else(|| {
                self.session_rt
                    .as_ref()
                    .map(|rt| rt.svc.session_fp().to_string())
            })
            .or_else(|| {
                if let Screen::Main { rt, .. } = &self.screen {
                    Some(rt.svc.session_fp().to_string())
                } else {
                    None
                }
            });
        if let Some(fp) = fp {
            Self::clear_editor_session_at(
                Path::new(&self.cfg.data_dir),
                &mut self.editor_resume,
                &fp,
            );
        }
    }

    fn restore_editor_from_session(&mut self, rt: &AppRuntime) {
        let fp = rt.svc.session_fp().to_string();
        let session = match self.editor_resume.take() {
            Some(s) if s.person_fp == fp || s.person_fp.is_empty() => Some(s),
            Some(_) => {
                // 换身份：丢弃内存快照，尝试磁盘（应为该身份文件）
                editor_session::load_session(Path::new(&self.cfg.data_dir), &fp)
            }
            None => editor_session::load_session(Path::new(&self.cfg.data_dir), &fp),
        };
        let Some(session) = session else {
            return;
        };
        if !session.worth_keeping() {
            editor_session::clear_session_file(Path::new(&self.cfg.data_dir), &fp);
            return;
        }

        self.shell.nav = editor_session::nav_from_str(&session.nav);
        self.edit_form =
            editor_session::form_from_snapshot(&session.form, Path::new(&rt.cfg.data_dir));
        self.memo_doc = self.edit_form.doc.clone();

        let Screen::Main {
            selected,
            title_draft,
            body_draft,
            visibility_draft,
            category_draft,
            due_date_draft,
            end_date_draft,
            tags_draft,
            done_draft,
            priority_draft,
            editing,
            show_new_memo,
            ..
        } = &mut self.screen
        else {
            self.editor_resume = Some(session);
            return;
        };

        *selected = session.selected.clone();
        *editing = session.editing;
        *show_new_memo = session.show_new_memo;
        *title_draft = session.title_draft.clone();
        *body_draft = session.body_draft.clone();
        *visibility_draft = session.visibility_draft;
        *category_draft = session.category_draft;
        *due_date_draft = session.due_date_draft.clone();
        *end_date_draft = session.end_date_draft.clone();
        *tags_draft = session.tags_draft.clone();
        *done_draft = session.done_draft;
        *priority_draft = session.priority_draft;

        // 恢复后仍保留磁盘会话，直到用户保存/取消
        self.editor_resume = Some(session);
    }

    fn refresh_stickies_from_list(&mut self, rt: &AppRuntime) {
        let list = rt.svc.list();
        for (id, h) in &self.stickies {
            if let Some(m) = list.iter().find(|m| m.id == *id) {
                h.lock().apply_memo_if_clean(m);
            }
        }
    }

    fn make_main(rt: Arc<AppRuntime>, cfg: &Config) -> Screen {
        let (audit_ok, audit_detail) = rt.svc.verify_audit();
        let peers = rt.engine.connected_peers();
        let discovered = rt.engine.discovered_peers();
        Screen::Main {
            peers,
            discovered,
            rt,
            search: String::new(),
            selected: None,
            title_draft: String::new(),
            body_draft: String::new(),
            visibility_draft: MemoVisibility::Private,
            category_draft: MemoCategory::General,
            due_date_draft: String::new(),
            end_date_draft: String::new(),
            tags_draft: String::new(),
            done_draft: false,
            priority_draft: MemoPriority::Normal,
            editing: false,
            show_settings: false,
            settings_draft: cfg.clone(),
            show_delete: false,
            show_change_pw: false,
            show_new_memo: false,
            purge_confirm_id: None,
            show_export: false,
            export_pw: String::new(),
            export_path: String::new(),
            export_ids: None,
            show_history: false,
            history_entity: "memo".into(),
            history_events: Vec::new(),
            history_sel_a: None,
            history_sel_b: None,
            show_backup: false,
            backup_import: false,
            backup_pw: String::new(),
            backup_path: String::new(),
            status_line: String::new(),
            audit_ok,
            audit_detail,
        }
    }

    fn draw_help_about_windows(&mut self, ctx: &egui::Context) {
        if self.show_help {
            let modal = theme::begin_modal(ctx, "help_modal");
            let mut open = self.show_help;
            theme::modal_fixed(ctx, "使用说明", [660.0, 540.0])
                .id(modal.window_id)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(theme::muted_label("以下内容已嵌入本程序，可离线阅读。").small());
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height((ui.available_height() - 8.0).max(120.0))
                        .show(ui, |ui| {
                            help_view::show(ui, HELP_DOC);
                        });
                });
            self.show_help = open;
            if modal.end(ctx, self.show_help) {
                self.show_help = false;
            }
        }
        if self.show_about {
            let modal = theme::begin_modal(ctx, "about_modal");
            let mut open = self.show_about;
            let mut want_check = false;
            let mut want_download = false;
            let busy = self.update_check_busy;
            let newer = self.update_newer;
            let msg = self.update_check_msg.clone();
            let latest = self.update_latest.clone();
            theme::modal_fixed(ctx, "关于", [480.0, 480.0])
                .id(modal.window_id)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        app_icon::show(ui, 48.0);
                        ui.add_space(8.0);
                        ui.label(theme::brand_title(22.0));
                        ui.label(
                            RichText::new(format!("版本 {APP_VERSION}"))
                                .color(theme::text_muted()),
                        );
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            if theme::page_outline_button(ui, "检查更新").clicked() {
                                want_check = true;
                            }
                        });
                        if newer {
                            if theme::page_primary_button(ui, "前往下载").clicked() {
                                want_download = true;
                            }
                        }
                    });
                    if !msg.is_empty() {
                        ui.add_space(6.0);
                        let fg = if newer {
                            theme::shell_accent()
                        } else {
                            theme::text_muted()
                        };
                        ui.label(RichText::new(&msg).size(12.5).color(fg));
                        if newer {
                            if let Some(v) = latest.as_ref() {
                                ui.label(
                                    RichText::new(format!("最新版本 {v}"))
                                        .size(12.0)
                                        .color(theme::text_muted()),
                                );
                            }
                        }
                    } else {
                        ui.add_space(4.0);
                        ui.label(
                            theme::muted_label("检查更新仅提示并跳转官网，不会自动安装。")
                                .small(),
                        );
                    }
                    ui.add_space(12.0);
                    about_kv(ui, "产品", theme::APP_NAME);
                    about_kv(ui, "说明", theme::APP_DESCRIPTION);
                    about_kv(ui, "版权", theme::APP_COPYRIGHT);
                    ui.add_space(8.0);
                    ui.label(RichText::new("主要能力").strong().color(theme::text()));
                    ui.add_space(4.0);
                    ui.label(RichText::new("· 身份密钥对登录 · 分类备忘壳层").color(theme::text()));
                    ui.label(RichText::new("· 私人密文异地托管 · 公开备忘局域网同步").color(theme::text()));
                    ui.label(RichText::new("· Ed25519 审计链 · 变更时间线").color(theme::text()));
                    ui.label(RichText::new("· 性别私密（经期关怀 + 体检健康）· 加密备份").color(theme::text()));
                    ui.add_space(10.0);
                    ui.label(
                        theme::muted_label(
                            "可在资源管理器中右键 memo.exe → 属性 → 详细信息 查看版本与版权。",
                        )
                        .small(),
                    );
                    if let Ok(p) = config::settings_path() {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!("配置目录: {}", p.display()))
                                .small()
                                .color(theme::text_muted()),
                        );
                    }
                });
            self.show_about = open;
            if modal.end(ctx, self.show_about) {
                self.show_about = false;
            }
            if want_check {
                self.spawn_update_check(false);
            }
            if want_download {
                let url = self
                    .update_download_url
                    .as_deref()
                    .unwrap_or(update_check::DOWNLOAD_PAGE_URL);
                update_check::open_download_page(url);
            }
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BgMsg::UnlockResult(Ok((rt, local_password))) => {
                    let cfg = rt.cfg.clone();
                    self.cfg = cfg.clone();
                    let unlock_fp = rt.svc.session_fp().to_string();
                    // 换身份：丢掉上一身份内存草稿
                    if self
                        .editor_resume
                        .as_ref()
                        .is_some_and(|s| !s.person_fp.is_empty() && s.person_fp != unlock_fp)
                    {
                        self.editor_resume = None;
                    }
                    self.session_rt = Some(rt.clone());
                    self.shell = ShellUi::default();
                    self.shell.cal.load_prefs(
                        Path::new(&cfg.data_dir),
                        rt.svc.session_fp(),
                    );
                    self.shell.drawer.load(
                        Path::new(&cfg.data_dir),
                        rt.svc.session_fp(),
                    );
                    self.last_input_at = Instant::now();
                    self.pending_sync_hint = 0;
                    self.session_login_password = Some(local_password);
                    self.change_pw_old.clear();
                    self.change_pw_new.clear();
                    self.change_pw_new2.clear();
                    self.restore_stickies_from_session(&rt);
                    // 每天首次解锁：按手标重算经期预测（排卵/易孕/下次）
                    let list = rt.svc.list();
                    let _ = period_view::ensure_daily_predict_recalc(&rt.svc, &list);
                    self.screen = Self::make_main(rt.clone(), &cfg);
                    self.restore_editor_from_session(&rt);
                    // 进主壳后静默检查一次（stub 未配置时不打扰）
                    self.update_silent_done = false;
                    self.update_newer = false;
                    self.update_latest = None;
                    self.update_download_url = None;
                    self.update_check_msg.clear();
                    self.spawn_update_check(true);
                    self.update_silent_done = true;
                }
                BgMsg::UnlockResult(Err(e)) => {
                    if let Screen::IdentityGate(st) = &mut self.screen {
                        st.status = format!("打开失败: {e}");
                        st.busy = false;
                    }
                }
                BgMsg::Error(e) => match &mut self.screen {
                    Screen::Main { status_line, .. } => {
                        *status_line = format!("错误: {e}");
                    }
                    Screen::IdentityGate(st) => {
                        st.status = format!("错误: {e}");
                        st.busy = false;
                    }
                },
                BgMsg::Info(s) => {
                    if let Screen::Main { status_line, .. } = &mut self.screen {
                        *status_line = s;
                    }
                }
                BgMsg::CreatedMemo {
                    id,
                    title,
                    body,
                    category,
                    due_date,
                    end_date,
                    priority,
                    tags,
                } => {
                    let created_fp = if let Screen::Main { rt, .. } = &self.screen {
                        Some(rt.svc.session_fp().to_string())
                    } else {
                        None
                    };
                    if let Screen::Main {
                        selected,
                        title_draft,
                        body_draft,
                        category_draft,
                        due_date_draft,
                        end_date_draft,
                        tags_draft,
                        done_draft,
                        priority_draft,
                        editing,
                        show_new_memo,
                        status_line,
                        ..
                    } = &mut self.screen
                    {
                        *show_new_memo = false;
                        *selected = Some(id.clone());
                        *title_draft = title.clone();
                        *body_draft = body.clone();
                        *category_draft = category;
                        *due_date_draft = due_date.clone();
                        *end_date_draft = end_date.clone();
                        *tags_draft = tags
                            .iter()
                            .map(|t| {
                                if t.starts_with('#') {
                                    t.clone()
                                } else {
                                    format!("#{t}")
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        *done_draft = false;
                        *priority_draft = priority;
                        *editing = false;
                        *status_line = "已创建".into();
                        self.memo_doc = doc_editor::Doc::from_store(&title, &body);
                        self.edit_form = MemoFormState::load_from_drafts(
                            category,
                            &title,
                            &body,
                            &due_date,
                            &end_date,
                            priority,
                            tags_draft,
                            false,
                            MemoVisibility::Private,
                            0,
                            &id,
                        );
                        self.edit_form.data_dir =
                            Some(std::path::PathBuf::from(&self.cfg.data_dir));
                    }
                    if let Some(fp) = created_fp.as_deref() {
                        self.clear_editor_session(Some(fp));
                    }
                }
                BgMsg::Deleted => {
                    if let Screen::Main {
                        selected,
                        title_draft,
                        body_draft,
                        tags_draft,
                        done_draft,
                        editing,
                        status_line,
                        rt,
                        peers,
                        discovered,
                        audit_ok,
                        audit_detail,
                        ..
                    } = &mut self.screen
                    {
                        *selected = None;
                        title_draft.clear();
                        body_draft.clear();
                        tags_draft.clear();
                        *done_draft = false;
                        *editing = false;
                        *status_line = "已移入回收站".into();
                        *peers = rt.engine.connected_peers();
                        *discovered = rt.engine.discovered_peers();
                        let (ok, detail) = rt.svc.verify_audit();
                        *audit_ok = ok;
                        *audit_detail = detail;
                    }
                    self.memo_doc = doc_editor::Doc::empty();
                }
                BgMsg::Refresh => {
                    if let Screen::Main {
                        rt,
                        peers,
                        discovered,
                        audit_ok,
                        audit_detail,
                        selected,
                        title_draft,
                        body_draft,
                        visibility_draft,
                        category_draft,
                        due_date_draft,
                        end_date_draft,
                        tags_draft,
                        done_draft,
                        priority_draft,
                        editing,
                        status_line,
                        ..
                    } = &mut self.screen
                    {
                        *peers = rt.engine.connected_peers();
                        *discovered = rt.engine.discovered_peers();
                        let (ok, detail) = rt.svc.verify_audit();
                        *audit_ok = ok;
                        *audit_detail = detail;
                        let conflicts = rt.svc.take_conflicts();
                        if !conflicts.is_empty() {
                            let n = conflicts.len();
                            self.pending_sync_hint = n;
                            let first = &conflicts[0];
                            let title = if first.title.is_empty() {
                                "(无标题)"
                            } else {
                                first.title.as_str()
                            };
                            *status_line = format!(
                                "同步冲突 {n} 条：保留本机「{title}」v{}，拒收节点 {} 的 v{}",
                                first.local_version, first.remote_node, first.remote_version
                            );
                        }
                        if !*editing {
                            if let Some(id) = selected.clone() {
                                if let Some(m) = rt.svc.list().into_iter().find(|m| m.id == id) {
                                    *title_draft = m.title;
                                    *body_draft = m.content;
                                    *visibility_draft = m.visibility;
                                    *category_draft = m.category.canonical();
                                    *due_date_draft = m.due_date;
                                    *end_date_draft = m.end_date;
                                    *tags_draft = m
                                        .tags
                                        .iter()
                                        .map(|t| {
                                            if t.starts_with('#') {
                                                t.clone()
                                            } else {
                                                format!("#{t}")
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    *done_draft = m.done;
                                    *priority_draft = m.priority;
                                    self.memo_doc =
                                        doc_editor::Doc::from_store(title_draft, body_draft);
                                } else {
                                    *selected = None;
                                    title_draft.clear();
                                    body_draft.clear();
                                    tags_draft.clear();
                                    *done_draft = false;
                                    *priority_draft = MemoPriority::Normal;
                                }
                            }
                        }
                    }
                }
                BgMsg::UpdateCheckResult { silent, result } => {
                    self.apply_update_check_result(silent, result);
                }
            }
            ctx.request_repaint();
        }
    }
}

fn about_kv(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{key}："))
                .color(theme::text_muted())
                .size(13.0),
        );
        ui.label(RichText::new(value).color(theme::text()).size(13.0));
    });
}

fn chrono_like_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

impl eframe::App for MemoApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        theme::sync(ctx, self.cfg.theme, &mut self.last_theme_mode);
        sanitize_cjk_ime_events(ctx);
        tray::bind_context(ctx);
        tray::capture_main_hwnd(frame);
        self.pump(ctx);

        // 开机 --tray：用 Win32 隐藏，避免 Visible(false) 冻死事件循环
        if self.start_hidden {
            if tray::has_main_hwnd() {
                tray::hide_main_window();
                self.main_hidden = true;
                self.start_hidden = false;
            } else {
                ctx.request_repaint();
            }
        }

        let (tray_show, tray_quit) = tray::poll();
        if tray_show {
            self.main_hidden = false;
            tray::show_main_window();
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.last_input_at = Instant::now();
            ctx.request_repaint();
        }
        if tray_quit {
            self.quit_requested = true;
        }

        // 关主窗（×）→ 藏托盘；最小化交给系统，不读 minimized
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if !self.quit_requested && close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.save_sticky_session_now();
            tray::hide_main_window();
            self.main_hidden = true;
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        // 托盘退出快路径（主循环仍在转时）：落盘 + dispose + exit。
        // 若主窗最小化/SW_HIDE 导致 update 不跑，由 tray::force_quit_process 兜底。
        if self.quit_requested {
            let rt_for_flush = self
                .session_rt
                .clone()
                .or_else(|| {
                    if let Screen::Main { rt, .. } = &self.screen {
                        Some(rt.clone())
                    } else {
                        None
                    }
                });
            if let Some(rt) = rt_for_flush {
                let mut dummy = String::new();
                self.flush_dirty_stickies(&rt, &mut dummy);
            }
            self.save_sticky_session_now();
            self.close_all_sticky_viewports(ctx);
            self.stickies.clear();
            self.stop_session_rt();
            if let Some(t) = self.tray.take() {
                t.dispose();
            }
            std::process::exit(0);
        }

        // 上一帧已从私密切到任务：本帧执行锁定
        if self.pending_lock_after_task {
            self.pending_lock_after_task = false;
            self.pending_switch_person = true;
        }

        if self.pending_switch_person {
            // 锁定 / 自动锁定：只挡 UI，保留 session_rt 使主节点继续 LAN 监听
            let rt_for_flush = self
                .session_rt
                .clone()
                .or_else(|| {
                    if let Screen::Main { rt, .. } = &self.screen {
                        Some(rt.clone())
                    } else {
                        None
                    }
                });
            if let Some(rt) = &rt_for_flush {
                self.flush_and_snapshot_editor_on_lock(rt);
                let mut dummy = String::new();
                self.flush_dirty_stickies(rt, &mut dummy);
                // 确保 Arc 挂在 session_rt（离开 Main 后仍存活）
                if self.session_rt.is_none() {
                    self.session_rt = Some(rt.clone());
                }
            }
            self.save_sticky_session_now();
            self.close_all_sticky_viewports(ctx);
            self.stickies.clear();
            self.session_login_password = None;
            self.change_pw_old.clear();
            self.change_pw_new.clear();
            self.change_pw_new2.clear();
            let mut gate = IdentityGateState::from_cfg(&self.cfg);
            gate.skip_auto_unlock = true;
            self.screen = Screen::IdentityGate(gate);
            self.pending_switch_person = false;
            self.pending_lock_after_task = false;
        }

        if let Screen::Main {
            selected, editing, ..
        } = &mut self.screen
        {
            let any_input = ctx.input(|i| {
                i.pointer.any_pressed()
                    || i.events.iter().any(|e| {
                        matches!(
                            e,
                            egui::Event::Key { pressed: true, .. }
                                | egui::Event::Text(_)
                                | egui::Event::Scroll(_)
                        )
                    })
            });
            if any_input {
                self.last_input_at = Instant::now();
            }
            if let Some(after) = auto_lock_after(&self.cfg) {
                if self.last_input_at.elapsed() >= after {
                    if shell::leave_private_for_lock(&mut self.shell, selected, editing) {
                        self.pending_lock_after_task = true;
                        ctx.request_repaint();
                    } else {
                        self.pending_switch_person = true;
                    }
                }
            }
            ctx.request_repaint_after(Duration::from_secs(1));
        }

        let mut show_help = self.show_help;
        let mut show_about = self.show_about;

        match &mut self.screen {
            Screen::IdentityGate(st) => {
                if st.busy {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                match identity_gate::show(ctx, &mut self.cfg, st) {
                    GateAction::ShowHelp => show_help = true,
                    GateAction::Unlock {
                        cfg,
                        identity,
                        export_path,
                        export_pass,
                        local_password,
                    } => {
                        st.busy = true;
                        st.status = if export_path.is_some() {
                            "正在导出并打开…".into()
                        } else {
                            "正在打开…".into()
                        };
                        let tx = self.tx.clone();
                        let session_pw = local_password.clone();
                        let fp = identity.fingerprint.clone();
                        // 同身份：复用已在听的 session_rt，避免重复 bind
                        let reuse = self
                            .session_rt
                            .as_ref()
                            .filter(|rt| rt.svc.session_fp() == fp)
                            .cloned();
                        if reuse.is_none() {
                            // 换身份：停掉旧引擎再开新会话
                            self.stop_session_rt();
                        }
                        ctx.request_repaint();
                        std::thread::spawn(move || {
                            let res: Result<(Arc<AppRuntime>, String), String> = (|| {
                                if let Some(path) = export_path.as_ref() {
                                    let p = PathBuf::from(path);
                                    // .memokey 明文+签名，无导出口令（export_pass 忽略）
                                    let _ = export_pass;
                                    identity.export_file(&p, "")?;
                                }
                                config::ensure_data_dir(&cfg)?;
                                let mut id = identity;
                                id.exported_once = true;
                                let rt = if let Some(existing) = reuse {
                                    // 校验登录密码后复用运行时（监听不中断）
                                    IdentityKeys::load(
                                        Path::new(&cfg.data_dir),
                                        &fp,
                                        &local_password,
                                    )?;
                                    let _ = id;
                                    existing
                                } else {
                                    unlock_runtime_identity(cfg, id, &local_password)?
                                };
                                Ok((rt, session_pw))
                            })()
                            .map_err(|e: anyhow::Error| e.to_string());
                            let _ = tx.send(BgMsg::UnlockResult(res));
                        });
                    }
                    GateAction::None => {}
                }
            }
            Screen::Main {
                rt,
                search,
                selected,
                title_draft,
                body_draft,
                visibility_draft,
                category_draft,
                due_date_draft,
                end_date_draft,
                tags_draft,
                done_draft,
                priority_draft,
                editing,
                show_settings,
                settings_draft,
                show_delete,
                show_change_pw,
                show_new_memo,
                purge_confirm_id,
                show_export,
                export_pw,
                export_path,
                export_ids,
                show_history,
                history_entity,
                history_events,
                history_sel_a,
                history_sel_b,
                show_backup,
                backup_import,
                backup_pw,
                backup_path,
                status_line,
                peers,
                discovered,
                audit_ok,
                audit_detail,
            } => {
                let rt = rt.clone();
                if self.last_peer_refresh.elapsed() >= Duration::from_secs(2) {
                    *peers = rt.engine.connected_peers();
                    *discovered = rt.engine.discovered_peers();
                    let conflicts = rt.svc.take_conflicts();
                    if !conflicts.is_empty() {
                        let n = conflicts.len();
                        self.pending_sync_hint = n;
                        let first = &conflicts[0];
                        let title = if first.title.is_empty() {
                            "(无标题)"
                        } else {
                            first.title.as_str()
                        };
                        *status_line = format!(
                            "同步冲突 {n} 条：保留本机「{title}」v{}，拒收节点 {} 的 v{}",
                            first.local_version, first.remote_node, first.remote_version
                        );
                    }
                    self.last_peer_refresh = Instant::now();
                }

                if self.last_remind_scan.elapsed() >= Duration::from_secs(30) {
                    self.last_remind_scan = Instant::now();
                    let list = rt.svc.list();
                    // 长期开机跨日：自动重算经期预测
                    let _ = period_view::ensure_daily_predict_recalc(&rt.svc, &list);
                    // 顶栏告警：自然日变化则立刻重绘（关怀按日已读 / 过期集合）
                    let today_ymd = chrono::Local::now()
                        .date_naive()
                        .format("%Y-%m-%d")
                        .to_string();
                    if self.shell.last_nav_warn_ymd != today_ymd {
                        self.shell.last_nav_warn_ymd = today_ymd;
                        ctx.request_repaint();
                    }
                    let now = chrono::Local::now().naive_local();
                    for m in list {
                        if m.done || m.due_date.trim().is_empty() {
                            continue;
                        }
                        // 经期/备注等日历备忘不当作到期待办提醒
                        if shell::is_private_calendar_memo(&m) {
                            continue;
                        }
                        if m.remind_seen_for == m.due_date {
                            continue;
                        }
                        let Some(at) = memo_core::remind_at(&m.due_date, m.remind_before_days)
                        else {
                            continue;
                        };
                        if now < at {
                            continue;
                        }
                        let title = if m.title.trim().is_empty() {
                            "备忘到期提醒"
                        } else {
                            m.title.trim()
                        };
                        let body = memo_core::display_due(&m.due_date);
                        let ok = notify_sys::notify(title, &body);
                        if let Err(e) = rt.svc.ack_remind(&m.id) {
                            *status_line = format!("提醒标记失败: {e}");
                        } else if ok {
                            *status_line = format!("已提醒：{title}");
                        } else {
                            *status_line = format!("到期提醒（系统通知失败）：{title} · {body}");
                            ctx.request_repaint();
                        }
                    }
                    ctx.request_repaint_after(Duration::from_secs(30));
                }

                // status bar（须在 CentralPanel 之前）
                egui::TopBottomPanel::bottom("shell_bottom")
                    .frame(theme::bottom_bar_frame())
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("节点 {}", rt.cfg.node_id))
                                    .small()
                                    .color(theme::text_muted()),
                            );
                            ui.separator();
                            ui.label(
                                RichText::new(format!(
                                    "发现 {} · 连接 {}",
                                    discovered.len(),
                                    peers.len()
                                ))
                                .small()
                                .color(theme::text_muted()),
                            );
                            ui.separator();
                            ui.label(
                                RichText::new(format!("待同步 {} 条", self.pending_sync_hint))
                                    .small()
                                    .color(theme::text_muted()),
                            );
                            ui.separator();
                            let disk = rt.svc.disk_space();
                            let disk_txt = if disk.total_bytes > 0 {
                                format!("磁盘 {}", disk.format_pair())
                            } else {
                                "磁盘 —".into()
                            };
                            let disk_color = if disk.is_low() {
                                theme::warn()
                            } else {
                                theme::text_muted()
                            };
                            let disk_resp = ui.label(
                                RichText::new(disk_txt).small().color(disk_color),
                            );
                            if disk.is_low() {
                                disk_resp.on_hover_text(memo_core::disk::disk_help_hint());
                            } else if disk.total_bytes > 0 {
                                disk_resp.on_hover_text("数据目录所在卷：可用 / 总量");
                            }
                            if *audit_ok {
                                ui.separator();
                                ui.label(
                                    RichText::new("审计完整").small().color(theme::success()),
                                );
                            }
                            if let Some(err) = rt.engine.listen_error() {
                                ui.separator();
                                ui.label(
                                    RichText::new(err)
                                        .small()
                                        .color(theme::warn()),
                                );
                            } else if !status_line.is_empty() {
                                ui.separator();
                                ui.label(
                                    RichText::new(status_line.as_str())
                                        .small()
                                        .color(theme::text_muted()),
                                );
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("数据本地加密 · 仅同步已授权节点")
                                            .small()
                                            .color(theme::text_muted()),
                                    );
                                },
                            );
                            let _ = audit_detail;
                        });
                    });

                let listen_err = rt.engine.listen_error();
                let editing_before = *editing;
                let new_memo_before = *show_new_memo;
                let shell_action: ShellAction = shell::show(
                    ctx,
                    &rt.svc,
                    &mut self.shell,
                    search,
                    selected,
                    title_draft,
                    body_draft,
                    visibility_draft,
                    category_draft,
                    due_date_draft,
                    end_date_draft,
                    tags_draft,
                    done_draft,
                    priority_draft,
                    editing,
                    &mut self.memo_doc,
                    &mut self.edit_form,
                    status_line,
                    *show_new_memo,
                    show_settings,
                    show_change_pw,
                    show_export,
                    export_pw,
                    export_path,
                    export_ids,
                    show_history,
                    history_entity,
                    history_events,
                    history_sel_a,
                    history_sel_b,
                    show_delete,
                    purge_confirm_id,
                    &mut show_help,
                    &mut show_about,
                    &rt.cfg.data_dir,
                    rt.cfg.show_backup_status,
                    discovered,
                    &rt.cfg.node_id,
                    rt.cfg.node_role,
                    rt.cfg.node_visible,
                    rt.cfg.lan_discovery,
                    rt.svc.disk_space(),
                    &mut settings_draft.backup_targets,
                    &self.tx,
                    if self.update_newer {
                        self.update_latest.as_deref()
                    } else {
                        None
                    },
                    if self.update_newer {
                        self.update_download_url.as_deref()
                    } else {
                        None
                    },
                    listen_err.as_deref(),
                );
                if shell_action.backup_dirty {
                    let _ = config::save_settings(settings_draft);
                    rt.engine
                        .set_backup_targets(settings_draft.backup_targets.clone());
                    self.cfg.backup_targets = settings_draft.backup_targets.clone();
                }
                if shell_action.open_new_memo {
                    let ymd = self.shell.cal.selected.format("%Y-%m-%d").to_string();
                    shell::begin_new_memo(
                        show_new_memo,
                        editing,
                        &mut self.edit_form,
                        self.shell.nav,
                        Some(ymd.as_str()),
                        &rt.cfg.data_dir,
                    );
                    // 清脏 tween（含重复点新建），本帧后续 begin 从 0 滑入
                    page_shell::drawer_anim_reset(ctx, shell::NEW_MEMO_DRAWER_SALT);
                    *status_line = "在右侧抽屉填写新建，保存后写入本机".into();
                }
                if let Some(id) = shell_action.open_sticky {
                    let list = rt.svc.list();
                    if let Some(existing) = self.stickies.get(&id) {
                        existing.lock().request_focus = true;
                        *status_line = "已聚焦桌面便签".into();
                    } else if let Some(m) = list.iter().find(|m| m.id == id) {
                        self.stickies
                            .insert(id, sticky_note::StickyNote::from_memo(m));
                        self.sticky_session_dirty = true;
                        *status_line = "已生成桌面便签".into();
                    }
                }
                if shell_action.defer_lock_after_task {
                    self.pending_lock_after_task = true;
                    ctx.request_repaint();
                }
                if shell_action.switch {
                    self.pending_switch_person = true;
                }
                if shell_action.sync {
                    *peers = rt.engine.connected_peers();
                    *discovered = rt.engine.discovered_peers();
                    match rt.svc.republish_private_backups() {
                        Ok(n) => {
                            self.pending_sync_hint = 0;
                            *status_line = if n > 0 {
                                format!("已同步：重新推送 {n} 条私人备份")
                            } else {
                                "已同步：节点列表已刷新".into()
                            };
                        }
                        Err(e) => {
                            *status_line = format!("同步失败: {e}");
                        }
                    }
                }


                // 新建：独立表单抽屉。任务/备忘/私密的编辑在壳层与预览共用同一抽屉。
                {
                    let host = self.shell.content_host;
                    let new_title = match self.shell.nav {
                        nav::NavItem::Calendar => "新建任务",
                        nav::NavItem::GenderPrivate => "新建私密",
                        _ => "新建备忘",
                    };
                    // 固定 salt：切顶栏不得分裂 anim id（顶栏已在新建/编辑时锁定）
                    let new_salt = shell::NEW_MEMO_DRAWER_SALT;
                    if *show_new_memo {
                        let mut drawer_dirty = false;
                        let drawer_action = shell::show_memo_form_drawer(
                            ctx,
                            host,
                            new_title,
                            &mut self.edit_form,
                            FormMode::Create,
                            new_salt,
                            &rt.cfg.data_dir,
                            &mut self.shell.drawer.form_w,
                            &mut drawer_dirty,
                        );
                        if drawer_dirty {
                            self.shell.drawer.mark_dirty();
                        }
                        match drawer_action {
                            MemoDrawerAction::Save => {
                                match memo_form::validate_title(&self.edit_form) {
                                    Ok(_) => {
                                        shell::submit_new_memo(
                                            &rt.svc,
                                            &self.edit_form,
                                            &self.tx,
                                        );
                                        *status_line = "正在创建…".into();
                                        *show_new_memo = false;
                                        self.edit_form.clear();
                                        Self::clear_editor_session_at(
                                            Path::new(&self.cfg.data_dir),
                                            &mut self.editor_resume,
                                            rt.svc.session_fp(),
                                        );
                                    }
                                    Err(e) => {
                                        *status_line = e;
                                        self.edit_form.request_title_focus();
                                    }
                                }
                            }
                            MemoDrawerAction::Dismiss => {
                                *show_new_memo = false;
                                self.edit_form.clear();
                                Self::clear_editor_session_at(
                                    Path::new(&self.cfg.data_dir),
                                    &mut self.editor_resume,
                                    rt.svc.session_fp(),
                                );
                                if status_line.starts_with("在右侧抽屉填写新建")
                                    || status_line.starts_with("填写新建")
                                {
                                    *status_line = "已取消新建".into();
                                }
                            }
                            MemoDrawerAction::None => {}
                        }
                    } else if *editing && !shell::nav_uses_unified_side_drawer(self.shell.nav)
                    {
                        // 其它页仍用独立编辑抽屉；任务/备忘/私密由壳层统一抽屉承接
                        let editing_hub = selected
                            .as_ref()
                            .is_some_and(|id| shell::is_private_hub(id));
                        if !editing_hub {
                            let all = rt.svc.list();
                            let mut drawer_dirty = false;
                            let drawer_action = shell::show_memo_form_drawer(
                                ctx,
                                host,
                                "编辑备忘",
                                &mut self.edit_form,
                                FormMode::Edit,
                                "misc_edit_drawer",
                                &rt.cfg.data_dir,
                                &mut self.shell.drawer.form_w,
                                &mut drawer_dirty,
                            );
                            if drawer_dirty {
                                self.shell.drawer.mark_dirty();
                            }
                            match drawer_action {
                                MemoDrawerAction::Save => {
                                    shell::save_memo_edit(
                                        &rt.svc,
                                        &all,
                                        selected,
                                        title_draft,
                                        body_draft,
                                        visibility_draft,
                                        category_draft,
                                        due_date_draft,
                                        end_date_draft,
                                        tags_draft,
                                        done_draft,
                                        priority_draft,
                                        editing,
                                        &mut self.memo_doc,
                                        &mut self.edit_form,
                                        status_line,
                                        &self.tx,
                                    );
                                    if !*editing {
                                        Self::clear_editor_session_at(
                                            Path::new(&self.cfg.data_dir),
                                            &mut self.editor_resume,
                                            rt.svc.session_fp(),
                                        );
                                    }
                                }
                                MemoDrawerAction::Dismiss => {
                                    shell::cancel_memo_edit(
                                        selected,
                                        &all,
                                        title_draft,
                                        body_draft,
                                        visibility_draft,
                                        category_draft,
                                        due_date_draft,
                                        end_date_draft,
                                        tags_draft,
                                        done_draft,
                                        priority_draft,
                                        editing,
                                        &mut self.memo_doc,
                                        &mut self.edit_form,
                                    );
                                    Self::clear_editor_session_at(
                                        Path::new(&self.cfg.data_dir),
                                        &mut self.editor_resume,
                                        rt.svc.session_fp(),
                                    );
                                }
                                MemoDrawerAction::None => {}
                            }
                        }
                    }
                }

                if (editing_before && !*editing) || (new_memo_before && !*show_new_memo) {
                    Self::clear_editor_session_at(
                        Path::new(&self.cfg.data_dir),
                        &mut self.editor_resume,
                        rt.svc.session_fp(),
                    );
                }

                if *show_delete {
                    let delete_id = selected.clone().unwrap_or_default();
                    let delete_title = if title_draft.trim().is_empty() {
                        "(无标题)".to_string()
                    } else {
                        title_draft.clone()
                    };
                    if delete_id.is_empty() {
                        *show_delete = false;
                    } else {
                        let modal = theme::begin_modal(ctx, "memo_trash_modal");
                        theme::modal_confirm(ctx, "移入回收站")
                            .id(modal.window_id)
                            .show(ctx, |ui| {
                                ui.label(
                                    RichText::new(format!("将「{delete_title}」移入回收站？"))
                                        .strong(),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    theme::muted_label(format!(
                                        "可在 {} 天内从回收站恢复；主机仍保留加密备份。彻底清除后才会从主机抹除。",
                                        memo_core::TRASH_RETENTION_DAYS
                                    )),
                                );
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add({
                                            egui::Button::new(
                                                RichText::new("移入回收站")
                                                    .color(Color32::WHITE)
                                                    .strong(),
                                            )
                                            .fill(theme::danger())
                                        })
                                        .clicked()
                                    {
                                        let svc = rt.svc.clone();
                                        let id2 = delete_id.clone();
                                        let tx = self.tx.clone();
                                        *show_delete = false;
                                        std::thread::spawn(move || match svc.delete(&id2) {
                                            Ok(()) => {
                                                let _ = tx.send(BgMsg::Deleted);
                                                let _ = tx.send(BgMsg::Refresh);
                                            }
                                            Err(e) => {
                                                let _ = tx.send(BgMsg::Error(e.to_string()));
                                            }
                                        });
                                    }
                                    if ui.button("取消").clicked() {
                                        *show_delete = false;
                                    }
                                });
                            });
                        if modal.end(ctx, true) {
                            *show_delete = false;
                        }
                    }
                }

                if let Some(pid) = purge_confirm_id.clone() {
                    let purge_title = rt
                        .svc
                        .list_trash()
                        .into_iter()
                        .find(|m| m.id == pid)
                        .map(|m| {
                            if m.title.is_empty() {
                                "(无标题)".into()
                            } else {
                                m.title
                            }
                        })
                        .unwrap_or_else(|| "(未知)".into());
                    let modal = theme::begin_modal(ctx, "purge_confirm_modal");
                    theme::modal_confirm(ctx, "彻底清除")
                        .id(modal.window_id)
                        .show(ctx, |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "彻底清除「{purge_title}」？此操作不可恢复，并将从主机删除备份。"
                                ))
                                .strong()
                                .color(theme::danger()),
                            );
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add({
                                        egui::Button::new(
                                            RichText::new("确认彻底清除")
                                                .color(Color32::WHITE)
                                                .strong(),
                                        )
                                        .fill(theme::danger())
                                    })
                                    .clicked()
                                {
                                    let svc = rt.svc.clone();
                                    let id = pid.clone();
                                    let tx = self.tx.clone();
                                    *purge_confirm_id = None;
                                    std::thread::spawn(move || match svc.purge_deleted(&id) {
                                        Ok(()) => {
                                            let _ = tx.send(BgMsg::Refresh);
                                        }
                                        Err(e) => {
                                            let _ = tx.send(BgMsg::Error(e.to_string()));
                                        }
                                    });
                                }
                                if ui.button("取消").clicked() {
                                    *purge_confirm_id = None;
                                }
                            });
                        });
                    if modal.end(ctx, true) {
                        *purge_confirm_id = None;
                    }
                }

                if *show_change_pw {
                    let modal = theme::begin_modal(ctx, "change_pw_modal");
                    let mut open = true;
                    let mut close_requested = false;
                    theme::modal_fixed(ctx, "重置密码", [420.0, 320.0])
                        .id(modal.window_id)
                        .open(&mut open)
                        .show(ctx, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(
                                    "修改本机登录密码。用于进入本机并加密数据备份；不加密 .memokey。",
                                )
                                .size(13.0)
                                .color(theme::text_muted()),
                            );
                            ui.add_space(10.0);
                            let pw_w = ui.available_width().max(120.0);
                            ui.label(
                                RichText::new("当前密码")
                                    .size(12.0)
                                    .color(theme::text_muted()),
                            );
                            theme::password_field(
                                ui,
                                &mut self.change_pw_old,
                                "当前登录密码",
                                pw_w,
                                32.0,
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("新密码（至少 6 位）")
                                    .size(12.0)
                                    .color(theme::text_muted()),
                            );
                            theme::password_field(
                                ui,
                                &mut self.change_pw_new,
                                "新登录密码",
                                pw_w,
                                32.0,
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("确认新密码")
                                    .size(12.0)
                                    .color(theme::text_muted()),
                            );
                            theme::password_field(
                                ui,
                                &mut self.change_pw_new2,
                                "再输入一次",
                                pw_w,
                                32.0,
                            );
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                if theme::success_button(ui, "保存新密码").clicked() {
                                    let old = self.change_pw_old.clone();
                                    let new = self.change_pw_new.clone();
                                    let new2 = self.change_pw_new2.clone();
                                    match memo_core::identity_keys::validate_local_password(
                                        &new, &new2,
                                    ) {
                                        Ok(()) => {
                                            let data = PathBuf::from(&rt.cfg.data_dir);
                                            let fp = rt.svc.session_fp().to_string();
                                            match memo_core::identity_keys::IdentityKeys::change_local_password(
                                                data.as_path(),
                                                &fp,
                                                &old,
                                                &new,
                                            ) {
                                                Ok(()) => {
                                                    self.session_login_password =
                                                        Some(new.trim().to_string());
                                                    self.change_pw_old.clear();
                                                    self.change_pw_new.clear();
                                                    self.change_pw_new2.clear();
                                                    *status_line = "登录密码已更新".into();
                                                    close_requested = true;
                                                }
                                                Err(e) => {
                                                    *status_line = format!("改密失败: {e}");
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            *status_line = e.to_string();
                                        }
                                    }
                                }
                                if theme::ghost_button(ui, "取消").clicked() {
                                    close_requested = true;
                                }
                            });
                        });
                    if modal.end(ctx, open) || close_requested || !open {
                        *show_change_pw = false;
                        self.change_pw_old.clear();
                        self.change_pw_new.clear();
                        self.change_pw_new2.clear();
                    }
                }

                if *show_export {
                    let scope = if export_ids.is_some() {
                        "导出当前备忘"
                    } else {
                        "导出全部备忘"
                    };
                    let modal = theme::begin_modal(ctx, "export_memo_modal");
                    let mut open = true;
                    theme::modal_fixed(ctx, scope, [480.0, 248.0])
                        .id(modal.window_id)
                        .open(&mut open)
                        .show(ctx, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new("导出为明文 TXT，请输入登录密码确认，并选择保存位置。")
                                    .strong(),
                            );
                            ui.add_space(8.0);
                            ui.label("登录密码");
                            let pw_w = ui.available_width().max(120.0);
                            theme::password_field(ui, export_pw, "输入登录密码", pw_w, 34.0);
                            ui.add_space(8.0);
                            ui.label("保存路径");
                            ui.horizontal(|ui| {
                                let browse_w = 64.0;
                                let gap = ui.spacing().item_spacing.x;
                                let path_w =
                                    (ui.available_width() - browse_w - gap).max(1.0);
                                ui.add(
                                    egui::TextEdit::singleline(export_path)
                                        .desired_width(path_w)
                                        .hint_text(theme::hint("例如 C:\\Users\\…\\memo.txt")),
                                );
                                if ui
                                    .add_sized(
                                        [browse_w, 24.0],
                                        egui::Button::new("浏览…"),
                                    )
                                    .clicked()
                                {
                                    if let Some(p) =
                                        save_dialog::save_txt_dialog(export_path.as_str())
                                    {
                                        *export_path = p.display().to_string();
                                    }
                                }
                            });
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                let can = !export_pw.is_empty() && !export_path.trim().is_empty();
                                if ui
                                    .add_enabled(can, {
                                        egui::Button::new(
                                            RichText::new("确认导出")
                                                .color(Color32::WHITE)
                                                .strong(),
                                        )
                                        .fill(theme::success())
                                    })
                                    .clicked()
                                {
                                    let svc = rt.svc.clone();
                                    let pw = export_pw.clone();
                                    let session_pw =
                                        self.session_login_password.clone().unwrap_or_default();
                                    let path =
                                        std::path::PathBuf::from(export_path.trim());
                                    let ids = export_ids.clone();
                                    let tx = self.tx.clone();
                                    *show_export = false;
                                    export_pw.clear();
                                    std::thread::spawn(move || {
                                        if pw.trim().is_empty() || pw.trim() != session_pw.trim()
                                        {
                                            let _ = tx.send(BgMsg::Error(
                                                "登录密码错误".into(),
                                            ));
                                            return;
                                        }
                                        let path_disp = path.display().to_string();
                                        match svc.export_txt(path, ids) {
                                            Ok(()) => {
                                                let _ = tx.send(BgMsg::Info(format!(
                                                    "已导出: {path_disp}"
                                                )));
                                            }
                                            Err(e) => {
                                                let _ = tx.send(BgMsg::Error(e.to_string()));
                                            }
                                        }
                                    });
                                }
                                if ui.button("取消").clicked() {
                                    *show_export = false;
                                    export_pw.clear();
                                }
                            });
                        });
                    if modal.end(ctx, open) {
                        *show_export = false;
                        export_pw.clear();
                    }
                }

                if *show_history {
                    let modal = theme::begin_modal(ctx, "history_modal");
                    let mut open = true;
                    theme::modal_fixed(
                        ctx,
                        history_view::entity_window_title(history_entity),
                        [560.0, 520.0],
                    )
                        .id(modal.window_id)
                        .open(&mut open)
                        .show(ctx, |ui| {
                            let svc = rt.svc.clone();
                            let dn = move |nid: &str| svc.display_name_for(nid);
                            history_view::show_list(
                                ui,
                                history_events,
                                &dn,
                                history_sel_a,
                                history_sel_b,
                            );
                            history_view::show_diff(
                                ui,
                                history_events,
                                *history_sel_a,
                                *history_sel_b,
                            );
                            ui.add_space(8.0);
                            if ui.button("关闭").clicked() {
                                *show_history = false;
                            }
                        });
                    if modal.end(ctx, open) {
                        *show_history = false;
                    }
                }

                if *show_backup {
                    let title = if *backup_import {
                        "导入加密备份"
                    } else {
                        "导出加密备份"
                    };
                    let modal = theme::begin_modal(ctx, "backup_modal");
                    let mut open = true;
                    theme::modal_fixed(ctx, title, [480.0, 280.0])
                        .id(modal.window_id)
                        .open(&mut open)
                        .show(ctx, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(if *backup_import {
                                    "选择 .memobak 文件并输入备份时使用的登录密码。"
                                } else {
                                    "将全部备忘加密导出为 .memobak（使用当前登录密码加密）。"
                                })
                                .weak(),
                            );
                            ui.add_space(8.0);
                            if *backup_import {
                                ui.label("登录密码（备份时使用的）");
                                let pw_w = ui.available_width().max(120.0);
                                // 默认填入当前会话密码，可改
                                if backup_pw.is_empty() {
                                    if let Some(p) = self.session_login_password.as_ref() {
                                        *backup_pw = p.clone();
                                    }
                                }
                                theme::password_field(
                                    ui,
                                    backup_pw,
                                    "登录密码",
                                    pw_w,
                                    34.0,
                                );
                                ui.add_space(6.0);
                            }
                            ui.label("文件路径");
                            ui.horizontal(|ui| {
                                let browse_w = 64.0;
                                let gap = ui.spacing().item_spacing.x;
                                let path_w =
                                    (ui.available_width() - browse_w - gap).max(1.0);
                                ui.add(
                                    egui::TextEdit::singleline(backup_path)
                                        .desired_width(path_w)
                                        .hint_text(theme::hint("*.memobak")),
                                );
                                if ui
                                    .add_sized(
                                        [browse_w, 24.0],
                                        egui::Button::new("浏览…"),
                                    )
                                    .clicked()
                                {
                                    if *backup_import {
                                        // 仍用保存对话框风格；用户可改扩展名路径
                                        if let Some(p) =
                                            save_dialog::save_txt_dialog(backup_path.as_str())
                                        {
                                            let mut p = p;
                                            if p.extension().is_none() {
                                                p.set_extension("memobak");
                                            }
                                            *backup_path = p.display().to_string();
                                        }
                                    } else if let Some(p) =
                                        save_dialog::save_txt_dialog(backup_path.as_str())
                                    {
                                        let mut p = p;
                                        p.set_extension("memobak");
                                        *backup_path = p.display().to_string();
                                    }
                                }
                            });
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                let session_ok = self.session_login_password.is_some();
                                let can = if *backup_import {
                                    !backup_pw.is_empty() && !backup_path.trim().is_empty()
                                } else {
                                    session_ok && !backup_path.trim().is_empty()
                                };
                                let btn = if *backup_import { "确认导入" } else { "确认备份" };
                                if ui
                                    .add_enabled(
                                        can,
                                        egui::Button::new(
                                            RichText::new(btn).color(Color32::WHITE).strong(),
                                        )
                                        .fill(theme::accent()),
                                    )
                                    .clicked()
                                {
                                    let svc = rt.svc.clone();
                                    let pw = if *backup_import {
                                        backup_pw.clone()
                                    } else {
                                        self.session_login_password.clone().unwrap_or_default()
                                    };
                                    let path =
                                        std::path::PathBuf::from(backup_path.trim());
                                    let import = *backup_import;
                                    let tx = self.tx.clone();
                                    *show_backup = false;
                                    backup_pw.clear();
                                    if pw.trim().is_empty() {
                                        let _ = self.tx.send(BgMsg::Error(
                                            "无登录密码，请重新登录后再备份".into(),
                                        ));
                                        return;
                                    }
                                    std::thread::spawn(move || {
                                        let res = if import {
                                            svc.import_backup(&path, &pw).map(|n| {
                                                format!("已导入 {n} 条备忘")
                                            })
                                        } else {
                                            svc.export_backup(&path, &pw).map(|_| {
                                                format!("已备份: {}", path.display())
                                            })
                                        };
                                        match res {
                                            Ok(msg) => {
                                                let _ = tx.send(BgMsg::Info(msg));
                                                let _ = tx.send(BgMsg::Refresh);
                                            }
                                            Err(e) => {
                                                let _ = tx.send(BgMsg::Error(e.to_string()));
                                            }
                                        }
                                    });
                                }
                                if ui.button("取消").clicked() {
                                    *show_backup = false;
                                    backup_pw.clear();
                                }
                            });
                        });
                    if modal.end(ctx, open) {
                        *show_backup = false;
                        backup_pw.clear();
                    }
                }

                if *show_settings {
                    let modal = theme::begin_modal(ctx, "settings_modal");
                    let mut open = true;
                    theme::modal_fixed(ctx, "设置", [640.0, 520.0])
                        .id(modal.window_id)
                        .open(&mut open)
                        .show(ctx, |ui| {
                            let footer_reserve = 52.0;
                            let body_h = (ui.available_height() - footer_reserve).max(120.0);
                            let field_w = 360.0_f32;

                            Frame::none()
                                .fill(theme::card())
                                .inner_margin(Margin::symmetric(20.0, 12.0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width().max(1.0));
                                    egui::ScrollArea::vertical()
                                        .id_source("settings_scroll")
                                        .max_height(body_h)
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());

                                            // —— 外观 ——
                                            settings_section(ui, "外观");
                                            settings_item_header(
                                                ui,
                                                "主题",
                                                "浅色 / 深色 / 跟随系统 / 柔美（玫瑰雾面，更舒心）。切换后立即生效并自动保存。",
                                            );
                                            egui::ComboBox::from_id_source("settings_theme")
                                                .selected_text(theme_pref_label(
                                                    settings_draft.theme,
                                                ))
                                                .width(field_w)
                                                .show_ui(ui, |ui| {
                                                    for p in [
                                                        memo_core::ThemePreference::System,
                                                        memo_core::ThemePreference::Light,
                                                        memo_core::ThemePreference::Dark,
                                                        memo_core::ThemePreference::Blush,
                                                    ] {
                                                        ui.selectable_value(
                                                            &mut settings_draft.theme,
                                                            p,
                                                            theme_pref_label(p),
                                                        );
                                                    }
                                                });
                                            if settings_draft.theme != self.cfg.theme {
                                                self.cfg.theme = settings_draft.theme;
                                                self.last_theme_mode = None;
                                                let mut persist = self.cfg.clone();
                                                persist.theme = settings_draft.theme;
                                                let _ = config::save_settings(&persist);
                                                ctx.request_repaint();
                                            }

                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "自动锁定",
                                                "主界面无操作多久后回到身份选择。更改后立即生效并自动保存。",
                                            );
                                            egui::ComboBox::from_id_source("settings_auto_lock")
                                                .selected_text(auto_lock_label(
                                                    settings_draft.auto_lock_minutes,
                                                ))
                                                .width(field_w)
                                                .show_ui(ui, |ui| {
                                                    for m in [0, 1, 3, 5, 10, 15, 30, 60] {
                                                        ui.selectable_value(
                                                            &mut settings_draft.auto_lock_minutes,
                                                            m,
                                                            auto_lock_label(m),
                                                        );
                                                    }
                                                });
                                            if settings_draft.auto_lock_minutes
                                                != self.cfg.auto_lock_minutes
                                            {
                                                self.cfg.auto_lock_minutes =
                                                    settings_draft.auto_lock_minutes;
                                                self.last_input_at = Instant::now();
                                                let mut persist = self.cfg.clone();
                                                persist.auto_lock_minutes =
                                                    settings_draft.auto_lock_minutes;
                                                let _ = config::save_settings(&persist);
                                                ctx.request_repaint();
                                            }

                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "开机自启动",
                                                if autostart::supported() {
                                                    "开机后在托盘启动本程序；仍需解锁身份后才显示便签。"
                                                } else {
                                                    "当前系统不支持写入开机启动项。"
                                                },
                                            );
                                            ui.add_enabled_ui(autostart::supported(), |ui| {
                                                if ui
                                                    .checkbox(
                                                        &mut settings_draft.start_on_boot,
                                                        "开机时启动 Memo",
                                                    )
                                                    .on_hover_text(
                                                        "与登录页勾选同步；开机后托盘启动，仍需解锁身份",
                                                    )
                                                    .changed()
                                                {
                                                    let want = settings_draft.start_on_boot;
                                                    match autostart::apply_and_persist(
                                                        &mut self.cfg,
                                                        want,
                                                    ) {
                                                        Ok(msg) => {
                                                            settings_draft.start_on_boot =
                                                                self.cfg.start_on_boot;
                                                            *status_line = msg.into();
                                                        }
                                                        Err(e) => {
                                                            settings_draft.start_on_boot = !want;
                                                            self.cfg.start_on_boot = !want;
                                                            *status_line = format!(
                                                                "开机自启动设置失败: {e}"
                                                            );
                                                        }
                                                    }
                                                    ctx.request_repaint();
                                                }
                                            });

                                            settings_item_gap(ui);
                                            // —— 关怀与健康（私密页模块开关 / 调参）——
                                            settings_section(ui, "关怀与健康");
                                            gender_private_view::show_care_health_settings(
                                                ui,
                                                &rt.svc,
                                                status_line,
                                            );

                                            settings_item_gap(ui);
                                            // —— 本机 ——
                                            settings_section(ui, "本机");
                                            settings_item_header(
                                                ui,
                                                "节点 ID",
                                                "本机在局域网中的唯一标识。更改后需重启。",
                                            );
                                            settings_text_field(
                                                ui,
                                                &mut settings_draft.node_id,
                                                "例如 memo-pc",
                                                field_w,
                                            );
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "显示名",
                                                "仅在本机界面展示，不影响同步身份。",
                                            );
                                            settings_text_field(
                                                ui,
                                                &mut settings_draft.node_display_name,
                                                "可选",
                                                field_w,
                                            );
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "当前身份性别",
                                                "可选填写，用于节点展示等；开通「性别私密」无需设置性别。",
                                            );
                                            {
                                                let mut gender = rt.svc.current_person_gender();
                                                let label = gender.label();
                                                egui::ComboBox::from_id_source("settings_gender")
                                                    .selected_text(label)
                                                    .width(field_w)
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(
                                                            &mut gender,
                                                            Gender::Female,
                                                            Gender::Female.label(),
                                                        );
                                                        ui.selectable_value(
                                                            &mut gender,
                                                            Gender::Male,
                                                            Gender::Male.label(),
                                                        );
                                                    });
                                                if gender != rt.svc.current_person_gender()
                                                    && !matches!(gender, Gender::Unknown)
                                                {
                                                    match rt.svc.set_current_person_gender(gender)
                                                    {
                                                        Ok(()) => {
                                                            *status_line = format!(
                                                                "已将当前身份性别设为「{}」",
                                                                gender.label()
                                                            );
                                                            ctx.request_repaint();
                                                        }
                                                        Err(e) => {
                                                            let _ = self
                                                                .tx
                                                                .send(BgMsg::Error(e.to_string()));
                                                        }
                                                    }
                                                }
                                            }
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "生日（可选）",
                                                "用于日历运势推演；不填则按娱乐模式。填写后更准。",
                                            );
                                            {
                                                let (cur_ymd, cur_hour) =
                                                    rt.svc.current_person_birth();
                                                let draft_id = egui::Id::new("settings_birth_draft");
                                                let hour_id =
                                                    egui::Id::new("settings_birth_hour_draft");
                                                let sync_id =
                                                    egui::Id::new("settings_birth_synced");
                                                let need_sync = ui.data(|d| {
                                                    d.get_temp::<(String, Option<u8>)>(sync_id)
                                                        != Some((cur_ymd.clone(), cur_hour))
                                                });
                                                if need_sync {
                                                    ui.data_mut(|d| {
                                                        d.insert_temp(draft_id, cur_ymd.clone());
                                                        d.insert_temp(hour_id, cur_hour);
                                                        d.insert_temp(
                                                            sync_id,
                                                            (cur_ymd.clone(), cur_hour),
                                                        );
                                                    });
                                                }
                                                let mut birth_ymd = ui.data(|d| {
                                                    d.get_temp::<String>(draft_id)
                                                        .unwrap_or_else(|| cur_ymd.clone())
                                                });
                                                let mut birth_hour: Option<u8> = ui
                                                    .data(|d| {
                                                        d.get_temp::<Option<u8>>(hour_id)
                                                            .unwrap_or(cur_hour)
                                                    });
                                                // 设置窗在 ScrollArea+遮罩下，popup/ComboBox 一点就关；改内联选日与时辰
                                                let picker_open_id =
                                                    egui::Id::new("settings_birth_picker_open");
                                                let mut picker_open = ui.data(|d| {
                                                    d.get_temp::<bool>(picker_open_id)
                                                        .unwrap_or(false)
                                                });
                                                let display = if birth_ymd.trim().is_empty() {
                                                    "选择生日…".to_string()
                                                } else if date_field::parse_ymd(&birth_ymd)
                                                    .is_some()
                                                {
                                                    birth_ymd.trim().chars().take(10).collect()
                                                } else {
                                                    format!("{} (无效)", birth_ymd.trim())
                                                };
                                                ui.horizontal(|ui| {
                                                    let open_btn = ui.add(
                                                        egui::Button::new(
                                                            RichText::new(format!("📅  {display}"))
                                                                .size(13.5)
                                                                .color(theme::text()),
                                                        )
                                                        .fill(theme::card())
                                                        .stroke(egui::Stroke::new(
                                                            1.0,
                                                            theme::border(),
                                                        ))
                                                        .rounding(egui::Rounding::same(8.0))
                                                        .min_size(egui::vec2(
                                                            (field_w - 56.0).max(120.0),
                                                            30.0,
                                                        )),
                                                    );
                                                    if open_btn.clicked() {
                                                        picker_open = !picker_open;
                                                    }
                                                    if !birth_ymd.trim().is_empty()
                                                        && ui
                                                            .add(
                                                                egui::Button::new(
                                                                    RichText::new("清除")
                                                                        .size(12.0)
                                                                        .color(theme::text_muted()),
                                                                )
                                                                .frame(false),
                                                            )
                                                            .clicked()
                                                    {
                                                        birth_ymd.clear();
                                                        picker_open = false;
                                                    }
                                                });
                                                if picker_open {
                                                    ui.add_space(4.0);
                                                    egui::Frame::none()
                                                        .fill(theme::panel())
                                                        .stroke(egui::Stroke::new(
                                                            1.0,
                                                            theme::border(),
                                                        ))
                                                        .rounding(egui::Rounding::same(8.0))
                                                        .inner_margin(egui::Margin::same(8.0))
                                                        .show(ui, |ui| {
                                                            ui.set_max_width(field_w);
                                                            if date_field::show_picker_body(
                                                                ui,
                                                                "settings_birth_inline",
                                                                &mut birth_ymd,
                                                                false,
                                                            ) {
                                                                picker_open = false;
                                                            }
                                                        });
                                                }
                                                ui.add_space(4.0);
                                                ui.label(
                                                    RichText::new("时辰（可选）")
                                                        .size(12.0)
                                                        .color(theme::text_muted()),
                                                );
                                                ui.horizontal(|ui| {
                                                    let mut known = birth_hour.is_some();
                                                    if ui
                                                        .checkbox(&mut known, "已知钟点")
                                                        .changed()
                                                    {
                                                        birth_hour =
                                                            if known { Some(12) } else { None };
                                                    }
                                                    if let Some(h) = birth_hour.as_mut() {
                                                        let mut hv = i32::from(*h);
                                                        if ui
                                                            .add(
                                                                egui::DragValue::new(&mut hv)
                                                                    .clamp_range(0..=23)
                                                                    .suffix(" 时")
                                                                    .speed(0.2),
                                                            )
                                                            .changed()
                                                        {
                                                            *h = hv.clamp(0, 23) as u8;
                                                        }
                                                    } else {
                                                        ui.label(
                                                            RichText::new("未知")
                                                                .size(12.5)
                                                                .color(theme::text_muted()),
                                                        );
                                                    }
                                                });
                                                ui.data_mut(|d| {
                                                    d.insert_temp(draft_id, birth_ymd.clone());
                                                    d.insert_temp(hour_id, birth_hour);
                                                    d.insert_temp(picker_open_id, picker_open);
                                                });
                                                if birth_ymd != cur_ymd || birth_hour != cur_hour {
                                                    ui.add_space(4.0);
                                                    if ui
                                                        .add(
                                                            egui::Button::new(
                                                                RichText::new("保存生辰")
                                                                    .size(12.5)
                                                                    .color(Color32::WHITE),
                                                            )
                                                            .fill(theme::accent())
                                                            .rounding(egui::Rounding::same(8.0)),
                                                        )
                                                        .clicked()
                                                    {
                                                        match rt.svc.set_current_person_birth(
                                                            &birth_ymd,
                                                            birth_hour,
                                                        ) {
                                                            Ok(()) => {
                                                                ui.data_mut(|d| {
                                                                    d.insert_temp(
                                                                        sync_id,
                                                                        (
                                                                            birth_ymd.clone(),
                                                                            birth_hour,
                                                                        ),
                                                                    );
                                                                });
                                                                *status_line =
                                                                    if birth_ymd.trim().is_empty()
                                                                    {
                                                                        "已清除生日，运势为娱乐模式"
                                                                            .into()
                                                                    } else {
                                                                        "已保存生日".into()
                                                                    };
                                                                ctx.request_repaint();
                                                            }
                                                            Err(e) => {
                                                                let _ = self.tx.send(
                                                                    BgMsg::Error(e.to_string()),
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "修改登录密码",
                                                "用于进入本机并加密数据备份；不加密 .memokey。",
                                            );
                                            {
                                                let pw_w = field_w.min(ui.available_width());
                                                ui.label(
                                                    RichText::new("当前密码")
                                                        .size(12.0)
                                                        .color(theme::text_muted()),
                                                );
                                                theme::password_field(
                                                    ui,
                                                    &mut self.change_pw_old,
                                                    "当前登录密码",
                                                    pw_w,
                                                    32.0,
                                                );
                                                ui.add_space(4.0);
                                                ui.label(
                                                    RichText::new("新密码（至少 6 位）")
                                                        .size(12.0)
                                                        .color(theme::text_muted()),
                                                );
                                                theme::password_field(
                                                    ui,
                                                    &mut self.change_pw_new,
                                                    "新登录密码",
                                                    pw_w,
                                                    32.0,
                                                );
                                                ui.add_space(4.0);
                                                ui.label(
                                                    RichText::new("确认新密码")
                                                        .size(12.0)
                                                        .color(theme::text_muted()),
                                                );
                                                theme::password_field(
                                                    ui,
                                                    &mut self.change_pw_new2,
                                                    "再输入一次",
                                                    pw_w,
                                                    32.0,
                                                );
                                                ui.add_space(6.0);
                                                if theme::success_button(ui, "保存新密码").clicked()
                                                {
                                                    let old = self.change_pw_old.clone();
                                                    let new = self.change_pw_new.clone();
                                                    let new2 = self.change_pw_new2.clone();
                                                    match memo_core::identity_keys::validate_local_password(
                                                        &new, &new2,
                                                    ) {
                                                        Ok(()) => {
                                                            let data = PathBuf::from(&rt.cfg.data_dir);
                                                            let fp = rt.svc.session_fp().to_string();
                                                            match memo_core::identity_keys::IdentityKeys::change_local_password(
                                                                data.as_path(),
                                                                &fp,
                                                                &old,
                                                                &new,
                                                            ) {
                                                                Ok(()) => {
                                                                    self.session_login_password =
                                                                        Some(new.trim().to_string());
                                                                    self.change_pw_old.clear();
                                                                    self.change_pw_new.clear();
                                                                    self.change_pw_new2.clear();
                                                                    *status_line =
                                                                        "登录密码已更新".into();
                                                                }
                                                                Err(e) => {
                                                                    *status_line =
                                                                        format!("改密失败: {e}");
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            *status_line = e.to_string();
                                                        }
                                                    }
                                                }
                                            }
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "数据目录",
                                                "备忘、密钥与本地库的存储路径。更改后需重启。",
                                            );
                                            ui.horizontal(|ui| {
                                                let browse_w = 64.0;
                                                let gap = ui.spacing().item_spacing.x;
                                                let path_w = (ui.available_width()
                                                    - browse_w
                                                    - gap)
                                                    .clamp(1.0, field_w.max(480.0));
                                                let te = ui.add(
                                                    egui::TextEdit::singleline(
                                                        &mut settings_draft.data_dir,
                                                    )
                                                    .desired_width(path_w)
                                                    .hint_text(theme::hint("绝对路径")),
                                                );
                                                if ui
                                                    .add_sized(
                                                        [browse_w, te.rect.height()],
                                                        egui::Button::new("浏览…"),
                                                    )
                                                    .clicked()
                                                {
                                                    if let Some(p) =
                                                        save_dialog::pick_folder_dialog(
                                                            &settings_draft.data_dir,
                                                        )
                                                    {
                                                        settings_draft.data_dir =
                                                            p.display().to_string();
                                                    }
                                                }
                                            });
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "监听端口",
                                                "TCP 同步监听端口。更改后需重启。",
                                            );
                                            let mut port =
                                                settings_draft.listen_port.to_string();
                                            if settings_text_field(
                                                ui, &mut port, "例如 17890", 120.0,
                                            ) {
                                                if let Ok(p) = port.parse() {
                                                    settings_draft.listen_port = p;
                                                }
                                            }

                                            settings_item_gap(ui);
                                            // —— 网络 ——
                                            settings_section(ui, "网络");
                                            settings_item_header(
                                                ui,
                                                "节点角色",
                                                "主机每 10s 广播并受理登记；从机不广播，发现主机后主动连接。更改后需重启。",
                                            );
                                            egui::ComboBox::from_id_source("settings_node_role")
                                                .selected_text(
                                                    settings_draft.node_role.label(),
                                                )
                                                .width(field_w)
                                                .show_ui(ui, |ui| {
                                                    ui.selectable_value(
                                                        &mut settings_draft.node_role,
                                                        NodeRole::Master,
                                                        "主机",
                                                    );
                                                    ui.selectable_value(
                                                        &mut settings_draft.node_role,
                                                        NodeRole::Slave,
                                                        "从机",
                                                    );
                                                });
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "局域网自动发现",
                                                "同网段通过 UDP 17000 发现对端。关闭后只连接下方「对端列表」里手填的地址。",
                                            );
                                            ui.checkbox(
                                                &mut settings_draft.lan_discovery,
                                                "启用局域网自动发现",
                                            );
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "显示本节点",
                                                "显示=对外广播，可被同网段发现；隐藏=不广播，别人扫不到本机。仅主机可改（从机本身不广播）。",
                                            );
                                            ui.add_enabled_ui(
                                                settings_draft.node_role.is_master(),
                                                |ui| {
                                                    ui.checkbox(
                                                        &mut settings_draft.node_visible,
                                                        "显示本节点（可被发现）",
                                                    );
                                                },
                                            );
                                            if !settings_draft.node_role.is_master() {
                                                ui.label(
                                                    theme::muted_label(
                                                        "当前为从机，不会对外广播。",
                                                    )
                                                    .small(),
                                                );
                                            }
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "对端列表",
                                                "手动指定对端，每行一个 host:port。同网段通常可依赖自动发现。",
                                            );
                                            let mut peers_text =
                                                settings_draft.peers.join("\n");
                                            if ui
                                                .add(
                                                    egui::TextEdit::multiline(&mut peers_text)
                                                        .desired_rows(4)
                                                        .desired_width(field_w.max(480.0))
                                                        .hint_text(theme::hint("host:port")),
                                                )
                                                .changed()
                                            {
                                                settings_draft.peers = peers_text
                                                    .lines()
                                                    .map(|l| l.trim().to_string())
                                                    .filter(|l| !l.is_empty())
                                                    .collect();
                                            }
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "集群盐",
                                                "同局域网互通须一致。修改后需重启，且各节点使用相同值。",
                                            );
                                            settings_text_field(
                                                ui,
                                                &mut settings_draft.cluster_salt_hex,
                                                "十六进制盐值",
                                                field_w.max(480.0),
                                            );

                                            settings_item_gap(ui);
                                            // —— 备份 ——
                                            settings_section(ui, "备份");
                                            settings_item_header(
                                                ui,
                                                "接受他节点托管",
                                                "允许本机接收其他节点推送的私人密文备份。也可在节点面板按节点关闭。",
                                            );
                                            ui.checkbox(
                                                &mut settings_draft.accept_foreign_backup,
                                                "接受他节点私人密文托管",
                                            );
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "推送到网络",
                                                "将本身份的私人备份推送到网络中可托管的节点。",
                                            );
                                            ui.checkbox(
                                                &mut settings_draft.backup_enabled,
                                                "推送本身份私人备份到网络",
                                            );
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "列表状态标签",
                                                "在左侧备忘列表中显示备份状态标记。",
                                            );
                                            ui.checkbox(
                                                &mut settings_draft.show_backup_status,
                                                "显示备份状态标签",
                                            );
                                            settings_item_gap(ui);
                                            settings_item_header(
                                                ui,
                                                "加密备份文件",
                                                "导出或导入本地 .memobak 加密备份。设置保存在系统用户配置目录。",
                                            );
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = 8.0;
                                                if ui.button("导出加密备份…").clicked() {
                                                    let dir = std::path::PathBuf::from(
                                                        &rt.cfg.data_dir,
                                                    )
                                                    .join("exports");
                                                    let path = dir.join(format!(
                                                        "backup-{}.memobak",
                                                        chrono_like_stamp()
                                                    ));
                                                    *backup_path = path.display().to_string();
                                                    backup_pw.clear();
                                                    *backup_import = false;
                                                    *show_backup = true;
                                                    *show_settings = false;
                                                }
                                                if ui.button("导入加密备份…").clicked() {
                                                    backup_path.clear();
                                                    backup_pw.clear();
                                                    *backup_import = true;
                                                    *show_backup = true;
                                                    *show_settings = false;
                                                }
                                            });

                                            ui.add_space(12.0);
                                        });
                                });

                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(6.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    if theme::success_button(ui, "保存").clicked() {
                                        let before = self.cfg.clone();
                                        match config::save_settings(settings_draft) {
                                            Ok(()) => {
                                                self.cfg.show_backup_status =
                                                    settings_draft.show_backup_status;
                                                self.cfg.backup_enabled =
                                                    settings_draft.backup_enabled;
                                                self.cfg.accept_foreign_backup =
                                                    settings_draft.accept_foreign_backup;
                                                self.cfg.theme = settings_draft.theme;
                                                self.cfg.auto_lock_minutes =
                                                    settings_draft.auto_lock_minutes;
                                                self.last_theme_mode = None;
                                                *show_settings = false;
                                                if config::needs_restart(&before, settings_draft)
                                                {
                                                    self.show_restart_confirm = true;
                                                } else {
                                                    let _ =
                                                        self.tx.send(BgMsg::Info("已保存".into()));
                                                }
                                            }
                                            Err(e) => {
                                                let _ =
                                                    self.tx.send(BgMsg::Error(e.to_string()));
                                            }
                                        }
                                    }
                                    if theme::ghost_button(ui, "取消").clicked() {
                                        *show_settings = false;
                                    }
                                },
                            );
                        });
                    if modal.end(ctx, open) {
                        *show_settings = false;
                    }
                }

                if self.show_restart_confirm {
                    let modal = theme::begin_modal(ctx, "restart_confirm_modal");
                    let mut open = true;
                    theme::modal_confirm(ctx, "需要重启")
                        .id(modal.window_id)
                        .open(&mut open)
                        .show(ctx, |ui| {
                            ui.label(
                                RichText::new("设置已保存")
                                    .size(15.0)
                                    .strong()
                                    .color(theme::text()),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                theme::muted_label(
                                    "集群盐、节点、端口或数据目录等更改需重启后生效。确定后将立即重启本程序。",
                                ),
                            );
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                if theme::primary_button(ui, "立即重启").clicked() {
                                    match Self::spawn_self_restart() {
                                        Ok(()) => {
                                            self.show_restart_confirm = false;
                                            self.quit_requested = true;
                                        }
                                        Err(e) => {
                                            self.show_restart_confirm = false;
                                            let _ = self.tx.send(BgMsg::Error(format!(
                                                "重启失败：{e}。请手动退出后重新打开。"
                                            )));
                                        }
                                    }
                                }
                                if theme::ghost_button(ui, "稍后手动重启").clicked() {
                                    self.show_restart_confirm = false;
                                    let _ = self.tx.send(BgMsg::Info(
                                        "已保存，请稍后手动重启使关键设置生效".into(),
                                    ));
                                }
                            });
                        });
                    if modal.end(ctx, open) {
                        self.show_restart_confirm = false;
                        let _ = self.tx.send(BgMsg::Info(
                            "已保存，请稍后手动重启使关键设置生效".into(),
                        ));
                    }
                }

            }
        }

        // 便签：须在 match 外处理，避免与 screen 字段借用冲突
        if matches!(self.screen, Screen::Main { .. }) {
            let rt = if let Screen::Main { rt, .. } = &self.screen {
                rt.clone()
            } else {
                unreachable!()
            };
            let mut sticky_status = String::new();
            self.flush_dirty_stickies(&rt, &mut sticky_status);
            self.refresh_stickies_from_list(&rt);
            if !sticky_status.is_empty() {
                if let Screen::Main { status_line, .. } = &mut self.screen {
                    *status_line = sticky_status;
                }
            }
            let sticky_ids: Vec<String> = self.stickies.keys().cloned().collect();
            let mut sticky_closed = Vec::new();
            for sid in sticky_ids {
                let Some(note) = self.stickies.get(&sid) else {
                    continue;
                };
                if note.lock().closed {
                    // 再注册一帧：回调里 Visible(false)，避免黑框残留
                    sticky_note::show_viewport(ctx, note);
                    sticky_note::force_hide_viewport(ctx, &sid);
                    sticky_closed.push(sid);
                    continue;
                }
                sticky_note::show_viewport(ctx, note);
            }
            let mut need_session_save = !sticky_closed.is_empty();
            for sid in sticky_closed {
                if let Some(h) = self.stickies.get(&sid) {
                    let mut n = h.lock();
                    n.flush_now = true;
                    let title = n.title.clone();
                    let body = n.body.clone();
                    let dirty = n.dirty;
                    drop(n);
                    if dirty {
                        let _ = rt.svc.edit(&sid, &title, &body);
                    }
                }
                sticky_note::force_hide_viewport(ctx, &sid);
                self.stickies.remove(&sid);
            }
            // 位置变动较频繁：约 1 秒落盘一次本机会话
            if self.sticky_session_dirty
                || self.last_sticky_session_save.elapsed() >= Duration::from_secs(1)
            {
                need_session_save = true;
            }
            if need_session_save {
                self.sticky_session_dirty = false;
                self.save_sticky_session_now();
            }
            self.shell.cal.save_prefs_if_dirty(
                Path::new(&self.cfg.data_dir),
                rt.svc.session_fp(),
            );
            self.shell.drawer.save_if_dirty(
                Path::new(&self.cfg.data_dir),
                rt.svc.session_fp(),
            );
            if self.main_hidden
                || !ctx.input(|i| i.viewport().focused.unwrap_or(false))
            {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }

        // 主窗藏托盘 / 身份门：保持低频刷新，便签与托盘退出才能响应
        if self.main_hidden || matches!(self.screen, Screen::IdentityGate(_)) {
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        self.show_help = show_help;
        self.show_about = show_about;
        self.draw_help_about_windows(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let fp = self
            .session_rt
            .as_ref()
            .map(|rt| rt.svc.session_fp().to_string())
            .or_else(|| {
                if let Screen::Main { rt, .. } = &self.screen {
                    Some(rt.svc.session_fp().to_string())
                } else {
                    None
                }
            });
        if let Some(fp) = fp.as_deref() {
            self.shell
                .cal
                .save_prefs_if_dirty(Path::new(&self.cfg.data_dir), fp);
            self.shell
                .drawer
                .save_if_dirty(Path::new(&self.cfg.data_dir), fp);
        }
        self.stop_session_rt();
        if let Some(t) = self.tray.take() {
            t.dispose();
        }
    }
}

fn unlock_runtime_identity(
    cfg: Config,
    identity: IdentityKeys,
    local_password: &str,
) -> anyhow::Result<Arc<AppRuntime>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("memo-sync")
        .worker_threads(2)
        .build()
        .map_err(|e| anyhow::anyhow!("无法启动后台运行时: {e}"))?;

        let (svc, engine) = {
        let _guard = rt.enter();
        let (svc, _audit) =
            memo_core::service::unlock_with_identity(cfg.clone(), &identity, local_password)?;
        let _ = svc.restore_from_hosted();
        let _ = svc.purge_expired_trash();
        svc.ensure_session_person()?;
        let engine = SyncEngine::new_with_identity(
            cfg.node_id.clone(),
            cfg.listen_port,
            cfg.peers.clone(),
            svc.store(),
            svc.person_store(),
            cfg.cluster_salt_hex.clone(),
            cfg.lan_discovery,
            cfg.node_role,
            cfg.node_visible,
            cfg.accept_foreign_backup,
            identity.fingerprint.clone(),
            identity.alias.clone(),
            PathBuf::from(&cfg.data_dir),
        );
        engine.set_service(&svc);
        engine.set_backup_targets(cfg.backup_targets.clone());
        svc.set_broadcaster(Arc::new(EngineBroadcaster::new(engine.clone())));
        engine.start();
        (svc, engine)
    };

    Ok(Arc::new(AppRuntime {
        svc,
        engine,
        cfg,
        _rt: rt,
    }))
}

pub fn run_gui(cfg: Config) -> eframe::Result<()> {
    run_gui_with_opts(cfg, false)
}

/// `start_hidden`：开机 `--tray` 时主窗先隐藏，仅托盘常驻。
pub fn run_gui_with_opts(cfg: Config, start_hidden: bool) -> eframe::Result<()> {
    if single_instance::try_acquire(std::path::Path::new(&cfg.data_dir))
        == single_instance::AcquireResult::AlreadyRunning
    {
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("分布式备忘录")
            .with_icon(app_icon::window_icon()),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "分布式备忘录",
        options,
        Box::new(move |cc| {
            fonts::configure_cjk_fonts(&cc.egui_ctx);
            Box::new(MemoApp::new(cfg, start_hidden))
        }),
    )
}
