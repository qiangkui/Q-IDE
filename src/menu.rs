//! 顶部菜单栏与各个下拉菜单。

use std::rc::Rc;

use gpui_kit::base::PopoverState;
use gpui_kit::component::button::*;
use gpui_kit::component::input::{Copy, Cut, Indent, Outdent, Paste, Redo, SelectAll, Undo};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::CodeWindow;
use crate::settings::{AppSettings, SettingsStore, ThemeChoice};
use crate::theme::Palette;

/// 菜单里的一个命令。
type MenuAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// 菜单栏需要的界面状态快照。
#[derive(Clone, Copy)]
pub struct MenuState {
    pub show_explorer: bool,
    pub show_outline: bool,
    pub show_search: bool,
    pub show_output: bool,
    pub show_status_bar: bool,
    pub has_files: bool,
    pub has_active: bool,
    pub dark_theme: bool,
}

/// 渲染整个菜单栏。
pub fn render(
    cx: &App,
    palette: Palette,
    handle: WeakEntity<CodeWindow>,
    state: MenuState,
    recent: &[String],
) -> AnyElement {
    h_flex()
        .id("menu-bar")
        .h_full()
        .items_center()
        .gap(px(1.0))
        .child(file_menu(cx, palette, handle.clone(), state, recent))
        .child(edit_menu(cx, palette, handle.clone(), state))
        .child(select_menu(cx, palette, handle.clone(), state))
        .child(view_menu(cx, palette, state))
        .child(go_menu(cx, palette, handle.clone()))
        .child(run_menu(cx, palette, handle.clone()))
        .child(help_menu(cx, palette, handle))
        .into_any_element()
}

/// 弹出菜单的内容面板：侧边栏同色的背景 + 最细（1px）的灰色边框。
fn menu_panel(
    palette: Palette,
    state: Entity<PopoverState>,
    rows: Vec<AnyElement>,
) -> impl IntoElement {
    v_flex()
        .id("menu-panel")
        .min_w(px(170.0))
        .py(px(4.0))
        .px(px(4.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgb(palette.sidebar_bg))
        .on_mouse_down_out(move |_, window, cx| {
            state.update(cx, |state, cx| state.dismiss(window, cx));
        })
        .children(rows)
}

/// 弹出菜单里的一行命令。
fn menu_row(
    palette: Palette,
    state: &Entity<PopoverState>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    disabled: bool,
    action: Option<MenuAction>,
) -> AnyElement {
    let state = state.clone();
    let label = label.into();

    let mut row = h_flex()
        .id(id)
        .w_full()
        .h(px(22.0))
        .items_center()
        .gap_2()
        .px(px(8.0))
        .rounded(px(3.0))
        .text_xs()
        .text_color(rgb(if disabled {
            palette.muted_text
        } else {
            palette.menu_text
        }))
            .child(div().w(px(14.0)).flex_shrink_0().when(checked, |this| {
                this.child(
                    Icon::new(IconName::Check)
                        .xsmall()
                        .text_color(rgb(palette.menu_text)),
                )
            }))
            .child(div().min_w_0().truncate().child(label));

    if !disabled {
        row = row
            .cursor(CursorStyle::PointingHand)
            .hover(move |style| style.bg(rgba(palette.hover)))
            .on_click(move |_, window, cx| {
                state.update(cx, |state, cx| state.dismiss(window, cx));
                if let Some(action) = &action {
                    action(window, cx);
                }
            });
    }

    row.into_any_element()
}

fn menu_separator(palette: Palette) -> AnyElement {
    div()
        .h(px(1.0))
        .my(px(4.0))
        .mx(px(4.0))
        .bg(rgba(palette.border))
        .into_any_element()
}

fn menu_button(cx: &App, palette: Palette, id: &'static str, label: &'static str) -> Button {
    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .foreground(Hsla::from(rgb(palette.menu_text)))
                .hover(Hsla::from(rgba(palette.hover))),
        )
        .xsmall()
        .compact()
        .label(label)
}

/// 一个下拉菜单：按钮 + 自定义样式的弹出面板。
fn menu_popover(
    cx: &App,
    palette: Palette,
    id: &'static str,
    label: &'static str,
    build: impl Fn(&Entity<PopoverState>) -> Vec<AnyElement> + 'static,
) -> impl IntoElement {
    Popover::new(id)
        .anchor(Anchor::TopLeft)
        .offset(px(2.0))
        .appearance(false)
        .trigger(menu_button(cx, palette, id, label))
        .content(move |_, _, cx| {
            let state = cx.entity();
            menu_panel(palette, state.clone(), build(&state))
        })
}

