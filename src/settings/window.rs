//! 设置窗口：独立的工具窗口，内容由若干设置页组成。
//!
//! 打开方式：活动栏齿轮、状态栏“设置”按钮、`文件 -> 设置…`。
//! 窗口是单例的：重复打开只会激活已有窗口。

use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::*;
use gpui_kit::*;

use super::{AppSettings, SettingsStore, SyntaxLanguage, ThemeChoice};
use crate::theme::{self as ui_theme, Palette};
use crate::title_bar;
use crate::window_frame;

/// 已打开设置窗口的句柄，用于复用窗口。
#[derive(Default)]
struct SettingsWindowHandle(Option<AnyWindowHandle>);

impl Global for SettingsWindowHandle {}

/// 设置窗口内容区的元素 id；分栏组也用它作为面板宽度的状态键。
const SETTINGS_ID: &str = "app-settings";

/// 打开（或聚焦）设置窗口。
pub fn open_settings_window(cx: &mut App) {
    if let Some(handle) = cx.try_global::<SettingsWindowHandle>().and_then(|h| h.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }

    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("设置".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(9.0), px(9.0))),
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(280.0), px(160.0)),
            size(px(780.0), px(560.0)),
        ))),
        window_min_size: Some(size(px(520.0), px(400.0))),
        is_resizable: true,
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };

    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let background = Palette::for_theme(cx).sidebar_bg;
        window_frame::setup(window, background);
        cx.new(|cx| SettingsWindow::new(window, cx))
    });

    match opened {
        Ok((handle, _)) => cx.set_global(SettingsWindowHandle(Some(handle))),
        Err(error) => eprintln!("无法打开设置窗口: {error:#}"),
    }
}

struct SettingsWindow {
    settings: Entity<AppSettings>,
    /// 内容区左右两栏的分栏状态，与 [`Settings`] 内部共用同一个键。
    panes: Option<Entity<ResizableState>>,
    _settings_subscription: Subscription,
    _panes_subscription: Option<Subscription>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = SettingsStore::entity(cx);
        let subscription = cx.observe_in(&settings, window, |_, _, _, cx| cx.notify());
        Self {
            settings,
            panes: None,
            _settings_subscription: subscription,
            _panes_subscription: None,
        }
    }

    /// 取到分栏组用于保存各栏宽度的状态实体（与 [`Settings`] 用同一个键）。
    ///
    /// 只有绘制期间才允许访问窗口上的 keyed state，因此首次在这里登记观察者：
    /// 拖动分隔条时状态变化会触发重绘，遮罩才能跟着移动。
    fn track_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) -> f32 {
        let panes = window.use_keyed_state(SETTINGS_ID, cx, |_, _| ResizableState::default());
        if self.panes.as_ref() != Some(&panes) {
            self._panes_subscription = Some(cx.observe_in(&panes, window, |_, _, _, cx| cx.notify()));
            self.panes = Some(panes.clone());
        }
        let width = panes
            .read(cx)
            .sizes()
            .first()
            .map(|width| width.as_f32());
        width.unwrap_or(ui_theme::SETTINGS_SIDEBAR_WIDTH)
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_theme(cx);
        let pages = build_pages(&self.settings);
        let sidebar_width = self.track_panes(window, cx);

        // 左侧导航栏与窗口同底色，靠 8px 圆角与 1px 细边框与右侧内容区分隔。
        let sidebar_style = StyleRefinement::default()
            .bg(rgb(palette.sidebar_bg))
            .rounded(px(ui_theme::PANE_RADIUS))
            .border_1()
            .border_color(rgba(palette.border));

        v_flex()
            .size_full()
            .bg(rgb(palette.sidebar_bg))
            .child(title_bar::dialog_title_bar(
                palette,
                palette.sidebar_bg,
                "settings-title-bar",
                "设置",
            ))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p(px(ui_theme::PANE_GAP))
                    .child(
                        div()
                            .relative()
                            .size_full()
                            .child(
                                Settings::new(SETTINGS_ID)
                                    .pages(pages)
                                    .sidebar_width(px(ui_theme::SETTINGS_SIDEBAR_WIDTH))
                                    .sidebar_style(&sidebar_style),
                            )
                            .child(
                                // 拖动分隔条自带一条 1px 竖线，紧贴着侧栏边框；
                                // 用底色盖住它，只留下侧栏自己的边框。
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left(px(sidebar_width + 0.5))
                                    .w(px(1.0))
                                    .bg(rgb(palette.sidebar_bg)),
                            ),
                    ),
            )
    }
}

