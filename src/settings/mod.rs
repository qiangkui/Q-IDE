//! 应用设置：数据模型、默认值与全局存储。
//!
//! 设置保存在 `Entity<AppSettings>` 中并通过 [`SettingsStore`] 注册为全局对象，
//! 因此主窗口与设置窗口共享同一份状态：任一侧修改后，观察者会立即收到通知。
//!
//! 唯一例外是 DeepSeek 的 API Key：它不进 [`AppSettings`]（避免被序列化、
//! `Debug` 打印或跟着设置一起拷贝），只由 [`crate::secret_store`] 存进
//! Windows 凭据管理器。

mod window;

pub use window::open_settings_window;
pub use window::remembered_window_size;

use gpui_kit::*;

/// 颜色主题。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
    /// 自定义主题名，定义在可执行文件同目录的 `themes.txt` 里。
    Custom(String),
}

impl ThemeChoice {
    /// 写进布局文件的键：内置主题用 `dark` / `light`，自定义主题用主题名。
    pub fn as_str(&self) -> &str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Custom(name) => name,
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" | "" => Self::Dark,
            name => Self::Custom(name.to_string()),
        }
    }
}

/// 编辑器使用的语法分析器。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyntaxLanguage {
    /// 按文件扩展名自动选择。
    #[default]
    Auto,
    Rust,
    /// VB 脚本语法。
    Vb,
    Json,
    /// 关闭语法分析，按纯文本显示。
    Text,
}

impl SyntaxLanguage {
    pub const OPTIONS: &'static [(&'static str, &'static str)] = &[
        ("auto", "自动（按文件类型）"),
        ("rust", "Rust"),
        ("vb", "Visual Basic"),
        ("json", "JSON"),
        ("text", "纯文本"),
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Rust => "rust",
            Self::Vb => "vb",
            Self::Json => "json",
            Self::Text => "text",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "rust" => Self::Rust,
            "vb" => Self::Vb,
            "json" => Self::Json,
            "text" => Self::Text,
            _ => Self::Auto,
        }
    }
}

/// 语法分析相关设置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxSettings {
    pub enabled: bool,
    pub language: SyntaxLanguage,
}

impl Default for SyntaxSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            language: SyntaxLanguage::Auto,
        }
    }
}

impl SyntaxSettings {
    /// 计算实际使用的语言名。`auto` 是当前文件自身的语言。
    pub fn resolve(&self, auto: &SharedString) -> SharedString {
        if !self.enabled {
            return "text".into();
        }
        match self.language {
            SyntaxLanguage::Auto => auto.clone(),
            SyntaxLanguage::Rust => "rust".into(),
            SyntaxLanguage::Vb => "vb".into(),
            SyntaxLanguage::Json => "json".into(),
            SyntaxLanguage::Text => "text".into(),
        }
    }
}

/// 左右两侧栏。每侧最多显示一个面板。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideSlot {
    Left,
    Right,
}

impl SideSlot {
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// 可以停靠到左右侧栏的面板。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Explorer,
    Outline,
    Search,
    Ai,
}

impl PanelKind {
    /// 固定顺序：遍历面板、恢复旧布局时都用它。
    pub const ALL: [Self; 4] = [Self::Explorer, Self::Outline, Self::Search, Self::Ai];

    /// 两侧都空着时默认去的一侧：侧栏面板在左，AI 在右。
    pub fn preferred_slot(self) -> SideSlot {
        match self {
            Self::Ai => SideSlot::Right,
            Self::Explorer | Self::Outline | Self::Search => SideSlot::Left,
        }
    }

    /// 写进布局文件的键。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Outline => "outline",
            Self::Search => "search",
            Self::Ai => "ai",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "explorer" => Self::Explorer,
            "outline" => Self::Outline,
            "search" => Self::Search,
            "ai" => Self::Ai,
            _ => return None,
        })
    }
}

/// 界面布局与外观设置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppearanceSettings {
    pub theme: ThemeChoice,
    /// 左侧栏显示的面板；每侧最多一个，与 `right_panel` 互相独立。
    pub left_panel: Option<PanelKind>,
    pub right_panel: Option<PanelKind>,
    pub show_output: bool,
    pub show_status_bar: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Dark,
            left_panel: Some(PanelKind::Explorer),
            right_panel: None,
            show_output: true,
            show_status_bar: true,
        }
    }
}

impl AppearanceSettings {
    /// 面板当前是否显示在某一侧。
    pub fn show(&self, kind: PanelKind) -> bool {
        self.slot_of(kind).is_some()
    }

    pub fn show_explorer(&self) -> bool {
        self.show(PanelKind::Explorer)
    }

    pub fn show_outline(&self) -> bool {
        self.show(PanelKind::Outline)
    }

    pub fn show_search(&self) -> bool {
        self.show(PanelKind::Search)
    }

    pub fn show_ai(&self) -> bool {
        self.show(PanelKind::Ai)
    }

    /// 面板所在的一侧；`None` 表示没有显示。
    pub fn slot_of(&self, kind: PanelKind) -> Option<SideSlot> {
        if self.left_panel == Some(kind) {
            Some(SideSlot::Left)
        } else if self.right_panel == Some(kind) {
            Some(SideSlot::Right)
        } else {
            None
        }
    }

