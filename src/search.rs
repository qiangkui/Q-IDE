//! 项目内文本搜索：匹配/扫描的纯逻辑，以及侧栏搜索面板的渲染。
//!
//! 搜索是同步执行的（结果直接回填到面板），因此用 [`MAX_SEARCH_HITS`] 与
//! [`MAX_SEARCH_BYTES`] 把最坏情况限制住：命中够多就停、文件太大就跳过。

use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::DragPanel;
use gpui_kit::component::*;
use gpui_kit::prelude::{
    FluentBuilder as _, InteractiveElement as _, StatefulInteractiveElement as _,
};
use gpui_kit::*;
use regex::{Regex, RegexBuilder};

use crate::outline::{ClosePanelAction, DragStartAction};
use crate::theme::{self as ui_theme, Palette};

/// 一次搜索最多记多少处命中：列表里再多也没法看。
pub const MAX_SEARCH_HITS: usize = 500;

/// 超过这个大小的文件直接跳过（多半是生成物或数据文件）。
pub const MAX_SEARCH_BYTES: u64 = 2 * 1024 * 1024;

/// 一次搜索最多读多少字节：搜索在界面线程同步执行，必须有总预算兜底。
pub const MAX_SEARCH_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

/// 一次搜索的条件。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    /// 区分大小写。
    pub case_sensitive: bool,
    /// 全词匹配：命中前后不能再是字母、数字或 `_`。
    pub whole_word: bool,
    /// 把 `text` 当正则表达式（`regex` crate 语法，替换串里可用 `$1` 引用分组）。
    pub regex: bool,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
        }
    }

    /// 有没有实际要搜的内容（只输空格不算）。
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// 正则模式下把用户输入编译成匹配器；非正则模式返回 `Ok(None)`，正则写错返回错误说明。
    ///
    /// `whole_word` 在正则模式下的语义是「整个模式再加一层词边界」，即 `\b(?:用户的正则)\b`。
    pub fn compile(&self) -> Result<Option<Regex>, String> {
        if !self.regex {
            return Ok(None);
        }
        let pattern = if self.whole_word {
            format!(r"\b(?:{})\b", self.text)
        } else {
            self.text.clone()
        };
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive)
            .build()
            .map(Some)
            .map_err(|error| error.to_string())
    }
}

/// 可切换的搜索选项。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchOption {
    CaseSensitive,
    WholeWord,
    Regex,
}

impl SearchOption {
    pub fn label(self) -> &'static str {
        match self {
            Self::CaseSensitive => "Aa",
            Self::WholeWord => "全词",
            Self::Regex => ".*",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::CaseSensitive => "区分大小写",
            Self::WholeWord => "全词匹配",
            Self::Regex => "正则表达式（替换串里可用 $1 引用分组）",
        }
    }
}

/// 一处命中：行号、列号都从 1 开始（与编辑器状态栏一致）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub path: String,
    pub line: usize,
    pub column: usize,
    /// 命中本身的字符数（正则模式下一行里长度不固定，高亮按它取）。
    pub length: usize,
    /// 命中所在行的原文，列表里作为上下文显示。
    pub preview: String,
}

/// 一次搜索的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// 真正读过内容的文件数。
    pub files: usize,
    /// 命中已达上限，后面的文件没有再搜。
    pub truncated: bool,
    /// 跳过（过大、二进制、读不了）的文件数。注意 GBK/UTF-16 源码**不再跳过**，会解码后照常搜索。
    pub skipped: usize,
    /// 用户又发起了一次搜索，这次被提前取消。
    pub cancelled: bool,
    /// 正则模式下的表达式写错了（面板显示「正则无效」）。
    pub invalid_pattern: bool,
}

/// 后台搜索的进度与取消标志：界面按它画进度，发起新搜索时把旧的置为取消。
#[derive(Debug, Default)]
pub struct SearchProgress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

impl SearchProgress {
    pub fn new(total: usize) -> Self {
        Self {
            done: AtomicUsize::new(0),
            total: AtomicUsize::new(total),
            cancel: AtomicBool::new(false),
        }
    }

