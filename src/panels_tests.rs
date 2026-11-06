//! 面板菜单结构的测试：资源管理器右键菜单的项与分隔线。

use crate::panels::explorer_menu_entries;

/// 分隔线不能出现在第一项，也不能连续出现（中间会空一行）。
fn assert_separators_are_sane(entries: &[(&'static str, bool)]) {
    assert!(!entries.is_empty());
    for (index, (label, separator)) in entries.iter().enumerate() {
        if *separator {
            assert!(index > 0, "{label} 是菜单第一项，不能带分隔线");
            assert!(
                !entries[index - 1].1,
                "{label} 与上一项之间出现了连续分隔线"
            );
        }
    }
}

#[test]
fn file_menu_starts_with_open_and_has_no_leading_separator() {
    let entries = explorer_menu_entries(false, false);
    assert_eq!(entries[0], ("打开", false));
    assert_separators_are_sane(&entries);
    assert_eq!(
        entries.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
        vec!["打开", "重命名…", "删除…", "复制路径", "刷新"]
    );
}

#[test]
fn folder_menu_starts_with_new_file_and_has_no_leading_separator() {
    let entries = explorer_menu_entries(true, false);
    assert_eq!(entries[0], ("新建文件", false));
    assert_separators_are_sane(&entries);
    assert_eq!(
        entries.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
        vec![
            "新建文件",
            "新建文件夹",
            "重命名…",
            "删除…",
            "复制路径",
            "刷新"
        ]
    );
}

#[test]
fn root_folder_menu_ends_with_remove_root() {
    let entries = explorer_menu_entries(true, true);
    assert_separators_are_sane(&entries);
    assert_eq!(entries.last(), Some(&("从资源管理器移除", true)));
}
