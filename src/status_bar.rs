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
    encoding_menu: Option<AnyElement>,
    output_open: bool,
    ai_open: bool,
    on_toggle_ai: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
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

    if let Some(encoding_menu) = encoding_menu {
        left = left.child(encoding_menu);
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
                // 输出面板（含终端页）的显示/隐藏开关：只画图标，打开时用选中底色标出来。
                .child({
                    let output_button = Button::new("status-output")
                        .ghost()
                        .xsmall()
                        .icon(IconName::SquareTerminal)
                        .tooltip(format!(
                            "输出 / 终端面板（{} 行，点击显示或隐藏）",
                            info.output_lines
                        ))
                        .selected(output_open)
                        .on_click(on_toggle_output);
                    highlight_if_open(output_button, output_open, palette)
                })
                // AI 面板开关放在最右边（贴着右边界），同样只画图标。
                .child({
                    let ai_button = Button::new("status-ai")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Bot)
                        .tooltip("AI 编程面板（打开 / 收起）")
                        .selected(ai_open)
                        .on_click(on_toggle_ai);
                    highlight_if_open(ai_button, ai_open, palette)
                }),
        )
        .into_any_element()
}

/// 面板开着时给状态栏上的开关加一层选中底色（关掉后恢复透明）。
fn highlight_if_open(button: Button, open: bool, palette: Palette) -> Button {
    if open {
        button.bg(rgba(palette.activity_bar_selected))
    } else {
        button
    }
}
