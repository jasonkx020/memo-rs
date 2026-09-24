use chrono::Local;
use std::fs;
use std::path::Path;

use crate::service::MemoView;

pub fn export_txt(path: &Path, node_id: &str, items: &[MemoView]) -> anyhow::Result<()> {
    if items.is_empty() {
        anyhow::bail!("没有可导出的备忘");
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut body = String::new();
    body.push_str("======== 分布式备忘录导出 ========\n");
    body.push_str(&format!("节点: {node_id}\n"));
    body.push_str(&format!("时间: {}\n\n", Local::now().to_rfc3339()));
    for it in items {
        body.push_str("---\n");
        body.push_str(&format!("标题: {}\n", it.title));
        body.push_str(&format!("ID: {}\n", it.id));
        body.push_str(&format!("版本: {}\n", it.version));
        body.push_str(&format!("来源节点: {}\n\n", it.node_id));
        body.push_str(&it.content);
        body.push_str("\n\n");
    }
    fs::write(path, body)?;
    Ok(())
}
