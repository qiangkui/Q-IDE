//! 输出面板。
//!
//! 这里提供三样东西：
//! - [`OutputLog`]：日志数据模型（可被任意视图观察）；
//! - [`OutputHandle`]：供其它模块写日志的对外接口；
//! - [`render`]：输出面板的界面渲染。
//!
//! 日志区支持鼠标拖选、双击选整行、Ctrl+A 全选；Ctrl+C 复制选区，
//! 标题栏的“复制”在没有选区时复制全部输出。
//! 连续出现的相同错误/警告会合并成一行，显示「（重复 N 次）」。
//!
//! 用法示例：
//! ```ignore
//! let output = window_entity.read(cx).output();
//! output.info(cx, "开始构建");
//! output.command(cx, "> cargo build");
//! ```

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::*;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{self as ui_theme, Palette};

/// 一条输出的级别，决定文字颜色与语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputLevel {
    /// 命令行回显，例如 `> cargo run`。
    Command,
    Info,
    Success,
    Warning,
    Error,
}

impl OutputLevel {
    pub fn color(self, palette: &Palette) -> u32 {
        match self {
            Self::Command => palette.command,
            Self::Info => palette.info,
            Self::Success => palette.success,
            Self::Warning => palette.warning,
            Self::Error => palette.error,
        }
    }

    /// 连续出现的同样文本是否合并成一行（错误与警告会），避免循环里刷屏。
    pub fn groups_repeats(self) -> bool {
        matches!(self, Self::Warning | Self::Error)
    }
}

/// 输出面板里的一行。
#[derive(Clone, Debug)]
pub struct OutputLine {
    pub level: OutputLevel,
    pub text: String,
    /// 相对程序启动时刻的写入时间。
    pub at: Duration,
    /// 连续出现的相同错误/警告次数；`1` 表示没有重复。
    pub repeat: usize,
}

/// 显示用的正文：重复多次的错误在后面补上次数。
pub(crate) fn display_text(line: &OutputLine) -> String {
    if line.repeat > 1 {
        format!("{}（重复 {} 次）", line.text, line.repeat)
    } else {
        line.text.clone()
    }
}

/// 新输出能否并入上一行：同类错误/警告，且文本完全相同。
pub(crate) fn can_group(last: &OutputLine, level: OutputLevel, text: &str) -> bool {
    level.groups_repeats() && last.level == level && last.text == text
}

/// 时间戳前缀，例如 `[   1.23s]`。
fn timestamp_prefix(line: &OutputLine) -> String {
    format!("[{:>7.2}s]", line.at.as_secs_f64())
}

/// 输出文本里的一个位置：第 `line` 行的第 `byte` 个字节（相对该行的正文，不含时间戳）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPosition {
    pub line: usize,
    pub byte: usize,
}

impl TextPosition {
    pub fn new(line: usize, byte: usize) -> Self {
        Self { line, byte }
    }

    /// 前面被裁掉 `excess` 行之后的对应位置。
    fn shifted(self, excess: usize) -> Self {
        if self.line < excess {
            Self::new(0, 0)
        } else {
            Self::new(self.line - excess, self.byte)
        }
    }
}

/// 输出面板里的选区：`anchor` 是鼠标按下的位置，`focus` 是当前拖动到的位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: TextPosition,
    pub focus: TextPosition,
}

impl TextSelection {
    pub fn new(anchor: TextPosition, focus: TextPosition) -> Self {
        Self { anchor, focus }
    }

    /// 按阅读顺序给出起止位置。
    pub fn ordered(self) -> (TextPosition, TextPosition) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_collapsed(self) -> bool {
        self.anchor == self.focus
    }
}

/// 从「第 12 行运行时错误: …」「第 5 行：…」这类文本开头取出行号。
pub(crate) fn line_number_prefix(text: &str) -> Option<usize> {
    let rest = text.strip_prefix("第 ")?;
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || !rest[digits..].starts_with(" 行") {
        return None;
    }
    rest[..digits].parse().ok()
}

