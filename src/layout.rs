//! 布局持久化：把面板尺寸、显示开关与资源管理器的根目录写到磁盘，
//! 下次启动时恢复成上次关闭时的样子。
//!
//! 文件放在可执行文件旁边的 `layout.txt`（绿色版），便于直接查看和手动修改；
//! 该目录不可写时退回用户目录 `%APPDATA%\gpui-first\layout.txt`。
//!
//! 文件格式是简单的 `键=值` 文本（值是百分号转义后的字符串），逐行解析，
//! 未知键与损坏的行会被忽略，因此某个版本写坏文件也不会导致启动失败。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::settings::{
    AiApproval, AiSettings, EditorSettings, OutputSettings, PanelKind, SyntaxLanguage,
};
use crate::theme as ui_theme;

/// Dock 布局的版本号。修改默认面板结构或修复旧布局问题时递增，
/// 旧版本保存的 `dock_state` 会被丢弃并重新按默认布局生成。
pub const LAYOUT_VERSION: u32 = 2;

/// 需要在重启后恢复的界面布局。
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub layout_version: u32,
    pub explorer_width: f32,
    pub outline_width: f32,
    pub search_width: f32,
    /// AI 编程面板的宽度。
    pub ai_width: f32,
    pub output_height: f32,
    /// 左侧栏显示的面板；`None` 表示左侧空着。
    pub left_panel: Option<PanelKind>,
    /// 右侧栏显示的面板；`None` 表示右侧空着。
    pub right_panel: Option<PanelKind>,
    pub show_output: bool,
    pub show_status_bar: bool,
    /// `"dark"`、`"light"`，或自定义主题名（见 `themes.txt`）。
    pub theme: String,
    /// 资源管理器根目录的绝对路径。
    pub roots: Vec<String>,
    /// 处于展开状态的文件夹 id。
    pub expanded: Vec<String>,
    /// 断点：文件路径 → 1 基行号（已排序）。只在文件重新打开时才会用到。
    pub breakpoints: Vec<(String, Vec<usize>)>,
    /// 条件断点：文件路径 → `(行号, 条件表达式)`。
    pub breakpoint_conditions: Vec<(String, Vec<(usize, String)>)>,
    /// 最近打开过的文件（最新的在前），文件名菜单里列前几个。
    pub recent: Vec<String>,
    /// 上次退出时还开着的标签（路径，按标签顺序）。
    pub open: Vec<String>,
    /// 上次激活的标签路径（`open` 里的某一项或空）。
    pub active: String,
    /// 「运行参数」：脚本里 `Command()` 读到的字符串。
    pub script_args: String,
    /// 监视表达式（调试暂停时求值）。
    pub watches: Vec<String>,
    /// Dock 面板的完整布局（JSON），用于保留用户拖动/拆分后的面板位置。
    pub dock_state: String,
    /// 编辑器设置（字号、换行、自动保存、语法分析……），重启后继续生效。
    pub editor: EditorSettings,
    /// 输出面板设置（字号、时间戳、保留行数）。
    pub output: OutputSettings,
    /// AI 设置（接口地址、模型）；API Key 不在布局文件里。
    pub ai: AiSettings,
    /// 上次关闭时的窗口位置与大小：`(x, y, 宽, 高)`，逻辑像素。
    pub window_bounds: Option<(f32, f32, f32, f32)>,
    /// 设置窗口上次的大小：`(宽, 高)`，逻辑像素。
    pub settings_size: Option<(f32, f32)>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            layout_version: LAYOUT_VERSION,
            explorer_width: ui_theme::EXPLORER_WIDTH,
            outline_width: ui_theme::OUTLINE_WIDTH,
            search_width: ui_theme::SEARCH_WIDTH,
            ai_width: ui_theme::AI_WIDTH,
            output_height: ui_theme::OUTPUT_HEIGHT,
            left_panel: Some(PanelKind::Explorer),
            right_panel: None,
            show_output: true,
            show_status_bar: true,
            theme: "dark".to_string(),
            roots: Vec::new(),
            expanded: Vec::new(),
            breakpoints: Vec::new(),
            breakpoint_conditions: Vec::new(),
            recent: Vec::new(),
            open: Vec::new(),
            active: String::new(),
            script_args: String::new(),
            watches: Vec::new(),
            dock_state: String::new(),
            editor: EditorSettings::default(),
            output: OutputSettings::default(),
            ai: AiSettings::default(),
            window_bounds: None,
            settings_size: None,
        }
    }
}

/// 最近打开列表最多记多少个文件。
pub const RECENT_LIMIT: usize = 10;

/// 上次退出时最多恢复多少个标签，避免几百个文件把启动拖死。
pub const SESSION_LIMIT: usize = 40;