/// 把「调用主窗口上的方法」包装成菜单命令；窗口已关闭时什么都不做。
fn app_action(
    handle: &WeakEntity<CodeWindow>,
    run: impl Fn(&mut CodeWindow, &mut Window, &mut Context<CodeWindow>) + 'static,
) -> MenuAction {
    let handle = handle.clone();
    Rc::new(move |window, cx| {
        let _ = handle.update(cx, |this, cx| run(this, window, cx));
    })
}

/// 把编辑器的动作（撤销、复制……）包装成菜单命令。
fn editor_action(handle: &WeakEntity<CodeWindow>, make: fn() -> Box<dyn Action>) -> MenuAction {
    app_action(handle, move |this, window, cx| {
        this.run_editor_action(make(), window, cx);
    })
}

/// 打开编辑器自带的查找 / 替换面板。
fn search_action(handle: &WeakEntity<CodeWindow>, replace: bool) -> MenuAction {
    app_action(handle, move |this, window, cx| {
        this.open_search(replace, window, cx);
    })
}

/// 修改设置里的某个开关。
fn toggle_setting(update: fn(&mut AppSettings)) -> MenuAction {
    Rc::new(move |_, cx| {
        SettingsStore::update(cx, update);
    })
}

/// “文件”菜单。
fn file_menu(
    cx: &App,
    palette: Palette,
    handle: WeakEntity<CodeWindow>,
    state: MenuState,
    recent: &[String],
) -> impl IntoElement {
    let close_one = app_action(&handle, |this, window, cx| {
        if let Some(index) = this.active_file_index() {
            this.close_file(index, window, cx);
        }
    });
    let close_all = app_action(&handle, |this, window, cx| this.close_all_files(window, cx));
    let reopen_closed = app_action(&handle, |this, window, cx| {
        this.reopen_closed_file(window, cx)
    });
    let settings: MenuAction = Rc::new(|_, cx| crate::settings::open_settings_window(cx));
    let quit: MenuAction = {
        let handle = handle.clone();
        Rc::new(move |window, cx| {
            if handle
                .update(cx, |this, cx| this.request_quit(window, cx))
                .is_err()
            {
                cx.quit();
            }
        })
    };
    let new_file = app_action(&handle, |this, window, cx| this.new_file(window, cx));
    let open_files = app_action(&handle, |this, window, cx| this.open_files_dialog(window, cx));
    let save_file = app_action(&handle, |this, _, cx| this.save_file(cx));
    let save_file_as = app_action(&handle, |this, window, cx| this.save_file_as(window, cx));
    let save_all = app_action(&handle, |this, _, cx| this.save_all_files(cx));
    let rename_file = app_action(&handle, |this, window, cx| {
        if let Some(index) = this.active_file_index() {
            this.rename_file_dialog(index, window, cx);
        }
    });
    let delete_file = app_action(&handle, |this, window, cx| {
        if let Some(index) = this.active_file_index() {
            this.delete_file_dialog(index, window, cx);
        }
    });
    // 最近打开的文件：最多列 5 个，点一下重新打开。
    let recent_rows: Vec<(String, MenuAction)> = recent
        .iter()
        .take(RECENT_IN_MENU)
        .map(|path| {
            let opened = path.clone();
            let action =
                app_action(&handle, move |this, window, cx| this.open_file(&opened, window, cx));
            (short_path(path), action)
        })
        .collect();
    let clear_recent = app_action(&handle, |this, _, cx| this.clear_recent_files(cx));

    menu_popover(cx, palette, "menu-file", "文件(F)", move |state_menu| {
        let mut rows = vec![
            menu_row(
                palette,
                state_menu,
                "file-new",
                "新建文件",
                false,
                false,
                Some(new_file.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "file-open",
                "打开文件…",
                false,
                false,
                Some(open_files.clone()),
            ),
        ];

        if !recent_rows.is_empty() {
            rows.push(menu_separator(palette));
            for (index, (label, action)) in recent_rows.iter().enumerate() {
                rows.push(menu_row(
                    palette,
                    state_menu,
                    format!("file-recent-{index}"),
                    label.clone(),
                    false,
                    false,
                    Some(action.clone()),
                ));
            }
            rows.push(menu_row(
                palette,
                state_menu,
                "file-recent-clear",
                "清除最近打开",
                false,
                false,
                Some(clear_recent.clone()),
            ));
        }

        rows.push(menu_separator(palette));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-save",
            "保存",
            false,
            !state.has_active,
            Some(save_file.clone()),
        ));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-save-as",
            "另存为…",
            false,
            !state.has_active,
            Some(save_file_as.clone()),
        ));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-save-all",
            "全部保存（Ctrl+Alt+S）",
            false,
            !state.has_files,
            Some(save_all.clone()),
        ));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-rename",
            "重命名当前文件…",
            false,
            !state.has_active,
            Some(rename_file.clone()),
        ));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-delete",
            "删除当前文件…",
            false,
            !state.has_active,
            Some(delete_file.clone()),
        ));
        rows.push(menu_separator(palette));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-close",
            "关闭编辑器",
            false,
            !state.has_active,
            Some(close_one.clone()),
        ));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-close-all",
            "关闭所有编辑器",
            false,
            !state.has_files,
            Some(close_all.clone()),
        ));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-reopen",
            "重新打开关闭的标签（Ctrl+Shift+T）",
            true,
            false,
            Some(reopen_closed.clone()),
        ));
        rows.push(menu_separator(palette));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-settings",
            "设置…",
            false,
            false,
            Some(settings.clone()),
        ));
        rows.push(menu_separator(palette));
        rows.push(menu_row(
            palette,
            state_menu,
            "file-quit",
            "退出",
            false,
            false,
            Some(quit.clone()),
        ));
        rows
    })
}

