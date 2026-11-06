//! 标签页模型与语言推断：资源管理器直接展示磁盘目录，工程里不再内置示例文件。

use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

use gpui_kit::SharedString;

use crate::encoding::FileEncoding;

/// 「转到文件」最多索引多少个文件，避免大仓库卡住界面。
pub const MAX_GOTO_FILES: usize = 5000;

/// 递归时跳过的目录（大小写不敏感）：构建产物、依赖缓存与版本库元数据。
const SKIPPED_DIRS: &[&str] = &["target", "node_modules", "vendor", ".git"];

/// 递归收集 `root` 下的文件路径，写入 `files`；`seen` 用来去重（外部可先放已打开/最近的文件）。
pub fn collect_files(root: &Path, files: &mut Vec<String>, seen: &mut HashSet<String>) {
    collect_files_inner(root, 0, files, seen);
}

fn collect_files_inner(
    dir: &Path,
    depth: usize,
    files: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    const MAX_DEPTH: usize = 12;
    if depth > MAX_DEPTH || files.len() >= MAX_GOTO_FILES {
        return;
    }
    let Ok(reader) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = reader.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if files.len() >= MAX_GOTO_FILES {
            return;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        // 不跟随符号链接，避免目录环。
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_dir() {
            if name.starts_with('.')
                || SKIPPED_DIRS
                    .iter()
                    .any(|skip| name.eq_ignore_ascii_case(skip))
            {
                continue;
            }
            collect_files_inner(&entry.path(), depth + 1, files, seen);
        } else if file_type.is_file() {
            let path = entry.path().to_string_lossy().into_owned();
            if seen.insert(path.clone()) {
                files.push(path);
            }
        }
    }
}

/// 读文件最后修改时间；读不到（文件不存在、无权限等）时为 `None`。
pub fn disk_stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// 磁盘上的时间戳与记录的不同，说明文件被外部改过。
///
/// 任何一边缺失都当作「没变」：新打开的文件、刚被删掉的文件都不该弹提示。
pub fn stamp_differs(known: Option<SystemTime>, current: Option<SystemTime>) -> bool {
    match (known, current) {
        (Some(known), Some(current)) => known != current,
        _ => false,
    }
}

/// 文件名模糊匹配得分；不匹配返回 `None`。
///
/// 优先级：完全相等 > 前缀 > 包含 > 子序列。查询串忽略大小写；
/// 查询串里带 `/` 或 `\` 时按完整路径匹配，否则只比文件名。
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let query = query.trim().to_lowercase().replace('\\', "/");
    if query.is_empty() {
        return None;
    }
    let full = candidate.to_lowercase().replace('\\', "/");
    let name = Path::new(candidate)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase().replace('\\', "/"))
        .unwrap_or_else(|| full.clone());
    let (haystack, path_mode) = if query.contains(['/', '\\']) {
        (full.as_str(), true)
    } else {
        (name.as_str(), false)
    };

    if haystack == query {
        return Some(10_000);
    }
    if haystack.starts_with(&query) {
        let penalty = (haystack.len() - query.len()) as i32;
        return Some(8_000 - penalty.min(1_000) - if path_mode { 500 } else { 0 });
    }
    if let Some(position) = haystack.find(&query) {
        let penalty = (position as i32) * 10 + (haystack.len() - query.len()) as i32;
        return Some(6_000 - penalty.min(2_000) - if path_mode { 500 } else { 0 });
    }
    subsequence_score(haystack, &query).map(|score| score - if path_mode { 500 } else { 0 })
}

/// 子序列匹配（字符按顺序出现即可）；空查询或匹配不上返回 `None`。
fn subsequence_score(haystack: &str, query: &str) -> Option<i32> {
    let mut chars = haystack.chars();
    let mut matched = 0i32;
    let mut gaps = 0i32;
    for expected in query.chars() {
        let mut skipped = 0i32;
        loop {
            match chars.next() {
                Some(ch) if ch == expected => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
        matched += 1;
    }
    Some(3_000 + matched * 20 - gaps * 10)
}

/// 在候选里挑最匹配的一个；同分时选路径更短的。
pub fn best_match<'a>(query: &str, candidates: &'a [String]) -> Option<&'a String> {
    candidates
        .iter()
        .filter_map(|candidate| fuzzy_score(query, candidate).map(|score| (score, candidate)))
        .max_by(|(left_score, left), (right_score, right)| {
            left_score
                .cmp(right_score)
                .then_with(|| right.len().cmp(&left.len()))
        })
        .map(|(_, candidate)| candidate)
}

/// 路径改名/删除后改写记录里的路径。
///
/// `value` 与 `old` 完全相同，或者位于 `old` 目录下时返回新路径；`new` 传空串
/// 表示删除，调用方可以据此判断「这条记录是不是被删掉了」。
pub fn reprefix_path(value: &str, old: &str, new: &str) -> Option<String> {
    if value == old {
        return Some(new.to_string());
    }
    let rest = value.strip_prefix(old)?;
    // `C:\a` 不能匹配 `C:\ab`：前缀后面必须是路径分隔符。
    if !rest.starts_with(['\\', '/']) {
        return None;
    }
    Some(format!("{new}{rest}"))
}

/// 标签栏中已打开的文件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFile {
    pub id: SharedString,
    pub name: SharedString,
    /// 文件自身的语言，用于“自动”语法分析。
    pub language: SharedString,
    /// 打开时探测到的编码，保存时按它写回。
    pub encoding: FileEncoding,
}

/// 语言名对应的展示文字。
pub fn language_label(language: &str) -> &'static str {
    match language {
        "rust" => "Rust",
        "vb" | "vb6" | "vba" => "Visual Basic",
        "json" => "JSON",
        "text" | "plaintext" | "plain" => "纯文本",
        _ => "纯文本",
    }
}

/// 按扩展名推断磁盘文件的高亮语言。
pub fn language_for_path(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("rs") => "rust",
        Some("vb" | "vbs" | "bas" | "cls" | "frm") => "vb",
        Some("json") => "json",
        _ => "text",
    }
}

