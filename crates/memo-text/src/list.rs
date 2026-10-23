//! 纯文本列表前缀：多样式、回车续项、重编号（空项不自动退出，由用户自行清前缀）。

use crate::widget::FlowState;

/// 列表样式（写入行首纯文本前缀）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyle {
    /// 1. 2. 3.
    Decimal,
    /// a. b. c.
    LowerAlpha,
    /// A. B. C.
    UpperAlpha,
    /// i. ii. iii.
    LowerRoman,
    /// I. II. III.
    UpperRoman,
    /// •
    Disc,
    /// -
    Dash,
    /// ○
    Circle,
    /// *
    Asterisk,
}

impl ListStyle {
    pub fn is_ordered(self) -> bool {
        matches!(
            self,
            Self::Decimal | Self::LowerAlpha | Self::UpperAlpha | Self::LowerRoman | Self::UpperRoman
        )
    }

    pub fn toolbar_label(self) -> &'static str {
        match self {
            Self::Decimal => "1. 2. 3.",
            Self::LowerAlpha => "a. b. c.",
            Self::UpperAlpha => "A. B. C.",
            Self::LowerRoman => "i. ii. iii.",
            Self::UpperRoman => "I. II. III.",
            Self::Disc => "• 实心圆",
            Self::Dash => "- 短横",
            Self::Circle => "○ 空心圆",
            Self::Asterisk => "* 星号",
        }
    }

    pub fn prefix(self, index: usize) -> String {
        let n = index.max(1);
        match self {
            Self::Decimal => format!("{n}. "),
            Self::LowerAlpha => format!("{}. ", alpha(n, false)),
            Self::UpperAlpha => format!("{}. ", alpha(n, true)),
            Self::LowerRoman => format!("{}. ", roman(n, false)),
            Self::UpperRoman => format!("{}. ", roman(n, true)),
            Self::Disc => "• ".to_string(),
            Self::Dash => "- ".to_string(),
            Self::Circle => "○ ".to_string(),
            Self::Asterisk => "* ".to_string(),
        }
    }
}

fn alpha(n: usize, upper: bool) -> String {
    let mut n = n;
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        let c = (b'a' + (n % 26) as u8) as char;
        s.insert(0, if upper { c.to_ascii_uppercase() } else { c });
        n /= 26;
    }
    if s.is_empty() {
        s.push(if upper { 'A' } else { 'a' });
    }
    s
}

fn roman(mut n: usize, upper: bool) -> String {
    if n == 0 {
        n = 1;
    }
    let table = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for &(v, sym) in &table {
        while n >= v {
            s.push_str(sym);
            n -= v;
        }
    }
    if upper {
        s.to_ascii_uppercase()
    } else {
        s
    }
}

