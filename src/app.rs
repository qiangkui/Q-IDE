//! 主窗口：负责整体布局、标签页状态，以及把设置同步到界面。
//!
//! 具体实现按职责拆到子模块：
//! - `ai`：AI 面板发来的「写入文件」请求；
//! - `breakpoints`：断点、断点条件与编辑器标记；
//! - `debug`：脚本运行、调试会话与监视；
//! - `dialogs`：输入、确认与未保存提示对话框；
//! - `dock`：Dock 面板初始化和左右停靠、可见性同步、拖动；
//! - `editor`：导航、命令面板、补全/诊断刷新与文本编辑动作；
//! - `files`：文件、标签与资源管理器操作；
//! - `session`：会话恢复、标签关闭、外部修改检测与布局保存；
//! - `view`：菜单栏、活动栏、状态栏与整体渲染。

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::base::input::{
    Diagnostic, DiagnosticSeverity, GutterDecoration, Position, RangeDecoration,
    RangeDecorationStyle, Rope, RopeExt as _,
};
use gpui_kit::base::input::{GutterDecorationCollection, RangeDecorationCollection};
use gpui_kit::component::button::*;
use gpui_kit::component::dock::{
    DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, DockSkin, DragPanel,
    InsertTarget, NodeId, PanelId, PanelStyle, PaneNode, PaneRef, panel_handle, register_panel,
};
use gpui_kit::component::input::{EditorState, Enter, Indent, Input, InputState, Outdent, SelectAll, ShowCompletions};
use gpui_kit::component::tree::TreeState;
use gpui_kit::component::*;
use gpui_kit::prelude::{FluentBuilder as _, InteractiveElement as _};
use gpui_kit::*;

use crate::ai::panel::AiPanel;
use crate::brackets;
use crate::commands;
use crate::encoding;
use crate::editing;
use crate::interpreter::RunOptions;
use crate::interpreter::debug::{self as vb_debug, DebugControl, DebugSnapshot};
use crate::explorer::{ExplorerModel, Root};
use crate::layout::{LAYOUT_VERSION, Layout, SESSION_LIMIT};
use crate::menu::{self, MenuState};
use crate::outline::{self, OutlineModel, WatchEntry, WatchState};
use crate::output::{OutputHandle, OutputLevel, OutputLog};
use crate::panels::{
    EditorPanel, ExplorerPanel, OutlineDebugState, OutlinePanel, OutputPanel, SearchPanel,
};
use crate::settings::{
    AppSettings, AppearanceSettings, PanelKind, SettingsStore, SideSlot, ThemeChoice,
};
use crate::search;
use crate::status_bar::{self, StatusInfo};
use crate::theme::{self as ui_theme, Palette};
use crate::title_bar;
use crate::workspace::{self, OpenFile};
use gpui_kit::component::dialog::{Cancel, DialogFooter};

mod ai;
mod breakpoints;
mod debug;
mod dialogs;
mod dock;
mod editor;
mod files;
mod session;
mod view;

actions!(
    first,
    [
        SaveFile,
        SaveFileAs,
        NewFile,
        OpenFiles,
        RunScript,
        StopScript,
        DebugRun,
        DebugPause,
        DebugResume,
        DebugStep,
        ToggleBreakpoint,
        RunToCursor,
        ClearBreakpoints,
        GoToDefinition,
        FindReferences,
        GoToLine,
        GoToFile,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleSearch,
        ToggleReplace,
        ToggleSearchPanel,
        MoveLineUp,
        MoveLineDown,
        ToggleComment,
        SaveAllFiles,
        NextTab,
        PreviousTab,
        ShowCommands,
        ReopenClosedFile
    ]
);

