mod app_icon;
mod calendar_view;
mod china_calendar;
mod doc_editor;
mod fonts;
mod help_view;
mod history_view;
mod identity_gate;
mod people_view;
mod save_dialog;
mod single_instance;
mod theme;

use calendar_view::CalUi;
use eframe::egui::{self, Color32, Frame, Margin, RichText, Rounding, Stroke, Vec2};
use identity_gate::{GateAction, IdentityGateState};
use memo_core::config::{self, Config, NodeRole};
use memo_core::identity_keys::IdentityKeys;
use memo_core::license::{self, LicenseStatus};
use memo_core::service::{HistoryEvent, MemoService};
use memo_core::store::{MemoLifecycle, MemoVisibility};
use memo_core::person::Gender;
use memo_core::task::{TaskKind, TaskStatus};
use memo_core::format_bytes;
use memo_sync::{
    BackupUiStatus, DiscoveredPeer, EngineBroadcaster, PeerStatus, SyncEngine, UdpPulse,
};
use people_view::PeopleUi;
use std::path::PathBuf;
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

/// 新建备忘模板：第一行标题，下面正文/事项/表格。
fn memo_new_template() -> String {
    doc_editor::memo_template(&chrono_like_stamp())
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
    /// 模板新建完成：选中并直接进入编辑
    CreatedMemo {
        id: String,
        title: String,
        body: String,
    },
}

struct AppRuntime {
    svc: Arc<MemoService>,
    engine: Arc<SyncEngine>,
    cfg: Config,
    /// 保持 tokio 运行时存活（同步引擎依赖它）
    _rt: tokio::runtime::Runtime,
}

