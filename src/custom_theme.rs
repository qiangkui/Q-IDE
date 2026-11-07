//! 自定义主题：多个具名配色方案，保存在可执行文件同目录的 `themes.txt`。
//!
//! 文件格式与 `layout.txt` 一样是逐行 `键=值`，每个主题以 `theme=名称` 开头：
//!
//! ```text
//! theme=深蓝
//! base=dark
//! window_bg=#0B1A2B
//! …
//! ```
//!
//! `base` 决定没写在文件里的颜色取自深色还是浅色内置主题，因此手工改几个键
//! 就能得到一个能用的主题；未知键与写坏的颜色会被忽略。

use std::fs;
use std::io;
#[cfg(test)]
use std::path::Path;

use gpui_kit::component::ThemeMode;
use gpui_kit::*;

use crate::layout;
use crate::settings::ThemeChoice;
use crate::theme::Palette;

/// 主题文件名。
pub const FILE_NAME: &str = "themes.txt";

/// 自定义主题里可编辑的颜色项。
pub struct ColorSpec {
    /// 主题文件里的键名。
    pub key: &'static str,
    /// 设置界面上的名称。
    pub label: &'static str,
    /// 设置界面上的说明。
    pub hint: &'static str,
}

/// 可编辑的颜色，顺序即 `CustomTheme::colors` 的顺序。
pub const COLORS: &[ColorSpec] = &[
    ColorSpec {
        key: "window_bg",
        label: "窗口底色",
        hint: "编辑器与窗口的底色",
    },
    ColorSpec {
        key: "activity_bar_bg",
        label: "活动栏底色",
        hint: "最左侧图标栏的底色",
    },
    ColorSpec {
        key: "sidebar_bg",
        label: "侧栏底色",
        hint: "资源管理器、菜单与标题栏的底色",
    },
    ColorSpec {
        key: "panel_bg",
        label: "内容区底色",
        hint: "输出面板、列表的底色",
    },
    ColorSpec {
        key: "accent",
        label: "选中/高亮色",
        hint: "菜单选中项、设置导航选中项的底色",
    },
    ColorSpec {
        key: "text",
        label: "正文文字",
        hint: "标题、标签与侧栏文字的颜色",
    },
    ColorSpec {
        key: "muted_text",
        label: "次要文字",
        hint: "说明文字、路径等的颜色",
    },
    ColorSpec {
        key: "selection",
        label: "选区底色",
        hint: "可写 #RRGGBBAA，末尾两位是透明度",
    },
    ColorSpec {
        key: "border",
        label: "分隔线",
        hint: "可写 #RRGGBBAA，末尾两位是透明度",
    },
    ColorSpec {
        key: "folder",
        label: "文件夹图标",
        hint: "资源管理器里的文件夹",
    },
    ColorSpec {
        key: "file",
        label: "文件图标",
        hint: "资源管理器里的文件",
    },
];

/// 一个具名的自定义主题。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomTheme {
    pub name: String,
    /// 基础配色是深色还是浅色。
    pub dark: bool,
    /// 与 [`COLORS`] 一一对应的颜色（`0xRRGGBB` 或 `0xRRGGBBAA`）。
    pub colors: Vec<u32>,
}

impl CustomTheme {
    pub fn mode(&self) -> ThemeMode {
        if self.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        }
    }

    /// 把自定义颜色盖到基础配色上，得到实际渲染用的调色板。
    pub fn palette(&self) -> Palette {
        let mut palette = if self.dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        for (spec, value) in COLORS.iter().zip(&self.colors) {
            let value = *value;
            match spec.key {
                "window_bg" => palette.window_bg = rgb_value(value),
                "activity_bar_bg" => palette.activity_bar_bg = rgb_value(value),
                "sidebar_bg" => {
                    palette.sidebar_bg = rgb_value(value);
                    palette.popover_bg = rgb_value(value);
                }
                "panel_bg" => palette.panel_bg = rgb_value(value),
                "accent" => {
                    palette.menu_item_active = rgb_value(value);
                    palette.nav_selected = rgb_value(value);
                    // 悬停比选中暗一档，设置窗口的导航才能看出当前项。
                    palette.nav_hover = blend(palette.window_bg, palette.nav_selected, 0.5);
                }
                "text" => {
                    let text = rgb_value(value);
                    palette.title_text = text;
                    palette.menu_text = text;
                    palette.sidebar_text = text;
                    palette.tab_active_text = text;
                    palette.tab_inactive_text = text;
                }
                "muted_text" => palette.muted_text = rgb_value(value),
                "selection" => palette.selection = with_alpha(value, palette.selection),
                "border" => palette.border = with_alpha(value, palette.border),
                "folder" => palette.folder = rgb_value(value),
                "file" => palette.file = rgb_value(value),
                _ => {}
            }
        }
        palette
    }
}

