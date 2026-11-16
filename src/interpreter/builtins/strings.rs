//! 字符串比较与查找：`StrComp`、`Filter`、`Split`、`InStr`/`InStrRev`、`Replace`。
//!
//! 从 `builtins.rs` 按职责拆出来；`call()` 仍在 `super` 里按名字分派。

use super::*;

/// `StrComp(a, b[, compare])`：`compare` 省略或为 -1 时按 `Option Compare`，`1` 忽略大小写。
pub(super) fn str_comp_value(args: &[Value], compare_text: bool) -> Result<Value, String> {
    let first = args.first().ok_or("`StrComp` 缺少第 1 个参数")?;
    let second = args.get(1).ok_or("`StrComp` 缺少第 2 个参数")?;
    if matches!(first, Value::Null) || matches!(second, Value::Null) {
        return Ok(Value::Null);
    }
    let compare = args.get(2).map(Value::as_int).transpose()?.unwrap_or(-1);
    let ignore_case = match compare {
        -1 => compare_text,
        1 => true,
        _ => false,
    };
    let left = first.to_display();
    let right = second.to_display();
    let ordering = if ignore_case {
        left.to_lowercase().cmp(&right.to_lowercase())
    } else {
        left.cmp(&right)
    };
    Ok(Value::Int(match ordering {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

/// `Filter(数组, 子串[, include[, compare]])`：返回 0 基的新数组。
pub(super) fn filter_value(args: &[Value], compare_text: bool) -> Result<Value, String> {
    let array = args
        .first()
        .ok_or("`Filter` 缺少第 1 个参数")?
        .as_array()
        .map_err(|message| format!("`Filter` {message}"))?;
    let needle = args.get(1).ok_or("`Filter` 缺少第 2 个参数")?.to_display();
    let include = match args.get(2) {
        Some(value) => value.to_bool()?,
        None => true,
    };
    let compare = args.get(3).map(Value::as_int).transpose()?.unwrap_or(-1);
    // -1 = 按 `Option Compare`；0 = 二进制；1 = 忽略大小写。
    let ignore_case = match compare {
        -1 => compare_text,
        1 => true,
        _ => false,
    };
    let needle = if ignore_case {
        needle.to_lowercase()
    } else {
        needle
    };
    let mut kept = Vec::new();
    let array = array.borrow();
    for cell in array.cells() {
        let value = cell.borrow();
        let text = value.to_display();
        let haystack = if ignore_case {
            text.to_lowercase()
        } else {
            text
        };
        if haystack.contains(&needle) == include {
            // 只复制留下的元素，避免给整份数组做一次深拷贝。
            kept.push((*value).clone());
        }
    }
    drop(array);
    Ok(Value::array(kept))
}

/// `Split` 的分段规则：源串为空时返回空数组（VB6 里 `UBound(Split(""))` 是 -1），
/// 分隔符为空或 `limit` 为 1 时整串作为唯一元素。
pub(super) fn split_parts(source: &str, delimiter: &str, limit: usize) -> Vec<Value> {
    if source.is_empty() {
        return Vec::new();
    }
    if delimiter.is_empty() || limit == 1 {
        return vec![Value::Str(source.to_string())];
    }
    let parts: Vec<&str> = if limit == 0 {
        source.split(delimiter).collect()
    } else {
        source.splitn(limit, delimiter).collect()
    };
    parts
        .into_iter()
        .map(|part| Value::Str(part.to_string()))
        .collect()
}

/// VB 的比较方式参数：0 = 二进制、1 = 文本（不区分大小写）、2 = 数据库。
///
/// 解释器没有区域/排序规则，`2` 按二进制处理。
pub(super) fn compare_is_text(value: Option<&Value>) -> Result<bool, String> {
    match value {
        None | Some(Value::Empty) => Ok(false),
        Some(value) => Ok(value.as_int()? != 0),
    }
}

pub(super) fn chars_match_at(
    haystack: &[char],
    needle: &[char],
    index: usize,
    ignore_case: bool,
) -> bool {
    if index + needle.len() > haystack.len() {
        return false;
    }
    needle.iter().enumerate().all(|(offset, expected)| {
        let actual = haystack[index + offset];
        if ignore_case {
            actual == *expected || actual.to_lowercase().eq(expected.to_lowercase())
        } else {
            actual == *expected
        }
    })
}

/// 从 `begin`（0 基字符下标）开始找子串，返回匹配的 0 基下标。
pub(super) fn find_substring(
    haystack: &[char],
    needle: &[char],
    begin: usize,
    ignore_case: bool,
) -> Option<usize> {
    if needle.is_empty() || begin >= haystack.len() {
        return None;
    }
    (begin..=haystack.len().saturating_sub(needle.len()))
        .find(|index| chars_match_at(haystack, needle, *index, ignore_case))
}

/// `Replace`：从 `start`（1 基字符位置）起替换，`count` 为负表示全部替换。
///
/// VB6 的返回值从 `start` 开始，`start` 之前的部分不会保留；空 `find` 不做任何替换。
pub(super) fn replace_substring(
    source: &str,
    find: &str,
    replacement: &str,
    start: usize,
    count: i64,
    ignore_case: bool,
) -> String {
    let find: Vec<char> = find.chars().collect();
    if find.is_empty() {
        return source.to_string();
    }
    let chars: Vec<char> = source.chars().collect();
    let mut output = String::with_capacity(source.len());
    let mut index = (start - 1).min(chars.len());
    let mut replaced = 0i64;
    while index < chars.len() {
        if (count < 0 || replaced < count) && chars_match_at(&chars, &find, index, ignore_case) {
            output.push_str(replacement);
            index += find.len();
            replaced += 1;
            continue;
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}