/// 把字节位置夹到 `text` 的字符边界上（顺带处理超出末尾的情况）。
fn clamp_byte(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// 选区落在某一行里的字节范围（相对该行正文）；这一行没被选中时为 `None`。
/// `start` / `end` 需按阅读顺序传入。
pub(crate) fn selection_range(
    start: TextPosition,
    end: TextPosition,
    line: usize,
    text: &str,
) -> Option<Range<usize>> {
    if line < start.line || line > end.line {
        return None;
    }
    let from = if line == start.line {
        clamp_byte(text, start.byte)
    } else {
        0
    };
    let to = if line == end.line {
        clamp_byte(text, end.byte)
    } else {
        text.len()
    };
    (from < to).then_some(from..to)
}

/// 选区里的纯文本：跨行时用换行连接。
///
/// 界面渲染用的是可见行版本（[`OutputLog::copy_text`]），这个函数留给单测与纯逻辑复用。
#[allow(dead_code)]
pub(crate) fn selection_text(lines: &[OutputLine], selection: TextSelection) -> String {
    let (start, end) = selection.ordered();
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index < start.line || index > end.line {
            continue;
        }
        if index > start.line {
            out.push('\n');
        }
        if let Some(range) = selection_range(start, end, index, &line.text) {
            out.push_str(&line.text[range]);
        }
    }
    out
}

/// 渲染时回填的几何信息，用来把鼠标位置换算成文本位置。
#[derive(Default)]
struct OutputGeometry {
    /// 每行正文的排版结果，顺序与可见行一致。
    layouts: Vec<TextLayout>,
    /// 每行在窗口中的矩形，顺序同上。
    rows: Vec<Bounds<Pixels>>,
}

/// 输出面板的过滤条件。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputFilter {
    /// 关键字（忽略大小写，空串表示不按关键字过滤）。
    pub text: String,
    /// 只看错误与警告。
    pub only_errors: bool,
}

impl OutputFilter {
    /// 这个过滤条件会不会真的隐藏内容。
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || self.only_errors
    }

    /// 一行是否应当显示。
    pub fn matches(&self, line: &OutputLine) -> bool {
        if self.only_errors && !matches!(line.level, OutputLevel::Warning | OutputLevel::Error) {
            return false;
        }
        let query = self.text.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        line.text.to_lowercase().contains(&query)
    }
}

/// 输出日志模型。由 `CodeWindow` 持有，任何拿到 [`OutputHandle`] 的代码都可以写入。
pub struct OutputLog {
    channel: SharedString,
    lines: Vec<OutputLine>,
    /// 过滤后仍然可见的行号（相对 `lines`），由 `filter` 与 `lines` 一起决定。
    visible: Vec<usize>,
    filter: OutputFilter,
    started_at: Instant,
    max_lines: usize,
    show_timestamps: bool,
    scroll: ScrollHandle,
    /// 当前选区；`None` 表示没有选中任何内容。
    selection: Option<TextSelection>,
    /// 拖动选择时指针所在的窗口坐标：指针停在可见区域外时靠它持续滚动。
    drag_pointer: Option<Point<Pixels>>,
    /// 自动滚动的帧循环是否已经在跑，避免重复启动。
    autoscroll_running: Rc<Cell<bool>>,
    /// 面板的键盘焦点：拿到焦点后 Ctrl+C / Ctrl+A 才生效。
    focus: FocusHandle,
    /// 每帧由 [`render`] 回填的几何信息。
    geometry: Rc<RefCell<OutputGeometry>>,
    /// 最近一次单击命中的「第 N 行…」行号；界面取走后跳到对应代码行。
    clicked_line: Option<usize>,
}

