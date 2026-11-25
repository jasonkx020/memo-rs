//! 数据目录所在卷的磁盘空间探测。

use std::path::Path;

#[derive(Debug, Clone, Copy, Default)]
pub struct DiskSpace {
    pub free_bytes: u64,
    pub total_bytes: u64,
}

impl DiskSpace {
    pub fn is_low(self) -> bool {
        const MIN_FREE: u64 = 512 * 1024 * 1024;
        if self.free_bytes < MIN_FREE {
            return true;
        }
        if self.total_bytes == 0 {
            return false;
        }
        self.free_bytes * 100 / self.total_bytes < 5
    }

    pub fn format_pair(self) -> String {
        format!(
            "{} / {}",
            format_bytes(self.free_bytes),
            format_bytes(self.total_bytes)
        )
    }
}

pub fn format_bytes(n: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;
    let x = n as f64;
    if x >= TIB {
        format!("{:.1} TiB", x / TIB)
    } else if x >= GIB {
        format!("{:.1} GiB", x / GIB)
    } else if x >= MIB {
        format!("{:.1} MiB", x / MIB)
    } else if x >= KIB {
        format!("{:.1} KiB", x / KIB)
    } else {
        format!("{n} B")
    }
}

/// 探测 `data_dir` 所在卷的可用/总空间；目录不存在时回退到父路径或当前目录。
pub fn probe_data_dir(data_dir: &Path) -> DiskSpace {
    let candidates = [
        Some(data_dir),
        data_dir.parent(),
        Some(Path::new(".")),
    ];
    for p in candidates.into_iter().flatten() {
        if let Ok(space) = probe_path(p) {
            return space;
        }
    }
    DiskSpace::default()
}

fn probe_path(path: &Path) -> std::io::Result<DiskSpace> {
    let free = fs2::available_space(path)?;
    let total = fs2::total_space(path)?;
    Ok(DiskSpace {
        free_bytes: free,
        total_bytes: total,
    })
}

/// 全量同步落盘需求估算：载荷 × 2.5（审计 before/after）+ 安全余量。
pub fn estimate_sync_need(payload_bytes: u64) -> u64 {
    let inflated = payload_bytes.saturating_mul(5) / 2; // * 2.5
    let pct = payload_bytes / 5; // 20%
    let margin = pct.max(256 * 1024 * 1024);
    inflated.saturating_add(margin)
}

pub const INCREMENTAL_MIN_FREE: u64 = 128 * 1024 * 1024;
pub const SKIP_FULL_SYNC_FREE: u64 = 256 * 1024 * 1024;

pub fn disk_help_hint() -> &'static str {
    "磁盘空间不足时：1) 清理数据盘无关大文件；2) 设置中将「数据目录」改到更大磁盘并迁移后重启；3) 删除不需要的 exports/ 与旧 .memobak；4) 满盘时勿大量导入或全量同步。"
}
