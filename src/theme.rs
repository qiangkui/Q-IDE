//! 主题：深色 / 浅色两套内置配色、自定义主题的解析结果、所有界面共用的尺寸常量，
//! 以及把 gpui 主题与自绘配色对齐的入口。

use std::sync::Arc;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::custom_theme;
use crate::settings::ThemeChoice;

/// 界面配色。渲染时通过 [`Palette::for_theme`] 取当前主题对应的一组颜色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// 窗口底色，比组件更暗。
    pub window_bg: u32,
    /// 活动栏（最左侧图标栏）的背景色。
    pub activity_bar_bg: u32,
    /// 活动栏选中按钮的背景：用偏白的高亮，避免看起来像按下变暗。
    pub activity_bar_selected: u32,
    /// 侧边栏（资源管理器）等组件表面的背景色。
    pub sidebar_bg: u32,
    /// 组件内容区（树形框、输出日志）的背景色。
    pub panel_bg: u32,
    /// 弹出菜单、下拉菜单的背景色。
    pub popover_bg: u32,
    /// 标题栏文字。
    pub title_text: u32,
    /// 顶部菜单文字。
    pub menu_text: u32,
    /// 侧边栏正文文字。
    pub sidebar_text: u32,
    /// 次要说明文字。
    pub muted_text: u32,
    /// 文件夹图标。
    pub folder: u32,
    /// 文件图标。
    pub file: u32,
    /// 当前标签文字。
    pub tab_active_text: u32,
    /// 非当前标签文字。
    pub tab_inactive_text: u32,
    /// 区域之间的细边框（RGBA）。
    pub border: u32,
    /// 半透明悬停高亮（RGBA）。
    pub hover: u32,
    /// 选中文字的底色（输出面板里的选区）。
    pub selection: u32,
    /// 菜单项选中与悬停底色（包括代码补全候选）。
    pub menu_item_active: u32,
    /// 设置窗口左侧导航：选中项底色。
    pub nav_selected: u32,
    /// 设置窗口左侧导航：悬停底色。
    pub nav_hover: u32,
    pub window_button_hover: u32,
    pub close_button_hover: u32,
    /// 输出面板：命令行提示。
    pub command: u32,
    /// 输出面板：普通信息。
    pub info: u32,
    /// 输出面板：成功。
    pub success: u32,
    /// 输出面板：警告。
    pub warning: u32,
    /// 输出面板：错误。
    pub error: u32,
}

impl Palette {
    pub const fn dark() -> Self {
        Self {
            window_bg: 0x181818,
            activity_bar_bg: 0x2d2d2d,
            activity_bar_selected: 0xffffff33,
            sidebar_bg: 0x252526,
            panel_bg: 0x1f1f1f,
            popover_bg: 0x252526,
            title_text: 0xcccccc,
            menu_text: 0xa0a0a0,
            sidebar_text: 0xbdbdbd,
            muted_text: 0x6f6f6f,
            folder: 0xdcb67a,
            file: 0x9cdcfe,
            tab_active_text: 0xfafafa,
            tab_inactive_text: 0xd4d4d4,
            border: 0x8a8a8a66,
            hover: 0xffffff12,
            selection: 0x264f78,
            menu_item_active: 0x3a3a3a,
            nav_selected: 0x3d3d40,
            nav_hover: 0x303032,
            window_button_hover: 0x2a2a2a,
            close_button_hover: 0xc42b1c,
            command: 0x9cdcfe,
            info: 0xdcdcdc,
            success: 0x4ec9b0,
            warning: 0xd7ba7d,
            error: 0xf48771,
        }
    }

    pub const fn light() -> Self {
        Self {
            window_bg: 0xf3f3f3,
            activity_bar_bg: 0xe7e7e7,
            activity_bar_selected: 0xffffffd9,
            sidebar_bg: 0xf8f8f8,
            panel_bg: 0xffffff,
            popover_bg: 0xffffff,
            title_text: 0x333333,
            menu_text: 0x5a5a5a,
            sidebar_text: 0x3b3b3b,
            muted_text: 0x8a8a8a,
            folder: 0xa07428,
            file: 0x0451a5,
            tab_active_text: 0x1f1f1f,
            tab_inactive_text: 0x4a4a4a,
            border: 0x00000022,
            hover: 0x00000010,
            selection: 0xadd6ff,
            menu_item_active: 0xe8e8e8,
            nav_selected: 0xdedee3,
            nav_hover: 0xebebef,
            window_button_hover: 0xe0e0e0,
            close_button_hover: 0xc42b1c,
            command: 0x0451a5,
            info: 0x333333,
            success: 0x0a7b45,
            warning: 0x8a6500,
            error: 0xb00020,
        }
    }

    /// 取当前生效主题对应的配色（内置或自定义）。
    pub fn for_theme(cx: &App) -> Self {
        active(cx).palette
    }
}

/// 当前生效的配色：内置深/浅色，或某个自定义主题解析出来的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivePalette {
    pub palette: Palette,
    pub mode: ThemeMode,
}

impl Global for ActivePalette {}

/// 按设置里的主题选择解析出实际配色；自定义主题已被删除时退回深色。
pub fn resolve(choice: &ThemeChoice, cx: &App) -> ActivePalette {
    match choice {
        ThemeChoice::Dark => ActivePalette {
            palette: Palette::dark(),
            mode: ThemeMode::Dark,
        },
        ThemeChoice::Light => ActivePalette {
            palette: Palette::light(),
            mode: ThemeMode::Light,
        },
        ThemeChoice::Custom(name) => match custom_theme::find(cx, name) {
            Some(theme) => ActivePalette {
                palette: theme.palette(),
                mode: theme.mode(),
            },
            None => ActivePalette {
                palette: Palette::dark(),
                mode: ThemeMode::Dark,
            },
        },
    }
}