/// 注册窗口级快捷键。
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-s", SaveFile, None),
        KeyBinding::new("ctrl-shift-s", SaveFileAs, None),
        KeyBinding::new("ctrl-n", NewFile, None),
        KeyBinding::new("ctrl-o", OpenFiles, None),
        KeyBinding::new("f5", RunScript, None),
        KeyBinding::new("shift-f5", StopScript, None),
        // 调试：F9 切换断点、Ctrl+F5 启动调试、F10 单步、F11 继续。
        KeyBinding::new("f9", ToggleBreakpoint, None),
        KeyBinding::new("ctrl-f5", DebugRun, None),
        KeyBinding::new("f10", DebugStep, None),
        KeyBinding::new("f11", DebugResume, None),
        // 运行到光标处 / 清除所有断点。
        KeyBinding::new("ctrl-f10", RunToCursor, None),
        KeyBinding::new("ctrl-shift-f9", ClearBreakpoints, None),
        // 转到定义 / 查找引用。
        KeyBinding::new("f12", GoToDefinition, None),
        KeyBinding::new("shift-f12", FindReferences, None),
        // 转到行 / 符号。
        KeyBinding::new("ctrl-g", GoToLine, None),
        // 对话框里再按一次 Ctrl+G 等同「取消」：关闭当前对话框。
        // 必须排在 GoToLine 之后，同深度时后注册的绑定优先。
        KeyBinding::new("ctrl-g", Cancel, Some("Dialog")),
        KeyBinding::new("ctrl-g", Cancel, Some("Dialog > Input")),
        // 转到文件（按文件名模糊匹配）。
        KeyBinding::new("ctrl-p", GoToFile, None),
        KeyBinding::new("ctrl-p", Cancel, Some("Dialog")),
        KeyBinding::new("ctrl-p", Cancel, Some("Dialog > Input")),
        // 项目内搜索面板（在文件里搜内容）。
        KeyBinding::new("ctrl-shift-f", ToggleSearchPanel, None),
        // 编辑增强：整行上移/下移、切换行注释、保存全部。
        KeyBinding::new("alt-up", MoveLineUp, None),
        KeyBinding::new("alt-down", MoveLineDown, None),
        KeyBinding::new("ctrl-/", ToggleComment, None),
        KeyBinding::new("ctrl-alt-s", SaveAllFiles, None),
        // 标签切换。
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        // 命令面板：按名字模糊匹配后执行（菜单里有同样的入口）。
        KeyBinding::new("ctrl-shift-p", ShowCommands, None),
        KeyBinding::new("ctrl-shift-p", Cancel, Some("Dialog")),
        KeyBinding::new("ctrl-shift-p", Cancel, Some("Dialog > Input")),
        // 重新打开最近关闭的标签。
        KeyBinding::new("ctrl-shift-t", ReopenClosedFile, None),
        // 查找 / 替换：由主窗口处理，再按一次同一快捷键收起面板。
        // 使用更具体的上下文覆盖编辑器内置的 `Search` / `Replace` 绑定。
        KeyBinding::new("ctrl-f", ToggleSearch, Some("Input && mode == editor")),
        KeyBinding::new("ctrl-h", ToggleReplace, Some("Input && mode == editor")),
        // 焦点在查找框里时，内层普通输入会以自己的 `Input` 上下文覆盖上面的绑定，
        // 因此再补一条更深的绑定，保证同一快捷键仍能关闭面板。
        KeyBinding::new("ctrl-f", ToggleSearch, Some("SearchPanel > Input")),
        KeyBinding::new("ctrl-h", ToggleReplace, Some("SearchPanel > Input")),
        // 编辑器字号。
        KeyBinding::new("ctrl-=", ZoomIn, None),
        KeyBinding::new("ctrl--", ZoomOut, None),
        KeyBinding::new("ctrl-0", ZoomReset, None),
        // 候选框通常随输入自动弹出，Ctrl+Space 用来手动唤出（例如空前缀列出全部）。
        KeyBinding::new(
            "ctrl-space",
            ShowCompletions,
            Some("Input && mode == editor"),
        ),
    ]);
}

