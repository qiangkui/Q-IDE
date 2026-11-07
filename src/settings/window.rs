//! 设置窗口：独立的工具窗口，内容由若干设置页组成。
//!
//! 打开方式：活动栏齿轮、状态栏“设置”按钮、`文件 -> 设置…`。
//! 窗口是单例的：重复打开只会激活已有窗口。

use gpui_kit::component::button::Button;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::Colorize as _;
use gpui_kit::component::*;
use gpui_kit::*;

use super::{AppSettings, SettingsStore, SyntaxLanguage, ThemeChoice};
use crate::custom_theme::{self, ColorSpec, DraftKind, ThemeDraft, ThemeStore, COLORS};
use crate::theme::{self as ui_theme, Palette};
use crate::title_bar;
use crate::window_frame;

/// 已打开设置窗口的句柄，用于复用窗口。
#[derive(Default)]
struct SettingsWindowHandle(Option<AnyWindowHandle>);

impl Global for SettingsWindowHandle {}

/// 设置窗口内容区的元素 id；分栏组也用它作为面板宽度的状态键。
const SETTINGS_ID: &str = "app-settings";

/// 设置窗口上次的尺寸（逻辑像素），由主窗口保存布局时写进 `layout.txt`。
#[derive(Default)]
struct SettingsWindowSize(Option<(f32, f32)>);

impl Global for SettingsWindowSize {}

/// 记下设置窗口的尺寸，供布局保存时读取。
pub fn remembered_window_size(cx: &App) -> Option<(f32, f32)> {
    cx.try_global::<SettingsWindowSize>()
        .and_then(|size| size.0)
}

fn remember_window_size(window: &Window, cx: &mut App) {
    let bounds = window.bounds();
    let next = (
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    );
    let changed = cx
        .try_global::<SettingsWindowSize>()
        .is_none_or(|size| size.0 != Some(next));
    if changed {
        cx.set_global(SettingsWindowSize(Some(next)));
    }
}

