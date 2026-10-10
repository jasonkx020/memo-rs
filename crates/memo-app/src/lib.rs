mod calendar_view;
mod app_icon;
mod autostart;
mod date_field;
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
mod period_view;
mod runtime;
mod save_dialog;
mod shell;
mod single_instance;
mod sticky_note;
mod theme;

use eframe::egui::{self, Color32, Frame, Margin, RichText};
use identity_gate::{GateAction, IdentityGateState};
use memo_core::config::{self, Config, NodeRole};
use memo_core::identity_keys::IdentityKeys;
use memo_core::person::Gender;
use memo_core::service::HistoryEvent;
use memo_core::store::{MemoCategory, MemoPriority, MemoVisibility};
use memo_form::{FormMode, MemoFormState};
use memo_sync::{EngineBroadcaster, SyncEngine};
use msg::BgMsg;
use runtime::AppRuntime;
use shell::{ShellAction, ShellUi};
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
    shell: ShellUi,
    memo_doc: doc_editor::Doc,
    edit_form: MemoFormState,
    last_input_at: Instant,
    pending_sync_hint: usize,
    last_remind_scan: Instant,
    /// 桌面便签（memo_id → 共享句柄；deferred viewport 需 Send+Sync）
    stickies: HashMap<String, StickyHandle>,
    /// 系统托盘（持有句柄防 drop）
    _tray: Option<tray::TrayHandle>,
    /// 真正退出（托盘菜单）；否则关主窗只隐藏
    quit_requested: bool,
    /// 主窗已藏到托盘（用于忽略残留的 close_requested，避免显示后立刻再藏）
    main_hidden: bool,
    /// 开机 `--tray`：首帧隐藏主窗
    start_hidden: bool,
    last_sticky_session_save: Instant,
}

const HELP_DOC: &str = include_str!("HELP.md");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

impl MemoApp {
    pub fn new(cfg: Config, start_hidden: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let gate = IdentityGateState::from_cfg(&cfg);
        let _tray = tray::create();
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
            shell: ShellUi::default(),
            memo_doc: doc_editor::Doc::empty(),
            edit_form: MemoFormState::default(),
            last_input_at: Instant::now(),
            pending_sync_hint: 0,
            last_remind_scan: Instant::now() - Duration::from_secs(30),
            stickies: HashMap::new(),
            _tray,
            quit_requested: false,
            main_hidden: start_hidden,
            start_hidden,
            last_sticky_session_save: Instant::now() - Duration::from_secs(60),
        }
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

