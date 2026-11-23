// 带 ui 特性时：Windows 默认 GUI 子系统；--no-ui / --help 再 AllocConsole。
// 无 ui（服务器包）：始终控制台子系统。
#![cfg_attr(all(windows, feature = "ui"), windows_subsystem = "windows")]

mod args;
mod daemon;
mod identity_unlock;

use args::Args;
use clap::Parser;
use daemon::run_no_ui;
use memo_core::config;

#[cfg(all(windows, feature = "ui"))]
fn ensure_console_if_needed() {
    let need = std::env::args().any(|a| {
        matches!(
            a.as_str(),
            "--headless"
                | "--no-ui"
                | "--repl"
                | "--init"
                | "--help"
                | "-h"
                | "--version"
                | "-V"
        )
    }) || std::env::var_os("MEMO_NO_UI").is_some();
    if need {
        unsafe {
            windows_sys::Win32::System::Console::AllocConsole();
        }
    }
}

#[cfg(not(all(windows, feature = "ui")))]
fn ensure_console_if_needed() {}

#[cfg(feature = "ui")]
fn auto_no_ui() -> bool {
    if std::env::var_os("MEMO_NO_UI").is_some() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        let display = std::env::var_os("DISPLAY").filter(|v| !v.is_empty());
        let wayland = std::env::var_os("WAYLAND_DISPLAY").filter(|v| !v.is_empty());
        return display.is_none() && wayland.is_none();
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(feature = "ui")]
fn want_no_ui(args: &Args) -> bool {
    if args.gui {
        return false;
    }
    args.headless || args.no_ui || auto_no_ui()
}

fn main() -> anyhow::Result<()> {
    ensure_console_if_needed();
    let args = Args::parse();
    let cfg = config::load_settings()?;

    #[cfg(not(feature = "ui"))]
    {
        if args.gui || args.tray {
            anyhow::bail!(
                "本二进制未编译 UI（--no-default-features）。请使用桌面包，或去掉 --gui/--tray 以 no-ui 运行"
            );
        }
        return run_no_ui(cfg, args);
    }

    #[cfg(feature = "ui")]
    {
        if want_no_ui(&args) {
            run_no_ui(cfg, args)
        } else {
            memo_app::run_gui_with_opts(cfg, args.tray).map_err(|e| anyhow::anyhow!("{e}"))
        }
    }
}