pub struct CodeWindow {
    /// 当前标签的代码编辑框状态（内容、高亮、选区、滚动与撤销历史等）。
    editor: Entity<EditorState>,
    /// 没有打开文件时使用的空编辑器。
    empty_editor: Entity<EditorState>,
    /// 资源管理器的树状态。
    tree: Entity<TreeState>,
    /// 资源管理器的数据模型：根目录列表与展开状态。
    explorer: ExplorerModel,
    /// 输出日志，外部通过 [`CodeWindow::output`] 拿到写入句柄。
    output: Entity<OutputLog>,
    /// Dock 布局容器：资源管理器/大纲、编辑区与输出面板都由它管理。
    dock_area: Entity<DockArea>,
    /// Dock 面板实体。面板自己持有渲染数据，主窗口只负责同步状态。
    editor_panel: Entity<EditorPanel>,
    explorer_panel: Entity<ExplorerPanel>,
    outline_panel: Entity<OutlinePanel>,
    search_panel: Entity<SearchPanel>,
    /// AI 编程面板。
    ai_panel: Entity<AiPanel>,
    output_panel: Entity<OutputPanel>,
    /// 全局应用设置。
    settings: Entity<AppSettings>,
    open_files: Vec<OpenFile>,
    /// 与 `open_files` 一一对应，保存每个标签当前的内容。
    contents: Vec<SharedString>,
    /// 与 `open_files` 一一对应，每个标签独立的编辑器状态：
    /// 切换标签时保留光标、选区、滚动位置、折叠与撤销历史。
    editors: Vec<Entity<EditorState>>,
    /// 与 `editors` 一一对应，监听各标签内容变化；关闭标签时一并释放。
    editor_subscriptions: Vec<Subscription>,
    /// 与 `open_files` 一一对应，保存上次打开/保存时的内容，用来判断未保存修改。
    saved: Vec<SharedString>,
    active_file: Option<usize>,
    /// 树形框当前宽度。
    explorer_width: f32,
    /// 代码大纲侧栏当前宽度。
    outline_width: f32,
    /// 搜索面板当前宽度。
    search_width: f32,
    /// AI 编程面板当前宽度。
    ai_width: f32,
    /// 窗口位置与大小：拖动/缩放后由 `observe_window_bounds` 更新，随后写进布局文件。
    window_bounds: Option<(f32, f32, f32, f32)>,
    /// 窗口矩形变了、还没写盘。
    window_bounds_dirty: bool,
    /// 设置窗口上次的大小：`(宽, 高)`；这次没打开设置时沿用文件里的值。
    settings_size: Option<(f32, f32)>,
    /// 输出组件当前高度。
    output_height: f32,
    /// 当前编辑内容对应的代码大纲。
    outline_model: OutlineModel,
    outline_source: Option<String>,
    /// 断点：文件 id → 1 基行号。
    breakpoints: HashMap<String, HashSet<usize>>,
    /// 条件断点：文件 id → 行号 → 条件表达式（没有条目的断点是无条件断点）。
    breakpoint_conditions: HashMap<String, HashMap<usize, String>>,
    /// 「运行到光标处」留下的临时断点：命中断点后撤掉。
    run_to: Option<usize>,
    /// 最近打开过的文件（最新的在前），持久化在布局文件里。
    recent_files: Vec<String>,
    /// 最近关闭的标签路径（后进先出），供「重新打开关闭的标签」使用。
    closed_files: Vec<String>,
    /// 与 `open_files` 一一对应：上次读盘/写盘时文件的修改时间，用来发现外部修改。
    disk_stamps: Vec<Option<SystemTime>>,
    /// 「运行参数」：脚本里 `Command()` 读到的字符串。
    script_args: String,
    /// 与 `editors` 一一对应：断点在行号栏里的小圆点。
    breakpoint_marks: Vec<Option<GutterDecorationCollection>>,
    /// 与 `editors` 一一对应：暂停时高亮当前执行行。
    paused_line_marks: Vec<Option<RangeDecorationCollection>>,
    /// 括号配对高亮：与标签一一对应，切换文件时各自保留。
    bracket_marks: Vec<Option<RangeDecorationCollection>>,
    /// 每个标签上一次画过的括号高亮。
    ///
    /// `RangeDecorationCollection::set` 每次都会 `notify`，如果无条件调用，
    /// 编辑器观察者会把自己再次唤醒，形成死循环（表现为启动即 100% CPU）。
    bracket_pairs: Vec<BracketHighlight>,
    /// 正在进行的调试会话。
    debug: Option<DebugSession>,
    /// 监视表达式（持久化在布局文件里）。
    watches: Vec<String>,
    /// 与 `watches` 一一对应：最近一次求值状态。
    watch_states: Vec<WatchState>,
    /// 上一次应用到编辑器的高亮语言，避免重复刷新语法树。
    applied_language: Option<SharedString>,
    /// 编辑器当前是否装有 VB 补全提供者。
    completion_active: bool,
    /// 上一次做过语法检查的编辑框文本；光标移动、滚动等变化不必重复解析。
    checked_source: Option<String>,
    /// 上次自动保存的时间。
    last_auto_save: Instant,
    /// 当前文本里的语法错误数量（编辑器诊断只报第一处，所以是 0 或 1）。
    error_count: usize,
    /// 正在运行的脚本的中断标志。
    script_interrupt: Option<Arc<AtomicBool>>,
    /// 新建文件的目标目录：最近点过的文件夹，或最近打开文件所在的目录。
    new_file_target: Option<PathBuf>,
    /// 上一次同步给 Dock 的左右槽位与输出开关。
    panel_visibility: Option<(Option<PanelKind>, Option<PanelKind>, bool)>,
    /// 侧栏面板拖动时的自定义停靠指示，避免 Dock 自带的半区占位框和
    /// 实际“停靠到左侧/右侧 dock”的结果不一致。
    panel_drag: Option<PanelDragState>,
    /// 是否已在轮询当前拖动会话是否结束。
    panel_drag_watch: bool,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, PartialEq)]
