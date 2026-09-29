mod calendar_view;
mod china_calendar;
mod fonts;
mod help_view;
mod history_view;
mod people_view;
mod save_dialog;
mod theme;

use calendar_view::CalUi;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Stroke, Vec2};
use memo_core::config::{self, Config};
use memo_core::license::{self, LicenseStatus};
use memo_core::service::{HistoryEvent, MemoService};
use memo_core::task::TaskStatus;
use memo_core::format_bytes;
use memo_sync::{DiscoveredPeer, EngineBroadcaster, PeerStatus, SyncEngine};
use people_view::PeopleUi;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeftTab {
    Memos,
    Calendar,
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

enum BgMsg {
    UnlockResult(Result<Arc<AppRuntime>, String>),
    Error(String),
    Info(String),
    Refresh,
    /// 删除成功后清除选中
    Deleted,
    /// 主密码校验通过后进入编辑模式
    EnterEdit,
}

struct AppRuntime {
    svc: Arc<MemoService>,
    engine: Arc<SyncEngine>,
    cfg: Config,
}

enum Screen {
    Unlock {
        password: String,
        confirm: String,
        first_setup: bool,
        status: String,
        busy: bool,
    },
    PersonGate {
        rt: Arc<AppRuntime>,
        selected: Option<String>,
        password: String,
        status: String,
        create_name: String,
        create_pw: String,
        create_pw2: String,
    },
    Main {
        rt: Arc<AppRuntime>,
        search: String,
        selected: Option<String>,
        /// 仅编辑模式下可改
        title_draft: String,
        body_draft: String,
        /// false=只读浏览；true=已通过密码的编辑态
        editing: bool,
        show_new: bool,
        new_title: String,
        new_body: String,
        show_settings: bool,
        settings_draft: Config,
        show_delete: bool,
        delete_pw: String,
        show_edit_auth: bool,
        edit_pw: String,
        /// 导出确认（主密码 + 路径）
        show_export: bool,
        export_pw: String,
        export_path: String,
        /// None = 全部；Some = 指定 id 列表
        export_ids: Option<Vec<String>>,
        /// 变更时间线
        show_history: bool,
        history_entity: String,
        history_events: Vec<HistoryEvent>,
        history_sel_a: Option<usize>,
        history_sel_b: Option<usize>,
        /// 加密备份 / 导入
        show_backup: bool,
        backup_import: bool,
        backup_pw: String,
        backup_path: String,
        status_line: String,
        peers: Vec<String>,
        discovered: Vec<DiscoveredPeer>,
        audit_ok: bool,
        audit_detail: String,
    },
}

pub struct MemoApp {
    cfg: Config,
    screen: Screen,
    tx: Sender<BgMsg>,
    rx: Receiver<BgMsg>,
    theme_applied: bool,
    last_peer_refresh: Instant,
    show_help: bool,
    show_about: bool,
    left_tab: LeftTab,
    cal: CalUi,
    people: PeopleUi,
    task_title: String,
    task_plan: String,
    task_date: String,
    task_start: f32,
    task_end_date: String,
    task_end_hour: f32,
    task_hours: f32,
    task_assignee: String,
    task_status: TaskStatus,
    task_editing: bool,
    show_new_task: bool,
    show_task_delete: bool,
    task_delete_pw: String,
    show_task_edit_auth: bool,
    task_edit_pw: String,
    pending_switch_person: bool,
    /// 右侧节点栏（默认收起，给详情更多宽度）
    show_nodes: bool,
}

const HELP_DOC: &str = include_str!("HELP.md");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

impl MemoApp {
    pub fn new(cfg: Config) -> Self {
        let (tx, rx) = mpsc::channel();
        let first_setup = is_first_setup(&cfg);
        Self {
            cfg,
            screen: Screen::Unlock {
                password: String::new(),
                confirm: String::new(),
                first_setup,
                status: String::new(),
                busy: false,
            },
            tx,
            rx,
            theme_applied: false,
            last_peer_refresh: Instant::now() - Duration::from_secs(10),
            show_help: false,
            show_about: false,
            left_tab: LeftTab::Memos,
            cal: CalUi::default(),
            people: PeopleUi::default(),
            task_title: String::new(),
            task_plan: String::new(),
            task_date: String::new(),
            task_start: 9.0,
            task_end_date: String::new(),
            task_end_hour: 10.0,
            task_hours: 1.0,
            task_assignee: String::new(),
            task_status: TaskStatus::NotStarted,
            task_editing: false,
            show_new_task: false,
            show_task_delete: false,
            task_delete_pw: String::new(),
            show_task_edit_auth: false,
            task_edit_pw: String::new(),
            pending_switch_person: false,
            show_nodes: false,
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
            editing: false,
            show_new: false,
            new_title: String::new(),
            new_body: String::new(),
            show_settings: false,
            settings_draft: cfg.clone(),
            show_delete: false,
            delete_pw: String::new(),
            show_edit_auth: false,
            edit_pw: String::new(),
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

    fn help_about_menu(ui: &mut egui::Ui, show_help: &mut bool, show_about: &mut bool) {
        ui.menu_button(RichText::new("帮助").color(Color32::WHITE), |ui| {
            if ui.button("使用说明").clicked() {
                *show_help = true;
                ui.close_menu();
            }
            if ui.button("关于").clicked() {
                *show_about = true;
                ui.close_menu();
            }
        });
    }

    fn draw_help_about_windows(ctx: &egui::Context, show_help: &mut bool, show_about: &mut bool) {
        if *show_help {
            egui::Window::new("使用说明")
                .collapsible(false)
                .resizable(true)
                .default_size([660.0, 540.0])
                .min_width(440.0)
                .min_height(340.0)
                .open(show_help)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(theme::muted_label("以下内容已嵌入本程序，可离线阅读。").small());
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(440.0)
                        .show(ui, |ui| {
                            help_view::show(ui, HELP_DOC);
                        });
                });
        }

        if *show_about {
            egui::Window::new("关于")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(show_about)
                .show(ctx, |ui| {
                    ui.set_min_width(400.0);
                    ui.add_space(6.0);
                    ui.label(theme::brand_title(22.0));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(theme::APP_DESCRIPTION)
                            .color(theme::TEXT_MUTED)
                            .size(13.0),
                    );
                    ui.add_space(14.0);
                    ui.separator();
                    ui.add_space(10.0);
                    about_kv(ui, "产品名称", theme::APP_NAME);
                    about_kv(ui, "版本", APP_VERSION);
                    about_kv(ui, "文件版本", theme::APP_FILE_VERSION);
                    about_kv(ui, "版权", theme::APP_COPYRIGHT);
                    let lic = if let Ok(p) = config::settings_path() {
                        if let Some(parent) = p.parent() {
                            license::load_status(parent)
                        } else {
                            LicenseStatus::Community
                        }
                    } else {
                        LicenseStatus::Community
                    };
                    about_kv(ui, "授权", &license::status_label(&lic));
                    #[cfg(windows)]
                    about_kv(ui, "平台", "Windows x64");
                    #[cfg(not(windows))]
                    about_kv(ui, "平台", std::env::consts::OS);
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.label(RichText::new("功能特性").strong().color(theme::TEXT));
                    ui.add_space(4.0);
                    ui.label(RichText::new("· 绿色单文件，无需安装运行库").color(theme::TEXT));
                    ui.label(RichText::new("· 本地 AES-256-GCM 加密存储").color(theme::TEXT));
                    ui.label(RichText::new("· Argon2id 主密码派生").color(theme::TEXT));
                    ui.label(RichText::new("· Ed25519 审计链 · 变更时间线").color(theme::TEXT));
                    ui.label(RichText::new("· LWW 同步 · 局域网 UDP 自动发现").color(theme::TEXT));
                    ui.label(RichText::new("· 加密备份 (.memobak)").color(theme::TEXT));
                    ui.label(RichText::new("· 人员管理与人员密码登录").color(theme::TEXT));
                    ui.label(RichText::new("· 日历：月历 / 周视图 / 甘特与工时").color(theme::TEXT));
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
                                .color(theme::TEXT_MUTED),
                        );
                    }
                    ui.add_space(4.0);
                });
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BgMsg::UnlockResult(Ok(rt)) => {
                    self.screen = Screen::PersonGate {
                        rt,
                        selected: None,
                        password: String::new(),
                        status: String::new(),
                        create_name: String::new(),
                        create_pw: String::new(),
                        create_pw2: String::new(),
                    };
                }
                BgMsg::UnlockResult(Err(e)) => {
                    if let Screen::Unlock { status, busy, .. } = &mut self.screen {
                        *status = format!("解锁失败: {e}");
                        *busy = false;
                    }
                }
                BgMsg::Error(e) => {
                    match &mut self.screen {
                        Screen::Main { status_line, .. } => {
                            *status_line = format!("错误: {e}");
                        }
                        Screen::PersonGate { status, .. } => {
                            *status = format!("错误: {e}");
                        }
                        _ => {}
                    }
                }
                BgMsg::Info(s) => {
                    if let Screen::Main { status_line, .. } = &mut self.screen {
                        *status_line = s;
                    }
                }
                BgMsg::EnterEdit => {
                    if let Screen::Main {
                        editing,
                        show_edit_auth,
                        edit_pw,
                        status_line,
                        ..
                    } = &mut self.screen
                    {
                        *editing = true;
                        *show_edit_auth = false;
                        edit_pw.clear();
                        *status_line = "已进入编辑模式，修改后请点「保存」".into();
                    }
                }
                BgMsg::Deleted => {
                    if let Screen::Main {
                        selected,
                        title_draft,
                        body_draft,
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
                        *editing = false;
                        *status_line = "已删除".into();
                        *peers = rt.engine.connected_peers();
                        *discovered = rt.engine.discovered_peers();
                        let (ok, detail) = rt.svc.verify_audit();
                        *audit_ok = ok;
                        *audit_detail = detail;
                    }
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
                        // 非编辑态时用服务端数据刷新只读展示
                        if !*editing {
                            if let Some(id) = selected.clone() {
                                if let Some(m) = rt.svc.list().into_iter().find(|m| m.id == id) {
                                    *title_draft = m.title;
                                    *body_draft = m.content;
                                } else {
                                    *selected = None;
                                    title_draft.clear();
                                    body_draft.clear();
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
                .color(theme::TEXT_MUTED)
                .size(13.0),
        );
        ui.label(RichText::new(value).color(theme::TEXT).size(13.0));
    });
}

impl eframe::App for MemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.theme_applied {
            theme::apply(ctx);
            self.theme_applied = true;
        }
        // 必须在绘制 TextEdit 之前修正 IME 事件
        sanitize_cjk_ime_events(ctx);
        self.pump(ctx);

        if self.pending_switch_person {
            if let Screen::Main { rt, .. } = &self.screen {
                let rt = rt.clone();
                rt.svc.set_current_person(None);
                self.screen = Screen::PersonGate {
                    rt,
                    selected: None,
                    password: String::new(),
                    status: String::new(),
                    create_name: String::new(),
                    create_pw: String::new(),
                    create_pw2: String::new(),
                };
            }
            self.pending_switch_person = false;
        }

        if let Screen::Main { .. } = &self.screen {
            ctx.request_repaint_after(Duration::from_secs(2));
        }

        let mut show_help = self.show_help;
        let mut show_about = self.show_about;
        let mut enter_main_rt: Option<Arc<AppRuntime>> = None;

        match &mut self.screen {
            Screen::Unlock {
                password,
                confirm,
                first_setup,
                status,
                busy,
            } => {
                egui::CentralPanel::default()
                    .frame(Frame::none().fill(theme::BG))
                    .show(ctx, |ui| {
                        let avail = ui.available_size();
                        let card_w = 400.0_f32;
                        let card_h = if *first_setup { 420.0_f32 } else { 360.0_f32 };
                        let left = ((avail.x - card_w) * 0.5).max(16.0);
                        let top = ((avail.y - card_h) * 0.42).max(40.0);
                        let card_rect = egui::Rect::from_min_size(
                            egui::pos2(ui.min_rect().left() + left, ui.min_rect().top() + top),
                            Vec2::new(card_w, card_h),
                        );
                        ui.allocate_ui_at_rect(card_rect, |ui| {
                            theme::card_frame().show(ui, |ui| {
                                ui.set_min_size(Vec2::new(card_w - 32.0, card_h - 32.0));
                                ui.vertical_centered(|ui| {
                                    ui.add_space(12.0);
                                    ui.label(theme::brand_title(26.0));
                                    ui.add_space(6.0);
                                    ui.label(
                                        theme::muted_label("本地加密 · 局域网同步 · 绿色单文件")
                                            .size(13.0),
                                    );
                                    ui.add_space(28.0);
                                    ui.label(
                                        RichText::new(if *first_setup {
                                            "设置主密码"
                                        } else {
                                            "主密码"
                                        })
                                        .strong()
                                        .color(theme::TEXT)
                                        .size(13.0),
                                    );
                                    ui.add_space(6.0);
                                    theme::password_field(
                                        ui,
                                        password,
                                        if *first_setup {
                                            "设置主密码"
                                        } else {
                                            "输入主密码解锁"
                                        },
                                        320.0,
                                        36.0,
                                    );
                                    if *first_setup {
                                        ui.add_space(12.0);
                                        ui.label(
                                            RichText::new("确认主密码")
                                                .strong()
                                                .color(theme::TEXT)
                                                .size(13.0),
                                        );
                                        ui.add_space(6.0);
                                        theme::password_field(
                                            ui,
                                            confirm,
                                            "再次输入以确认",
                                            320.0,
                                            36.0,
                                        );
                                    }
                                    ui.add_space(18.0);
                                    let btn_label = if *busy {
                                        if *first_setup {
                                            "初始化中…"
                                        } else {
                                            "解锁中…"
                                        }
                                    } else if *first_setup {
                                        "开始使用"
                                    } else {
                                        "解锁"
                                    };
                                    let unlock = ui.add_sized(
                                        [320.0, 36.0],
                                        egui::Button::new(
                                            RichText::new(btn_label)
                                                .color(Color32::WHITE)
                                                .strong()
                                                .size(15.0),
                                        )
                                        .fill(theme::ACCENT),
                                    );
                                    if (unlock.clicked()
                                        || ui.input(|i| i.key_pressed(egui::Key::Enter)))
                                        && !*busy
                                    {
                                        if password.is_empty() {
                                            *status = "主密码不能为空".into();
                                        } else if *first_setup
                                            && (confirm.is_empty() || confirm != password)
                                        {
                                            *status = "两次输入不一致，请重新输入".into();
                                        } else {
                                            *busy = true;
                                            *status = "正在派生密钥，请稍候…".into();
                                            let cfg = self.cfg.clone();
                                            let pw = password.clone();
                                            *password = String::new();
                                            *confirm = String::new();
                                            let tx = self.tx.clone();
                                            std::thread::spawn(move || {
                                                let res = unlock_runtime(cfg, pw.as_bytes())
                                                    .map_err(|e| e.to_string());
                                                let _ = tx.send(BgMsg::UnlockResult(res));
                                            });
                                        }
                                    }
                                    ui.add_space(14.0);
                                    if !status.is_empty() {
                                        let c = if *busy {
                                            theme::TEXT_MUTED
                                        } else {
                                            theme::DANGER
                                        };
                                        ui.label(RichText::new(status.as_str()).color(c).size(13.0));
                                    }
                                    ui.add_space(20.0);
                                    ui.horizontal(|ui| {
                                        if ui
                                            .link(RichText::new("使用说明").color(theme::ACCENT))
                                            .clicked()
                                        {
                                            show_help = true;
                                        }
                                        ui.label(theme::muted_label("·").small());
                                        if ui
                                            .link(RichText::new("关于").color(theme::ACCENT))
                                            .clicked()
                                        {
                                            show_about = true;
                                        }
                                    });
                                });
                            });
                        });
                    });
            }
            Screen::PersonGate {
                rt,
                selected,
                password,
                status,
                create_name,
                create_pw,
                create_pw2,
            } => {
                let rt = rt.clone();
                let persons = rt.svc.list_active_persons();
                let need_create = persons.is_empty();
                egui::CentralPanel::default()
                    .frame(Frame::none().fill(theme::BG))
                    .show(ctx, |ui| {
                        let avail = ui.available_size();
                        let card_w = 420.0_f32;
                        let card_h = if need_create { 380.0_f32 } else { 420.0_f32 };
                        let left = ((avail.x - card_w) * 0.5).max(16.0);
                        let top = ((avail.y - card_h) * 0.35).max(40.0);
                        let card_rect = egui::Rect::from_min_size(
                            egui::pos2(ui.min_rect().left() + left, ui.min_rect().top() + top),
                            Vec2::new(card_w, card_h),
                        );
                        ui.allocate_ui_at_rect(card_rect, |ui| {
                            theme::card_frame().show(ui, |ui| {
                                ui.set_min_width(card_w - 24.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new("选择人员")
                                            .size(22.0)
                                            .strong()
                                            .color(theme::TEXT),
                                    );
                                    ui.label(
                                        theme::muted_label(
                                            "主密码已解锁 · 请登录人员身份后继续",
                                        )
                                        .small(),
                                    );
                                });
                                ui.add_space(12.0);
                                if need_create {
                                    ui.label(
                                        RichText::new("首次使用：创建首位人员")
                                            .strong()
                                            .color(theme::TEXT),
                                    );
                                    ui.horizontal(|ui| {
                                        ui.label("姓名");
                                        ui.add(
                                            egui::TextEdit::singleline(create_name)
                                                .desired_width(220.0),
                                        );
                                    });
                                    theme::password_field(ui, create_pw, "人员密码", 280.0, 34.0);
                                    theme::password_field(ui, create_pw2, "确认密码", 280.0, 34.0);
                                    ui.add_space(8.0);
                                    if theme::primary_button(ui, "创建并进入").clicked() {
                                        if create_pw != create_pw2 {
                                            *status = "两次密码不一致".into();
                                        } else {
                                            match rt.svc.add_person(create_name, create_pw) {
                                                Ok(id) => {
                                                    if let Err(e) =
                                                        rt.svc.verify_person_password(&id, create_pw)
                                                    {
                                                        *status = e.to_string();
                                                    } else {
                                                        enter_main_rt = Some(rt.clone());
                                                    }
                                                }
                                                Err(e) => *status = e.to_string(),
                                            }
                                        }
                                    }
                                } else {
                                    egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                                        for p in &persons {
                                            let sel = selected.as_deref() == Some(p.id.as_str());
                                            if ui
                                                .selectable_label(
                                                    sel,
                                                    RichText::new(&p.name).size(15.0),
                                                )
                                                .clicked()
                                            {
                                                *selected = Some(p.id.clone());
                                                password.clear();
                                            }
                                        }
                                    });
                                    ui.add_space(8.0);
                                    theme::password_field(ui, password, "人员密码", 280.0, 34.0);
                                    ui.add_space(8.0);
                                    let can = selected.is_some() && !password.is_empty();
                                    if ui
                                        .add_enabled(
                                            can,
                                            egui::Button::new(
                                                RichText::new("进入").color(Color32::WHITE).strong(),
                                            )
                                            .fill(theme::ACCENT),
                                        )
                                        .clicked()
                                    {
                                        if let Some(id) = selected.clone() {
                                            match rt.svc.verify_person_password(&id, password) {
                                                Ok(()) => {
                                                    enter_main_rt = Some(rt.clone());
                                                }
                                                Err(e) => *status = e.to_string(),
                                            }
                                        }
                                    }
                                }
                                if !status.is_empty() {
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(status.as_str()).color(theme::DANGER).small(),
                                    );
                                }
                            });
                        });
                    });
            }
            Screen::Main {
                rt,
                search,
                selected,
                title_draft,
                body_draft,
                editing,
                show_new,
                new_title,
                new_body,
                show_settings,
                settings_draft,
                show_delete,
                delete_pw,
                show_edit_auth,
                edit_pw,
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
                // 发现列表低频刷新，避免每帧改状态干扰输入法
                if self.last_peer_refresh.elapsed() >= Duration::from_secs(2) {
                    *peers = rt.engine.connected_peers();
                    *discovered = rt.engine.discovered_peers();
                    let conflicts = rt.svc.take_conflicts();
                    if !conflicts.is_empty() {
                        let n = conflicts.len();
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

                let person_label = rt
                    .svc
                    .current_person_id()
                    .map(|id| rt.svc.person_name(&id))
                    .unwrap_or_else(|| "未选择".into());

                egui::TopBottomPanel::top("top")
                    .frame(theme::top_bar_frame())
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(theme::brand_title_on_navy(17.0));
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(format!("人员 {person_label}"))
                                    .color(Color32::from_rgb(0xE2, 0xE8, 0xF0))
                                    .size(13.0),
                            );
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("切换")
                                            .small()
                                            .color(Color32::from_rgb(0xE2, 0xE8, 0xF0)),
                                    )
                                    .fill(theme::NAVY_MID),
                                )
                                .clicked()
                            {
                                self.pending_switch_person = true;
                            }
                            ui.add_space(10.0);
                            if self.left_tab == LeftTab::Memos {
                                ui.add(
                                    egui::TextEdit::singleline(search)
                                        .hint_text("搜索备忘…")
                                        .desired_width(200.0),
                                );
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                Self::help_about_menu(ui, &mut show_help, &mut show_about);
                                let secondary = |ui: &mut egui::Ui, label: &str| {
                                    ui.add(
                                        egui::Button::new(
                                            RichText::new(label).color(Color32::from_rgb(0xE2, 0xE8, 0xF0)),
                                        )
                                        .fill(theme::NAVY_MID)
                                        .stroke(Stroke::new(1.0, Color32::from_rgb(0x33, 0x55, 0x7A))),
                                    )
                                };
                                if secondary(ui, "人员").clicked() {
                                    self.people.show = true;
                                }
                                let nodes_label = if self.show_nodes {
                                    "隐藏节点"
                                } else {
                                    "节点"
                                };
                                if secondary(ui, nodes_label).clicked() {
                                    self.show_nodes = !self.show_nodes;
                                }
                                if secondary(ui, "设置").clicked() {
                                    *settings_draft =
                                        config::load_settings().unwrap_or(self.cfg.clone());
                                    *show_settings = true;
                                }
                                if secondary(ui, "导出全部").clicked() {
                                    let dir = std::path::PathBuf::from(&rt.cfg.data_dir)
                                        .join("exports");
                                    let path = dir.join(format!(
                                        "memo-all-{}.txt",
                                        chrono_like_stamp()
                                    ));
                                    *export_ids = None;
                                    *export_path = path.display().to_string();
                                    export_pw.clear();
                                    *show_export = true;
                                }
                                let new_label = if self.left_tab == LeftTab::Calendar {
                                    "新建任务"
                                } else {
                                    "新建"
                                };
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(new_label)
                                                .color(Color32::WHITE)
                                                .strong(),
                                        )
                                        .fill(theme::ACCENT),
                                    )
                                    .clicked()
                                {
                                    if self.left_tab == LeftTab::Calendar {
                                        self.show_new_task = true;
                                        self.task_title.clear();
                                        self.task_plan.clear();
                                        self.task_date = self.cal.selected_day.clone();
                                        self.task_start = 9.0;
                                        self.task_hours = 1.0;
                                        calendar_view::sync_end_from_hours(
                                            &self.task_date,
                                            self.task_start,
                                            self.task_hours,
                                            &mut self.task_end_date,
                                            &mut self.task_end_hour,
                                        );
                                        self.task_assignee = rt
                                            .svc
                                            .current_person_id()
                                            .unwrap_or_default();
                                        self.task_status = TaskStatus::NotStarted;
                                    } else {
                                        *show_new = true;
                                        new_title.clear();
                                        new_body.clear();
                                    }
                                }
                            });
                        });
                    });

                egui::TopBottomPanel::bottom("bottom")
                    .frame(theme::bottom_bar_frame())
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("节点 {}", rt.cfg.node_id))
                                    .small()
                                    .color(theme::TEXT_MUTED),
                            );
                            ui.separator();
                            {
                                let online_n = discovered
                                    .iter()
                                    .filter(|d| d.status == PeerStatus::Online)
                                    .count();
                                let ch_n = discovered.iter().filter(|d| d.sync_ready).count();
                                ui.label(
                                    RichText::new(format!(
                                        "发现 {} · 在线 {} · 通道 {}",
                                        discovered.len(),
                                        online_n,
                                        ch_n
                                    ))
                                    .small()
                                    .color(theme::TEXT_MUTED),
                                );
                            }
                            ui.separator();
                            {
                                let disk = rt.engine.local_disk_space();
                                let disk_color = if disk.is_low() {
                                    theme::DANGER
                                } else if disk.free_bytes < 1024 * 1024 * 1024 {
                                    theme::WARN
                                } else {
                                    theme::TEXT_MUTED
                                };
                                ui.label(
                                    RichText::new(format!("磁盘 {}", disk.format_pair()))
                                        .small()
                                        .color(disk_color),
                                )
                                .on_hover_text("数据目录所在卷：可用 / 总计");
                            }
                            if let Some(reason) = rt.engine.take_sync_block_reason() {
                                *status_line = reason;
                            }
                            ui.separator();
                            if *audit_ok {
                                ui.label(
                                    RichText::new("审计完整").small().color(theme::SUCCESS),
                                );
                            } else {
                                ui.label(
                                    RichText::new(format!("审计: {audit_detail}"))
                                        .small()
                                        .color(theme::DANGER),
                                );
                            }
                            if let Some(err) = rt.engine.listen_error() {
                                ui.separator();
                                ui.label(RichText::new(err).small().color(theme::DANGER));
                            }
                            if !status_line.is_empty() {
                                ui.separator();
                                ui.label(
                                    RichText::new(status_line.as_str())
                                        .small()
                                        .color(theme::TEXT_MUTED),
                                );
                            }
                        });
                    });

                if self.show_nodes {
                    egui::SidePanel::right("nodes")
                        .resizable(true)
                        .default_width(200.0)
                        .min_width(140.0)
                        .max_width(280.0)
                        .frame(theme::panel_frame())
                        .show(ctx, |ui| {
                            ui.label(
                                RichText::new("节点").strong().size(15.0).color(theme::TEXT),
                            );
                            ui.label(
                                theme::muted_label(if rt.cfg.lan_discovery {
                                    "UDP 在线 · TCP 同步通道"
                                } else {
                                    "局域网发现已关闭"
                                })
                                .small(),
                            );
                            {
                                let disk = rt.engine.local_disk_space();
                                let c = if disk.is_low() {
                                    theme::DANGER
                                } else {
                                    theme::TEXT_MUTED
                                };
                                ui.label(
                                    RichText::new(format!("本机磁盘 {}", disk.format_pair()))
                                        .small()
                                        .color(c),
                                );
                            }
                            egui::CollapsingHeader::new(
                                RichText::new("如何腾出空间").small().color(theme::TEXT_MUTED),
                            )
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label(
                                    theme::muted_label(
                                        "1) 清理数据盘无关大文件\n\
                                         2) 设置中把「数据目录」改到更大磁盘并迁移后重启\n\
                                         3) 删除不需要的 exports/ 与旧 .memobak\n\
                                         4) 满盘时勿大量导入或全量同步",
                                    )
                                    .small(),
                                );
                            });
                            if rt.engine.peer_count_warning() {
                                ui.label(
                                    RichText::new("节点偏多，建议减少同 salt 设备")
                                        .small()
                                        .color(theme::WARN),
                                );
                            }
                            ui.add_space(8.0);
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                if discovered.is_empty() {
                                    ui.label(
                                        theme::muted_label(
                                            "暂无发现节点\n请确认同网段、同 salt，并放行 UDP 17000",
                                        )
                                        .small(),
                                    );
                                }
                                for d in discovered.iter() {
                                    let (st_color, dot) = match d.status {
                                        PeerStatus::Online => (theme::SUCCESS, theme::SUCCESS),
                                        PeerStatus::Stale => (theme::WARN, theme::WARN),
                                        PeerStatus::Offline => (theme::DANGER, theme::BORDER_STRONG),
                                        PeerStatus::Undiscovered => {
                                            (theme::TEXT_MUTED, theme::BORDER_STRONG)
                                        }
                                    };
                                    theme::card_frame()
                                        .inner_margin(Margin::same(10.0))
                                        .show(ui, |ui| {
                                            ui.set_min_width(ui.available_width());
                                            ui.horizontal(|ui| {
                                                let (rect, _) = ui.allocate_exact_size(
                                                    Vec2::splat(8.0),
                                                    egui::Sense::hover(),
                                                );
                                                ui.painter().circle_filled(rect.center(), 3.5, dot);
                                                ui.label(
                                                    RichText::new(&d.node_id)
                                                        .strong()
                                                        .small()
                                                        .color(theme::TEXT),
                                                );
                                            });
                                            ui.label(
                                                RichText::new(&d.addr)
                                                    .small()
                                                    .color(theme::TEXT_MUTED),
                                            );
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(d.status.label())
                                                        .small()
                                                        .color(st_color),
                                                );
                                                if d.sync_ready {
                                                    ui.label(
                                                        RichText::new("通道")
                                                            .small()
                                                            .color(theme::SUCCESS),
                                                    );
                                                }
                                                if d.status != PeerStatus::Undiscovered
                                                    && d.last_seen_secs < 3600
                                                {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "{}s 前",
                                                            d.last_seen_secs
                                                        ))
                                                        .small()
                                                        .color(theme::TEXT_MUTED),
                                                    );
                                                }
                                            });
                                            let disk_line = match (d.disk_free, d.disk_total) {
                                                (Some(f), Some(t)) => {
                                                    format!(
                                                        "磁盘 {} / {}",
                                                        format_bytes(f),
                                                        format_bytes(t)
                                                    )
                                                }
                                                _ => "空间未知".into(),
                                            };
                                            ui.label(
                                                RichText::new(disk_line)
                                                    .small()
                                                    .color(theme::TEXT_MUTED),
                                            );
                                        });
                                    ui.add_space(6.0);
                                }
                            });
                        });
                }

                let mut filtered: Vec<_> = rt
                    .svc
                    .list()
                    .into_iter()
                    .filter(|m| {
                        let q = search.to_lowercase();
                        q.is_empty()
                            || m.title.to_lowercase().contains(&q)
                            || m.content.to_lowercase().contains(&q)
                    })
                    .collect();
                filtered.sort_by(|a, b| b.version.cmp(&a.version));

                let tasks_all = rt.svc.list_tasks();
                let persons_all = rt.svc.list_active_persons();

                let left_w = if self.left_tab == LeftTab::Calendar {
                    300.0
                } else {
                    280.0
                };
                egui::SidePanel::left("list")
                    .resizable(true)
                    .default_width(left_w)
                    .min_width(220.0)
                    .max_width(420.0)
                    .frame(theme::panel_frame())
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(
                                    self.left_tab == LeftTab::Memos,
                                    RichText::new("备忘").strong().size(14.0),
                                )
                                .clicked()
                            {
                                self.left_tab = LeftTab::Memos;
                                self.cal.selected_task = None;
                            }
                            if ui
                                .selectable_label(
                                    self.left_tab == LeftTab::Calendar,
                                    RichText::new("日历").strong().size(14.0),
                                )
                                .clicked()
                            {
                                self.left_tab = LeftTab::Calendar;
                                *selected = None;
                                *editing = false;
                            }
                        });
                        ui.add_space(6.0);

                        match self.left_tab {
                            LeftTab::Memos => {
                                ui.label(
                                    theme::muted_label("单击打开 · 右键删除").small(),
                                );
                                ui.add_space(6.0);
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    if filtered.is_empty() {
                                        ui.label(theme::muted_label(
                                            "还没有备忘，点击右上角「新建」",
                                        ));
                                    }
                                    for m in &filtered {
                                        let title_raw = if m.title.is_empty() {
                                            "(无标题)".to_string()
                                        } else {
                                            m.title.clone()
                                        };
                                        let summary_raw: String =
                                            m.content.chars().take(48).collect();
                                        let sel = selected.as_deref() == Some(m.id.as_str());
                                        let item_id = m.id.clone();
                                        let item_title = m.title.clone();
                                        let item_body = m.content.clone();

                                        let row_w = ui.available_width().max(40.0);
                                        let row_h = 52.0_f32;
                                        let (rect, resp) = ui.allocate_exact_size(
                                            Vec2::new(row_w, row_h),
                                            egui::Sense::click(),
                                        );
                                        let resp =
                                            resp.on_hover_cursor(egui::CursorIcon::PointingHand);

                                        if ui.is_rect_visible(rect) {
                                            let fill = theme::list_row_fill(sel, resp.hovered());
                                            ui.painter().rect(
                                                rect,
                                                Rounding::same(theme::ROUND_CTRL),
                                                fill,
                                                Stroke::new(1.0, theme::BORDER),
                                            );

                                            let pad_x = 10.0;
                                            let pad_y = 8.0;
                                            let text_w = (rect.width() - pad_x * 2.0).max(12.0);
                                            let title_font = egui::FontId::proportional(14.5);
                                            let summary_font = egui::FontId::proportional(12.0);
                                            let title = ellipsize_ui(
                                                ui,
                                                &title_raw,
                                                title_font.clone(),
                                                text_w,
                                            );
                                            let summary = ellipsize_ui(
                                                ui,
                                                &summary_raw,
                                                summary_font.clone(),
                                                text_w,
                                            );

                                            let painter = ui.painter().with_clip_rect(rect);
                                            painter.text(
                                                egui::pos2(
                                                    rect.left() + pad_x,
                                                    rect.top() + pad_y,
                                                ),
                                                egui::Align2::LEFT_TOP,
                                                &title,
                                                title_font,
                                                theme::TEXT,
                                            );
                                            painter.text(
                                                egui::pos2(
                                                    rect.left() + pad_x,
                                                    rect.top() + pad_y + 20.0,
                                                ),
                                                egui::Align2::LEFT_TOP,
                                                &summary,
                                                summary_font,
                                                theme::TEXT_MUTED,
                                            );
                                        }

                                        if resp.clicked() {
                                            *selected = Some(item_id.clone());
                                            *title_draft = item_title.clone();
                                            *body_draft = item_body.clone();
                                            *editing = false;
                                            self.cal.selected_task = None;
                                        }
                                        resp.context_menu(|ui| {
                                            if ui.button("打开").clicked() {
                                                *selected = Some(item_id.clone());
                                                *title_draft = item_title.clone();
                                                *body_draft = item_body.clone();
                                                *editing = false;
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            if ui
                                                .add(egui::Button::new(
                                                    RichText::new("删除…").color(theme::DANGER),
                                                ))
                                                .clicked()
                                            {
                                                *selected = Some(item_id.clone());
                                                *title_draft = item_title.clone();
                                                *body_draft = item_body.clone();
                                                *editing = false;
                                                *show_delete = true;
                                                delete_pw.clear();
                                                ui.close_menu();
                                            }
                                        });
                                        ui.add_space(6.0);
                                    }
                                });
                            }
                            LeftTab::Calendar => {
                                if let Some(tid) =
                                    calendar_view::show_left(ui, &mut self.cal, &tasks_all, &persons_all)
                                {
                                    if let Some(t) = tasks_all.iter().find(|t| t.id == tid) {
                                        self.task_title = t.title.clone();
                                        self.task_plan = t.plan.clone();
                                        self.task_date = t.date.clone();
                                        self.task_start = t.start_hour;
                                        self.task_end_date = t.end_date.clone();
                                        self.task_end_hour = t.end_hour;
                                        self.task_hours = t.hours;
                                        self.task_assignee = t.assignee_id.clone();
                                        self.task_status = t.status;
                                        self.task_editing = false;
                                        *selected = None;
                                    }
                                }
                            }
                        }
                    });

                let _ = people_view::show_window(ctx, &mut self.people, &rt.svc);
                if let Some(pid) = self.people.take_history_id() {
                    match rt.svc.history_for("person", &pid) {
                        Ok(ev) => {
                            *history_entity = "person".into();
                            *history_events = ev;
                            *history_sel_a = None;
                            *history_sel_b = None;
                            *show_history = true;
                        }
                        Err(e) => {
                            *status_line = format!("读取历史失败: {e}");
                        }
                    }
                }

                egui::CentralPanel::default()
                    .frame(Frame::none().inner_margin(Margin::same(16.0)).fill(theme::BG))
                    .show(ctx, |ui| {
                        if self.left_tab == LeftTab::Calendar {
                            if self.cal.selected_task.is_some() {
                                ui.horizontal(|ui| {
                                    if theme::ghost_button(ui, "← 返回总览").clicked() {
                                        self.cal.selected_task = None;
                                        self.task_editing = false;
                                    }
                                    ui.label(
                                        theme::muted_label("任务详情").small(),
                                    );
                                });
                                ui.add_space(6.0);
                                draw_task_central(
                                    ui,
                                    &rt,
                                    &persons_all,
                                    &mut self.cal,
                                    &mut self.task_title,
                                    &mut self.task_plan,
                                    &mut self.task_date,
                                    &mut self.task_start,
                                    &mut self.task_end_date,
                                    &mut self.task_end_hour,
                                    &mut self.task_hours,
                                    &mut self.task_assignee,
                                    &mut self.task_status,
                                    &mut self.task_editing,
                                    &mut self.show_task_edit_auth,
                                    &mut self.task_edit_pw,
                                    &mut self.show_task_delete,
                                    &mut self.task_delete_pw,
                                    status_line,
                                    show_history,
                                    history_entity,
                                    history_events,
                                    history_sel_a,
                                    history_sel_b,
                                    &self.tx,
                                );
                            } else if let Some(tid) = calendar_view::show_central_overview(
                                ui,
                                &mut self.cal,
                                &tasks_all,
                                &persons_all,
                            ) {
                                if let Some(t) = tasks_all.iter().find(|t| t.id == tid) {
                                    self.task_title = t.title.clone();
                                    self.task_plan = t.plan.clone();
                                    self.task_date = t.date.clone();
                                    self.task_start = t.start_hour;
                                    self.task_end_date = t.end_date.clone();
                                    self.task_end_hour = t.end_hour;
                                    self.task_hours = t.hours;
                                    self.task_assignee = t.assignee_id.clone();
                                    self.task_status = t.status;
                                    self.task_editing = false;
                                    *selected = None;
                                }
                            }
                            return;
                        }
                        if selected.is_none() {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    theme::muted_label(
                                        "在左侧选择一条备忘查看详情\n默认只读 · 编辑与删除需主密码",
                                    )
                                    .size(15.0),
                                );
                            });
                            return;
                        }
                        let id = selected.clone().unwrap();
                        let meta = filtered.iter().find(|m| m.id == id);

                        let avail_h = ui.available_height();
                        theme::card_frame().show(ui, |ui| {
                                ui.set_min_height(avail_h - 8.0);
                                ui.set_min_width(ui.available_width());
                                let title_show = if title_draft.is_empty() {
                                    "(无标题)"
                                } else {
                                    title_draft.as_str()
                                };
                                ui.horizontal(|ui| {
                                    ui.heading(
                                        RichText::new(title_show).size(24.0).color(theme::TEXT),
                                    );
                                    if *editing {
                                        ui.label(
                                            RichText::new("编辑中")
                                                .small()
                                                .color(theme::WARN),
                                        );
                                    } else {
                                        ui.label(theme::muted_label("只读").small());
                                    }
                                });
                                if let Some(m) = meta {
                                    let src = rt.svc.display_name_for(&m.node_id);
                                    ui.label(
                                        RichText::new(format!(
                                            "ID  {}    版本 {}    来源 {}",
                                            m.id, m.version, src
                                        ))
                                        .small()
                                        .color(theme::TEXT_MUTED),
                                    );
                                }
                                ui.add_space(10.0);
                                ui.horizontal_wrapped(|ui| {
                                    if !*editing {
                                        if theme::primary_button(ui, "编辑").clicked() {
                                            *show_edit_auth = true;
                                            edit_pw.clear();
                                        }
                                    } else {
                                        if theme::success_button(ui, "保存").clicked() {
                                            let svc = rt.svc.clone();
                                            let t = title_draft.clone();
                                            let b = body_draft.clone();
                                            let id2 = id.clone();
                                            let tx = self.tx.clone();
                                            std::thread::spawn(move || match svc.edit(&id2, &t, &b)
                                            {
                                                Ok(()) => {
                                                    let _ = tx.send(BgMsg::Info("已保存".into()));
                                                    let _ = tx.send(BgMsg::Refresh);
                                                }
                                                Err(e) => {
                                                    let _ = tx.send(BgMsg::Error(e.to_string()));
                                                }
                                            });
                                            *editing = false;
                                        }
                                        if theme::ghost_button(ui, "取消编辑").clicked() {
                                            *editing = false;
                                            if let Some(m) = meta {
                                                *title_draft = m.title.clone();
                                                *body_draft = m.content.clone();
                                            }
                                        }
                                    }
                                    ui.separator();
                                    if theme::ghost_button(ui, "复制全文").clicked() {
                                        let text = format!("{}\n\n{}", title_draft, body_draft);
                                        ui.output_mut(|o| o.copied_text = text);
                                        *status_line = "已复制到剪贴板".into();
                                    }
                                    if theme::ghost_button(ui, "复制标题").clicked() {
                                        ui.output_mut(|o| o.copied_text = title_draft.clone());
                                        *status_line = "标题已复制".into();
                                    }
                                    if theme::ghost_button(ui, "导出").clicked() {
                                        let dir = std::path::PathBuf::from(&rt.cfg.data_dir)
                                            .join("exports");
                                        let path = dir.join(format!(
                                            "memo-{}-{}.txt",
                                            &id[..8.min(id.len())],
                                            chrono_like_stamp()
                                        ));
                                        *export_ids = Some(vec![id.clone()]);
                                        *export_path = path.display().to_string();
                                        export_pw.clear();
                                        *show_export = true;
                                    }
                                    if theme::ghost_button(ui, "历史").clicked() {
                                        match rt.svc.history_for("memo", &id) {
                                            Ok(ev) => {
                                                *history_entity = "memo".into();
                                                *history_events = ev;
                                                *history_sel_a = None;
                                                *history_sel_b = None;
                                                *show_history = true;
                                            }
                                            Err(e) => {
                                                *status_line = format!("读取历史失败: {e}");
                                            }
                                        }
                                    }
                                });
                                ui.add_space(10.0);
                                ui.separator();
                                ui.add_space(8.0);

                                if *editing {
                                    ui.label(RichText::new("标题").strong());
                                    ui.add(
                                        egui::TextEdit::multiline(title_draft)
                                            .desired_rows(2)
                                            .desired_width(f32::INFINITY)
                                            .hint_text("标题"),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("内容").strong());
                                    let body_h = (ui.available_height() - 24.0).max(220.0);
                                    ui.add(
                                        egui::TextEdit::multiline(body_draft)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(
                                                ((body_h / 22.0) as usize).clamp(12, 40),
                                            )
                                            .hint_text("内容"),
                                    );
                                } else {
                                    ui.label(RichText::new("内容").strong().size(14.0));
                                    ui.add_space(4.0);
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .id_source("memo_body_scroll")
                                        .show(ui, |ui| {
                                            ui.set_min_width(ui.available_width());
                                            if body_draft.trim().is_empty() {
                                                ui.label(
                                                    theme::muted_label("（正文为空）").size(15.0),
                                                );
                                            } else {
                                                ui.label(
                                                    RichText::new(body_draft.as_str())
                                                        .size(15.5)
                                                        .color(theme::TEXT)
                                                        ,
                                                );
                                            }
                                        });
                                }
                            });
                    });

                if *show_edit_auth {
                    egui::Window::new("确认编辑")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                        .show(ctx, |ui| {
                            ui.label("为防止误改，编辑前请输入主密码确认");
                            ui.add_space(8.0);
                            theme::password_field(ui, edit_pw, "主密码", 280.0, 34.0);
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                    if theme::primary_button(ui, "确认并编辑").clicked() {
                                    let svc = rt.svc.clone();
                                    let pw = edit_pw.clone();
                                    let tx = self.tx.clone();
                                    std::thread::spawn(move || match svc.verify_password(&pw) {
                                        Ok(()) => {
                                            let _ = tx.send(BgMsg::EnterEdit);
                                        }
                                        Err(e) => {
                                            let _ = tx.send(BgMsg::Error(e.to_string()));
                                        }
                                    });
                                }
                                if ui.button("取消").clicked() {
                                    *show_edit_auth = false;
                                    edit_pw.clear();
                                }
                            });
                        });
                }

                if self.show_new_task {
                    egui::Window::new("新建任务")
                        .collapsible(false)
                        .resizable(true)
                        .default_size([480.0, 420.0])
                        .show(ctx, |ui| {
                            ui.label("标题");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.task_title)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.horizontal(|ui| {
                                ui.label("开始日期");
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut self.task_date)
                                            .desired_width(110.0)
                                            .hint_text("YYYY-MM-DD"),
                                    )
                                    .changed()
                                {
                                    calendar_view::ensure_end_after_start(
                                        &self.task_date,
                                        self.task_start,
                                        &mut self.task_end_date,
                                        &mut self.task_end_hour,
                                    );
                                    if let Ok(h) = memo_core::task::duration_hours(
                                        &self.task_date,
                                        self.task_start,
                                        &self.task_end_date,
                                        self.task_end_hour,
                                    ) {
                                        self.task_hours = h;
                                    }
                                }
                                ui.label("开始时");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut self.task_start)
                                            .speed(0.25)
                                            .clamp_range(0.0..=23.75),
                                    )
                                    .changed()
                                {
                                    calendar_view::ensure_end_after_start(
                                        &self.task_date,
                                        self.task_start,
                                        &mut self.task_end_date,
                                        &mut self.task_end_hour,
                                    );
                                    if let Ok(h) = memo_core::task::duration_hours(
                                        &self.task_date,
                                        self.task_start,
                                        &self.task_end_date,
                                        self.task_end_hour,
                                    ) {
                                        self.task_hours = h;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("结束日期");
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut self.task_end_date)
                                            .desired_width(110.0)
                                            .hint_text("YYYY-MM-DD"),
                                    )
                                    .changed()
                                {
                                    calendar_view::ensure_end_after_start(
                                        &self.task_date,
                                        self.task_start,
                                        &mut self.task_end_date,
                                        &mut self.task_end_hour,
                                    );
                                    if let Ok(h) = memo_core::task::duration_hours(
                                        &self.task_date,
                                        self.task_start,
                                        &self.task_end_date,
                                        self.task_end_hour,
                                    ) {
                                        self.task_hours = h;
                                    }
                                }
                                ui.label("结束时");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut self.task_end_hour)
                                            .speed(0.25)
                                            .clamp_range(0.0..=24.0),
                                    )
                                    .changed()
                                {
                                    calendar_view::ensure_end_after_start(
                                        &self.task_date,
                                        self.task_start,
                                        &mut self.task_end_date,
                                        &mut self.task_end_hour,
                                    );
                                    if let Ok(h) = memo_core::task::duration_hours(
                                        &self.task_date,
                                        self.task_start,
                                        &self.task_end_date,
                                        self.task_end_hour,
                                    ) {
                                        self.task_hours = h;
                                    }
                                }
                                ui.label("工时");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut self.task_hours)
                                            .speed(0.25)
                                            .clamp_range(0.25..=240.0),
                                    )
                                    .changed()
                                {
                                    calendar_view::sync_end_from_hours(
                                        &self.task_date,
                                        self.task_start,
                                        self.task_hours,
                                        &mut self.task_end_date,
                                        &mut self.task_end_hour,
                                    );
                                }
                            });
                            ui.label("负责人");
                            egui::ComboBox::from_id_source("new_task_assignee")
                                .selected_text(
                                    persons_all
                                        .iter()
                                        .find(|p| p.id == self.task_assignee)
                                        .map(|p| p.name.as_str())
                                        .unwrap_or("(选择)"),
                                )
                                .show_ui(ui, |ui| {
                                    for p in &persons_all {
                                        ui.selectable_value(
                                            &mut self.task_assignee,
                                            p.id.clone(),
                                            &p.name,
                                        );
                                    }
                                });
                            ui.label("状态");
                            egui::ComboBox::from_id_source("new_task_status")
                                .selected_text(self.task_status.label())
                                .show_ui(ui, |ui| {
                                    for s in TaskStatus::ALL {
                                        ui.selectable_value(
                                            &mut self.task_status,
                                            s,
                                            s.label(),
                                        );
                                    }
                                });
                            ui.label("计划");
                            ui.add(
                                egui::TextEdit::multiline(&mut self.task_plan)
                                    .desired_rows(8)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.horizontal(|ui| {
                                if theme::success_button(ui, "添加").clicked() {
                                    match rt.svc.add_task(
                                        &self.task_title,
                                        &self.task_plan,
                                        &self.task_date,
                                        self.task_start,
                                        &self.task_end_date,
                                        self.task_end_hour,
                                        &self.task_assignee,
                                        self.task_status,
                                    ) {
                                        Ok(id) => {
                                            self.show_new_task = false;
                                            self.cal.selected_task = Some(id.clone());
                                            *status_line = format!("已添加任务 {id}");
                                            let _ = self.tx.send(BgMsg::Refresh);
                                        }
                                        Err(e) => *status_line = format!("错误: {e}"),
                                    }
                                }
                                if ui.button("取消").clicked() {
                                    self.show_new_task = false;
                                }
                            });
                        });
                }

                if self.show_task_edit_auth {
                    egui::Window::new("验证主密码以编辑任务")
                        .collapsible(false)
                        .show(ctx, |ui| {
                            theme::password_field(
                                ui,
                                &mut self.task_edit_pw,
                                "主密码",
                                280.0,
                                34.0,
                            );
                            ui.horizontal(|ui| {
                                if theme::primary_button(ui, "确认").clicked() {
                                    match rt.svc.verify_password(&self.task_edit_pw) {
                                        Ok(()) => {
                                            self.task_editing = true;
                                            self.show_task_edit_auth = false;
                                            self.task_edit_pw.clear();
                                        }
                                        Err(e) => *status_line = e.to_string(),
                                    }
                                }
                                if ui.button("取消").clicked() {
                                    self.show_task_edit_auth = false;
                                }
                            });
                        });
                }

                if self.show_task_delete {
                    egui::Window::new("删除任务")
                        .collapsible(false)
                        .show(ctx, |ui| {
                            ui.label("删除任务需要主密码确认。");
                            theme::password_field(
                                ui,
                                &mut self.task_delete_pw,
                                "主密码",
                                280.0,
                                34.0,
                            );
                            ui.horizontal(|ui| {
                                if theme::danger_button(ui, "删除").clicked() {
                                    if let Some(id) = self.cal.selected_task.clone() {
                                        match rt.svc.delete_task(&id, &self.task_delete_pw) {
                                            Ok(()) => {
                                                self.cal.selected_task = None;
                                                self.show_task_delete = false;
                                                self.task_delete_pw.clear();
                                                *status_line = "任务已删除".into();
                                                let _ = self.tx.send(BgMsg::Refresh);
                                            }
                                            Err(e) => *status_line = e.to_string(),
                                        }
                                    }
                                }
                                if ui.button("取消").clicked() {
                                    self.show_task_delete = false;
                                }
                            });
                        });
                }

                if *show_new {
                    egui::Window::new("新建备忘")
                        .collapsible(false)
                        .resizable(true)
                        .default_size([520.0, 400.0])
                        .min_width(420.0)
                        .show(ctx, |ui| {
                            ui.label("标题");
                            // IME 已在 sanitize_cjk_ime_events 中修复；标题用多行框便于中文输入
                            ui.add(
                                egui::TextEdit::multiline(new_title)
                                    .desired_rows(2)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("输入标题…"),
                            );
                            ui.label("内容");
                            ui.add(
                                egui::TextEdit::multiline(new_body)
                                    .desired_rows(10)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("输入内容…"),
                            );
                            ui.horizontal(|ui| {
                                if theme::success_button(ui, "添加").clicked() {
                                    let svc = rt.svc.clone();
                                    let t = new_title.clone();
                                    let b = new_body.clone();
                                    let tx = self.tx.clone();
                                    *show_new = false;
                                    std::thread::spawn(move || match svc.add(&t, &b) {
                                        Ok(id) => {
                                            let _ = tx.send(BgMsg::Info(format!("已添加 {id}")));
                                            let _ = tx.send(BgMsg::Refresh);
                                        }
                                        Err(e) => {
                                            let _ = tx.send(BgMsg::Error(e.to_string()));
                                        }
                                    });
                                }
                                if ui.button("取消").clicked() {
                                    *show_new = false;
                                }
                            });
                        });
                }

                if *show_delete {
                    let delete_id = selected.clone().unwrap_or_default();
                    if delete_id.is_empty() {
                        *show_delete = false;
                    } else {
                        egui::Window::new("确认删除")
                            .collapsible(false)
                            .resizable(false)
                            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                            .show(ctx, |ui| {
                                ui.label(
                                    RichText::new("删除后不可轻易恢复，请输入主密码确认。").strong(),
                                );
                                ui.add_space(8.0);
                                ui.label("主密码");
                                theme::password_field(ui, delete_pw, "输入主密码", 280.0, 34.0);
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    let can_del = !delete_pw.is_empty();
                                    if ui
                                        .add_enabled(can_del, {
                                            egui::Button::new(
                                                RichText::new("确认删除")
                                                    .color(Color32::WHITE)
                                                    .strong(),
                                            )
                                            .fill(theme::DANGER)
                                        })
                                        .clicked()
                                    {
                                        let svc = rt.svc.clone();
                                        let id2 = delete_id.clone();
                                        let pw = delete_pw.clone();
                                        let tx = self.tx.clone();
                                        *show_delete = false;
                                        delete_pw.clear();
                                        std::thread::spawn(move || match svc.delete(&id2, &pw) {
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
                                        delete_pw.clear();
                                    }
                                });
                            });
                    }
                }

                if *show_export {
                    let scope = if export_ids.is_some() {
                        "导出当前备忘"
                    } else {
                        "导出全部备忘"
                    };
                    egui::Window::new(scope)
                        .collapsible(false)
                        .resizable(true)
                        .default_width(520.0)
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                        .show(ctx, |ui| {
                            ui.label(
                                RichText::new("导出为明文 TXT，请输入主密码确认，并选择保存位置。")
                                    .strong(),
                            );
                            ui.add_space(8.0);
                            ui.label("主密码");
                            theme::password_field(ui, export_pw, "输入主密码", 400.0, 34.0);
                            ui.add_space(8.0);
                            ui.label("保存路径");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(export_path)
                                        .desired_width(360.0)
                                        .hint_text("例如 C:\\Users\\…\\memo.txt"),
                                );
                                if ui.button("浏览…").clicked() {
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
                                        .fill(theme::SUCCESS)
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
                }

                if *show_history {
                    egui::Window::new(history_view::entity_window_title(history_entity))
                        .collapsible(false)
                        .resizable(true)
                        .default_size([560.0, 520.0])
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
                }

                if *show_backup {
                    let title = if *backup_import {
                        "导入加密备份"
                    } else {
                        "导出加密备份"
                    };
                    egui::Window::new(title)
                        .collapsible(false)
                        .resizable(false)
                        .show(ctx, |ui| {
                            ui.set_min_width(420.0);
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
                            theme::password_field(ui, backup_pw, "备份密码", 400.0, 34.0);
                            ui.add_space(6.0);
                            ui.label("文件路径");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(backup_path)
                                        .desired_width(300.0)
                                        .hint_text("*.memobak"),
                                );
                                if ui.button("浏览…").clicked() {
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
                                        .fill(theme::ACCENT),
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
                }

                if *show_settings {
                    egui::Window::new("设置")
                        .collapsible(false)
                        .resizable(true)
                        .default_width(480.0)
                        .show(ctx, |ui| {
                            ui.label(
                                RichText::new(
                                    "设置保存在系统用户配置目录。更改节点/端口/数据目录/盐后需重启。",
                                )
                                .weak(),
                            );
                            ui.separator();
                            ui.label("节点 ID");
                            ui.text_edit_singleline(&mut settings_draft.node_id);
                            ui.label("本机显示名（可选，仅本地展示）");
                            ui.text_edit_singleline(&mut settings_draft.node_display_name);
                            ui.label("数据目录");
                            ui.text_edit_singleline(&mut settings_draft.data_dir);
                            ui.label("监听端口");
                            let mut port = settings_draft.listen_port.to_string();
                            if ui.text_edit_singleline(&mut port).changed() {
                                if let Ok(p) = port.parse() {
                                    settings_draft.listen_port = p;
                                }
                            }
                            ui.label("对端列表（每行 host:port，可选；同网段可依赖自动发现）");
                            let mut peers_text = settings_draft.peers.join("\n");
                            if ui
                                .add(
                                    egui::TextEdit::multiline(&mut peers_text)
                                        .desired_rows(4)
                                        .desired_width(f32::INFINITY),
                                )
                                .changed()
                            {
                                settings_draft.peers = peers_text
                                    .lines()
                                    .map(|l| l.trim().to_string())
                                    .filter(|l| !l.is_empty())
                                    .collect();
                            }
                            ui.checkbox(&mut settings_draft.lan_discovery, "局域网自动发现 (UDP 17000)");
                            ui.label(
                                RichText::new(
                                    "集群盐 cluster_salt_hex（同局域网互通须一致；与主密码无关）",
                                )
                                .small()
                                .color(theme::TEXT_MUTED),
                            );
                            ui.text_edit_singleline(&mut settings_draft.cluster_salt_hex);
                            ui.label(
                                RichText::new(
                                    "主密码仅用于本机解锁；各机主密码可不同。改集群盐后需重启。",
                                )
                                .small()
                                .color(theme::TEXT_MUTED),
                            );
                            ui.add_space(8.0);
                            ui.separator();
                            ui.label(RichText::new("加密备份").strong());
                            ui.horizontal(|ui| {
                                if ui.button("导出加密备份…").clicked() {
                                    let dir = std::path::PathBuf::from(&rt.cfg.data_dir)
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
                            ui.horizontal(|ui| {
                                if ui.button("保存").clicked() {
                                    let before = self.cfg.clone();
                                    match config::save_settings(settings_draft) {
                                        Ok(()) => {
                                            let msg = if config::needs_restart(
                                                &before,
                                                settings_draft,
                                            ) {
                                                "已保存，请重启程序使关键设置生效"
                                            } else {
                                                "已保存（对端列表重启后完全生效）"
                                            };
                                            let _ = self.tx.send(BgMsg::Info(msg.into()));
                                            *show_settings = false;
                                        }
                                        Err(e) => {
                                            let _ = self.tx.send(BgMsg::Error(e.to_string()));
                                        }
                                    }
                                }
                                if ui.button("取消").clicked() {
                                    *show_settings = false;
                                }
                            });
                        });
                }
            }
        }

        if let Some(rt) = enter_main_rt {
            self.screen = Self::make_main(rt, &self.cfg);
        }

        Self::draw_help_about_windows(ctx, &mut show_help, &mut show_about);
        self.show_help = show_help;
        self.show_about = show_about;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_task_central(
    ui: &mut egui::Ui,
    rt: &AppRuntime,
    persons: &[memo_core::PersonView],
    cal: &mut CalUi,
    task_title: &mut String,
    task_plan: &mut String,
    task_date: &mut String,
    task_start: &mut f32,
    task_end_date: &mut String,
    task_end_hour: &mut f32,
    task_hours: &mut f32,
    task_assignee: &mut String,
    task_status: &mut TaskStatus,
    task_editing: &mut bool,
    show_task_edit_auth: &mut bool,
    _task_edit_pw: &mut String,
    show_task_delete: &mut bool,
    _task_delete_pw: &mut String,
    status_line: &mut String,
    show_history: &mut bool,
    history_entity: &mut String,
    history_events: &mut Vec<HistoryEvent>,
    history_sel_a: &mut Option<usize>,
    history_sel_b: &mut Option<usize>,
    tx: &Sender<BgMsg>,
) {
    let Some(tid) = cal.selected_task.clone() else {
        return;
    };
    let meta = rt.svc.get_task(&tid);
    let avail_h = ui.available_height();

    theme::card_frame().show(ui, |ui| {
        ui.set_min_height(avail_h - 8.0);
        ui.set_min_width(ui.available_width());

        let title_show = if task_title.is_empty() {
            "(无标题)"
        } else {
            task_title.as_str()
        };
        ui.horizontal(|ui| {
            ui.heading(RichText::new(title_show).size(24.0).color(theme::TEXT));
            status_badge(ui, *task_status);
            if *task_editing {
                ui.label(RichText::new("编辑中").small().color(theme::WARN));
            } else {
                ui.label(theme::muted_label("只读").small());
            }
        });
        if let Some(t) = &meta {
            let assignee = persons
                .iter()
                .find(|p| p.id == t.assignee_id)
                .map(|p| p.name.as_str())
                .unwrap_or("?");
            ui.label(
                RichText::new(format!(
                    "{}  ·  {:.1}h  ·  {}  ·  {}  ·  v{}",
                    calendar_view::format_datetime_range(
                        &t.date,
                        t.start_hour,
                        &t.end_date,
                        t.end_hour
                    ),
                    t.hours,
                    assignee,
                    t.status.label(),
                    t.version
                ))
                .size(13.0)
                .color(theme::TEXT_MUTED),
            );
        }
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            if !*task_editing {
                if theme::primary_button(ui, "编辑").clicked() {
                    *show_task_edit_auth = true;
                }
            } else {
                if theme::success_button(ui, "保存").clicked() {
                    match rt.svc.update_task(
                        &tid,
                        task_title,
                        task_plan,
                        task_date,
                        *task_start,
                        task_end_date,
                        *task_end_hour,
                        task_assignee,
                        *task_status,
                    ) {
                        Ok(()) => {
                            *task_editing = false;
                            *status_line = "任务已保存".into();
                            let _ = tx.send(BgMsg::Refresh);
                        }
                        Err(e) => *status_line = format!("错误: {e}"),
                    }
                }
                if theme::ghost_button(ui, "取消编辑").clicked() {
                    *task_editing = false;
                    if let Some(t) = &meta {
                        *task_title = t.title.clone();
                        *task_plan = t.plan.clone();
                        *task_date = t.date.clone();
                        *task_start = t.start_hour;
                        *task_end_date = t.end_date.clone();
                        *task_end_hour = t.end_hour;
                        *task_hours = t.hours;
                        *task_assignee = t.assignee_id.clone();
                        *task_status = t.status;
                    }
                }
            }
            ui.separator();
            if theme::ghost_button(ui, "历史").clicked() {
                match rt.svc.history_for("task", &tid) {
                    Ok(ev) => {
                        *history_entity = "task".into();
                        *history_events = ev;
                        *history_sel_a = None;
                        *history_sel_b = None;
                        *show_history = true;
                    }
                    Err(e) => *status_line = format!("读取历史失败: {e}"),
                }
            }
            if theme::ghost_button(ui, "复制计划").clicked() {
                let text = format!("{}\n\n{}", task_title, task_plan);
                ui.output_mut(|o| o.copied_text = text);
                *status_line = "已复制到剪贴板".into();
            }
            if theme::ghost_button(ui, "删除…").clicked() {
                *show_task_delete = true;
            }
        });
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        if *task_editing {
            ui.label(RichText::new("标题").small().color(theme::TEXT_MUTED));
            ui.add(egui::TextEdit::singleline(task_title).desired_width(f32::INFINITY));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("开始日期").small().color(theme::TEXT_MUTED));
                    if ui
                        .add(
                            egui::TextEdit::singleline(task_date)
                                .desired_width(110.0)
                                .hint_text("YYYY-MM-DD"),
                        )
                        .changed()
                    {
                        calendar_view::ensure_end_after_start(
                            task_date,
                            *task_start,
                            task_end_date,
                            task_end_hour,
                        );
                        if let Ok(h) = memo_core::task::duration_hours(
                            task_date,
                            *task_start,
                            task_end_date,
                            *task_end_hour,
                        ) {
                            *task_hours = h;
                        }
                    }
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new("开始时").small().color(theme::TEXT_MUTED));
                    if ui
                        .add(
                            egui::DragValue::new(task_start)
                                .speed(0.25)
                                .clamp_range(0.0..=23.75),
                        )
                        .changed()
                    {
                        calendar_view::ensure_end_after_start(
                            task_date,
                            *task_start,
                            task_end_date,
                            task_end_hour,
                        );
                        if let Ok(h) = memo_core::task::duration_hours(
                            task_date,
                            *task_start,
                            task_end_date,
                            *task_end_hour,
                        ) {
                            *task_hours = h;
                        }
                    }
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new("结束日期").small().color(theme::TEXT_MUTED));
                    if ui
                        .add(
                            egui::TextEdit::singleline(task_end_date)
                                .desired_width(110.0)
                                .hint_text("YYYY-MM-DD"),
                        )
                        .changed()
                    {
                        calendar_view::ensure_end_after_start(
                            task_date,
                            *task_start,
                            task_end_date,
                            task_end_hour,
                        );
                        if let Ok(h) = memo_core::task::duration_hours(
                            task_date,
                            *task_start,
                            task_end_date,
                            *task_end_hour,
                        ) {
                            *task_hours = h;
                        }
                    }
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new("结束时").small().color(theme::TEXT_MUTED));
                    if ui
                        .add(
                            egui::DragValue::new(task_end_hour)
                                .speed(0.25)
                                .clamp_range(0.0..=24.0),
                        )
                        .changed()
                    {
                        calendar_view::ensure_end_after_start(
                            task_date,
                            *task_start,
                            task_end_date,
                            task_end_hour,
                        );
                        if let Ok(h) = memo_core::task::duration_hours(
                            task_date,
                            *task_start,
                            task_end_date,
                            *task_end_hour,
                        ) {
                            *task_hours = h;
                        }
                    }
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new("工时").small().color(theme::TEXT_MUTED));
                    if ui
                        .add(
                            egui::DragValue::new(task_hours)
                                .speed(0.25)
                                .clamp_range(0.25..=240.0),
                        )
                        .changed()
                    {
                        calendar_view::sync_end_from_hours(
                            task_date,
                            *task_start,
                            *task_hours,
                            task_end_date,
                            task_end_hour,
                        );
                    }
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new("负责人").small().color(theme::TEXT_MUTED));
                    let label = persons
                        .iter()
                        .find(|p| p.id == *task_assignee)
                        .map(|p| p.name.as_str())
                        .unwrap_or("(未指定)");
                    egui::ComboBox::from_id_source("task_assignee")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for p in persons {
                                ui.selectable_value(task_assignee, p.id.clone(), &p.name);
                            }
                        });
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new("状态").small().color(theme::TEXT_MUTED));
                    egui::ComboBox::from_id_source("task_status")
                        .selected_text(task_status.label())
                        .show_ui(ui, |ui| {
                            for s in TaskStatus::ALL {
                                ui.selectable_value(task_status, s, s.label());
                            }
                        });
                });
            });
            ui.add_space(8.0);
            ui.label(RichText::new("工作计划").strong().size(14.0));
            let plan_h = (ui.available_height() - 24.0).max(200.0);
            ui.add(
                egui::TextEdit::multiline(task_plan)
                    .desired_rows(((plan_h / 22.0) as usize).clamp(10, 36))
                    .desired_width(f32::INFINITY),
            );
        } else {
            // meta chips
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("时段").small().color(theme::TEXT_MUTED));
                ui.label(
                    RichText::new(calendar_view::format_datetime_range(
                        task_date,
                        *task_start,
                        task_end_date,
                        *task_end_hour,
                    ))
                    .size(14.0)
                    .color(theme::TEXT),
                );
                ui.add_space(12.0);
                ui.label(RichText::new("工时").small().color(theme::TEXT_MUTED));
                ui.label(
                    RichText::new(format!("{:.1} h", *task_hours))
                        .size(14.0)
                        .color(theme::TEXT),
                );
                ui.add_space(12.0);
                ui.label(RichText::new("负责人").small().color(theme::TEXT_MUTED));
                let name = persons
                    .iter()
                    .find(|p| p.id == *task_assignee)
                    .map(|p| p.name.as_str())
                    .unwrap_or("?");
                ui.label(RichText::new(name).size(14.0).color(theme::TEXT));
                ui.add_space(12.0);
                ui.label(RichText::new("状态").small().color(theme::TEXT_MUTED));
                status_badge(ui, *task_status);
            });
            ui.add_space(12.0);
            ui.label(RichText::new("工作计划").strong().size(14.0));
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_source("task_plan_scroll")
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if task_plan.trim().is_empty() {
                        ui.label(theme::muted_label("（暂无计划正文）").size(15.0));
                    } else {
                        ui.label(
                            RichText::new(task_plan.as_str())
                                .size(15.5)
                                .color(theme::TEXT),
                        );
                    }
                });
        }
    });
}

