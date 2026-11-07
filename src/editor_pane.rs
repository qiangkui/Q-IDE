//! 编辑区：文件标签栏 + 代码编辑框。

use std::rc::Rc;

use ::gpui::{Anchor, anchored, deferred};
use gpui_kit::component::button::*;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{self as ui_theme, Palette};
use crate::workspace::OpenFile;

/// 菜单项动作（与主菜单共用的签名）。
pub type MenuCallback = Rc<dyn Fn(&mut Window, &mut App)>;

/// 右键菜单里的一项。
#[derive(Clone)]
pub struct ContextMenuItem {
    pub label: &'static str,
    /// 在这一项前面画一条分隔线。
    pub separator_before: bool,
    pub action: MenuCallback,
}

/// 编辑器右键菜单：窗口坐标里的位置、菜单项与关闭回调。
#[derive(Clone)]
pub struct EditorContextMenu {
    pub position: Point<Pixels>,
    pub items: Vec<ContextMenuItem>,
    pub dismiss: MenuCallback,
}

/// 右键事件回调：把鼠标位置交给面板打开菜单。
pub type OpenContextMenuAction = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

/// 从资源管理器拖文件到标签栏时的回调。
pub type DropFileAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// 编辑区渲染参数。
pub struct EditorPaneProps<'a> {
    pub editor: &'a Entity<EditorState>,
    pub open_files: &'a [OpenFile],
    /// 与 `open_files` 对应：文件是否有未保存的修改。
    pub dirty: &'a [bool],
    pub active: Option<usize>,
    pub palette: Palette,
    /// 编辑器字号，来自设置。
    pub font_size: f32,
    /// 正在显示的右键菜单；`None` 表示没有打开。
    pub context_menu: Option<EditorContextMenu>,
    /// 在编辑区右键时调用，参数是鼠标的窗口坐标。
    pub on_context_menu: OpenContextMenuAction,
    /// 在标签上右键时调用，参数是标签下标与鼠标位置。
    pub on_tab_menu: TabMenuAction,
    /// 从资源管理器把文件拖到标签栏时调用，参数是文件路径。
    pub on_drop_file: DropFileAction,
}

/// 标签栏的回调：参数是标签下标。
pub type TabAction = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// 标签右键回调：标签下标与鼠标位置。
pub type TabMenuAction = Rc<dyn Fn(usize, Point<Pixels>, &mut Window, &mut App)>;

/// 渲染编辑区。`on_activate` / `on_close` 分别用于切换标签和关闭标签。
pub fn render(
    cx: &App,
    props: EditorPaneProps<'_>,
    on_activate: TabAction,
    on_close: TabAction,
) -> AnyElement {
    let EditorPaneProps {
        editor,
        open_files,
        dirty,
        active,
        palette,
        font_size,
        context_menu,
        on_context_menu,
        on_tab_menu,
        on_drop_file,
    } = props;

    // 路径栏显示当前标签对应的文件。
    let active_path = active.and_then(|index| open_files.get(index));

    let body: AnyElement = if open_files.is_empty() {
        div()
            .flex_1()
            .w_full()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted_text))
                    .child("没有打开的文件：在资源管理器中单击文件即可打开"),
            )
            .into_any_element()
    } else {
        div()
            .flex_1()
            .w_full()
            .min_h_0()
            // 右键位置就是菜单的弹出位置（窗口坐标）。
            .on_mouse_up(MouseButton::Right, {
                let on_context_menu = on_context_menu.clone();
                move |event: &MouseUpEvent, window, cx| {
                    on_context_menu(event.position, window, cx);
                }
            })
            .child(
                Editor::new(editor)
                    .bordered(false)
                    .w_full()
                    .h(relative(1.0))
                    .font_family(ui_theme::MONO_FONT)
                    .text_size(px(font_size))
                    .line_height(relative(1.5))
                    // 自绘菜单在面板里显示；这里留一个空的原生菜单，让编辑器
                    // 仍然把光标移到右键位置，同时不弹出系统菜单。
                    .context_menu(|menu, _, _| menu),
            )
            .into_any_element()
    };

    let pane = v_flex()
        .size_full()
        .min_w_0()
        .min_h_0()
        .bg(rgb(palette.window_bg))
        .rounded(px(ui_theme::PANE_RADIUS))
        .border_1()
        .border_color(rgba(palette.border))
        .overflow_hidden()
        .child(
            // 从资源管理器把文件拖到标签栏上就是打开它。
            div()
                .w_full()
                .flex_shrink_0()
                .drag_over::<crate::explorer::ExplorerDrag>(move |style, drag, _, _| {
                    if drag.is_dir {
                        style
                    } else {
                        style.bg(rgba(palette.hover))
                    }
                })
                .on_drop(move |drag: &crate::explorer::ExplorerDrag, window, cx| {
                    cx.stop_propagation();
                    if !drag.is_dir {
                        let path = drag.path.clone();
                        on_drop_file(&path, window, cx);
                    }
                })
                .child(tab_strip(
                    cx,
                    open_files,
                    dirty,
                    active,
                    palette,
                    TabCallbacks {
                        activate: on_activate,
                        close: on_close,
                        menu: on_tab_menu,
                    },
                )),
        )
        // 路径栏夹在标签栏和编辑框之间，编辑器的查找框就落在它下面。
        .when_some(active_path, |this, file| {
            this.child(path_bar(file.id.as_ref(), palette))
        })
        .child(body);

    match context_menu {
        // 菜单放在带 overflow_hidden 的面板外面，靠近边缘时不会被裁掉。
        Some(menu) => div()
            .size_full()
            .child(pane)
            .child(context_menu_element(menu, palette))
            .into_any_element(),
        None => pane.into_any_element(),
    }
}