/// 「新建主题」在下拉框里的取值。
pub const NEW_THEME_KEY: &str = "__new__";

/// 设置窗口里正在编辑的主题草稿：颜色先按文本保存，允许输入到一半。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeDraft {
    /// 草稿来源，决定名称/颜色能否编辑、能否删除。
    pub kind: DraftKind,
    /// 名称输入框里的文本。
    pub name: String,
    /// 基础配色：没写进颜色框的字段取自它。
    pub dark: bool,
    /// 与 [`COLORS`] 一一对应的文本，例如 `#1E1E1E`。
    pub colors: Vec<String>,
    /// 上一次操作失败的原因。
    pub error: Option<String>,
}

/// 草稿对应的主题类型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftKind {
    /// 内置深色主题：只读。
    Dark,
    /// 内置浅色主题：只读。
    Light,
    /// 已保存的自定义主题；`original` 是它在文件里的名字。
    Saved { original: String },
    /// 正在新建的主题。
    New,
}

impl ThemeDraft {
    /// 内置深/浅色主题：名称固定、颜色只读。
    pub fn builtin(dark: bool) -> Self {
        let palette = if dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        let kind = if dark {
            DraftKind::Dark
        } else {
            DraftKind::Light
        };
        let name = if dark { "深色" } else { "浅色" };
        Self::from_palette(kind, name, &palette, dark)
    }

    /// 已保存的自定义主题。
    pub fn saved(theme: &CustomTheme) -> Self {
        Self {
            kind: DraftKind::Saved {
                original: theme.name.clone(),
            },
            name: theme.name.clone(),
            dark: theme.dark,
            colors: theme.colors.iter().copied().map(format_color).collect(),
            error: None,
        }
    }

    /// 新建主题：名字留空，颜色以当前主题为底。
    pub fn new_from(palette: &Palette, dark: bool) -> Self {
        Self::from_palette(DraftKind::New, "", palette, dark)
    }

    fn from_palette(
        kind: DraftKind,
        name: impl Into<String>,
        palette: &Palette,
        dark: bool,
    ) -> Self {
        Self {
            kind,
            name: name.into(),
            dark,
            colors: COLORS
                .iter()
                .map(|spec| format_color(palette_value(palette, spec.key)))
                .collect(),
            error: None,
        }
    }

    /// 内置主题只能看不能改。
    pub fn is_builtin(&self) -> bool {
        matches!(self.kind, DraftKind::Dark | DraftKind::Light)
    }

    pub fn is_editable(&self) -> bool {
        !self.is_builtin()
    }

    /// 正在新建、还没写进主题文件。
    pub fn is_new(&self) -> bool {
        matches!(self.kind, DraftKind::New)
    }

    /// 只有已保存的自定义主题才能删除。
    pub fn can_delete(&self) -> bool {
        matches!(self.kind, DraftKind::Saved { .. })
    }

    /// 已保存主题在文件里原来的名字。
    pub fn saved_original(&self) -> Option<&str> {
        match &self.kind {
            DraftKind::Saved { original } => Some(original),
            _ => None,
        }
    }

    /// 下拉框当前应显示的取值。
    pub fn pick_key(&self) -> &str {
        match &self.kind {
            DraftKind::Dark => "dark",
            DraftKind::Light => "light",
            DraftKind::Saved { .. } => &self.name,
            DraftKind::New => NEW_THEME_KEY,
        }
    }

    /// 设置里的主题是否就是这份草稿（外部切换主题后用来判断要不要刷新草稿）。
    pub fn matches_choice(&self, choice: &ThemeChoice) -> bool {
        match (&self.kind, choice) {
            (DraftKind::Dark, ThemeChoice::Dark) => true,
            (DraftKind::Light, ThemeChoice::Light) => true,
            (DraftKind::Saved { original }, ThemeChoice::Custom(name)) => {
                &self.name == name || original == name
            }
            _ => false,
        }
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
        self.error = None;
    }

    pub fn set_color(&mut self, index: usize, text: impl Into<String>) {
        if let Some(slot) = self.colors.get_mut(index) {
            *slot = text.into();
        }
        self.error = None;
    }