impl OutputLog {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            channel: "输出".into(),
            lines: Vec::new(),
            visible: Vec::new(),
            filter: OutputFilter::default(),
            started_at: Instant::now(),
            max_lines: 2_000,
            show_timestamps: false,
            scroll: ScrollHandle::new(),
            selection: None,
            drag_pointer: None,
            autoscroll_running: Rc::new(Cell::new(false)),
            focus: cx.focus_handle(),
            geometry: Rc::new(RefCell::new(OutputGeometry::default())),
            clicked_line: None,
        }
    }

    /// 输出通道名，显示在面板标题栏，例如 `cargo run`。
    pub fn channel(&self) -> &SharedString {
        &self.channel
    }

    pub fn set_channel(&mut self, channel: impl Into<SharedString>) {
        self.channel = channel.into();
    }

    /// 过滤后的可见行数（界面里用的行下标都是可见行下标）。
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// 第 `index` 行可见内容；`index` 超出可见范围时为 `None`。
    pub fn visible_line(&self, index: usize) -> Option<&OutputLine> {
        self.visible.get(index).and_then(|line| self.lines.get(*line))
    }

    /// 设置过滤条件；条件没变时什么都不做。
    pub fn set_filter(&mut self, filter: OutputFilter, cx: &mut Context<Self>) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        self.rebuild_visible();
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// 按当前过滤条件重算可见行；行号可能整体变化，选区一并清掉。
    fn rebuild_visible(&mut self) {
        self.visible = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| self.filter.matches(line))
            .map(|(index, _)| index)
            .collect();
        self.selection = None;
        self.drag_pointer = None;
    }

    /// 可见内容的纯文本（复制时用）。
    pub fn visible_text(&self) -> String {
        if !self.filter.is_active() {
            return self.text();
        }
        let mut out = String::new();
        for index in 0..self.visible.len() {
            if index > 0 {
                out.push('\n');
            }
            if let Some(line) = self.visible_line(index) {
                if self.show_timestamps {
                    out.push_str(&timestamp_prefix(line));
                    out.push(' ');
                }
                out.push_str(&display_text(line));
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// 把全部输出拼成纯文本，供复制到剪贴板。
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            if self.show_timestamps {
                out.push_str(&timestamp_prefix(line));
                out.push(' ');
            }
            out.push_str(&display_text(line));
        }
        out
    }

    /// 复制用文本：有选区时只复制选区内容，否则复制可见的全部输出。
    pub fn copy_text(&self) -> String {
        match self.selection.filter(|selection| !selection.is_collapsed()) {
            Some(selection) => self.visible_selection_text(selection),
            None => self.visible_text(),
        }
    }

    /// 选区里的纯文本（行下标按可见行算）。
    fn visible_selection_text(&self, selection: TextSelection) -> String {
        let (start, end) = selection.ordered();
        let mut out = String::new();
        for index in 0..self.visible.len() {
            if index < start.line || index > end.line {
                continue;
            }
            let Some(line) = self.visible_line(index) else {
                continue;
            };
            if index > start.line {
                out.push('\n');
            }
            if let Some(range) = selection_range(start, end, index, &line.text) {
                out.push_str(&line.text[range]);
            }
        }
        out
    }

    pub fn selection(&self) -> Option<TextSelection> {
        self.selection
    }

    pub fn has_selection(&self) -> bool {
        self.selection
            .is_some_and(|selection| !selection.is_collapsed())
    }

    /// 面板的键盘焦点。
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// 鼠标按下：从这一点开始新的选区，并记下正在拖动（用于拖到面板外时继续滚动）。
    pub fn begin_selection(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(position) = self.position_at(pointer) else {
            return;
        };
        self.drag_pointer = Some(pointer);
        let selection = TextSelection::new(position, position);
        if self.selection != Some(selection) {
            self.selection = Some(selection);
            cx.notify();
        }
    }

    /// 是否正在进行鼠标拖动选择。
    pub fn is_dragging(&self) -> bool {
        self.drag_pointer.is_some()
    }

    /// 拖动：更新选区终点。
    pub fn extend_selection(&mut self, position: TextPosition, cx: &mut Context<Self>) {
        if let Some(selection) = self.selection.as_mut()
            && selection.focus != position
        {
            selection.focus = position;
            cx.notify();
        }
    }

    /// 拖动中：更新选区，并记住指针位置 —— 拖到可见区域外时自动滚动。
    pub fn drag_select(
        &mut self,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag_pointer = Some(pointer);
        if let Some(position) = self.position_at(pointer) {
            self.extend_selection(position, cx);
        }
        self.start_autoscroll(window, cx);
    }

    /// 松开鼠标：结束拖动，没有拖动过的选区（单击）直接取消。
    ///
    /// 单击落在「第 N 行…」这样的输出行上时，记下行号供界面跳到对应代码行。
    pub fn finish_selection(&mut self, cx: &mut Context<Self>) {
        self.drag_pointer = None;
        if self.selection.is_some_and(TextSelection::is_collapsed) {
            let line = self.selection.map(|selection| selection.focus.line);
            self.selection = None;
            if let Some(number) = line
                .and_then(|line| self.visible_line(line))
                .and_then(|line| line_number_prefix(&line.text))
            {
                self.clicked_line = Some(number);
            }
            cx.notify();
        }
    }

    /// 取出「用户点了带行号的输出行」的行号（取走后清空）。
    pub fn take_clicked_line(&mut self) -> Option<usize> {
        self.clicked_line.take()
    }

    /// 选中整行（双击）。
    pub fn select_line(&mut self, line: usize, cx: &mut Context<Self>) {
        let Some(target) = self.visible_line(line) else {
            return;
        };
        let end = target.text.len();
        self.selection = Some(TextSelection::new(
            TextPosition::new(line, 0),
            TextPosition::new(line, end),
        ));
        cx.notify();
    }

    /// 全选（Ctrl+A 或三击）。
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(last) = self.visible.len().checked_sub(1) else {
            return;
        };
        let text_len = self.visible_line(last).map_or(0, |line| line.text.len());
        self.selection = Some(TextSelection::new(
            TextPosition::new(0, 0),
            TextPosition::new(last, text_len),
        ));
        cx.notify();
    }

    /// 取消选区。
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.take().is_some() {
            cx.notify();
        }
    }

    /// 命中测试：窗口坐标 → 文本位置。面板这一帧还没排版完成时返回 `None`。
    pub fn position_at(&self, position: Point<Pixels>) -> Option<TextPosition> {
        let visible = self.visible.len();
        let geometry = self.geometry.borrow();
        if visible == 0
            || geometry.layouts.len() != visible
            || geometry.rows.len() != visible
        {
            return None;
        }

        // 先按 y 找到所在行：落在行间空隙里归到下面一行，超出末尾时归到最后一行。
        let mut index = visible - 1;
        for (line, row) in geometry.rows.iter().enumerate() {
            if position.y <= row.bottom() {
                index = line;
                break;
            }
        }

        let text = &self.visible_line(index)?.text;
        let byte = geometry.layouts[index]
            .index_for_position(position)
            .unwrap_or_else(|byte| byte);
        Some(TextPosition::new(index, clamp_byte(text, byte)))
    }

    /// 拖动到可见区域外时启动按帧滚动的循环（已经在跑就什么都不做）。
    fn start_autoscroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.autoscroll_running.get() || self.autoscroll_step().is_none() {
            return;
        }
        self.autoscroll_running.set(true);
        let entity = cx.entity();
        window.on_next_frame(move |window, cx| autoscroll_frame(&entity, window, cx));
    }

    /// 自动滚动一帧：滚动一点、按滚动后的内容更新选区，返回是否继续。
    fn autoscroll_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pointer) = self.drag_pointer else {
            self.autoscroll_running.set(false);
            return false;
        };
        let Some(step) = self.autoscroll_step() else {
            // 指针回到可见区域里了，滚动停下来；下一次鼠标移动会重新启动。
            self.autoscroll_running.set(false);
            return false;
        };
        if !self.scroll_by(step) {
            // 已经滚到顶 / 底。
            self.autoscroll_running.set(false);
            return false;
        }

        // 几何信息还是滚动前的：内容上移了 `step`，等价于在 `y + step` 处命中。
        if let Some(position) = self.position_at(point(pointer.x, pointer.y + step)) {
            self.extend_selection(position, cx);
        }
        cx.notify();
        true
    }

    /// 指针停在可见区域上下边缘之外时，每帧应滚动的像素数（正数滚向末尾）。
    fn autoscroll_step(&self) -> Option<Pixels> {
        /// 距离边缘多近就开始滚动。
        const EDGE: f32 = 24.0;
        const MIN_STEP: f32 = 4.0;
        const MAX_STEP: f32 = 24.0;

        let pointer = self.drag_pointer?;
        let viewport = self.scroll.bounds();
        if f32::from(viewport.size.height) <= 0.0 {
            return None;
        }

        let top = f32::from(viewport.top()) + EDGE;
        let bottom = f32::from(viewport.bottom()) - EDGE;
        let y = f32::from(pointer.y);
        let over = if y < top {
            y - top
        } else if y > bottom {
            y - bottom
        } else {
            return None;
        };

        // 离边缘越远滚得越快。
        let ratio = (over.abs() / EDGE).clamp(0.0, 1.0);
        let step = MIN_STEP + ratio * (MAX_STEP - MIN_STEP);
        Some(px(if over < 0.0 { -step } else { step }))
    }

    /// 按像素滚动面板（正数滚向末尾），返回是否真的滚动了。
    fn scroll_by(&mut self, step: Pixels) -> bool {
        let offset = self.scroll.offset();
        let current = f32::from(-offset.y);
        let limit = f32::from(self.scroll.max_offset().y);
        let next = (current + f32::from(step)).clamp(0.0, limit);
        if (next - current).abs() < 0.5 {
            return false;
        }
        self.scroll.set_offset(point(offset.x, px(-next)));
        true
    }

    pub fn show_timestamps(&self) -> bool {
        self.show_timestamps
    }

    pub fn scroll_handle(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// 追加一行输出。
    ///
    /// 连续两条同类错误/警告文本相同时不新增一行，而是把上一条的重复次数加一。
    pub fn push(&mut self, level: OutputLevel, text: impl Into<String>, cx: &mut Context<Self>) {
        let text = text.into();
        if let Some(last) = self.lines.last_mut()
            && can_group(last, level, &text)
        {
            last.repeat += 1;
            last.at = self.started_at.elapsed();
            if self.filter.matches(last) && self.visible.last() != Some(&(self.lines.len() - 1)) {
                self.visible.push(self.lines.len() - 1);
            }
            self.scroll.scroll_to_bottom();
            cx.notify();
            return;
        }
        self.lines.push(OutputLine {
            level,
            text,
            at: self.started_at.elapsed(),
            repeat: 1,
        });
        if let Some(last) = self.lines.last()
            && self.filter.matches(last)
        {
            self.visible.push(self.lines.len() - 1);
        }
        self.trim();
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// 追加不带换行的文本，模拟进程的标准输出：接在当前最后一行之后。
    #[allow(dead_code)] // 供后续接入真实进程输出时使用。
    pub fn write(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let text = text.into();
        match self.lines.last_mut() {
            Some(line) => line.text.push_str(&text),
            None => self.lines.push(OutputLine {
                level: OutputLevel::Info,
                text,
                at: self.started_at.elapsed(),
                repeat: 1,
            }),
        }
        // 行内容变了，可见状态可能跟着变（例如关键字过滤）。
        self.rebuild_visible();
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// 清空全部输出。
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.lines.clear();
        self.visible.clear();
        self.selection = None;
        self.drag_pointer = None;
        cx.notify();
    }

    pub fn set_max_lines(&mut self, max_lines: usize) {
        self.max_lines = max_lines.max(50);
        self.trim();
    }

    pub fn set_show_timestamps(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.show_timestamps != show {
            self.show_timestamps = show;
            cx.notify();
        }
    }

    fn trim(&mut self) {
        if self.lines.len() > self.max_lines {
            let excess = self.lines.len() - self.max_lines;
            self.lines.drain(..excess);
            // 可见行号整体前移，被裁掉的行从可见列表里去掉。
            self.visible.retain(|index| *index >= excess);
            for index in self.visible.iter_mut() {
                *index -= excess;
            }
            // 行号整体前移，选区跟着走，别指到别的行上。
            if let Some(selection) = self.selection.as_mut() {
                selection.anchor = selection.anchor.shifted(excess);
                selection.focus = selection.focus.shifted(excess);
            }
        }
    }
}

/// 自动滚动的一帧：滚一点、更新选区，需要的话安排下一帧。
fn autoscroll_frame(entity: &Entity<OutputLog>, window: &mut Window, cx: &mut App) {
    if !entity.update(cx, |log, cx| log.autoscroll_tick(cx)) {
        return;
    }
    let entity = entity.clone();
    window.on_next_frame(move |window, cx| autoscroll_frame(&entity, window, cx));
}

/// 输出接口：可以克隆、跨模块传递，写入后自动重绘面板。
#[derive(Clone)]
#[allow(dead_code)] // 这是给其它模块使用的输出接口，示例中不会全部调用。
pub struct OutputHandle {
    log: Entity<OutputLog>,
}

#[allow(dead_code)] // 这是给其它模块使用的输出接口，示例中不会全部调用。
impl OutputHandle {
    pub fn new(log: Entity<OutputLog>) -> Self {
        Self { log }
    }

    pub fn entity(&self) -> &Entity<OutputLog> {
        &self.log
    }

    pub fn update(
        &self,
        cx: &mut App,
        update: impl FnOnce(&mut OutputLog, &mut Context<OutputLog>),
    ) {
        self.log.update(cx, update);
    }

    pub fn push(&self, cx: &mut App, level: OutputLevel, text: impl Into<String>) {
        let text = text.into();
        self.log.update(cx, |log, cx| log.push(level, text, cx));
    }

    pub fn command(&self, cx: &mut App, text: impl Into<String>) {
        self.push(cx, OutputLevel::Command, text);
    }

    pub fn info(&self, cx: &mut App, text: impl Into<String>) {
        self.push(cx, OutputLevel::Info, text);
    }

    pub fn success(&self, cx: &mut App, text: impl Into<String>) {
        self.push(cx, OutputLevel::Success, text);
    }

    pub fn warning(&self, cx: &mut App, text: impl Into<String>) {
        self.push(cx, OutputLevel::Warning, text);
    }

    pub fn error(&self, cx: &mut App, text: impl Into<String>) {
        self.push(cx, OutputLevel::Error, text);
    }

    /// 追加不带换行的文本，模拟进程的实时输出。
    pub fn write(&self, cx: &mut App, text: impl Into<String>) {
        let text = text.into();
        self.log.update(cx, |log, cx| log.write(text, cx));
    }

    pub fn clear(&self, cx: &mut App) {
        self.log.update(cx, |log, cx| log.clear(cx));
    }
}

/// 过滤条上「只看错误」按钮的回调。
pub type FilterToggle = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// 输出面板的渲染参数。
pub struct OutputPanelProps<'a> {
    pub log: &'a OutputLog,
    /// 面板自身的实体：鼠标与键盘回调通过它更新选区。
    pub entity: Entity<OutputLog>,
    pub palette: Palette,
    /// Dock 内由容器决定高度时为 `None`。
    pub height: Option<f32>,
    pub font_size: f32,
    /// 过滤输入框；`None` 表示不显示过滤条。
    pub filter_input: Option<Entity<InputState>>,
    /// 「只看错误」是否选中。
    pub only_errors: bool,
    /// 点「只看错误」时调用（切换开关）。
    pub on_toggle_only_errors: Option<FilterToggle>,
}

