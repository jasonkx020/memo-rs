//! Windows：桌面便签对抗「显示桌面」。
//!
//! 「显示桌面」会最小化普通窗并抬起桌面壳；未置顶窗即便 Show 出来也会被壳盖住。
//! 仅在前台为桌面壳、或本窗被最小化时临时 TOPMOST 拉回；状态不变时不 SetWindowPos，
//! 避免每帧抬层导致主窗口点击无响应。

/// 前台是否为桌面壳（用户点了显示桌面 / Win+D）。
#[cfg(windows)]
pub fn should_float_over_desktop() -> bool {
    unsafe { foreground_is_desktop_shell() }
}

#[cfg(not(windows))]
pub fn should_float_over_desktop() -> bool {
    false
}

/// 确保便签在「显示桌面」后仍可见。
/// `float_over_desktop`：用户置顶，或显示桌面期间的临时置顶。
/// `last_float`：缓存上次 Win32 浮起状态，未变化则跳过 SetWindowPos。
#[cfg(windows)]
pub fn ensure_visible(
    hwnd_cache: &mut isize,
    window_title: &str,
    float_over_desktop: bool,
    last_float: &mut Option<bool>,
) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, IsIconic, IsWindow, IsWindowVisible, SetWindowPlacement, SetWindowPos,
        ShowWindow, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WINDOWPLACEMENT,
    };

    unsafe {
        let mut hwnd = *hwnd_cache as HWND;
        if hwnd == 0 || IsWindow(hwnd) == 0 {
            hwnd = find_sticky_hwnd(window_title);
            *hwnd_cache = hwnd as isize;
        }
        if hwnd == 0 {
            return;
        }

        let iconic = IsIconic(hwnd) != 0;
        let hidden = iconic || IsWindowVisible(hwnd) == 0;

        if hidden {
            let mut place = std::mem::zeroed::<WINDOWPLACEMENT>();
            place.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
            if GetWindowPlacement(hwnd, &mut place) != 0 {
                place.showCmd = SW_SHOWNOACTIVATE as u32;
                let _ = SetWindowPlacement(hwnd, &place);
            }
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if IsIconic(hwnd) != 0 {
                let mut place = std::mem::zeroed::<WINDOWPLACEMENT>();
                place.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
                if GetWindowPlacement(hwnd, &mut place) != 0 {
                    place.showCmd = SW_SHOWNOACTIVATE as u32;
                    let _ = SetWindowPlacement(hwnd, &place);
                }
            }
            // 刚从最小化拉回：强制下一拍更新层级
            *last_float = None;
        }

        // 被最小化时也必须临时 TOPMOST，否则拉回后仍在桌面壳下面
        let want_topmost = float_over_desktop || iconic || hidden;
        if *last_float == Some(want_topmost) {
            return;
        }

        let insert = if want_topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        let flags = if hidden || want_topmost {
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW
        } else {
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE
        };
        let _ = SetWindowPos(hwnd, insert, 0, 0, 0, 0, flags);
        *last_float = Some(want_topmost);
    }
}

#[cfg(not(windows))]
pub fn ensure_visible(
    _hwnd_cache: &mut isize,
    _window_title: &str,
    _float_over_desktop: bool,
    _last_float: &mut Option<bool>,
) {
}

#[cfg(windows)]
unsafe fn foreground_is_desktop_shell() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let fg = GetForegroundWindow();
    if fg == 0 {
        return false;
    }
    is_desktop_shell_hwnd(fg)
}

#[cfg(windows)]
unsafe fn is_desktop_shell_hwnd(hwnd: windows_sys::Win32::Foundation::HWND) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW;
    let mut buf = [0u16; 64];
    let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return false;
    }
    let class = String::from_utf16_lossy(&buf[..n as usize]);
    matches!(
        class.as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

#[cfg(windows)]
struct EnumSticky {
    self_pid: u32,
    title: Vec<u16>,
    hwnd: windows_sys::Win32::Foundation::HWND,
}

#[cfg(windows)]
unsafe extern "system" fn enum_sticky_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::BOOL {
    use windows_sys::Win32::Foundation::{FALSE, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowTextW, GetWindowThreadProcessId, GW_OWNER,
    };

    let data = &mut *(lparam as *mut EnumSticky);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid != data.self_pid {
        return TRUE;
    }
    if GetWindow(hwnd, GW_OWNER) != 0 {
        return TRUE;
    }
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return TRUE;
    }
    let want = data.title.as_slice();
    let want_len = want.iter().position(|&c| c == 0).unwrap_or(want.len());
    if n as usize == want_len && buf[..n as usize] == want[..want_len] {
        data.hwnd = hwnd;
        return FALSE;
    }
    TRUE
}

#[cfg(windows)]
fn find_sticky_hwnd(window_title: &str) -> windows_sys::Win32::Foundation::HWND {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowW};

    let title: Vec<u16> = format!("{window_title}\0").encode_utf16().collect();
    let by_title = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if by_title != 0 {
        return by_title;
    }

    let mut data = EnumSticky {
        self_pid: unsafe { GetCurrentProcessId() },
        title,
        hwnd: 0,
    };
    unsafe {
        EnumWindows(
            Some(enum_sticky_proc),
            &mut data as *mut _ as windows_sys::Win32::Foundation::LPARAM,
        );
    }
    data.hwnd
}