/// 打开（或聚焦）设置窗口。
pub fn open_settings_window(cx: &mut App) {
    if let Some(handle) = cx.try_global::<SettingsWindowHandle>().and_then(|h| h.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }

    // 上次关闭时的尺寸；没有记录时用默认大小。
    let (width, height) = remembered_window_size(cx)
        .or_else(|| crate::layout::Layout::load().settings_size)
        .unwrap_or((780.0, 560.0));
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("设置".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(9.0), px(9.0))),
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(280.0), px(160.0)),
            size(px(width), px(height)),
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
    themes: Entity<ThemeStore>,
    /// 「自定义主题」分组里正在编辑的草稿。
    draft: Entity<ThemeDraft>,
    /// 内容区左右两栏的分栏状态，与 [`Settings`] 内部共用同一个键。
    panes: Option<Entity<ResizableState>>,
    _subscriptions: Vec<Subscription>,
    _panes_subscription: Option<Subscription>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = SettingsStore::entity(cx);
        let themes = ThemeStore::init(cx);
        // 打开设置时重新读一遍主题文件，手工改过的 themes.txt 也能立即生效。
        themes.update(cx, |store, cx| {
            *store = ThemeStore::load();
            cx.notify();
        });
        ui_theme::apply_from_settings(cx);
        let draft_value = {
            let choice = settings.read(cx).appearance.theme.clone();
            draft_for_choice(&choice, themes.read(cx))
        };
        let draft = cx.new(|_| draft_value);
        let settings_draft = draft.clone();
        let settings_themes = themes.clone();
        let settings_subscription =
            cx.observe_in(&settings, window, move |_, settings, _, cx| {
                sync_draft_with_settings(&settings, &settings_draft, &settings_themes, cx);
                cx.notify();
            });
        let size_subscription =
            cx.observe_window_bounds(window, |_, window, cx| remember_window_size(window, cx));
        let subscriptions = vec![
            settings_subscription,
            size_subscription,
            cx.observe_in(&themes, window, |_, _, _, cx| cx.notify()),
            cx.observe_in(&draft, window, |_, _, _, cx| cx.notify()),
        ];
        Self {
            settings,
            themes,
            draft,
            panes: None,
            _subscriptions: subscriptions,
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
        let pages = build_pages(&self.settings, &self.themes, &self.draft, cx);
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

fn build_pages(
    store: &Entity<AppSettings>,
    themes: &Entity<ThemeStore>,
    draft: &Entity<ThemeDraft>,
    cx: &App,
) -> Vec<SettingPage> {
    // 每个分类单独一页：点左侧导航直接切换，不再在同一页里往下滚动找分组。
    vec![
        SettingPage::new("布局")
            .icon(IconName::PanelLeft)
            .default_open(true)
            .group(SettingGroup::new().items([
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
            ])),
        SettingPage::new("主题").icon(IconName::Palette).group(
            SettingGroup::new().items(
                std::iter::once(theme_choice_item(themes, draft, cx))
                    .chain(theme_draft_items(draft, cx)),
            ),
        ),
        SettingPage::new("排版")
            .icon(IconName::ALargeSmall)
            .group(SettingGroup::new().items([
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
            ])),
        SettingPage::new("保存")
            .icon(IconName::HardDrive)
            .group(SettingGroup::new().items([
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
            ])),
        SettingPage::new("语法分析器")
            .icon(IconName::FileText)
            .group(SettingGroup::new().items([
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
            ])),
        SettingPage::new("输出")
            .icon(IconName::SquareTerminal)
            .group(SettingGroup::new().items([
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
            ])),
    ]
}

/// 「颜色主题」下拉：内置深色/浅色 + 主题文件里的每个自定义主题 + 新建主题。
fn theme_choice_item(
    themes: &Entity<ThemeStore>,
    draft: &Entity<ThemeDraft>,
    cx: &App,
) -> SettingItem {
    let mut options = vec![
        (SharedString::from("dark"), SharedString::from("深色（内置）")),
        (SharedString::from("light"), SharedString::from("浅色（内置）")),
    ];
    for theme in themes.read(cx).themes() {
        options.push((
            SharedString::from(theme.name.clone()),
            SharedString::from(theme.name.clone()),
        ));
    }
    options.push((
        SharedString::from(custom_theme::NEW_THEME_KEY),
        SharedString::from("新建主题…"),
    ));

    let read = draft.clone();
    let write = draft.clone();
    let field = SettingField::dropdown(
        options,
        move |cx: &App| SharedString::from(read.read(cx).pick_key()),
        move |value: SharedString, cx: &mut App| select_theme(&value, &write, cx),
    );
    SettingItem::new("颜色主题", field)
        .description("选内置主题或已保存的自定义主题；也可以「新建主题」")
}

/// 下拉框选中项变化：内置主题直接切换；自定义主题载入草稿；新建则清空草稿。
fn select_theme(value: &str, draft: &Entity<ThemeDraft>, cx: &mut App) {
    let next = match value {
        "dark" => {
            SettingsStore::update(cx, |settings| {
                settings.appearance.theme = ThemeChoice::Dark;
            });
            ThemeDraft::builtin(true)
        }
        "light" => {
            SettingsStore::update(cx, |settings| {
                settings.appearance.theme = ThemeChoice::Light;
            });
            ThemeDraft::builtin(false)
        }
        custom_theme::NEW_THEME_KEY => {
            // 以当前正在使用的主题为底，名字留空；先不切换正在使用的主题。
            let active = ui_theme::active(cx);
            ThemeDraft::new_from(&active.palette, active.mode.is_dark())
        }
        name => match custom_theme::find(cx, name) {
            Some(theme) => {
                SettingsStore::update(cx, |settings| {
                    settings.appearance.theme = ThemeChoice::Custom(theme.name.clone());
                });
                ThemeDraft::saved(&theme)
            }
            None => return,
        },
    };
    draft.update(cx, |draft, cx| {
        *draft = next;
        cx.notify();
    });
}

/// 按设置里的主题生成对应草稿（主题已被删除时退回深色）。
fn draft_for_choice(choice: &ThemeChoice, themes: &ThemeStore) -> ThemeDraft {
    match choice {
        ThemeChoice::Dark => ThemeDraft::builtin(true),
        ThemeChoice::Light => ThemeDraft::builtin(false),
        ThemeChoice::Custom(name) => themes
            .find(name)
            .map(ThemeDraft::saved)
            .unwrap_or_else(|| ThemeDraft::builtin(true)),
    }
}

/// 设置里的主题被别处改掉时，让草稿跟着刷新（新建中的草稿保持不动）。
fn sync_draft_with_settings(
    settings: &Entity<AppSettings>,
    draft: &Entity<ThemeDraft>,
    themes: &Entity<ThemeStore>,
    cx: &mut App,
) {
    let choice = settings.read(cx).appearance.theme.clone();
    let snapshot = themes.read(cx).clone();
    draft.update(cx, |draft, cx| {
        if draft.is_new() || draft.matches_choice(&choice) {
            return;
        }
        *draft = draft_for_choice(&choice, &snapshot);
        cx.notify();
    });
}

/// 「主题」页下半部分：名称、各项颜色与操作按钮。
fn theme_draft_items(draft: &Entity<ThemeDraft>, cx: &App) -> Vec<SettingItem> {
    let (editable, is_new) = {
        let draft = draft.read(cx);
        (draft.is_editable(), draft.is_new())
    };
    let name_hint = if !editable {
        "内置主题固定只读，改颜色请先「新建主题」"
    } else if is_new {
        "保存时使用这个名字，例如：深蓝"
    } else {
        "改名会替换掉同名的已保存主题"
    };

    let mut items = vec![
        draft_text_item(
            draft,
            "主题名称",
            name_hint,
            |draft| draft.name.clone(),
            |draft, value| draft.set_name(value),
        )
        .disabled(!editable),
    ];
    for (index, spec) in COLORS.iter().enumerate() {
        items.push(draft_color_item(draft, index, spec, editable));
    }
    items.push(theme_action_item(draft));
    items
}

fn draft_text_item(
    draft: &Entity<ThemeDraft>,
    title: &'static str,
    hint: &'static str,
    get: impl Fn(&ThemeDraft) -> String + 'static,
    set: impl Fn(&mut ThemeDraft, String) + 'static,
) -> SettingItem {
    let read = draft.clone();
    let write = draft.clone();
    let field = SettingField::input(
        move |cx: &App| SharedString::from(get(read.read(cx))),
        move |value: SharedString, cx: &mut App| {
            write.update(cx, |draft, cx| {
                set(draft, value.to_string());
                cx.notify();
            });
            persist_draft_edit(&write, cx);
        },
    );
    SettingItem::new(title, field).description(hint)
}

/// 一行颜色：输入框的底色就是当前颜色，框内右侧的小按钮打开取色器。
fn draft_color_item(
    draft: &Entity<ThemeDraft>,
    index: usize,
    spec: &'static ColorSpec,
    editable: bool,
) -> SettingItem {
    let read = draft.clone();
    let write = draft.clone();
    let key = SharedString::from(format!("theme-color-{index}"));
    let field = SettingField::render(move |_, window, cx| {
        let text = read
            .read(cx)
            .colors
            .get(index)
            .cloned()
            .unwrap_or_default();
        let editable = read.read(cx).is_editable();
        let state = color_field_state(window, cx, &write, index, &text, key.clone());
        sync_color_field(&state, &text, window, cx);

        let (input, picker) = {
            let state = state.read(cx);
            (state.input.clone(), state.picker.clone())
        };
        let palette = Palette::for_theme(cx);
        let parsed = custom_theme::parse_color(&text);
        let fill = parsed
            .map(|color| rgb(color & 0x00ff_ffff))
            .unwrap_or_else(|| rgb(palette.panel_bg));
        let ink = parsed
            .map(contrast_text)
            .unwrap_or_else(|| Hsla::from(rgb(palette.title_text)));

        let mut element = Input::new(&input)
            .bg(fill)
            .text_color(ink)
            .aria_label(spec.label);
        if editable {
            element = element.suffix(color_button(&picker));
        } else {
            element = element.disabled(true);
        }

        h_flex()
            .w_64()
            .items_center()
            .child(div().flex_1().min_w_0().child(element))
    });
    SettingItem::new(spec.label, field)
        .description(spec.hint)
        .disabled(!editable)
}

/// 颜色框里的小按钮：固定浅灰，点开取色器。
fn color_button(picker: &Entity<ColorPickerState>) -> AnyElement {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .size_5()
        .rounded(px(4.))
        .bg(rgb(0xd4d4d4))
        .hover(|style| style.bg(rgb(0xc0c0c0)))
        .child(
            ColorPicker::new(picker)
                .with_size(gpui_kit::component::Size::XSmall)
                .icon(Icon::new(IconName::Palette).text_color(rgb(0x404040)))
                .anchor(Anchor::TopRight),
        )
        .into_any_element()
}

/// 每个颜色行自己的输入框与取色器状态，按颜色下标存在窗口的 keyed state 里。
struct ColorFieldState {
    input: Entity<InputState>,
    picker: Entity<ColorPickerState>,
    /// 上一次同步给控件的文本。
    synced: String,
    _subscriptions: Vec<Subscription>,
}

fn color_field_state(
    window: &mut Window,
    cx: &mut App,
    draft: &Entity<ThemeDraft>,
    index: usize,
    text: &str,
    key: SharedString,
) -> Entity<ColorFieldState> {
    let initial = text.to_string();
    let input_draft = draft.clone();
    let picker_draft = draft.clone();
    window.use_keyed_state(key, cx, move |window, cx| {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial.clone()));
        let picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let input_sub = cx.subscribe(&input, move |_, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let value = input.read(cx).value().to_string();
                input_draft.update(cx, |draft, cx| {
                    draft.set_color(index, value);
                    cx.notify();
                });
                persist_draft_edit(&input_draft, cx);
            }
        });
        let picker_sub = cx.subscribe(&picker, move |_, _, event: &ColorPickerEvent, cx| {
            let ColorPickerEvent::Change(Some(color)) = event else {
                return;
            };
            let text = color.to_hex();
            picker_draft.update(cx, |draft, cx| {
                draft.set_color(index, text);
                cx.notify();
            });
            persist_draft_edit(&picker_draft, cx);
        });
        ColorFieldState {
            input,
            picker,
            synced: initial.clone(),
            _subscriptions: vec![input_sub, picker_sub],
        }
    })
}

