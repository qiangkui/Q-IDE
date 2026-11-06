//! 快捷键绑定测试：确认主窗口的查找/替换切换能覆盖编辑器内置动作。

use gpui_kit::component::dialog::Cancel;
use gpui_kit::component::input::{Replace, Search};
use gpui_kit::{KeyBinding, KeyContext, Keymap, Keystroke};

use crate::app::{GoToLine, ToggleReplace, ToggleSearch};

/// 按 `init` 里的顺序注册绑定后，某个按键在指定上下文里命中哪个动作。
fn first_action(keystroke: &'static str, contexts: &[&str]) -> Option<String> {
    let keymap = Keymap::new(vec![
        // 编辑器内置绑定先注册，主窗口的覆盖绑定后注册。
        KeyBinding::new("ctrl-f", Search, Some("Input")),
        KeyBinding::new("ctrl-h", Replace, Some("Input")),
        KeyBinding::new("ctrl-f", ToggleSearch, Some("Input && mode == editor")),
        KeyBinding::new("ctrl-h", ToggleReplace, Some("Input && mode == editor")),
        KeyBinding::new("ctrl-f", ToggleSearch, Some("SearchPanel > Input")),
        KeyBinding::new("ctrl-h", ToggleReplace, Some("SearchPanel > Input")),
        KeyBinding::new("ctrl-g", GoToLine, None),
        KeyBinding::new("ctrl-g", Cancel, Some("Dialog")),
        KeyBinding::new("ctrl-g", Cancel, Some("Dialog > Input")),
    ]);
    let input = [Keystroke::parse(keystroke).expect("合法按键")];
    let contexts: Vec<KeyContext> = contexts
        .iter()
        .map(|context| KeyContext::parse(context).expect("合法上下文"))
        .collect();
    let (bindings, _) = keymap.bindings_for_input(&input, &contexts);
    bindings
        .first()
        .map(|binding| binding.action().name().to_string())
}

#[test]
fn editor_search_shortcuts_toggle_the_panel() {
    assert_eq!(
        first_action("ctrl-f", &["Input mode=editor"]).as_deref(),
        Some("first::ToggleSearch")
    );
    assert_eq!(
        first_action("ctrl-h", &["Input mode=editor"]).as_deref(),
        Some("first::ToggleReplace")
    );
}

#[test]
fn search_shortcut_also_works_while_the_panel_has_focus() {
    assert_eq!(
        first_action("ctrl-f", &["Input mode=editor", "SearchPanel"]).as_deref(),
        Some("first::ToggleSearch"),
        "焦点在面板上时也要能再按一次关闭"
    );
    assert_eq!(
        first_action(
            "ctrl-f",
            &["Input mode=editor", "SearchPanel", "Input"]
        )
        .as_deref(),
        Some("first::ToggleSearch"),
        "焦点在查找输入框里时，必须覆盖内层输入的内置绑定"
    );
    assert_eq!(
        first_action(
            "ctrl-h",
            &["Input mode=editor", "SearchPanel", "Input"]
        )
        .as_deref(),
        Some("first::ToggleReplace")
    );
}

#[test]
fn goto_shortcut_closes_the_open_dialog() {
    let expected = first_action("ctrl-g", &["Dialog"]).expect("应当有绑定");
    assert!(
        expected.contains("Cancel"),
        "对话框里 Ctrl+G 应当等同取消：{expected}"
    );
    let expected = first_action("ctrl-g", &["Dialog", "Input"]).expect("应当有绑定");
    assert!(
        expected.contains("Cancel"),
        "焦点在对话框输入框里也应当关闭：{expected}"
    );
    assert_eq!(
        first_action("ctrl-g", &["Input mode=editor"]).as_deref(),
        Some("first::GoToLine"),
        "编辑区里 Ctrl+G 仍然打开对话框"
    );
}

#[test]
fn other_inputs_keep_the_builtin_search() {
    assert_eq!(
        first_action("ctrl-f", &["Input"]).as_deref(),
        Some("input::Search"),
        "设置窗口等普通输入框仍使用编辑器内置查找"
    );
}

#[test]
fn new_shortcuts_parse() {
    for stroke in ["alt-up", "alt-down", "ctrl-/", "ctrl-alt-s", "ctrl-shift-f"] {
        assert!(
            Keystroke::parse(stroke).is_ok(),
            "{stroke} 应当能被解析成合法按键"
        );
    }
}