fn build_pages(store: &Entity<AppSettings>) -> Vec<SettingPage> {
    vec![
        SettingPage::new("界面")
            .icon(IconName::Palette)
            .default_open(true)
            .groups([
                SettingGroup::new().title("布局").items([
                    bool_item(
                        store,
                        "资源管理器",
                        |s| s.appearance.show_explorer,
                        |s, v| s.appearance.set_explorer(v),
                        true,
                    )
                    .description("在左侧显示项目文件树（会收起代码大纲与搜索）"),
                    bool_item(
                        store,
                        "搜索面板",
                        |s| s.appearance.show_search,
                        |s, v| s.appearance.set_search(v),
                        false,
                    )
                    .description("在左侧显示项目内文本搜索（会收起资源管理器与代码大纲）"),
                    bool_item(
                        store,
                        "输出面板",
                        |s| s.appearance.show_output,
                        |s, v| s.appearance.show_output = v,
                        true,
                    )
                    .description("在编辑区下方显示输出日志"),
                    bool_item(
                        store,
                        "状态栏",
                        |s| s.appearance.show_status_bar,
                        |s, v| s.appearance.show_status_bar = v,
                        true,
                    )
                    .description("在窗口底部显示文件与语法信息"),
                ]),
                SettingGroup::new().title("主题").items([dropdown_item(
                    store,
                    "颜色主题",
                    ThemeChoice::OPTIONS,
                    |s| s.appearance.theme.as_str().into(),
                    |s, v| s.appearance.theme = ThemeChoice::from_str(v),
                )
                .description("切换界面与编辑器的配色，立即生效")]),
            ]),
        SettingPage::new("编辑器").icon(IconName::FileText).groups([
            SettingGroup::new().title("排版").items([
                number_item(
                    store,
                    "字号",
                    10.0,
                    32.0,
                    1.0,
                    |s| s.editor.font_size as f64,
                    |s, v| s.editor.font_size = v as f32,
                ),
                bool_item(
                    store,
                    "自动换行",
                    |s| s.editor.soft_wrap,
                    |s, v| s.editor.soft_wrap = v,
                    false,
                )
                .description("长行按窗口宽度折行显示"),
                bool_item(
                    store,
                    "显示行号",
                    |s| s.editor.line_numbers,
                    |s, v| s.editor.line_numbers = v,
                    true,
                ),
                bool_item(
                    store,
                    "代码折叠",
                    |s| s.editor.folding,
                    |s, v| s.editor.folding = v,
                    true,
                ),
                bool_item(
                    store,
                    "自动闭合括号",
                    |s| s.editor.auto_close,
                    |s, v| s.editor.auto_close = v,
                    true,
                ),
            ]),
            SettingGroup::new().title("保存").items([
                bool_item(
                    store,
                    "自动保存",
                    |s| s.editor.auto_save,
                    |s, v| s.editor.auto_save = v,
                    false,
                )
                .description("定时把有改动的文件写回磁盘，不改变光标位置"),
                number_item(
                    store,
                    "自动保存间隔（秒）",
                    5.0,
                    600.0,
                    5.0,
                    |s| s.editor.auto_save_interval_secs as f64,
                    |s, v| s.editor.auto_save_interval_secs = v.round().max(1.0) as u64,
                )
                .description("自动保存开启时，每隔这么久检查并保存一次"),
            ]),
            SettingGroup::new().title("语法分析器").items([
                bool_item(
                    store,
                    "启用语法高亮",
                    |s| s.editor.syntax.enabled,
                    |s, v| s.editor.syntax.enabled = v,
                    true,
                )
                .description("关闭后编辑器按纯文本显示"),
                dropdown_item(
                    store,
                    "语法分析器",
                    SyntaxLanguage::OPTIONS,
                    |s| s.editor.syntax.language.as_str().into(),
                    |s, v| s.editor.syntax.language = SyntaxLanguage::from_str(v),
                )
                .description("“自动”按文件扩展名选择；当前内置 Rust 与 JSON 语法"),
            ]),
        ]),
        SettingPage::new("输出")
            .icon(IconName::SquareTerminal)
            .groups([SettingGroup::new().title("显示").items([
                number_item(
                    store,
                    "字号",
                    8.0,
                    24.0,
                    1.0,
                    |s| s.output.font_size as f64,
                    |s, v| s.output.font_size = v as u32,
                ),
                bool_item(
                    store,
                    "显示写入时间",
                    |s| s.output.show_timestamps,
                    |s, v| s.output.show_timestamps = v,
                    false,
                )
                .description("在每行输出前显示相对程序启动的时间"),
                number_item(
                    store,
                    "最多保留行数",
                    200.0,
                    20000.0,
                    100.0,
                    |s| s.output.max_lines as f64,
                    |s, v| s.output.max_lines = v as usize,
                )
                .description("超出后自动丢弃最早的输出"),
            ])]),
    ]
}

fn bool_item(
    store: &Entity<AppSettings>,
    title: &'static str,
    get: fn(&AppSettings) -> bool,
    set: fn(&mut AppSettings, bool),
    default: bool,
) -> SettingItem {
    let read = store.clone();
    let write = store.clone();

    let field = SettingField::switch(
        move |cx: &App| get(read.read(cx)),
        move |value: bool, cx: &mut App| {
            write.update(cx, |settings, cx| {
                set(settings, value);
                cx.notify();
            });
        },
    )
    .default_value(default);

    SettingItem::new(title, field)
}

fn dropdown_item(
    store: &Entity<AppSettings>,
    title: &'static str,
    options: &'static [(&'static str, &'static str)],
    get: fn(&AppSettings) -> SharedString,
    set: fn(&mut AppSettings, &str),
) -> SettingItem {
    let read = store.clone();
    let write = store.clone();

    let options = options
        .iter()
        .map(|(value, label)| (SharedString::from(*value), SharedString::from(*label)))
        .collect::<Vec<_>>();

    let field = SettingField::dropdown(
        options,
        move |cx: &App| get(read.read(cx)),
        move |value: SharedString, cx: &mut App| {
            write.update(cx, |settings, cx| {
                set(settings, &value);
                cx.notify();
            });
        },
    );

    SettingItem::new(title, field)
}

fn number_item(
    store: &Entity<AppSettings>,
    title: &'static str,
    min: f64,
    max: f64,
    step: f64,
    get: fn(&AppSettings) -> f64,
    set: fn(&mut AppSettings, f64),
) -> SettingItem {
    let read = store.clone();
    let write = store.clone();

    let field = SettingField::number_input(
        NumberFieldOptions { min, max, step },
        move |cx: &App| get(read.read(cx)),
        move |value: f64, cx: &mut App| {
            write.update(cx, |settings, cx| {
                set(settings, value);
                cx.notify();
            });
        },
    );

    SettingItem::new(title, field)
}