struct PanelDragState {
    panel: PanelId,
    target: DockPlacement,
    /// 左侧/右侧指示条宽度；底部指示时为高度。
    extent: f32,
}

/// 一次括号高亮的状态；`Unset` 表示这个标签还没画过。
#[derive(Clone, Copy, PartialEq, Eq)]
enum BracketHighlight {
    Unset,
    None,
    Pair(usize, usize),
}

/// 脚本线程回传给界面的消息。
enum ScriptEvent {
    Line(String),
    Finished(Result<(), ScriptFailure>),
}

/// 脚本失败：消息里已含行号，行号另外保留，便于把光标送回出错行。
struct ScriptFailure {
    message: String,
    line: usize,
}

impl ScriptFailure {
    fn from_error(error: &crate::interpreter::Error) -> Self {
        Self {
            line: error.line(),
            message: error.to_string(),
        }
    }
}

/// 一次调试会话：解释器线程的控制器 + 最近一次暂停快照 + 监视求值进度。
struct DebugSession {
    control: Arc<DebugControl>,
    paused: bool,
    snapshot: Option<DebugSnapshot>,
    /// 这次暂停里还没发出的监视求值下标。
    watch_queue: VecDeque<usize>,
    /// 已经发出、等结果的求值请求：`(请求 id, 监视下标)`。
    watch_request: Option<(u64, usize)>,
}

