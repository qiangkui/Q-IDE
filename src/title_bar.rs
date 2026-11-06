//! 自绘标题栏与窗口控制按钮。

use gpui_kit::component::*;
use gpui_kit::*;

use crate::theme::{self as ui_theme, Palette};

pub const APP_TITLE: &str = "Q-IDE";

/// 主窗口标题栏：左侧菜单栏，中间是窗口拖动区域，右侧是最小化/最大化/关闭按钮。
/// 控制按钮贴住窗口上沿；拖动区域顶部留出一点空白，让出系统自带的上边缘缩放热区。
pub fn render(palette: Palette, maximized: bool, menu_bar: impl IntoElement) -> AnyElement {
    h_flex()
        .h(px(ui_theme::TITLE_BAR_HEIGHT))
        .w_full()
        .pl(px(6.0))
        .bg(rgb(palette.window_bg))
        .items_start()
        .child(
            h_flex()
                .h(px(ui_theme::TITLE_BAR_HEIGHT))
                .flex_shrink_0()
                .items_center()
                .child(menu_bar),
        )
        .child(
            h_flex()
                .id("title-bar-drag")
                .window_control_area(WindowControlArea::Drag)
                .flex_1()
                .h(px(ui_theme::TITLE_BAR_HEIGHT - ui_theme::TITLE_BAR_INSET))
                .mt(px(ui_theme::TITLE_BAR_INSET))
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.title_text))
                        .child(APP_TITLE),
                ),
        )
        .child(
            h_flex()
                .h(px(ui_theme::TITLE_BAR_HEIGHT))
                .flex_shrink_0()
                .child(window_button(
                    "window-minimize",
                    WindowControlArea::Min,
                    IconName::WindowMinimize,
                    palette.title_text,
                    palette.window_button_hover,
                ))
                .child(window_button(
                    "window-maximize",
                    WindowControlArea::Max,
                    if maximized {
                        IconName::WindowRestore
                    } else {
                        IconName::WindowMaximize
                    },
                    palette.title_text,
                    palette.window_button_hover,
                ))
                .child(window_button(
                    "window-close",
                    WindowControlArea::Close,
                    IconName::WindowClose,
                    palette.title_text,
                    palette.close_button_hover,
                )),
        )
        .into_any_element()
}

/// 对话框/工具窗口标题栏：左侧标题，中间拖动区域，右侧关闭按钮。
/// `background` 由调用方给出，使标题栏与窗口内容同色。
pub fn dialog_title_bar(
    palette: Palette,
    background: u32,
    id: &'static str,
    title: &'static str,
) -> AnyElement {
    h_flex()
        .h(px(ui_theme::TITLE_BAR_HEIGHT))
        .w_full()
        .pl(px(14.0))
        .bg(rgb(background))
        .items_start()
        .child(
            h_flex()
                .h(px(ui_theme::TITLE_BAR_HEIGHT))
                .flex_shrink_0()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.title_text))
                        .child(title),
                ),
        )
        .child(
            h_flex()
                .id(id)
                .window_control_area(WindowControlArea::Drag)
                .flex_1()
                .h(px(ui_theme::TITLE_BAR_HEIGHT - ui_theme::TITLE_BAR_INSET))
                .mt(px(ui_theme::TITLE_BAR_INSET)),
        )
        .child(
            h_flex()
                .h(px(ui_theme::TITLE_BAR_HEIGHT))
                .flex_shrink_0()
                .child(window_button(
                    "dialog-close",
                    WindowControlArea::Close,
                    IconName::WindowClose,
                    palette.title_text,
                    palette.close_button_hover,
                )),
        )
        .into_any_element()
}

fn window_button(
    id: &'static str,
    area: WindowControlArea,
    icon: IconName,
    foreground: u32,
    hover_background: u32,
) -> impl IntoElement {
    div()
        .id(id)
        .window_control_area(area)
        .flex()
        .items_center()
        .justify_center()
        .w(px(ui_theme::WINDOW_BUTTON_WIDTH))
        .h_full()
        .flex_shrink_0()
        .hover(move |style| style.bg(rgb(hover_background)))
        .child(Icon::new(icon).small().text_color(rgb(foreground)))
}
