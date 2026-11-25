//! 无 UI / 存储节点单实例锁（与 GUI 的 `gui.single.lock` 分离）。

use fs2::FileExt;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const LOCK_NAME: &str = "node.single.lock";

/// 持有锁直至 drop（进程退出自动释放）。
pub struct NodeLockGuard {
    _file: std::fs::File,
}

fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LOCK_NAME)
}

/// 在 `data_dir` 下独占锁定；已有实例则返回错误。
pub fn try_acquire(data_dir: &Path) -> anyhow::Result<NodeLockGuard> {
    let _ = std::fs::create_dir_all(data_dir);
    let path = lock_path(data_dir);
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| anyhow::anyhow!("无法创建节点锁 {}: {e}", path.display()))?;
    file.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("数据目录已被其他 memo 节点占用: {}", data_dir.display()))?;
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    write!(file, "{}", std::process::id())?;
    let _ = file.flush();
    Ok(NodeLockGuard { _file: file })
}
