//! 流内原子嵌入（表 / 图占位）。

/// 对象替换符：在 `FlowState.text` 中占 1 个字符槽。
pub const EMBED_CHAR: char = '\u{FFFC}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowEmbedKind {
    Table,
    Image,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowEmbed {
    pub id: u64,
    pub kind: FlowEmbedKind,
    /// 在 text 中的字符下标（该处为 [`EMBED_CHAR`]）。
    pub slot: usize,
    /// 宿主回写的占位高度（逻辑像素）。
    pub height: f32,
    /// 占位宽度；0 表示跟编辑器宽。
    pub width: f32,
}

impl FlowEmbed {
    pub fn new(id: u64, kind: FlowEmbedKind, slot: usize) -> Self {
        Self {
            id,
            kind,
            slot,
            height: match kind {
                FlowEmbedKind::Table => 120.0,
                FlowEmbedKind::Image => 160.0,
            },
            width: 0.0,
        }
    }
}

/// 按字符下标查嵌入。
pub fn embed_at_slot(embeds: &[FlowEmbed], slot: usize) -> Option<&FlowEmbed> {
    embeds.iter().find(|e| e.slot == slot)
}

/// 光标落在嵌入槽上时，规范到槽前（左）或槽后（右）。
#[allow(dead_code)]
pub fn snap_cursor_off_embed(embeds: &[FlowEmbed], cursor: usize, prefer_after: bool) -> usize {
    if let Some(e) = embed_at_slot(embeds, cursor) {
        if prefer_after {
            e.slot + 1
        } else {
            e.slot
        }
    } else {
        cursor
    }
}

/// 左移一格：若左侧是嵌入则跳到嵌入前。
pub fn move_left_over_embeds(embeds: &[FlowEmbed], cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let next = cursor - 1;
    if embed_at_slot(embeds, next).is_some() {
        next // 停在槽位上表示「对象被选中/光标在对象前」——统一：光标停在 slot 表示对象前，删除时删 slot
    } else {
        next
    }
}

/// 右移：若当前在嵌入槽则跳到槽后。
pub fn move_right_over_embeds(embeds: &[FlowEmbed], cursor: usize, len: usize) -> usize {
    if cursor >= len {
        return len;
    }
    if embed_at_slot(embeds, cursor).is_some() {
        (cursor + 1).min(len)
    } else {
        (cursor + 1).min(len)
    }
}

/// 文本编辑后按新 text 重算 embeds 的 slot（按 id 顺序扫描 FFFC）。
pub fn resync_embed_slots(text: &str, embeds: &mut Vec<FlowEmbed>) {
    let mut by_id: std::collections::HashMap<u64, FlowEmbed> =
        embeds.drain(..).map(|e| (e.id, e)).collect();
    let mut out = Vec::new();
    for (i, ch) in text.chars().enumerate() {
        if ch != EMBED_CHAR {
            continue;
        }
        // 按出现顺序匹配：优先 slot 最接近的剩余 embed
        let mut best: Option<u64> = None;
        let mut best_dist = usize::MAX;
        for (id, e) in by_id.iter() {
            let d = e.slot.abs_diff(i);
            if d < best_dist {
                best_dist = d;
                best = Some(*id);
            }
        }
        if let Some(id) = best {
            if let Some(mut e) = by_id.remove(&id) {
                e.slot = i;
                out.push(e);
            }
        }
    }
    // 未匹配到的嵌入视为已删除（调用方应读 deleted）
    *embeds = out;
}

/// 从旧 embeds 与新 text 得到被删掉的 id。
#[allow(dead_code)]
pub fn missing_embed_ids(old: &[FlowEmbed], new_text: &str) -> Vec<u64> {
    let mut slots = std::collections::HashSet::new();
    for (i, ch) in new_text.chars().enumerate() {
        if ch == EMBED_CHAR {
            slots.insert(i);
        }
    }
    // 粗略：按数量差；精确由宿主在 sync 时对比 id
    let keep: std::collections::HashSet<u64> = {
        let mut ids: Vec<_> = old.iter().map(|e| e.id).collect();
        let n = new_text.chars().filter(|c| *c == EMBED_CHAR).count();
        ids.truncate(n);
        ids.into_iter().collect()
    };
    old.iter()
        .map(|e| e.id)
        .filter(|id| !keep.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resync_keeps_ids() {
        let mut embeds = vec![
            FlowEmbed::new(1, FlowEmbedKind::Table, 2),
            FlowEmbed::new(2, FlowEmbedKind::Image, 5),
        ];
        let text: String = "ab".chars().chain(std::iter::once(EMBED_CHAR)).chain("cd".chars()).chain(std::iter::once(EMBED_CHAR)).collect();
        resync_embed_slots(&text, &mut embeds);
        assert_eq!(embeds.len(), 2);
        assert_eq!(embeds[0].id, 1);
        assert_eq!(embeds[0].slot, 2);
        assert_eq!(embeds[1].id, 2);
        assert_eq!(embeds[1].slot, 5);
    }
}