/// 当前正在生效的配色；还没应用过主题时按设置现算一份。
pub fn active(cx: &App) -> ActivePalette {
    if let Some(active) = cx.try_global::<ActivePalette>() {
        return *active;
    }
    let choice = cx
        .try_global::<crate::settings::SettingsStore>()
        .map(|_| crate::settings::SettingsStore::snapshot(cx).appearance.theme)
        .unwrap_or(ThemeChoice::Dark);
    resolve(&choice, cx)
}

/// 标题栏高度。
pub const TITLE_BAR_HEIGHT: f32 = 34.0;
/// 标题栏顶部留白，让出系统边框的缩放热区。
pub const TITLE_BAR_INSET: f32 = 8.0;
pub const WINDOW_BUTTON_WIDTH: f32 = 44.0;
/// 活动栏固定宽度。
pub const ACTIVITY_BAR_WIDTH: f32 = 48.0;
/// 资源管理器初始宽度。
pub const EXPLORER_WIDTH: f32 = 220.0;
/// 代码大纲侧栏初始宽度。
pub const OUTLINE_WIDTH: f32 = 240.0;
/// 搜索面板初始宽度。
pub const SEARCH_WIDTH: f32 = 300.0;

/// AI 编程面板的默认宽度。
pub const AI_WIDTH: f32 = 340.0;
/// 设置窗口左侧导航栏初始宽度。
pub const SETTINGS_SIDEBAR_WIDTH: f32 = 250.0;
/// 拖动分隔条时允许的最小尺寸。
pub const EXPLORER_MIN_WIDTH: f32 = 120.0;
pub const OUTLINE_MIN_WIDTH: f32 = 150.0;
pub const OUTLINE_MAX_WIDTH: f32 = 520.0;
pub const SEARCH_MIN_WIDTH: f32 = 200.0;
pub const SEARCH_MAX_WIDTH: f32 = 560.0;
pub const AI_MIN_WIDTH: f32 = 240.0;
pub const AI_MAX_WIDTH: f32 = 720.0;
pub const OUTPUT_MIN_HEIGHT: f32 = 80.0;
/// 窗口内容区的外边距。
pub const PANE_GAP: f32 = 8.0;
/// 面板之间的可见间隔；面板自身向内缩 `PANEL_GAP / 2`。
pub const PANEL_GAP: f32 = 6.0;
/// 各区域的圆角半径。
pub const PANE_RADIUS: f32 = 8.0;
/// 标签栏高度；标签铺满整个高度。
pub const TAB_BAR_HEIGHT: f32 = 35.0;
/// 标签栏下方路径栏的高度。
pub const PATH_BAR_HEIGHT: f32 = 22.0;
/// 输出组件初始高度，编辑框占据其余空间。
pub const OUTPUT_HEIGHT: f32 = 190.0;
/// 面板标题栏（输出、侧边栏）高度。
pub const PANEL_HEADER_HEIGHT: f32 = 30.0;
/// 侧边栏标题栏高度。
pub const EXPLORER_HEADER_HEIGHT: f32 = 35.0;
/// 状态栏高度。
pub const STATUS_BAR_HEIGHT: f32 = 26.0;

/// 编辑器字体族。
pub const MONO_FONT: &str = "Consolas";

/// 调试暂停行的背景色：淡黄色半透明，深浅主题下都看得见。
///
/// gpui 的 `hsla` 色相是 0..1（不是角度），`1/6` 正好是黄色 60°。
pub fn debug_line_highlight() -> Hsla {
    hsla(1.0 / 6.0, 0.95, 0.62, 0.35)
}

/// 括号配对高亮的底色：淡蓝色半透明，两个主题下都能看清。
pub fn bracket_match_highlight() -> Hsla {
    hsla(0.58, 0.85, 0.62, 0.45)
}

/// 应用主题：切换 gpui 主题，并让 Root 表面、编辑器与弹出层使用同一底色。
pub fn apply(active: ActivePalette, cx: &mut App) {
    let ActivePalette { palette, mode } = active;
    Theme::change(mode, None, cx);

    Theme::update(cx, |theme| {
        theme.tokens.background = Hsla::from(rgb(palette.window_bg)).into();
        theme.popover = Hsla::from(rgb(palette.popover_bg));
        theme.tokens.popover = Hsla::from(rgb(palette.popover_bg)).into();
        // 设置窗口左侧导航：组件默认的选中底色（#262626）和本应用的侧栏底色
        // （#252526）几乎一样，选中项分辨不出来，这里换成更亮一档的灰色。
        theme.colors.sidebar_accent = Hsla::from(rgb(palette.nav_hover));
        theme.tokens.sidebar_accent = Hsla::from(rgb(palette.nav_selected)).into();
        // 补全菜单使用 MenuItemElement，其选中/悬停背景取 accent 而非 list_active。
        theme.accent = Hsla::from(rgb(palette.menu_item_active));
        Arc::make_mut(&mut theme.highlight_theme)
            .style
            .editor_background = Some(Hsla::from(rgb(palette.window_bg)));
    });
    cx.set_global(active);
}

/// 按当前设置重新解析并应用主题（自定义主题内容被修改后调用）。
pub fn apply_from_settings(cx: &mut App) {
    let choice = crate::settings::SettingsStore::snapshot(cx).appearance.theme;
    apply(resolve(&choice, cx), cx);
}
