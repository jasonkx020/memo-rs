//! Windows 原生「打开 / 另存为」对话框；其它平台返回 None（由 UI 文本框填路径）。

use std::path::PathBuf;

/// 弹出另存为对话框。`default_name` 为默认文件名（可含路径或仅文件名）。
/// 用户取消返回 `None`。
pub fn save_txt_dialog(default_name: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows_save_dialog(
            default_name,
            "导出备忘录",
            "txt",
            &[
                ("文本文件 (*.txt)", "*.txt"),
                ("所有文件 (*.*)", "*.*"),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        let _ = default_name;
        None
    }
}

/// 另存为 `.memokey` 密钥备份。
pub fn save_memokey_dialog(default_name: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows_save_dialog(
            default_name,
            "导出身份密钥",
            "memokey",
            &[
                ("身份密钥 (*.memokey)", "*.memokey"),
                ("所有文件 (*.*)", "*.*"),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        let _ = default_name;
        None
    }
}

/// 打开已有 `.memokey`。
pub fn open_memokey_dialog() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows_open_dialog(
            "选择身份密钥",
            &[
                ("身份密钥 (*.memokey)", "*.memokey"),
                ("所有文件 (*.*)", "*.*"),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn encode_filter(pairs: &[(&str, &str)]) -> Vec<u16> {
    let mut v = Vec::new();
    for (label, pattern) in pairs {
        v.extend(label.encode_utf16());
        v.push(0);
        v.extend(pattern.encode_utf16());
        v.push(0);
    }
    v.push(0);
    v
}

#[cfg(windows)]
fn windows_save_dialog(
    default_name: &str,
    title: &str,
    def_ext: &str,
    filters: &[(&str, &str)],
) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetSaveFileNameW, OPENFILENAMEW, OFN_EXPLORER, OFN_HIDEREADONLY, OFN_OVERWRITEPROMPT,
        OFN_PATHMUSTEXIST,
    };

    let filter = encode_filter(filters);
    let path = std::path::Path::new(default_name);
    let (dir, file) =
        if path.is_absolute() || default_name.contains('\\') || default_name.contains('/') {
            let dir = path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let file = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("memo.{def_ext}"));
            (dir, file)
        } else if default_name.is_empty() {
            (String::new(), format!("memo.{def_ext}"))
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

    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let def_ext: Vec<u16> = def_ext.encode_utf16().chain(std::iter::once(0)).collect();

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

#[cfg(windows)]
fn windows_open_dialog(title: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OPENFILENAMEW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
        OFN_PATHMUSTEXIST,
    };

    let filter = encode_filter(filters);
    let mut file_buf = vec![0u16; 1024];
    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();

    let mut ofn = unsafe { std::mem::zeroed::<OPENFILENAMEW>() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = 0 as HWND;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = file_buf.as_mut_ptr();
    ofn.nMaxFile = file_buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_HIDEREADONLY | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;

    let ok = unsafe { GetOpenFileNameW(&mut ofn) };
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
