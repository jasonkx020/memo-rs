fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        // 使用 ASCII / 简体语言 ID，保证「属性 → 详细信息」可读
        res.set_language(0x0804); // zh-CN
        res.set("ProductName", "DistributedMemo");
        res.set("FileDescription", "Distributed Memo - encrypted local notes with LAN sync");
        res.set(
            "LegalCopyright",
            "Copyright (C) 2026 DistributedMemo. All rights reserved.",
        );
        res.set("CompanyName", "DistributedMemo");
        res.set("InternalName", "memo");
        res.set("OriginalFilename", "memo.exe");
        // 0.2.0.0
        res.set_version_info(winres::VersionInfo::PRODUCTVERSION, 0x0000_0002_0000_0000);
        res.set_version_info(winres::VersionInfo::FILEVERSION, 0x0000_0002_0000_0000);
        res.set("ProductVersion", "0.2.0");
        res.set("FileVersion", "0.2.0.0");
        if let Err(e) = res.compile() {
            eprintln!("cargo:warning=winres compile failed: {e}");
        }
    }
}