/// 自绘右键菜单：圆角、边框与底色跟窗口顶部的弹出菜单保持一致。
pub fn context_menu_element(menu: EditorContextMenu, palette: Palette) -> AnyElement {
    let EditorContextMenu {
        position,
        items,
        dismiss,
    } = menu;

    let mut panel = v_flex()
        .id("editor-context-menu")
        .min_w(px(180.0))
        .py(px(4.0))
        .px(px(4.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgb(palette.sidebar_bg))
        .occlude()
        .on_mouse_down_out({
            let dismiss = dismiss.clone();
            move |_, window, cx| dismiss(window, cx)
        });

    for (index, item) in items.into_iter().enumerate() {
        // 第一项上面永远不画分隔线，免得菜单顶部多一条空线。
        if item.separator_before && index > 0 {
            panel = panel.child(div().h(px(1.0)).my(px(4.0)).bg(rgba(palette.border)));
        }
        let dismiss = dismiss.clone();
        let action = item.action;
        panel = panel.child(
            h_flex()
                .id(("editor-context-menu-item", index))
                .w_full()
                .h(px(22.0))
                .items_center()
                .px(px(8.0))
                .rounded(px(3.0))
                .text_xs()
                .text_color(rgb(palette.menu_text))
                .cursor(CursorStyle::PointingHand)
                .hover(move |style| style.bg(rgba(palette.hover)))
                .on_click(move |_, window, cx| {
                    dismiss(window, cx);
                    action(window, cx);
                })
                .child(div().min_w_0().truncate().child(item.label)),
        );
    }

    deferred(
        anchored()
            .position(position)
            .anchor(Anchor::TopLeft)
            .snap_to_window_with_margin(px(8.0))
            .child(panel),
    )
    .into_any_element()
}

/// 标签栏的三个回调。
struct TabCallbacks {
    activate: TabAction,
    close: TabAction,
    menu: TabMenuAction,
}

/// 把完整路径拆成「目录 + 文件名」。
///
/// 路径栏里目录部分可以省略显示，文件名始终保持完整。
pub fn split_path_for_display(path: &str) -> (&str, &str) {
    match path.rfind(['\\', '/']) {
        Some(index) => path.split_at(index + 1),
        None => ("", path),
    }
}

/// 路径栏：底色与编辑框一致，只显示当前文件的完整路径。
fn path_bar(path: &str, palette: Palette) -> AnyElement {
    let (dir, name) = split_path_for_display(path);
    let mut bar = h_flex()
        .id("editor-path-bar")
        .h(px(ui_theme::PATH_BAR_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .px(px(10.0))
        .bg(rgb(palette.window_bg));
    if !dir.is_empty() {
        bar = bar.child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(dir.to_string()),
        );
    }
    bar.child(
        div()
            .flex_shrink_0()
            .text_xs()
            .text_color(rgb(palette.sidebar_text))
            .child(name.to_string()),
    )
    .into_any_element()
}

/// 标签栏：自定义标签条，标签铺满整个高度并带圆角。
fn tab_strip(
    cx: &App,
    open_files: &[OpenFile],
    dirty: &[bool],
    active: Option<usize>,
    palette: Palette,
    callbacks: TabCallbacks,
) -> AnyElement {
    let TabCallbacks {
        activate: on_activate,
        close: on_close,
        menu: on_tab_menu,
    } = callbacks;
    let mut strip = h_flex()
        .h(px(ui_theme::TAB_BAR_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .items_stretch()
        .gap(px(2.0))
        .pr(px(6.0))
        .bg(rgb(palette.sidebar_bg));

    for (index, file) in open_files.iter().enumerate() {
        let is_active = active == Some(index);
        let is_dirty = dirty.get(index).copied().unwrap_or(false);
        let text_color = if is_active {
            palette.tab_active_text
        } else {
            palette.tab_inactive_text
        };

        let activate = on_activate.clone();
        let close = on_close.clone();
        let tab_menu = on_tab_menu.clone();

        strip = strip.child(
            h_flex()
                .id(("editor-tab", index))
                .h_full()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .pl(px(10.0))
                .pr(px(4.0))
                .rounded_t(px(6.0))
                .cursor(CursorStyle::PointingHand)
                .when(is_active, |this| this.bg(rgb(palette.window_bg)))
                .when(!is_active, |this| {
                    this.hover(move |style| style.bg(rgba(palette.hover)))
                })
                .on_click(move |_, window, cx| activate(index, window, cx))
                .on_mouse_up(MouseButton::Right, move |event: &MouseUpEvent, window, cx| {
                    cx.stop_propagation();
                    tab_menu(index, event.position, window, cx);
                })
                .child(
                    Icon::new(IconName::File)
                        .xsmall()
                        .text_color(rgb(text_color)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(text_color))
                        .child(file.name.clone()),
                )
                .when(is_dirty, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted_text))
                            .child("●"),
                    )
                })
                .child(
                    Button::new(("close-tab", index))
                        .icon(IconName::Close)
                        .xsmall()
                        .tab_stop(false)
                        .custom(
                            ButtonCustomVariant::new(cx).foreground(Hsla::from(rgb(text_color))),
                        )
                        .ml(px(2.0))
                        .mr(px(2.0))
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            close(index, window, cx);
                        }),
                ),
        );
    }

    strip.into_any_element()
}