fn status_badge(ui: &mut egui::Ui, status: TaskStatus) {
    let (fg, bg) = status_colors(status);
    egui::Frame::none()
        .fill(bg)
        .rounding(Rounding::same(4.0))
        .inner_margin(Margin::symmetric(8.0, 3.0))
        .show(ui, |ui| {
            ui.label(RichText::new(status.label()).size(12.0).strong().color(fg));
        });
}

fn status_colors(status: TaskStatus) -> (Color32, Color32) {
    match status {
        TaskStatus::NotStarted => (theme::TEXT_MUTED, theme::PANEL),
        TaskStatus::InProgress => (theme::ACCENT, theme::ACCENT_SOFT),
        TaskStatus::Paused => (theme::WARN, Color32::from_rgb(0xFE, 0xF3, 0xC7)),
        TaskStatus::Blocked => (
            Color32::from_rgb(0xC2, 0x41, 0x0C),
            Color32::from_rgb(0xFF, 0xED, 0xD5),
        ),
        TaskStatus::Cancelled => (theme::TEXT_MUTED, Color32::from_rgb(0xF1, 0xF5, 0xF9)),
        TaskStatus::Done => (theme::SUCCESS, theme::SUCCESS_SOFT),
    }
}

fn chrono_like_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// 单行省略：按像素宽度截断并加省略号，避免列表文字溢出到下一行。
fn ellipsize_ui(ui: &egui::Ui, text: &str, font_id: egui::FontId, max_width: f32) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    if max_width <= 8.0 {
        return "…".into();
    }
    let fits = |s: &str| {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_owned(), font_id.clone(), Color32::WHITE)
                .size()
                .x
        }) <= max_width
    };
    if fits(&flat) {
        return flat;
    }
    let chars: Vec<char> = flat.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(&candidate) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        "…".into()
    } else {
        chars[..lo].iter().collect::<String>() + "…"
    }
}