/// 把 `path` 放到最近打开列表最前面：去重、限长。返回值是否真的变化了。
pub fn remember_recent(recent: &mut Vec<String>, path: &str) -> bool {
    if recent.first().is_some_and(|first| first == path) {
        return false;
    }
    recent.retain(|item| item != path);
    recent.insert(0, path.to_string());
    recent.truncate(RECENT_LIMIT);
    true
}

/// 编辑导致断点行号变化后，按行号顺序把条件搬到新行上。
///
/// 断点数量不变时按顺序一一对应（常见情况：在断点上方插入/删除行）；
/// 数量变化时只保留行号仍然存在的条件，宁可丢条件也不要把条件挂到错误的行。
pub fn remap_breakpoint_conditions(
    conditions: &HashMap<usize, String>,
    new_lines: &HashSet<usize>,
) -> HashMap<usize, String> {
    if conditions.is_empty() {
        return HashMap::new();
    }
    let mut old_lines: Vec<usize> = conditions.keys().copied().collect();
    old_lines.sort_unstable();
    let mut sorted_new: Vec<usize> = new_lines.iter().copied().collect();
    sorted_new.sort_unstable();
    if old_lines.len() == sorted_new.len() {
        return old_lines
            .into_iter()
            .zip(sorted_new)
            .filter_map(|(old, new)| {
                conditions
                    .get(&old)
                    .map(|condition| (new, condition.clone()))
            })
            .collect();
    }
    conditions
        .iter()
        .filter(|(line, _)| new_lines.contains(line))
        .map(|(line, condition)| (*line, condition.clone()))
        .collect()
}

/// 配置文件名：与可执行文件放在同一目录。
pub const FILE_NAME: &str = "layout.txt";

/// 可执行文件所在目录；路径取不到时返回 `None`。
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// 旧版本使用的用户目录：`%APPDATA%\gpui-first`。
/// 只用于迁移旧配置以及 exe 目录不可写时的退路。
fn legacy_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("LOCALAPPDATA"))?;
    Some(PathBuf::from(base).join("gpui-first"))
}