impl CodeWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let layout = Layout::load();
        let settings = SettingsStore::init(cx);
        // 用上次关闭时保存的布局给设置播种，窗口一打开就是上次的样子。
        SettingsStore::update(cx, |settings| {
            settings.appearance.theme = ThemeChoice::from_str(&layout.theme);
            settings.appearance.left_panel = layout.left_panel;
            settings.appearance.right_panel = layout.right_panel;
            settings.appearance.show_output = layout.show_output;
            settings.appearance.show_status_bar = layout.show_status_bar;
            // 编辑器 / 输出设置整块恢复：字号、换行、自动保存、输出上限等。
            settings.editor = layout.editor.clone();
            settings.output = layout.output.clone();
            settings.ai = layout.ai.clone();
        });
        let current = settings.read(cx).clone();

        let explorer = ExplorerModel::new(roots_from_layout(&layout), layout.expanded.clone());
        let tree = cx.new(|cx| TreeState::new(cx).items(explorer.items()));
        let output = cx.new(OutputLog::new);
        let empty_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("vb")
                .soft_wrap(current.editor.soft_wrap)
                .default_value("")
        });
        // VB 脚本用自定义词法高亮，其余语言沿用 gpui-component 的树语法。
        // 补全提供者由 `refresh_editor_language` 按文件语言挂上。
        empty_editor.update(cx, |editor, cx| {
            editor.set_highlighter_factory(crate::syntax::highlighter_factory(), cx);
        });

        let app_handle = cx.weak_entity();
        let editor_panel = cx.new(|_| {
            EditorPanel::new(
                app_handle.clone(),
                empty_editor.clone(),
                current.editor.font_size,
            )
        });
        let explorer_panel = cx.new(|cx| {
            ExplorerPanel::new(
                app_handle.clone(),
                tree.clone(),
                explorer.roots().is_empty(),
                cx,
            )
        });
        let outline_panel = cx.new(|cx| OutlinePanel::new(app_handle.clone(), window, cx));
        let search_panel = cx.new(|cx| SearchPanel::new(app_handle.clone(), window, cx));
        let ai_panel = cx.new(|cx| AiPanel::new(app_handle.clone(), window, cx));
        let output_panel = cx.new(|cx| {
            OutputPanel::new(
                app_handle.clone(),
                output.clone(),
                current.output.font_size as f32,
                window,
                cx,
            )
        });
        let (dock_area, dock_skin) = DockSkin::dock_area("main", Some(1), window, cx);
        dock_skin.set_panel_style(PanelStyle::Auto, cx);
        dock_skin.set_toggle_button_visible(false, cx);
        dock_skin.set_close_button_visible(false, cx);
        let mut this = Self {
            editor: empty_editor.clone(),
            empty_editor,
            tree,
            explorer,
            output: output.clone(),
            dock_area,
            editor_panel,
            explorer_panel,
            outline_panel,
            search_panel,
            ai_panel,
            output_panel,
            settings: settings.clone(),
            open_files: Vec::new(),
            contents: Vec::new(),
            editors: Vec::new(),
            editor_subscriptions: Vec::new(),
            saved: Vec::new(),
            active_file: None,
            explorer_width: layout
                .explorer_width
                .clamp(ui_theme::EXPLORER_MIN_WIDTH, 480.0),
            outline_width: layout
                .outline_width
                .clamp(ui_theme::OUTLINE_MIN_WIDTH, ui_theme::OUTLINE_MAX_WIDTH),
            search_width: layout
                .search_width
                .clamp(ui_theme::SEARCH_MIN_WIDTH, ui_theme::SEARCH_MAX_WIDTH),
            ai_width: layout
                .ai_width
                .clamp(ui_theme::AI_MIN_WIDTH, ui_theme::AI_MAX_WIDTH),
            window_bounds: layout.window_bounds,
            window_bounds_dirty: false,
            settings_size: layout.settings_size,
            output_height: layout
                .output_height
                .clamp(ui_theme::OUTPUT_MIN_HEIGHT, 800.0),
            outline_model: OutlineModel::default(),
            outline_source: None,
            breakpoints: layout
                .breakpoints
                .iter()
                .map(|(path, lines)| (path.clone(), lines.iter().copied().collect()))
                .collect(),
            breakpoint_conditions: layout
                .breakpoint_conditions
                .iter()
                .map(|(path, conditions)| {
                    let lines = layout
                        .breakpoints
                        .iter()
                        .find(|(file, _)| file == path)
                        .map(|(_, lines)| lines)
                        .cloned()
                        .unwrap_or_default();
                    // 只保留仍然存在的断点上的条件，避免布局文件里的脏数据挂到别的行。
                    let conditions = conditions
                        .iter()
                        .filter(|(line, _)| lines.contains(line))
                        .cloned()
                        .collect();
                    (path.clone(), conditions)
                })
                .collect(),
            run_to: None,
            recent_files: layout.recent.clone(),
            closed_files: Vec::new(),
            disk_stamps: Vec::new(),
            script_args: layout.script_args.clone(),
            watches: layout.watches.clone(),
            watch_states: Vec::new(),
            breakpoint_marks: Vec::new(),
            paused_line_marks: Vec::new(),
            bracket_marks: Vec::new(),
            bracket_pairs: Vec::new(),
            debug: None,
            applied_language: None,
            completion_active: false,
            checked_source: None,
            last_auto_save: Instant::now(),
            error_count: 0,
            script_interrupt: None,
            new_file_target: None,
            panel_visibility: None,
            panel_drag: None,
            panel_drag_watch: false,
            _subscriptions: Vec::new(),
        };

        this.setup_dock(&layout.dock_state, window, cx);
        // 恢复上次退出时还开着的标签（文件已被删除/移走的自然跳过）。
        this.restore_session(&layout, window, cx);

        // 输出变化后重绘窗口；点了带行号的输出行就跳到对应代码行。
        let output_subscription = cx.observe_in(&output, window, |this, output, window, cx| {
            if let Some(line) = output.update(cx, |log, _| log.take_clicked_line()) {
                this.go_to_line(line, window, cx);
            }
            cx.notify();
        });
        // 面板被拖走后，某些 dock 可能只剩隐藏面板；空 dock 仍会占着宽度/高度。
        // 布局变化后延迟检查一次，把没有可见面板的 dock 收起或移除。
        let dock_subscription =
            cx.subscribe_in(&this.dock_area, window, |_this, _, event: &DockEvent, window, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    cx.defer_in(window, |this, window, cx| this.normalize_dock_docks(window, cx));
                }
            });
        // 编辑内容变化时同步到当前标签，标签上的未保存圆点才能实时更新；
        // 顺带做一次语法检查，把错误画成编辑器里的波浪线。
        let editor_subscription = cx.observe_in(&this.editor, window, |this, editor, _, cx| {
            if !this.is_active_editor(&editor) {
                return;
            }
            this.sync_active_content(cx);
            this.refresh_outline(cx);
            this.refresh_diagnostics(cx);
            this.sync_breakpoints_from_marks(cx);
            this.refresh_bracket_match(cx);
        });
        // 设置变化后把新值应用到界面（主题、编辑器选项、输出选项……）。
        let settings_subscription = cx.observe_in(&settings, window, |this, _, window, cx| {
            this.sync_settings(window, cx);
            this.refresh_outline(cx);
        });
        this._subscriptions.push(output_subscription);
        this._subscriptions.push(dock_subscription);
        this._subscriptions.push(editor_subscription);
        this._subscriptions.push(settings_subscription);
        // 拖动/缩放窗口后记下新矩形，下一次每秒检查时写进布局文件。
        let bounds_subscription =
            cx.observe_window_bounds(window, |this, window, cx| {
                this.remember_window_bounds(window, cx);
            });
        this._subscriptions.push(bounds_subscription);

        this.sync_settings(window, cx);
        this.spawn_auto_save_timer(window, cx);
        // 窗口关闭前检查未保存修改。
        let close_handle = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            close_handle
                .upgrade()
                .map(|this| this.update(cx, |this, cx| this.request_window_close(window, cx)))
                .unwrap_or(true)
        });
        // 启动后编辑框直接拿到焦点，快捷键（Ctrl+S 等）无需先点一下窗口。
        this.editor
            .update(cx, |editor, cx| editor.focus(window, cx));
        this
    }

    /// 输出接口：任何模块都可以拿到它写日志。
    pub fn output(&self) -> OutputHandle {
        OutputHandle::new(self.output.clone())
    }

    pub fn active_file_index(&self) -> Option<usize> {
        self.active_file
    }

    /// 演示“关于”：把版本信息写到输出面板。
    pub(crate) fn show_about(&mut self, cx: &mut App) {
        let output = self.output();
        output.update(cx, |log, _| log.set_channel("关于"));
        output.info(cx, crate::title_bar::APP_TITLE);
        output.info(cx, "基于 gpui / gpui-kit 构建的示例 IDE 界面");
        output.success(cx, "界面、编辑器、输出面板与设置均已模块化");
    }

    /// 把当前设置应用到编辑器、主题与输出面板。
    pub(super) fn sync_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = SettingsStore::snapshot(cx);

        let active = ui_theme::resolve(&settings.appearance.theme, cx);
        if cx.try_global::<ui_theme::ActivePalette>() != Some(&active) {
            ui_theme::apply(active, cx);
        }

        let editor_settings = settings.editor.clone();
        let editors: Vec<Entity<EditorState>> = std::iter::once(self.empty_editor.clone())
            .chain(self.editors.iter().cloned())
            .collect();
        for editor in editors {
            editor.update(cx, |editor, cx| {
                editor.set_soft_wrap(editor_settings.soft_wrap, window, cx);
                editor.set_line_number(editor_settings.line_numbers, window, cx);
                editor.set_folding(editor_settings.folding, window, cx);
                editor.set_auto_close(editor_settings.auto_close, window, cx);
            });
        }

        let output = self.output.clone();
        let output_settings = settings.output;
        output.update(cx, |log, cx| {
            log.set_max_lines(output_settings.max_lines);
            log.set_show_timestamps(output_settings.show_timestamps, cx);
        });

        self.sync_editor_panel(cx);
        self.sync_output_panel(cx);
        self.apply_dock_visibility(window, cx);
        self.refresh_editor_language(cx);
        // 主题与面板开关也属于布局，变化后顺手写盘。
        self.save_layout(cx);
        cx.notify();
    }
}

