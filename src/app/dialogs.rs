//! 主窗口常用的输入、确认与未保存提示对话框。

use super::*;

/// 对话框里「保存」「不保存」按钮的回调。
pub(super) type DialogCallback = Rc<dyn Fn(&mut Window, &mut App)>;

/// 另存为选项确认后的回调：所选编码与换行符。
pub(super) type SaveAsConfirm =
    Rc<dyn Fn(encoding::FileEncoding, encoding::LineEnding, &mut Window, &mut App)>;

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

/// 快捷键一览：只读、可滚动；分区标题用次要色，按键用等宽字体。
pub(super) fn show_shortcuts_dialog(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, window, cx| {
        let palette = Palette::for_theme(cx);
        // 窗口很矮时让列表在对话框内部滚动，避免把标题与按钮顶到屏幕外。
        let body_height = (window.bounds().size.height.as_f32() - 190.0).clamp(160.0, 460.0);
        let mut body = v_flex()
            .id("shortcuts-scroll")
            .gap_4()
            .max_h(px(body_height))
            .overflow_y_scroll();
        for section in crate::shortcuts::SECTIONS {
            let mut group = v_flex().gap_1().child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.sidebar_text))
                    .child(section.title),
            );
            for entry in section.entries {
                group = group.child(
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(
                            div()
                                .w(px(168.0))
                                .flex_shrink_0()
                                .font_family(ui_theme::MONO_FONT)
                                .text_xs()
                                .child(entry.keys),
                        )
                        .child(div().text_xs().child(entry.action)),
                );
            }
            body = body.child(group);
        }
        dialog
            .w(px(560.0))
            .title(div().text_sm().child("快捷键一览"))
            .child(body)
            .footer(
                DialogFooter::new().child(
                    Button::new("shortcuts-close")
                        .small()
                        .label("关闭")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

/// 另存为对话框里的「编码 + 换行符」选择状态。
pub(super) struct SaveOptions {
    encoding: encoding::FileEncoding,
    line_ending: encoding::LineEnding,
}

impl SaveOptions {
    pub(super) fn new(encoding: encoding::FileEncoding, line_ending: encoding::LineEnding) -> Self {
        Self {
            encoding,
            line_ending,
        }
    }
}

impl Render for SaveOptions {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_theme(cx);
        let selected_encoding = self.encoding;
        let selected_ending = self.line_ending;

        let mut encoding_group = ToggleGroup::new("save-as-encoding").segmented();
        for kind in encoding::FileEncoding::ALL {
            let kind = *kind;
            encoding_group = encoding_group.child(
                Toggle::new(format!("save-as-encoding-{}", kind.label()))
                    .label(kind.label())
                    .checked(kind == selected_encoding)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.encoding = kind;
                        cx.notify();
                    })),
            );
        }

        let mut ending_group = ToggleGroup::new("save-as-ending").segmented();
        for ending in encoding::LineEnding::ALL {
            let ending = *ending;
            ending_group = ending_group.child(
                Toggle::new(format!("save-as-ending-{}", ending.label()))
                    .label(ending.label())
                    .checked(ending == selected_ending)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.line_ending = ending;
                        cx.notify();
                    })),
            );
        }

        v_flex()
            .gap_4()
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted_text))
                            .child("编码"),
                    )
                    .child(encoding_group),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted_text))
                            .child("换行符"),
                    )
                    .child(ending_group),
            )
    }
}

/// 另存为：选好路径后确认编码与换行符，再写盘。
pub(super) fn show_save_as_dialog(
    window: &mut Window,
    cx: &mut App,
    options: Entity<SaveOptions>,
    on_confirm: SaveAsConfirm,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let options = options.clone();
        let apply = on_confirm.clone();
        dialog
            .w(px(520.0))
            .title(div().text_sm().child("另存为"))
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        div().text_xs().child(
                            "选择写入使用的编码与换行符；换行符会同时应用到编辑器里的内容。",
                        ),
                    )
                    .child(options.clone()),
            )
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("save-as-cancel")
                            .small()
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("save-as-apply").small().label("保存").on_click(
                        move |_, window, cx| {
                            let (encoding, ending) = {
                                let options = options.read(cx);
                                (options.encoding, options.line_ending)
                            };
                            apply(encoding, ending, window, cx);
                            window.close_dialog(cx);
                        },
                    )),
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