    /// 已处理的文件数 / 候选总数。
    pub fn snapshot(&self) -> (usize, usize) {
        (
            self.done.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn advance(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }
}

impl SearchResults {
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// 命中的文件数：命中按文件顺序排列，相邻的同一路径只算一次。
    pub fn file_count(&self) -> usize {
        let mut count = 0;
        let mut last: Option<&str> = None;
        for hit in &self.hits {
            if last != Some(hit.path.as_str()) {
                count += 1;
                last = Some(hit.path.as_str());
            }
        }
        count
    }
}

/// 在 `text` 里找 `query` 的命中并追加到 `hits`；每行最多记第一处，
/// 这样列表可以按「一行一条」展示。返回是否已经达到 [`MAX_SEARCH_HITS`]。
///
/// 正则写错时不产生命中（`search_files` 会在结果里标 `invalid_pattern`）。
/// 生产路径走 [`find_in_text_with`]，正则只编译一次；这个入口是给「单份文本」和测试用的。
#[cfg_attr(not(test), allow(dead_code))]
pub fn find_in_text(
    text: &str,
    query: &SearchQuery,
    path: &str,
    hits: &mut Vec<SearchHit>,
) -> bool {
    let regex = query.compile().ok().flatten();
    let needle: Vec<char> = if query.regex {
        Vec::new()
    } else {
        query.text.chars().collect()
    };
    find_in_text_with(text, query, regex.as_ref(), &needle, path, hits)
}

/// [`find_in_text`] 的内核：正则与关键字字符表都只准备一次，整份文本共用。
fn find_in_text_with(
    text: &str,
    query: &SearchQuery,
    regex: Option<&Regex>,
    needle: &[char],
    path: &str,
    hits: &mut Vec<SearchHit>,
) -> bool {
    if query.text.is_empty() {
        return false;
    }
    for (index, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some((column, length)) = match_span(line, query, regex, needle) {
            hits.push(SearchHit {
                path: path.to_string(),
                line: index + 1,
                column: column + 1,
                length,
                preview: line.to_string(),
            });
            if hits.len() >= MAX_SEARCH_HITS {
                return true;
            }
        }
    }
    false
}

/// 直接同步搜索一批文件（小列表或测试用）。
#[cfg(test)]
pub fn search_files(paths: &[String], query: &SearchQuery) -> SearchResults {
    search_files_inner(paths, query, MAX_SEARCH_TOTAL_BYTES, None)
}

/// 同 [`search_files`]，但可以指定读取预算（测试用）。
#[cfg(test)]
pub fn search_files_with_budget(
    paths: &[String],
    query: &SearchQuery,
    budget: u64,
) -> SearchResults {
    search_files_inner(paths, query, budget, None)
}

/// 后台线程上的搜索：每处理完一个文件更新一次进度，`progress` 被取消时提前返回。
pub fn search_files_with_progress(
    paths: &[String],
    query: &SearchQuery,
    progress: &SearchProgress,
) -> SearchResults {
    search_files_inner(paths, query, MAX_SEARCH_TOTAL_BYTES, Some(progress))
}

fn search_files_inner(
    paths: &[String],
    query: &SearchQuery,
    budget: u64,
    progress: Option<&SearchProgress>,
) -> SearchResults {
    let mut results = SearchResults::default();
    if query.is_empty() {
        return results;
    }
    // 正则与关键字字符表都只准备一次；写错就把结果标成 invalid_pattern（面板显示「正则无效」）。
    let regex = match query.compile() {
        Ok(regex) => regex,
        Err(_) => {
            results.invalid_pattern = true;
            return results;
        }
    };
    let needle: Vec<char> = if query.regex {
        Vec::new()
    } else {
        query.text.chars().collect()
    };
    let mut budget = budget;
    for path in paths {
        if progress.is_some_and(SearchProgress::is_cancelled) {
            results.cancelled = true;
            break;
        }
        if results.hits.len() >= MAX_SEARCH_HITS || budget == 0 {
            results.truncated = true;
            break;
        }
        let Some(text) = read_text(path) else {
            results.skipped += 1;
            if let Some(progress) = progress {
                progress.advance();
            }
            continue;
        };
        results.files += 1;
        budget = budget.saturating_sub(text.len() as u64);
        let full = find_in_text_with(
            &text,
            query,
            regex.as_ref(),
            &needle,
            path,
            &mut results.hits,
        );
        if let Some(progress) = progress {
            progress.advance();
        }
        if full {
            results.truncated = true;
            break;
        }
    }
    results
}

/// 读出可搜索的文本：过大、含 NUL 字节（二进制）或读不了时返回 `None`。
///
/// **不是 UTF-8 的文件照样搜**：中文 VB6 工程的源码常常是 GBK（或带 BOM 的 UTF-16），
/// 这里用与「打开文件」同一套探测（`encoding::decode`）解码，否则这些工程整片搜不到。
fn read_text(path: &str) -> Option<String> {
    read_text_with_encoding(path).map(|(text, _)| text)
}

/// [`read_text`] 的完整版：同时给回探测到的编码，替换写回时按原编码写，不然会把 GBK 文件写成 UTF-8。
fn read_text_with_encoding(path: &str) -> Option<(String, crate::encoding::FileEncoding)> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SEARCH_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    // 带 BOM 的 UTF-16 正文里天然有 0 字节，所以先看 BOM 再判「二进制」。
    let has_utf8_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let has_bom =
        has_utf8_bom || bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]);
    if !has_bom && bytes.iter().take(8192).any(|byte| *byte == 0) {
        return None;
    }
    // 无 BOM 的 UTF-8 走快路径；带 BOM 的必须走 `decode`，否则写回时会把 BOM 丢掉。
    if !has_bom && let Ok(text) = std::str::from_utf8(&bytes) {
        return Some((text.to_string(), crate::encoding::FileEncoding::Utf8));
    }
    let decoded = crate::encoding::decode(&bytes);
    Some((decoded.text, decoded.encoding))
}

