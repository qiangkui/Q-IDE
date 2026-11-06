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
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            case_sensitive: false,
            whole_word: false,
        }
    }

    /// 有没有实际要搜的内容（只输空格不算）。
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// 可切换的搜索选项。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchOption {
    CaseSensitive,
    WholeWord,
}

impl SearchOption {
    pub fn label(self) -> &'static str {
        match self {
            Self::CaseSensitive => "Aa",
            Self::WholeWord => "全词",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::CaseSensitive => "区分大小写",
            Self::WholeWord => "全词匹配",
        }
    }
}

/// 一处命中：行号、列号都从 1 开始（与编辑器状态栏一致）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub path: String,
    pub line: usize,
    pub column: usize,
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
    /// 跳过（过大、二进制、非 UTF-8、读不了）的文件数。
    pub skipped: usize,
    /// 用户又发起了一次搜索，这次被提前取消。
    pub cancelled: bool,
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
pub fn find_in_text(
    text: &str,
    query: &SearchQuery,
    path: &str,
    hits: &mut Vec<SearchHit>,
) -> bool {
    if query.text.is_empty() {
        return false;
    }
    for (index, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(column) = first_match(line, query) {
            hits.push(SearchHit {
                path: path.to_string(),
                line: index + 1,
                column: column + 1,
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
        let full = find_in_text(&text, query, path, &mut results.hits);
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

/// 读出可搜索的文本：过大、含 NUL 字节（二进制）或非 UTF-8 时返回 `None`。
fn read_text(path: &str) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SEARCH_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// 一行里第一处命中的字符下标（0 基）。
fn first_match(line: &str, query: &SearchQuery) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    let needle: Vec<char> = query.text.chars().collect();
    if needle.is_empty() || needle.len() > chars.len() {
        return None;
    }
    for start in 0..=chars.len() - needle.len() {
        if !match_at(&chars, start, &needle, query) {
            continue;
        }
        if query.whole_word && !is_word_boundary(&chars, start, needle.len()) {
            continue;
        }
        return Some(start);
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
fn replace_line(line: &str, query: &SearchQuery, replacement: &str) -> (String, usize) {
    let chars: Vec<char> = line.chars().collect();
    let needle: Vec<char> = query.text.chars().collect();
    if needle.is_empty() || needle.len() > chars.len() {
        return (line.to_string(), 0);
    }
    let mut out = String::with_capacity(line.len());
    let mut count = 0;
    let mut index = 0;
    while index < chars.len() {
        if index + needle.len() <= chars.len()
            && match_at(&chars, index, &needle, query)
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

/// 把 `text` 里所有命中替换成 `replacement`，返回新文本与替换次数。
///
/// 匹配规则与搜索一致（区分大小写、全词），逐行处理，行尾（`\n` / `\r\n`）原样保留。
pub fn replace_in_text(text: &str, query: &SearchQuery, replacement: &str) -> (String, usize) {
    if query.text.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    for line in text.split_inclusive('\n') {
        let (raw_body, ending) = match line.strip_suffix('\n') {
            Some(rest) => (rest, "\n"),
            None => (line, ""),
        };
        // `\r\n` 里的 `\r` 属于行尾，不能当成正文参与替换。
        let (body, ending) = match raw_body.strip_suffix('\r') {
            Some(body) if !ending.is_empty() => (body, "\r\n"),
            _ => (raw_body, ending),
        };
        let (replaced, hits) = replace_line(body, query, replacement);
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
    /// 读不了、被跳过（过大 / 二进制 / 非 UTF-8）的文件数。
    pub skipped: usize,
    /// 写回失败的文件与原因。
    pub failures: Vec<String>,
}

/// 在一批文件里替换关键字并写回。
///
/// 只处理搜索能读的文本（UTF-8、不含 NUL、不超过 [`MAX_SEARCH_BYTES`]），
/// 所以写回的就是原编码（UTF-8）内容，换行符也保持原样。
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
        let Some(text) = read_text(path) else {
            outcome.skipped += 1;
            continue;
        };
        let (replaced, count) = replace_in_text(&text, query, replacement);
        if count == 0 {
            continue;
        }
        match std::fs::write(path, replaced.as_bytes()) {
            Ok(()) => {
                outcome.replacements += count;
                outcome.changed.push((path.clone(), count));
            }
            Err(error) => outcome.failures.push(format!("{path}：{error}")),
        }
    }
    outcome
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
        .bg(rgb(palette.panel_bg));
    for option in [SearchOption::CaseSensitive, SearchOption::WholeWord] {
        let selected = match option {
            SearchOption::CaseSensitive => query.case_sensitive,
            SearchOption::WholeWord => query.whole_word,
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
    let replace = run_replace.clone();

    // 搜索范围：默认全部候选位置，可以限定到某个文件夹。
    let scope_label = match scope {
        Some(scope) => format!("范围：{}", scope_name(scope)),
        None => "范围：全部位置".to_string(),
    };
    let mut scope_row = h_flex()
        .gap_1()
        .px(px(8.0))
        .pb(px(4.0))
        .bg(rgb(palette.panel_bg))
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
                list = list.child(file_header(hit.path.as_str(), group_index, palette));
            }
            let open = open_hit.clone();
            list = list.child(hit_row(index, hit, query, palette, open));
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
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .px(px(8.0))
                .pb(px(4.0))
                .bg(rgb(palette.panel_bg))
                .child(div().flex_1().min_w_0().child(replace_input))
                .child(
                    Button::new("search-replace-all")
                        .ghost()
                        .xsmall()
                        .label("全部替换")
                        .tooltip("把结果里每个文件中的命中都替换掉")
                        .disabled(results.is_empty() || progress.is_some())
                        .on_click(move |_, window, cx| replace(window, cx)),
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
                .bg(rgb(palette.panel_bg))
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
        .bg(rgb(palette.panel_bg))
        .text_xs()
        .text_color(rgb(palette.muted_text))
        .child(text)
        .when(results.skipped > 0, |this| {
            this.child(format!("跳过 {} 个文件", results.skipped))
        })
}

fn file_header(path: &str, index: usize, palette: Palette) -> impl IntoElement {
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    h_flex()
        .id(format!("search-file-{index}"))
        .w_full()
        .h(px(24.0))
        .px(px(8.0))
        .gap_1()
        .items_center()
        .bg(rgb(palette.sidebar_bg))
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
    query: &SearchQuery,
    palette: Palette,
    open: OpenHitAction,
) -> impl IntoElement {
    let path = hit.path.clone();
    let line = hit.line;
    h_flex()
        .id(format!("search-hit-{index}"))
        .w_full()
        .min_h(px(22.0))
        .pl(px(20.0))
        .pr(px(8.0))
        .gap_2()
        .items_center()
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(rgba(palette.hover)))
        .on_click(move |_, window, cx| open(&path, line, window, cx))
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(hit.line.to_string()),
        )
        .child(highlighted_preview(&hit.preview, query, palette))
}

/// 命中行的上下文：只截取命中附近的一段，并把关键字本身用醒目的颜色标出来。
fn highlighted_preview(preview: &str, query: &SearchQuery, palette: Palette) -> AnyElement {
    let chars: Vec<char> = preview.chars().collect();
    let Some(start) = first_match(preview, query) else {
        return div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_xs()
            .text_color(rgb(palette.sidebar_text))
            .child(preview.to_string())
            .into_any_element();
    };
    let len = query.text.chars().count();
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
