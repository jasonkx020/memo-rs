//! 系统托盘：关主窗隐藏到托盘；菜单「显示主窗口 / 退出」。
//!
//! 重要：Windows 上 **不要** 用 `ViewportCommand::Visible(false)`——eframe/winit
//! 隐藏后不再派发重绘，导致 `Visible(true)` / `request_repaint` 永久失效
//!（见 egui#5229 / #3655）。改为 Win32 `ShowWindow(SW_HIDE/SW_RESTORE)`，
//! 让 winit 仍认为窗口“可见”，事件循环与便签 deferred 视口才能继续跑。

use crate::app_icon;
use eframe::egui;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::OnceLock;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

static SHOW_MAIN: AtomicBool = AtomicBool::new(false);
static QUIT_APP: AtomicBool = AtomicBool::new(false);
/// 主窗 HWND（Windows）；0 表示尚未捕获。
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

static MENU_SHOW_ID: OnceLock<String> = OnceLock::new();
static MENU_QUIT_ID: OnceLock<String> = OnceLock::new();
static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();
static HANDLERS_INSTALLED: AtomicBool = AtomicBool::new(false);

pub struct TrayHandle {
    _tray: TrayIcon,
}

pub fn create() -> Option<TrayHandle> {
    install_event_handlers();
    let icon = load_tray_icon()?;
    let menu = Menu::new();
    let show = MenuItem::new("显示主窗口", true, None);
    let quit = MenuItem::new("退出", true, None);
    let _ = MENU_SHOW_ID.set(show.id().0.clone());
    let _ = MENU_QUIT_ID.set(quit.id().0.clone());
    let _ = menu.append(&show);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&quit);
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("分布式备忘录")
        .with_icon(icon)
        .build()
        .ok()?;
    Some(TrayHandle { _tray: tray })
}

/// 绑定 egui Context，使托盘线程能 `request_repaint` 唤醒主循环。
pub fn bind_context(ctx: &egui::Context) {
    let _ = EGUI_CTX.set(ctx.clone());
}

/// 从 eframe Frame 捕获主窗 HWND（每帧可调用，幂等）。
pub fn capture_main_hwnd(frame: &eframe::Frame) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = frame.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                let hwnd = h.hwnd.get() as isize;
                if hwnd != 0 {
                    MAIN_HWND.store(hwnd, Ordering::SeqCst);
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = frame;
    }
}

pub fn has_main_hwnd() -> bool {
    MAIN_HWND.load(Ordering::SeqCst) != 0
}

/// 藏到托盘：Win32 隐藏，**不**发 ViewportCommand::Visible(false)。
pub fn hide_main_window() {
    #[cfg(windows)]
    {
        let hwnd = MAIN_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                ShowWindow(hwnd as _, SW_HIDE);
            }
        }
    }
    #[cfg(not(windows))]
    if let Some(ctx) = EGUI_CTX.get() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }
}

/// 从托盘恢复主窗（可在托盘回调线程直接调用；不置位 poll 标志）。
pub fn show_main_window() {
    #[cfg(windows)]
    {
        let hwnd = MAIN_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    BringWindowToTop, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
                };
                ShowWindow(hwnd as _, SW_SHOW);
                ShowWindow(hwnd as _, SW_RESTORE);
                BringWindowToTop(hwnd as _);
                SetForegroundWindow(hwnd as _);
            }
        }
    }
    #[cfg(not(windows))]
    if let Some(ctx) = EGUI_CTX.get() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
    wake_ui();
}

fn request_show_main() {
    SHOW_MAIN.store(true, Ordering::SeqCst);
    show_main_window();
}

fn wake_ui() {
    if let Some(ctx) = EGUI_CTX.get() {
        ctx.request_repaint();
    }
}

fn install_event_handlers() {
    if HANDLERS_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    TrayIconEvent::set_event_handler(Some(|ev| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = ev
        {
            request_show_main();
        } else {
            wake_ui();
        }
    }));
    MenuEvent::set_event_handler(Some(|ev: MenuEvent| {
        let id = ev.id.0.as_str();
        if MENU_SHOW_ID.get().map(|s| s.as_str()) == Some(id) {
            request_show_main();
        } else if MENU_QUIT_ID.get().map(|s| s.as_str()) == Some(id) {
            QUIT_APP.store(true, Ordering::SeqCst);
            // 退出前先露出主窗，便于 eframe 处理 Close
            #[cfg(windows)]
            {
                let hwnd = MAIN_HWND.load(Ordering::SeqCst);
                if hwnd != 0 {
                    unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOW};
                        ShowWindow(hwnd as _, SW_SHOW);
                    }
                }
            }
            wake_ui();
        } else {
            wake_ui();
        }
    }));
}

fn load_tray_icon() -> Option<Icon> {
    let img = image::load_from_memory(app_icon::APP_ICON_PNG)
        .ok()?
        .into_rgba8();
    let (w, h) = img.dimensions();
    let rgba = if w > 32 || h > 32 {
        let resized = image::imageops::resize(&img, 32, 32, image::imageops::FilterType::Triangle);
        Icon::from_rgba(resized.into_raw(), 32, 32).ok()
    } else {
        Icon::from_rgba(img.into_raw(), w, h).ok()
    };
    rgba
}

/// 每帧读取托盘动作标志。
pub fn poll() -> (bool, bool) {
    let show = SHOW_MAIN.swap(false, Ordering::SeqCst);
    let quit = QUIT_APP.swap(false, Ordering::SeqCst);
    (show, quit)
}
