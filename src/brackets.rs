//! 括号配对：光标贴着一个括号时，找出与它配对的那一个，供高亮使用。
//!
//! 只处理 `()` `[]` `{}`。字符串里的括号不算，VB 的 `'` 注释里也不算
//! （按行判断，VB 的字符串不会跨行）。

/// 光标左右两侧的括号候选：`(字节偏移, 字符)`。
fn candidates(text: &str, cursor: usize) -> [Option<(usize, char)>; 2] {
    let before = text[..cursor]
        .char_indices()
        .next_back()
        .filter(|(index, ch)| index + ch.len_utf8() == cursor);
    let after = text[cursor..]
        .char_indices()
        .next()
        .map(|(index, ch)| (cursor + index, ch));
    [before, after]
}

fn closer_for(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn opener_for(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

/// 一行的有效代码结束位置：`'` 注释之前（不含注释）；字符串里的 `'` 不算。
fn code_end(line: &str) -> usize {
    let mut in_string = false;
    for (index, ch) in line.char_indices() {
        if ch == '"' {
            in_string = !in_string;
        } else if ch == '\'' && !in_string {
            return index;
        }
    }
    line.len()
}

/// 每个字节是否落在字符串字面量里（用于反向扫描时跳过字符串）。
fn string_mask(line: &str) -> Vec<bool> {
    let mut mask = vec![false; line.len()];
    let mut in_string = false;
    for (index, ch) in line.char_indices() {
        if ch == '"' {
            in_string = !in_string;
            mask[index] = true;
            continue;
        }
        mask[index] = in_string;
    }
    mask
}

/// 光标贴着括号时返回配对的两个括号的字节偏移：`(左括号, 右括号)`。
///
/// 光标在开括号右边或闭括号左边都能匹配；找不到配对时返回 `None`。
pub fn matching_pair(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let cursor = cursor.min(text.len());
    for (index, ch) in candidates(text, cursor).into_iter().flatten() {
        if let Some(close) = closer_for(ch)
            && let Some(end) = scan_forward(text, index, ch, close)
        {
            return Some((index, end));
        }
        if let Some(open) = opener_for(ch)
            && let Some(start) = scan_backward(text, index, open, ch)
        {
            return Some((start, index));
        }
    }
    None
}

/// 从开括号往后找配对的闭括号。
fn scan_forward(text: &str, from: usize, open: char, close: char) -> Option<usize> {
    let line_start = text[..from].rfind('\n').map_or(0, |index| index + 1);
    let mut depth = 0usize;
    let mut position = line_start;
    for line in text[line_start..].split_inclusive('\n') {
        let end = code_end(line);
        let mut in_string = false;
        for (offset, ch) in line.char_indices() {
            if offset >= end {
                break;
            }
            let index = position + offset;
            if ch == '"' {
                in_string = !in_string;
                continue;
            }
            if in_string || index <= from {
                continue;
            }
            if ch == open {
                depth += 1;
            } else if ch == close {
                if depth == 0 {
                    return Some(index);
                }
                depth -= 1;
            }
        }
        position += line.len();
    }
    None
}

/// 从闭括号往前找配对的开括号。
fn scan_backward(text: &str, from: usize, open: char, close: char) -> Option<usize> {
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut position = 0usize;
    for line in text.split_inclusive('\n') {
        lines.push((position, line));
        position += line.len();
        if position > from {
            break;
        }
    }

    let mut depth = 0usize;
    for (line_start, line) in lines.into_iter().rev() {
        let limit = (from - line_start).min(line.len()).min(code_end(line));
        let mask = string_mask(line);
        for (offset, ch) in line[..limit].char_indices().rev() {
            if mask[offset] {
                continue;
            }
            let index = line_start + offset;
            if ch == close {
                depth += 1;
            } else if ch == open {
                if depth == 0 {
                    return Some(index);
                }
                depth -= 1;
            }
        }
    }
    None
}
