//! 主窗口常用的输入、确认与未保存提示对话框。

use super::*;

/// 对话框里「保存」「不保存」按钮的回调。
pub(super) type DialogCallback = Rc<dyn Fn(&mut Window, &mut App)>;

/// 对话框打开后把键盘焦点交给输入框（首帧渲染之后再确认一次，避免焦点被对话框容器拿走）。
pub(super) fn focus_dialog_input(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| input.focus(window, cx));
    let field = input.clone();
    window.on_next_frame(move |window, cx| {
        field.update(cx, |input, cx| input.focus(window, cx));
    });
}

/// 单行输入对话框的文案与元素 id 前缀。
pub(super) struct InputDialogSpec {
    pub(super) id_prefix: &'static str,
    pub(super) title: &'static str,
    pub(super) description: Option<&'static str>,
    pub(super) confirm: &'static str,
}

/// 单行输入对话框：回车与「确定」都执行 `apply`，然后关闭对话框。
pub(super) fn show_input_dialog(
    window: &mut Window,
    cx: &mut App,
    spec: InputDialogSpec,
    input: Entity<InputState>,
    apply: DialogCallback,
) {
    let InputDialogSpec {
        id_prefix,
        title,
        description,
        confirm,
    } = spec;
    let dialog_input = input.clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        let field = dialog_input.clone();
        let on_click = apply.clone();
        let on_enter = apply.clone();
        // 焦点只用更亮的边框表示，不使用组件在边框外再画一圈的样式。
        let palette = Palette::for_theme(cx);
        let focused = field.read(cx).has_selection_focus(window, cx);
        let mut field_input = Input::new(&field).xsmall().focus_ring(false);
        if focused {
            field_input = field_input.border_color(rgb(palette.sidebar_text));
        }
        let mut body = v_flex()
            .gap_2()
            .on_action(move |_: &Enter, window, cx| on_enter(window, cx));
        if let Some(description) = description {
            body = body.child(div().text_xs().child(description));
        }
        dialog
            .title(div().text_sm().child(title))
            .child(body.child(field_input))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new(format!("{id_prefix}-cancel"))
                            .small()
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new(format!("{id_prefix}-apply"))
                            .small()
                            .label(confirm)
                            .on_click(move |_, window, cx| on_click(window, cx)),
                    ),
            )
    });

    focus_dialog_input(&input, window, cx);
}

/// 通用确认对话框：取消 / 确认。
pub(super) fn show_confirm_dialog(
    window: &mut Window,
    cx: &mut App,
    title: &'static str,
    description: String,
    confirm: &'static str,
    on_confirm: DialogCallback,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let on_confirm = on_confirm.clone();
        let description = description.clone();
        dialog
            .title(div().text_sm().child(title))
            .child(div().text_xs().child(description))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("confirm-cancel")
                            .small()
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("confirm-apply")
                            .small()
                            .label(confirm)
                            .on_click(move |_, window, cx| {
                                on_confirm(window, cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

/// 有未保存修改时的三选一对话框：取消 / 不保存 / 保存并关闭。
pub(super) fn show_unsaved_dialog(
    window: &mut Window,
    cx: &mut App,
    description: String,
    on_save: DialogCallback,
    on_discard: DialogCallback,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let on_save = on_save.clone();
        let on_discard = on_discard.clone();
        let description = description.clone();
        dialog
            .title(div().text_sm().child("未保存的修改"))
            .child(div().text_xs().child(description))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("unsaved-cancel")
                            .small()
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("unsaved-discard")
                            .small()
                            .label("不保存")
                            .on_click(move |_, window, cx| {
                                on_discard(window, cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("unsaved-save")
                            .small()
                            .label("保存并关闭")
                            .on_click(move |_, window, cx| {
                                on_save(window, cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}