    /// 校验并转换成可保存的主题。
    pub fn to_theme(&self) -> Result<CustomTheme, String> {
        if self.is_builtin() {
            return Err("内置主题不能修改，请先「新建主题」".to_string());
        }
        let name = validate_name(&self.name)?;
        let mut colors = Vec::with_capacity(COLORS.len());
        for (spec, text) in COLORS.iter().zip(&self.colors) {
            match parse_color(text) {
                Some(color) => colors.push(color),
                None => {
                    return Err(format!(
                        "“{}”要写成 #RRGGBB 或 #RRGGBBAA",
                        spec.label
                    ));
                }
            }
        }
        Ok(CustomTheme {
            name,
            dark: self.dark,
            colors,
        })
    }

    /// 已保存主题的即时替换：名称或颜色还没输完时返回 `None`。
    pub fn replace_saved(&self) -> Option<CustomTheme> {
        if !self.can_delete() {
            return None;
        }
        self.to_theme().ok()
    }
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("请先填写主题名称".to_string());
    }
    if name.chars().count() > 40 {
        return Err("主题名称不能超过 40 个字符".to_string());
    }
    if is_reserved_name(name) {
        return Err("主题名称不能使用 dark 或 light".to_string());
    }
    if name.contains(['=', '\n', '\r']) {
        return Err("主题名称不能包含 = 或换行".to_string());
    }
    Ok(name.to_string())
}

/// 主题库：`themes.txt` 里的全部自定义主题。
#[derive(Clone, Debug, Default)]
pub struct ThemeStore {
    themes: Vec<CustomTheme>,
}

impl ThemeStore {
    /// 初始化全局主题库（已初始化则直接复用）。
    pub fn init(cx: &mut App) -> Entity<Self> {
        if !cx.has_global::<GlobalStore>() {
            let store = cx.new(|_| {
                let store = Self::load();
                // 第一次运行时把带说明的空文件写出来，方便用户直接查看和手工编辑。
                if !layout::config_candidates(FILE_NAME)
                    .first()
                    .is_some_and(|path| path.exists())
                {
                    let _ = store.save();
                }
                store
            });
            cx.set_global(GlobalStore(store.clone()));
        }
        Self::entity(cx)
    }

    pub fn entity(cx: &App) -> Entity<Self> {
        cx.global::<GlobalStore>().0.clone()
    }

    pub fn try_entity(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalStore>().map(|store| store.0.clone())
    }

    /// 从可执行文件同目录（不可用时为用户目录）读取主题文件。
    pub fn load() -> Self {
        for path in layout::config_candidates(FILE_NAME) {
            if let Ok(text) = fs::read_to_string(&path) {
                return Self::parse(&text);
            }
        }
        Self::default()
    }

    #[cfg(test)]
    pub fn load_from(file: &Path) -> Self {
        fs::read_to_string(file)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let mut themes: Vec<CustomTheme> = Vec::new();
        let mut builder = Builder::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                push_theme(&mut themes, std::mem::take(&mut builder));
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "theme" => {
                    push_theme(&mut themes, std::mem::take(&mut builder));
                    builder.name = value.to_string();
                }
                "base" => builder.dark = !value.eq_ignore_ascii_case("light"),
                key => {
                    if let Some(index) = COLORS.iter().position(|spec| spec.key == key)
                        && let Some(color) = parse_color(value)
                    {
                        builder.color(index, color);
                    }
                }
            }
        }
        push_theme(&mut themes, builder);
        Self { themes }
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("# Q-IDE 自定义主题文件，与可执行文件放在同一目录\n");
        out.push_str("# 每个主题从 theme=名称 开始；base=dark|light 决定未列出的颜色取自哪套内置配色\n");
        out.push_str("# 颜色写成 #RRGGBB 或 #RRGGBBAA（末尾两位是透明度，00 为完全透明）\n");
        out.push_str("# 示例：\n");
        out.push_str("# theme=深蓝\n");
        out.push_str("# base=dark\n");
        out.push_str("# window_bg=#0B1A2B\n");
        out.push_str("# sidebar_bg=#10233A\n");
        for theme in &self.themes {
            out.push('\n');
            out.push_str(&format!("theme={}\n", theme.name));
            out.push_str(&format!(
                "base={}\n",
                if theme.dark { "dark" } else { "light" }
            ));
            for (spec, value) in COLORS.iter().zip(&theme.colors) {
                out.push_str(&format!("{}={}\n", spec.key, format_color(*value)));
            }
        }
        out
    }

    /// 写回 exe 同目录；目录不可写时依次退到用户目录、临时目录。
    pub fn save(&self) -> io::Result<()> {
        let text = self.to_text();
        let mut last: Option<io::Error> = None;
        for path in layout::config_candidates(FILE_NAME) {
            match layout::write_text(&path, &text) {
                Ok(()) => return Ok(()),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("没有可写的主题文件位置")))
    }

    #[cfg(test)]
    pub fn save_to(&self, file: &Path) -> io::Result<()> {
        layout::write_text(file, &self.to_text())
    }

    pub fn themes(&self) -> &[CustomTheme] {
        &self.themes
    }

    pub fn find(&self, name: &str) -> Option<&CustomTheme> {
        self.themes.iter().find(|theme| theme.name == name)
    }

    /// 同名主题会被替换，否则追加到末尾。
    pub fn upsert(&mut self, theme: CustomTheme) {
        match self.themes.iter_mut().find(|item| item.name == theme.name) {
            Some(slot) => *slot = theme,
            None => self.themes.push(theme),
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.themes.len();
        self.themes.retain(|theme| theme.name != name);
        self.themes.len() != before
    }
}