/// 配置文件名的候选位置，按优先级排列：
/// 可执行文件同目录 → 旧版用户目录 → 临时目录。
pub(crate) fn config_candidates(name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(3);
    if let Some(dir) = exe_dir() {
        paths.push(dir.join(name));
    }
    if let Some(dir) = legacy_dir() {
        push_unique(&mut paths, dir.join(name));
    }
    push_unique(&mut paths, std::env::temp_dir().join(name));
    paths
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

/// 布局文件的存放位置：可执行文件同目录的 `layout.txt`。
/// 取不到可执行文件路径时退回用户目录，再不行用临时目录。
pub fn path() -> PathBuf {
    config_candidates(FILE_NAME)
        .into_iter()
        .next()
        .unwrap_or_else(|| std::env::temp_dir().join(FILE_NAME))
}

impl Layout {
    /// 读取布局文件；文件不存在或无法解析时返回默认布局。
    pub fn load() -> Self {
        let file = path();
        let legacy = config_candidates(FILE_NAME)
            .into_iter()
            .find(|candidate| *candidate != file);
        Self::load_with(&file, legacy.as_deref())
    }

    /// 优先读 `file`；不存在时把旧位置的配置搬过来再用。
    fn load_with(file: &Path, legacy: Option<&Path>) -> Self {
        if file.is_file() {
            return Self::load_from(file);
        }
        if let Some(legacy) = legacy.filter(|legacy| *legacy != file)
            && let Ok(text) = fs::read_to_string(legacy)
        {
            // 迁移成功后删掉旧文件，避免用户删掉新配置时又读回旧设置。
            if write_text(file, &text).is_ok() {
                let _ = fs::remove_file(legacy);
            }
            return Self::parse(&text);
        }
        Self::default()
    }

    /// 从指定文件读取布局，便于测试。
    pub fn load_from(file: &Path) -> Self {
        let Ok(text) = fs::read_to_string(file) else {
            return Self::default();
        };
        Self::parse(&text)
    }

    /// 解析布局文本，缺失的键保持默认值。
    pub fn parse(text: &str) -> Self {
        // 手工编辑过的文件可能带 UTF-8 BOM。
        let text = text.trim_start_matches('\u{feff}');
        // 文件里没有版本号说明是旧格式，下面会丢弃它保存的 Dock 布局。
        let mut layout = Self {
            layout_version: 0,
            ..Self::default()
        };
        // 根项可能被全部移除，所以用 `roots` 记录数量，0 也表示“以文件为准”。
        let mut declared_roots: Option<usize> = None;
        let mut roots: Vec<String> = Vec::new();
        let mut saw_root = false;
        // 新的左右槽位键；旧文件只有 show_* 布尔值，解析完再折算。
        let mut left_panel: Option<PanelKind> = None;
        let mut right_panel: Option<PanelKind> = None;
        let mut saw_slot = false;
        let mut legacy_visible: Vec<PanelKind> = Vec::new();
        let mut saw_legacy_panel = false;
        let mut legacy_side_right = false;

        for line in text.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = unescape(value);

            match key.trim() {
                "layout_version" => {
                    layout.layout_version = value.parse().unwrap_or(0);
                }
                "explorer_width" => {
                    if let Ok(width) = value.parse::<f32>()
                        && width.is_finite()
                    {
                        layout.explorer_width = width;
                    }
                }
                "outline_width" => {
                    if let Ok(width) = value.parse::<f32>()
                        && width.is_finite()
                    {
                        layout.outline_width = width;
                    }
                }
                "search_width" => {
                    if let Ok(width) = value.parse::<f32>()
                        && width.is_finite()
                    {
                        layout.search_width = width;
                    }
                }
                "ai_width" => {
                    if let Ok(width) = value.parse::<f32>()
                        && width.is_finite()
                    {
                        layout.ai_width = width;
                    }
                }
                "output_height" => {
                    if let Ok(height) = value.parse::<f32>()
                        && height.is_finite()
                    {
                        layout.output_height = height;
                    }
                }
                "left_panel" => {
                    saw_slot = true;
                    left_panel = PanelKind::from_str(&value);
                }
                "right_panel" => {
                    saw_slot = true;
                    right_panel = PanelKind::from_str(&value);
                }
                "show_explorer" => {
                    saw_legacy_panel = true;
                    if truthy(&value) {
                        legacy_visible.push(PanelKind::Explorer);
                    }
                }
                "show_outline" => {
                    saw_legacy_panel = true;
                    if truthy(&value) {
                        legacy_visible.push(PanelKind::Outline);
                    }
                }
                "show_search" => {
                    saw_legacy_panel = true;
                    if truthy(&value) {
                        legacy_visible.push(PanelKind::Search);
                    }
                }
                "show_ai" => {
                    saw_legacy_panel = true;
                    if truthy(&value) {
                        legacy_visible.push(PanelKind::Ai);
                    }
                }
                "side_dock" => legacy_side_right = value == "right",
                "show_output" => layout.show_output = truthy(&value),
                "show_status_bar" => layout.show_status_bar = truthy(&value),
                "theme" => layout.theme = value,
                "ai_base_url" => layout.ai.base_url = value,
                "ai_model" => layout.ai.model = value,
                "ai_approval" => layout.ai.approval = AiApproval::from_str(&value),
                "ai_round_limit" => {
                    layout.ai.round_limit = crate::settings::AiRoundLimit::from_str(&value)
                }
                "roots" => declared_roots = value.parse::<usize>().ok(),
                "root" => {
                    saw_root = true;
                    if let Some(path) = value.strip_prefix("dir:") {
                        roots.push(path.to_string());
                    }
                }
                "expanded" => layout.expanded.push(value),
                "recent" => {
                    if !value.is_empty() && !layout.recent.iter().any(|item| item == &value) {
                        layout.recent.push(value);
                    }
                }
                // 上次开着的标签；恢复时会再检查文件是否还在。
                "open" => {
                    if !value.is_empty() && !layout.open.iter().any(|item| item == &value) {
                        layout.open.push(value);
                    }
                }
                "active" => layout.active = value,
                "script_args" => layout.script_args = value,
                "watch" => {
                    if !value.trim().is_empty()
                        && !layout
                            .watches
                            .iter()
                            .any(|item| item.eq_ignore_ascii_case(&value))
                    {
                        layout.watches.push(value);
                    }
                }
                "dock_state" => layout.dock_state = value,
                "window_bounds" => {
                    let parts: Vec<f32> = value
                        .split(',')
                        .filter_map(|part| part.trim().parse::<f32>().ok())
                        .collect();
                    // 只接受看起来合理的窗口矩形，坏数据直接忽略（用默认位置）。
                    if parts.len() == 4
                        && parts.iter().all(|value| value.is_finite())
                        && parts[2] >= 200.0
                        && parts[3] >= 150.0
                        && parts[0].abs() < 20_000.0
                        && parts[1].abs() < 20_000.0
                    {
                        layout.window_bounds = Some((parts[0], parts[1], parts[2], parts[3]));
                    }
                }
                "settings_size" => {
                    let parts: Vec<f32> = value
                        .split(',')
                        .filter_map(|part| part.trim().parse::<f32>().ok())
                        .collect();
                    // 和设置窗口的最小尺寸保持一致，坏数据直接忽略。
                    if parts.len() == 2
                        && parts.iter().all(|value| value.is_finite())
                        && parts[0] >= 400.0
                        && parts[1] >= 300.0
                        && parts[0] < 20_000.0
                        && parts[1] < 20_000.0
                    {
                        layout.settings_size = Some((parts[0], parts[1]));
                    }
                }
                // 编辑器 / 输出设置：缺失的键沿用默认值，方便旧布局文件升级。
                "editor_font_size" => {
                    if let Ok(size) = value.parse::<f32>()
                        && size.is_finite()
                    {
                        layout.editor.font_size = size.clamp(8.0, 48.0);
                    }
                }
                "editor_soft_wrap" => layout.editor.soft_wrap = truthy(&value),
                "editor_line_numbers" => layout.editor.line_numbers = truthy(&value),
                "editor_folding" => layout.editor.folding = truthy(&value),
                "editor_auto_close" => layout.editor.auto_close = truthy(&value),
                "editor_auto_save" => layout.editor.auto_save = truthy(&value),
                "editor_auto_save_interval" => {
                    if let Ok(secs) = value.parse::<u64>() {
                        layout.editor.auto_save_interval_secs = secs.clamp(5, 600);
                    }
                }
                "syntax_enabled" => layout.editor.syntax.enabled = truthy(&value),
                "syntax_language" => {
                    layout.editor.syntax.language = SyntaxLanguage::from_str(&value);
                }
                "output_font_size" => {
                    if let Ok(size) = value.parse::<u32>() {
                        layout.output.font_size = size.clamp(8, 32);
                    }
                }
                "output_show_timestamps" => layout.output.show_timestamps = truthy(&value),
                "output_max_lines" => {
                    if let Ok(lines) = value.parse::<usize>() {
                        layout.output.max_lines = lines.clamp(100, 100_000);
                    }
                }
                "breakpoint" => {
                    // 值形如 `路径%0A1,5,9`（换行被转义过）。
                    if let Some((path, lines)) = value.split_once('\n') {
                        let lines: Vec<usize> = lines
                            .split(',')
                            .filter_map(|line| line.trim().parse().ok())
                            .collect();
                        if !path.is_empty() && !lines.is_empty() {
                            layout.breakpoints.push((path.to_string(), lines));
                        }
                    }
                }
                "breakpoint_condition" => {
                    // 值形如 `路径%0A行号%0A条件表达式`。
                    let mut parts = value.splitn(3, '\n');
                    let (Some(path), Some(line), Some(expression)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    let Ok(line) = line.trim().parse::<usize>() else {
                        continue;
                    };
                    if path.is_empty() || expression.trim().is_empty() {
                        continue;
                    }
                    match layout
                        .breakpoint_conditions
                        .iter_mut()
                        .find(|(file, _)| file == path)
                    {
                        Some((_, items)) => items.push((line, expression.to_string())),
                        None => layout
                            .breakpoint_conditions
                            .push((path.to_string(), vec![(line, expression.to_string())])),
                    }
                }
                _ => {}
            }
        }

        if declared_roots == Some(0) {
            layout.roots.clear();
        } else if saw_root {
            layout.roots = roots;
        }
        if saw_slot {
            layout.left_panel = left_panel;
            layout.right_panel = right_panel;
        } else if saw_legacy_panel {
            // 旧版三个侧栏面板互斥，只保留优先级最高的那个；AI 固定在右侧。
            let side = [PanelKind::Explorer, PanelKind::Outline, PanelKind::Search]
                .into_iter()
                .find(|kind| legacy_visible.contains(kind));
            let ai = legacy_visible.contains(&PanelKind::Ai);
            layout.left_panel = None;
            layout.right_panel = None;
            match (side, ai) {
                (Some(side), true) => {
                    // 一侧只留一个：AI 保持右侧，侧栏面板让到左侧。
                    layout.left_panel = Some(side);
                    layout.right_panel = Some(PanelKind::Ai);
                }
                (Some(side), false) => {
                    if legacy_side_right {
                        layout.right_panel = Some(side);
                    } else {
                        layout.left_panel = Some(side);
                    }
                }
                (None, true) => layout.right_panel = Some(PanelKind::Ai),
                (None, false) => {}
            }
        }
        if layout.layout_version < LAYOUT_VERSION {
            layout.layout_version = LAYOUT_VERSION;
            layout.dock_state.clear();
        }
        layout
    }

    /// 把布局写回磁盘，目录不存在时自动创建。
    /// 可执行文件所在目录不可写（如装在 Program Files）时退回用户目录。
    pub fn save(&self) -> io::Result<()> {
        let text = self.to_text();
        let mut last: Option<io::Error> = None;
        for file in config_candidates(FILE_NAME) {
            match write_text(&file, &text) {
                Ok(()) => return Ok(()),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("没有可写的配置文件位置")))
    }

    /// 写入指定文件，便于测试。
    #[cfg(test)]
    pub fn save_to(&self, file: &Path) -> io::Result<()> {
        write_text(file, &self.to_text())
    }

    /// 序列化成布局文件内容。
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# GPUI Kit 代码编辑器布局文件，删除后恢复默认布局\n");
        out.push_str(&format!("layout_version={}\n", self.layout_version));
        out.push_str(&format!("explorer_width={}\n", self.explorer_width));
        out.push_str(&format!("outline_width={}\n", self.outline_width));
        out.push_str(&format!("search_width={}\n", self.search_width));
        out.push_str(&format!("ai_width={}\n", self.ai_width));
        out.push_str(&format!("output_height={}\n", self.output_height));
        out.push_str(&format!(
            "left_panel={}\n",
            self.left_panel.map(PanelKind::as_str).unwrap_or_default()
        ));
        out.push_str(&format!(
            "right_panel={}\n",
            self.right_panel.map(PanelKind::as_str).unwrap_or_default()
        ));
        // 同时写出面板开关，方便阅读，也让旧版本仍能识别布局意图。
        let show = |kind: PanelKind| {
            self.left_panel == Some(kind) || self.right_panel == Some(kind)
        };
        out.push_str(&format!(
            "show_explorer={}\n",
            flag(show(PanelKind::Explorer))
        ));
        out.push_str(&format!(
            "show_outline={}\n",
            flag(show(PanelKind::Outline))
        ));
        out.push_str(&format!(
            "show_search={}\n",
            flag(show(PanelKind::Search))
        ));
        out.push_str(&format!("show_ai={}\n", flag(show(PanelKind::Ai))));
        out.push_str(&format!("show_output={}\n", flag(self.show_output)));
        out.push_str(&format!("show_status_bar={}\n", flag(self.show_status_bar)));
        out.push_str(&format!("theme={}\n", escape(&self.theme)));
        out.push_str(&format!("ai_base_url={}\n", escape(&self.ai.base_url)));
        out.push_str(&format!("ai_model={}\n", escape(&self.ai.model)));
        out.push_str(&format!("ai_approval={}\n", self.ai.approval.as_str()));
        out.push_str(&format!(
            "ai_round_limit={}\n",
            self.ai.round_limit.as_str()
        ));
        out.push_str(&format!("roots={}\n", self.roots.len()));
        for root in &self.roots {
            out.push_str(&format!("root=dir:{}\n", escape(root)));
        }
        for id in &self.expanded {
            out.push_str(&format!("expanded={}\n", escape(id)));
        }
        for (path, lines) in &self.breakpoints {
            let lines = lines
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(&format!("breakpoint={}\n", escape(&format!("{path}\n{lines}"))));
        }
        for (path, conditions) in &self.breakpoint_conditions {
            for (line, expression) in conditions {
                out.push_str(&format!(
                    "breakpoint_condition={}\n",
                    escape(&format!("{path}\n{line}\n{expression}"))
                ));
            }
        }
        for path in self.recent.iter().take(RECENT_LIMIT) {
            out.push_str(&format!("recent={}\n", escape(path)));
        }
        for path in self.open.iter().take(SESSION_LIMIT) {
            out.push_str(&format!("open={}\n", escape(path)));
        }
        if !self.active.is_empty() {
            out.push_str(&format!("active={}\n", escape(&self.active)));
        }
        if !self.script_args.is_empty() {
            out.push_str(&format!("script_args={}\n", escape(&self.script_args)));
        }
        for watch in &self.watches {
            out.push_str(&format!("watch={}\n", escape(watch)));
        }
        out.push_str(&format!("dock_state={}\n", escape(&self.dock_state)));
        out.push_str(&format!("editor_font_size={}\n", self.editor.font_size));
        out.push_str(&format!("editor_soft_wrap={}\n", flag(self.editor.soft_wrap)));
        out.push_str(&format!(
            "editor_line_numbers={}\n",
            flag(self.editor.line_numbers)
        ));
        out.push_str(&format!("editor_folding={}\n", flag(self.editor.folding)));
        out.push_str(&format!(
            "editor_auto_close={}\n",
            flag(self.editor.auto_close)
        ));
        out.push_str(&format!("editor_auto_save={}\n", flag(self.editor.auto_save)));
        out.push_str(&format!(
            "editor_auto_save_interval={}\n",
            self.editor.auto_save_interval_secs
        ));
        out.push_str(&format!(
            "syntax_enabled={}\n",
            flag(self.editor.syntax.enabled)
        ));
        out.push_str(&format!(
            "syntax_language={}\n",
            self.editor.syntax.language.as_str()
        ));
        out.push_str(&format!("output_font_size={}\n", self.output.font_size));
        out.push_str(&format!(
            "output_show_timestamps={}\n",
            flag(self.output.show_timestamps)
        ));
        out.push_str(&format!("output_max_lines={}\n", self.output.max_lines));
        if let Some((x, y, width, height)) = self.window_bounds {
            out.push_str(&format!("window_bounds={x},{y},{width},{height}\n"));
        }
        if let Some((width, height)) = self.settings_size {
            out.push_str(&format!("settings_size={width},{height}\n"));
        }
        out
    }
}