/// 文件菜单里最多列几个最近打开的文件。
const RECENT_IN_MENU: usize = 5;

/// 最近打开列表里显示的短路径：`父目录\文件名`。
fn short_path(path: &str) -> String {
    let mut parts = path.rsplit(['\\', '/']);
    let name = parts.next().unwrap_or(path);
    match parts.next() {
        Some(parent) => format!("{parent}\\{name}"),
        None => name.to_string(),
    }
}

/// “编辑”菜单：把操作转交给编辑器处理。
fn edit_menu(
    cx: &App,
    palette: Palette,
    handle: WeakEntity<CodeWindow>,
    state: MenuState,
) -> impl IntoElement {
    let undo = editor_action(&handle, || Box::new(Undo));
    let redo = editor_action(&handle, || Box::new(Redo));
    let cut = editor_action(&handle, || Box::new(Cut));
    let copy = editor_action(&handle, || Box::new(Copy));
    let paste = editor_action(&handle, || Box::new(Paste));
    let find = search_action(&handle, false);
    let replace = search_action(&handle, true);
    let move_up = app_action(&handle, |this, window, cx| {
        this.move_active_lines(true, window, cx)
    });
    let move_down = app_action(&handle, |this, window, cx| {
        this.move_active_lines(false, window, cx)
    });
    let toggle_comment = app_action(&handle, |this, window, cx| {
        this.toggle_active_comment(window, cx)
    });
    let next_tab = app_action(&handle, |this, window, cx| this.cycle_tab(true, window, cx));
    let previous_tab = app_action(&handle, |this, window, cx| this.cycle_tab(false, window, cx));
    // 缩进走编辑器内置动作（`Tab` / `Shift+Tab` 的默认绑定）。
    let indent = editor_action(&handle, || Box::new(Indent));
    let outdent = editor_action(&handle, || Box::new(Outdent));
    let disabled = !state.has_active;

    menu_popover(cx, palette, "menu-edit", "编辑(E)", move |state_menu| {
        vec![
            menu_row(
                palette,
                state_menu,
                "edit-undo",
                "撤销",
                false,
                disabled,
                Some(undo.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "edit-redo",
                "重做",
                false,
                disabled,
                Some(redo.clone()),
            ),
            menu_separator(palette),
            menu_row(
                palette,
                state_menu,
                "edit-cut",
                "剪切",
                false,
                disabled,
                Some(cut.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "edit-copy",
                "复制",
                false,
                disabled,
                Some(copy.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "edit-paste",
                "粘贴",
                false,
                disabled,
                Some(paste.clone()),
            ),
            menu_separator(palette),
            menu_row(
                palette,
                state_menu,
                "edit-move-up",
                "上移行（Alt+↑）",
                false,
                disabled,
                Some(move_up.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "edit-move-down",
                "下移行（Alt+↓）",
                false,
                disabled,
                Some(move_down.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "edit-toggle-comment",
                "切换注释（Ctrl+/）",
                false,
                disabled,
                Some(toggle_comment.clone()),
            ),
            menu_separator(palette),
            menu_row(
                palette,
                state_menu,
                "edit-indent",
                "增加缩进（Tab）",
                false,
                disabled,
                Some(indent.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "edit-outdent",
                "减少缩进（Shift+Tab）",
                false,
                disabled,
                Some(outdent.clone()),
            ),
            menu_separator(palette),
            menu_row(
                palette,
                state_menu,
                "edit-next-tab",
                "下一个标签（Ctrl+Tab）",
                false,
                disabled,
                Some(next_tab.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "edit-previous-tab",
                "上一个标签（Ctrl+Shift+Tab）",
                false,
                disabled,
                Some(previous_tab.clone()),
            ),
            menu_separator(palette),
            menu_row(
                palette,
                state_menu,
                "edit-find",
                "查找",
                false,
                disabled,
                Some(find.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "edit-replace",
                "替换",
                false,
                disabled,
                Some(replace.clone()),
            ),
        ]
    })
}

/// “选择”菜单。
fn select_menu(
    cx: &App,
    palette: Palette,
    handle: WeakEntity<CodeWindow>,
    state: MenuState,
) -> impl IntoElement {
    let select_all = editor_action(&handle, || Box::new(SelectAll));
    let disabled = !state.has_active;

    menu_popover(cx, palette, "menu-select", "选择(S)", move |state_menu| {
        vec![menu_row(
            palette,
            state_menu,
            "select-all",
            "全选",
            false,
            disabled,
            Some(select_all.clone()),
        )]
    })
}

/// “查看”菜单：切换面板显示、主题与全屏。
fn view_menu(cx: &App, palette: Palette, state: MenuState) -> impl IntoElement {
    let toggle_explorer = toggle_setting(|settings| {
        settings.appearance.toggle_explorer();
    });
    let toggle_output = toggle_setting(|settings| {
        settings.appearance.show_output = !settings.appearance.show_output;
    });
    let toggle_outline = toggle_setting(|settings| {
        settings.appearance.toggle_outline();
    });
    let toggle_search_panel = toggle_setting(|settings| {
        settings.appearance.toggle_search();
    });
    let toggle_status_bar = toggle_setting(|settings| {
        settings.appearance.show_status_bar = !settings.appearance.show_status_bar;
    });
    let toggle_theme = toggle_setting(|settings| {
        settings.appearance.theme = match settings.appearance.theme {
            ThemeChoice::Dark => ThemeChoice::Light,
            ThemeChoice::Light => ThemeChoice::Dark,
        };
    });
    let fullscreen: MenuAction = Rc::new(|window, _| window.toggle_fullscreen());
    let theme_label = if state.dark_theme {
        "切换到浅色主题"
    } else {
        "切换到深色主题"
    };

    menu_popover(cx, palette, "menu-view", "查看(V)", move |state_menu| {
        vec![
            menu_row(
                palette,
                state_menu,
                "view-explorer",
                "资源管理器",
                state.show_explorer,
                false,
                Some(toggle_explorer.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "view-outline",
                "代码大纲",
                state.show_outline,
                false,
                Some(toggle_outline.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "view-search",
                "搜索",
                state.show_search,
                false,
                Some(toggle_search_panel.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "view-output",
                "输出",
                state.show_output,
                false,
                Some(toggle_output.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "view-status-bar",
                "状态栏",
                state.show_status_bar,
                false,
                Some(toggle_status_bar.clone()),
            ),
            menu_separator(palette),
            menu_row(
                palette,
                state_menu,
                "view-theme",
                theme_label,
                false,
                false,
                Some(toggle_theme.clone()),
            ),
            menu_separator(palette),
            menu_row(
                palette,
                state_menu,
                "view-fullscreen",
                "全屏",
                false,
                false,
                Some(fullscreen.clone()),
            ),
        ]
    })
}

/// “运行”菜单：这个 IDE 只运行 VB 脚本，其它语言的文件会提示无法运行。
fn run_menu(cx: &App, palette: Palette, handle: WeakEntity<CodeWindow>) -> impl IntoElement {
    let run = app_action(&handle, |this, _, cx| this.run_script(cx));
    let stop_script = app_action(&handle, |this, _, cx| this.stop_script(cx));
    let check_syntax = app_action(&handle, |this, _, cx| this.check_syntax(cx));
    let run_arguments = app_action(&handle, |this, window, cx| this.edit_run_arguments(window, cx));
    let debug_run = app_action(&handle, |this, _, cx| this.start_debug(cx));
    let debug_pause = app_action(&handle, |this, _, cx| this.debug_pause(cx));
    let debug_resume = app_action(&handle, |this, _, cx| this.debug_resume(cx));
    let debug_step = app_action(&handle, |this, _, cx| this.debug_step(cx));
    let toggle_breakpoint = app_action(&handle, |this, _, cx| this.toggle_breakpoint(cx));
    let set_breakpoint_condition =
        app_action(&handle, |this, window, cx| {
            this.set_breakpoint_condition_dialog(window, cx)
        });
    let run_to_cursor = app_action(&handle, |this, _, cx| this.run_to_cursor(cx));
    let clear_breakpoints = app_action(&handle, |this, _, cx| this.clear_all_breakpoints(cx));
    let add_watch = app_action(&handle, |this, window, cx| this.add_watch_dialog(window, cx));
    let clear_watches = app_action(&handle, |this, _, cx| this.clear_watches(cx));

    menu_popover(cx, palette, "menu-run", "运行(R)", move |state_menu| {
        vec![
            // 正常运行相关的命令。
            menu_row(
                palette,
                state_menu,
                "run-vb-script",
                "运行",
                false,
                false,
                Some(run.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "check-vb-syntax",
                "检查语法",
                false,
                false,
                Some(check_syntax.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "stop-vb-script",
                "停止脚本（Shift+F5）",
                false,
                false,
                Some(stop_script.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "run-arguments",
                "运行参数…",
                false,
                false,
                Some(run_arguments.clone()),
            ),
            menu_separator(palette),
            // 调试相关的命令。
            menu_row(
                palette,
                state_menu,
                "debug-vb-script",
                "调试运行（Ctrl+F5）",
                false,
                false,
                Some(debug_run.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "debug-pause-vb-script",
                "暂停",
                false,
                false,
                Some(debug_pause.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "debug-resume-vb-script",
                "继续（F11）",
                false,
                false,
                Some(debug_resume.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "debug-step-vb-script",
                "单步（F10）",
                false,
                false,
                Some(debug_step.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "toggle-vb-breakpoint",
                "切换断点（F9）",
                false,
                false,
                Some(toggle_breakpoint.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "set-vb-breakpoint-condition",
                "设置条件断点…",
                false,
                false,
                Some(set_breakpoint_condition.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "run-to-cursor",
                "运行到光标处（Ctrl+F10）",
                false,
                false,
                Some(run_to_cursor.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "clear-breakpoints",
                "清除所有断点（Ctrl+Shift+F9）",
                false,
                false,
                Some(clear_breakpoints.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "add-watch",
                "添加监视…",
                false,
                false,
                Some(add_watch.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "clear-watches",
                "清除所有监视",
                false,
                false,
                Some(clear_watches.clone()),
            ),
        ]
    })
}

/// “转到”菜单：转到定义 / 查找引用 / 转到行 / 转到文件 / 命令面板。
fn go_menu(cx: &App, palette: Palette, handle: WeakEntity<CodeWindow>) -> impl IntoElement {
    let goto_definition =
        app_action(&handle, |this, window, cx| this.go_to_definition(window, cx));
    let find_references = app_action(&handle, |this, _, cx| this.find_references(cx));
    let goto_line = app_action(&handle, |this, window, cx| this.goto_dialog(window, cx));
    let goto_file = app_action(&handle, |this, window, cx| this.goto_file_dialog(window, cx));
    let show_commands = app_action(&handle, |this, window, cx| {
        this.command_palette_dialog(window, cx)
    });

    menu_popover(cx, palette, "menu-go", "转到(G)", move |state_menu| {
        vec![
            menu_row(
                palette,
                state_menu,
                "goto-definition",
                "转到定义（F12）",
                false,
                false,
                Some(goto_definition.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "find-references",
                "查找引用（Shift+F12）",
                false,
                false,
                Some(find_references.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "goto-line",
                "转到行 / 符号…（Ctrl+G）",
                false,
                false,
                Some(goto_line.clone()),
            ),
            menu_separator(palette),
            menu_row(
                palette,
                state_menu,
                "goto-file",
                "转到文件…（Ctrl+P）",
                false,
                false,
                Some(goto_file.clone()),
            ),
            menu_row(
                palette,
                state_menu,
                "show-commands",
                "命令面板…（Ctrl+Shift+P）",
                true,
                false,
                Some(show_commands.clone()),
            ),
        ]
    })
}

/// “帮助”菜单。
fn help_menu(cx: &App, palette: Palette, handle: WeakEntity<CodeWindow>) -> impl IntoElement {
    let about = app_action(&handle, |this, _, cx| this.show_about(cx));

    menu_popover(cx, palette, "menu-help", "帮助(H)", move |state_menu| {
        vec![menu_row(
            palette,
            state_menu,
            "help-about",
            "关于 Q-IDE",
            false,
            false,
            Some(about.clone()),
        )]
    })
}