/// 解析行首列表前缀 → (样式, 序号, 正文, 前缀字符数)。
/// `prefix_char_len` 为原行中前缀所占字符数（含标记后空格），供回车切开用。
///
/// 仅识别与 [`ListStyle::prefix`] 一致的编号/项目符号；拒绝 `1.hello`、`NOTE. x`、
/// `Dr. x` 等，避免普通段落误走列表回车替换。
pub fn parse_list_prefix(line: &str) -> Option<(ListStyle, usize, &str, usize)> {
    let t = line;
    // bullets：行首 •/○/-/*，其后为空或空白（拒绝 `-foo`）
    for (pref, style) in [
        ("• ", ListStyle::Disc),
        ("○ ", ListStyle::Circle),
        ("- ", ListStyle::Dash),
        ("* ", ListStyle::Asterisk),
    ] {
        if let Some(rest) = t.strip_prefix(pref) {
            return Some((style, 1, rest, pref.chars().count()));
        }
    }
    for (pref, style) in [
        ('•', ListStyle::Disc),
        ('○', ListStyle::Circle),
        ('-', ListStyle::Dash),
        ('*', ListStyle::Asterisk),
    ] {
        if let Some(rest) = t.strip_prefix(pref) {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                let body = rest.trim_start();
                let prefix_char_len = t.chars().count() - body.chars().count();
                return Some((style, 1, body, prefix_char_len));
            }
        }
    }
    // ordered：纯数字 / 纯字母 / 罗马 + '.'，且点后必须为空或空白（对齐 `"1. "`）
    let bytes = t.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() || bytes[i] != b'.' {
        return None;
    }
    let marker = &t[..i];
    let after_dot = &t[i + 1..];
    // 拒绝 `1.hello`：点后若有内容，首字符必须是空白
    if !after_dot.is_empty() && !after_dot.starts_with(char::is_whitespace) {
        return None;
    }
    let body = after_dot.trim_start();
    let prefix_char_len = t.chars().count() - body.chars().count();

    // Decimal：纯 ASCII 数字
    if marker.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(n) = marker.parse::<usize>() {
            return Some((ListStyle::Decimal, n.max(1), body, prefix_char_len));
        }
        return None;
    }

    // Alpha / Roman：长度上限（alpha≤3 覆盖 a..aaa，排除 NOTE./CHAPTER.）
    const MAX_ALPHA_MARKER: usize = 3;
    const MAX_ROMAN_MARKER: usize = 12;

    if marker.chars().all(|c| c.is_ascii_lowercase()) {
        if looks_roman(marker) && marker.len() <= MAX_ROMAN_MARKER {
            if let Some(n) = parse_roman(marker) {
                return Some((ListStyle::LowerRoman, n, body, prefix_char_len));
            }
        }
        if marker.len() <= MAX_ALPHA_MARKER {
            if let Some(n) = parse_alpha(marker) {
                return Some((ListStyle::LowerAlpha, n, body, prefix_char_len));
            }
        }
        return None;
    }
    if marker.chars().all(|c| c.is_ascii_uppercase()) {
        let lower = marker.to_ascii_lowercase();
        if looks_roman(&lower) && lower.len() <= MAX_ROMAN_MARKER {
            if let Some(n) = parse_roman(&lower) {
                return Some((ListStyle::UpperRoman, n, body, prefix_char_len));
            }
        }
        if lower.len() <= MAX_ALPHA_MARKER {
            if let Some(n) = parse_alpha(&lower) {
                return Some((ListStyle::UpperAlpha, n, body, prefix_char_len));
            }
        }
        return None;
    }
    // 大小写混合（如 Dr.）不认作列表
    None
}

fn looks_roman(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'))
}

fn parse_alpha(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut n = 0usize;
    for c in s.chars() {
        if !c.is_ascii_lowercase() {
            return None;
        }
        n = n * 26 + (c as u8 - b'a') as usize + 1;
    }
    Some(n)
}

fn parse_roman(s: &str) -> Option<usize> {
    let map = |c| match c {
        'i' => 1,
        'v' => 5,
        'x' => 10,
        'l' => 50,
        'c' => 100,
        'd' => 500,
        'm' => 1000,
        _ => 0,
    };
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() || chars.iter().any(|&c| map(c) == 0) {
        return None;
    }
    let mut total = 0i32;
    for i in 0..chars.len() {
        let v = map(chars[i]);
        let next = if i + 1 < chars.len() {
            map(chars[i + 1])
        } else {
            0
        };
        if v < next {
            total -= v;
        } else {
            total += v;
        }
    }
    if total <= 0 {
        None
    } else {
        Some(total as usize)
    }
}

/// 光标所在行的 `[line_start, line_end)` 字符下标（不含末尾 `\n`）。
pub fn line_at_cursor(text: &str, cursor: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let mut start = cursor;
    while start > 0 && chars[start - 1] != '\n' {
        start -= 1;
    }
    let mut end = cursor;
    while end < chars.len() && chars[end] != '\n' {
        end += 1;
    }
    (start, end)
}

fn line_str(chars: &[char], start: usize, end: usize) -> String {
    chars[start..end].iter().collect()
}

/// 列表行正文起点（非列表行则为行首）。
pub fn line_body_start(text: &str, cursor: usize) -> usize {
    let (ls, le) = line_at_cursor(text, cursor);
    let chars: Vec<char> = text.chars().collect();
    let line = line_str(&chars, ls, le);
    match parse_list_prefix(&line) {
        Some((_, _, _, plen)) => ls + plen,
        None => ls,
    }
}

/// 若光标落在列表前缀内，夹到正文起点（避免在 `1.|` / `•|` 里打字）。
pub fn clamp_cursor_to_list_body(text: &str, cursor: usize) -> usize {
    let len = text.chars().count();
    let cursor = cursor.min(len);
    let body = line_body_start(text, cursor);
    if cursor < body {
        body
    } else {
        cursor
    }
}

