//! Windows 原生「另存为」对话框；其它平台返回 None（由 UI 文本框填路径）。

use std::path::PathBuf;

/// 弹出另存为对话框。`default_name` 为默认文件名（可含路径或仅文件名）。
/// 用户取消返回 `None`。
pub fn save_txt_dialog(default_name: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows_save_txt(default_name)
    }
    #[cfg(not(windows))]
    {
        let _ = default_name;
        None
    }
}

#[cfg(windows)]
fn windows_save_txt(default_name: &str) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetSaveFileNameW, OPENFILENAMEW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
        OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST,
    };

    // filter: "文本文件 (*.txt)\0*.txt\0所有文件 (*.*)\0*.*\0\0"
    let filter: Vec<u16> = {
        let mut v = Vec::new();
        for part in [
            "文本文件 (*.txt)",
            "*.txt",
            "所有文件 (*.*)",
            "*.*",
        ] {
            v.extend(part.encode_utf16());
            v.push(0);
        }
        v.push(0);
        v
    };

    let path = std::path::Path::new(default_name);
    let (dir, file) = if path.is_absolute() || default_name.contains('\\') || default_name.contains('/')
    {
        let dir = path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "memo-export.txt".into());
        (dir, file)
    } else {
        (String::new(), default_name.to_string())
    };

    let mut file_buf: Vec<u16> = std::ffi::OsStr::new(&file)
        .encode_wide()
        .chain(std::iter::repeat(0).take(1024))
        .collect();
    if file_buf.len() < 1024 {
        file_buf.resize(1024, 0);
    }

    let dir_buf: Vec<u16> = if dir.is_empty() {
        vec![0]
    } else {
        std::ffi::OsStr::new(&dir)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };

    let title: Vec<u16> = "导出备忘录"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let def_ext: Vec<u16> = "txt".encode_utf16().chain(std::iter::once(0)).collect();

    let mut ofn = unsafe { std::mem::zeroed::<OPENFILENAMEW>() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = 0 as HWND;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = file_buf.as_mut_ptr();
    ofn.nMaxFile = file_buf.len() as u32;
    ofn.lpstrInitialDir = if dir.is_empty() {
        ptr::null()
    } else {
        dir_buf.as_ptr()
    };
    ofn.lpstrTitle = title.as_ptr();
    ofn.lpstrDefExt = def_ext.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_HIDEREADONLY | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;

    // OFN_FILEMUSTEXIST 不适合「另存为」新建文件
    let _ = OFN_FILEMUSTEXIST;

    let ok = unsafe { GetSaveFileNameW(&mut ofn) };
    if ok == 0 {
        return None;
    }

    let len = file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len());
    let path = String::from_utf16_lossy(&file_buf[..len]);
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}