/// 一行里第一处命中的 (起始字符下标, 长度字符数)。
///
/// `regex` 是正则模式预先编译好的匹配器；`needle` 是字面量模式的关键字字符表（每次搜索只建一次，
/// 别再按行 `chars().collect()`——大文件里那是几十万次分配）。
fn match_span(
    line: &str,
    query: &SearchQuery,
    regex: Option<&Regex>,
    needle: &[char],
) -> Option<(usize, usize)> {
    if let Some(regex) = regex {
        let hit = regex.find(line)?;
        // 列号按字符算（与状态栏、编辑器一致），正则给的是字节下标。
        let start = line[..hit.start()].chars().count();
        let length = line[hit.start()..hit.end()].chars().count();
        return Some((start, length));
    }
    if needle.is_empty() {
        return None;
    }
    let chars: Vec<char> = line.chars().collect();
    if needle.len() > chars.len() {
        return None;
    }
    for start in 0..=chars.len() - needle.len() {
        if !match_at(&chars, start, needle, query) {
            continue;
        }
        if query.whole_word && !is_word_boundary(&chars, start, needle.len()) {
            continue;
        }
        return Some((start, needle.len()));
    }
    None
}

/// `chars[start..start + needle.len()]` 是否就是一次命中（不考虑全词边界）。
fn match_at(chars: &[char], start: usize, needle: &[char], query: &SearchQuery) -> bool {
    needle
        .iter()
        .enumerate()
        .all(|(offset, expected)| char_eq(chars[start + offset], *expected, query.case_sensitive))
}

/// 把一行里的命中全部替换掉，返回新行与替换次数。
fn replace_line(
    line: &str,
    query: &SearchQuery,
    regex: Option<&Regex>,
    needle: &[char],
    replacement: &str,
) -> (String, usize) {
    if let Some(regex) = regex {
        let count = regex.find_iter(line).count();
        if count == 0 {
            return (line.to_string(), 0);
        }
        // `$1` / `${name}` 分组引用由 `regex` 自己展开。
        return (regex.replace_all(line, replacement).into_owned(), count);
    }
    let chars: Vec<char> = line.chars().collect();
    if needle.is_empty() || needle.len() > chars.len() {
        return (line.to_string(), 0);
    }
    let mut out = String::with_capacity(line.len());
    let mut count = 0;
    let mut index = 0;
    while index < chars.len() {
        if index + needle.len() <= chars.len()
            && match_at(&chars, index, needle, query)
            && (!query.whole_word || is_word_boundary(&chars, index, needle.len()))
        {
            out.push_str(replacement);
            index += needle.len();
            count += 1;
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    (out, count)
}

/// 把一行拆成（正文, 行尾）：`\r\n` / `\n` 里的换行符属于行尾，不参与匹配替换。
fn split_line_ending(line: &str) -> (&str, &str) {
    let (raw_body, ending) = match line.strip_suffix('\n') {
        Some(rest) => (rest, "\n"),
        None => (line, ""),
    };
    match raw_body.strip_suffix('\r') {
        Some(body) if !ending.is_empty() => (body, "\r\n"),
        _ => (raw_body, ending),
    }
}

/// 把 `text` 里所有命中替换成 `replacement`，返回新文本与替换次数。
///
/// 匹配规则与搜索一致（区分大小写、全词、正则），逐行处理，行尾（`\n` / `\r\n`）原样保留。
/// 正则写错时原样返回、次数为 0（面板会另外提示「正则无效」）。
pub fn replace_in_text(text: &str, query: &SearchQuery, replacement: &str) -> (String, usize) {
    if query.text.is_empty() {
        return (text.to_string(), 0);
    }
    let regex = match query.compile() {
        Ok(regex) => regex,
        Err(_) => return (text.to_string(), 0),
    };
    // 关键字字符表只建一次（逐行再 `chars().collect()` 是大文件里的主要开销）。
    let needle: Vec<char> = if query.regex {
        Vec::new()
    } else {
        query.text.chars().collect()
    };
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    for line in text.split_inclusive('\n') {
        let (body, ending) = split_line_ending(line);
        let (replaced, hits) = replace_line(body, query, regex.as_ref(), &needle, replacement);
        out.push_str(&replaced);
        out.push_str(ending);
        count += hits;
    }
    (out, count)
}

/// 批量替换的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplaceOutcome {
    /// 真正写回的文件（路径, 该文件里的替换次数）。
    pub changed: Vec<(String, usize)>,
    /// 总替换次数。
    pub replacements: usize,
    /// 读不了、被跳过（过大 / 二进制）的文件数。
    pub skipped: usize,
    /// 写回失败的文件与原因。
    pub failures: Vec<String>,
}

/// 替换前的文件原始内容，供「撤销替换」写回。
///
/// 只记本轮真的要改的文件（含原来的编码与行尾），撤销时逐字节写回，
/// 不依赖「再替换回去」——正则替换不是可逆的。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplaceUndo {
    files: Vec<(String, Vec<u8>)>,
}