/// 渲染输出面板：标题栏（复制 / 清空 / 关闭）+ 可滚动的日志区。
///
/// 日志区里可以拖动鼠标选择文字、双击选整行、Ctrl+A 全选，Ctrl+C 复制选区。
pub fn render(
    props: OutputPanelProps<'_>,
    on_copy: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_clear: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    show_header: bool,
) -> AnyElement {
    let palette = props.palette;
    let log = props.log;

    // 几何信息每帧重建：`position_at` 只有行数对得上时才敢用。
    {
        let mut geometry = log.geometry.borrow_mut();
        geometry.layouts.clear();
        geometry.rows.clear();
    }

    let body: AnyElement = if log.is_empty() {
        div()
            .flex_1()
            .w_full()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(palette.side_panel_bg))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted_text))
                    .child("暂无输出"),
            )
            .into_any_element()
    } else if log.visible_len() == 0 {
        // 有输出但都被过滤掉了：提示一下，免得以为输出没了。
        div()
            .flex_1()
            .w_full()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(palette.side_panel_bg))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted_text))
                    .child("没有匹配当前过滤条件的输出"),
            )
            .into_any_element()
    } else {
        log_body(log, props.entity, palette, props.font_size)
    };

    let mut panel = v_flex()
        .bg(rgb(palette.sidebar_bg))
        .rounded(px(ui_theme::PANE_RADIUS))
        .border_1()
        .border_color(rgba(palette.border))
        .overflow_hidden();
    panel = match props.height {
        Some(height) => panel.h(px(height)).w_full().flex_shrink_0(),
        None => panel.w_full().h_full().min_h_0(),
    };

    panel
        .when(show_header, |this| {
            this.child(
                h_flex()
                    .h(px(ui_theme::PANEL_HEADER_HEIGHT))
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .px(px(12.0))
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.sidebar_text))
                                    .child("输出"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.file))
                                    .child(log.channel().clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("output-copy")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Copy)
                                    .label(if log.has_selection() {
                                        "复制选区"
                                    } else {
                                        "复制"
                                    })
                                    .disabled(log.is_empty())
                                    .on_click(on_copy),
                            )
                            .child(
                                Button::new("output-clear")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Delete)
                                    .label("清空")
                                    .on_click(on_clear),
                            )
                            .when_some(props.on_toggle_only_errors.clone(), |this, toggle| {
                                this.child(
                                    Button::new("output-only-errors")
                                        .ghost()
                                        .xsmall()
                                        .label("只看错误")
                                        .selected(props.only_errors)
                                        .on_click(move |event, window, cx| toggle(event, window, cx)),
                                )
                            })
                            .when_some(props.filter_input.clone(), |this, field| {
                                this.child(
                                    div().w(px(150.0)).child(
                                        Input::new(&field)
                                            .xsmall()
                                            .focus_ring(false)
                                            .appearance(false),
                                    ),
                                )
                            })
                            .child(
                                div()
                                    .id("output-close")
                                    .flex_shrink_0()
                                    .size(px(18.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.0))
                                    .cursor(CursorStyle::PointingHand)
                                    .hover(move |this| this.bg(rgba(palette.hover)))
                                    .child(
                                        Icon::new(IconName::ChevronDown)
                                            .xsmall()
                                            .text_color(rgb(palette.sidebar_text)),
                                    )
                                    .on_click(on_close),
                            ),
                    ),
            )
        })
        .child(body)
        .into_any_element()
}

