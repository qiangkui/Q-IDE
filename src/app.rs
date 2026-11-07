//! 主窗口：负责整体布局、标签页状态，以及把设置同步到界面。

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
use crate::settings::{AppSettings, AppearanceSettings, SettingsStore, ThemeChoice};
use crate::search;
use crate::status_bar::{self, StatusInfo};
use crate::theme::{self as ui_theme, Palette};
use crate::title_bar;
use crate::workspace::{self, OpenFile};
use gpui_kit::component::dialog::{Cancel, DialogFooter};

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
    /// 窗口位置与大小：拖动/缩放后由 `observe_window_bounds` 更新，随后写进布局文件。
    window_bounds: Option<(f32, f32, f32, f32)>,
    /// 窗口矩形变了、还没写盘。
    window_bounds_dirty: bool,
    /// 设置窗口上次的大小：`(宽, 高)`；这次没打开设置时沿用文件里的值。
    settings_size: Option<(f32, f32)>,
    /// 输出组件当前高度。
    output_height: f32,
    /// 侧栏（资源管理器/代码大纲）最近一次停靠的一侧。
    /// 面板互斥时，新打开的面板应该出现在用户上一次使用的侧栏，而不是固定回左侧。
    side_dock: DockPlacement,
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
    /// 上一次同步给 Dock 的面板开关（资源管理器、代码大纲、输出）。
    panel_visibility: Option<(bool, bool, bool, bool)>,
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
            settings.appearance.show_explorer = layout.show_explorer;
            settings.appearance.show_outline = layout.show_outline;
            settings.appearance.show_search = layout.show_search;
            settings.appearance.show_output = layout.show_output;
            settings.appearance.show_status_bar = layout.show_status_bar;
            // 编辑器 / 输出设置整块恢复：字号、换行、自动保存、输出上限等。
            settings.editor = layout.editor.clone();
            settings.output = layout.output.clone();
            // 资源管理器与代码大纲互斥；旧布局可能两个都开着，默认保留资源管理器。
            settings.appearance.normalize_side_panels();
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
            window_bounds: layout.window_bounds,
            window_bounds_dirty: false,
            settings_size: layout.settings_size,
            output_height: layout
                .output_height
                .clamp(ui_theme::OUTPUT_MIN_HEIGHT, 800.0),
            side_dock: if layout.side_dock == "right" {
                DockPlacement::Right
            } else {
                DockPlacement::Left
            },
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
        this.refresh_side_dock(cx);
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

    /// 当前脚本在磁盘上的路径：相对路径按它所在目录解析，`App.Path` 也用这个目录。
    fn active_script_path(&self) -> Option<PathBuf> {
        self.active_file
            .and_then(|index| self.open_files.get(index))
            .map(|file| PathBuf::from(file.id.to_string()))
    }

    /// 运行当前编辑框里的 VB 脚本。
    ///
    /// 只有 VB 脚本才能运行；其它语言的文件直接在输出面板提示无法运行。
    /// 脚本在独立线程里执行（`MsgBox` 之类的阻塞调用不会冻结窗口），
    /// 输出的每一行通过通道回到界面线程，追加到输出面板。
    pub(crate) fn run_script(&mut self, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        let source = self.editor.read(cx).value().to_string();
        let output = self.output();
        output.update(cx, |log, _| log.set_channel("vb 脚本"));

        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            output.error(cx, "无法运行：当前文件不是 VB 脚本");
            return;
        }

        if source.trim().is_empty() {
            output.warning(cx, "当前文件没有内容，无法运行");
            return;
        }

        // 上一次脚本（或暂停中的调试会话）还没结束就先结束它。
        if let Some(session) = &self.debug {
            session.control.stop();
        }
        self.debug = None;
        self.show_debug_line(None, cx);
        if let Some(interrupt) = self.script_interrupt.take() {
            interrupt.store(true, Ordering::Relaxed);
        }
        // 上一次脚本可能卡在对话框上，先把它关掉，否则那个线程会一直挂着。
        crate::interpreter::ffi::close_dialogs();

        output.command(cx, "> 运行 VB 脚本");

        let interrupt = Arc::new(AtomicBool::new(false));
        self.script_interrupt = Some(interrupt.clone());
        let options = RunOptions {
            command_line: self.script_args.clone(),
            script_path: self.active_script_path(),
            ..RunOptions::default()
        };

        let (sender, receiver) = mpsc::channel::<ScriptEvent>();
        std::thread::spawn(move || {
            let line_sender = sender.clone();
            let result = crate::interpreter::run_with(
                &source,
                move |line| {
                    let _ = line_sender.send(ScriptEvent::Line(line.to_string()));
                },
                &interrupt,
                options,
            );
            let failure = result.map_err(|error| ScriptFailure::from_error(&error));
            let _ = sender.send(ScriptEvent::Finished(failure));
        });

        let jump_source = self.editor.read(cx).value().to_string();
        self.spawn_script_events(receiver, jump_source, None, cx);
    }

    /// 把脚本线程的事件搬到界面上；调试会话还会轮询暂停状态并高亮当前行。
    fn spawn_script_events(
        &self,
        receiver: mpsc::Receiver<ScriptEvent>,
        source: String,
        debug: Option<Arc<DebugControl>>,
        cx: &mut Context<Self>,
    ) {
        let log = self.output.clone();
        let editor = self.editor.clone();
        // 脚本从启动到结束的墙钟时间（调试时的暂停也算在内）。
        let started = Instant::now();
        cx.spawn(async move |this, cx| {
            let mut last_debug_state: Option<(bool, u64)> = Some((false, 0));
            loop {
                loop {
                    match receiver.try_recv() {
                        Ok(ScriptEvent::Line(text)) => {
                            log.update(cx, |log, cx| log.push(OutputLevel::Info, text, cx));
                        }
                        Ok(ScriptEvent::Finished(result)) => {
                            let stopped = result.as_ref().err().is_some_and(|failure| {
                                failure.message.contains(vb_debug::STOPPED_MESSAGE)
                            });
                            match result {
                                Ok(()) => {
                                    let elapsed = started.elapsed().as_secs_f64();
                                    log.update(cx, |log, cx| {
                                        log.push(
                                            OutputLevel::Success,
                                            format!("脚本执行结束（用时 {elapsed:.2}s）"),
                                            cx,
                                        )
                                    });
                                    editor.update(cx, crate::lint::clear);
                                }
                                Err(_) if stopped => {
                                    let elapsed = started.elapsed().as_secs_f64();
                                    log.update(cx, |log, cx| {
                                        log.push(
                                            OutputLevel::Warning,
                                            format!("脚本已停止（用时 {elapsed:.2}s）"),
                                            cx,
                                        )
                                    });
                                }
                                Err(failure) => {
                                    log.update(cx, |log, cx| {
                                        log.push(OutputLevel::Error, failure.message.clone(), cx)
                                    });
                                    editor.update(cx, |editor, cx| {
                                        reveal_error_line(
                                            editor,
                                            &source,
                                            failure.line,
                                            &failure.message,
                                            cx,
                                        );
                                    });
                                }
                            }
                            let _ = this.update(cx, |this, cx| {
                                // 旧会话的结束事件不能把刚启动的新会话清掉。
                                let is_current = match (&debug, &this.debug) {
                                    (Some(finished), Some(current)) => {
                                        Arc::ptr_eq(finished, &current.control)
                                    }
                                    (None, current) => current.is_none(),
                                    (Some(_), None) => true,
                                };
                                if is_current {
                                    this.debug = None;
                                    this.watch_states =
                                        vec![WatchState::Inactive; this.watches.len()];
                                    this.show_debug_line(None, cx);
                                    this.sync_outline_panel(cx);
                                    cx.notify();
                                }
                            });
                            return;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }

                if let Some(control) = &debug {
                    // 只在「暂停状态 / 暂停次数」变化时取快照：变量快照可能有几 MB，
                    // 每 40ms 克隆一次会让菜单、面板开关明显发卡。
                    // 求值结果很轻，每轮轮询一次，有结果才碰界面。
                    let state = (control.is_paused(), control.pause_epoch());
                    let changed = last_debug_state != Some(state);
                    let outcome = if state.0 {
                        control.take_eval_result()
                    } else {
                        None
                    };
                    if changed || outcome.is_some() {
                        if changed {
                            last_debug_state = Some(state);
                        }
                        let snapshot = if changed && state.0 {
                            control.snapshot()
                        } else {
                            None
                        };
                        let _ = this.update(cx, |this, cx| {
                            let mut dirty = false;
                            let mut run_to_hit = false;
                            if changed {
                                let mut current_line = None;
                                let mut matched = false;
                                if let Some(session) = &mut this.debug
                                    && Arc::ptr_eq(control, &session.control)
                                {
                                    session.paused = state.0;
                                    session.snapshot = snapshot;
                                    session.watch_request = None;
                                    if state.0 {
                                        // 每次暂停都重新求值：变量可能已经变了。
                                        session.watch_queue =
                                            (0..this.watches.len()).collect();
                                        this.watch_states =
                                            vec![WatchState::Pending; this.watches.len()];
                                    } else {
                                        session.watch_queue.clear();
                                        this.watch_states =
                                            vec![WatchState::Inactive; this.watches.len()];
                                    }
                                    let line = session
                                        .snapshot
                                        .as_ref()
                                        .filter(|_| session.paused)
                                        .map(|snapshot| snapshot.line);
                                    run_to_hit = line.is_some() && this.run_to == line;
                                    current_line = Some(line);
                                    matched = true;
                                    dirty = true;
                                }
                                if matched {
                                    this.show_debug_line(current_line.flatten(), cx);
                                }
                            }
                            if let Some(outcome) = outcome {
                                let mut applied = false;
                                if let Some(session) = &mut this.debug
                                    && Arc::ptr_eq(control, &session.control)
                                    && session
                                        .watch_request
                                        .is_some_and(|(id, _)| id == outcome.id)
                                {
                                    if let Some((_, index)) = session.watch_request.take()
                                        && let Some(slot) = this.watch_states.get_mut(index)
                                    {
                                        *slot = match outcome.result {
                                            Ok(value) => WatchState::Ready {
                                                value: value.value,
                                                ty: value.ty,
                                            },
                                            Err(message) => WatchState::Failed(message),
                                        };
                                    }
                                    applied = true;
                                }
                                dirty |= applied;
                            }
                            // 一条求值完成后接着发下一条。
                            dirty |= this.pump_watch_requests();
                            // 「运行到光标处」的临时断点命中一次就撤掉。
                            if run_to_hit {
                                this.run_to = None;
                                let breakpoints = this.active_breakpoints();
                                if let Some(session) = &this.debug {
                                    session.control.set_breakpoints(breakpoints);
                                }
                            }
                            if dirty {
                                this.sync_outline_panel(cx);
                                cx.notify();
                            }
                        });
                    }
                }

                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
            }
        })
        .detach();
    }

    /// 带调试器运行当前脚本（`Ctrl+F5`）；已经停顿的会话用 `F11` 继续。
    pub(crate) fn start_debug(&mut self, cx: &mut Context<Self>) {
        self.begin_debug(false, cx);
    }

    /// 启动调试会话；`pause_first` 为真时在第一条语句处就停下（单步启动）。
    fn begin_debug(&mut self, pause_first: bool, cx: &mut Context<Self>) {
        self.begin_debug_with(pause_first, None, cx);
    }

    /// 启动调试会话，并额外在一个临时断点处停下（「运行到光标处」）。
    fn begin_debug_with(
        &mut self,
        pause_first: bool,
        run_to: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.save_active_content(cx);
        let output = self.output();
        output.update(cx, |log, _| log.set_channel("vb 调试"));

        if let Some(session) = &self.debug {
            if !session.control.is_finished() {
                output.warning(cx, "已经在调试中：F11 继续、F10 单步、Shift+F5 停止");
                return;
            }
            // 上一次会话已经结束（界面还没来得及收尾），直接换成新的。
            self.debug = None;
            self.show_debug_line(None, cx);
        }
        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            output.error(cx, "无法调试：当前文件不是 VB 脚本");
            return;
        }

        let source = self.editor.read(cx).value().to_string();
        if source.trim().is_empty() {
            output.warning(cx, "当前文件没有内容，无法调试");
            return;
        }

        // 上一次脚本还没结束就先中断它。
        if let Some(interrupt) = self.script_interrupt.take() {
            interrupt.store(true, Ordering::Relaxed);
        }

        output.command(cx, "> 调试 VB 脚本");

        let interrupt = Arc::new(AtomicBool::new(false));
        self.script_interrupt = Some(interrupt.clone());
        let mut breakpoints = self.active_breakpoints();
        if let Some(line) = run_to {
            breakpoints.insert(line);
        }
        self.run_to = run_to;
        let control = Arc::new(DebugControl::new(breakpoints));
        control.set_breakpoint_conditions(self.active_breakpoint_conditions());
        if pause_first {
            control.pause();
        }
        // 上一次会话留下的监视结果已经过期。
        self.watch_states = vec![WatchState::Inactive; self.watches.len()];
        self.debug = Some(DebugSession {
            control: control.clone(),
            paused: false,
            snapshot: None,
            watch_queue: VecDeque::new(),
            watch_request: None,
        });

        let (sender, receiver) = mpsc::channel::<ScriptEvent>();
        let thread_interrupt = interrupt;
        let thread_control = control.clone();
        let options = RunOptions {
            command_line: self.script_args.clone(),
            script_path: self.active_script_path(),
            ..RunOptions::default()
        };
        std::thread::spawn(move || {
            let line_sender = sender.clone();
            let result = crate::interpreter::run_debug(
                &source,
                move |line| {
                    let _ = line_sender.send(ScriptEvent::Line(line.to_string()));
                },
                &thread_interrupt,
                options,
                thread_control,
            );
            let failure = result.map_err(|error| ScriptFailure::from_error(&error));
            let _ = sender.send(ScriptEvent::Finished(failure));
        });

        self.spawn_script_events(receiver, self.editor.read(cx).value().to_string(), Some(control), cx);
    }

    /// 在下一行暂停（菜单「运行 → 暂停」）。
    pub(crate) fn debug_pause(&mut self, cx: &mut Context<Self>) {
        match &self.debug {
            Some(session) => {
                session.control.pause();
                self.output().command(cx, "> 暂停：下一条语句处停下");
            }
            None => self.output().warning(cx, "当前没有调试会话"),
        }
    }

    /// 继续运行到下一个断点；没有会话时直接启动调试运行。
    pub(crate) fn debug_resume(&mut self, cx: &mut Context<Self>) {
        match &self.debug {
            Some(session) => {
                session.control.resume();
                self.output().command(cx, "> 继续");
            }
            None => self.begin_debug(false, cx),
        }
    }

    /// 单步：没有会话时启动调试并停在第一条语句。
    pub(crate) fn debug_step(&mut self, cx: &mut Context<Self>) {
        match &self.debug {
            Some(session) => {
                session.control.step();
                self.output().command(cx, "> 单步");
            }
            None => self.begin_debug(true, cx),
        }
    }

    /// 打开「添加监视」对话框：暂停时输入的表达式会在每次暂停时求值。
    pub(crate) fn add_watch_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个。
            return;
        }
        // 光标停在某个名字上时先填进去，省得手输。
        let current = {
            let editor = self.editor.read(cx);
            let text = editor.text();
            let offset = editor.cursor().min(text.len());
            Self::word_at(text, offset).unwrap_or_default()
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("表达式，例如 total + 1 或 p.Name")
                .default_value(current)
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle.update(cx, |this, cx| this.add_watch(&text, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "watch",
                title: "添加监视",
                description: Some("脚本暂停时在「代码大纲 → 监视」里显示结果。"),
                confirm: "添加",
            },
            input,
            apply,
        );
    }

    /// 记住一条监视表达式；正在暂停时立刻排队求值。
    pub(crate) fn add_watch(&mut self, expression: &str, cx: &mut Context<Self>) {
        let expression = expression.trim().to_string();
        if expression.is_empty() {
            return;
        }
        if self
            .watches
            .iter()
            .any(|watch| watch.eq_ignore_ascii_case(&expression))
        {
            self.output()
                .warning(cx, format!("已经在监视 `{expression}`"));
            return;
        }

        self.watches.push(expression.clone());
        self.watch_states.push(WatchState::Inactive);
        let index = self.watches.len() - 1;
        if let Some(session) = &mut self.debug
            && session.paused
        {
            session.watch_queue.push_back(index);
            self.watch_states[index] = WatchState::Pending;
        }
        self.pump_watch_requests();
        self.save_layout(cx);
        self.sync_outline_panel(cx);
        self.output()
            .command(cx, format!("> 添加监视：{expression}（显示在「代码大纲」面板）"));
        cx.notify();
    }

    /// 清空监视列表。
    pub(crate) fn clear_watches(&mut self, cx: &mut Context<Self>) {
        if self.watches.is_empty() {
            self.output().warning(cx, "监视列表已经是空的");
            return;
        }
        self.watches.clear();
        self.watch_states.clear();
        if let Some(session) = &mut self.debug {
            session.watch_queue.clear();
            session.watch_request = None;
        }
        self.save_layout(cx);
        self.sync_outline_panel(cx);
        self.output().command(cx, "> 已清除所有监视");
        cx.notify();
    }

    /// 暂停中还有没求值的监视就发出下一条；返回是否发了新请求。
    fn pump_watch_requests(&mut self) -> bool {
        let Some(session) = &mut self.debug else {
            return false;
        };
        if !session.paused || session.watch_request.is_some() {
            return false;
        }
        while let Some(index) = session.watch_queue.pop_front() {
            let Some(expression) = self.watches.get(index) else {
                continue;
            };
            let Some(id) = session.control.request_eval(expression) else {
                // 已经不在暂停状态，本轮不再发。
                return false;
            };
            session.watch_request = Some((id, index));
            if let Some(slot) = self.watch_states.get_mut(index) {
                *slot = WatchState::Pending;
            }
            return true;
        }
        false
    }

    /// 转到定义（`F12`）：光标处的名字在当前脚本里的声明位置。
    pub(crate) fn go_to_definition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            self.output().warning(cx, "转到定义目前只支持 VB 脚本");
            return;
        }
        let (word, line) = {
            let editor = self.editor.read(cx);
            let text = editor.text();
            let offset = editor.cursor().min(text.len());
            let line = text.offset_to_position(offset).line as usize + 1;
            (Self::word_at(text, offset), line)
        };
        let Some(word) = word else {
            self.output().warning(cx, "光标处没有可跳转的名字");
            return;
        };

        let target = outline::definition(&self.outline_model, &word, line)
            .map(|symbol| (symbol.name.clone(), symbol.line));
        let Some((name, target_line)) = target else {
            self.output().warning(cx, format!("找不到 `{word}` 的定义"));
            return;
        };

        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(
                Position::new(target_line.saturating_sub(1) as u32, 0),
                window,
                cx,
            );
        });
        self.output()
            .command(cx, format!("> 转到定义：{name}（第 {target_line} 行）"));
    }

    /// 把光标移到指定行（输出面板里点错误行时用）。
    pub(crate) fn go_to_line(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            let lines = editor.text().lines_len();
            if lines == 0 {
                return;
            }
            let row = line.saturating_sub(1).min(lines - 1) as u32;
            editor.set_cursor_position(Position::new(row, 0), window, cx);
        });
    }

    /// 查找引用（`Shift+F12`）：光标处的名字在脚本里出现的位置，列到输出面板，
    /// 点某一条就跳到那一行。
    pub(crate) fn find_references(&mut self, cx: &mut Context<Self>) {
        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            self.output().warning(cx, "查找引用目前只支持 VB 脚本");
            return;
        }
        let (word, source) = {
            let editor = self.editor.read(cx);
            let text = editor.text();
            let offset = editor.cursor().min(text.len());
            (Self::word_at(text, offset), text.to_string())
        };
        let Some(word) = word else {
            self.output().warning(cx, "光标处没有可查找的名字");
            return;
        };

        let found = outline::references(&source, &word);
        let output = self.output();
        output.update(cx, |log, _| log.set_channel("查找引用"));
        if found.is_empty() {
            output.warning(cx, format!("找不到 `{word}` 的引用"));
            return;
        }
        output.command(cx, format!("> 查找引用：{word}（{} 处）", found.len()));
        for (line, text) in found {
            output.info(cx, format!("第 {line} 行：{text}"));
        }
    }

    /// 调整编辑器字号（`Ctrl+=` / `Ctrl+-` / `Ctrl+0`）；`None` 表示恢复默认。
    pub(crate) fn zoom_editor(&mut self, delta: Option<f32>, cx: &mut Context<Self>) {
        const MIN_SIZE: f32 = 8.0;
        const MAX_SIZE: f32 = 32.0;
        let default = crate::settings::EditorSettings::default().font_size;

        SettingsStore::update(cx, |settings| {
            settings.editor.font_size = match delta {
                Some(delta) => (settings.editor.font_size + delta).clamp(MIN_SIZE, MAX_SIZE),
                None => default,
            };
        });
        let size = self.settings.read(cx).editor.font_size;
        self.output().command(cx, format!("> 编辑器字号：{size}"));
        cx.notify();
    }

    /// 清空「最近打开」列表。
    pub(crate) fn clear_recent_files(&mut self, cx: &mut Context<Self>) {
        if self.recent_files.is_empty() {
            self.output().warning(cx, "最近打开列表已经是空的");
            return;
        }
        self.recent_files.clear();
        self.save_layout(cx);
        self.output().command(cx, "> 已清除最近打开列表");
        cx.notify();
    }

    /// 打开「运行参数」对话框：设置脚本里 `Command()` 读到的字符串。
    pub(crate) fn edit_run_arguments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个。
            return;
        }
        let current = self.script_args.clone();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("例如：input.txt /v")
                .default_value(current)
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle.update(cx, |this, cx| this.set_run_arguments(text, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "args",
                title: "运行参数",
                description: Some("脚本里用 Command() 读取这段文字。"),
                confirm: "确定",
            },
            input,
            apply,
        );
    }

    /// 记住新的运行参数（持久化到布局文件）。
    pub(crate) fn set_run_arguments(&mut self, args: String, cx: &mut Context<Self>) {
        self.script_args = args;
        self.save_layout(cx);
        let shown = if self.script_args.is_empty() {
            "（空）".to_string()
        } else {
            self.script_args.clone()
        };
        self.output().command(cx, format!("> 运行参数：{shown}"));
        cx.notify();
    }

    /// 打开「转到行 / 符号」对话框（`Ctrl+G`；对话框里再按一次会关闭）。
    pub(crate) fn goto_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个（Ctrl+G 在对话框里由「取消」绑定处理）。
            return;
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("行号或过程 / 变量名，例如 42 或 AddOne")
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle.update(cx, |this, cx| this.apply_goto(&text, window, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "goto",
                title: "转到行 / 符号",
                description: None,
                confirm: "转到",
            },
            input,
            apply,
        );
    }

    /// 执行「转到行 / 符号」。
    pub(crate) fn apply_goto(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match outline::goto_target(text) {
            Some(outline::GotoTarget::Line(line)) => self.go_to_line(line, window, cx),
            Some(outline::GotoTarget::Symbol(name)) => {
                let current = self.cursor_line(cx);
                let target = outline::definition(&self.outline_model, &name, current)
                    .map(|symbol| symbol.line);
                match target {
                    Some(line) => self.go_to_line(line, window, cx),
                    None => self.output().warning(cx, format!("找不到 `{name}`")),
                }
            }
            None => self.output().warning(cx, "请输入行号或名字"),
        }
    }

    /// 打开「转到文件」对话框（`Ctrl+P`）：按文件名（或路径）模糊匹配。
    pub(crate) fn goto_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个。
            return;
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("文件名或路径，例如 main 或 src/main.rs")
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle.update(cx, |this, cx| this.open_matching_file(&text, window, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "goto-file",
                title: "转到文件",
                description: Some("按文件名模糊匹配；输入 / 或 \\ 时按完整路径匹配。"),
                confirm: "打开",
            },
            input,
            apply,
        );
    }

    /// 命令面板（`Ctrl+Shift+P`）：输入命令名，回车执行。
    pub(crate) fn command_palette_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("命令名，例如 保存 / 运行 / 主题")
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                // 先关掉命令面板再执行：有些命令自己会弹对话框（打开文件、转到行），
                // 后关会把新对话框一起关掉。
                window.close_dialog(cx);
                let _ = handle.update(cx, |this, cx| this.run_command_text(&text, window, cx));
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "command-palette",
                title: "命令面板",
                description: Some("按命令名模糊匹配，回车执行。"),
                confirm: "执行",
            },
            input,
            apply,
        );
    }

    /// 按用户输入的名字找命令并执行。
    pub(crate) fn run_command_text(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = commands::best_command(text) else {
            let query = text.trim();
            self.output()
                .warning(cx, format!("找不到命令 `{query}`，按 Ctrl+Shift+P 重新输入"));
            return;
        };
        let shortcut = if command.shortcut.is_empty() {
            String::new()
        } else {
            format!("（{}）", command.shortcut)
        };
        self.output()
            .command(cx, format!("> {}{}", command.label, shortcut));
        self.run_command(command.id, window, cx);
    }

    /// 执行命令面板里的命令（按 `commands::Command::id` 分发）。
    pub(crate) fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "file.new" => self.new_file(window, cx),
            "file.open" => self.open_files_dialog(window, cx),
            "file.save" => self.save_file(cx),
            "file.save-as" => self.save_file_as(window, cx),
            "file.save-all" => self.save_all_files(cx),
            "file.close" => {
                if let Some(index) = self.active_file {
                    self.close_file(index, window, cx);
                }
            }
            "file.close-all" => self.close_all_files(window, cx),
            "file.reopen" => self.reopen_closed_file(window, cx),
            "edit.find" => self.open_search(false, window, cx),
            "edit.replace" => self.open_search(true, window, cx),
            "edit.comment" => self.toggle_active_comment(window, cx),
            "edit.move-up" => self.move_active_lines(true, window, cx),
            "edit.move-down" => self.move_active_lines(false, window, cx),
            "edit.indent" => self.run_editor_action(Box::new(Indent), window, cx),
            "edit.outdent" => self.run_editor_action(Box::new(Outdent), window, cx),
            "edit.select-all" => self.run_editor_action(Box::new(SelectAll), window, cx),
            "goto.file" => self.goto_file_dialog(window, cx),
            "goto.line" => self.goto_dialog(window, cx),
            "goto.definition" => self.go_to_definition(window, cx),
            "goto.references" => self.find_references(cx),
            "view.explorer" => {
                SettingsStore::update(cx, |settings| settings.appearance.toggle_explorer());
            }
            "view.outline" => {
                SettingsStore::update(cx, |settings| settings.appearance.toggle_outline());
            }
            "view.search" => {
                SettingsStore::update(cx, |settings| settings.appearance.toggle_search());
            }
            "view.output" => {
                SettingsStore::update(cx, |settings| {
                    settings.appearance.show_output = !settings.appearance.show_output;
                });
            }
            "view.status-bar" => {
                SettingsStore::update(cx, |settings| {
                    settings.appearance.show_status_bar = !settings.appearance.show_status_bar;
                });
            }
            "view.theme" => {
                let dark = ui_theme::active(cx).mode.is_dark();
                SettingsStore::update(cx, |settings| {
                    settings.appearance.theme = if dark {
                        ThemeChoice::Light
                    } else {
                        ThemeChoice::Dark
                    };
                });
            }
            "view.fullscreen" => window.toggle_fullscreen(),
            "run.start" => self.run_script(cx),
            "run.check" => self.check_syntax(cx),
            "run.stop" => self.stop_script(cx),
            "run.debug" => self.start_debug(cx),
            "run.pause" => self.debug_pause(cx),
            "run.resume" => self.debug_resume(cx),
            "run.step" => self.debug_step(cx),
            "run.to-cursor" => self.run_to_cursor(cx),
            "debug.toggle-breakpoint" => self.toggle_breakpoint(cx),
            "debug.clear-breakpoints" => self.clear_all_breakpoints(cx),
            _ => {}
        }
    }

    /// 执行「转到文件」：在已打开、最近打开与资源管理器根目录下的文件里找最匹配的一个。
    pub(crate) fn open_matching_file(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = text.trim();
        if query.is_empty() {
            self.output().warning(cx, "先输入文件名再转到");
            return;
        }
        let candidates = self.file_candidates();
        if candidates.is_empty() {
            self.output()
                .warning(cx, "没有可打开的文件：先在资源管理器里添加文件夹或打开文件");
            return;
        }
        let Some(path) = workspace::best_match(query, &candidates).cloned() else {
            self.output()
                .warning(cx, format!("找不到匹配 `{query}` 的文件"));
            return;
        };
        let name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        self.open_file(&path, window, cx);
        self.output()
            .command(cx, format!("> 转到文件：{name}（{path}）"));
    }

    /// 搜索候选：给了 `scope` 就只在该文件夹下递归收集，否则与「转到文件」一致。
    pub(crate) fn search_candidates(&self, scope: Option<&str>) -> Vec<String> {
        match scope {
            Some(scope) => {
                let mut files = Vec::new();
                let mut seen = HashSet::new();
                workspace::collect_files(Path::new(scope), &mut files, &mut seen);
                files
            }
            None => self.file_candidates(),
        }
    }

    /// 弹出文件夹选择框，把选中的文件夹设为搜索范围。
    pub(crate) fn choose_search_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择搜索范围".into()),
        });
        let panel = self.search_panel.clone();
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(dir) = paths.into_iter().next()
            {
                let scope = dir.to_string_lossy().into_owned();
                panel.update(cx, |panel, cx| panel.set_scope(Some(scope), cx));
            }
        })
        .detach();
    }

    /// 打开搜索命中的位置：先把文件打开（必要时切标签），再跳到那一行。
    pub(crate) fn open_search_hit(
        &mut self,
        path: &str,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self
            .active_file
            .is_some_and(|index| self.open_files[index].id == path);
        if !active {
            self.open_file(path, window, cx);
        }
        self.go_to_line(line, window, cx);
    }

    /// 「转到文件」的候选：已打开的文件、最近打开的文件与资源管理器各根目录下的文件。
    fn file_candidates(&self) -> Vec<String> {
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for file in &self.open_files {
            let id = file.id.to_string();
            if seen.insert(id.clone()) {
                files.push(id);
            }
        }
        for path in &self.recent_files {
            if Path::new(path).is_file() && seen.insert(path.clone()) {
                files.push(path.clone());
            }
        }
        for root in self.explorer.roots() {
            workspace::collect_files(root.path(), &mut files, &mut seen);
        }
        files
    }

    /// 光标处的标识符（`_` 与字母数字），用于转到定义。
    fn word_at(text: &Rope, offset: usize) -> Option<String> {
        let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
        let mut start = offset;
        for ch in text.chars_at(offset).reversed() {
            if is_word(ch) {
                start -= ch.len_utf8();
            } else {
                break;
            }
        }
        let mut end = offset;
        for ch in text.chars_at(offset) {
            if is_word(ch) {
                end += ch.len_utf8();
            } else {
                break;
            }
        }
        (start < end).then(|| text.slice(start..end).to_string())
    }

    /// 在当前行切换断点（`F9`）。断点按文件分别记录。
    pub(crate) fn toggle_breakpoint(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.active_file else {
            self.output().warning(cx, "没有打开的文件，无法设置断点");
            return;
        };
        let id = self.open_files[index].id.to_string();
        let line = self.cursor_line(cx);
        let set = self.breakpoints.entry(id.clone()).or_default();
        let added = !set.remove(&line) && set.insert(line);
        if !added && let Some(conditions) = self.breakpoint_conditions.get_mut(&id) {
            // 取消断点时条件一并清掉。
            conditions.remove(&line);
        }

        self.push_breakpoints_to_session();
        self.refresh_breakpoint_marks(cx);
        self.sync_outline_panel(cx);
        // 断点会写进布局文件，重启后还在。
        self.save_layout(cx);

        let output = self.output();
        if added {
            output.command(cx, format!("> 断点：第 {line} 行"));
        } else {
            output.command(cx, format!("> 取消断点：第 {line} 行"));
        }
        cx.notify();
    }

    /// 当前文件的断点集合。
    fn active_breakpoints(&self) -> HashSet<usize> {
        self.active_file
            .and_then(|index| self.open_files.get(index))
            .and_then(|file| self.breakpoints.get(file.id.as_ref()))
            .cloned()
            .unwrap_or_default()
    }

    /// 当前文件的条件断点：行号 → 条件表达式。
    fn active_breakpoint_conditions(&self) -> HashMap<usize, String> {
        self.active_file
            .and_then(|index| self.open_files.get(index))
            .and_then(|file| self.breakpoint_conditions.get(file.id.as_ref()))
            .cloned()
            .unwrap_or_default()
    }

    /// 把当前文件的断点与条件交给正在运行的调试控制器。
    fn push_breakpoints_to_session(&self) {
        let Some(session) = &self.debug else {
            return;
        };
        session.control.set_breakpoints(self.active_breakpoints());
        session
            .control
            .set_breakpoint_conditions(self.active_breakpoint_conditions());
    }

    /// 打开「条件断点」对话框：给当前行的断点加条件，留空表示清除条件。
    pub(crate) fn set_breakpoint_condition_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个。
            return;
        }
        let Some(index) = self.active_file else {
            self.output().warning(cx, "没有打开的文件，无法设置条件断点");
            return;
        };
        let line = self.cursor_line(cx);
        let current = self
            .breakpoint_conditions
            .get(self.open_files[index].id.as_ref())
            .and_then(|conditions| conditions.get(&line))
            .cloned()
            .unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("例如 i > 3；留空表示清除条件")
                .default_value(current)
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle
                    .update(cx, |this, cx| this.set_breakpoint_condition(line, &text, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "breakpoint-condition",
                title: "条件断点",
                description: Some("只有表达式为真时才会在这一行停下；留空表示清除条件。"),
                confirm: "确定",
            },
            input,
            apply,
        );
    }

    /// 设置或清除某一行的断点条件；设置时如果这一行还没有断点，会一并加上。
    pub(crate) fn set_breakpoint_condition(
        &mut self,
        line: usize,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.active_file else {
            self.output().warning(cx, "没有打开的文件，无法设置条件断点");
            return;
        };
        let id = self.open_files[index].id.to_string();
        let condition = text.trim();
        if condition.is_empty() {
            let removed = self
                .breakpoint_conditions
                .get_mut(&id)
                .is_some_and(|conditions| conditions.remove(&line).is_some());
            if removed {
                self.push_breakpoints_to_session();
                self.refresh_breakpoint_marks(cx);
                self.sync_outline_panel(cx);
                self.save_layout(cx);
                self.output()
                    .command(cx, format!("> 已清除第 {line} 行的断点条件"));
                cx.notify();
            } else {
                self.output()
                    .warning(cx, format!("第 {line} 行没有断点条件"));
            }
            return;
        }

        self.breakpoints.entry(id.clone()).or_default().insert(line);
        self.breakpoint_conditions
            .entry(id)
            .or_default()
            .insert(line, condition.to_string());
        self.push_breakpoints_to_session();
        self.refresh_breakpoint_marks(cx);
        self.sync_outline_panel(cx);
        self.save_layout(cx);
        self.output().command(
            cx,
            format!("> 条件断点：第 {line} 行，条件 `{condition}`"),
        );
        cx.notify();
    }

    /// 编辑之后装饰会跟着文字挪动，这里把行号同步回断点集合。
    fn sync_breakpoints_from_marks(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.active_file else {
            return;
        };
        let Some(collection) = self
            .breakpoint_marks
            .get(index)
            .and_then(|marks| marks.as_ref())
        else {
            return;
        };
        let file_id = self.open_files[index].id.to_string();
        let lines: HashSet<usize> = {
            let editor = self.editor.read(cx);
            let text = editor.text();
            collection
                .get_ranges(cx)
                .into_iter()
                .map(|range| text.offset_to_position(range.start).line as usize + 1)
                .collect()
        };
        let entry = self.breakpoints.entry(file_id.clone()).or_default();
        if *entry != lines {
            *entry = lines;
            // 装饰范围会跟着编辑移动，条件也要按新的行号重新挂。
            if let Some(conditions) = self.breakpoint_conditions.get(&file_id) {
                let remapped = crate::layout::remap_breakpoint_conditions(conditions, entry);
                self.breakpoint_conditions.insert(file_id, remapped);
            }
            self.push_breakpoints_to_session();
            self.sync_outline_panel(cx);
        }
    }

    /// 光标所在行（1 基）。
    fn cursor_line(&self, cx: &App) -> usize {
        let editor = self.editor.read(cx);
        let text = editor.text();
        let offset = editor.cursor().min(text.len());
        text.offset_to_position(offset).line as usize + 1
    }

    /// 某一行的字节范围（含行尾的换行符）；行号越界时返回 `None`。
    ///
    /// 空范围会被编辑器当成「行边界」画在两行之间，所以行号栏的断点标记
    /// 也必须用整行范围。
    fn line_offsets(state: &EditorState, row: usize) -> Option<Range<usize>> {
        let text = state.text();
        if row >= text.lines_len() {
            return None;
        }
        let start = text.position_to_offset(&Position::new(row as u32, 0));
        let end = if row + 1 < text.lines_len() {
            text.position_to_offset(&Position::new(row as u32 + 1, 0))
        } else {
            text.len()
        };
        (start < end).then_some(start..end)
    }

    /// 把断点画到当前标签的行号栏里。
    fn refresh_breakpoint_marks(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.active_file else {
            return;
        };
        let Some(editor) = self.editors.get(index).cloned() else {
            return;
        };
        let palette = Palette::for_theme(cx);
        let breakpoints = self.active_breakpoints();
        let conditions = self.active_breakpoint_conditions();
        let marks: Vec<GutterDecoration> = {
            let state = editor.read(cx);
            breakpoints
                .iter()
                .filter_map(|line| {
                    let row = line.checked_sub(1)?;
                    let range = Self::line_offsets(state, row)?;
                    // 条件断点用不同颜色，和普通断点一眼能分开。
                    let color = if conditions.contains_key(line) {
                        palette.warning
                    } else {
                        palette.error
                    };
                    Some(GutterDecoration::new(range).with_color(rgb(color).into()))
                })
                .collect()
        };
        if self.breakpoint_marks[index].is_none() {
            let collection = editor.update(cx, |editor, cx| {
                editor.create_gutter_decorations_collection(Vec::new(), cx)
            });
            self.breakpoint_marks[index] = Some(collection);
        }
        if let Some(collection) = &self.breakpoint_marks[index] {
            collection.set(marks, cx);
        }
    }

    /// 高亮光标旁边括号的配对括号；光标不挨着括号时清掉高亮。
    fn refresh_bracket_match(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.active_file else {
            return;
        };
        let Some(editor) = self.editors.get(index).cloned() else {
            return;
        };
        let pair = {
            let state = editor.read(cx);
            let text = state.value().to_string();
            brackets::matching_pair(&text, state.cursor())
        };
        let next = match pair {
            Some((open, close)) => BracketHighlight::Pair(open, close),
            None => BracketHighlight::None,
        };
        // 只有配对变化时才重画：`set` 每次都会 notify，否则会把自己再叫醒。
        if self.bracket_pairs[index] == next {
            return;
        }
        self.bracket_pairs[index] = next;
        if self.bracket_marks[index].is_none() {
            let collection = editor.update(cx, |editor, cx| {
                editor.create_range_decorations_collection(Vec::new(), cx)
            });
            self.bracket_marks[index] = Some(collection);
        }
        if let Some(collection) = &self.bracket_marks[index] {
            let marks: Vec<RangeDecoration> = match pair {
                Some((open, close)) => {
                    let text = editor.read(cx).value().to_string();
                    let open_len = text[open..].chars().next().map_or(1, char::len_utf8);
                    let close_len = text[close..].chars().next().map_or(1, char::len_utf8);
                    [open..open + open_len, close..close + close_len]
                        .into_iter()
                        .map(|range| {
                            RangeDecoration::new(range)
                                .with_style(RangeDecorationStyle::Fill)
                                .with_color(ui_theme::bracket_match_highlight())
                        })
                        .collect()
                }
                None => Vec::new(),
            };
            collection.set(marks, cx);
        }
    }

    /// 高亮（或取消高亮）当前执行行。
    fn show_debug_line(&mut self, line: Option<usize>, cx: &mut Context<Self>) {
        let Some(line) = line else {
            // 调试行高亮按标签保存；结束时把所有标签上的高亮都清掉。
            for collection in self.paused_line_marks.iter().flatten() {
                collection.clear(cx);
            }
            return;
        };
        let Some(index) = self.active_file else {
            return;
        };
        let editor = self.editor.clone();
        let range = {
            let state = editor.read(cx);
            let row = line
                .saturating_sub(1)
                .min(state.text().lines_len().saturating_sub(1));
            Self::line_offsets(state, row)
        };
        let Some(range) = range else {
            return;
        };
        if self.paused_line_marks[index].is_none() {
            let collection = editor.update(cx, |editor, cx| {
                editor.create_range_decorations_collection(Vec::new(), cx)
            });
            self.paused_line_marks[index] = Some(collection);
        }
        if let Some(collection) = &self.paused_line_marks[index] {
            collection.set(
                vec![
                    RangeDecoration::new(range)
                        .with_style(RangeDecorationStyle::Fill)
                        .with_color(ui_theme::debug_line_highlight()),
                ],
                cx,
            );
        }
    }

    /// 运行到光标处（`Ctrl+F10`）：临时在当前行停下，命中后自动撤掉这个断点。
    pub(crate) fn run_to_cursor(&mut self, cx: &mut Context<Self>) {
        if self.active_file.is_none() {
            self.output().warning(cx, "没有打开的文件，无法运行");
            return;
        }
        let line = self.cursor_line(cx);
        let running = self
            .debug
            .as_ref()
            .map(|session| session.control.clone())
            .filter(|control| !control.is_finished());

        match running {
            Some(control) => {
                let mut breakpoints = self.active_breakpoints();
                breakpoints.insert(line);
                control.set_breakpoints(breakpoints);
                self.run_to = Some(line);
                control.resume();
                self.output().command(cx, format!("> 运行到第 {line} 行"));
            }
            None => self.begin_debug_with(false, Some(line), cx),
        }
    }

    /// 清除所有文件的断点（`Ctrl+Shift+F9`）。
    pub(crate) fn clear_all_breakpoints(&mut self, cx: &mut Context<Self>) {
        let total: usize = self.breakpoints.values().map(HashSet::len).sum();
        if total == 0 {
            self.output().warning(cx, "当前没有断点");
            return;
        }
        self.breakpoints.clear();
        self.breakpoint_conditions.clear();
        self.run_to = None;
        self.push_breakpoints_to_session();
        self.refresh_breakpoint_marks(cx);
        self.sync_outline_panel(cx);
        self.save_layout(cx);
        self.output().command(cx, format!("> 已清除 {total} 个断点"));
        cx.notify();
    }

    /// 请求中断正在运行的脚本；调试会话暂停中也会立刻结束。
    pub(crate) fn stop_script(&mut self, cx: &mut Context<Self>) {
        let output = self.output();
        let debugging = self.debug.as_ref().is_some();
        if let Some(session) = &self.debug {
            session.control.stop();
        }
        // 脚本可能正等在一个对话框上，光设标志它看不到。
        crate::interpreter::ffi::close_dialogs();
        match self.script_interrupt.take() {
            Some(interrupt) => {
                interrupt.store(true, Ordering::Relaxed);
                output.warning(cx, "已请求中断脚本");
            }
            None if !debugging => output.warning(cx, "当前没有正在运行的脚本"),
            None => {}
        }
    }

    /// 语法 + 语义检查（菜单「运行 → 检查语法」）：不执行脚本。
    pub(crate) fn check_syntax(&mut self, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        let output = self.output();
        output.update(cx, |log, _| log.set_channel("vb 语法"));

        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            output.warning(cx, "当前文件不是 VB 脚本，已跳过语法检查");
            return;
        }

        let source = self.editor.read(cx).value().to_string();
        output.command(cx, "> 语法检查");

        let editor = self.editor.clone();
        let analysis = crate::lint::analyze(&source);
        if analysis.issues.is_empty() {
            editor.update(cx, crate::lint::clear);
            self.checked_source = Some(source);
            self.error_count = 0;
            let stats = analysis.program.as_ref().map_or_else(String::new, |program| {
                format!(
                    "检查通过：{} 个过程 / {} 个 API 声明 / {} 条主语句",
                    program.procedures.len(),
                    program.declares.len(),
                    program.main.len()
                )
            });
            output.success(cx, stats);
            return;
        }

        // 语法错误和语义问题都写到输出面板，编辑器里定位到第一处。
        for issue in &analysis.issues {
            output.error(cx, format!("第 {} 行：{}", issue.line, issue.message));
        }
        let first = &analysis.issues[0];
        let (line, message) = (first.line, first.message.clone());
        editor.update(cx, |editor, cx| {
            reveal_error_line(editor, &source, line, &message, cx);
        });
        self.checked_source = Some(source);
        self.error_count = analysis.issues.len();
        cx.notify();
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
    fn sync_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// 初始化 Dock：优先恢复上次的完整面板布局，没有可用布局时创建默认布局。
    fn setup_dock(&mut self, dock_state: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.register_dock_panels(cx);

        if !dock_state.is_empty()
            && let Ok(state) = serde_json::from_str::<DockAreaState>(dock_state)
            && self
                .dock_area
                .update(cx, |area, cx| area.load(state, window, cx))
                .is_ok()
        {
            return;
        }

        let editor = panel_handle(self.editor_panel.clone());
        let explorer = panel_handle(self.explorer_panel.clone());
        let outline = panel_handle(self.outline_panel.clone());
        let search = panel_handle(self.search_panel.clone());
        let output = panel_handle(self.output_panel.clone());
        let side_width = match self.settings.read(cx).appearance.side_panel() {
            Some(crate::settings::SidePanel::Outline) => self.outline_width,
            Some(crate::settings::SidePanel::Search) => self.search_width,
            _ => self.explorer_width,
        };
        let side = if self.side_dock == DockPlacement::Right {
            DockPlacement::Right
        } else {
            DockPlacement::Left
        };
        let output_height = self.output_height;
        self.dock_area.update(cx, |area, cx| {
            area.set_center(DockLayout::tabs().panel_view(editor, cx), window, cx);
            area.set_dock(
                side,
                DockLayout::tabs()
                    .panel_view(explorer, cx)
                    .panel_view(outline, cx)
                    .panel_view(search, cx),
                window,
                cx,
            );
            area.set_dock_size(side, px(side_width), window, cx);
            area.set_dock(
                DockPlacement::Bottom,
                DockLayout::tabs().panel_view(output, cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Bottom, px(output_height), window, cx);
        });
    }

    /// 注册面板构造器，供 [`DockArea::load`] 恢复持久化布局时复用现有面板实体。
    fn register_dock_panels(&self, cx: &mut App) {
        let editor = self.editor_panel.clone();
        register_panel(cx, crate::panels::EDITOR_PANEL, move |_, _, _| {
            panel_handle(editor.clone())
        });
        let explorer = self.explorer_panel.clone();
        register_panel(cx, crate::panels::EXPLORER_PANEL, move |_, _, _| {
            panel_handle(explorer.clone())
        });
        let outline = self.outline_panel.clone();
        register_panel(cx, crate::panels::OUTLINE_PANEL, move |_, _, _| {
            panel_handle(outline.clone())
        });
        let search = self.search_panel.clone();
        register_panel(cx, crate::panels::SEARCH_PANEL, move |_, _, _| {
            panel_handle(search.clone())
        });
        let output = self.output_panel.clone();
        register_panel(cx, crate::panels::OUTPUT_PANEL, move |_, _, _| {
            panel_handle(output.clone())
        });
    }

    /// 面板的显示状态由 `Panel::visible` 读取设置；Dock 只在 reconcile 时重新计算可见面板。
    ///
    /// `DockArea` 没有公开的“仅重新计算可见性”方法，这里临时切换 `locked` 触发一次
    /// reconcile，再恢复原值，不会改变布局或锁定状态。若恢复的布局里缺少某个 dock，
    /// 则按当前设置补回来。
    fn apply_dock_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let appearance = self.settings.read(cx).appearance.clone();
        let current = (
            appearance.show_explorer,
            appearance.show_outline,
            appearance.show_search,
            appearance.show_output,
        );
        if self.panel_visibility == Some(current) {
            return;
        }
        self.panel_visibility = Some(current);

        // 记住用户拖动后的实际尺寸；隐藏面板时 dock 会被移除，重新打开时用它恢复。
        self.remember_dock_sizes(cx);

        let explorer = self.explorer_panel.clone();
        let outline = self.outline_panel.clone();
        let search = self.search_panel.clone();
        let output = self.output_panel.clone();
        let side = self.side_dock;
        let explorer_width = self.explorer_width;
        let outline_width = self.outline_width;
        let search_width = self.search_width;
        let output_height = self.output_height;
        let explorer_id = PanelId::from(explorer.entity_id());
        let outline_id = PanelId::from(outline.entity_id());
        let search_id = PanelId::from(search.entity_id());

        self.dock_area.update(cx, |area, cx| {
            let output_id = PanelId::from(output.entity_id());
            let present = |panel: PanelId| {
                [
                    DockPlacement::Center,
                    DockPlacement::Left,
                    DockPlacement::Right,
                    DockPlacement::Bottom,
                ]
                .into_iter()
                .any(|placement| {
                    area.layout(placement)
                        .is_some_and(|tree| tree.panels().any(|id| id == panel))
                })
            };
            let explorer_present = present(explorer_id);
            let outline_present = present(outline_id);
            let search_present = present(search_id);
            let output_present = present(output_id);
            let want_left = appearance.show_explorer
                || appearance.show_outline
                || appearance.show_search;

            // 被 DragPanel 拖动时，隐藏面板可能被 base 从布局树里移除。
            // 按开关补回缺失的那一个面板，而不是用 set_dock 覆盖整个 dock。
            if appearance.show_explorer && !explorer_present {
                area.add_panel_view(
                    panel_handle(explorer),
                    side,
                    Some(px(explorer_width)),
                    window,
                    cx,
                );
            }
            if appearance.show_outline && !outline_present {
                area.add_panel_view(
                    panel_handle(outline),
                    side,
                    Some(px(outline_width)),
                    window,
                    cx,
                );
            }
            if appearance.show_search && !search_present {
                area.add_panel_view(
                    panel_handle(search),
                    side,
                    Some(px(search_width)),
                    window,
                    cx,
                );
            }
            if appearance.show_output && !output_present {
                area.add_panel_view(
                    panel_handle(output),
                    DockPlacement::Bottom,
                    Some(px(output_height)),
                    window,
                    cx,
                );
            }

            let locked = area.is_locked();
            area.set_locked(!locked, window, cx);
            area.set_locked(locked, window, cx);

            let left_visible = area.layout(DockPlacement::Left).is_some_and(|tree| {
                tree.panels()
                    .any(|id| area.panel(id).is_some_and(|panel| panel.visible(cx)))
            });
            if want_left && left_visible {
                if area.has_dock(DockPlacement::Left)
                    && !area.is_dock_open(DockPlacement::Left)
                {
                    area.toggle_dock(DockPlacement::Left, window, cx);
                }
            } else if !want_left && area.has_dock(DockPlacement::Left) {
                // 隐藏时彻底移除左侧 dock，编辑区才能真正占满剩余空间。
                area.remove_dock(DockPlacement::Left, window, cx);
            }

            let right_visible = area.layout(DockPlacement::Right).is_some_and(|tree| {
                tree.panels()
                    .any(|id| area.panel(id).is_some_and(|panel| panel.visible(cx)))
            });
            if right_visible
                && area.has_dock(DockPlacement::Right)
                && !area.is_dock_open(DockPlacement::Right)
            {
                area.toggle_dock(DockPlacement::Right, window, cx);
            }

            let bottom_visible = area.layout(DockPlacement::Bottom).is_some_and(|tree| {
                tree.panels()
                    .any(|id| area.panel(id).is_some_and(|panel| panel.visible(cx)))
            });
            if appearance.show_output && bottom_visible {
                if area.has_dock(DockPlacement::Bottom)
                    && !area.is_dock_open(DockPlacement::Bottom)
                {
                    area.toggle_dock(DockPlacement::Bottom, window, cx);
                }
            } else if !appearance.show_output && area.has_dock(DockPlacement::Bottom) {
                // 底部 dock 关闭后仍会保留 29px 的折叠条，必须直接移除。
                area.remove_dock(DockPlacement::Bottom, window, cx);
            }
        });

        // 互斥的两个侧栏面板应继续停在用户上一次使用的侧栏。面板被隐藏时
        // 可能还留在另一侧，重新显示后要搬回目标侧，而不是固定回左侧。
        let active_side_panel = match appearance.side_panel() {
            Some(crate::settings::SidePanel::Explorer) => Some(explorer_id),
            Some(crate::settings::SidePanel::Outline) => Some(outline_id),
            Some(crate::settings::SidePanel::Search) => Some(search_id),
            None => None,
        };
        if let Some(panel) = active_side_panel {
            let (current, target_width) = {
                let area = self.dock_area.read(cx);
                (
                    panel_placement(area, panel),
                    area.dock_size(side).map(f32::from),
                )
            };
            if matches!(
                current,
                Some(DockPlacement::Left | DockPlacement::Right)
            ) && current != Some(side)
            {
                self.dock_panel(panel, side, window, cx);
                // 切换互斥面板时沿用目标侧栏原来的宽度，避免整条侧栏突然变窄/变宽。
                if let Some(width) = target_width {
                    self.dock_area.update(cx, |area, cx| {
                        area.set_dock_size(side, px(width), window, cx);
                    });
                }
            }
        }

        // 右侧栏里的面板也可能被开关隐藏；统一收掉空 dock。
        self.normalize_dock_docks(window, cx);
        self.refresh_side_dock(cx);
    }

    /// 从当前 dock 读取并记录侧栏宽度、输出高度。
    fn remember_dock_sizes(&mut self, cx: &App) {
        let explorer_id = PanelId::from(self.explorer_panel.entity_id());
        let outline_id = PanelId::from(self.outline_panel.entity_id());
        let search_id = PanelId::from(self.search_panel.entity_id());
        let area = self.dock_area.read(cx);
        for placement in [DockPlacement::Left, DockPlacement::Right] {
            let Some(width) = area.dock_size(placement).map(f32::from) else {
                continue;
            };
            let Some(tree) = area.layout(placement) else {
                continue;
            };
            let panels: Vec<PanelId> = tree.panels().collect();
            if panels.contains(&explorer_id) {
                self.explorer_width = width;
            }
            if panels.contains(&outline_id) {
                self.outline_width = width;
            }
            if panels.contains(&search_id) {
                self.search_width = width;
            }
        }
        if let Some(height) = area.dock_size(DockPlacement::Bottom).map(f32::from) {
            self.output_height = height;
        }
    }

    /// 用当前布局刷新“最近使用的侧栏”，供互斥面板切换时沿用。
    fn refresh_side_dock(&mut self, cx: &App) {
        let explorer_id = PanelId::from(self.explorer_panel.entity_id());
        let outline_id = PanelId::from(self.outline_panel.entity_id());
        let search_id = PanelId::from(self.search_panel.entity_id());
        let left_visible = self.dock_has_visible_panel(DockPlacement::Left, cx);
        let right_visible = self.dock_has_visible_panel(DockPlacement::Right, cx);
        let (left_present, right_present) = {
            let area = self.dock_area.read(cx);
            let present = |placement| {
                area.layout(placement).is_some_and(|tree| {
                    tree.panels().any(|id| {
                        id == explorer_id || id == outline_id || id == search_id
                    })
                })
            };
            (present(DockPlacement::Left), present(DockPlacement::Right))
        };
        self.side_dock = choose_side_dock(
            left_present,
            left_visible,
            right_present,
            right_visible,
            self.side_dock,
        );
    }

    /// 把被拖到错误区域的面板送回它应该在的 dock。
    fn enforce_dock_placements(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor_id = PanelId::from(self.editor_panel.entity_id());
        let explorer_id = PanelId::from(self.explorer_panel.entity_id());
        let outline_id = PanelId::from(self.outline_panel.entity_id());
        let search_id = PanelId::from(self.search_panel.entity_id());
        let output_id = PanelId::from(self.output_panel.entity_id());

        let (editor, explorer, outline, search, output, explorer_side, outline_side, search_side) = {
            let area = self.dock_area.read(cx);
            (
                panel_placement(area, editor_id),
                panel_placement(area, explorer_id),
                panel_placement(area, outline_id),
                panel_placement(area, search_id),
                panel_placement(area, output_id),
                center_side(area, editor_id, explorer_id),
                center_side(area, editor_id, outline_id),
                center_side(area, editor_id, search_id),
            )
        };

        let mut moves: Vec<(PanelId, DockPlacement)> = Vec::new();
        if editor != Some(DockPlacement::Center) {
            moves.push((editor_id, DockPlacement::Center));
        }
        if matches!(
            explorer,
            Some(DockPlacement::Center | DockPlacement::Bottom)
        ) {
            moves.push((explorer_id, explorer_side));
        }
        if matches!(outline, Some(DockPlacement::Center | DockPlacement::Bottom)) {
            moves.push((outline_id, outline_side));
        }
        if matches!(search, Some(DockPlacement::Center | DockPlacement::Bottom)) {
            moves.push((search_id, search_side));
        }
        if output.is_some() && output != Some(DockPlacement::Bottom) {
            moves.push((output_id, DockPlacement::Bottom));
        }

        for (panel, placement) in moves {
            self.dock_panel(panel, placement, window, cx);
        }
    }

    /// 某个 dock 里是否还有可见面板。
    fn dock_has_visible_panel(&self, placement: DockPlacement, cx: &App) -> bool {
        let area = self.dock_area.read(cx);
        area.layout(placement).is_some_and(|tree| {
            tree.panels()
                .any(|id| area.panel(id).is_some_and(|panel| panel.visible(cx)))
        })
    }

    /// 拖动/合并后，被搬空的 dock 仍可能保留宽度或 29px 折叠条。
    /// 没有可见面板时移除左/右/底部 dock，让编辑区填满。
    fn normalize_dock_docks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 编辑器、资源和输出有固定职责：编辑器必须在中间，资源和代码大纲只能
        // 在左右侧栏，输出只能在底部。拖到中间时 base 会生成一条分裂区域，
        // 那样面板会跑到输出上方，且没有 dock 的宽度调节手柄。
        self.enforce_dock_placements(window, cx);

        // 移除 dock 前记住它最后的大小，下次打开面板时沿用。
        self.remember_dock_sizes(cx);
        let left_visible = self.dock_has_visible_panel(DockPlacement::Left, cx);
        let right_visible = self.dock_has_visible_panel(DockPlacement::Right, cx);
        let bottom_visible = self.dock_has_visible_panel(DockPlacement::Bottom, cx);
        let (has_left, has_right, has_bottom) = {
            let area = self.dock_area.read(cx);
            (
                area.has_dock(DockPlacement::Left),
                area.has_dock(DockPlacement::Right),
                area.has_dock(DockPlacement::Bottom),
            )
        };

        self.dock_area.update(cx, |area, cx| {
            // 关闭的左右 dock 仍保留自己的宽度，base 计算另一侧 dock 的最大
            // 宽度时也会把它算进去，导致分割条拖到一半就拖不动。直接移除，
            // 下次打开时再用保存的宽度重建。
            if has_left && !left_visible {
                area.remove_dock(DockPlacement::Left, window, cx);
            }
            if has_right && !right_visible {
                area.remove_dock(DockPlacement::Right, window, cx);
            }
            if has_bottom && !bottom_visible {
                area.remove_dock(DockPlacement::Bottom, window, cx);
            }
        });
    }

    /// 把被拖动的面板移动到指定 dock（主要供活动栏的“拖回左侧”使用）。
    fn dock_panel(
        &mut self,
        panel: PanelId,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remember_dock_sizes(cx);
        let explorer_id = PanelId::from(self.explorer_panel.entity_id());
        let outline_id = PanelId::from(self.outline_panel.entity_id());
        let search_id = PanelId::from(self.search_panel.entity_id());
        let output_id = PanelId::from(self.output_panel.entity_id());
        if matches!(placement, DockPlacement::Left | DockPlacement::Right)
            && (panel == explorer_id || panel == outline_id || panel == search_id)
        {
            self.side_dock = placement;
        }
        let size = if panel == outline_id {
            px(self.outline_width)
        } else if panel == search_id {
            px(self.search_width)
        } else if panel == output_id {
            px(self.output_height)
        } else {
            px(self.explorer_width)
        };

        let target = {
            let area = self.dock_area.read(cx);
            area.layout(placement)
                .and_then(|tree| first_tabs_node(tree.root()))
        };
        if let Some(node) = target {
            self.dock_area.update(cx, |area, cx| {
                area.move_panel(
                    panel,
                    InsertTarget::Tabs {
                        node,
                        ix: None,
                        activate: true,
                    },
                    window,
                    cx,
                );
                if !area.is_dock_open(placement) {
                    area.toggle_dock(placement, window, cx);
                }
                area.set_dock_size(placement, size, window, cx);
            });
            return;
        }

        // 目标 dock 还没有标签组：先从原位置移除，再以现有视图加入目标 dock。
        let view = self.dock_area.read(cx).panel(panel).cloned();
        let Some(view) = view else {
            return;
        };
        let explorer = self.explorer_panel.clone();
        let outline = self.outline_panel.clone();
        let search = self.search_panel.clone();
        let output = self.output_panel.clone();
        self.dock_area.update(cx, |area, cx| {
            if panel == explorer_id {
                area.remove_panel(explorer, window, cx);
            } else if panel == outline_id {
                area.remove_panel(outline, window, cx);
            } else if panel == search_id {
                area.remove_panel(search, window, cx);
            } else if panel == output_id {
                area.remove_panel(output, window, cx);
            } else {
                return;
            }
            area.add_panel_view(view, placement, Some(size), window, cx);
            if !area.is_dock_open(placement) {
                area.toggle_dock(placement, window, cx);
            }
            area.set_dock_size(placement, size, window, cx);
        });
    }

    /// 侧栏标题开始拖动时记录状态，并在 Dock 上方显示自定义停靠区域。
    pub(crate) fn begin_panel_drag(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        self.remember_dock_sizes(cx);
        let output_id = PanelId::from(self.output_panel.entity_id());
        let placement = {
            let area = self.dock_area.read(cx);
            panel_placement(area, panel)
        };
        let target = if panel == output_id {
            DockPlacement::Bottom
        } else if placement == Some(DockPlacement::Right) {
            DockPlacement::Right
        } else {
            DockPlacement::Left
        };
        let extent = if panel == output_id {
            self.output_height
        } else if panel == PanelId::from(self.outline_panel.entity_id()) {
            self.outline_width
        } else if panel == PanelId::from(self.search_panel.entity_id()) {
            self.search_width
        } else {
            self.explorer_width
        };
        self.panel_drag = Some(PanelDragState {
            panel,
            target,
            extent,
        });
        cx.notify();
        self.watch_panel_drag(cx);
    }

    /// 指针在 Dock 区域移动时更新目标侧栏，指示条始终和最终停靠位置一致。
    fn update_panel_drag_target(
        &mut self,
        event: &DragMoveEvent<DragPanel>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.panel_drag else {
            return;
        };
        let output_id = PanelId::from(self.output_panel.entity_id());
        let outline_id = PanelId::from(self.outline_panel.entity_id());
        let search_id = PanelId::from(self.search_panel.entity_id());
        let (target, extent) = if state.panel == output_id {
            let height = self
                .output_height
                .min(f32::from(event.bounds.size.height) * 0.5)
                .max(80.0);
            (DockPlacement::Bottom, height)
        } else {
            let middle = event.bounds.left() + event.bounds.size.width * 0.5;
            let target = if event.event.position.x < middle {
                DockPlacement::Left
            } else {
                DockPlacement::Right
            };
            let width = if state.panel == outline_id {
                self.outline_width
            } else if state.panel == search_id {
                self.search_width
            } else {
                self.explorer_width
            }
            .min(f32::from(event.bounds.size.width))
            .max(80.0);
            (target, width)
        };
        let next = PanelDragState {
            panel: state.panel,
            target,
            extent,
        };
        if next != state {
            self.panel_drag = Some(next);
            cx.notify();
        }
    }

    /// 放下被拖动的面板；目标来自自定义指示条。
    fn finish_panel_drag(
        &mut self,
        drag: &DragPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .panel_drag
            .map(|state| state.target)
            .unwrap_or(DockPlacement::Left);
        self.panel_drag = None;
        self.dock_panel(drag.panel(), target, window, cx);
        cx.notify();
    }

    /// 拖动结束后关闭自定义指示层；Dock 没有公开“拖动取消”事件，用轻量轮询收尾。
    fn watch_panel_drag(&mut self, cx: &mut Context<Self>) {
        if self.panel_drag_watch {
            return;
        }
        self.panel_drag_watch = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        if this.panel_drag.is_none() {
                            this.panel_drag_watch = false;
                            return false;
                        }
                        if cx.has_active_drag() {
                            return true;
                        }
                        this.panel_drag = None;
                        this.panel_drag_watch = false;
                        cx.notify();
                        false
                    })
                    .unwrap_or_default();
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    /// Dock 上方的拖动目标层：指示条贴着最终停靠的侧栏/底栏，而不是编辑区分裂区。
    fn panel_drag_overlay(
        &self,
        state: PanelDragState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = cx.theme().tokens.drop_target;
        let indicator = match state.target {
            DockPlacement::Left => div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(state.extent))
                .bg(color),
            DockPlacement::Right => div()
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .w(px(state.extent))
                .bg(color),
            DockPlacement::Bottom => div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(state.extent))
                .bg(color),
            DockPlacement::Center => div(),
        };

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .on_drag_move(cx.listener(
                |this, event: &DragMoveEvent<DragPanel>, _, cx| {
                    this.update_panel_drag_target(event, cx);
                },
            ))
            .on_drop(cx.listener(|this, drag: &DragPanel, window, cx| {
                cx.stop_propagation();
                this.finish_panel_drag(drag, window, cx);
            }))
            .child(indicator)
            .into_any_element()
    }

    /// 把标签栏数据同步给 Dock 里的编辑器面板。
    fn sync_editor_panel(&mut self, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        let open_files = self.open_files.clone();
        let dirty = (0..self.open_files.len())
            .map(|index| self.file_is_dirty(index))
            .collect();
        let active = self.active_file;
        let font_size = self.settings.read(cx).editor.font_size;
        self.editor_panel.update(cx, |panel, cx| {
            panel.sync(editor, open_files, dirty, active, font_size, cx);
        });
    }

    /// 根目录数量变化后刷新资源管理器面板的空状态。
    fn sync_explorer_panel(&mut self, cx: &mut Context<Self>) {
        let is_empty = self.explorer.roots().is_empty();
        self.explorer_panel
            .update(cx, |panel, cx| panel.set_empty(is_empty, cx));
    }

    /// 把大纲模型与调试快照同步给 Dock 里的大纲面板。
    fn sync_outline_panel(&mut self, cx: &mut Context<Self>) {
        let model = self.outline_model.clone();
        let vb_supported = self.active_file_language().eq_ignore_ascii_case("vb");
        let debug = self.debug.as_ref().map(|session| OutlineDebugState {
            paused: session.paused,
            snapshot: session.snapshot.clone(),
        });
        let watches: Vec<WatchEntry> = self
            .watches
            .iter()
            .enumerate()
            .map(|(index, expression)| WatchEntry {
                expression: expression.clone(),
                state: self.watch_states.get(index).cloned().unwrap_or_default(),
            })
            .collect();
        let breakpoints = {
            let source = self.outline_source.as_deref().unwrap_or_default();
            let conditions = self.active_breakpoint_conditions();
            outline::breakpoint_entries(self.active_breakpoints(), &conditions, source)
        };
        self.outline_panel.update(cx, |panel, cx| {
            panel.sync(model, vb_supported, debug, breakpoints, watches, cx);
        });
    }

    /// 输出面板字号来自设置。
    fn sync_output_panel(&mut self, cx: &mut Context<Self>) {
        let font_size = self.settings.read(cx).output.font_size as f32;
        self.output_panel
            .update(cx, |panel, cx| panel.set_font_size(font_size, cx));
    }

    pub(crate) fn close_output(&mut self, cx: &mut Context<Self>) {
        SettingsStore::update(cx, |settings| settings.appearance.show_output = false);
    }

    /// 按当前文件与语法设置刷新编辑器的语法分析器与补全提供者。
    ///
    /// 补全只对 VB 脚本开放：打开 Rust / JSON / 纯文本文件时不提示 VB 关键字。
    fn refresh_editor_language(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.read(cx).clone();
        let language = self.effective_language(&settings);
        let completion = self
            .active_file_language()
            .eq_ignore_ascii_case(crate::syntax::VB_LANGUAGE);
        if self.applied_language.as_ref() == Some(&language)
            && self.completion_active == completion
        {
            return;
        }

        let editor = self.editor.clone();
        let language_changed = self.applied_language.as_ref() != Some(&language);
        let completion_changed = self.completion_active != completion;
        editor.update(cx, |editor, cx| {
            if language_changed {
                editor.set_highlighter(language.clone(), cx);
            }
            if completion_changed {
                editor.lsp_mut().completion_provider =
                    completion.then(crate::completion::provider);
            }
        });
        self.applied_language = Some(language);
        self.completion_active = completion;
    }

    fn refresh_outline(&mut self, cx: &mut Context<Self>) {
        let source = self.editor.read(cx).value().to_string();
        if self.outline_source.as_deref() == Some(source.as_str()) {
            return;
        }
        self.outline_source = Some(source.clone());
        self.outline_model = if self.active_file_language().eq_ignore_ascii_case("vb") {
            OutlineModel::from_source(&source)
        } else {
            OutlineModel::default()
        };
        self.sync_outline_panel(cx);
        cx.notify();
    }

    /// 当前生效的语言：设置里开启语法高亮时按设置解析，否则为纯文本。
    fn effective_language(&self, settings: &AppSettings) -> SharedString {
        settings.editor.syntax.resolve(&self.active_file_language())
    }

    fn active_file_language(&self) -> SharedString {
        self.active_file
            .and_then(|index| self.open_files.get(index))
            .map(|file| file.language.clone())
            // 没有打开文件时编辑框是新建的 VB 缓冲区。
            .unwrap_or_else(|| SharedString::from("vb"))
    }

    /// 这些缓存都属于「当前标签」：切换或关闭标签后要按新标签重新计算。
    fn reset_editor_caches(&mut self) {
        self.outline_source = None;
        self.checked_source = None;
        self.applied_language = None;
        self.completion_active = false;
    }

    /// 把编辑器里的最新内容写回当前标签。
    fn save_active_content(&mut self, cx: &App) {
        if let Some(active) = self.active_file
            && let Some(content) = self.contents.get_mut(active)
        {
            *content = self.editor.read(cx).value();
        }
    }

    /// 某个编辑器是否是当前标签的编辑器。
    fn is_active_editor(&self, editor: &Entity<EditorState>) -> bool {
        self.active_file
            .and_then(|index| self.editors.get(index))
            .is_some_and(|active| active.entity_id() == editor.entity_id())
    }

    /// 按当前设置创建一个独立的编辑器状态。
    fn create_editor(
        &self,
        content: SharedString,
        language: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorState> {
        let settings = self.settings.read(cx).editor.clone();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .soft_wrap(settings.soft_wrap)
                .line_number(settings.line_numbers)
                .folding(settings.folding)
                .auto_close(settings.auto_close)
                .default_value(content)
        });
        editor.update(cx, |editor, cx| {
            editor.set_highlighter_factory(crate::syntax::highlighter_factory(), cx);
        });
        editor
    }

    /// 为新标签创建编辑器并挂上变化监听，同时补齐与标签一一对应的状态容器。
    fn add_editor(
        &mut self,
        content: SharedString,
        language: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorState> {
        let editor = self.create_editor(content, language, window, cx);
        let subscription = cx.observe_in(&editor, window, |this, editor, _, cx| {
            if !this.is_active_editor(&editor) {
                return;
            }
            this.sync_active_content(cx);
            this.refresh_outline(cx);
            this.refresh_diagnostics(cx);
            this.sync_breakpoints_from_marks(cx);
        });
        self.editors.push(editor.clone());
        self.editor_subscriptions.push(subscription);
        self.breakpoint_marks.push(None);
        self.paused_line_marks.push(None);
        self.bracket_marks.push(None);
        self.bracket_pairs.push(BracketHighlight::Unset);
        editor
    }

    /// 编辑框内容变化时把它同步到当前标签；内容真的变了才重绘（用于未保存圆点）。
    fn sync_active_content(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active_file else {
            return;
        };
        let value = self.editor.read(cx).value();
        let mut changed = false;
        if let Some(content) = self.contents.get_mut(active)
            && *content != value
        {
            *content = value;
            changed = true;
        }
        if changed {
            self.sync_editor_panel(cx);
            cx.notify();
        }
    }

    /// 编辑内容变化后重新做一次 VB 语法检查。
    ///
    /// 只有 VB 脚本才检查；文本没变（只是移动光标、滚动）时直接返回，
    /// 避免重复解析同一个文本。
    fn refresh_diagnostics(&mut self, cx: &mut Context<Self>) {
        let editor = self.editor.clone();

        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            if self.checked_source.take().is_some() {
                editor.update(cx, crate::lint::clear);
            }
            if self.error_count != 0 {
                self.error_count = 0;
                cx.notify();
            }
            return;
        }

        let source = self.editor.read(cx).value().to_string();
        if self.checked_source.as_deref() == Some(source.as_str()) {
            return;
        }
        self.checked_source = Some(source);

        let issues = editor.update(cx, crate::lint::refresh);
        let errors = issues.len();
        if self.error_count != errors {
            self.error_count = errors;
            cx.notify();
        }
    }

    /// 切换标签：保留各标签自己的编辑器状态，只切换当前使用的编辑器实体。
    pub(crate) fn activate_file(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.open_files.len() || self.active_file == Some(index) {
            return;
        }
        let Some(editor) = self.editors.get(index).cloned() else {
            return;
        };

        self.save_active_content(cx);
        self.active_file = Some(index);
        self.editor = editor;
        self.editor.update(cx, |editor, cx| editor.focus(window, cx));

        // 这些缓存属于「当前标签」，切换后要按新标签重新计算。
        self.reset_editor_caches();

        self.refresh_outline(cx);
        self.refresh_editor_language(cx);
        self.refresh_diagnostics(cx);
        self.refresh_breakpoint_marks(cx);
        self.refresh_bracket_match(cx);
        self.sync_editor_panel(cx);
        cx.notify();
    }

    /// 从资源管理器打开文件；已打开则直接聚焦对应标签。
    /// 磁盘上的路径按绝对路径读取内容。
    pub(crate) fn open_file(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let path = Path::new(id);
        // 空文件夹没有子项，树会把它当成普通条目，点上去时按文件夹处理。
        if path.is_dir() {
            self.toggle_folder(id, cx);
            return;
        }

        if let Some(parent) = path.parent()
            && parent.is_dir()
        {
            self.new_file_target = Some(parent.to_path_buf());
        }

        if let Some(index) = self.open_files.iter().position(|open| open.id == id) {
            self.activate_file(index, window, cx);
            return;
        }

        let (name, language, decoded) = match std::fs::read(path) {
            Ok(bytes) => (
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| id.to_string()),
                workspace::language_for_path(path).to_string(),
                encoding::decode(&bytes),
            ),
            Err(error) => {
                let output = self.output();
                output.error(cx, format!("无法打开 {id}：{error}"));
                return;
            }
        };
        let content = decoded.text;
        let stamp = workspace::disk_stamp(path);

        self.save_active_content(cx);
        self.open_files.push(OpenFile {
            id: id.into(),
            name: name.into(),
            language: language.clone().into(),
            encoding: decoded.encoding,
        });
        self.contents.push(content.clone().into());
        self.saved.push(content.clone().into());
        self.disk_stamps.push(stamp);

        let index = self.open_files.len() - 1;
        self.active_file = Some(index);

        // 每个标签持有自己的编辑器实体，切换回来时光标、滚动和撤销历史都还在。
        let editor = self.add_editor(content.into(), language.into(), window, cx);
        self.editor = editor;
        self.editor.update(cx, |editor, cx| editor.focus(window, cx));

        self.reset_editor_caches();

        self.refresh_outline(cx);
        self.refresh_editor_language(cx);
        self.refresh_diagnostics(cx);
        self.refresh_breakpoint_marks(cx);
        self.sync_editor_panel(cx);
        if crate::layout::remember_recent(&mut self.recent_files, id) {
            self.save_layout(cx);
        }
        cx.notify();
    }

    /// 新建 VB 文件。
    ///
    /// 资源管理器里有文件夹时，建在最近点过的文件夹里（点过文件就用它所在的目录，
    /// 都没点过就用第一个根文件夹）；资源管理器为空时先弹出文件夹选择框，
    /// 选好后把该文件夹加入资源管理器再新建。
    pub(crate) fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dir) = self.target_dir() {
            self.create_file_in(dir, window, cx);
            return;
        }

        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择新建文件所在的文件夹".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(dir) = paths.into_iter().next()
            {
                this.update_in(cx, |this, window, cx| {
                    if this.explorer.add_folder(dir.clone()) {
                        this.rebuild_tree(cx);
                        this.save_layout(cx);
                    }
                    this.create_file_in(dir, window, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// 新建文件的目标目录。
    fn target_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = &self.new_file_target
            && dir.is_dir()
            && self
                .explorer
                .roots()
                .iter()
                .any(|root| dir.starts_with(root.path()))
        {
            return Some(dir.clone());
        }
        self.explorer
            .roots()
            .first()
            .map(|root| root.path().to_path_buf())
    }

    /// 在 `dir` 下新建一个空的 `新建文件*.vb`，刷新树并打开它。
    pub(crate) fn create_file_in(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = free_vb_path(&dir);
        if let Err(error) = std::fs::write(&path, "") {
            let output = self.output();
            output.error(cx, format!("无法新建文件：{error}"));
            return;
        }
        self.explorer.expand(&dir);
        self.rebuild_tree(cx);
        self.save_layout(cx);
        self.open_file(&path.to_string_lossy(), window, cx);
    }

    /// 在目标目录下新建一个空文件夹（名字自动避开重名）。
    pub(crate) fn new_folder(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.target_dir() else {
            self.output()
                .warning(cx, "先在资源管理器里添加一个文件夹");
            return;
        };
        self.create_folder_in(dir, cx);
    }

    /// 在 `dir` 下新建一个空文件夹。
    pub(crate) fn create_folder_in(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let mut index = 1;
        let path = loop {
            let name = if index == 1 {
                "新建文件夹".to_string()
            } else {
                format!("新建文件夹 ({index})")
            };
            let candidate = dir.join(name);
            if !candidate.exists() {
                break candidate;
            }
            index += 1;
        };
        match std::fs::create_dir(&path) {
            Ok(()) => {
                self.explorer.expand(&dir);
                self.rebuild_tree(cx);
                self.save_layout(cx);
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.output().success(cx, format!("已新建文件夹 {name}"));
            }
            Err(error) => {
                self.output()
                    .error(cx, format!("无法新建文件夹：{error}"));
            }
        }
    }

    /// 重新读盘刷新资源管理器（外部新增/删除的文件立刻可见）。
    pub(crate) fn refresh_explorer(&mut self, cx: &mut Context<Self>) {
        self.rebuild_tree(cx);
        self.output().info(cx, "资源管理器已刷新");
    }

    /// 把一段文字（比如文件路径）复制到剪贴板。
    pub(crate) fn copy_path_text(&mut self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        self.output().command(cx, format!("> 已复制路径：{text}"));
    }

    /// 打开「重命名」对话框（标签菜单/文件菜单用，作用于当前标签）。
    pub(crate) fn rename_file_dialog(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.open_files.get(index).map(|file| file.id.to_string()) else {
            return;
        };
        self.rename_path_dialog(path, window, cx);
    }

    /// 打开「重命名」对话框：文件或文件夹都支持，改的是磁盘上的名字。
    pub(crate) fn rename_path_dialog(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name.clone())
                .placeholder("新名字")
        });
        let handle = cx.entity().downgrade();
        let field = input.clone();
        let apply: DialogCallback = Rc::new(move |window, cx| {
            let value = field.read(cx).value().to_string();
            let path = path.clone();
            let _ = handle.update(cx, |this, cx| this.rename_path(&path, &value, window, cx));
        });
        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "rename-path",
                title: "重命名",
                description: Some("只改名字；有未保存的修改会先写回原文件。"),
                confirm: "重命名",
            },
            input,
            apply,
        );
    }

    fn rename_path(
        &mut self,
        path: &str,
        new_name: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old_path = PathBuf::from(path);
        let name = new_name.trim().to_string();
        if name.is_empty() {
            self.output().warning(cx, "名字不能为空");
            return;
        }
        if name.contains(['/', '\\']) || Path::new(&name).file_name().is_none() {
            self.output().warning(cx, "名字里不能带路径分隔符");
            return;
        }
        let Some(parent) = old_path.parent() else {
            return;
        };
        let new_path = parent.join(&name);
        if new_path == old_path {
            return;
        }
        if new_path.exists() {
            self.output().error(cx, format!("{name} 已经存在"));
            return;
        }
        // 打开着的被改文件如果有未保存内容，先写回旧路径，避免重命名后丢改动。
        if let Some(index) = self
            .open_files
            .iter()
            .position(|file| file.id.as_ref() == path)
            && self.file_is_dirty(index)
        {
            let _ = self.write_content(index, &old_path);
        }
        match std::fs::rename(&old_path, &new_path) {
            Ok(()) => {
                let new_id = new_path.to_string_lossy().into_owned();
                self.rewrite_path_prefix(path, &new_id);
                self.rebuild_tree(cx);
                self.refresh_editor_language(cx);
                self.sync_editor_panel(cx);
                self.save_layout(cx);
                self.output().success(cx, format!("已重命名为 {name}"));
            }
            Err(error) => {
                self.output().error(cx, format!("重命名失败：{error}"));
            }
        }
    }

    /// 路径改名后同步所有记录：打开的文件、断点、条件断点、最近打开。
    ///
    /// 文件夹改名时按前缀替换，目录下已打开的文件也跟着走。
    fn rewrite_path_prefix(&mut self, old: &str, new: &str) {
        for file in &mut self.open_files {
            if let Some(updated) = reprefix(&file.id, old, new) {
                file.name = Path::new(&updated)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| updated.clone())
                    .into();
                file.language = workspace::language_for_path(Path::new(&updated)).into();
                file.id = updated.into();
            }
        }
        self.breakpoints = std::mem::take(&mut self.breakpoints)
            .into_iter()
            .map(|(path, lines)| (reprefix(&path, old, new).unwrap_or(path), lines))
            .collect();
        self.breakpoint_conditions = std::mem::take(&mut self.breakpoint_conditions)
            .into_iter()
            .map(|(path, conditions)| (reprefix(&path, old, new).unwrap_or(path), conditions))
            .collect();
        for item in &mut self.recent_files {
            if let Some(updated) = reprefix(item, old, new) {
                *item = updated;
            }
        }
    }

    /// 打开「删除」确认框（标签菜单/文件菜单用，作用于当前标签）。
    pub(crate) fn delete_file_dialog(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.open_files.get(index).map(|file| file.id.to_string()) else {
            return;
        };
        self.delete_path_dialog(path, false, window, cx);
    }

    /// 打开「删除」确认框：文件或文件夹（文件夹连同内容一起删）。
    pub(crate) fn delete_path_dialog(
        &mut self,
        path: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let description = if is_dir {
            format!("确定要删除文件夹 {name} 及其中的全部内容吗？删除后无法撤销。")
        } else {
            format!("确定要删除 {name} 吗？删除后无法撤销。")
        };
        let handle = cx.entity().downgrade();
        let on_confirm: DialogCallback = Rc::new(move |window, cx| {
            let path = path.clone();
            let _ = handle.update(cx, |this, cx| this.delete_path(&path, is_dir, window, cx));
        });
        show_confirm_dialog(window, cx, "删除", description, "删除", on_confirm);
    }

    fn delete_path(&mut self, path: &str, is_dir: bool, window: &mut Window, cx: &mut Context<Self>) {
        let result = if is_dir {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        if let Err(error) = result {
            self.output().error(cx, format!("删除失败：{error}"));
            return;
        }
        self.forget_paths(&[path.to_string()], window, cx);
        self.output().success(cx, format!("已删除 {path}"));
    }

    /// 把资源管理器里拖动的条目移动到目标目录。
    pub(crate) fn move_path(&mut self, source: &str, target_dir: &str, cx: &mut Context<Self>) {
        let source_path = Path::new(source);
        let Some(name) = source_path.file_name() else {
            return;
        };
        let target = Path::new(target_dir).join(name);
        if target == source_path {
            return;
        }
        if target.exists() {
            self.output().error(
                cx,
                format!("目标里已经有 {} 了", name.to_string_lossy()),
            );
            return;
        }
        if let Some(index) = self
            .open_files
            .iter()
            .position(|file| file.id.as_ref() == source)
            && self.file_is_dirty(index)
        {
            let _ = self.write_content(index, source_path);
        }
        match std::fs::rename(source_path, &target) {
            Ok(()) => {
                let new_path = target.to_string_lossy().into_owned();
                self.rewrite_path_prefix(source, &new_path);
                self.explorer.expand(Path::new(target_dir));
                self.rebuild_tree(cx);
                self.refresh_editor_language(cx);
                self.sync_editor_panel(cx);
                self.save_layout(cx);
                self.output().success(
                    cx,
                    format!(
                        "已移动 {} 到 {}",
                        name.to_string_lossy(),
                        target_dir
                    ),
                );
            }
            Err(error) => {
                self.output().error(cx, format!("移动失败：{error}"));
            }
        }
    }

    /// 这些路径被删除后：关掉相关标签，清掉断点与最近打开记录。
    fn forget_paths(&mut self, paths: &[String], window: &mut Window, cx: &mut Context<Self>) {
        let affected = |file: &str| {
            paths
                .iter()
                .any(|path| workspace::reprefix_path(file, path, "").is_some())
        };
        let mut indexes: Vec<usize> = self
            .open_files
            .iter()
            .enumerate()
            .filter(|(_, file)| affected(&file.id))
            .map(|(index, _)| index)
            .collect();
        while let Some(index) = indexes.pop() {
            self.close_file_now(index, window, cx);
        }
        self.breakpoints.retain(|file, _| !affected(file));
        self.breakpoint_conditions
            .retain(|file, _| !affected(file));
        self.recent_files.retain(|item| !affected(item));
        self.rebuild_tree(cx);
        self.save_layout(cx);
    }

    /// 关闭除 `keep` 之外的其它标签。
    pub(crate) fn close_other_files(
        &mut self,
        keep: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(keep_id) = self.open_files.get(keep).map(|file| file.id.to_string()) else {
            return;
        };
        let mut index = 0;
        while index < self.open_files.len() {
            if self.open_files[index].id.as_ref() == keep_id {
                index += 1;
                continue;
            }
            let before = self.open_files.len();
            self.close_file(index, window, cx);
            // 有未保存的修改时会弹确认框，此时先停下，等用户决定。
            if self.open_files.len() == before {
                break;
            }
        }
    }

    /// 关闭 `index` 右侧的所有标签。
    pub(crate) fn close_files_to_right(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        while self.open_files.len() > index + 1 {
            let last = self.open_files.len() - 1;
            let before = self.open_files.len();
            self.close_file(last, window, cx);
            if self.open_files.len() == before {
                break;
            }
        }
    }

    /// 把某个标签对应文件的完整路径复制到剪贴板。
    pub(crate) fn copy_file_path(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(path) = self
            .open_files
            .get(index)
            .map(|file| file.id.to_string())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
        self.output().command(cx, format!("> 已复制路径：{path}"));
    }

    /// 打开文件对话框（与资源管理器「+」添加文件夹同一套系统选择框）。
    ///
    /// 选中的文件会逐个打开；它们所在的文件夹同时挂到资源管理器里。
    pub(crate) fn open_files_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("打开文件".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && !paths.is_empty()
            {
                this.update_in(cx, |this, window, cx| {
                    for path in paths {
                        if let Some(parent) = path.parent()
                            && parent.is_dir()
                        {
                            this.explorer.add_folder(parent.to_path_buf());
                            this.explorer.expand(parent);
                        }
                        this.open_file(&path.to_string_lossy(), window, cx);
                    }
                    this.rebuild_tree(cx);
                    this.save_layout(cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// 把所有有改动的文件写回磁盘，返回保存成功的数量。
    fn save_dirty_files(&mut self, cx: &mut Context<Self>) -> usize {
        let mut saved = 0;
        let mut failures = Vec::new();
        for index in 0..self.open_files.len() {
            if !self.file_is_dirty(index) {
                continue;
            }
            let id = self.open_files[index].id.to_string();
            let name = self.open_files[index].name.to_string();
            match self.write_content(index, Path::new(&id)) {
                Ok(()) => saved += 1,
                Err(error) => failures.push(format!("{name}：{error}")),
            }
        }
        if saved > 0 {
            self.sync_editor_panel(cx);
            self.output().success(cx, format!("已自动保存 {saved} 个文件"));
        }
        for failure in failures {
            self.output().error(cx, format!("自动保存失败：{failure}"));
        }
        saved
    }

    /// 每秒检查一次自动保存；设置里没开就只是更新时间戳。
    fn spawn_auto_save_timer(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(1))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.auto_save_tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn auto_save_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 每秒读一次窗口矩形：拖动窗口（不只是缩放）也能被记下来。
        self.remember_window_bounds(window, cx);
        // 窗口被拖动或缩放过：顺手把布局写盘（每秒最多一次，开销可以忽略）。
        if self.window_bounds_dirty {
            self.window_bounds_dirty = false;
            self.save_layout(cx);
        }
        let dirty = (0..self.open_files.len())
            .filter(|index| self.file_is_dirty(*index))
            .count();
        // 每秒顺带看一眼磁盘：外部改过的文件重新加载（或问用户）。
        self.check_external_changes(window, cx);
        let settings = self.settings.read(cx).editor.clone();
        if !settings.auto_save_due(self.last_auto_save.elapsed(), dirty) {
            return;
        }
        self.last_auto_save = Instant::now();
        self.save_dirty_files(cx);
    }

    /// 保存当前文件（`Ctrl+S`），把编辑框内容写回磁盘。
    pub(crate) fn save_file(&mut self, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        let Some(index) = self.active_file else {
            let output = self.output();
            output.warning(cx, "没有打开的文件，无法保存");
            return;
        };
        let id = self.open_files[index].id.to_string();
        let name = self.open_files[index].name.to_string();
        match self.write_content(index, Path::new(&id)) {
            Ok(()) => {
                self.sync_editor_panel(cx);
                let output = self.output();
                output.success(cx, format!("已保存 {name}"));
            }
            Err(error) => {
                let output = self.output();
                output.error(cx, format!("保存 {name} 失败：{error}"));
            }
        }
    }

    /// 把第 `index` 个标签的内容按原编码写到 `path`；成功后同步「已保存」快照。
    fn write_content(&mut self, index: usize, path: &Path) -> std::io::Result<()> {
        let content = self.contents.get(index).cloned().unwrap_or_default();
        let file_encoding = self
            .open_files
            .get(index)
            .map_or(Default::default(), |file| file.encoding);
        std::fs::write(path, encoding::encode(&content, file_encoding))?;
        if let Some(slot) = self.saved.get_mut(index) {
            *slot = content;
        }
        // 自己写盘后的时间戳要记下来，否则会被当成「外部修改」。
        if let Some(slot) = self.disk_stamps.get_mut(index) {
            *slot = workspace::disk_stamp(path);
        }
        Ok(())
    }

    /// 另存为（`Ctrl+Shift+S`）：选一个路径，写入后把标签切到新文件。
    pub(crate) fn save_file_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        let Some(index) = self.active_file else {
            let output = self.output();
            output.warning(cx, "没有打开的文件，无法另存为");
            return;
        };
        let current = PathBuf::from(self.open_files[index].id.to_string());
        let directory = current
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = self.open_files[index].name.to_string();
        let receiver = cx.prompt_for_new_path(&directory, Some(&name));

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = receiver.await {
                this.update_in(cx, |this, _window, cx| this.save_as_to(index, path, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// 把第 `index` 个标签的内容写到 `path`，并把标签改名。
    fn save_as_to(&mut self, index: usize, path: PathBuf, cx: &mut Context<Self>) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        if let Err(error) = self.write_content(index, &path) {
            let output = self.output();
            output.error(cx, format!("保存 {name} 失败：{error}"));
            return;
        }
        if let Some(file) = self.open_files.get_mut(index) {
            file.id = path.to_string_lossy().into_owned().into();
            file.name = name.clone().into();
            file.language = workspace::language_for_path(&path).to_string().into();
        }
        self.applied_language = None;
        self.outline_source = None;
        self.refresh_editor_language(cx);
        self.refresh_outline(cx);
        self.sync_editor_panel(cx);
        if let Some(parent) = path.parent()
            && parent.is_dir()
        {
            self.explorer.add_folder(parent.to_path_buf());
            self.explorer.expand(parent);
            self.rebuild_tree(cx);
            self.save_layout(cx);
        }
        let output = self.output();
        output.success(cx, format!("已另存为 {name}"));
        cx.notify();
    }

    /// 弹出系统文件夹选择框，把选中的文件夹加入资源管理器。
    pub(crate) fn add_folder_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("添加文件夹".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                this.update_in(cx, |this, window, cx| this.add_folders(paths, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// 把选中的文件夹逐个加入资源管理器（已在树里的会跳过）。
    fn add_folders(&mut self, paths: Vec<PathBuf>, _window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = false;
        for path in paths {
            changed |= self.explorer.add_folder(path);
        }
        if changed {
            self.rebuild_tree(cx);
            self.save_layout(cx);
        }
    }

    /// 把根项从树里移除（不删磁盘文件）。
    pub(crate) fn remove_root(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.explorer.remove_root(id) {
            self.rebuild_tree(cx);
            self.save_layout(cx);
        }
    }

    /// 展开或折叠文件夹；被点到的文件夹同时记作新建文件的目标目录。
    pub(crate) fn toggle_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        self.new_file_target = Some(PathBuf::from(id));
        self.explorer.toggle(id);
        self.rebuild_tree(cx);
        self.save_layout(cx);
    }

    /// 用模型重新生成整棵树。
    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let items = self.explorer.items();
        self.tree.update(cx, |tree, cx| tree.set_items(items, cx));
        self.sync_explorer_panel(cx);
        cx.notify();
    }

    /// 保存所有有改动的文件（`Ctrl+Alt+S`、文件菜单与标签右键菜单都用它）。
    pub(crate) fn save_all_files(&mut self, cx: &mut Context<Self>) {
        if self.open_files.is_empty() {
            self.output().warning(cx, "没有打开的文件，无法保存");
            return;
        }
        self.save_active_content(cx);
        if self.dirty_count() == 0 {
            let output = self.output();
            output.info(cx, "没有需要保存的文件");
            return;
        }
        self.save_all_dirty(cx);
    }

    /// 把当前行（或选区覆盖的整行）上移 / 下移一行。
    pub(crate) fn move_active_lines(
        &mut self,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_file.is_none() {
            return;
        }
        let editor = self.editor.clone();
        let (text, selection, scroll) = {
            let state = editor.read(cx);
            (
                state.value().to_string(),
                state.selected_range(),
                state.scroll_offset(),
            )
        };
        let Some(edit) = editing::move_lines(&text, selection, up) else {
            let message = if up {
                "已经在第一行，无法上移"
            } else {
                "已经在最后一行，无法下移"
            };
            self.output().info(cx, message);
            return;
        };
        self.apply_text_edit(&editor, edit, scroll, window, cx);
    }

    /// 切换当前行（或选区覆盖的整行）的行注释（`Ctrl+/`）。
    pub(crate) fn toggle_active_comment(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_file.is_none() {
            return;
        }
        let language = self.active_file_language();
        let Some(token) = editing::comment_token(&language) else {
            self.output().info(cx, "当前文件类型没有行注释");
            return;
        };
        let editor = self.editor.clone();
        let (text, selection, scroll) = {
            let state = editor.read(cx);
            (
                state.value().to_string(),
                state.selected_range(),
                state.scroll_offset(),
            )
        };
        let Some(edit) = editing::toggle_comment(&text, selection, token) else {
            return;
        };
        self.apply_text_edit(&editor, edit, scroll, window, cx);
    }

    /// 用一次纯文本编辑替换整篇内容，并把选区、滚动位置放回原处。
    fn apply_text_edit(
        &mut self,
        editor: &Entity<EditorState>,
        edit: editing::TextEdit,
        scroll: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        editor.update(cx, |state, cx| {
            state.replace_all(edit.text, window, cx);
            state.set_selected_range(edit.selection, cx);
            state.set_scroll_offset(scroll, cx);
            state.focus(window, cx);
        });
        self.sync_active_content(cx);
        cx.notify();
    }

    /// 恢复上次退出时开着的标签，并把焦点放回当时激活的那一个。
    fn restore_session(&mut self, layout: &Layout, window: &mut Window, cx: &mut Context<Self>) {
        if layout.open.is_empty() {
            return;
        }
        let mut restored = 0usize;
        let mut active_index = None;
        for path in layout.open.iter().take(SESSION_LIMIT) {
            // 只恢复还在磁盘上的文件；被删掉的标签不值得弹错误。
            if !Path::new(path).is_file() {
                continue;
            }
            self.open_file(path, window, cx);
            if let Some(index) = self.open_files.iter().position(|file| file.id == path.as_str()) {
                if path == &layout.active {
                    active_index = Some(index);
                }
                restored += 1;
            }
        }
        if let Some(index) = active_index {
            self.activate_file(index, window, cx);
        }
        if restored > 1 {
            self.output()
                .info(cx, format!("已恢复上次打开的 {restored} 个文件"));
        }
    }

    /// 在标签之间循环切换（`Ctrl+Tab` / `Ctrl+Shift+Tab`）。
    pub(crate) fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active_file else {
            return;
        };
        let len = self.open_files.len();
        if len < 2 {
            return;
        }
        let next = if forward {
            (active + 1) % len
        } else {
            (active + len - 1) % len
        };
        self.activate_file(next, window, cx);
    }

    /// 把当前布局写盘，下次启动时恢复。
    fn save_layout(&self, cx: &App) {
        let settings = self.settings.read(cx);
        let dock = self.dock_area.read(cx);
        let left_width = dock
            .dock_size(DockPlacement::Left)
            .map(f32::from)
            .unwrap_or(self.explorer_width);
        let right_width = dock
            .dock_size(DockPlacement::Right)
            .map(f32::from)
            .unwrap_or(self.outline_width);
        let output_height = dock
            .dock_size(DockPlacement::Bottom)
            .map(f32::from)
            .unwrap_or(self.output_height);
        let layout = Layout {
            layout_version: LAYOUT_VERSION,
            explorer_width: left_width,
            outline_width: right_width,
            search_width: self.search_width,
            output_height,
            show_explorer: settings.appearance.show_explorer,
            show_outline: settings.appearance.show_outline,
            show_search: settings.appearance.show_search,
            show_output: settings.appearance.show_output,
            show_status_bar: settings.appearance.show_status_bar,
            theme: settings.appearance.theme.as_str().to_string(),
            roots: self
                .explorer
                .roots()
                .iter()
                .map(|root| root.path().to_string_lossy().into_owned())
                .collect(),
            expanded: self.explorer.expanded_ids(),
            breakpoints: {
                let mut breakpoints: Vec<(String, Vec<usize>)> = self
                    .breakpoints
                    .iter()
                    .map(|(path, lines)| {
                        let mut lines: Vec<usize> = lines.iter().copied().collect();
                        lines.sort_unstable();
                        (path.clone(), lines)
                    })
                    .collect();
                breakpoints.sort_by(|left, right| left.0.cmp(&right.0));
                breakpoints
            },
            breakpoint_conditions: {
                let mut conditions: Vec<(String, Vec<(usize, String)>)> = self
                    .breakpoint_conditions
                    .iter()
                    .map(|(path, items)| {
                        let mut items: Vec<(usize, String)> = items
                            .iter()
                            .map(|(line, condition)| (*line, condition.clone()))
                            .collect();
                        items.sort_by_key(|(line, _)| *line);
                        (path.clone(), items)
                    })
                    .filter(|(_, items)| !items.is_empty())
                    .collect();
                conditions.sort_by(|left, right| left.0.cmp(&right.0));
                conditions
            },
            recent: self.recent_files.clone(),
            open: self
                .open_files
                .iter()
                .map(|file| file.id.to_string())
                .take(SESSION_LIMIT)
                .collect(),
            active: self
                .active_file
                .and_then(|index| self.open_files.get(index))
                .map(|file| file.id.to_string())
                .unwrap_or_default(),
            script_args: self.script_args.clone(),
            watches: self.watches.clone(),
            dock_state: serde_json::to_string(&dock.dump(cx)).unwrap_or_default(),
            side_dock: if self.side_dock == DockPlacement::Right {
                "right".to_string()
            } else {
                "left".to_string()
            },
            editor: settings.editor.clone(),
            output: settings.output.clone(),
            window_bounds: self.window_bounds,
            // 这次打开过设置窗口就用新的尺寸，否则沿用文件里记住的值。
            settings_size: crate::settings::remembered_window_size(cx).or(self.settings_size),
        };
        // 写盘失败不影响本次使用，下次启动会退回默认布局。
        let _ = layout.save();
    }

    /// 文件是否有未保存的修改。
    fn file_is_dirty(&self, index: usize) -> bool {
        match (self.contents.get(index), self.saved.get(index)) {
            (Some(current), Some(saved)) => current != saved,
            _ => false,
        }
    }

    fn any_dirty(&self) -> bool {
        (0..self.contents.len()).any(|index| self.file_is_dirty(index))
    }

    fn dirty_count(&self) -> usize {
        (0..self.contents.len())
            .filter(|&index| self.file_is_dirty(index))
            .count()
    }

    /// 记下窗口当前的位置与大小；变化时标记为待写盘。
    fn remember_window_bounds(&mut self, window: &Window, _cx: &mut Context<Self>) {
        let bounds = window.bounds();
        let next = (
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        );
        if self.window_bounds != Some(next) {
            self.window_bounds = Some(next);
            self.window_bounds_dirty = true;
        }
    }

    /// 保存所有有改动的文件。
    fn save_all_dirty(&mut self, cx: &mut Context<Self>) {
        let mut saved_any = false;
        for index in 0..self.open_files.len() {
            if !self.file_is_dirty(index) {
                continue;
            }
            let id = self.open_files[index].id.to_string();
            match self.write_content(index, Path::new(&id)) {
                Ok(()) => saved_any = true,
                Err(error) => {
                    let output = self.output();
                    output.error(cx, format!("保存 {} 失败：{error}", self.open_files[index].name));
                }
            }
        }
        if saved_any {
            let output = self.output();
            output.success(cx, "已保存全部文件");
        }
        self.sync_editor_panel(cx);
    }

    /// 关闭标签（有未保存修改时先确认）。
    pub(crate) fn close_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.open_files.len() {
            return;
        }
        self.save_active_content(cx);
        if !self.file_is_dirty(index) {
            self.close_file_now(index, window, cx);
            return;
        }

        let name = self.open_files[index].name.to_string();
        let handle = cx.weak_entity();
        let save_handle = handle.clone();
        let on_save: DialogCallback = Rc::new(move |window, cx| {
            let _ = save_handle.update(cx, |this, cx| {
                this.save_file(cx);
                if !this.file_is_dirty(index) {
                    this.close_file_now(index, window, cx);
                }
            });
        });
        let discard_handle = handle;
        let on_discard: DialogCallback = Rc::new(move |window, cx| {
            let _ = discard_handle.update(cx, |this, cx| this.close_file_now(index, window, cx));
        });
        show_unsaved_dialog(
            window,
            cx,
            format!("\"{name}\" 有未保存的修改。"),
            on_save,
            on_discard,
        );
    }

    /// 直接关闭标签（不再检查未保存修改）；关掉当前标签后聚焦相邻文件。
    fn close_file_now(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.open_files.len() {
            return;
        }

        self.save_active_content(cx);
        self.remember_closed(self.open_files[index].id.to_string());
        self.open_files.remove(index);
        self.contents.remove(index);
        self.saved.remove(index);
        self.disk_stamps.remove(index);
        self.editors.remove(index);
        drop(self.editor_subscriptions.remove(index));
        self.breakpoint_marks.remove(index);
        self.paused_line_marks.remove(index);
        self.bracket_marks.remove(index);
        self.bracket_pairs.remove(index);

        if let Some(active) = self.active_file {
            if index < active {
                self.active_file = Some(active - 1);
            } else if index == active {
                if self.open_files.is_empty() {
                    self.active_file = None;
                } else {
                    let next = index.min(self.open_files.len() - 1);
                    self.active_file = Some(next);
                    self.editor = self.editors[next].clone();
                    self.editor.update(cx, |editor, cx| editor.focus(window, cx));
                }
            }
        }

        if self.active_file.is_none() {
            // 没有标签时回到空编辑器，避免继续显示已关闭文件的内容。
            self.editor = self.empty_editor.clone();
            self.editor.update(cx, |editor, cx| {
                editor.set_value("", window, cx);
                editor.focus(window, cx);
            });
        }

        // 这些缓存属于「当前标签」，切换后要按新标签重新计算。
        self.reset_editor_caches();
        if self.active_file.is_some() {
            self.refresh_outline(cx);
            self.refresh_diagnostics(cx);
        }
        self.refresh_editor_language(cx);
        self.refresh_breakpoint_marks(cx);
        self.sync_editor_panel(cx);
        cx.notify();
    }

    /// 关闭所有标签（有未保存修改时先确认）。
    pub(crate) fn close_all_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        if !self.any_dirty() {
            self.close_all_files_now(window, cx);
            return;
        }

        let count = self.dirty_count();
        let handle = cx.weak_entity();
        let save_handle = handle.clone();
        let on_save: DialogCallback = Rc::new(move |window, cx| {
            let _ = save_handle.update(cx, |this, cx| {
                this.save_all_dirty(cx);
                if !this.any_dirty() {
                    this.close_all_files_now(window, cx);
                }
            });
        });
        let discard_handle = handle;
        let on_discard: DialogCallback = Rc::new(move |window, cx| {
            let _ =
                discard_handle.update(cx, |this, cx| this.close_all_files_now(window, cx));
        });
        show_unsaved_dialog(
            window,
            cx,
            format!("有 {count} 个文件有未保存的修改。"),
            on_save,
            on_discard,
        );
    }

    /// 直接关闭所有标签并清空编辑框。
    fn close_all_files_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let closed: Vec<String> = self
            .open_files
            .iter()
            .map(|file| file.id.to_string())
            .collect();
        for path in closed {
            self.remember_closed(path);
        }
        self.open_files.clear();
        self.contents.clear();
        self.saved.clear();
        self.disk_stamps.clear();
        self.editors.clear();
        self.editor_subscriptions.clear();
        self.breakpoint_marks.clear();
        self.paused_line_marks.clear();
        self.bracket_marks.clear();
        self.bracket_pairs.clear();
        self.active_file = None;
        self.editor = self.empty_editor.clone();
        self.editor.update(cx, |editor, cx| {
            editor.set_value("", window, cx);
            editor.focus(window, cx);
        });
        self.reset_editor_caches();
        self.refresh_outline(cx);
        self.refresh_editor_language(cx);
        self.sync_editor_panel(cx);
        cx.notify();
    }

    /// 记住刚关闭的标签，供「重新打开关闭的标签」使用（后进先出，最多 20 个）。
    fn remember_closed(&mut self, path: String) {
        self.closed_files.retain(|closed| closed != &path);
        self.closed_files.push(path);
        let limit = 20;
        if self.closed_files.len() > limit {
            let excess = self.closed_files.len() - limit;
            self.closed_files.drain(..excess);
        }
    }

    /// 重新打开最近关闭的标签（`Ctrl+Shift+T`）。
    pub(crate) fn reopen_closed_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(path) = self.closed_files.pop() {
            if Path::new(&path).is_file() {
                self.open_file(&path, window, cx);
                return;
            }
            // 磁盘上已经没有了：跳过这一条，接着看更早关闭的标签。
        }
        self.output().warning(cx, "没有可重新打开的标签");
    }

    /// 检查已打开文件在磁盘上是否被外部改过。
    ///
    /// 没有未保存修改时直接重新加载；有修改时弹确认框，由用户决定重新加载还是保留。
    fn check_external_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 已经有对话框在前台时先不打扰，下一轮再检查。
        if window.has_active_dialog(cx) {
            return;
        }
        self.save_active_content(cx);
        for index in 0..self.open_files.len() {
            let path = PathBuf::from(self.open_files[index].id.to_string());
            let current = workspace::disk_stamp(&path);
            let known = self.disk_stamps.get(index).copied().flatten();
            if !workspace::stamp_differs(known, current) {
                continue;
            }
            // 先记下新时间戳：同一处改动只处理一次，也避免反复弹框。
            if let Some(slot) = self.disk_stamps.get_mut(index) {
                *slot = current;
            }
            if self.file_is_dirty(index) {
                self.prompt_external_reload(index, window, cx);
                return;
            }
            let name = self.open_files[index].name.to_string();
            self.reload_from_disk(index, window, cx);
            self.output()
                .command(cx, format!("> {name} 在磁盘上被修改，已重新加载"));
        }
    }

    /// 把磁盘上的新内容读回标签（丢掉未保存内容时也走这里）。
    fn reload_from_disk(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.open_files.len() {
            return;
        }
        let path = PathBuf::from(self.open_files[index].id.to_string());
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let decoded = encoding::decode(&bytes);
        let content: SharedString = decoded.text.into();
        if let Some(file) = self.open_files.get_mut(index) {
            file.encoding = decoded.encoding;
        }
        if let Some(slot) = self.disk_stamps.get_mut(index) {
            *slot = workspace::disk_stamp(&path);
        }
        if let Some(slot) = self.contents.get_mut(index) {
            *slot = content.clone();
        }
        if let Some(slot) = self.saved.get_mut(index) {
            *slot = content.clone();
        }
        if let Some(editor) = self.editors.get(index).cloned() {
            editor.update(cx, |editor, cx| editor.set_value(content.to_string(), window, cx));
        }
        if self.active_file == Some(index) {
            self.reset_editor_caches();
            self.refresh_outline(cx);
            self.refresh_diagnostics(cx);
            self.refresh_breakpoint_marks(cx);
            self.refresh_editor_language(cx);
        }
        self.sync_editor_panel(cx);
        cx.notify();
    }

    /// 有未保存修改时问用户：保留我的内容，还是用磁盘上的版本覆盖。
    fn prompt_external_reload(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.open_files[index].name.to_string();
        let handle = cx.entity().downgrade();
        let on_reload: DialogCallback = Rc::new(move |window, cx| {
            let _ = handle.update(cx, |this, cx| this.reload_from_disk(index, window, cx));
        });
        show_confirm_dialog(
            window,
            cx,
            "文件已被外部修改",
            format!("\"{name}\" 在磁盘上被修改过，同时还有未保存的修改。重新加载会丢掉这些修改。"),
            "重新加载",
            on_reload,
        );
    }

    /// 项目内「全部替换」之后：刷新受影响的打开标签，并把结果写进输出。
    ///
    /// 有未保存修改的标签不动（磁盘上的新内容会和用户的编辑冲突，留给外部修改检测去问）。
    pub(crate) fn after_replace(
        &mut self,
        outcome: &search::ReplaceOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for index in 0..self.open_files.len() {
            let id = self.open_files[index].id.to_string();
            let changed = outcome.changed.iter().any(|(path, _)| path == &id);
            if changed && !self.file_is_dirty(index) {
                self.reload_from_disk(index, window, cx);
            }
        }
        let files = outcome.changed.len();
        let total = outcome.replacements;
        if files == 0 {
            self.output().warning(cx, "没有替换任何内容");
        } else {
            self.output()
                .success(cx, format!("已替换 {files} 个文件、共 {total} 处"));
        }
        if outcome.skipped > 0 {
            self.output().warning(
                cx,
                format!("{} 个文件无法读取，已跳过", outcome.skipped),
            );
        }
        for failure in &outcome.failures {
            let message = format!("替换失败：{failure}");
            self.output().error(cx, message);
        }
        cx.notify();
    }

    /// 退出程序（有未保存修改时先确认）。
    pub(crate) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_window_close(window, cx) {
            cx.quit();
        }
    }

    /// 窗口关闭前的确认：有未保存修改时弹窗，返回 `false` 阻止本次关闭。
    fn request_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // 暂停中的调试线程会一直等指令，关窗口时先放它走。
        if let Some(session) = &self.debug {
            session.control.stop();
        }
        self.save_active_content(cx);
        self.save_layout(cx);
        if !self.any_dirty() {
            return true;
        }

        let count = self.dirty_count();
        let handle = cx.weak_entity();
        let on_save: DialogCallback = Rc::new(move |_window, cx| {
            let all_saved = handle
                .update(cx, |this, cx| {
                    this.save_all_dirty(cx);
                    !this.any_dirty()
                })
                .unwrap_or(false);
            if all_saved {
                cx.quit();
            }
        });
        let on_discard: DialogCallback = Rc::new(move |_window, cx| cx.quit());
        show_unsaved_dialog(
            window,
            cx,
            format!("有 {count} 个文件有未保存的修改。关闭前要保存吗？"),
            on_save,
            on_discard,
        );
        false
    }

    /// 把 gpui 动作交给编辑器执行（菜单里的撤销、复制等）。
    pub(crate) fn run_editor_action(
        &mut self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        window.dispatch_action(action, cx);
    }

    /// 打开编辑器自带的查找 / 替换面板（菜单「编辑 → 查找/替换」）。
    pub(crate) fn open_search(
        &mut self,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.focus(window, cx);
            editor.open_search(replace, cx);
        });
    }

    /// 打开或收起项目内搜索面板（`Ctrl+Shift+F` 与活动栏的搜索图标）。
    pub(crate) fn toggle_search_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let show = !self.settings.read(cx).appearance.show_search;
        SettingsStore::update(cx, |settings| {
            settings.appearance.set_search(show);
        });
        if show {
            // 打开搜索面板就把焦点放进关键字输入框，直接可以打字。
            self.search_panel
                .update(cx, |panel, cx| panel.focus_query(window, cx));
        }
    }

    /// 切换查找 / 替换面板（`Ctrl+F` / `Ctrl+H`）：已经是同一种模式就收起。
    pub(crate) fn toggle_search(
        &mut self,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            let (open, replace_mode) = {
                let session = editor.search_session();
                (session.open, session.replace_mode)
            };
            if open && replace_mode == replace {
                editor.close_search(cx);
            } else {
                editor.focus(window, cx);
                editor.open_search(replace, cx);
            }
        });
    }

    /// 菜单栏。
    fn menu_bar(&self, cx: &mut Context<Self>, palette: Palette) -> AnyElement {
        let settings = self.settings.read(cx).clone();
        let state = MenuState {
            show_explorer: settings.appearance.show_explorer,
            show_outline: settings.appearance.show_outline,
            show_search: settings.appearance.show_search,
            show_output: settings.appearance.show_output,
            show_status_bar: settings.appearance.show_status_bar,
            has_files: !self.open_files.is_empty(),
            has_active: self.active_file.is_some(),
            dark_theme: ui_theme::resolve(&settings.appearance.theme, cx)
                .mode
                .is_dark(),
        };
        menu::render(
            cx,
            palette,
            cx.entity().downgrade(),
            state,
            &self.recent_files,
        )
    }

    /// 最左侧活动栏。
    fn activity_bar(&self, cx: &mut Context<Self>, appearance: &AppearanceSettings) -> AnyElement {
        v_flex()
            .id("activity-bar")
            .w(px(ui_theme::ACTIVITY_BAR_WIDTH))
            // 与面板一样向内缩 PANE_GAP/2，上下沿才能和右侧面板对齐。
            .my(px(ui_theme::PANE_GAP / 2.0))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .py(px(6.0))
            .bg(rgb(Palette::for_theme(cx).activity_bar_bg))
            .rounded(px(ui_theme::PANE_RADIUS))
            .border_1()
            .border_color(rgba(Palette::for_theme(cx).border))
            .overflow_hidden()
            // 把面板拖到活动栏上，就把它放回左侧 dock。
            .drag_over::<DragPanel>(|this, _, _, cx| {
                this.bg(rgba(Palette::for_theme(cx).hover))
            })
            .on_drop(cx.listener(|this, drag: &DragPanel, window, cx| {
                this.panel_drag = None;
                this.dock_panel(drag.panel(), DockPlacement::Left, window, cx);
                cx.notify();
            }))
            .child(
                Button::new("toggle-explorer")
                    .ghost()
                    .icon(IconName::PanelLeft)
                    .selected(appearance.show_explorer)
                    .on_click(cx.listener(|_, _, _, cx| {
                        SettingsStore::update(cx, |settings| {
                            settings.appearance.toggle_explorer();
                        })
                    })),
            )
            .child(
                Button::new("toggle-outline")
                    .ghost()
                    .icon(IconName::FileText)
                    .selected(appearance.show_outline)
                    .on_click(cx.listener(|_, _, _, cx| {
                        SettingsStore::update(cx, |settings| {
                            settings.appearance.toggle_outline();
                        })
                    })),
            )
            .child(
                Button::new("toggle-search-panel")
                    .ghost()
                    .icon(IconName::Search)
                    .selected(appearance.show_search)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_search_panel(window, cx)
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .on_click(cx.listener(|_, _, _, cx| crate::settings::open_settings_window(cx))),
            )
            .into_any_element()
    }

    /// 状态栏。
    fn status_bar(
        &self,
        cx: &mut Context<Self>,
        palette: Palette,
        settings: &AppSettings,
    ) -> AnyElement {
        let (line_count, char_count) = self
            .active_file
            .and_then(|index| self.contents.get(index))
            .map(|content| (content.lines().count().max(1), content.chars().count()))
            .unwrap_or((0, 0));
        let file_name = self
            .active_file
            .and_then(|index| self.open_files.get(index))
            .map(|file| file.name.to_string());
        let language = workspace::language_label(&self.effective_language(settings));
        let file_encoding = self
            .active_file
            .and_then(|index| self.open_files.get(index))
            .map(|file| file.encoding.label());
        let output_lines = self.output.read(cx).len();
        let cursor = if self.active_file.is_some() {
            let editor = self.editor.read(cx);
            let text = editor.text();
            let offset = editor.cursor().min(text.len());
            let position = text.offset_to_position(offset);
            let line = position.line as usize + 1;
            let line_start = text.position_to_offset(&Position::new(position.line, 0));
            let column = text.slice(line_start..offset).chars().count() + 1;
            Some((line, column))
        } else {
            None
        };

        let on_toggle_output = cx.listener(|_, _, _, cx| {
            SettingsStore::update(cx, |settings| {
                settings.appearance.show_output = !settings.appearance.show_output;
            })
        });
        let debug_state = self.debug.as_ref().map(|session| {
            if session.paused {
                match &session.snapshot {
                    Some(snapshot) => {
                        let stack = call_stack_label(snapshot);
                        if stack.is_empty() {
                            format!("调试：已暂停 · 第 {} 行", snapshot.line)
                        } else {
                            format!("调试：已暂停 · 第 {} 行 · {stack}", snapshot.line)
                        }
                    }
                    None => "调试：已暂停".to_string(),
                }
            } else {
                "调试：运行中".to_string()
            }
        });

        status_bar::render(
            palette,
            StatusInfo {
                file_name: file_name.as_deref(),
                language,
                encoding: file_encoding,
                line_count,
                char_count,
                cursor,
                errors: self.error_count,
                output_lines,
                debug_state: debug_state.as_deref(),
            },
            on_toggle_output,
        )
    }
}