    fn restore_stickies_from_session(&mut self, rt: &AppRuntime) {
        let Some(session) = sticky_note::load_session(Path::new(&self.cfg.data_dir)) else {
            return;
        };
        if session.person_fp != rt.svc.session_fp() {
            return;
        }
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
                    Some([g.w, g.h]),
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

    fn draw_help_about_windows(ctx: &egui::Context, show_help: &mut bool, show_about: &mut bool) {
        if *show_help {
            let modal = theme::begin_modal(ctx, "help_modal");
            theme::modal_fixed(ctx, "使用说明", [660.0, 540.0])
                .id(modal.window_id)
                .open(show_help)
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
            if modal.end(ctx, *show_help) {
                *show_help = false;
            }
        }
        if *show_about {
            let modal = theme::begin_modal(ctx, "about_modal");
            theme::modal_fixed(ctx, "关于", [480.0, 420.0])
                .id(modal.window_id)
                .open(show_about)
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
            if modal.end(ctx, *show_about) {
                *show_about = false;
            }
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BgMsg::UnlockResult(Ok(rt)) => {
                    let cfg = rt.cfg.clone();
                    self.cfg = cfg.clone();
                    self.shell = ShellUi::default();
                    self.last_input_at = Instant::now();
                    self.pending_sync_hint = 0;
                    self.restore_stickies_from_session(&rt);
                    self.screen = Self::make_main(rt, &cfg);
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
                        *selected = Some(id);
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
                        );
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
            // 托盘回调里已 ShowWindow；此处再同步一次并聚焦
            tray::show_main_window();
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.last_input_at = Instant::now();
            ctx.request_repaint();
        }
        if tray_quit {
            self.quit_requested = true;
        }

        // 关主窗 → Win32 隐藏到托盘（真正退出仅托盘「退出」）
        // 切勿 Visible(false)：Windows 上会导致无法再显示（egui#5229）
        if !self.quit_requested
            && !self.main_hidden
            && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide_main_window();
            self.main_hidden = true;
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if self.quit_requested {
            let rt_for_flush = if let Screen::Main { rt, .. } = &self.screen {
                Some(rt.clone())
            } else {
                None
            };
            if let Some(rt) = rt_for_flush {
                let mut dummy = String::new();
                self.flush_dirty_stickies(&rt, &mut dummy);
            }
            self.save_sticky_session_now();
            self.close_all_sticky_viewports(ctx);
            self.stickies.clear();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if self.pending_switch_person {
            let rt_for_flush = if let Screen::Main { rt, .. } = &self.screen {
                Some(rt.clone())
            } else {
                None
            };
            if let Some(rt) = rt_for_flush {
                let mut dummy = String::new();
                self.flush_dirty_stickies(&rt, &mut dummy);
            }
            self.save_sticky_session_now();
            self.close_all_sticky_viewports(ctx);
            self.stickies.clear();
            let mut gate = IdentityGateState::from_cfg(&self.cfg);
            gate.skip_auto_unlock = true;
            self.screen = Screen::IdentityGate(gate);
            self.pending_switch_person = false;
        }

        if let Screen::Main { .. } = &self.screen {
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
                    self.pending_switch_person = true;
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
                    } => {
                        st.busy = true;
                        st.status = if export_path.is_some() {
                            "正在导出并打开…".into()
                        } else {
                            "正在打开…".into()
                        };
                        let tx = self.tx.clone();
                        ctx.request_repaint();
                        std::thread::spawn(move || {
                            let res = (|| {
                                if let Some(path) = export_path.as_ref() {
                                    let p = PathBuf::from(path);
                                    identity.export_file(&p, &export_pass)?;
                                }
                                config::ensure_data_dir(&cfg)?;
                                let mut id = identity;
                                id.mark_exported(PathBuf::from(&cfg.data_dir).as_path())?;
                                unlock_runtime_identity(cfg, id)
                            })()
                            .map_err(|e| e.to_string());
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
                    let now = chrono::Local::now().naive_local();
                    for m in rt.svc.list() {
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
                            if !status_line.is_empty() {
                                ui.separator();
                                ui.label(
                                    RichText::new(status_line.as_str())
                                        .small()
                                        .color(theme::text_muted()),
                                );
                            }
                            let _ = audit_detail;
                        });
                    });

                let auto_lock_remaining = auto_lock_after(&self.cfg)
                    .map(|after| after.saturating_sub(self.last_input_at.elapsed()));
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
                    show_settings,
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
                    peers.len(),
                    auto_lock_remaining,
                    self.pending_sync_hint,
                    &rt.cfg.node_id,
                    rt.cfg.node_role,
                    rt.cfg.node_visible,
                    rt.cfg.lan_discovery,
                    rt.svc.disk_space(),
                    &mut settings_draft.backup_targets,
                    &self.tx,
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
                    );
                    *status_line = "填写新建备忘，保存后写入本机".into();
                }
                if let Some(id) = shell_action.open_sticky {
                    let list = rt.svc.list();
                    if let Some(existing) = self.stickies.get(&id) {
                        existing.lock().request_focus = true;
                        *status_line = "已聚焦桌面便签".into();
                    } else if let Some(m) = list.iter().find(|m| m.id == id) {
                        self.stickies
                            .insert(id, sticky_note::StickyNote::from_memo(m));
                        *status_line = "已生成桌面便签".into();
                    }
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


                if *show_new_memo {
                    let modal = theme::begin_modal(ctx, "new_memo_modal");
                    let mut open = true;
                    let mut close_requested = false;
                    theme::modal_fixed(ctx, "新建备忘", [560.0, 640.0])
                        .id(modal.window_id)
                        .open(&mut open)
                        .show(ctx, |ui| {
                            let mut scroll = egui::ScrollArea::vertical()
                                .id_source("new_memo_modal_scroll")
                                .auto_shrink([false, false])
                                .max_height(520.0);
                            // 标题未填保存时强制滚回顶部，露出标题输入栏
                            if self.edit_form.force_scroll_top() {
                                scroll = scroll.vertical_scroll_offset(0.0);
                                self.edit_form.tick_force_scroll_top();
                            }
                            scroll.show(ui, |ui| {
                                memo_form::show_fields(
                                    ui,
                                    &mut self.edit_form,
                                    FormMode::Create,
                                    "new_memo_modal",
                                );
                            });
                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if theme::success_button(ui, "保存备忘").clicked() {
                                    match memo_form::validate_title(&self.edit_form) {
                                        Ok(_) => {
                                            shell::submit_new_memo(
                                                &rt.svc,
                                                &self.edit_form,
                                                &self.tx,
                                            );
                                            *status_line = "正在创建…".into();
                                        }
                                        Err(e) => {
                                            *status_line = e;
                                            self.edit_form.request_title_focus();
                                        }
                                    }
                                }
                                if theme::ghost_button(ui, "取消").clicked() {
                                    close_requested = true;
                                }
                            });
                        });
                    if modal.end(ctx, open) || close_requested || !open {
                        *show_new_memo = false;
                        self.edit_form.clear();
                        if status_line.starts_with("填写新建") {
                            *status_line = "已取消新建".into();
                        }
                    }
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
                                RichText::new("导出为明文 TXT，请输入主密码确认，并选择保存位置。")
                                    .strong(),
                            );
                            ui.add_space(8.0);
                            ui.label("主密码");
                            let pw_w = ui.available_width().max(120.0);
                            theme::password_field(ui, export_pw, "输入主密码", pw_w, 34.0);
                            ui.add_space(8.0);
                            ui.label("保存路径");
                            ui.horizontal(|ui| {
                                let browse_w = 64.0;
                                let gap = ui.spacing().item_spacing.x;
                                let path_w =
                                    (ui.available_width() - browse_w - gap).max(80.0);
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
                                    let path =
                                        std::path::PathBuf::from(export_path.trim());
                                    let ids = export_ids.clone();
                                    let tx = self.tx.clone();
                                    *show_export = false;
                                    export_pw.clear();
                                    std::thread::spawn(move || {
                                        if let Err(e) = svc.verify_password(&pw) {
                                            let _ = tx.send(BgMsg::Error(e.to_string()));
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
                                    "选择 .memobak 文件并输入备份时使用的密码。"
                                } else {
                                    "将全部备忘加密导出为 .memobak（可用独立备份密码）。"
                                })
                                .weak(),
                            );
                            ui.add_space(8.0);
                            ui.label("备份密码");
                            let pw_w = ui.available_width().max(120.0);
                            theme::password_field(ui, backup_pw, "备份密码", pw_w, 34.0);
                            ui.add_space(6.0);
                            ui.label("文件路径");
                            ui.horizontal(|ui| {
                                let browse_w = 64.0;
                                let gap = ui.spacing().item_spacing.x;
                                let path_w =
                                    (ui.available_width() - browse_w - gap).max(80.0);
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
                                let can =
                                    !backup_pw.is_empty() && !backup_path.trim().is_empty();
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
                                    let pw = backup_pw.clone();
                                    let path =
                                        std::path::PathBuf::from(backup_path.trim());
                                    let import = *backup_import;
                                    let tx = self.tx.clone();
                                    *show_backup = false;
                                    backup_pw.clear();
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
                                                    .changed()
                                                {
                                                    self.cfg.start_on_boot =
                                                        settings_draft.start_on_boot;
                                                    let mut persist = self.cfg.clone();
                                                    persist.start_on_boot =
                                                        settings_draft.start_on_boot;
                                                    let _ = config::save_settings(&persist);
                                                    match autostart::apply(
                                                        settings_draft.start_on_boot,
                                                    ) {
                                                        Ok(()) => {
                                                            *status_line = if settings_draft
                                                                .start_on_boot
                                                            {
                                                                "已开启开机自启动".into()
                                                            } else {
                                                                "已关闭开机自启动".into()
                                                            };
                                                        }
                                                        Err(e) => {
                                                            *status_line =
                                                                format!("开机自启动设置失败: {e}");
                                                            settings_draft.start_on_boot =
                                                                !settings_draft.start_on_boot;
                                                            self.cfg.start_on_boot =
                                                                settings_draft.start_on_boot;
                                                        }
                                                    }
                                                    ctx.request_repaint();
                                                }
                                            });

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
                                                "数据目录",
                                                "备忘、密钥与本地库的存储路径。更改后需重启。",
                                            );
                                            ui.horizontal(|ui| {
                                                let browse_w = 64.0;
                                                let gap = ui.spacing().item_spacing.x;
                                                let path_w = (ui.available_width()
                                                    - browse_w
                                                    - gap)
                                                    .clamp(80.0, field_w.max(480.0));
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
                                                let msg = if config::needs_restart(
                                                    &before,
                                                    settings_draft,
                                                ) {
                                                    "已保存，请重启程序使关键设置生效"
                                                } else {
                                                    "已保存"
                                                };
                                                let _ = self.tx.send(BgMsg::Info(msg.into()));
                                                *show_settings = false;
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
            if self.last_sticky_session_save.elapsed() >= Duration::from_secs(5) {
                need_session_save = true;
            }
            if need_session_save {
                self.save_sticky_session_now();
            }
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
        Self::draw_help_about_windows(ctx, &mut self.show_help, &mut self.show_about);
    }
}

fn unlock_runtime_identity(
    cfg: Config,
    identity: IdentityKeys,
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
            memo_core::service::unlock_with_identity(cfg.clone(), &identity)?;
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
            .with_inner_size([1280.0, 840.0])
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