/// 全局主题库。
struct GlobalStore(Entity<ThemeStore>);

impl Global for GlobalStore {}

/// 按名字取一份自定义主题（主题库尚未初始化或名字不存在时返回 `None`）。
pub fn find(cx: &App, name: &str) -> Option<CustomTheme> {
    ThemeStore::try_entity(cx)?
        .read(cx)
        .find(name)
        .cloned()
}

/// `dark` / `light` 留给内置主题，自定义主题不能占用。
pub fn is_reserved_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("dark") || name.eq_ignore_ascii_case("light")
}

/// 解析 `#RRGGBB` / `#RRGGBBAA`（`#` 可省略）。
pub fn parse_color(text: &str) -> Option<u32> {
    let text = text.trim();
    let text = text.strip_prefix('#').unwrap_or(text);
    if text.len() != 6 && text.len() != 8 {
        return None;
    }
    if !text.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(text, 16).ok()
}

/// 有透明度时写成 8 位，否则写 6 位，方便手工编辑。
pub fn format_color(value: u32) -> String {
    if value > 0x00ff_ffff {
        format!("#{value:08X}")
    } else {
        format!("#{value:06X}")
    }
}

fn rgb_value(value: u32) -> u32 {
    value & 0x00ff_ffff
}

/// 只给了 RGB 的颜色补上基础配色里的透明度，避免分隔线/选区变成全透明。
fn with_alpha(value: u32, base: u32) -> u32 {
    if value > 0x00ff_ffff {
        value
    } else {
        (value << 8) | (base & 0xFF)
    }
}

fn blend(from: u32, to: u32, ratio: f32) -> u32 {
    let channel = |shift: u32| {
        let from = ((from >> shift) & 0xFF) as f32;
        let to = ((to >> shift) & 0xFF) as f32;
        ((from + (to - from) * ratio).round() as u32) & 0xFF
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

/// 从调色板取出某个可编辑颜色项当前的值。
fn palette_value(palette: &Palette, key: &str) -> u32 {
    match key {
        "window_bg" => palette.window_bg,
        "activity_bar_bg" => palette.activity_bar_bg,
        "sidebar_bg" => palette.sidebar_bg,
        "panel_bg" => palette.panel_bg,
        "accent" => palette.menu_item_active,
        "text" => palette.title_text,
        "muted_text" => palette.muted_text,
        "selection" => palette.selection,
        "border" => palette.border,
        "folder" => palette.folder,
        "file" => palette.file,
        _ => 0,
    }
}

struct Builder {
    name: String,
    dark: bool,
    colors: Vec<Option<u32>>,
}

impl Default for Builder {
    fn default() -> Self {
        // 文件里没写 base 时按深色处理，与应用默认主题一致。
        Self {
            name: String::new(),
            dark: true,
            colors: Vec::new(),
        }
    }
}

impl Builder {
    fn color(&mut self, index: usize, value: u32) {
        if self.colors.len() <= index {
            self.colors.resize(index + 1, None);
        }
        self.colors[index] = Some(value);
    }
}

fn push_theme(themes: &mut Vec<CustomTheme>, builder: Builder) {
    if builder.name.trim().is_empty() {
        return;
    }
    let base = if builder.dark {
        Palette::dark()
    } else {
        Palette::light()
    };
    let name = builder.name.trim().to_string();
    if is_reserved_name(&name) {
        return;
    }
    let colors = COLORS
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            builder
                .colors
                .get(index)
                .copied()
                .flatten()
                .unwrap_or_else(|| palette_value(&base, spec.key))
        })
        .collect();
    let theme = CustomTheme {
        name,
        dark: builder.dark,
        colors,
    };
    match themes.iter_mut().find(|item| item.name == theme.name) {
        Some(slot) => *slot = theme,
        None => themes.push(theme),
    }
}
