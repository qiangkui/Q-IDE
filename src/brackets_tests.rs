//! 括号配对查找的测试。

use crate::brackets::matching_pair;

fn pair(text: &str, cursor: usize) -> Option<(usize, usize)> {
    matching_pair(text, cursor)
}

#[test]
fn matches_a_pair_when_the_cursor_is_after_the_opening_bracket() {
    let text = "MsgBox (1 + 2)";
    // 光标在 `(` 之后（下标 8）。
    assert_eq!(pair(text, 8), Some((7, 13)));
    assert_eq!(&text[7..8], "(");
    assert_eq!(&text[13..14], ")");
}

#[test]
fn matches_a_pair_when_the_cursor_is_before_the_closing_bracket() {
    let text = "call(a, b)";
    // 光标在 `)` 之前（下标 9）。
    assert_eq!(pair(text, 9), Some((4, 9)));
}

#[test]
fn handles_nested_pairs() {
    let text = "f(g(1), 2)";
    assert_eq!(pair(text, 2), Some((1, 9)), "外层");
    assert_eq!(pair(text, 4), Some((3, 5)), "内层");
}

#[test]
fn ignores_brackets_inside_strings_and_comments() {
    let text = "MsgBox \"(\" & f(1)";
    let open = text.find('f').unwrap() + 1;
    assert_eq!(pair(text, open), Some((open, text.len() - 1)));

    let commented = "' ( 注释里的括号\nf(1)";
    let line_start = commented.find("f(").unwrap();
    assert_eq!(
        pair(commented, line_start + 2),
        Some((line_start + 1, commented.len() - 1))
    );
}

#[test]
fn unmatched_brackets_return_none() {
    assert_eq!(pair("f(1", 2), None);
    assert_eq!(pair("f) ", 1), None, "没有配对的闭括号");
    assert_eq!(pair("no brackets here", 3), None);
}

#[test]
fn cursor_between_brackets_prefers_the_following_opening_bracket() {
    let text = "()";
    assert_eq!(pair(text, 0), Some((0, 1)), "光标在开括号之前");
}
