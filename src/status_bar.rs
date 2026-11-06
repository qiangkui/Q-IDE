//! 状态栏：显示当前文件、语法分析器与统计信息，并提供常用入口。

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::theme::{self as ui_theme, Palette};

/// 状态栏要展示的信息。
pub struct StatusInfo<'a> {
    pub file_name: Option<&'a str>,
    /// 当前生效的语法分析器，例如 `Rust` / `纯文本`。
    pub language: &'a str,
    /// 当前文件的编码，例如 `UTF-8` / `GBK`；没有打开文件时为 `None`。
    pub encoding: Option<&'a str>,
    pub line_count: usize,
    pub char_count: usize,
    /// 光标位置（1 基行、列）；没有打开文件时为 `None`。
    pub cursor: Option<(usize, usize)>,
    /// 当前文本里的语法错误数量（由实时语法检查得出）。
    pub errors: usize,
    pub output_lines: usize,
    /// 调试会话的状态文字（`调试：已暂停 · 第 12 行`）；没有会话时为 `None`。
    pub debug_state: Option<&'a str>,
}

pub fn render(
    palette: Palette,
    info: StatusInfo<'_>,
    on_toggle_output: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let file_name = info.file_name.unwrap_or("未打开文件").to_string();

    let mut left = h_flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.sidebar_text))
                .child(file_name),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(format!("语法：{}", info.language)),
        );

    if let Some(encoding) = info.encoding {
        left = left.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(format!("编码：{encoding}")),
        );
    }

    if info.file_name.is_some() {
        left = left.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(format!("{} 行 · {} 字符", info.line_count, info.char_count)),
        );
    }

    if let Some((line, column)) = info.cursor {
        left = left.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(format!("行 {line}，列 {column}")),
        );
    }

    if let Some(state) = info.debug_state {
        left = left.child(
            div()
                .text_xs()
                .text_color(rgb(palette.file))
                .min_w_0()
                .truncate()
                .child(state.to_string()),
        );
    }

    if info.errors > 0 {
        left = left.child(
            div()
                .text_xs()
                .text_color(rgb(palette.error))
                .child(format!("{} 个错误", info.errors)),
        );
    }

    h_flex()
        .h(px(ui_theme::STATUS_BAR_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .px(px(12.0))
        .bg(rgb(palette.sidebar_bg))
        .border_t_1()
        .border_color(rgba(palette.border))
        .child(left)
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .child(
                    Button::new("status-output")
                        .ghost()
                        .xsmall()
                        .icon(IconName::SquareTerminal)
                        .label(format!("输出 {}", info.output_lines))
                        .on_click(on_toggle_output),
                ),
        )
        .into_any_element()
}
