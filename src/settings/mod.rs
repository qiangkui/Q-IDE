//! 应用设置：数据模型、默认值与全局存储。
//!
//! 设置保存在 `Entity<AppSettings>` 中并通过 [`SettingsStore`] 注册为全局对象，
//! 因此主窗口与设置窗口共享同一份状态：任一侧修改后，观察者会立即收到通知。

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

/// 界面布局与外观设置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppearanceSettings {
    pub theme: ThemeChoice,
    pub show_explorer: bool,
    pub show_outline: bool,
    /// 项目内搜索面板：与资源管理器、代码大纲同侧，三者互斥。
    pub show_search: bool,
    pub show_output: bool,
    pub show_status_bar: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Dark,
            show_explorer: true,
            show_outline: true,
            show_search: false,
            show_output: true,
            show_status_bar: true,
        }
    }
}

impl AppearanceSettings {
    /// 资源管理器与代码大纲共用左侧栏，二者互斥。
    pub fn toggle_explorer(&mut self) {
        self.set_explorer(!self.show_explorer);
    }

    /// 打开资源管理器时自动收起代码大纲与搜索面板。
    pub fn set_explorer(&mut self, show: bool) {
        self.show_explorer = show;
        if show {
            self.show_outline = false;
            self.show_search = false;
        }
    }

    /// 资源管理器与代码大纲共用左侧栏，二者互斥。
    pub fn toggle_outline(&mut self) {
        self.set_outline(!self.show_outline);
    }

    /// 打开代码大纲时自动收起资源管理器与搜索面板。
    pub fn set_outline(&mut self, show: bool) {
        self.show_outline = show;
        if show {
            self.show_explorer = false;
            self.show_search = false;
        }
    }

    /// 搜索面板与资源管理器、代码大纲同侧，三者互斥。
    pub fn toggle_search(&mut self) {
        self.set_search(!self.show_search);
    }

    /// 打开搜索面板时自动收起另外两个侧栏面板。
    pub fn set_search(&mut self, show: bool) {
        self.show_search = show;
        if show {
            self.show_explorer = false;
            self.show_outline = false;
        }
    }

    /// 哪个侧栏面板正在显示（互斥，最多一个）。
    pub fn side_panel(&self) -> Option<SidePanel> {
        if self.show_explorer {
            Some(SidePanel::Explorer)
        } else if self.show_outline {
            Some(SidePanel::Outline)
        } else if self.show_search {
            Some(SidePanel::Search)
        } else {
            None
        }
    }

    /// 修正旧布局里两个侧栏同时打开的状态：默认保留资源管理器。
    pub fn normalize_side_panels(&mut self) {
        let mut panels = [
            (SidePanel::Explorer, self.show_explorer),
            (SidePanel::Outline, self.show_outline),
            (SidePanel::Search, self.show_search),
        ]
        .into_iter()
        .filter(|(_, visible)| *visible);
        // 只留第一个（优先资源管理器 → 代码大纲 → 搜索）。
        let kept = panels.next().map(|(panel, _)| panel);
        self.show_explorer = kept == Some(SidePanel::Explorer);
        self.show_outline = kept == Some(SidePanel::Outline);
        self.show_search = kept == Some(SidePanel::Search);
    }
}

/// 侧栏（与资源管理器同侧）的三个互斥面板。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidePanel {
    Explorer,
    Outline,
    Search,
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

/// 全部应用设置。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
    pub editor: EditorSettings,
    pub output: OutputSettings,
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