impl ReplaceUndo {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// 被记下来的文件路径（撤销后要刷新这些打开着的标签）。
    pub fn paths(&self) -> Vec<String> {
        self.files.iter().map(|(path, _)| path.clone()).collect()
    }
}

/// 把几个文件的当前内容读进内存（替换前调用）。
pub fn snapshot_files(paths: &[String]) -> ReplaceUndo {
    let mut undo = ReplaceUndo::default();
    for path in paths {
        match std::fs::read(path) {
            Ok(bytes) => undo.files.push((path.clone(), bytes)),
            Err(_) => continue,
        }
    }
    undo
}

/// 把 [`snapshot_files`] 记下的内容原样写回，返回（恢复的文件数, 失败原因）。
pub fn undo_replace(undo: &ReplaceUndo) -> (usize, Vec<String>) {
    let mut restored = 0;
    let mut failures = Vec::new();
    for (path, bytes) in &undo.files {
        match std::fs::write(path, bytes) {
            Ok(()) => restored += 1,
            Err(error) => failures.push(format!("{path}：{error}")),
        }
    }
    (restored, failures)
}

/// 在一批文件里替换关键字并写回。
///
/// 只处理搜索能读的文本（含 NUL 的二进制、超过 [`MAX_SEARCH_BYTES`] 的跳过）。
/// **写回按每个文件原来的编码**（`read_text_with_encoding` 探测出来的 UTF-8 / UTF-8 BOM / UTF-16 / GBK），
/// 否则把 GBK 源码写成 UTF-8 会直接毁掉文件；行尾也保持原样。
pub fn replace_in_files(
    paths: &[String],
    query: &SearchQuery,
    replacement: &str,
) -> ReplaceOutcome {
    let mut outcome = ReplaceOutcome::default();
    if query.is_empty() {
        return outcome;
    }
    for path in paths {
        let Some((text, encoding)) = read_text_with_encoding(path) else {
            outcome.skipped += 1;
            continue;
        };
        let (replaced, count) = replace_in_text(&text, query, replacement);
        if count == 0 {
            continue;
        }
        let bytes = crate::encoding::encode(&replaced, encoding);
        match std::fs::write(path, bytes) {
            Ok(()) => {
                outcome.replacements += count;
                outcome.changed.push((path.clone(), count));
            }
            Err(error) => outcome.failures.push(format!("{path}：{error}")),
        }
    }
    outcome
}