/// 树条目的路径改名/删除后，把记录里的路径一起改写。
fn reprefix(value: &str, old: &str, new: &str) -> Option<String> {
    workspace::reprefix_path(value, old, new)
}

/// 在目录里找一个还没被占用的 `新建文件*.vb` 路径。
fn free_vb_path(dir: &Path) -> PathBuf {
    let first = dir.join("新建文件.vb");
    if !first.exists() {
        return first;
    }
    for n in 2..1000 {
        let candidate = dir.join(format!("新建文件{n}.vb"));
        if !candidate.exists() {
            return candidate;
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    dir.join(format!("新建文件_{stamp}.vb"))
}

/// 从布局文件恢复资源管理器的根项；磁盘上已不存在的文件夹会被丢弃。
fn roots_from_layout(layout: &Layout) -> Vec<Root> {
    layout
        .roots
        .iter()
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .map(Root::new)
        .collect()
}

/// 出错行的 UTF-8 字节范围（`line` 为 1-based，行尾不含换行）。
fn line_byte_range(source: &str, line: usize) -> Range<usize> {
    let target = line.max(1);
    let mut start = 0usize;
    let mut current = 1usize;
    for (offset, ch) in source.char_indices() {
        if ch != '\n' {
            continue;
        }
        if current == target {
            return start..offset;
        }
        current += 1;
        start = offset + ch.len_utf8();
    }
    if current == target {
        start..source.len()
    } else {
        source.len()..source.len()
    }
}

/// 把光标送回出错行（选中整行并标注错误诊断），方便直接定位。
fn reveal_error_line(
    editor: &mut EditorState,
    source: &str,
    line: usize,
    message: &str,
    cx: &mut Context<EditorState>,
) {
    let range = line_byte_range(source, line);
    let character = source[range.clone()].chars().count() as u32;
    editor.set_selected_range(range, cx);

    let text = editor.text().clone();
    let lines = text.lines_len();
    if line == 0 || line > lines {
        cx.notify();
        return;
    }
    let row = (line - 1).min(lines.saturating_sub(1)) as u32;
    if let Some(diagnostics) = editor.diagnostics_mut() {
        diagnostics.reset(&text);
        diagnostics.push(
            Diagnostic::new(Position::new(row, 0)..Position::new(row, character), message)
                .with_severity(DiagnosticSeverity::Error)
                .with_source("vb"),
        );
    }
    cx.notify();
}

/// 把调用栈写成一行，例如 `模块 → Main → Calc`；层数太多时只留最里面几层。
fn call_stack_label(snapshot: &vb_debug::DebugSnapshot) -> String {
    const MAX_DEPTH: usize = 3;

    let frames = &snapshot.call_stack;
    if frames.len() <= 1 {
        return String::new();
    }
    let start = frames.len().saturating_sub(MAX_DEPTH);
    let names: Vec<&str> = frames[start..]
        .iter()
        .map(|frame| frame.procedure.as_str())
        .collect();
    let label = names.join(" → ");
    if start > 0 {
        format!("… → {label}")
    } else {
        label
    }
}