/// 切换主题或手动输入后，把文本同步给输入框与取色器。
fn sync_color_field(
    state: &Entity<ColorFieldState>,
    text: &str,
    window: &mut Window,
    cx: &mut App,
) {
    if state.read(cx).synced == text {
        return;
    }
    state.update(cx, |state, cx| {
        state.synced = text.to_string();
        if state.input.read(cx).value().as_str() != text {
            let value = text.to_string();
            state
                .input
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
        if let Some(color) = custom_theme::parse_color(text) {
            let color = Hsla::from(rgba(color));
            if state.picker.read(cx).value() != Some(color) {
                state
                    .picker
                    .update(cx, |picker, cx| picker.set_value(color, window, cx));
            }
        }
    });
}

/// 在给定底色上挑一个看得清的文字颜色。
fn contrast_text(color: u32) -> Hsla {
    let red_channel = ((color >> 16) & 0xFF) as f32;
    let green_channel = ((color >> 8) & 0xFF) as f32;
    let blue_channel = (color & 0xFF) as f32;
    let luminance = 0.299 * red_channel + 0.587 * green_channel + 0.114 * blue_channel;
    if luminance > 140.0 {
        Hsla::from(rgb(0x1f1f1f))
    } else {
        Hsla::from(rgb(0xf5f5f5))
    }
}

fn theme_action_item(draft: &Entity<ThemeDraft>) -> SettingItem {
    let read = draft.clone();
    let save = draft.clone();
    let delete = draft.clone();
    SettingItem::render(move |_, _, cx| {
        let (editable, can_delete, error) = {
            let draft = read.read(cx);
            (draft.is_editable(), draft.can_delete(), draft.error.clone())
        };
        let palette = Palette::for_theme(cx);
        let save_draft = save.clone();
        let delete_draft = delete.clone();
        let row = h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("theme-save")
                    .label("保存为主题")
                    .disabled(!editable)
                    .on_click(move |_, _, cx| save_theme(&save_draft, cx)),
            )
            .child(
                Button::new("theme-delete")
                    .label("删除主题")
                    .disabled(!can_delete)
                    .on_click(move |_, _, cx| delete_theme(&delete_draft, cx)),
            );
        match error {
            Some(error) => row
                .child(div().text_xs().text_color(rgb(palette.error)).child(error))
                .into_any_element(),
            None => row.into_any_element(),
        }
    })
}