/// 在列表项正文起点按 Backspace：去掉本行前缀（不删正文），并重编号。
/// 返回是否已处理（未在正文起点则 false，交给普通退格）。
pub fn handle_backspace_list(state: &mut FlowState) -> bool {
    if state.selection.is_some_and(|s| !s.is_empty()) {
        return false;
    }
    let cursor = state.cursor.min(state.text.chars().count());
    let (ls, le) = line_at_cursor(&state.text, cursor);
    let chars: Vec<char> = state.text.chars().collect();
    let line = line_str(&chars, ls, le);
    let Some((_, _, body, plen)) = parse_list_prefix(&line) else {
        return false;
    };
    if cursor != ls + plen {
        return false;
    }
    let body = body.to_string();
    replace_range(state, ls, le, &body);
    state.cursor = ls;
    state.ime_anchor = state.cursor;
    state.prefer_next_row = ls == 0 || state.text.chars().nth(ls.saturating_sub(1)) == Some('\n');
    state.clear_selection();
    renumber_ordered(state);
    state.ime_anchor = state.cursor;
    true
}

/// 光标所在行的行号（0-based）。
pub fn line_index_at(text: &str, cursor: usize) -> usize {
    let (ls, _) = line_at_cursor(text, cursor);
    let mut at = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        if at == ls {
            return i;
        }
        at += l.chars().count() + 1;
    }
    0
}

/// 指定行正文起点的字符下标（列表行跳过前缀；行号越界则回文末）。
pub fn cursor_at_line_body(text: &str, line_idx: usize) -> usize {
    let mut at = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        if i == line_idx {
            return match parse_list_prefix(l) {
                Some((_, _, _, p)) => at + p,
                None => at,
            };
        }
        at += l.chars().count() + 1;
    }
    text.chars().count()
}

/// 按逻辑行（`\n`）上下移动，保持正文内列位；列表行列位相对正文起点。
/// 与回车/列表编辑同一套行模型，避免 galley 视觉行在 `\n` 边界把光标画到下一行。
pub fn move_vert_logical(
    text: &str,
    cursor: usize,
    dir: i32,
    preferred_col: &mut Option<usize>,
) -> usize {
    let len = text.chars().count();
    let cursor = clamp_cursor_to_list_body(text, cursor.min(len));
    let body = line_body_start(text, cursor);
    let (_, le) = line_at_cursor(text, cursor);
    if preferred_col.is_none() {
        *preferred_col = Some(cursor.saturating_sub(body));
    }
    let col = preferred_col.unwrap_or(0);
    let idx = line_index_at(text, cursor);
    let nlines = text.split('\n').count().max(1);
    let new_idx = if dir < 0 {
        if idx == 0 {
            return body;
        }
        idx - 1
    } else if idx + 1 >= nlines {
        return le;
    } else {
        idx + 1
    };
    let mut at = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        if i == new_idx {
            let bstart = match parse_list_prefix(l) {
                Some((_, _, _, p)) => at + p,
                None => at,
            };
            let bend = at + l.chars().count();
            return (bstart + col).min(bend);
        }
        at += l.chars().count() + 1;
    }
    cursor
}

/// 回车：仅在光标处插入 `\n`，光标必须落到该 `\n` **之后**（换行在光标前）。
/// 不继承编号/项目符号，不做列表整行替换或自动续项。
pub fn handle_enter(state: &mut FlowState) -> bool {
    let cursor = state.cursor.min(state.text.chars().count());
    replace_range(state, cursor, cursor, "\n");
    // 显式越过刚插入的换行，避免停在 `\n` 下标上被画成「还在旧行」
    state.cursor = (cursor + 1).min(state.text.chars().count());
    debug_assert_eq!(
        state.text.chars().nth(cursor),
        Some('\n'),
        "插入点应是换行符"
    );
    state.ime_anchor = state.cursor;
    // 回车后光标在新行行首
    state.prefer_next_row = true;
    state.clear_selection();
    true
}