/// 日志区：可滚动的行列表，支持鼠标拖选、双击选行、Ctrl+A 全选与 Ctrl+C 复制。
fn log_body(
    log: &OutputLog,
    entity: Entity<OutputLog>,
    palette: Palette,
    font_size: f32,
) -> AnyElement {
    let focus = log.focus_handle();
    let selection = log
        .selection()
        .filter(|selection| !selection.is_collapsed())
        .map(TextSelection::ordered);
    let highlight = Hsla::from(rgb(palette.selection));

    let count = log.visible_len();
    let mut layouts = Vec::with_capacity(count);
    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let Some(line) = log.visible_line(index) else {
            continue;
        };
        let display = display_text(line);
        let mut text = StyledText::new(display);
        if let Some(range) =
            selection.and_then(|(start, end)| selection_range(start, end, index, &line.text))
        {
            text = text.with_highlights([(
                range,
                HighlightStyle {
                    background_color: Some(highlight),
                    ..Default::default()
                },
            )]);
        }
        layouts.push(text.layout().clone());

        let mut row = h_flex().items_start().gap_2();
        if log.show_timestamps() {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(palette.muted_text))
                    .child(timestamp_prefix(line)),
            );
        }
        rows.push(
            row.child(
                div()
                    .text_color(rgb(line.level.color(&palette)))
                    .child(text),
            )
            .into_any_element(),
        );
    }
    log.geometry.borrow_mut().layouts = layouts;
    let rows_sink = log.geometry.clone();

    let on_mouse_down = {
        let entity = entity.clone();
        let focus = focus.clone();
        move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            focus.focus(window, cx);
            let Some(position) = entity.read(cx).position_at(event.position) else {
                return;
            };
            entity.update(cx, |log, cx| match event.click_count {
                2 => log.select_line(position.line, cx),
                3.. => log.select_all(cx),
                _ => log.begin_selection(event.position, cx),
            });
        }
    };

    let on_mouse_move = {
        let entity = entity.clone();
        move |event: &MouseMoveEvent, window: &mut Window, cx: &mut App| {
            if !event.dragging() || !entity.read(cx).is_dragging() {
                return;
            }
            entity.update(cx, |log, cx| log.drag_select(event.position, window, cx));
        }
    };

    // 松开鼠标的位置可能在面板外，用捕获监听保证拖动一定会结束。
    let on_mouse_up = {
        let entity = entity.clone();
        move |_: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
            entity.update(cx, |log, cx| log.finish_selection(cx));
        }
    };

    let on_key_down = {
        move |event: &KeyDownEvent, _window: &mut Window, cx: &mut App| {
            let keystroke = &event.keystroke;
            let shortcut = keystroke.modifiers.control || keystroke.modifiers.platform;
            match keystroke.key.as_str() {
                "escape" => {
                    entity.update(cx, |log, cx| log.clear_selection(cx));
                    cx.stop_propagation();
                }
                "c" if shortcut => {
                    let text = entity.read(cx).copy_text();
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    cx.stop_propagation();
                }
                "a" if shortcut => {
                    entity.update(cx, |log, cx| log.select_all(cx));
                    cx.stop_propagation();
                }
                _ => {}
            }
        }
    };

    div()
        .id("output-scroll")
        .relative()
        .flex_1()
        .w_full()
        .min_h_0()
        .child(
            div()
                .id("output-scroll-area")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(log.scroll_handle())
                .bg(rgb(palette.side_panel_bg))
                .p(px(12.0))
                .font_family(ui_theme::MONO_FONT)
                .text_size(px(font_size))
                .cursor(CursorStyle::IBeam)
                .track_focus(&focus)
                .on_mouse_down(MouseButton::Left, on_mouse_down)
                .on_mouse_move(on_mouse_move)
                .capture_any_mouse_up(on_mouse_up)
                .on_key_down(on_key_down)
                .child(
                    v_flex()
                        .gap_1()
                        .on_children_prepainted(move |rows, _window, _cx| {
                            rows_sink.borrow_mut().rows = rows;
                        })
                        .children(rows),
                ),
        )
        // 滚动条浮在日志区上方，不占正文宽度。
        .child(
            div().absolute().inset_0().child(
                Scrollbar::vertical(log.scroll_handle())
                    .id("output-scrollbar")
                    .mode(ScrollbarMode::Always)
                    .viewport_from_layout(),
            ),
        )
        .into_any_element()
}
