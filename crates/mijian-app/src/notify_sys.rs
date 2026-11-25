//! 跨平台系统通知：Windows WinRT Toast / macOS osascript / Linux notify-send。
//! Windows 在进程内调用 WinRT，不拉起 PowerShell，避免黑控制台闪烁。

/// 尝试弹出系统原生通知。成功返回 true；失败由调用方回退到顶栏状态。
pub fn notify(title: &str, body: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        notify_windows(title, body)
    }
    #[cfg(target_os = "macos")]
    {
        notify_macos(title, body)
    }
    #[cfg(target_os = "linux")]
    {
        notify_linux(title, body)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (title, body);
        false
    }
}

#[cfg(target_os = "windows")]
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(target_os = "windows")]
fn notify_windows(title: &str, body: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    // 未打包应用需有效 AUMID；使用系统内置 PowerShell 身份仅作 Toast 宿主标识，
    // 不会启动 powershell.exe 进程。
    const AUMID: &str =
        "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

    let xml_str = format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{title}</text><text>{body}</text></binding></visual></toast>"#,
        title = escape_xml(title),
        body = escape_xml(body),
    );

    (|| -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml_str))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
        notifier.Show(&toast)?;
        Ok(())
    })()
    .is_ok()
}

#[cfg(target_os = "macos")]
fn notify_macos(title: &str, body: &str) -> bool {
    use std::process::Command;
    let title = title.replace('\\', "\\\\").replace('"', "\\\"");
    let body = body.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(r#"display notification "{body}" with title "{title}""#);
    Command::new("osascript")
        .args(["-e", &script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn notify_linux(title: &str, body: &str) -> bool {
    use std::process::Command;
    Command::new("notify-send")
        .args([title, body])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