fn is_first_setup(cfg: &Config) -> bool {
    !std::path::Path::new(&cfg.data_dir)
        .join("keys")
        .join("private.key")
        .exists()
}

fn unlock_runtime(cfg: Config, password: &[u8]) -> anyhow::Result<Arc<AppRuntime>> {
    let rt = tokio::runtime::Runtime::new()?;
    let _guard = rt.enter();
    let (svc, _audit) = memo_core::service::unlock(cfg.clone(), password)?;
    let engine = SyncEngine::new(
        cfg.node_id.clone(),
        cfg.listen_port,
        cfg.peers.clone(),
        svc.store(),
        svc.person_store(),
        svc.task_store(),
        cfg.cluster_salt_hex.clone(),
        cfg.lan_discovery,
        std::path::PathBuf::from(&cfg.data_dir),
    );
    engine.set_service(&svc);
    svc.set_broadcaster(Arc::new(EngineBroadcaster::new(engine.clone())));
    engine.start();
    std::mem::forget(rt);
    Ok(Arc::new(AppRuntime { svc, engine, cfg }))
}

pub fn run_gui(cfg: Config) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("分布式备忘录"),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "分布式备忘录",
        options,
        Box::new(|cc| {
            fonts::configure_cjk_fonts(&cc.egui_ctx);
            Box::new(MemoApp::new(cfg))
        }),
    )
}
