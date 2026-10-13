//! 系统托盘：关主窗隐藏到托盘；菜单「显示主窗口 / 退出」。
//!
//! 重要：Windows 上 **不要** 用 `ViewportCommand::Visible(false)`——eframe/winit
//! 隐藏后不再派发重绘（见 egui#5229）。改为 Win32 `ShowWindow(SW_HIDE/SW_RESTORE)`。
//!
//! - 最小化：系统默认（本模块不拦截）
//! - 关窗 ×：`hide_main_window` 进托盘
//! - 托盘退出：**不重显主窗**，置位后无闪唤醒，由 `update` dispose + `process::exit`

use crate::app_icon;
use eframe::egui;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::OnceLock;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

static SHOW_MAIN: AtomicBool = AtomicBool::new(false);
static QUIT_APP: AtomicBool = AtomicBool::new(false);
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static UI_THREAD_ID: AtomicU32 = AtomicU32::new(0);

static MENU_SHOW_ID: OnceLock<String> = OnceLock::new();
static MENU_QUIT_ID: OnceLock<String> = OnceLock::new();
static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();
static HANDLERS_INSTALLED: AtomicBool = AtomicBool::new(false);

pub struct TrayHandle {
    tray: TrayIcon,
}

impl TrayHandle {
    /// 卸载托盘图标（须在 UI 线程、且不在 TrackPopupMenu 嵌套泵内调用）。
    pub fn dispose(self) {
        let _ = self.tray.set_visible(false);
        drop(self.tray);
    }
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
    Some(TrayHandle { tray })
}

pub fn bind_context(ctx: &egui::Context) {
    let _ = EGUI_CTX.set(ctx.clone());
}

pub fn capture_main_hwnd(frame: &eframe::Frame) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = frame.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                let hwnd = h.hwnd.get() as isize;
                if hwnd != 0 {
                    MAIN_HWND.store(hwnd, Ordering::SeqCst);
                    unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
                        let tid = GetWindowThreadProcessId(hwnd as _, std::ptr::null_mut());
                        if tid != 0 {
                            UI_THREAD_ID.store(tid, Ordering::SeqCst);
                        }
                    }
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

/// 藏到托盘（仅关窗 / 开机 `--tray`；不要在用户点最小化时调用）。
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

/// 从托盘显示主窗。
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

fn wake_ui() {
    if let Some(ctx) = EGUI_CTX.get() {
        ctx.request_repaint();
    }
}

/// 退出唤醒：不 SW_SHOW/RESTORE（仿微信，不重显主窗）。
/// iconic 时用 SetWindowPlacement(SW_HIDE) 清挂起，便于 eframe 跑到 update。
fn wake_for_quit() {
    #[cfg(windows)]
    {
        let hwnd = MAIN_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    GetWindowPlacement, IsIconic, PostMessageW, SetWindowPlacement, ShowWindow,
                    SW_HIDE, WINDOWPLACEMENT, WM_NULL,
                };
                if IsIconic(hwnd as _) != 0 {
                    let mut place = std::mem::zeroed::<WINDOWPLACEMENT>();
                    place.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
                    if GetWindowPlacement(hwnd as _, &mut place) != 0 {
                        place.showCmd = SW_HIDE as u32;
                        SetWindowPlacement(hwnd as _, &place);
                    } else {
                        ShowWindow(hwnd as _, SW_HIDE);
                    }
                }
                PostMessageW(hwnd as _, WM_NULL as _, 0, 0);
            }
        }
        let tid = UI_THREAD_ID.load(Ordering::SeqCst);
        if tid != 0 {
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_NULL};
                PostThreadMessageW(tid, WM_NULL as _, 0, 0);
            }
        }
    }
    wake_ui();
    if let Some(ctx) = EGUI_CTX.get() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn request_show_main() {
    if QUIT_APP.load(Ordering::SeqCst) {
        wake_for_quit();
        return;
    }
    SHOW_MAIN.store(true, Ordering::SeqCst);
    show_main_window();
}

/// tray-icon Windows：隐藏托盘图标的自定义消息（platform_impl WM_USER_HIDE_TRAYICON）。
#[cfg(windows)]
const WM_USER_HIDE_TRAYICON: u32 = 6006;

/// 不依赖 eframe `update` 的强制退出（仿微信：不重显主窗）。
/// 先尽力唤醒主循环落盘；超时后 SendMessage 让托盘窗 NIM_DELETE，再 exit。
fn force_quit_process() {
    QUIT_APP.store(true, Ordering::SeqCst);
    wake_for_quit();
    // 给 update 快路径约 300ms 落盘；若已 exit，本线程随之消失
    std::thread::sleep(std::time::Duration::from_millis(300));

    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW};
        let cls = to_wide("tray_icon_app");
        let hwnd = FindWindowW(cls.as_ptr(), std::ptr::null());
        if hwnd != 0 {
            // 同步切到 UI 线程执行 NIM_DELETE，不依赖 RedrawRequested
            SendMessageW(hwnd, WM_USER_HIDE_TRAYICON, 0, 0);
        }
    }

    std::process::exit(0);
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
            // 避开 TrackPopupMenu 嵌套泵；延迟后强制退出（不 SW_SHOW）
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(50));
                force_quit_process();
            });
        } else {
            wake_ui();
        }
    }));
}

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

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

#[cfg(windows)]
unsafe fn animate_window_to_tray(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::Graphics::Gdi::DrawAnimatedRects;
    let Some(from) = window_normal_rect(hwnd) else {
        return;
    };
    let Some(to) = tray_notify_rect() else {
        return;
    };
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