fn replace_range(state: &mut FlowState, start: usize, end: usize, insert: &str) {
    let chars: Vec<char> = state.text.chars().collect();
    let start = start.min(chars.len());
    let end = end.min(chars.len()).max(start);
    let before: String = chars[..start].iter().collect();
    let after: String = chars[end..].iter().collect();
    let old_len = end - start;
    let new_len = insert.chars().count();
    state.text = format!("{before}{insert}{after}");
    crate::style::adjust_runs_for_edit(&mut state.style_runs, start, old_len, new_len);
}

/// 列表前缀改写后，按「行内正文偏移」重映射光标，避免停在编号/`•` 中间。
///
/// 光标下标等于本行后 `\n` 的位置时，视为「本行行末」（空正文列表项的正文起点也在此），
/// **不要**落到下一行，否则会在「5. 末回车」后看起来跳到旧的「6.」。
fn remap_cursor_across_list_rewrite(old: &str, new: &str, cursor: usize) -> usize {
    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();
    let new_len = new.chars().count();
    if old_lines.len() != new_lines.len() {
        return cursor.min(new_len);
    }

    let cursor = cursor.min(old.chars().count());
    let mut old_at = 0usize;
    let mut new_at = 0usize;
    for (oi, ol) in old_lines.iter().enumerate() {
        let ol_len = ol.chars().count();
        let nl = new_lines[oi];
        let nl_len = nl.chars().count();
        let line_end = old_at + ol_len; // 亦为本行后 `\n` 的下标（若有）
        if cursor <= line_end {
            let old_in = cursor.saturating_sub(old_at);
            let (old_pre, old_body) = match parse_list_prefix(ol) {
                Some((_, _, b, p)) => (p, b),
                None => (0usize, *ol),
            };
            let new_pre = parse_list_prefix(nl)
                .map(|(_, _, _, p)| p)
                .unwrap_or(0);
            let body_len = old_body.chars().count();
            let body_off = if old_in <= old_pre {
                0
            } else {
                (old_in - old_pre).min(body_len)
            };
            // 空正文：始终停在新前缀后（行末），勿用绝对下标滑到下一行
            return (new_at + new_pre + body_off).min(new_at + nl_len).min(new_len);
        }
        old_at = line_end + 1; // skip `\n`
        new_at += nl_len + 1;
    }
    new_len
}

/// 对连续有序列表行重编号。
pub fn renumber_ordered(state: &mut FlowState) {
    let lines: Vec<String> = state.text.split('\n').map(|s| s.to_string()).collect();
    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        if let Some((style, _, _body, _)) = parse_list_prefix(&lines[i]) {
            if style.is_ordered() {
                let mut j = i;
                let mut group = Vec::new();
                while j < lines.len() {
                    if let Some((s, _, b, _)) = parse_list_prefix(&lines[j]) {
                        if s == style {
                            group.push(b.to_string());
                            j += 1;
                            continue;
                        }
                    }
                    break;
                }
                for (k, body) in group.into_iter().enumerate() {
                    out_lines.push(format!("{}{}", style.prefix(k + 1), body));
                }
                i = j;
                continue;
            }
        }
        out_lines.push(lines[i].clone());
        i += 1;
    }
    let new_text = out_lines.join("\n");
    if new_text != state.text {
        let old_text = state.text.clone();
        let cursor = remap_cursor_across_list_rewrite(&old_text, &new_text, state.cursor);
        state.text = new_text;
        let len = state.text.chars().count();
        state.style_runs = if len == 0 {
            Vec::new()
        } else {
            vec![crate::style::StyleRun::plain(0, len)]
        };
        state.cursor = cursor.min(len);
    }
}