enum Screen {
    IdentityGate(IdentityGateState),
    Main {
        rt: Arc<AppRuntime>,
        search: String,
        selected: Option<String>,
        /// 仅编辑模式下可改
        title_draft: String,
        body_draft: String,
        /// 编辑中的生命周期预设：0=永久 30/90/365 天
        lifecycle_days: i64,
        /// 编辑中的可见性（默认私密）
        visibility_draft: MemoVisibility,
        /// false=只读浏览；true=编辑态
        editing: bool,
        show_settings: bool,
        settings_draft: Config,
        show_delete: bool,
        /// 回收站窗口
        show_trash: bool,
        /// 彻底删除二次确认的 memo id
        purge_confirm_id: Option<String>,
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
    task_kind: TaskKind,
    task_on_calendar: bool,
    task_remind: bool,
    task_editing: bool,
    show_new_task: bool,
    show_task_delete: bool,
    task_delete_pw: String,
    show_task_edit_auth: bool,
    task_edit_pw: String,
    pending_switch_person: bool,
    /// 右侧节点栏（默认收起，给详情更多宽度）
    show_nodes: bool,
    /// 备忘公文编辑文档（标题=第一行）
    memo_doc: doc_editor::Doc,
    /// 任务计划公文编辑文档（标题=第一行，同步 task_title）
    task_doc: doc_editor::Doc,
}

const HELP_DOC: &str = include_str!("HELP.md");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

impl MemoApp {
    pub fn new(cfg: Config) -> Self {
        let (tx, rx) = mpsc::channel();
        let gate = IdentityGateState::from_cfg(&cfg);
        Self {
            cfg,
            screen: Screen::IdentityGate(gate),
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
            task_kind: TaskKind::Normal,
            task_on_calendar: true,
            task_remind: false,
            task_editing: false,
            show_new_task: false,
            show_task_delete: false,
            task_delete_pw: String::new(),
            show_task_edit_auth: false,
            task_edit_pw: String::new(),
            pending_switch_person: false,
            show_nodes: false,
            memo_doc: doc_editor::Doc::empty(),
            task_doc: doc_editor::Doc::empty(),
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
            lifecycle_days: 0,
            visibility_draft: MemoVisibility::Private,
            editing: false,
            show_settings: false,
            settings_draft: cfg.clone(),
            show_delete: false,
            show_trash: false,
            purge_confirm_id: None,
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
        app_icon::show(ui, 20.0);
        ui.add_space(4.0);
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
                    ui.horizontal(|ui| {
                        app_icon::show(ui, 48.0);
                        ui.add_space(10.0);
                        ui.vertical(|ui| {
                            ui.label(theme::brand_title(22.0));
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(theme::APP_DESCRIPTION)
                                    .color(theme::TEXT_MUTED)
                                    .size(13.0),
                            );
                        });
                    });
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
                    ui.label(RichText::new("· 身份密钥对登录 · 强制导出备份").color(theme::TEXT));
                    ui.label(RichText::new("· 私人密文异地托管 · 公开备忘局域网同步").color(theme::TEXT));
                    ui.label(RichText::new("· Ed25519 审计链 · 变更时间线").color(theme::TEXT));
                    ui.label(RichText::new("· 日历：月历 / 周视图 / 甘特与工时").color(theme::TEXT));
                    ui.label(RichText::new("· 加密备份 (.memobak)").color(theme::TEXT));
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
                    let cfg = rt.cfg.clone();
                    self.cfg = cfg.clone();
                    self.screen = Self::make_main(rt, &cfg);
                }
                BgMsg::UnlockResult(Err(e)) => {
                    if let Screen::IdentityGate(st) = &mut self.screen {
                        st.status = format!("打开失败: {e}");
                        st.busy = false;
                    }
                }
                BgMsg::Error(e) => {
                    match &mut self.screen {
                        Screen::Main { status_line, .. } => {
                            *status_line = format!("错误: {e}");
                        }
                        Screen::IdentityGate(st) => {
                            st.status = format!("错误: {e}");
                            st.busy = false;
                        }
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
                    if let Screen::Main {
                        title_draft,
                        body_draft,
                        ..
                    } = &self.screen
                    {
                        self.memo_doc =
                            doc_editor::Doc::from_store(title_draft, body_draft);
                    }
                }
                BgMsg::CreatedMemo { id, title, body } => {
                    if let Screen::Main {
                        selected,
                        title_draft,
                        body_draft,
                        editing,
                        show_edit_auth,
                        status_line,
                        ..
                    } = &mut self.screen
                    {
                        *selected = Some(id);
                        *title_draft = title;
                        *body_draft = body;
                        *editing = true;
                        *show_edit_auth = false;
                        *status_line = "已创建，直接修改后保存即可".into();
                    }
                    if let Screen::Main {
                        title_draft,
                        body_draft,
                        ..
                    } = &self.screen
                    {
                        self.memo_doc =
                            doc_editor::Doc::from_store(title_draft, body_draft);
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
                        lifecycle_days,
                        visibility_draft,
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
                                    *lifecycle_days = match &m.lifecycle {
                                        MemoLifecycle::Permanent => 0,
                                        MemoLifecycle::ExpiresAt { .. } => 30,
                                    };
                                    *visibility_draft = m.visibility;
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

fn udp_pulse_row(pulse: &UdpPulse) -> (bool, &'static str, String) {
    if !pulse.lan_on {
        return (false, "发现已关闭", String::new());
    }
    if pulse.is_master {
        let lit = pulse.tx_pulse;
        let detail = if pulse.tx_pulse {
            "发送中".into()
        } else if let Some(ago) = pulse.last_tx_ago_secs {
            format!("{ago}s 前发包")
        } else if pulse.broadcasting {
            "等待首次广播…".into()
        } else {
            "未广播（已关可见性）".into()
        };
        (lit, "广播心跳", detail)
    } else {
        let lit = pulse.rx_pulse;
        let detail = if pulse.rx_pulse {
            "收到广播".into()
        } else if let Some(ago) = pulse.last_rx_ago_secs {
            format!("{ago}s 前收到")
        } else {
            "等待主机信号…".into()
        };
        (lit, "主机信号", detail)
    }
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
            let mut gate = IdentityGateState::from_cfg(&self.cfg);
            gate.skip_auto_unlock = true;
            self.screen = Screen::IdentityGate(gate);
            self.pending_switch_person = false;
        }

        if let Screen::Main { .. } = &self.screen {
            ctx.request_repaint_after(Duration::from_secs(2));
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
                lifecycle_days,
                visibility_draft,
                editing,
                show_settings,
                settings_draft,
                show_delete,
                show_trash,
                purge_confirm_id,
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

                egui::TopBottomPanel::top("top")
                    .frame(theme::top_bar_frame())
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            app_icon::show(ui, 22.0);
                            ui.add_space(6.0);
                            ui.label(theme::brand_title_on_navy(17.0));
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(format!(
                                    "身份 {} ({})",
                                    rt.svc.session_alias(),
                                    IdentityKeys::short_fp(rt.svc.session_fp())
                                ))
                                .color(Color32::from_rgb(0xE2, 0xE8, 0xF0))
                                .size(13.0),
                            );
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("退出")
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
                                        self.task_doc = doc_editor::parse(
                                            &doc_editor::task_plan_template("未命名任务"),
                                        );
                                        self.task_title = self.task_doc.title.clone();
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
                                        self.task_kind = TaskKind::Normal;
                                        self.task_on_calendar = true;
                                        self.task_remind = false;
                                    } else {
                                        let note = memo_new_template();
                                        let (title, body) =
                                            doc_editor::split_note(&note, "未命名备忘");
                                        let svc = rt.svc.clone();
                                        let tx = self.tx.clone();
                                        std::thread::spawn(move || {
                                            match svc.add(
                                                &title,
                                                &body,
                                                MemoVisibility::Private,
                                            ) {
                                                Ok(id) => {
                                                    let _ = tx.send(BgMsg::CreatedMemo {
                                                        id,
                                                        title,
                                                        body,
                                                    });
                                                }
                                                Err(e) => {
                                                    let _ = tx.send(BgMsg::Error(e.to_string()));
                                                }
                                            }
                                        });
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
                                theme::muted_label(if !rt.cfg.lan_discovery {
                                    "局域网发现已关闭"
                                } else if rt.cfg.node_role.is_master() {
                                    "主机 10s 广播 · 从机/主机登记 · TCP 通道"
                                } else {
                                    "从机：收听主机广播并登记（不广播）"
                                })
                                .small(),
                            );
                            ui.label(
                                RichText::new(format!("本机角色：{}", rt.cfg.node_role.label()))
                                    .small()
                                    .color(theme::TEXT_MUTED),
                            );
                            {
                                let pulse = rt.engine.udp_pulse();
                                ui.horizontal(|ui| {
                                    let (lit, label, detail) = udp_pulse_row(&pulse);
                                    let (rect, _) = ui.allocate_exact_size(
                                        Vec2::splat(8.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        3.5,
                                        if lit {
                                            theme::SUCCESS
                                        } else {
                                            theme::BORDER_STRONG
                                        },
                                    );
                                    ui.label(
                                        RichText::new(label)
                                            .small()
                                            .color(if lit {
                                                theme::SUCCESS
                                            } else {
                                                theme::TEXT_MUTED
                                            }),
                                    );
                                    ui.label(
                                        RichText::new(detail)
                                            .small()
                                            .color(theme::TEXT_MUTED),
                                    );
                                });
                                ctx.request_repaint_after(Duration::from_millis(100));
                            }
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
                            {
                                let g = rt.svc.current_person_gender();
                                let tag = g.tag();
                                if !tag.is_empty() {
                                    let who = rt
                                        .svc
                                        .current_person_id()
                                        .map(|id| rt.svc.person_name(&id))
                                        .unwrap_or_else(|| "本机".into());
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new("本机当前")
                                                .small()
                                                .color(theme::TEXT_MUTED),
                                        );
                                        gender_chip(ui, g);
                                        ui.label(
                                            RichText::new(who)
                                                .small()
                                                .color(theme::TEXT),
                                        );
                                    });
                                } else {
                                    ui.label(
                                        theme::muted_label("本机未设置当前人员性别（人员目录中「设为当前」）")
                                            .small(),
                                    );
                                }
                            }
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                if discovered.is_empty() {
                                    ui.label(
                                        theme::muted_label(if rt.cfg.node_role.is_master() {
                                            "暂无从机/对端主机\n请确认同网段、同 salt，并放行 UDP 17000"
                                        } else {
                                            "暂未发现主机\n请确认有主机在广播，同网段、同 salt"
                                        })
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
                                                let title = if d.alias.is_empty() {
                                                    d.node_id.clone()
                                                } else {
                                                    format!("{} · {}", d.alias, d.node_id)
                                                };
                                                ui.label(
                                                    RichText::new(title)
                                                        .strong()
                                                        .small()
                                                        .color(theme::TEXT),
                                                );
                                                if d.gender.tag() != "" {
                                                    gender_chip(ui, d.gender);
                                                }
                                            });
                                            if !d.key_fingerprint.is_empty() {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "身份 {}",
                                                        IdentityKeys::short_fp(&d.key_fingerprint)
                                                    ))
                                                    .small()
                                                    .color(theme::TEXT_MUTED),
                                                );
                                            }
                                            ui.label(
                                                RichText::new(&d.addr)
                                                    .small()
                                                    .color(theme::TEXT_MUTED),
                                            );
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(d.role.label())
                                                        .small()
                                                        .color(theme::ACCENT),
                                                );
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
                                            if rt.cfg.node_role.is_master() {
                                                if let Some(mut acl) = rt
                                                    .engine
                                                    .list_peer_acls()
                                                    .into_iter()
                                                    .find(|a| a.node_id == d.node_id)
                                                {
                                                    let mut priv_b = acl.allow_private_backup;
                                                    let mut pub_s = acl.allow_public_sync;
                                                    ui.horizontal(|ui| {
                                                        if ui
                                                            .checkbox(&mut priv_b, "私有备份")
                                                            .changed()
                                                            || ui
                                                                .checkbox(&mut pub_s, "公开同步")
                                                                .changed()
                                                        {
                                                            let _ = rt.engine.set_peer_acl(
                                                                &acl.node_id,
                                                                priv_b,
                                                                pub_s,
                                                            );
                                                            acl.allow_private_backup = priv_b;
                                                            acl.allow_public_sync = pub_s;
                                                        }
                                                    });
                                                }
                                            }
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
                let female_current =
                    matches!(rt.svc.current_person_gender(), Gender::Female);
                let cycle_cfg = rt.svc.current_cycle();
                if let Some(pid) = rt.svc.current_person_id() {
                    if female_current && self.cal.cycle_synced_person != pid {
                        self.cal.load_cycle_draft(cycle_cfg.as_ref());
                        self.cal.cycle_synced_person = pid;
                    }
                } else {
                    self.cal.cycle_synced_person.clear();
                }
                if let Some(cfg) = self.cal.take_cycle_save() {
                    if let Some(pid) = rt.svc.current_person_id() {
                        match rt.svc.set_cycle(&pid, cfg) {
                            Ok(()) => {
                                *status_line = "周期已保存（仅本机）".into();
                                self.cal.cycle_synced_person = pid;
                            }
                            Err(e) => *status_line = format!("保存周期失败: {e}"),
                        }
                    } else {
                        *status_line = "请先设置当前人员".into();
                    }
                }

                let reminders = rt.svc.today_reminders();
                if !reminders.is_empty() {
                    egui::TopBottomPanel::top("remind_banner")
                        .frame(
                            Frame::none()
                                .fill(Color32::from_rgb(0xFE, 0xF3, 0xC7))
                                .inner_margin(Margin::symmetric(16.0, 6.0)),
                        )
                        .show(ctx, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new("今日提醒")
                                        .strong()
                                        .size(13.0)
                                        .color(Color32::from_rgb(0xB4, 0x53, 0x09)),
                                );
                                for t in &reminders {
                                    let label = format!("[{}] {}", t.kind.label(), t.title);
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(label)
                                                    .size(13.0)
                                                    .color(calendar_view::kind_color(t.kind)),
                                            )
                                            .frame(false),
                                        )
                                        .on_hover_text("打开任务")
                                        .clicked()
                                    {
                                        self.left_tab = LeftTab::Calendar;
                                        apply_selected_task(
                                            &mut self.cal,
                                            &mut self.task_title,
                                            &mut self.task_plan,
                                            &mut self.task_doc,
                                            &mut self.task_date,
                                            &mut self.task_start,
                                            &mut self.task_end_date,
                                            &mut self.task_end_hour,
                                            &mut self.task_hours,
                                            &mut self.task_assignee,
                                            &mut self.task_status,
                                            &mut self.task_kind,
                                            &mut self.task_on_calendar,
                                            &mut self.task_remind,
                                            &mut self.task_editing,
                                            t,
                                        );
                                        *selected = None;
                                        *editing = false;
                                    }
                                    ui.separator();
                                }
                            });
                        });
                }

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
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let trash_n = rt.svc.list_trash().len();
                                let label = if trash_n > 0 {
                                    format!("回收站 ({trash_n})")
                                } else {
                                    "回收站".into()
                                };
                                if theme::ghost_button(ui, &label).clicked() {
                                    *show_trash = true;
                                    *purge_confirm_id = None;
                                }
                            });
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
                                            "还没有备忘，点击右上角「新建」即可记下第一笔",
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
                                        let show_bak = self.cfg.show_backup_status;
                                        let bak = if show_bak {
                                            let owner = if m.owner_fp.is_empty() {
                                                rt.svc.session_fp().to_string()
                                            } else {
                                                m.owner_fp.clone()
                                            };
                                            Some(rt.engine.backup_status_for(
                                                &owner,
                                                &m.id,
                                                m.version,
                                                &m.modified_at,
                                                m.visibility,
                                            ))
                                        } else {
                                            None
                                        };
                                        let bak_color = bak.map(|b| match b {
                                            BackupUiStatus::Synced => theme::SUCCESS,
                                            BackupUiStatus::Queued | BackupUiStatus::Pending => {
                                                theme::WARN
                                            }
                                            BackupUiStatus::Offline | BackupUiStatus::Public => {
                                                theme::TEXT_MUTED
                                            }
                                        });

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
                                            let badge_font = egui::FontId::proportional(11.0);
                                            let badge_galley = bak.zip(bak_color).map(|(b, c)| {
                                                ui.fonts(|f| {
                                                    f.layout_no_wrap(
                                                        b.label().to_string(),
                                                        badge_font.clone(),
                                                        c,
                                                    )
                                                })
                                            });
                                            let badge_w = badge_galley
                                                .as_ref()
                                                .map(|g| g.size().x + 4.0)
                                                .unwrap_or(0.0);
                                            let text_w =
                                                (rect.width() - pad_x * 2.0 - badge_w).max(12.0);
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
                                            if let (Some(galley), Some(c)) =
                                                (badge_galley, bak_color)
                                            {
                                                let gw = galley.size().x;
                                                painter.galley(
                                                    egui::pos2(
                                                        rect.right() - pad_x - gw,
                                                        rect.top() + pad_y + 1.0,
                                                    ),
                                                    galley,
                                                    c,
                                                );
                                            }
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
                                            *lifecycle_days = match &m.lifecycle {
                                                MemoLifecycle::Permanent => 0,
                                                MemoLifecycle::ExpiresAt { .. } => 30,
                                            };
                                            *visibility_draft = m.visibility;
                                            *editing = false;
                                            self.cal.selected_task = None;
                                        }
                                        resp.context_menu(|ui| {
                                            if ui.button("打开").clicked() {
                                                *selected = Some(item_id.clone());
                                                *title_draft = item_title.clone();
                                                *body_draft = item_body.clone();
                                                *lifecycle_days = match &m.lifecycle {
                                                    MemoLifecycle::Permanent => 0,
                                                    MemoLifecycle::ExpiresAt { .. } => 30,
                                                };
                                                *visibility_draft = m.visibility;
                                                *editing = false;
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            if ui
                                                .add(egui::Button::new(
                                                    RichText::new("移入回收站…").color(theme::DANGER),
                                                ))
                                                .clicked()
                                            {
                                                *selected = Some(item_id.clone());
                                                *title_draft = item_title.clone();
                                                *body_draft = item_body.clone();
                                                *editing = false;
                                                *show_delete = true;
                                                ui.close_menu();
                                            }
                                        });
                                        ui.add_space(6.0);
                                    }
                                });
                            }
                            LeftTab::Calendar => {
                                if let Some(tid) = calendar_view::show_left(
                                    ui,
                                    &mut self.cal,
                                    &tasks_all,
                                    &persons_all,
                                    female_current,
                                    cycle_cfg.as_ref(),
                                ) {
                                    if let Some(t) = tasks_all.iter().find(|t| t.id == tid) {
                                        apply_selected_task(
                                            &mut self.cal,
                                            &mut self.task_title,
                                            &mut self.task_plan,
                                            &mut self.task_doc,
                                            &mut self.task_date,
                                            &mut self.task_start,
                                            &mut self.task_end_date,
                                            &mut self.task_end_hour,
                                            &mut self.task_hours,
                                            &mut self.task_assignee,
                                            &mut self.task_status,
                                            &mut self.task_kind,
                                            &mut self.task_on_calendar,
                                            &mut self.task_remind,
                                            &mut self.task_editing,
                                            t,
                                        );
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
                                    &mut self.task_doc,
                                    &mut self.task_date,
                                    &mut self.task_start,
                                    &mut self.task_end_date,
                                    &mut self.task_end_hour,
                                    &mut self.task_hours,
                                    &mut self.task_assignee,
                                    &mut self.task_status,
                                    &mut self.task_kind,
                                    &mut self.task_on_calendar,
                                    &mut self.task_remind,
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
                                cycle_cfg.as_ref(),
                            ) {
                                if let Some(t) = tasks_all.iter().find(|t| t.id == tid) {
                                    apply_selected_task(
                                        &mut self.cal,
                                        &mut self.task_title,
                                        &mut self.task_plan,
                                        &mut self.task_doc,
                                        &mut self.task_date,
                                        &mut self.task_start,
                                        &mut self.task_end_date,
                                        &mut self.task_end_hour,
                                        &mut self.task_hours,
                                        &mut self.task_assignee,
                                        &mut self.task_status,
                                        &mut self.task_kind,
                                        &mut self.task_on_calendar,
                                        &mut self.task_remind,
                                        &mut self.task_editing,
                                        t,
                                    );
                                    *selected = None;
                                }
                            }
                            return;
                        }
                        if selected.is_none() {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    theme::muted_label(
                                        "在左侧选择一条备忘查看详情\n默认只读 · 删除将移入回收站",
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
                                            "ID  {}    版本 {}    来源 {}    {}    生命周期 {}",
                                            m.id,
                                            m.version,
                                            src,
                                            m.visibility.label(),
                                            m.lifecycle.label()
                                        ))
                                        .small()
                                        .color(theme::TEXT_MUTED),
                                    );
                                }
                                if *editing {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new("可见性")
                                                .small()
                                                .color(theme::TEXT_MUTED),
                                        );
                                        ui.radio_value(
                                            visibility_draft,
                                            MemoVisibility::Private,
                                            "私密",
                                        );
                                        ui.radio_value(
                                            visibility_draft,
                                            MemoVisibility::Public,
                                            "公开",
                                        );
                                        ui.separator();
                                        ui.label(
                                            RichText::new("生命周期")
                                                .small()
                                                .color(theme::TEXT_MUTED),
                                        );
                                        egui::ComboBox::from_id_source("memo_lifecycle")
                                            .selected_text(match *lifecycle_days {
                                                0 => "永久",
                                                30 => "30 天",
                                                90 => "90 天",
                                                365 => "1 年",
                                                _ => "自定义",
                                            })
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(lifecycle_days, 0, "永久");
                                                ui.selectable_value(lifecycle_days, 30, "30 天");
                                                ui.selectable_value(lifecycle_days, 90, "90 天");
                                                ui.selectable_value(lifecycle_days, 365, "1 年");
                                            });
                                    });
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
                                            let (t, b) =
                                                self.memo_doc.to_store("未命名备忘");
                                            *title_draft = t.clone();
                                            *body_draft = b.clone();
                                            let life = if *lifecycle_days <= 0 {
                                                MemoLifecycle::Permanent
                                            } else {
                                                MemoLifecycle::from_days(*lifecycle_days)
                                            };
                                            let vis = *visibility_draft;
                                            let svc = rt.svc.clone();
                                            let id2 = id.clone();
                                            let tx = self.tx.clone();
                                            std::thread::spawn(move || {
                                                match svc.edit_with_lifecycle(
                                                    &id2, &t, &b, vis, life,
                                                ) {
                                                    Ok(()) => {
                                                        let _ =
                                                            tx.send(BgMsg::Info("已保存".into()));
                                                        let _ = tx.send(BgMsg::Refresh);
                                                    }
                                                    Err(e) => {
                                                        let _ =
                                                            tx.send(BgMsg::Error(e.to_string()));
                                                    }
                                                }
                                            });
                                            *editing = false;
                                        }
                                        if theme::ghost_button(ui, "取消编辑").clicked() {
                                            *editing = false;
                                            if let Some(m) = meta {
                                                *title_draft = m.title.clone();
                                                *body_draft = m.content.clone();
                                                *visibility_draft = m.visibility;
                                                *lifecycle_days = match &m.lifecycle {
                                                    MemoLifecycle::Permanent => 0,
                                                    MemoLifecycle::ExpiresAt { .. } => 30,
                                                };
                                                self.memo_doc = doc_editor::Doc::from_store(
                                                    &m.title,
                                                    &m.content,
                                                );
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
                                    if theme::ghost_button(ui, "加入日历").clicked() {
                                        let day = self.cal.selected_day.clone();
                                        let title = if title_draft.trim().is_empty() {
                                            "未命名备忘".to_string()
                                        } else {
                                            title_draft.trim().to_string()
                                        };
                                        let plan = format!("来自备忘\n\n{}", body_draft);
                                        let assignee = rt
                                            .svc
                                            .current_person_id()
                                            .unwrap_or_default();
                                        match rt.svc.add_task(
                                            &title,
                                            &plan,
                                            &day,
                                            9.0,
                                            &day,
                                            10.0,
                                            &assignee,
                                            TaskStatus::NotStarted,
                                            TaskKind::Normal,
                                            true,
                                            false,
                                        ) {
                                            Ok(tid) => {
                                                self.left_tab = LeftTab::Calendar;
                                                if let Some(t) = rt.svc.get_task(&tid) {
                                                    apply_selected_task(
                                                        &mut self.cal,
                                                        &mut self.task_title,
                                                        &mut self.task_plan,
                                                        &mut self.task_doc,
                                                        &mut self.task_date,
                                                        &mut self.task_start,
                                                        &mut self.task_end_date,
                                                        &mut self.task_end_hour,
                                                        &mut self.task_hours,
                                                        &mut self.task_assignee,
                                                        &mut self.task_status,
                                                        &mut self.task_kind,
                                                        &mut self.task_on_calendar,
                                                        &mut self.task_remind,
                                                        &mut self.task_editing,
                                                        &t,
                                                    );
                                                } else {
                                                    self.cal.selected_task = Some(tid.clone());
                                                }
                                                *selected = None;
                                                *editing = false;
                                                *status_line =
                                                    format!("已加入日历任务 {}", &tid[..8.min(tid.len())]);
                                                let _ = self.tx.send(BgMsg::Refresh);
                                            }
                                            Err(e) => {
                                                *status_line = format!("加入日历失败: {e}");
                                            }
                                        }
                                    }
                                });
                                ui.add_space(10.0);
                                ui.separator();
                                ui.add_space(8.0);

                                if *editing {
                                    let (t, b) = self.memo_doc.to_store("未命名备忘");
                                    *title_draft = t;
                                    *body_draft = b;
                                    doc_editor::show_editor(ui, &mut self.memo_doc, "memo");
                                } else {
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .id_source("memo_body_scroll")
                                        .show(ui, |ui| {
                                            ui.set_min_width(ui.available_width());
                                            doc_editor::show_viewer_body(ui, body_draft);
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
                        .default_size([560.0, 560.0])
                        .show(ctx, |ui| {
                            ui.label(
                                RichText::new("第一行是标题，下面写计划")
                                    .small()
                                    .color(theme::TEXT_MUTED),
                            );
                            ui.add_space(4.0);
                            {
                                let (t, p) = self.task_doc.to_store("未命名任务");
                                self.task_title = t;
                                self.task_plan = p;
                            }
                            doc_editor::show_editor(ui, &mut self.task_doc, "new_task");
                            ui.add_space(8.0);
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
                            ui.horizontal(|ui| {
                                ui.label("类型");
                                let prev = self.task_kind;
                                egui::ComboBox::from_id_source("new_task_kind")
                                    .selected_text(self.task_kind.label())
                                    .show_ui(ui, |ui| {
                                        for k in TaskKind::ALL {
                                            ui.selectable_value(
                                                &mut self.task_kind,
                                                k,
                                                k.label(),
                                            );
                                        }
                                    });
                                if self.task_kind != prev {
                                    self.task_on_calendar = true;
                                    self.task_remind = self.task_kind.default_remind();
                                }
                                ui.checkbox(&mut self.task_on_calendar, "加入日历");
                                ui.checkbox(&mut self.task_remind, "提醒我");
                            });
                            ui.horizontal(|ui| {
                                if theme::success_button(ui, "添加").clicked() {
                                    let (t, p) = self.task_doc.to_store("未命名任务");
                                    self.task_title = t;
                                    self.task_plan = p;
                                    match rt.svc.add_task(
                                        &self.task_title,
                                        &self.task_plan,
                                        &self.task_date,
                                        self.task_start,
                                        &self.task_end_date,
                                        self.task_end_hour,
                                        &self.task_assignee,
                                        self.task_status,
                                        self.task_kind,
                                        self.task_on_calendar,
                                        self.task_remind,
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
                                            self.task_doc = doc_editor::Doc::from_store(
                                                &self.task_title,
                                                &self.task_plan,
                                            );
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
                        egui::Window::new("移入回收站")
                            .collapsible(false)
                            .resizable(false)
                            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
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
                                            .fill(theme::DANGER)
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
                    }
                }

                if *show_trash {
                    let mut close_trash = false;
                    egui::Window::new("回收站")
                        .collapsible(false)
                        .resizable(true)
                        .default_size([480.0, 420.0])
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                        .show(ctx, |ui| {
                            ui.label(
                                theme::muted_label(format!(
                                    "已删除备忘保留 {} 天，可恢复或彻底清除（清除后主机备份一并删除）。",
                                    memo_core::TRASH_RETENTION_DAYS
                                )),
                            );
                            ui.add_space(8.0);
                            let trash = rt.svc.list_trash();
                            if trash.is_empty() {
                                ui.label(theme::muted_label("回收站为空"));
                            } else {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_height(300.0)
                                    .show(ui, |ui| {
                                        for m in &trash {
                                            ui.group(|ui| {
                                                ui.horizontal(|ui| {
                                                    ui.vertical(|ui| {
                                                        ui.label(
                                                            RichText::new(if m.title.is_empty() {
                                                                "(无标题)"
                                                            } else {
                                                                m.title.as_str()
                                                            })
                                                            .strong(),
                                                        );
                                                        if !m.deleted_at.is_empty() {
                                                            ui.label(
                                                                theme::muted_label(format!(
                                                                    "删除于 {}",
                                                                    &m.deleted_at
                                                                        [..m.deleted_at.len().min(19)]
                                                                ))
                                                                .small(),
                                                            );
                                                        }
                                                    });
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            if theme::danger_button(ui, "彻底清除")
                                                                .clicked()
                                                            {
                                                                *purge_confirm_id =
                                                                    Some(m.id.clone());
                                                            }
                                                            if theme::ghost_button(ui, "恢复")
                                                                .clicked()
                                                            {
                                                                let svc = rt.svc.clone();
                                                                let id = m.id.clone();
                                                                let tx = self.tx.clone();
                                                                std::thread::spawn(move || {
                                                                    match svc.undelete(&id) {
                                                                        Ok(()) => {
                                                                            let _ = tx.send(
                                                                                BgMsg::Refresh,
                                                                            );
                                                                        }
                                                                        Err(e) => {
                                                                            let _ = tx.send(
                                                                                BgMsg::Error(
                                                                                    e.to_string(),
                                                                                ),
                                                                            );
                                                                        }
                                                                    }
                                                                });
                                                            }
                                                        },
                                                    );
                                                });
                                            });
                                            ui.add_space(4.0);
                                        }
                                    });
                            }
                            ui.add_space(8.0);
                            if ui.button("关闭").clicked() {
                                close_trash = true;
                            }
                        });
                    if close_trash {
                        *show_trash = false;
                        *purge_confirm_id = None;
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
                    egui::Window::new("彻底清除")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                        .show(ctx, |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "彻底清除「{purge_title}」？此操作不可恢复，并将从主机删除备份。"
                                ))
                                .strong()
                                .color(theme::DANGER),
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
                                        .fill(theme::DANGER)
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
                    let screen = ctx.screen_rect();
                    // 半透明遮罩（拦截底层点击）
                    egui::Area::new(egui::Id::new("settings_modal_bg"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(screen.min)
                        .interactable(true)
                        .show(ctx, |ui| {
                            ui.painter().rect_filled(
                                screen,
                                0.0,
                                Color32::from_rgba_unmultiplied(0, 0, 0, 110),
                            );
                            let _ = ui.allocate_response(screen.size(), egui::Sense::click());
                        });

                    let panel_w = (screen.width() - 48.0).clamp(320.0, 520.0);
                    let panel_h = (screen.height() * 0.88).clamp(360.0, 680.0);
                    let panel_pos = egui::pos2(
                        screen.center().x - panel_w * 0.5,
                        screen.center().y - panel_h * 0.5,
                    );

                    egui::Area::new(egui::Id::new("settings_modal"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(panel_pos)
                        .movable(false)
                        .interactable(true)
                        .show(ctx, |ui| {
                            theme::card_frame()
                                .inner_margin(Margin::symmetric(18.0, 14.0))
                                .show(ui, |ui| {
                                    ui.set_width(panel_w - 8.0);
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new("设置")
                                                .strong()
                                                .size(17.0)
                                                .color(theme::TEXT),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new("关闭")
                                                                .small()
                                                                .color(theme::TEXT_MUTED),
                                                        )
                                                        .frame(false),
                                                    )
                                                    .clicked()
                                                {
                                                    *show_settings = false;
                                                }
                                            },
                                        );
                                    });
                                    ui.label(
                                        RichText::new(
                                            "设置保存在系统用户配置目录。更改节点/端口/数据目录/盐后需重启。",
                                        )
                                        .small()
                                        .color(theme::TEXT_MUTED),
                                    );
                                    ui.add_space(6.0);
                                    ui.separator();
                                    ui.add_space(4.0);

                                    let scroll_h = (panel_h - 120.0).max(200.0);
                                    egui::ScrollArea::vertical()
                                        .id_source("settings_scroll")
                                        .max_height(scroll_h)
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.label("节点 ID");
                                            ui.text_edit_singleline(
                                                &mut settings_draft.node_id,
                                            );
                                            ui.label("本机显示名（可选，仅本地展示）");
                                            ui.text_edit_singleline(
                                                &mut settings_draft.node_display_name,
                                            );
                                            ui.label("数据目录");
                                            ui.text_edit_singleline(
                                                &mut settings_draft.data_dir,
                                            );
                                            ui.label("监听端口");
                                            let mut port =
                                                settings_draft.listen_port.to_string();
                                            if ui.text_edit_singleline(&mut port).changed() {
                                                if let Ok(p) = port.parse() {
                                                    settings_draft.listen_port = p;
                                                }
                                            }
                                            ui.label(
                                                "对端列表（每行 host:port，可选；同网段可依赖自动发现）",
                                            );
                                            let mut peers_text =
                                                settings_draft.peers.join("\n");
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
                                            ui.checkbox(
                                                &mut settings_draft.lan_discovery,
                                                "局域网自动发现 (UDP 17000)",
                                            );
                                            ui.label(
                                                RichText::new("节点角色（更改后需重启）")
                                                    .small()
                                                    .color(theme::TEXT_MUTED),
                                            );
                                            ui.horizontal(|ui| {
                                                ui.radio_value(
                                                    &mut settings_draft.node_role,
                                                    NodeRole::Master,
                                                    "主机",
                                                );
                                                ui.radio_value(
                                                    &mut settings_draft.node_role,
                                                    NodeRole::Slave,
                                                    "从机",
                                                );
                                            });
                                            ui.label(
                                                RichText::new(
                                                    "主机：每 10s 广播并受理从机登记，也可向其他主机远程备份私有/公开数据。\n\
                                                     从机：不广播，收到主机广播后主动连接登记。",
                                                )
                                                .small()
                                                .color(theme::TEXT_MUTED),
                                            );
                                            ui.add_enabled_ui(
                                                settings_draft.node_role.is_master(),
                                                |ui| {
                                                    ui.checkbox(
                                                        &mut settings_draft.node_visible,
                                                        "主机对外广播（关闭则本机不发 UDP）",
                                                    );
                                                },
                                            );
                                            ui.checkbox(
                                                &mut settings_draft.accept_foreign_backup,
                                                "接受他节点私人密文托管（全局；逐节点还可在节点面板关闭）",
                                            );
                                            ui.checkbox(
                                                &mut settings_draft.backup_enabled,
                                                "推送本身份私人备份到网络",
                                            );
                                            ui.checkbox(
                                                &mut settings_draft.show_backup_status,
                                                "左侧列表显示备份状态标签",
                                            );
                                            ui.label(
                                                RichText::new(
                                                    "集群盐 cluster_salt_hex（同局域网互通须一致）",
                                                )
                                                .small()
                                                .color(theme::TEXT_MUTED),
                                            );
                                            ui.text_edit_singleline(
                                                &mut settings_draft.cluster_salt_hex,
                                            );
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
                                                    let dir =
                                                        std::path::PathBuf::from(&rt.cfg.data_dir)
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

                                    ui.separator();
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        if theme::success_button(ui, "保存").clicked() {
                                            let before = self.cfg.clone();
                                            match config::save_settings(settings_draft) {
                                                Ok(()) => {
                                                    // UI 开关立即生效
                                                    self.cfg.show_backup_status =
                                                        settings_draft.show_backup_status;
                                                    self.cfg.backup_enabled =
                                                        settings_draft.backup_enabled;
                                                    self.cfg.accept_foreign_backup =
                                                        settings_draft.accept_foreign_backup;
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
                                    });
                                });
                        });
                }
            }
        }

        Self::draw_help_about_windows(ctx, &mut show_help, &mut show_about);
        self.show_help = show_help;
        self.show_about = show_about;
    }
}