/// 只替换**选中**的命中：按（路径, 行号）定位，把那一行里的所有匹配换掉。
///
/// 搜索列表一行只记一条命中（见 [`find_in_text`]），所以「逐处勾选」的粒度是行：
/// 勾上某行就把这行里的匹配全部替换，实际次数按 [`replace_line`] 的真实命中累加。
/// 写回仍按每个文件原来的编码与行尾（与 [`replace_in_files`] 相同）。
pub fn replace_selected(
    hits: &[SearchHit],
    query: &SearchQuery,
    replacement: &str,
) -> ReplaceOutcome {
    let mut outcome = ReplaceOutcome::default();
    if query.is_empty() || hits.is_empty() {
        return outcome;
    }
    let Ok(regex) = query.compile() else {
        return outcome;
    };
    let needle: Vec<char> = if query.regex {
        Vec::new()
    } else {
        query.text.chars().collect()
    };
    // 路径按第一次出现的顺序分组（与结果列表的分组顺序一致），每个路径记一组的行号。
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for hit in hits {
        match groups
            .iter_mut()
            .find(|(path, _)| *path == hit.path.as_str())
        {
            Some((_, lines)) => {
                if !lines.contains(&hit.line) {
                    lines.push(hit.line);
                }
            }
            None => groups.push((hit.path.as_str(), vec![hit.line])),
        }
    }
    for (path, lines) in groups {
        let Some((text, encoding)) = read_text_with_encoding(path) else {
            outcome.skipped += 1;
            continue;
        };
        let mut out = String::with_capacity(text.len());
        let mut count = 0;
        for (index, line) in text.split_inclusive('\n').enumerate() {
            if !lines.contains(&(index + 1)) {
                out.push_str(line);
                continue;
            }
            let (body, ending) = split_line_ending(line);
            let (replaced, hits) = replace_line(body, query, regex.as_ref(), &needle, replacement);
            out.push_str(&replaced);
            out.push_str(ending);
            count += hits;
        }
        if count == 0 {
            continue;
        }
        let bytes = crate::encoding::encode(&out, encoding);
        match std::fs::write(path, bytes) {
            Ok(()) => {
                outcome.replacements += count;
                outcome.changed.push((path.to_string(), count));
            }
            Err(error) => outcome.failures.push(format!("{path}：{error}")),
        }
    }
    outcome
}

/// 某个文件分组里的命中下标（结果按文件分组排列，这里按下标过滤）。
pub fn file_hit_indices(hits: &[SearchHit], path: &str) -> Vec<usize> {
    hits.iter()
        .enumerate()
        .filter(|(_, hit)| hit.path == path)
        .map(|(index, _)| index)
        .collect()
}

/// 切换某个文件分组的勾选：这一组全勾着就全不勾，否则全勾。
///
/// `selected` 与 `hits` 一一对应；返回这一组切换后的勾选数。
pub fn toggle_file_selection(hits: &[SearchHit], selected: &mut [bool], path: &str) -> usize {
    let indices = file_hit_indices(hits, path);
    if indices.is_empty() {
        return 0;
    }
    let all = indices
        .iter()
        .all(|index| selected.get(*index).copied().unwrap_or(false));
    let mut chosen = 0;
    for index in indices {
        if let Some(slot) = selected.get_mut(index) {
            *slot = !all;
            if *slot {
                chosen += 1;
            }
        }
    }
    chosen
}

fn char_eq(left: char, right: char, case_sensitive: bool) -> bool {
    if case_sensitive || left == right {
        return left == right;
    }
    left.to_lowercase().eq(right.to_lowercase())
}

fn is_word_boundary(chars: &[char], start: usize, len: usize) -> bool {
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let before = start.checked_sub(1).and_then(|index| chars.get(index));
    let after = chars.get(start + len);
    !before.is_some_and(|ch| is_word(*ch)) && !after.is_some_and(|ch| is_word(*ch))
}

pub type OpenHitAction = Rc<dyn Fn(&str, usize, &mut Window, &mut App)>;
/// 勾选/取消勾选一条命中（参数是 `hits` 里的下标）。
pub type ToggleHitAction = Rc<dyn Fn(usize, &mut Window, &mut App)>;
/// 全选 / 全不选。
pub type SelectAllAction = Rc<dyn Fn(bool, &mut Window, &mut App)>;
/// 勾选/取消勾选某个文件里的全部命中（参数是文件路径）。
pub type ToggleFileAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;
pub type ToggleOptionAction = Rc<dyn Fn(SearchOption, &mut Window, &mut App)>;
pub type RunSearchAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// 搜索面板的渲染参数。
pub struct SearchRender<'a> {
    pub palette: Palette,
    pub query: &'a SearchQuery,
    /// 是否已经搜过（区分「还没搜」与「搜了没有结果」）。
    pub searched: bool,
    pub results: &'a SearchResults,
    /// 正在后台搜索时的进度：`(已处理, 总数)`。
    pub progress: Option<(usize, usize)>,
    /// 限定搜索的文件夹；`None` 表示所有候选位置。
    pub scope: Option<&'a str>,
    pub toggle_option: ToggleOptionAction,
    pub run_search: RunSearchAction,
    /// 替换成什么：点「全部替换」时执行。
    pub run_replace: RunSearchAction,
    /// 只替换勾选的命中。
    pub replace_selected: RunSearchAction,
    /// 撤销上一次替换。
    pub undo_replace: RunSearchAction,
    pub toggle_hit: ToggleHitAction,
    pub select_all: SelectAllAction,
    /// 双击文件分组标题（或点它左边的勾选框）：把这个文件里的命中全选/全不选。
    pub toggle_file: ToggleFileAction,
    /// 每条命中是否勾选，与 `results.hits` 一一对应。
    pub selected: &'a [bool],
    /// 有可以撤销的替换（面板显示「撤销替换」按钮）。
    pub undo_available: bool,
    pub choose_scope: RunSearchAction,
    pub clear_scope: RunSearchAction,
    pub close_panel: ClosePanelAction,
    pub open_hit: OpenHitAction,
}