/// 对话框里「保存」「不保存」按钮的回调。
type DialogCallback = Rc<dyn Fn(&mut Window, &mut App)>;

/// 对话框打开后把键盘焦点交给输入框（首帧渲染之后再确认一次，避免焦点被对话框容器拿走）。
fn focus_dialog_input(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| input.focus(window, cx));
    let field = input.clone();
    window.on_next_frame(move |window, cx| {
        field.update(cx, |input, cx| input.focus(window, cx));
    });
}

/// 单行输入对话框的文案与元素 id 前缀。
struct InputDialogSpec {
    id_prefix: &'static str,
    title: &'static str,
    description: Option<&'static str>,
    confirm: &'static str,
}

/// 单行输入对话框：回车与「确定」都执行 `apply`，然后关闭对话框。
fn show_input_dialog(
    window: &mut Window,
    cx: &mut App,
    spec: InputDialogSpec,
    input: Entity<InputState>,
    apply: DialogCallback,
) {
    let InputDialogSpec {
        id_prefix,
        title,
        description,
        confirm,
    } = spec;
    let dialog_input = input.clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        let field = dialog_input.clone();
        let on_click = apply.clone();
        let on_enter = apply.clone();
        // 焦点只用更亮的边框表示，不使用组件在边框外再画一圈的样式。
        let palette = Palette::for_theme(cx);
        let focused = field.read(cx).has_selection_focus(window, cx);
        let mut field_input = Input::new(&field).xsmall().focus_ring(false);
        if focused {
            field_input = field_input.border_color(rgb(palette.sidebar_text));
        }
        let mut body = v_flex()
            .gap_2()
            .on_action(move |_: &Enter, window, cx| on_enter(window, cx));
        if let Some(description) = description {
            body = body.child(div().text_xs().child(description));
        }
        dialog
            .title(div().text_sm().child(title))
            .child(body.child(field_input))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new(format!("{id_prefix}-cancel"))
                            .small()
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new(format!("{id_prefix}-apply"))
                            .small()
                            .label(confirm)
                            .on_click(move |_, window, cx| on_click(window, cx)),
                    ),
            )
    });

    focus_dialog_input(&input, window, cx);
}