fn apply_selected_task(
    cal: &mut CalUi,
    task_title: &mut String,
    task_plan: &mut String,
    task_doc: &mut doc_editor::Doc,
    task_date: &mut String,
    task_start: &mut f32,
    task_end_date: &mut String,
    task_end_hour: &mut f32,
    task_hours: &mut f32,
    task_assignee: &mut String,
    task_status: &mut TaskStatus,
    task_kind: &mut TaskKind,
    task_on_calendar: &mut bool,
    task_remind: &mut bool,
    task_editing: &mut bool,
    t: &memo_core::TaskView,
) {
    cal.selected_task = Some(t.id.clone());
    *task_title = t.title.clone();
    *task_plan = t.plan.clone();
    *task_doc = doc_editor::Doc::from_store(&t.title, &t.plan);
    *task_date = t.date.clone();
    *task_start = t.start_hour;
    *task_end_date = t.end_date.clone();
    *task_end_hour = t.end_hour;
    *task_hours = t.hours;
    *task_assignee = t.assignee_id.clone();
    *task_status = t.status;
    *task_kind = t.kind;
    *task_on_calendar = t.on_calendar;
    *task_remind = t.remind;
    *task_editing = false;
}

#[allow(clippy::too_many_arguments)]
fn draw_task_central(
    ui: &mut egui::Ui,
    rt: &AppRuntime,
    persons: &[memo_core::PersonView],
    cal: &mut CalUi,
    task_title: &mut String,
    task_plan: &mut String,
    task_doc: &mut doc_editor::Doc,
    task_date: &mut String,
    task_start: &mut f32,
    task_end_date: &mut String,
    task_end_hour: &mut f32,
    task_hours: &mut f32,
    task_assignee: &mut String,
    task_status: &mut TaskStatus,
    task_kind: &mut TaskKind,
    task_on_calendar: &mut bool,
    task_remind: &mut bool,
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
                    let (t, p) = task_doc.to_store("未命名任务");
                    *task_title = t;
                    *task_plan = p;
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
                        *task_kind,
                        *task_on_calendar,
                        *task_remind,
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
                        *task_doc = doc_editor::Doc::from_store(&t.title, &t.plan);
                        *task_date = t.date.clone();
                        *task_start = t.start_hour;
                        *task_end_date = t.end_date.clone();
                        *task_end_hour = t.end_hour;
                        *task_hours = t.hours;
                        *task_assignee = t.assignee_id.clone();
                        *task_status = t.status;
                        *task_kind = t.kind;
                        *task_on_calendar = t.on_calendar;
                        *task_remind = t.remind;
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
            {
                let (t, p) = task_doc.to_store("未命名任务");
                *task_title = t;
                *task_plan = p;
            }
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
                ui.vertical(|ui| {
                    ui.label(RichText::new("类型").small().color(theme::TEXT_MUTED));
                    let prev = *task_kind;
                    egui::ComboBox::from_id_source("task_kind")
                        .selected_text(task_kind.label())
                        .show_ui(ui, |ui| {
                            for k in TaskKind::ALL {
                                ui.selectable_value(task_kind, k, k.label());
                            }
                        });
                    if *task_kind != prev {
                        *task_on_calendar = true;
                        *task_remind = task_kind.default_remind();
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.checkbox(task_on_calendar, "加入日历");
                ui.checkbox(task_remind, "提醒我");
            });
            ui.add_space(8.0);
            ui.label(RichText::new("工作计划").strong().size(14.0));
            ui.add_space(4.0);
            doc_editor::show_editor(ui, task_doc, "task");
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
                ui.add_space(12.0);
                ui.label(RichText::new("类型").small().color(theme::TEXT_MUTED));
                ui.label(
                    RichText::new(task_kind.label())
                        .size(14.0)
                        .color(calendar_view::kind_color(*task_kind)),
                );
                ui.add_space(12.0);
                ui.label(
                    RichText::new(if *task_on_calendar {
                        "已入日历"
                    } else {
                        "未入日历"
                    })
                    .size(13.0)
                    .color(theme::TEXT_MUTED),
                );
                if *task_remind {
                    ui.label(
                        RichText::new("提醒")
                            .size(13.0)
                            .color(theme::WARN),
                    );
                }
            });
            ui.add_space(12.0);
            ui.label(RichText::new("工作计划").strong().size(14.0));
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_source("task_plan_scroll")
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    doc_editor::show_viewer_body(ui, task_plan);
                });
        }
    });
}

fn gender_chip(ui: &mut egui::Ui, g: Gender) {
    let (label, color) = match g {
        Gender::Male => ("男", theme::ACCENT),
        Gender::Female => ("女", Color32::from_rgb(0xDB, 0x27, 0x77)),
        Gender::Unknown => return,
    };
    egui::Frame::none()
        .fill(theme::PANEL)
        .rounding(Rounding::same(4.0))
        .inner_margin(Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).small().strong().color(color));
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
            svc.task_store(),
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
    if single_instance::try_acquire(std::path::Path::new(&cfg.data_dir))
        == single_instance::AcquireResult::AlreadyRunning
    {
        // 已激活既有窗口；本进程安静退出
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
        Box::new(|cc| {
            fonts::configure_cjk_fonts(&cc.egui_ctx);
            Box::new(MemoApp::new(cfg))
        }),
    )
}
