//! 保证 GUI 单实例：再次启动时激活已有窗口并退出。

use fs2::FileExt;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

const WINDOW_TITLE: &str = "分布式备忘录";
const LOCK_NAME: &str = "gui.single.lock";

/// 持有锁，进程退出时自动释放。
struct SingleInstanceGuard {
    _file: std::fs::File,
}

static GUARD: Mutex<Option<SingleInstanceGuard>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquireResult {
    Acquired,
    AlreadyRunning,
}

/// 在 `data_dir` 下用独占文件锁保证 GUI 单实例。
/// 若已有实例：将其窗口恢复并置前，然后返回 `AlreadyRunning`。
pub fn try_acquire(data_dir: &Path) -> AcquireResult {
    let Ok(mut slot) = GUARD.lock() else {
        return AcquireResult::AlreadyRunning;
    };
    if slot.is_some() {
        return AcquireResult::Acquired;
    }
    match acquire_inner(data_dir) {
        Ok(guard) => {
            *slot = Some(guard);
            AcquireResult::Acquired
        }
        Err(()) => {
            activate_existing(data_dir);
            AcquireResult::AlreadyRunning
        }
    }
}

fn lock_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(LOCK_NAME)
}

fn acquire_inner(data_dir: &Path) -> Result<SingleInstanceGuard, ()> {
    let _ = std::fs::create_dir_all(data_dir);
    let path = lock_path(data_dir);
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|_| ())?;
    file.try_lock_exclusive().map_err(|_| ())?;
    file.set_len(0).map_err(|_| ())?;
    file.seek(SeekFrom::Start(0)).map_err(|_| ())?;
    write!(file, "{}", std::process::id()).map_err(|_| ())?;
    let _ = file.flush();
    Ok(SingleInstanceGuard { _file: file })
}

fn read_owner_pid(data_dir: &Path) -> Option<u32> {
    let mut file = OpenOptions::new().read(true).open(lock_path(data_dir)).ok()?;
    let mut s = String::new();
    file.read_to_string(&mut s).ok()?;
    s.trim().parse().ok()
}

#[cfg(windows)]
fn activate_existing(data_dir: &Path) {
    let pid = read_owner_pid(data_dir);
    if let Some(hwnd) = find_window(pid) {
        bring_to_foreground(hwnd);
    }
}

#[cfg(windows)]
fn find_window(owner_pid: Option<u32>) -> Option<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

    let title: Vec<u16> = format!("{WINDOW_TITLE}\0").encode_utf16().collect();
    let by_title = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if by_title != 0 {
        if let Some(pid) = owner_pid {
            if window_pid(by_title) == Some(pid) {
                return Some(by_title);
            }
            // 标题匹配但 PID 不符时仍优先标题（用户可见主窗）
            return Some(by_title);
        }
        return Some(by_title);
    }
    owner_pid.and_then(find_main_window_by_pid)
}

#[cfg(windows)]
fn window_pid(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<u32> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if tid == 0 || pid == 0 {
        None
    } else {
        Some(pid)
    }
}

#[cfg(windows)]
struct EnumFind {
    target_pid: u32,
    hwnd: windows_sys::Win32::Foundation::HWND,
}

#[cfg(windows)]
unsafe extern "system" fn enum_windows_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::BOOL {
    use windows_sys::Win32::Foundation::{FALSE, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowTextW, GetWindowThreadProcessId, GW_OWNER, IsWindowVisible,
    };

    let data = &mut *(lparam as *mut EnumFind);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid != data.target_pid {
        return TRUE;
    }
    // 只要顶层窗口（无 owner）
    if GetWindow(hwnd, GW_OWNER) != 0 {
        return TRUE;
    }
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        // 无标题：若可见仍可作候选
        if IsWindowVisible(hwnd) == FALSE {
            return TRUE;
        }
    }
    data.hwnd = hwnd;
    FALSE
}

#[cfg(windows)]
fn find_main_window_by_pid(pid: u32) -> Option<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows;

    let mut data = EnumFind {
        target_pid: pid,
        hwnd: 0,
    };
    unsafe {
        EnumWindows(
            Some(enum_windows_proc),
            &mut data as *mut _ as windows_sys::Win32::Foundation::LPARAM,
        );
    }
    if data.hwnd == 0 {
        None
    } else {
        Some(data.hwnd)
    }
}

#[cfg(windows)]
fn bring_to_foreground(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::Foundation::{BOOL, FALSE, TRUE};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SetForegroundWindow, ShowWindow,
        SW_RESTORE, SW_SHOW,
    };

    #[link(name = "user32")]
    extern "system" {
        fn AttachThreadInput(id_attach: u32, id_attach_to: u32, f_attach: BOOL) -> BOOL;
        fn BringWindowToTop(hwnd: windows_sys::Win32::Foundation::HWND) -> BOOL;
    }

    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        } else {
            ShowWindow(hwnd, SW_SHOW);
        }

        let fg = GetForegroundWindow();
        let mut _fg_pid = 0u32;
        let fg_tid = if fg != 0 {
            GetWindowThreadProcessId(fg, &mut _fg_pid)
        } else {
            0
        };
        let mut _target_pid = 0u32;
        let target_tid = GetWindowThreadProcessId(hwnd, &mut _target_pid);
        let this_tid = GetCurrentThreadId();

        if fg_tid != 0 && fg_tid != this_tid {
            AttachThreadInput(this_tid, fg_tid, TRUE);
        }
        if target_tid != 0 && target_tid != this_tid {
            AttachThreadInput(this_tid, target_tid, TRUE);
        }

        BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);

        if target_tid != 0 && target_tid != this_tid {
            AttachThreadInput(this_tid, target_tid, FALSE);
        }
        if fg_tid != 0 && fg_tid != this_tid {
            AttachThreadInput(this_tid, fg_tid, FALSE);
        }
    }
}

#[cfg(not(windows))]
fn activate_existing(_data_dir: &Path) {
    eprintln!("程序已在运行；请切换到已打开的窗口。");
}