/// 通用确认对话框：取消 / 确认。
fn show_confirm_dialog(
    window: &mut Window,
    cx: &mut App,
    title: &'static str,
    description: String,
    confirm: &'static str,
    on_confirm: DialogCallback,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let on_confirm = on_confirm.clone();
        let description = description.clone();
        dialog
            .title(div().text_sm().child(title))
            .child(div().text_xs().child(description))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("confirm-cancel")
                            .small()
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("confirm-apply")
                            .small()
                            .label(confirm)
                            .on_click(move |_, window, cx| {
                                on_confirm(window, cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

/// 有未保存修改时的三选一对话框：取消 / 不保存 / 保存并关闭。
fn show_unsaved_dialog(
    window: &mut Window,
    cx: &mut App,
    description: String,
    on_save: DialogCallback,
    on_discard: DialogCallback,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let on_save = on_save.clone();
        let on_discard = on_discard.clone();
        let description = description.clone();
        dialog
            .title(div().text_sm().child("未保存的修改"))
            .child(div().text_xs().child(description))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("unsaved-cancel")
                            .small()
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("unsaved-discard")
                            .small()
                            .label("不保存")
                            .on_click(move |_, window, cx| {
                                on_discard(window, cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("unsaved-save")
                            .small()
                            .label("保存并关闭")
                            .on_click(move |_, window, cx| {
                                on_save(window, cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
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

/// 选择侧栏应该停靠的一侧：优先当前可见的面板，其次保留仍在布局里的
/// 面板所在侧，最后沿用上一次的选择。
pub(crate) fn choose_side_dock(
    left_present: bool,
    left_visible: bool,
    right_present: bool,
    right_visible: bool,
    current: DockPlacement,
) -> DockPlacement {
    if right_visible {
        DockPlacement::Right
    } else if left_visible {
        DockPlacement::Left
    } else if right_present {
        DockPlacement::Right
    } else if left_present {
        DockPlacement::Left
    } else {
        current
    }
}

/// 面板当前位于哪个区域。
fn panel_placement(area: &DockArea, panel: PanelId) -> Option<DockPlacement> {
    [
        DockPlacement::Center,
        DockPlacement::Left,
        DockPlacement::Right,
        DockPlacement::Bottom,
    ]
    .into_iter()
    .find(|placement| {
        area.layout(*placement)
            .is_some_and(|tree| tree.panels().any(|id| id == panel))
    })
}

/// 面板在中间区域里位于编辑器左边还是右边。
///
/// 只在用户把侧栏拖进中间时用到；没有明显的左右关系时回退到左侧。
fn center_side(area: &DockArea, editor: PanelId, panel: PanelId) -> DockPlacement {
    let Some(tree) = area.layout(DockPlacement::Center) else {
        return DockPlacement::Left;
    };
    let PaneRef::Split { children, .. } = tree.root().kind() else {
        return DockPlacement::Left;
    };
    let panel_ix = children
        .iter()
        .position(|child| node_has_panel(child, panel));
    let editor_ix = children
        .iter()
        .position(|child| node_has_panel(child, editor));
    match (panel_ix, editor_ix) {
        (Some(panel_ix), Some(editor_ix)) if panel_ix > editor_ix => DockPlacement::Right,
        _ => DockPlacement::Left,
    }
}

fn node_has_panel(node: &PaneNode, panel: PanelId) -> bool {
    match node.kind() {
        PaneRef::Tabs { panels, .. } => panels.contains(&panel),
        PaneRef::Split { children, .. } => {
            children.iter().any(|child| node_has_panel(child, panel))
        }
    }
}

/// 找到 Dock 布局树里的第一个标签组。
fn first_tabs_node(node: &PaneNode) -> Option<NodeId> {
    match node.kind() {
        PaneRef::Tabs { .. } => Some(node.id()),
        PaneRef::Split { children, .. } => children.iter().find_map(first_tabs_node),
    }
}

impl Render for CodeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_theme(cx);
        let settings = self.settings.read(cx).clone();
        let appearance = settings.appearance.clone();

        div()
            .relative()
            .v_flex()
            .size_full()
            .bg(rgb(palette.window_bg))
            .on_action(cx.listener(|this, _: &SaveFile, _window, cx| this.save_file(cx)))
            .on_action(cx.listener(|this, _: &SaveFileAs, window, cx| {
                this.save_file_as(window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewFile, window, cx| this.new_file(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFiles, window, cx| {
                this.open_files_dialog(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RunScript, _window, cx| this.run_script(cx)))
            .on_action(cx.listener(|this, _: &StopScript, _window, cx| this.stop_script(cx)))
            .on_action(cx.listener(|this, _: &DebugRun, _window, cx| this.start_debug(cx)))
            .on_action(cx.listener(|this, _: &DebugPause, _window, cx| this.debug_pause(cx)))
            .on_action(cx.listener(|this, _: &DebugResume, _window, cx| this.debug_resume(cx)))
            .on_action(cx.listener(|this, _: &DebugStep, _window, cx| this.debug_step(cx)))
            .on_action(cx.listener(|this, _: &GoToDefinition, window, cx| {
                this.go_to_definition(window, cx)
            }))
            .on_action(cx.listener(|this, _: &FindReferences, _, cx| {
                this.find_references(cx)
            }))
            .on_action(cx.listener(|this, _: &GoToLine, window, cx| {
                this.goto_dialog(window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToFile, window, cx| {
                this.goto_file_dialog(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSearch, window, cx| {
                this.toggle_search(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleReplace, window, cx| {
                this.toggle_search(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSearchPanel, window, cx| {
                this.toggle_search_panel(window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveLineUp, window, cx| {
                this.move_active_lines(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveLineDown, window, cx| {
                this.move_active_lines(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleComment, window, cx| {
                this.toggle_active_comment(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SaveAllFiles, _window, cx| {
                this.save_all_files(cx)
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                this.cycle_tab(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| {
                this.cycle_tab(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowCommands, window, cx| {
                this.command_palette_dialog(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ReopenClosedFile, window, cx| {
                this.reopen_closed_file(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RunToCursor, _, cx| {
                this.run_to_cursor(cx)
            }))
            .on_action(cx.listener(|this, _: &ClearBreakpoints, _, cx| {
                this.clear_all_breakpoints(cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.zoom_editor(Some(1.0), cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.zoom_editor(Some(-1.0), cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| {
                this.zoom_editor(None, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleBreakpoint, window, cx| {
                // 断点按编辑框光标所在行切换，先让编辑框拿到焦点。
                let editor = this.editor.clone();
                editor.update(cx, |editor, cx| editor.focus(window, cx));
                this.toggle_breakpoint(cx);
            }))
            .child(title_bar::render(
                palette,
                window.is_maximized(),
                self.menu_bar(cx, palette),
            ))
            .child(
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .p(px(ui_theme::PANE_GAP))
                    .child(self.activity_bar(cx, &appearance))
                    .child(div().w(px(ui_theme::PANE_GAP)).flex_shrink_0())
                    .child({
                        let drag_overlay = self
                            .panel_drag
                            .map(|state| self.panel_drag_overlay(state, cx));
                        v_flex()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(self.dock_area.clone())
                            .when_some(drag_overlay, |this, overlay| this.child(overlay))
                    }),
            )
            .when(appearance.show_status_bar, |this| {
                this.child(self.status_bar(cx, palette, &settings))
            })
    }
}
