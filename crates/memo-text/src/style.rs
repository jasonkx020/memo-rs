//! 字符级样式 runs 与补丁。

/// 与宿主 Span 对齐的样式字段（作用于 `[start, end)` 字符下标）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyleRun {
    pub start: usize,
    pub end: usize,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub size: Option<u8>,
    pub color: Option<String>,
    pub link: Option<String>,
}

impl StyleRun {
    pub fn plain(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            ..Default::default()
        }
    }

    pub fn same_style(&self, other: &Self) -> bool {
        self.bold == other.bold
            && self.italic == other.italic
            && self.underline == other.underline
            && self.size == other.size
            && self.color == other.color
            && self.link == other.link
    }
}

/// 中立样式补丁：`Some` 才改对应字段。
#[derive(Debug, Clone, Default)]
pub struct StyleDelta {
    pub color: Option<Option<String>>,
    pub link: Option<Option<String>>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub size: Option<Option<u8>>,
}

/// 在 `[start, end)` 上应用补丁；自动切分/合并 runs。
pub fn apply_delta(runs: &mut Vec<StyleRun>, text_len: usize, start: usize, end: usize, delta: &StyleDelta) {
    if start >= end || text_len == 0 {
        return;
    }
    let start = start.min(text_len);
    let end = end.min(text_len);
    if start >= end {
        return;
    }

    // 展平为每字符一份样式，再切回 runs（文本通常不长，实现简单可靠）
    let mut per: Vec<StyleRun> = (0..text_len)
        .map(|i| {
            find_style(runs, i).unwrap_or_else(|| StyleRun::plain(i, i + 1))
        })
        .collect();
    for i in start..end {
        let r = &mut per[i];
        if let Some(b) = delta.bold {
            r.bold = b;
        }
        if let Some(it) = delta.italic {
            r.italic = it;
        }
        if let Some(u) = delta.underline {
            r.underline = u;
        }
        if let Some(sz) = &delta.size {
            r.size = *sz;
        }
        if let Some(c) = &delta.color {
            r.color = c.clone();
        }
        if let Some(l) = &delta.link {
            r.link = l.clone();
            if l.is_some() {
                r.color = None;
            }
        }
        r.start = i;
        r.end = i + 1;
    }
    *runs = coalesce_runs(&per);
}

fn find_style(runs: &[StyleRun], idx: usize) -> Option<StyleRun> {
    runs.iter()
        .find(|r| r.start <= idx && idx < r.end)
        .map(|r| {
            let mut c = r.clone();
            c.start = idx;
            c.end = idx + 1;
            c
        })
}

pub fn coalesce_runs(per_char: &[StyleRun]) -> Vec<StyleRun> {
    let mut out: Vec<StyleRun> = Vec::new();
    for (i, r) in per_char.iter().enumerate() {
        if let Some(last) = out.last_mut() {
            if last.same_style(r) && last.end == i {
                last.end = i + 1;
                continue;
            }
        }
        out.push(StyleRun {
            start: i,
            end: i + 1,
            bold: r.bold,
            italic: r.italic,
            underline: r.underline,
            size: r.size.clone(),
            color: r.color.clone(),
            link: r.link.clone(),
        });
    }
    out
}

/// 文本插入/删除后平移 runs。
pub fn adjust_runs_for_edit(runs: &mut Vec<StyleRun>, at: usize, old_len: usize, new_len: usize) {
    let mut next = Vec::new();
    for r in runs.drain(..) {
        if r.end <= at {
            next.push(r);
            continue;
        }
        if r.start >= at + old_len {
            let delta = new_len as isize - old_len as isize;
            next.push(StyleRun {
                start: (r.start as isize + delta).max(0) as usize,
                end: (r.end as isize + delta).max(0) as usize,
                ..r
            });
            continue;
        }
        // 与编辑区间相交：保留编辑前段；新插入段继承左邻样式
        let keep_end = at.min(r.end);
        if r.start < keep_end {
            next.push(StyleRun {
                end: keep_end,
                ..r.clone()
            });
        }
        if new_len > 0 && r.start <= at && at < r.end {
            // 插入落在本 run 内：扩展
            next.push(StyleRun {
                start: at,
                end: at + new_len,
                bold: r.bold,
                italic: r.italic,
                underline: r.underline,
                size: r.size.clone(),
                color: r.color.clone(),
                link: r.link.clone(),
            });
        }
        if r.end > at + old_len {
            let delta = new_len as isize - old_len as isize;
            next.push(StyleRun {
                start: (at as isize + new_len as isize).max(0) as usize,
                end: (r.end as isize + delta).max(0) as usize,
                ..r
            });
        }
    }
    // 合并相邻同样式
    next.sort_by_key(|r| r.start);
    let mut merged: Vec<StyleRun> = Vec::new();
    for r in next {
        if r.start >= r.end {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if last.end == r.start && last.same_style(&r) {
                last.end = r.end;
                continue;
            }
        }
        merged.push(r);
    }
    *runs = merged;
}

/// 从「每段 spans」展平为 text + runs（段间插入 `sep`，sep 无样式）。
#[allow(dead_code)]
pub fn flatten_paragraphs(
    paragraphs: &[Vec<(String, StyleRun)>],
    sep: &str,
) -> (String, Vec<StyleRun>) {
    let mut text = String::new();
    let mut runs = Vec::new();
    let sep_chars = sep.chars().count();
    for (pi, para) in paragraphs.iter().enumerate() {
        if pi > 0 {
            text.push_str(sep);
        }
        let base = text.chars().count();
        let mut local = 0usize;
        for (chunk, style) in para {
            let n = chunk.chars().count();
            if n > 0 {
                let mut r = style.clone();
                r.start = base + local;
                r.end = base + local + n;
                runs.push(r);
                text.push_str(chunk);
                local += n;
            }
        }
        let _ = sep_chars;
    }
    (text, coalesce_from_ranges(runs))
}

fn coalesce_from_ranges(mut runs: Vec<StyleRun>) -> Vec<StyleRun> {
    runs.sort_by_key(|r| r.start);
    let mut out: Vec<StyleRun> = Vec::new();
    for r in runs {
        if r.start >= r.end {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.end == r.start && last.same_style(&r) {
                last.end = r.end;
                continue;
            }
        }
        out.push(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_bold_range() {
        let mut runs = vec![StyleRun::plain(0, 5)];
        apply_delta(
            &mut runs,
            5,
            1,
            3,
            &StyleDelta {
                bold: Some(true),
                ..Default::default()
            },
        );
        assert!(runs.iter().any(|r| r.bold && r.start == 1 && r.end == 3));
    }
}
