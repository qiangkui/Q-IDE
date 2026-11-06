//! 编辑器的文本命令：整行上移/下移、切换注释。
//!
//! 全部是纯字符串运算（偏移都是 UTF-8 字节偏移，与 `InputState::selected_range` 一致），
//! 因此可以脱离界面直接单测。

use std::ops::Range;

/// 一次编辑的结果：新的全文与新的选区（字节偏移）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub text: String,
    pub selection: Range<usize>,
}

/// 某语言的行注释符号；纯文本等没有注释概念的语言返回 `None`。
pub fn comment_token(language: &str) -> Option<&'static str> {
    match language {
        "vb" => Some("'"),
        "rust" | "json" => Some("//"),
        _ => None,
    }
}

/// 每行起始字节偏移（末尾换行后多出来的空行也会有一项）。
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

/// 选区覆盖的整行下标范围；选区终点正落在行首时不算那一行。
fn selected_lines(starts: &[usize], selection: &Range<usize>, len: usize) -> (usize, usize) {
    let start = selection.start.min(len);
    let end = selection.end.min(len);
    let first = starts
        .partition_point(|line_start| *line_start <= start)
        .saturating_sub(1);
    let mut last = starts
        .partition_point(|line_start| *line_start <= end)
        .saturating_sub(1);
    if last > first && end == starts[last] {
        last -= 1;
    }
    (first, last)
}

fn shift(range: &Range<usize>, delta: isize) -> Range<usize> {
    let apply = |offset: usize| (offset as isize + delta).max(0) as usize;
    apply(range.start)..apply(range.end)
}

/// 把选区所在的整行（可以多行）上移或下移一行；到文件边界时返回 `None`。
///
/// 最后一行的处理与编辑器一致：跨过文件末尾那行（可能没有换行符）时，
/// 会把换行符挪到新的行尾，避免两行粘在一起。
pub fn move_lines(text: &str, selection: Range<usize>, up: bool) -> Option<TextEdit> {
    let starts = line_starts(text);
    let (first, last) = selected_lines(&starts, &selection, text.len());
    let block_start = starts[first];
    let block_end = starts.get(last + 1).copied().unwrap_or(text.len());
    let block = &text[block_start..block_end];

    let (block_text, neighbor_text, neighbor_start) = if up {
        // 上面没有行了：不动。
        let prev_start = *starts.get(first.checked_sub(1)?)?;
        let prev = &text[prev_start..block_start];
        let (block_text, prev_text) = if !block.ends_with('\n') && prev.ends_with('\n') {
            (format!("{block}\n"), prev[..prev.len() - 1].to_string())
        } else {
            (block.to_string(), prev.to_string())
        };
        (block_text, prev_text, prev_start)
    } else {
        // 已经是最后一行：不动。
        if block_end >= text.len() {
            return None;
        }
        let next_end = starts.get(last + 2).copied().unwrap_or(text.len());
        let next = &text[block_end..next_end];
        let (block_text, next_text) = if !next.ends_with('\n') && block.ends_with('\n') {
            (block[..block.len() - 1].to_string(), format!("{next}\n"))
        } else {
            (block.to_string(), next.to_string())
        };
        (block_text, next_text, block_end)
    };

    let mut new_text = String::with_capacity(text.len());
    if up {
        new_text.push_str(&text[..neighbor_start]);
        let new_block_start = new_text.len();
        new_text.push_str(&block_text);
        new_text.push_str(&neighbor_text);
        new_text.push_str(&text[block_end..]);
        let delta = new_block_start as isize - block_start as isize;
        Some(TextEdit {
            text: new_text,
            selection: shift(&selection, delta),
        })
    } else {
        new_text.push_str(&text[..block_start]);
        new_text.push_str(&neighbor_text);
        let new_block_start = new_text.len();
        new_text.push_str(&block_text);
        new_text.push_str(&text[starts.get(last + 2).copied().unwrap_or(text.len())..]);
        let delta = new_block_start as isize - block_start as isize;
        Some(TextEdit {
            text: new_text,
            selection: shift(&selection, delta),
        })
    }
}

/// 切换选区所在整行的行注释：全都注释了就去掉，否则逐行加上。
///
/// 空行不参与判断也不加注释；注释符号加在行首缩进之后，并跟一个空格。
pub fn toggle_comment(text: &str, selection: Range<usize>, token: &str) -> Option<TextEdit> {
    if token.is_empty() {
        return None;
    }
    let starts = line_starts(text);
    let (first, last) = selected_lines(&starts, &selection, text.len());
    let lines: Vec<(usize, usize)> = (first..=last)
        .map(|index| (starts[index], starts.get(index + 1).copied().unwrap_or(text.len())))
        .collect();

    let indent_len = |line: &str| line.len() - line.trim_start_matches([' ', '\t']).len();
    let non_empty = lines
        .iter()
        .filter(|&&(start, end)| !text[start..end].trim().is_empty());
    let mut all_commented = true;
    let mut any_line = false;
    for &(start, end) in non_empty {
        any_line = true;
        let line = &text[start..end];
        let indent = indent_len(line);
        if !line[indent..].starts_with(token) {
            all_commented = false;
            break;
        }
    }
    if !any_line {
        return None;
    }

    let marker = format!("{token} ");
    let mut out = String::with_capacity(text.len() + marker.len() * lines.len());
    out.push_str(&text[..starts[first]]);
    // 每行：改动发生在行内第几字节、行长变化了多少。
    let mut infos: Vec<(usize, isize)> = Vec::with_capacity(lines.len());
    let mut new_line_starts: Vec<usize> = Vec::with_capacity(lines.len());
    for (start, end) in &lines {
        let line = &text[*start..*end];
        let indent = indent_len(line);
        let empty = line.trim().is_empty();
        let new_line = if all_commented && !empty {
            let rest = &line[indent + token.len()..];
            format!("{}{}", &line[..indent], rest.strip_prefix(' ').unwrap_or(rest))
        } else if !all_commented && !empty {
            format!("{}{}{}", &line[..indent], marker, &line[indent..])
        } else {
            line.to_string()
        };
        new_line_starts.push(out.len());
        infos.push((indent, new_line.len() as isize - line.len() as isize));
        out.push_str(&new_line);
    }
    let tail_start = lines.last().map(|(_, end)| *end).unwrap_or(text.len());
    out.push_str(&text[tail_start..]);

    let map = |offset: usize| -> usize {
        let offset = offset.clamp(starts[first], tail_start);
        let index = starts[first..=last]
            .partition_point(|line_start| *line_start <= offset)
            .saturating_sub(1)
            .min(infos.len() - 1);
        let (insert_at, delta) = infos[index];
        let column = offset - starts[first + index];
        let new_column = if column > insert_at {
            ((column as isize + delta).max(insert_at as isize)) as usize
        } else {
            column
        };
        new_line_starts[index] + new_column
    };
    let selection = map(selection.start)..map(selection.end);
    Some(TextEdit {
        text: out,
        selection,
    })
}
