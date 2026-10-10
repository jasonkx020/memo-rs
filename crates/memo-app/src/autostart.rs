//! 开机自启动（Windows：HKCU Run；其它平台：配置可存但不生效）。

const RUN_VALUE_NAME: &str = "DistributedMemo";

/// 是否支持本机写入开机启动项。
pub fn supported() -> bool {
    cfg!(windows)
}

/// 按开关同步系统启动项；成功返回 Ok(())。
pub fn apply(enabled: bool) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        apply_windows(enabled)
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Ok(())
    }
}

#[cfg(windows)]
fn apply_windows(enabled: bool) -> anyhow::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_WRITE,
        REG_SZ,
    };

    let subkey: Vec<u16> = OsStr::new(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let value_name: Vec<u16> = OsStr::new(RUN_VALUE_NAME)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut hkey = 0isize;
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_WRITE,
            &mut hkey,
        );
        if status != ERROR_SUCCESS {
            anyhow::bail!("无法打开注册表 Run 键 (code {status})");
        }

        let result = (|| -> anyhow::Result<()> {
            if enabled {
                let exe = std::env::current_exe()
                    .map_err(|e| anyhow::anyhow!("无法解析可执行路径: {e}"))?;
                let cmd = format!("\"{}\" --tray", exe.display());
                let wide: Vec<u16> = OsStr::new(&cmd)
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let bytes = (wide.len() * 2) as u32;
                let st = RegSetValueExW(
                    hkey,
                    value_name.as_ptr(),
                    0,
                    REG_SZ,
                    wide.as_ptr() as *const u8,
                    bytes,
                );
                if st != ERROR_SUCCESS {
                    anyhow::bail!("写入开机启动失败 (code {st})");
                }
            } else {
                let st = RegDeleteValueW(hkey, value_name.as_ptr());
                if st != ERROR_SUCCESS && st != ERROR_FILE_NOT_FOUND {
                    anyhow::bail!("删除开机启动失败 (code {st})");
                }
            }
            Ok(())
        })();

        RegCloseKey(hkey);
        result
    }
}