pub fn render(
    props: SearchRender<'_>,
    drag: Option<DragPanel>,
    on_drag_start: Option<DragStartAction>,
    query_input: AnyElement,
    replace_input: AnyElement,
) -> AnyElement {
    let SearchRender {
        palette,
        query,
        searched,
        results,
        progress,
        scope,
        toggle_option,
        run_search,
        run_replace,
        replace_selected,
        undo_replace,
        toggle_hit,
        select_all,
        toggle_file,
        selected,
        undo_available,
        choose_scope,
        clear_scope,
        close_panel,
        open_hit,
    } = props;

    let title = div()
        .id("search-title")
        .flex_1()
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .child("搜索");
    let title = if let Some(drag) = drag {
        title.on_drag(drag, move |drag, offset, _, cx| {
            cx.stop_propagation();
            if let Some(on_drag_start) = on_drag_start.clone() {
                on_drag_start(cx);
            }
            drag.set_drag_offset(offset);
            drag.set_preview_size(size(px(96.0), px(30.0)));
            cx.new(|_| crate::panel_drag::PanelDragPreview::new("搜索"))
        })
    } else {
        title
    };

    let mut options = h_flex()
        .gap_1()
        .px(px(8.0))
        .pb(px(4.0))
        .bg(rgb(palette.side_panel_bg));
    for option in [
        SearchOption::CaseSensitive,
        SearchOption::WholeWord,
        SearchOption::Regex,
    ] {
        let selected = match option {
            SearchOption::CaseSensitive => query.case_sensitive,
            SearchOption::WholeWord => query.whole_word,
            SearchOption::Regex => query.regex,
        };
        let toggle = toggle_option.clone();
        options = options.child(
            Button::new(format!("search-option-{option:?}"))
                .ghost()
                .xsmall()
                .label(option.label())
                .tooltip(option.tooltip())
                .selected(selected)
                .on_click(move |_, window, cx| toggle(option, window, cx)),
        );
    }
    let rerun = run_search.clone();
    options = options.child(
        Button::new("search-run")
            .ghost()
            .xsmall()
            .icon(IconName::Search)
            .tooltip("重新搜索")
            .on_click(move |_, window, cx| rerun(window, cx)),
    );
    if !results.is_empty() {
        let picked = selected.iter().filter(|hit| **hit).count();
        options = options.child(
            Button::new("search-select-all")
                .ghost()
                .xsmall()
                .label(format!("勾选 {picked}/{}", results.hits.len()))
                .tooltip("点一下全选，再点一下全不选")
                .on_click({
                    let select_all = select_all.clone();
                    let all = picked < results.hits.len();
                    move |_, window, cx| select_all(all, window, cx)
                }),
        );
    }
    let replace = run_replace.clone();
    let replace_chosen = replace_selected.clone();
    let undo_last = undo_replace.clone();

    // 搜索范围：默认全部候选位置，可以限定到某个文件夹。
    let scope_label = match scope {
        Some(scope) => format!("范围：{}", scope_name(scope)),
        None => "范围：全部位置".to_string(),
    };
    let mut scope_row = h_flex()
        .gap_1()
        .px(px(8.0))
        .pb(px(4.0))
        .bg(rgb(palette.side_panel_bg))
        .items_center();
    scope_row = scope_row.child(
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_xs()
            .text_color(rgb(palette.muted_text))
            .child(scope_label),
    );
    let pick_scope = choose_scope.clone();
    scope_row = scope_row.child(
        Button::new("search-scope-pick")
            .ghost()
            .xsmall()
            .label("选择…")
            .tooltip("只在选中的文件夹里搜索")
            .on_click(move |_, window, cx| pick_scope(window, cx)),
    );
    if scope.is_some() {
        let reset_scope = clear_scope.clone();
        scope_row = scope_row.child(
            Button::new("search-scope-clear")
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .tooltip("搜索全部位置")
                .on_click(move |_, window, cx| reset_scope(window, cx)),
        );
    }

    let body = if results.is_empty() {
        let message = if progress.is_some() {
            "正在搜索…"
        } else if !searched || query.is_empty() {
            "输入关键字后按回车搜索"
        } else {
            "没有找到匹配内容"
        };
        div()
            .p(px(12.0))
            .text_xs()
            .text_color(rgb(palette.muted_text))
            .child(message)
            .into_any_element()
    } else {
        let mut list = v_flex().w_full();
        let mut last_path: Option<&str> = None;
        let mut group_index = 0usize;
        for (index, hit) in results.hits.iter().enumerate() {
            if last_path != Some(hit.path.as_str()) {
                if last_path.is_some() {
                    group_index += 1;
                }
                last_path = Some(hit.path.as_str());
                list = list.child(file_header(
                    hit.path.as_str(),
                    group_index,
                    palette,
                    toggle_file.clone(),
                ));
            }
            let open = open_hit.clone();
            let toggle = toggle_hit.clone();
            let checked = selected.get(index).copied().unwrap_or(false);
            list = list.child(hit_row(index, hit, checked, palette, open, toggle));
        }
        list.into_any_element()
    };

    v_flex()
        .h_full()
        .w_full()
        .bg(rgb(palette.sidebar_bg))
        .rounded(px(ui_theme::PANE_RADIUS))
        .border_1()
        .border_color(rgba(palette.border))
        .overflow_hidden()
        .child(
            h_flex()
                .h(px(ui_theme::EXPLORER_HEADER_HEIGHT))
                .px(px(16.0))
                .items_center()
                .justify_between()
                .text_xs()
                .text_color(rgb(palette.sidebar_text))
                .child(title)
                .child(close_button(close_panel, palette)),
        )
        .child(query_input)
        // 替换输入框单独占一行：面板窄的时候和按钮挤在一起会被挡住。
        .child(
            div()
                .px(px(8.0))
                .pb(px(4.0))
                .bg(rgb(palette.side_panel_bg))
                .child(replace_input),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .px(px(8.0))
                .pb(px(4.0))
                .bg(rgb(palette.side_panel_bg))
                .child(
                    Button::new("search-replace-all")
                        .ghost()
                        .xsmall()
                        .label("全部替换")
                        .tooltip("把结果里每个文件中的命中都替换掉")
                        .disabled(results.is_empty() || progress.is_some())
                        .on_click(move |_, window, cx| replace(window, cx)),
                )
                .child(
                    Button::new("search-replace-selected")
                        .ghost()
                        .xsmall()
                        .label("替换选中")
                        .tooltip("只替换勾选的命中（一行一条，勾上就把这行的匹配都换掉）")
                        .disabled(
                            results.is_empty()
                                || progress.is_some()
                                || !selected.iter().any(|hit| *hit),
                        )
                        .on_click(move |_, window, cx| replace_chosen(window, cx)),
                )
                .child(
                    Button::new("search-undo-replace")
                        .ghost()
                        .xsmall()
                        .label("撤销替换")
                        .tooltip("把上一次替换改回原文（只保留最近一次）")
                        .disabled(!undo_available)
                        .on_click(move |_, window, cx| undo_last(window, cx)),
                ),
        )
        .child(options)
        .child(scope_row)
        .child(status_row(searched, query, results, progress, palette))
        .child(
            div()
                .id("search-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .bg(rgb(palette.side_panel_bg))
                .child(body),
        )
        .into_any_element()
}

fn close_button(close_panel: ClosePanelAction, palette: Palette) -> impl IntoElement {
    div()
        .id("search-close-panel")
        .flex_shrink_0()
        .size(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(rgba(palette.hover)))
        .child(
            Icon::new(IconName::PanelLeftClose)
                .xsmall()
                .text_color(rgb(palette.sidebar_text)),
        )
        .on_click(move |_, window, cx| close_panel(window, cx))
}

/// 范围路径在按钮文字里的短名：只显示最后一级文件夹名。
fn scope_name(scope: &str) -> String {
    Path::new(scope)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| scope.to_string())
}