/// 布局文件里的布尔值：`1` / `true` 为真，其余为假。
fn truthy(value: &str) -> bool {
    value == "1" || value == "true"
}

fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

/// 写入文本，父目录不存在时自动创建。
pub(crate) fn write_text(file: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, text)
}

/// 转义换行与 `%`，保证一行只存一个值；其余字符原样保留，便于人工查看。
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        let hex: String = chars.by_ref().take(2).collect();
        match u8::from_str_radix(&hex, 16) {
            Ok(byte) => out.push(byte as char),
            Err(_) => {
                out.push('%');
                out.push_str(&hex);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AiRoundLimit, SyntaxSettings};

    #[test]
    fn round_trip_keeps_every_field() {
        let layout = Layout {
            layout_version: LAYOUT_VERSION,
            explorer_width: 320.5,
            outline_width: 350.0,
            search_width: 310.0,
            ai_width: 380.0,
            output_height: 210.0,
            left_panel: Some(PanelKind::Outline),
            right_panel: Some(PanelKind::Ai),
            show_output: true,
            show_status_bar: false,
            theme: "light".to_string(),
            roots: vec!["C:\\Users\\测试 用户\\脚本 = 1".to_string()],
            expanded: vec!["C:\\Users\\测试".to_string()],
            breakpoints: vec![
                ("C:\\Users\\测试\\demo.vb".to_string(), vec![2, 5, 9]),
                ("D:\\b.vb".to_string(), vec![1]),
            ],
            breakpoint_conditions: vec![(
                "C:\\Users\\测试\\demo.vb".to_string(),
                vec![(5, "i > 3".to_string())],
            )],
            recent: vec!["C:\\脚本\\a.vb".to_string(), "D:\\b.vb".to_string()],
            open: vec!["C:\\脚本\\a.vb".to_string(), "D:\\b.vb".to_string()],
            active: "D:\\b.vb".to_string(),
            script_args: "input.txt /v".to_string(),
            watches: vec!["total + 1".to_string(), "p.Name".to_string()],
            dock_state: "{\"center\":{}}".to_string(),
            editor: EditorSettings {
                font_size: 17.5,
                soft_wrap: true,
                line_numbers: false,
                folding: false,
                auto_close: false,
                auto_save: true,
                auto_save_interval_secs: 45,
                syntax: SyntaxSettings {
                    enabled: false,
                    language: SyntaxLanguage::Vb,
                },
            },
            output: OutputSettings {
                font_size: 15,
                show_timestamps: true,
                max_lines: 512,
            },
            ai: AiSettings {
                base_url: "https://example.com/v1".to_string(),
                model: "deepseek-reasoner".to_string(),
                approval: AiApproval::Auto,
                round_limit: AiRoundLimit::Twelve,
            },
            window_bounds: Some((120.0, 90.0, 1280.0, 800.0)),
            settings_size: Some((900.0, 640.0)),
        };

        assert_eq!(Layout::parse(&layout.to_text()), layout);
    }

    #[test]
    fn ai_panel_state_survives_a_round_trip() {
        let layout = Layout {
            ai_width: 412.0,
            right_panel: Some(PanelKind::Ai),
            ..Layout::default()
        };
        let parsed = Layout::parse(&layout.to_text());
        assert_eq!(parsed.ai_width, 412.0);
        assert_eq!(parsed.right_panel, Some(PanelKind::Ai));

        // 旧布局文件没有 AI 面板的键：默认不占用右侧空间。
        let parsed = Layout::parse("show_explorer=1\n");
        assert_eq!(parsed.ai_width, ui_theme::AI_WIDTH);
        assert_eq!(parsed.left_panel, Some(PanelKind::Explorer));
        assert_eq!(parsed.right_panel, None);
    }

    #[test]
    fn ai_settings_survive_a_round_trip_without_api_key() {
        let layout = Layout {
            ai: AiSettings {
                base_url: "https://api.deepseek.com/v1".to_string(),
                model: "deepseek-reasoner".to_string(),
                approval: AiApproval::Auto,
                round_limit: AiRoundLimit::Twelve,
            },
            ..Layout::default()
        };
        let parsed = Layout::parse(&layout.to_text());
        assert_eq!(parsed.ai, layout.ai);
        assert!(layout.to_text().contains("ai_approval=auto"));
        assert!(layout.to_text().contains("ai_round_limit=12"));

        // 旧布局里没有 AI 键时使用默认值。
        let parsed = Layout::parse("explorer_width=300\n");
        assert_eq!(parsed.ai, AiSettings::default());
        // 布局文件里不允许出现 API Key 字段。
        assert!(!layout.to_text().contains("api_key"));
    }

    #[test]
    fn empty_slots_survive_a_round_trip() {
        let layout = Layout {
            left_panel: None,
            right_panel: None,
            ..Layout::default()
        };
        let parsed = Layout::parse(&layout.to_text());
        assert_eq!(parsed.left_panel, None);
        assert_eq!(parsed.right_panel, None);
    }

    #[test]
    fn legacy_show_flags_are_converted_to_left_and_right_slots() {
        // 旧文件把侧栏面板与 AI 都记成“显示”：一侧只留一个，AI 留在右侧，
        // 旧版停靠在右侧的侧栏面板让到左侧。
        let parsed = Layout::parse(
            "show_explorer=1\nshow_outline=1\nshow_ai=1\nside_dock=right\n",
        );
        assert_eq!(parsed.left_panel, Some(PanelKind::Explorer));
        assert_eq!(parsed.right_panel, Some(PanelKind::Ai));

        let parsed = Layout::parse("show_outline=1\nside_dock=right\n");
        assert_eq!(parsed.left_panel, None);
        assert_eq!(parsed.right_panel, Some(PanelKind::Outline));
    }

    #[test]
    fn broken_settings_size_is_ignored() {
        assert_eq!(Layout::parse("settings_size=300,200\n").settings_size, None);
        assert_eq!(Layout::parse("settings_size=900\n").settings_size, None);
        assert_eq!(
            Layout::parse("settings_size=900,640\n").settings_size,
            Some((900.0, 640.0))
        );
    }

    #[test]
    fn missing_settings_keys_fall_back_to_defaults() {
        let layout = Layout::parse("show_explorer=0\n");
        assert_eq!(layout.editor, EditorSettings::default());
        assert_eq!(layout.output, OutputSettings::default());
        assert_eq!(layout.window_bounds, None);
    }

    #[test]
    fn broken_window_bounds_are_ignored() {
        assert_eq!(Layout::parse("window_bounds=1,2\n").window_bounds, None);
        assert_eq!(
            Layout::parse("window_bounds=1,2,3,4\n").window_bounds,
            None,
            "尺寸太小视为坏数据"
        );
        assert_eq!(
            Layout::parse("window_bounds=10,20,800,600\n").window_bounds,
            Some((10.0, 20.0, 800.0, 600.0))
        );
    }

    #[test]
    fn settings_values_are_clamped_on_load() {
        let layout = Layout::parse(
            "editor_font_size=1\neditor_auto_save_interval=0\noutput_max_lines=1\n",
        );
        assert_eq!(layout.editor.font_size, 8.0);
        assert_eq!(layout.editor.auto_save_interval_secs, 5);
        assert_eq!(layout.output.max_lines, 100);
    }

    #[test]
    fn remember_recent_moves_to_front_and_dedupes() {
        let mut recent = vec!["a.vb".to_string(), "b.vb".to_string()];
        assert!(remember_recent(&mut recent, "c.vb"));
        assert_eq!(recent, ["c.vb", "a.vb", "b.vb"]);

        // 已经是最前面：不变，也不再写盘。
        assert!(!remember_recent(&mut recent, "c.vb"));

        // 重复打开旧文件：挪到最前面，不留重复项。
        assert!(remember_recent(&mut recent, "b.vb"));
        assert_eq!(recent, ["b.vb", "c.vb", "a.vb"]);

        for index in 0..RECENT_LIMIT + 5 {
            remember_recent(&mut recent, &format!("f{index}.vb"));
        }
        assert_eq!(recent.len(), RECENT_LIMIT, "列表有上限");
    }

    #[test]
    fn breakpoints_survive_a_round_trip() {
        let mut layout = Layout::default();
        layout
            .breakpoints
            .push(("C:\\脚本\\a = 1.vb".to_string(), vec![3, 7]));
        let parsed = Layout::parse(&layout.to_text());
        assert_eq!(
            parsed.breakpoints,
            vec![("C:\\脚本\\a = 1.vb".to_string(), vec![3, 7])]
        );
    }

    #[test]
    fn watch_expressions_survive_a_round_trip() {
        let layout = Layout {
            watches: vec!["total + 1".to_string(), "p.Name".to_string()],
            ..Layout::default()
        };
        assert_eq!(Layout::parse(&layout.to_text()).watches, layout.watches);

        // 空表达式与重复项（忽略大小写）会被丢掉。
        let parsed = Layout::parse("watch=total + 1\nwatch=\nwatch=TOTAL + 1\n");
        assert_eq!(parsed.watches, ["total + 1"]);
    }

    #[test]
    fn breakpoint_conditions_survive_a_round_trip() {
        let mut layout = Layout::default();
        layout.breakpoints.push((
            "C:\\脚本\\a = 1.vb".to_string(),
            vec![3, 7],
        ));
        layout.breakpoint_conditions.push((
            "C:\\脚本\\a = 1.vb".to_string(),
            vec![(7, "i > 3 And s <> \"\"".to_string())],
        ));
        let parsed = Layout::parse(&layout.to_text());
        assert_eq!(parsed.breakpoints, layout.breakpoints);
        assert_eq!(parsed.breakpoint_conditions, layout.breakpoint_conditions);

        // 缺字段或空条件的行会被忽略。
        let parsed = Layout::parse("breakpoint_condition=a.vb\n3\n\nbreakpoint_condition=a.vb\nx\n1\n");
        assert!(parsed.breakpoint_conditions.is_empty());
    }

    #[test]
    fn conditions_follow_breakpoint_lines_after_edits() {
        let conditions = HashMap::from([
            (5, "i = 1".to_string()),
            (9, "i = 2".to_string()),
        ]);

        // 在上方插入一行：两个断点都往下挪一行，条件跟着走。
        let shifted = HashSet::from([6, 10]);
        assert_eq!(
            remap_breakpoint_conditions(&conditions, &shifted),
            HashMap::from([(6, "i = 1".to_string()), (10, "i = 2".to_string())])
        );

        // 断点数量变了：只保留行号仍存在的条件，不猜测对应关系。
        let fewer = HashSet::from([5]);
        assert_eq!(
            remap_breakpoint_conditions(&conditions, &fewer),
            HashMap::from([(5, "i = 1".to_string())])
        );
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let dir = std::env::temp_dir().join("gpui-first-layout-missing");
        let file = dir.join("nope.txt");
        assert_eq!(Layout::load_from(&file), Layout::default());
    }

    #[test]
    fn bom_and_unknown_keys_are_tolerated() {
        let layout = Layout::parse("\u{feff}explorer_width=300\n# 注释\n未知键=1\nshow_output=0\n");
        assert_eq!(layout.explorer_width, 300.0);
        assert!(!layout.show_output);
        assert!(layout.roots.is_empty());
    }

    #[test]
    fn old_dock_state_is_discarded_on_upgrade() {
        // 旧版本保存过错误的 Dock 结构（例如把资源管理器放进中间区域），
        // 升级后必须丢掉它，启动时才能回到默认布局。
        let layout = Layout::parse(
            "explorer_width=300\ndock_state={\"center\":{\"panel_name\":\"TabPanel\"}}\n",
        );
        assert_eq!(layout.layout_version, LAYOUT_VERSION);
        assert!(layout.dock_state.is_empty());
        assert_eq!(layout.explorer_width, 300.0, "其他布局字段仍然保留");
    }

    #[test]
    fn current_dock_state_survives_a_round_trip() {
        let layout = Layout {
            dock_state: "{\"center\":{}}".to_string(),
            ..Layout::default()
        };
        assert_eq!(Layout::parse(&layout.to_text()), layout);
    }

    #[test]
    fn older_layouts_without_visible_panels_leave_both_sides_empty() {
        let layout = Layout::parse("show_explorer=0\nshow_outline=0\nshow_search=0\n");
        assert_eq!(layout.left_panel, None);
        assert_eq!(layout.right_panel, None);
        assert_eq!(layout.outline_width, ui_theme::OUTLINE_WIDTH);
    }

    #[test]
    fn empty_root_list_is_restored() {
        let layout = Layout {
            roots: Vec::new(),
            ..Layout::default()
        };
        assert!(Layout::parse(&layout.to_text()).roots.is_empty());
    }

    #[test]
    fn legacy_sample_root_is_ignored() {
        // 旧版本会把内置示例工程写进布局文件，现在启动时不再恢复它。
        let layout = Layout::parse("roots=2\nroot=sample\nroot=dir:D:\\demo\n");
        assert_eq!(layout.roots, vec!["D:\\demo".to_string()]);
    }

    #[test]
    fn save_and_load_from_disk() {
        let dir = std::env::temp_dir().join("gpui-first-layout-test");
        let file = dir.join("layout.txt");
        let layout = Layout {
            explorer_width: 260.0,
            roots: vec!["D:\\demo".to_string()],
            ..Layout::default()
        };
        layout.save_to(&file).expect("save");
        assert_eq!(Layout::load_from(&file), layout);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_file_lives_next_to_the_executable() {
        let exe = std::env::current_exe().expect("可执行文件路径");
        let expected = exe.parent().expect("可执行文件所在目录").join(FILE_NAME);
        assert_eq!(path(), expected);
    }

    #[test]
    fn legacy_user_dir_config_is_migrated_next_to_the_executable() {
        let dir = std::env::temp_dir().join(format!(
            "gpui-first-layout-migrate-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let legacy = dir.join("appdata").join(FILE_NAME);
        let file = dir.join("exe").join(FILE_NAME);
        write_text(&legacy, "explorer_width=321\ntheme=light\n").expect("写旧配置");

        let layout = Layout::load_with(&file, Some(&legacy));
        assert_eq!(layout.explorer_width, 321.0);
        assert_eq!(layout.theme, "light");
        assert!(file.exists(), "旧配置应搬到可执行文件同目录");
        assert!(!legacy.exists(), "迁移后应删除旧配置");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_config_and_no_legacy_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!(
            "gpui-first-layout-default-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("exe").join(FILE_NAME);
        let legacy = dir.join("appdata").join(FILE_NAME);
        assert_eq!(Layout::load_with(&file, Some(&legacy)), Layout::default());
        let _ = fs::remove_dir_all(&dir);
    }
}
