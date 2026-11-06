//! 编辑区路径栏相关的纯逻辑测试。

use crate::editor_pane::split_path_for_display;

#[test]
fn splits_windows_paths_into_directory_and_name() {
    assert_eq!(
        split_path_for_display("C:\\work\\demo\\main.vb"),
        ("C:\\work\\demo\\", "main.vb")
    );
}

#[test]
fn splits_forward_slash_paths_too() {
    assert_eq!(split_path_for_display("src/app.rs"), ("src/", "app.rs"));
}

#[test]
fn a_bare_file_name_has_no_directory() {
    assert_eq!(split_path_for_display("main.vb"), ("", "main.vb"));
}

#[test]
fn a_trailing_separator_leaves_an_empty_name() {
    assert_eq!(split_path_for_display("C:\\work\\"), ("C:\\work\\", ""));
}

#[test]
fn only_the_last_separator_splits() {
    assert_eq!(
        split_path_for_display("C:\\a\\b\\c\\d.txt"),
        ("C:\\a\\b\\c\\", "d.txt")
    );
}