fn status_row(
    searched: bool,
    query: &SearchQuery,
    results: &SearchResults,
    progress: Option<(usize, usize)>,
    palette: Palette,
) -> impl IntoElement {
    let text = if let Some((done, total)) = progress {
        format!("搜索中… {done}/{total} 个文件")
    } else if !searched || query.is_empty() {
        String::new()
    } else if results.invalid_pattern {
        "正则无效".to_string()
    } else if results.is_empty() {
        "0 处结果".to_string()
    } else {
        let mut text = format!(
            "{} 处结果 · {} 个文件",
            results.hits.len(),
            results.file_count()
        );
        if results.truncated {
            text.push_str("（已达上限）");
        }
        if results.cancelled {
            text.push_str("（已取消）");
        }
        text
    };
    h_flex()
        .px(px(8.0))
        .pb(px(4.0))
        .gap_2()
        .bg(rgb(palette.side_panel_bg))
        .text_xs()
        .text_color(rgb(palette.muted_text))
        .child(text)
        .when(results.skipped > 0, |this| {
            this.child(format!("跳过 {} 个文件", results.skipped))
        })
}

/// 文件分组标题：**双击整行**把这个文件里的命中全选/全不选。
fn file_header(
    path: &str,
    index: usize,
    palette: Palette,
    toggle: ToggleFileAction,
) -> impl IntoElement {
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let target = path.to_string();
    let double = toggle;
    h_flex()
        .id(format!("search-file-{index}"))
        .w_full()
        .h(px(24.0))
        .px(px(8.0))
        .gap_1()
        .items_center()
        .bg(rgb(palette.sidebar_bg))
        // 双击整行切换这一组的勾选；单击不做事。
        .on_mouse_down(
            MouseButton::Left,
            move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                if event.click_count < 2 {
                    return;
                }
                cx.stop_propagation();
                double(target.as_str(), window, cx);
            },
        )
        .child(
            Icon::new(IconName::File)
                .xsmall()
                .text_color(rgb(palette.file)),
        )
        .child(
            div()
                .min_w_0()
                .flex_shrink_0()
                .truncate()
                .text_xs()
                .text_color(rgb(palette.sidebar_text))
                .child(name),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(path.to_string()),
        )
}