    /// 某一侧当前显示的面板。
    pub fn panel_in(&self, slot: SideSlot) -> Option<PanelKind> {
        match slot {
            SideSlot::Left => self.left_panel,
            SideSlot::Right => self.right_panel,
        }
    }

    fn set_slot(&mut self, slot: SideSlot, kind: Option<PanelKind>) {
        match slot {
            SideSlot::Left => self.left_panel = kind,
            SideSlot::Right => self.right_panel = kind,
        }
    }

    /// 打开面板：偏好的那一侧空着就放那里，否则放到另一侧的空位；
    /// 两侧都占用时替换偏好那一侧的面板。
    pub fn open_panel(&mut self, kind: PanelKind) {
        if self.show(kind) {
            return;
        }
        let preferred = kind.preferred_slot();
        let other = preferred.other();
        let slot = if self.panel_in(preferred).is_none() {
            preferred
        } else if self.panel_in(other).is_none() {
            other
        } else {
            preferred
        };
        // 目标侧原来的面板随之关闭。
        self.set_slot(slot, Some(kind));
    }

    pub fn close_panel(&mut self, kind: PanelKind) {
        if self.left_panel == Some(kind) {
            self.left_panel = None;
        }
        if self.right_panel == Some(kind) {
            self.right_panel = None;
        }
    }

    pub fn toggle_panel(&mut self, kind: PanelKind) {
        if self.show(kind) {
            self.close_panel(kind);
        } else {
            self.open_panel(kind);
        }
    }

    /// 把面板移到指定一侧；目标侧原来的面板关闭。
    pub fn move_panel(&mut self, kind: PanelKind, slot: SideSlot) {
        if self.slot_of(kind) == Some(slot) {
            return;
        }
        self.close_panel(kind);
        self.set_slot(slot, Some(kind));
    }

    pub fn set_panel(&mut self, kind: PanelKind, show: bool) {
        if show {
            self.open_panel(kind);
        } else {
            self.close_panel(kind);
        }
    }

    pub fn toggle_explorer(&mut self) {
        self.toggle_panel(PanelKind::Explorer);
    }

    pub fn set_explorer(&mut self, show: bool) {
        self.set_panel(PanelKind::Explorer, show);
    }

    pub fn toggle_outline(&mut self) {
        self.toggle_panel(PanelKind::Outline);
    }

    pub fn set_outline(&mut self, show: bool) {
        self.set_panel(PanelKind::Outline, show);
    }

    pub fn toggle_search(&mut self) {
        self.toggle_panel(PanelKind::Search);
    }

    pub fn set_search(&mut self, show: bool) {
        self.set_panel(PanelKind::Search, show);
    }

    pub fn toggle_ai(&mut self) {
        self.toggle_panel(PanelKind::Ai);
    }

    pub fn set_ai(&mut self, show: bool) {
        self.set_panel(PanelKind::Ai, show);
    }
}

/// 代码编辑器设置。
#[derive(Clone, Debug, PartialEq)]
pub struct EditorSettings {
    pub font_size: f32,
    pub soft_wrap: bool,
    pub line_numbers: bool,
    pub folding: bool,
    pub auto_close: bool,
    /// 定时把有改动的文件写回磁盘。
    pub auto_save: bool,
    /// 自动保存间隔（秒）。
    pub auto_save_interval_secs: u64,
    pub syntax: SyntaxSettings,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            soft_wrap: false,
            line_numbers: true,
            folding: true,
            auto_close: true,
            auto_save: false,
            auto_save_interval_secs: 30,
            syntax: SyntaxSettings::default(),
        }
    }
}

impl EditorSettings {
    /// 是否该自动保存：已开启、有未保存文件，且距上次保存超过了间隔。
    pub fn auto_save_due(&self, since_last: std::time::Duration, dirty_files: usize) -> bool {
        self.auto_save
            && dirty_files > 0
            && since_last >= std::time::Duration::from_secs(self.auto_save_interval_secs.max(1))
    }
}

/// 输出面板设置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputSettings {
    pub font_size: u32,
    pub show_timestamps: bool,
    pub max_lines: usize,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            font_size: 12,
            show_timestamps: false,
            max_lines: 2_000,
        }
    }
}

/// DeepSeek 接口的默认地址。
pub const DEFAULT_AI_BASE_URL: &str = "https://api.deepseek.com";
/// DeepSeek 的默认模型。
pub const DEFAULT_AI_MODEL: &str = "deepseek-chat";

/// AI 对话区的默认字号（像素），与 `text_xs` 的视觉大小一致。
pub const DEFAULT_AI_FONT_SIZE: u32 = 12;

/// AI 面板底部「模型」下拉里列出的常用模型：`(值, 界面文字)`。
pub const AI_MODEL_OPTIONS: &[(&str, &str)] = &[
    ("deepseek-chat", "deepseek-chat（对话）"),
    ("deepseek-reasoner", "deepseek-reasoner（推理，不改文件）"),
];