/// 给若干行施加/切换列表样式（`indices` 为行号 0-based）。
///
/// - `style = Some(s)`：目标行已全是 `s` 则去掉前缀；否则加上/切换为 `s`。
/// - `style = None`：去掉目标行上的任意列表前缀。
/// 空行不参与「已是目标样式」判断，施加时也跳过（避免全选带尾空行时无法取消）。
pub fn apply_list_style_to_lines(
    state: &mut FlowState,
    line_indices: &[usize],
    style: Option<ListStyle>,
) {
    let old_text = state.text.clone();
    let old_cursor = state.cursor;
    let mut lines: Vec<String> = state.text.split('\n').map(|s| s.to_string()).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    let non_empty: Vec<usize> = line_indices
        .iter()
        .copied()
        .filter(|&i| lines.get(i).is_some_and(|l| !l.trim().is_empty()))
        .collect();
    let all_want = match style {
        Some(want) => {
            !non_empty.is_empty()
                && non_empty.iter().all(|&i| {
                    parse_list_prefix(&lines[i]).map(|(s, _, _, _)| s) == Some(want)
                })
        }
        None => false,
    };
    let apply = if all_want { None } else { style };

    let mut n = 1usize;
    for &i in line_indices {
        if i >= lines.len() {
            continue;
        }
        // 施加样式时跳过空行；取消时仍处理（空行无前缀则不变）
        if apply.is_some() && lines[i].trim().is_empty() {
            continue;
        }
        let body = parse_list_prefix(&lines[i])
            .map(|(_, _, b, _)| b.to_string())
            .unwrap_or_else(|| lines[i].clone());
        lines[i] = match apply {
            Some(s) => {
                let p = s.prefix(n);
                if s.is_ordered() {
                    n += 1;
                }
                format!("{p}{body}")
            }
            None => body,
        };
    }
    state.text = lines.join("\n");
    let len = state.text.chars().count();
    state.style_runs = if len == 0 {
        Vec::new()
    } else {
        vec![crate::style::StyleRun::plain(0, len)]
    };
    state.cursor = remap_cursor_across_list_rewrite(&old_text, &state.text, old_cursor);
    renumber_ordered(state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::FlowState;

    #[test]
    fn enter_on_list_line_is_plain_newline_no_continue() {
        // 列表行回车不续编号/项目符号
        let mut s = FlowState::from_plain("1. hello");
        s.cursor = s.text.chars().count();
        handle_enter(&mut s);
        assert_eq!(s.text, "1. hello\n");
        assert_eq!(s.cursor, s.text.chars().count());
        assert!(!s.text.contains("2. "));
    }

    #[test]
    fn enter_splits_mid_list_item_plain() {
        let mut s = FlowState::from_plain("1. helloworld");
        s.cursor = "1. hello".chars().count();
        handle_enter(&mut s);
        assert_eq!(s.text, "1. hello\nworld");
        assert_eq!(s.cursor, "1. hello\n".chars().count());
    }

    #[test]
    fn enter_splits_mid_bullet_plain() {
        let mut s = FlowState::from_plain("• helloworld");
        s.cursor = "• hello".chars().count();
        handle_enter(&mut s);
        assert_eq!(s.text, "• hello\nworld");
    }

    #[test]
    fn enter_mid_plain_roundtrip_shape() {
        let mut s = FlowState::from_plain("1. abcd");
        s.cursor = "1. ab".chars().count();
        handle_enter(&mut s);
        assert_eq!(s.text, "1. ab\ncd");
        s.cursor = s.text.chars().count();
        handle_enter(&mut s);
        assert_eq!(s.text, "1. ab\ncd\n");
    }

    #[test]
    fn empty_list_item_enter_plain_newline() {
        let mut s = FlowState::from_plain("1. hello\n2. ");
        s.cursor = s.text.chars().count();
        handle_enter(&mut s);
        assert_eq!(s.text, "1. hello\n2. \n");
        assert_eq!(s.cursor, s.text.chars().count());
        assert_eq!(s.ime_anchor, s.cursor);
    }

    #[test]
    fn consecutive_plain_enters_advance_cursor() {
        let mut s = FlowState::from_plain("1. hello");
        s.cursor = s.text.chars().count();
        let mut prev = s.cursor;
        for _ in 0..4 {
            handle_enter(&mut s);
            assert!(s.cursor > prev);
            let before: String = s.text.chars().take(s.cursor).collect();
            assert!(before.ends_with('\n'));
            prev = s.cursor;
        }
        assert_eq!(s.text, "1. hello\n\n\n\n");
    }

    #[test]
    fn enter_puts_newline_before_cursor() {
        let mut s = FlowState::from_plain("1. hello");
        s.cursor = s.text.chars().count();
        handle_enter(&mut s);
        let before: String = s.text.chars().take(s.cursor).collect();
        assert!(before.ends_with("1. hello\n"), "before={before:?}");
        assert_ne!(s.text.chars().nth(s.cursor), Some('\n'));
    }

    #[test]
    fn enter_at_trailing_nl_inserts_before_it() {
        // 光标在行后 `\n` 上：插入另一个 `\n`，光标越过新插入的换行
        let mut s = FlowState::from_plain("1. hello\n2. \n");
        s.cursor = cursor_at_line_body(&s.text, 1);
        assert_eq!(s.text.chars().nth(s.cursor), Some('\n'));
        let at = s.cursor;
        handle_enter(&mut s);
        assert_eq!(s.cursor, at + 1);
        assert_eq!(&s.text.chars().skip(at).take(2).collect::<String>(), "\n\n");
    }

    #[test]
    fn enter_mid_list_plain_keeps_following_lines() {
        let mut s = FlowState::from_plain("1. a\n2. \n3. foo");
        s.cursor = cursor_at_line_body(&s.text, 1);
        handle_enter(&mut s);
        assert_eq!(s.text, "1. a\n2. \n\n3. foo");
        assert!(s.text.split('\n').any(|l| l == "3. foo"));
    }

    #[test]
    fn plain_enter_moves_cursor_to_new_line() {
        let mut s = FlowState::from_plain("hello");
        s.cursor = s.text.chars().count();
        handle_enter(&mut s);
        assert_eq!(s.text, "hello\n");
        assert_eq!(s.cursor, 6);
        handle_enter(&mut s);
        assert_eq!(s.text, "hello\n\n");
        assert_eq!(s.cursor, 7);
    }

    #[test]
    fn parse_disc() {
        let (st, _, body, plen) = parse_list_prefix("• abc").unwrap();
        assert_eq!(st, ListStyle::Disc);
        assert_eq!(body, "abc");
        assert_eq!(plen, 2);
    }

    #[test]
    fn parse_list_prefix_accepts_real_markers() {
        assert!(parse_list_prefix("1. a").is_some());
        assert!(parse_list_prefix("1. ").is_some());
        assert!(parse_list_prefix("• a").is_some());
        assert!(parse_list_prefix("- a").is_some());
        assert!(parse_list_prefix("* a").is_some());
        assert!(parse_list_prefix("a. a").is_some());
        assert!(parse_list_prefix("i. a").is_some());
        assert!(parse_list_prefix("A. a").is_some());
        assert!(parse_list_prefix("II. a").is_some());
        // 空项：点后无正文
        assert!(parse_list_prefix("2.").is_some());
        assert!(parse_list_prefix("-").is_some());
    }

    #[test]
    fn parse_list_prefix_rejects_false_positives() {
        assert!(parse_list_prefix("hello").is_none());
        assert!(parse_list_prefix("1.hello").is_none());
        assert!(parse_list_prefix("NOTE. x").is_none());
        assert!(parse_list_prefix("CHAPTER. x").is_none());
        assert!(parse_list_prefix("Dr. x").is_none());
        assert!(parse_list_prefix("1.2").is_none());
        assert!(parse_list_prefix("a) item").is_none());
        assert!(parse_list_prefix("-foo").is_none());
        // 行中编号不算行首前缀
        assert!(parse_list_prefix("see 1. above").is_none());
    }

    #[test]
    fn enter_never_auto_continues_list_markers() {
        for src in ["1. a", "• a", "- a", "a. a", "i. a"] {
            let mut s = FlowState::from_plain(src);
            s.cursor = s.text.chars().count();
            handle_enter(&mut s);
            assert_eq!(s.text, format!("{src}\n"), "src={src:?}");
            let next = s.text.lines().nth(1).unwrap_or("");
            assert!(
                next.is_empty(),
                "不得自动续前缀：src={src:?} next={next:?}"
            );
        }
    }

    #[test]
    fn enter_plain_for_non_list_lines() {
        for src in ["hello", "NOTE. x", "1.hello", "Dr. x"] {
            let mut s = FlowState::from_plain(src);
            s.cursor = s.text.chars().count();
            handle_enter(&mut s);
            assert_eq!(s.text, format!("{src}\n"), "src={src:?}");
        }
    }

    #[test]
    fn toggle_same_style_strips() {
        let mut s = FlowState::from_plain("1. hello");
        apply_list_style_to_lines(&mut s, &[0], Some(ListStyle::Decimal));
        assert_eq!(s.text, "hello");
    }

    #[test]
    fn toggle_ignores_trailing_blank_for_strip() {
        let mut s = FlowState::from_plain("1. a\n2. b\n");
        // 含尾空行时仍应能取消
        apply_list_style_to_lines(&mut s, &[0, 1, 2], Some(ListStyle::Decimal));
        assert_eq!(s.text, "a\nb\n");
    }

    #[test]
    fn clear_any_list_prefix() {
        let mut s = FlowState::from_plain("• x\n- y");
        apply_list_style_to_lines(&mut s, &[0, 1], None);
        assert_eq!(s.text, "x\ny");
    }

    #[test]
    fn apply_list_moves_cursor_into_body() {
        let mut s = FlowState::from_plain("hello");
        s.cursor = 0; // 行首
        apply_list_style_to_lines(&mut s, &[0], Some(ListStyle::Decimal));
        assert_eq!(s.text, "1. hello");
        // 原在前缀区 → 夹到正文起点
        assert_eq!(s.cursor, "1. ".chars().count());
    }

    #[test]
    fn apply_list_keeps_cursor_at_body_end() {
        let mut s = FlowState::from_plain("hello");
        s.cursor = s.text.chars().count();
        apply_list_style_to_lines(&mut s, &[0], Some(ListStyle::Decimal));
        assert_eq!(s.text, "1. hello");
        assert_eq!(s.cursor, "1. hello".chars().count());
    }

    #[test]
    fn strip_list_keeps_cursor_in_body() {
        let mut s = FlowState::from_plain("1. hello");
        s.cursor = "1. hello".chars().count();
        apply_list_style_to_lines(&mut s, &[0], Some(ListStyle::Decimal));
        assert_eq!(s.text, "hello");
        assert_eq!(s.cursor, "hello".chars().count());
    }

    #[test]
    fn clamp_moves_out_of_ordered_prefix() {
        let t = "1. hello";
        assert_eq!(clamp_cursor_to_list_body(t, 0), "1. ".chars().count());
        assert_eq!(clamp_cursor_to_list_body(t, 1), "1. ".chars().count());
        assert_eq!(clamp_cursor_to_list_body(t, "1. ".chars().count()), "1. ".chars().count());
        assert_eq!(clamp_cursor_to_list_body(t, "1. he".chars().count()), "1. he".chars().count());
    }

    #[test]
    fn clamp_moves_out_of_bullet_prefix() {
        let t = "• abc";
        assert_eq!(clamp_cursor_to_list_body(t, 0), "• ".chars().count());
        assert_eq!(clamp_cursor_to_list_body(t, 1), "• ".chars().count());
    }

    #[test]
    fn backspace_at_body_start_strips_prefix() {
        let mut s = FlowState::from_plain("1. hello\n2. world");
        s.cursor = "1. hello\n2. ".chars().count();
        assert!(handle_backspace_list(&mut s));
        assert_eq!(s.text, "1. hello\nworld");
        assert_eq!(s.cursor, "1. hello\n".chars().count());
    }

    #[test]
    fn backspace_mid_body_not_handled() {
        let mut s = FlowState::from_plain("1. hello");
        s.cursor = "1. hel".chars().count();
        assert!(!handle_backspace_list(&mut s));
        assert_eq!(s.text, "1. hello");
    }

    #[test]
    fn enter_on_bullet_plain_no_inherit() {
        let mut s = FlowState::from_plain("• one");
        s.cursor = s.text.chars().count();
        handle_enter(&mut s);
        assert_eq!(s.text, "• one\n");
        assert_eq!(s.cursor, s.text.chars().count());
    }

    #[test]
    fn move_vert_logical_keeps_body_column_across_list() {
        let t = "1. aa\n2. bbbb\n3. c";
        let mut col = None;
        // 在「2. bb|bb」
        let cur = "1. aa\n2. bb".chars().count();
        let up = move_vert_logical(t, cur, -1, &mut col);
        assert_eq!(up, "1. aa".chars().count()); // 列 2 → 行1 正文末
        let down = move_vert_logical(t, up, 1, &mut col);
        assert_eq!(down, cur);
        let down2 = move_vert_logical(t, down, 1, &mut col);
        // 列 2，行3 正文只有 1 字符 → 夹到末尾
        assert_eq!(down2, t.chars().count());
    }

}
