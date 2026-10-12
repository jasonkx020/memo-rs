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

/// 藏到托盘：先播放「缩入通知区」动画，再 Win32 隐藏。
/// **不**发 ViewportCommand::Visible(false)（见 egui#5229）。
///
/// 若窗口已最小化（IsIconic），必须先 Restore 再 Hide，否则会停在
/// iconic+hidden，托盘「退出」投递的 WM_CLOSE 无法及时进入 update。
pub fn hide_main_window() {
    #[cfg(windows)]
    {
        let hwnd = MAIN_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    IsIconic, ShowWindow, SW_HIDE, SW_RESTORE,
                };
                if IsIconic(hwnd as _) != 0 {
                    ShowWindow(hwnd as _, SW_RESTORE);
                }
                animate_window_to_tray(hwnd as _);
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
                animate_window_from_tray(hwnd as _);
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

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 任务栏通知区域（托盘）附近的小矩形，作为缩小动画终点。
#[cfg(windows)]
unsafe fn tray_notify_rect() -> Option<windows_sys::Win32::Foundation::RECT> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect};
    let tray_cls = to_wide("Shell_TrayWnd");
    let notify_cls = to_wide("TrayNotifyWnd");
    let tray = FindWindowW(tray_cls.as_ptr(), std::ptr::null());
    if tray == 0 {
        return None;
    }
    let notify = FindWindowExW(tray, 0, notify_cls.as_ptr(), std::ptr::null());
    let target = if notify != 0 { notify } else { tray };
    let mut rc = std::mem::zeroed::<RECT>();
    if GetWindowRect(target, &mut rc) == 0 {
        return None;
    }
    // 收到通知区中部一小块，视觉上更像「吸进托盘」
    let cx = (rc.left + rc.right) / 2;
    let cy = (rc.top + rc.bottom) / 2;
    Some(RECT {
        left: cx - 10,
        top: cy - 10,
        right: cx + 10,
        bottom: cy + 10,
    })
}

#[cfg(windows)]
unsafe fn window_normal_rect(
    hwnd: windows_sys::Win32::Foundation::HWND,
) -> Option<windows_sys::Win32::Foundation::RECT> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, GetWindowRect, WINDOWPLACEMENT,
    };
    let mut place = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..std::mem::zeroed()
    };
    if GetWindowPlacement(hwnd, &mut place) != 0 {
        let r = place.rcNormalPosition;
        if r.right > r.left && r.bottom > r.top {
            return Some(r);
        }
    }
    let mut rc = std::mem::zeroed::<RECT>();
    if GetWindowRect(hwnd, &mut rc) != 0 {
        Some(rc)
    } else {
        None
    }
}

/// 经典标题栏缩小动画：窗口矩形 → 托盘通知区（DrawAnimatedRects）。
#[cfg(windows)]
unsafe fn animate_window_to_tray(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::Graphics::Gdi::DrawAnimatedRects;
    let Some(from) = window_normal_rect(hwnd) else {
        return;
    };
    let Some(to) = tray_notify_rect() else {
        return;
    };
    // IDANI_CAPTION = 3：标题栏样式的缩小动画
    DrawAnimatedRects(hwnd, 3, &from, &to);
}

#[cfg(windows)]
unsafe fn animate_window_from_tray(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::Graphics::Gdi::DrawAnimatedRects;
    let Some(to) = window_normal_rect(hwnd) else {
        return;
    };
    let Some(from) = tray_notify_rect() else {
        return;
    };
    DrawAnimatedRects(hwnd, 3, &from, &to);
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
            // 必须先置位：随后 WM_CLOSE 进入 update 时不能被「关窗→藏托盘」吃掉
            QUIT_APP.store(true, Ordering::SeqCst);
            request_quit_wake();
        } else {
            wake_ui();
        }
    }));
}

/// 主窗在前台空闲 / 最小化 / 已藏托盘时事件循环可能停转，仅 `request_repaint` 不够；
/// Win32 投递消息才能立刻跑到 `update` 处理退出。
fn request_quit_wake() {
    #[cfg(windows)]
    {
        let hwnd = MAIN_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    PostMessageW, ShowWindow, SW_RESTORE, SW_SHOW, WM_CLOSE, WM_NULL,
                };
                // 覆盖：系统最小化（iconic）与关窗藏托盘（SW_HIDE）
                ShowWindow(hwnd as _, SW_RESTORE);
                ShowWindow(hwnd as _, SW_SHOW);
                PostMessageW(hwnd as _, WM_NULL as _, 0, 0);
                PostMessageW(hwnd as _, WM_CLOSE as _, 0, 0);
            }
        }
    }
    wake_ui();
    if let Some(ctx) = EGUI_CTX.get() {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
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
