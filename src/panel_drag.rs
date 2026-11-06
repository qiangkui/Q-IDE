//! 自定义标题栏拖动面板时跟随光标的预览。

use gpui_kit::*;

use crate::theme::{self as ui_theme, Palette};

pub struct PanelDragPreview {
    title: SharedString,
}

impl PanelDragPreview {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
        }
    }
}

impl Render for PanelDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_theme(cx);
        div()
            .id("panel-drag-preview")
            .cursor_grab()
            .py_1()
            .px_3()
            .w_24()
            .overflow_hidden()
            .whitespace_nowrap()
            .border_1()
            .border_color(rgba(palette.border))
            .rounded(px(ui_theme::PANE_RADIUS))
            .text_color(rgb(palette.sidebar_text))
            .bg(rgb(palette.sidebar_bg))
            .opacity(0.75)
            .child(self.title.clone())
    }
}
