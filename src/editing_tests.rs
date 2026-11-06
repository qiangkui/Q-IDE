//! 编辑器文本命令（移动行、切换注释）的纯逻辑测试。

use crate::editing::{comment_token, move_lines, toggle_comment, TextEdit};

fn moved(text: &str, selection: std::ops::Range<usize>, up: bool) -> TextEdit {
    move_lines(text, selection, up).expect("应当可以移动")
}

#[test]
fn moving_a_single_line_up_swaps_it_with_the_previous_line() {
    let text = "a\nb\nc\n";
    // 光标在 "b" 的第 0 列：偏移 2。
    let edit = moved(text, 2..2, true);
    assert_eq!(edit.text, "b\na\nc\n");
    assert_eq!(edit.selection, 0..0, "选区跟着这一行往上走");
}

#[test]
fn moving_a_single_line_down_swaps_it_with_the_next_line() {
    let edit = moved("a\nb\nc\n", 2..2, false);
    assert_eq!(edit.text, "a\nc\nb\n");
    assert_eq!(edit.selection, 4..4);
}

#[test]
fn moving_the_last_line_without_trailing_newline_keeps_lines_apart() {
    let edit = moved("a\nb\nc", 4..4, true);
    assert_eq!(edit.text, "a\nc\nb");
    assert_eq!(edit.selection, 2..2);

    let edit = moved("a\nb\nc", 2..2, false);
    assert_eq!(edit.text, "a\nc\nb");
    assert_eq!(edit.selection, 4..4);
}

#[test]
fn first_line_cannot_move_up_and_last_line_cannot_move_down() {
    assert_eq!(move_lines("a\nb\n", 0..0, true), None);
    assert_eq!(move_lines("a\nb\n", 2..2, false), None);
    assert_eq!(move_lines("only line", 0..0, false), None);
    assert_eq!(move_lines("only line", 0..0, true), None);
}

#[test]
fn a_multi_line_selection_moves_as_a_block() {
    let text = "a\nb\nc\nd\n";
    // 选中 "b\nc"，偏移 2..5。
    let edit = moved(text, 2..5, true);
    assert_eq!(edit.text, "b\nc\na\nd\n");
    assert_eq!(edit.selection, 0..3);

    let edit = moved(text, 2..5, false);
    assert_eq!(edit.text, "a\nd\nb\nc\n");
    assert_eq!(edit.selection, 4..7);
}

#[test]
fn selection_ending_at_a_line_start_does_not_include_that_line() {
    // 选中 "b\n"（2..4），终点正好是下一行行首：只移动 b。
    let edit = moved("a\nb\nc\n", 2..4, false);
    assert_eq!(edit.text, "a\nc\nb\n");
}

#[test]
fn adding_comments_uses_indent_and_skips_empty_lines() {
    let text = "Sub Main()\n    MsgBox \"hi\"\n\nEnd Sub\n";
    let edit = toggle_comment(text, 0..text.len(), "'").expect("应当加注释");
    assert_eq!(edit.text, "' Sub Main()\n    ' MsgBox \"hi\"\n\n' End Sub\n");
    // 空行不变，选区仍然覆盖同样的范围。
    assert_eq!(&edit.text[edit.selection.clone()], edit.text.as_str());
}

#[test]
fn toggling_off_removes_the_marker_and_one_space() {
    let text = "' Sub Main()\n'MsgBox\n";
    let edit = toggle_comment(text, 0..text.len(), "'").expect("应当去掉注释");
    assert_eq!(edit.text, "Sub Main()\nMsgBox\n");
    assert_eq!(edit.selection, 0..18);
}

#[test]
fn mixed_comments_get_all_lines_commented() {
    let text = "' a\nb\n";
    let edit = toggle_comment(text, 0..text.len(), "'").expect("应当补注释");
    // 与编辑器一致：只要不是整段都注释，就把每一行都加上注释。
    assert_eq!(edit.text, "' ' a\n' b\n");
}

#[test]
fn comment_column_is_tracked_through_the_edit() {
    // 光标在第二个 "b" 的后面（偏移 4），加注释后落在同一行的同一列。
    let text = "a\nbb\n";
    let edit = toggle_comment(text, 4..4, "'").expect("应当加注释");
    assert_eq!(edit.text, "a\n' bb\n");
    assert_eq!(edit.selection, 6..6);
}

#[test]
fn comment_token_matches_language() {
    assert_eq!(comment_token("vb"), Some("'"));
    assert_eq!(comment_token("rust"), Some("//"));
    assert_eq!(comment_token("json"), Some("//"));
    assert_eq!(comment_token("text"), None);
}
