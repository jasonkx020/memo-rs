fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/app_icon.ico");
        let mut res = winres::WindowsResource::new();
        // 使用 ASCII / 简体语言 ID，保证「属性 → 详细信息」可读
        res.set_language(0x0804); // zh-CN
        res.set("ProductName", "密笺");
        res.set(
            "FileDescription",
            "Mijian (密笺) - local encrypted notes with LAN sync",
        );
        res.set(
            "LegalCopyright",
            "Copyright (C) 2026 Mijian. All rights reserved.",
        );
        res.set("CompanyName", "Mijian");
        res.set("InternalName", "mijian");
        res.set("OriginalFilename", "mijian.exe");

        // 与 Cargo.toml workspace version（本包 CARGO_PKG_VERSION）同源
        let ver = env!("CARGO_PKG_VERSION");
        let (maj, min, pat, build) = parse_version_quad(ver);
        let packed = ((maj as u64) << 48)
            | ((min as u64) << 32)
            | ((pat as u64) << 16)
            | (build as u64);
        res.set_version_info(winres::VersionInfo::PRODUCTVERSION, packed);
        res.set_version_info(winres::VersionInfo::FILEVERSION, packed);
        res.set("ProductVersion", ver);
        res.set(
            "FileVersion",
            &format!("{maj}.{min}.{pat}.{build}"),
        );

        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app_icon.ico");
        if icon.is_file() {
            res.set_icon(icon.to_string_lossy().as_ref());
        } else {
            eprintln!("cargo:warning=missing {}", icon.display());
        }
        if let Err(e) = res.compile() {
            eprintln!("cargo:warning=winres compile failed: {e}");
        }
    }
}

#[cfg(windows)]
fn parse_version_quad(s: &str) -> (u16, u16, u16, u16) {
    let mut parts = s
        .trim()
        .trim_start_matches('v')
        .split(|c: char| c == '.' || c == '-' || c == '+')
        .filter_map(|p| {
            let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                None
            } else {
                digits.parse::<u16>().ok()
            }
        });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}