/// AI 写文件与脚本执行操作时的确认方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiApproval {
    /// 写入文件或启动/推进脚本前在对话里询问（默认）。
    #[default]
    Ask,
    /// 自动允许所有工具操作，不再询问。
    Auto,
}

impl AiApproval {
    /// 所有可选项：`(值, 界面文字)`。
    pub const OPTIONS: &'static [(&'static str, &'static str)] = &[
        ("ask", "执行前确认"),
        ("auto", "自动允许所有操作"),
    ];

    /// 写进布局文件的键。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }

    /// AI 面板底栏上显示的短标签。
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Ask => "执行前确认",
            Self::Auto => "全部允许",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim() {
            "auto" => Self::Auto,
            _ => Self::Ask,
        }
    }
}

/// 一次提问里最多连续执行多少轮工具调用。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiRoundLimit {
    /// 默认 6 轮，够完成大多数小改动。
    #[default]
    Six,
    /// 复杂任务可以放宽到 12 轮。
    Twelve,
    /// 不限制轮数，只靠用户点「停止」结束。
    Unlimited,
}

impl AiRoundLimit {
    /// 所有可选项：`(值, 界面文字)`。
    pub const OPTIONS: &'static [(&'static str, &'static str)] = &[
        ("6", "6 轮"),
        ("12", "12 轮"),
        ("unlimited", "一直执行"),
    ];

    /// 写进布局文件的键。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Six => "6",
            Self::Twelve => "12",
            Self::Unlimited => "unlimited",
        }
    }

    /// AI 面板底栏上显示的短标签。
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Six => "6 轮",
            Self::Twelve => "12 轮",
            Self::Unlimited => "一直执行",
        }
    }

    /// 轮数上限；`None` 表示一直执行。
    pub fn limit(self) -> Option<usize> {
        match self {
            Self::Six => Some(6),
            Self::Twelve => Some(12),
            Self::Unlimited => None,
        }
    }

    /// 已经跑完的轮数是否到达上限。
    pub fn reached(self, rounds: usize) -> bool {
        self.limit().is_some_and(|limit| rounds >= limit)
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim() {
            "12" => Self::Twelve,
            "unlimited" | "always" | "none" => Self::Unlimited,
            _ => Self::Six,
        }
    }
}

/// AI 编程设置。
///
/// 注意：API Key 不在这里保存，也不写进布局文件；它只交给
/// [`crate::secret_store`] 放进系统凭据管理器。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiSettings {
    /// 接口基础地址。
    pub base_url: String,
    /// 模型名。
    pub model: String,
    /// 写文件时要不要先问用户。
    pub approval: AiApproval,
    /// 一次提问里最多连续跑多少轮工具调用。
    pub round_limit: AiRoundLimit,
    /// 对话区（消息、代码块、提示）的字号，单位像素。
    pub font_size: u32,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_AI_BASE_URL.to_string(),
            model: DEFAULT_AI_MODEL.to_string(),
            approval: AiApproval::default(),
            round_limit: AiRoundLimit::default(),
            font_size: DEFAULT_AI_FONT_SIZE,
        }
    }
}

impl AiSettings {
    /// 空地址回退到默认地址，避免请求打到空字符串上。
    // 供下一阶段的 DeepSeek 请求层使用。
    #[allow(dead_code)]
    pub fn effective_base_url(&self) -> &str {
        let trimmed = self.base_url.trim();
        if trimmed.is_empty() {
            DEFAULT_AI_BASE_URL
        } else {
            trimmed
        }
    }

    /// 空模型名回退到默认模型。
    // 供下一阶段的 DeepSeek 请求层使用。
    #[allow(dead_code)]
    pub fn effective_model(&self) -> &str {
        let trimmed = self.model.trim();
        if trimmed.is_empty() {
            DEFAULT_AI_MODEL
        } else {
            trimmed
        }
    }
}

/// 全部应用设置。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
    pub editor: EditorSettings,
    pub output: OutputSettings,
    pub ai: AiSettings,
}

/// 全局设置存储：持有唯一的设置实体。
pub struct SettingsStore(Entity<AppSettings>);

impl Global for SettingsStore {}

impl SettingsStore {
    /// 初始化全局设置，返回共享的设置实体。
    pub fn init(cx: &mut App) -> Entity<AppSettings> {
        if !cx.has_global::<Self>() {
            let entity = cx.new(|_| AppSettings::default());
            cx.set_global(Self(entity));
        }
        Self::entity(cx)
    }

    /// 取得设置实体（必须先调用 [`SettingsStore::init`]）。
    pub fn entity(cx: &App) -> Entity<AppSettings> {
        cx.global::<Self>().0.clone()
    }

    /// 读取一份当前设置的快照。
    pub fn snapshot(cx: &App) -> AppSettings {
        Self::entity(cx).read(cx).clone()
    }

    /// 修改设置并通知所有观察者（主窗口会据此刷新界面）。
    pub fn update<R>(cx: &mut App, update: impl FnOnce(&mut AppSettings) -> R) -> R {
        let entity = Self::entity(cx);
        entity.update(cx, |settings, cx| {
            let result = update(settings);
            cx.notify();
            result
        })
    }
}