/// 已保存主题的即时替换：改名时删掉旧条目，颜色改动直接覆盖原主题。
fn persist_draft_edit(draft: &Entity<ThemeDraft>, cx: &mut App) {
    let (theme, original) = {
        let draft = draft.read(cx);
        let Some(original) = draft.saved_original().map(str::to_string) else {
            return;
        };
        // 名称或颜色还没输完时先不动已保存的内容。
        let Some(theme) = draft.replace_saved() else {
            return;
        };
        (theme, original)
    };
    let name = theme.name.clone();
    let Some(store) = ThemeStore::try_entity(cx) else {
        return;
    };
    let renamed = name != original;
    store.update(cx, |store, cx| {
        store.upsert(theme);
        if renamed {
            store.remove(&original);
        }
        let _ = store.save();
        cx.notify();
    });
    if renamed {
        draft.update(cx, |draft, cx| {
            draft.kind = DraftKind::Saved {
                original: name.clone(),
            };
            cx.notify();
        });
    }
    // 主窗口按新的主题内容重新解析并应用。
    SettingsStore::update(cx, |settings| {
        settings.appearance.theme = ThemeChoice::Custom(name);
    });
}

/// 保存按钮：新建时写进主题库，已保存时再写一次（也用于立刻应用）。
fn save_theme(draft: &Entity<ThemeDraft>, cx: &mut App) {
    let theme = match draft.read(cx).to_theme() {
        Ok(theme) => theme,
        Err(message) => {
            draft.update(cx, |draft, cx| {
                draft.error = Some(message);
                cx.notify();
            });
            return;
        }
    };
    let name = theme.name.clone();
    let original = draft.read(cx).saved_original().map(str::to_string);
    let Some(store) = ThemeStore::try_entity(cx) else {
        return;
    };
    let saved = store.update(cx, |store, cx| {
        store.upsert(theme);
        if let Some(original) = &original
            && original != &name
        {
            store.remove(original);
        }
        let saved = store.save();
        cx.notify();
        saved
    });
    draft.update(cx, |draft, cx| {
        draft.kind = DraftKind::Saved {
            original: name.clone(),
        };
        draft.error = saved
            .as_ref()
            .err()
            .map(|error| format!("写入 themes.txt 失败：{error}"));
        cx.notify();
    });
    // 切到刚保存的主题；改的是当前主题时，内容变化也会触发重新应用。
    SettingsStore::update(cx, |settings| {
        settings.appearance.theme = ThemeChoice::Custom(name);
    });
}

fn delete_theme(draft: &Entity<ThemeDraft>, cx: &mut App) {
    let Some(name) = draft.read(cx).saved_original().map(str::to_string) else {
        return;
    };
    let Some(store) = ThemeStore::try_entity(cx) else {
        return;
    };
    let removed = store.update(cx, |store, cx| {
        let removed = store.remove(&name);
        if removed {
            let _ = store.save();
        }
        cx.notify();
        removed
    });
    if !removed {
        draft.update(cx, |draft, cx| {
            draft.error = Some(format!("没有名为“{name}”的自定义主题"));
            cx.notify();
        });
        return;
    }
    draft.update(cx, |draft, cx| {
        *draft = ThemeDraft::builtin(true);
        cx.notify();
    });
    SettingsStore::update(cx, |settings| {
        settings.appearance.theme = ThemeChoice::Dark;
    });
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