fn hit_row(
    index: usize,
    hit: &SearchHit,
    checked: bool,
    palette: Palette,
    open: OpenHitAction,
    toggle: ToggleHitAction,
) -> impl IntoElement {
    let path = hit.path.clone();
    let line = hit.line;
    h_flex()
        .id(format!("search-hit-{index}"))
        .w_full()
        .min_h(px(22.0))
        .pl(px(12.0))
        .pr(px(8.0))
        .gap_2()
        .items_center()
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(rgba(palette.hover)))
        .on_click(move |_, window, cx| open(&path, line, window, cx))
        // 勾选框：只切勾选，不跳转（所以要先 `stop_propagation`）。
        .child(hit_checkbox(index, checked, palette, toggle))
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(hit.line.to_string()),
        )
        .child(highlighted_preview(hit, palette))
}

/// 命中行勾选框的两种状态。
#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckState {
    All,
    None,
}

fn check_box(
    id: String,
    state: CheckState,
    palette: Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let border = match state {
        CheckState::None => palette.muted_text,
        CheckState::All => palette.sidebar_text,
    };
    let box_ = div()
        .id(id)
        .flex_shrink_0()
        .size(px(13.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .border_1()
        .border_color(rgb(border))
        .cursor(CursorStyle::PointingHand)
        .on_click(on_click);
    match state {
        CheckState::All => box_.child(
            Icon::new(IconName::Check)
                .xsmall()
                .text_color(rgb(palette.sidebar_text)),
        ),
        CheckState::None => box_,
    }
}

/// 命中行左侧的勾选框：控制这一条命中要不要参与「替换选中」。
fn hit_checkbox(
    index: usize,
    checked: bool,
    palette: Palette,
    toggle: ToggleHitAction,
) -> impl IntoElement {
    let state = if checked {
        CheckState::All
    } else {
        CheckState::None
    };
    check_box(
        format!("search-hit-check-{index}"),
        state,
        palette,
        move |_, window, cx| {
            cx.stop_propagation();
            toggle(index, window, cx);
        },
    )
}

/// 命中行的上下文：只截取命中附近的一段，并把关键字本身用醒目的颜色标出来。
fn highlighted_preview(hit: &SearchHit, palette: Palette) -> AnyElement {
    let preview = hit.preview.as_str();
    let chars: Vec<char> = preview.chars().collect();
    let (start, len) = (hit.column.saturating_sub(1), hit.length);
    if len == 0 || start + len > chars.len() {
        return div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_xs()
            .text_color(rgb(palette.sidebar_text))
            .child(preview.to_string())
            .into_any_element();
    }
    // 命中两侧各留一段上下文；行太长时只显示这一段。
    const CONTEXT: usize = 60;
    let from = start.saturating_sub(CONTEXT);
    let to = (start + len + CONTEXT).min(chars.len());
    let before: String = chars[from..start].iter().collect();
    let matched: String = chars[start..start + len].iter().collect();
    let after: String = chars[start + len..to].iter().collect();
    let lead = if from > 0 { "…" } else { "" };
    let tail = if to < chars.len() { "…" } else { "" };

    h_flex()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_xs()
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(palette.muted_text))
                .child(lead.to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(palette.sidebar_text))
                .child(before),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(palette.warning))
                .child(matched),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(palette.sidebar_text))
                .child(after),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(palette.muted_text))
                .child(tail.to_string()),
        )
        .into_any_element()
}
