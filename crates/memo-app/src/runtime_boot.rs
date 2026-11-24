//! 桌面解锁与 GUI 入口（调用 memo-sync 共用节点启动）。

use crate::app_icon;
use crate::commercial;
use crate::fonts;
use crate::runtime::AppRuntime;
use crate::single_instance;
use crate::MemoApp;
use eframe::egui;
use memo_core::config::Config;
use memo_core::identity_keys::IdentityKeys;
use memo_sync::{start_node, NodeStartOpts};
use std::sync::Arc;

pub fn unlock_runtime_identity(
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
        let ok = commercial::commercial_features_ok();
        let mut opts = NodeStartOpts::desktop_user();
        opts.lan_discovery = Some(cfg.lan_discovery && ok);
        opts.node_visible = Some(cfg.node_visible && ok);
        opts.accept_foreign_backup = Some(cfg.accept_foreign_backup && ok);
        let started = start_node(cfg.clone(), &identity, local_password, opts)?;
        (started.svc, started.engine)
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
            .with_inner_size([960.0, 720.0])
            .with_min_inner_size([960.0, 720.0])
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
