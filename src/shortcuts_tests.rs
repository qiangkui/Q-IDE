//! 快捷键一览表的测试：与命令面板清单保持一致，避免两处各说各话。

use std::collections::HashSet;

use crate::commands;
use crate::shortcuts;

#[test]
fn sections_are_not_empty() {
    assert!(!shortcuts::SECTIONS.is_empty());
    for section in shortcuts::SECTIONS {
        assert!(!section.title.is_empty(), "分区标题不能为空");
        assert!(
            !section.entries.is_empty(),
            "分区「{}」没有条目",
            section.title
        );
        for entry in section.entries {
            assert!(
                !entry.keys.is_empty() && !entry.action.is_empty(),
                "分区「{}」有空行",
                section.title
            );
        }
    }
}

#[test]
fn covers_every_command_shortcut() {
    let mut missing = Vec::new();
    for command in commands::COMMANDS {
        if command.shortcut.is_empty() {
            continue;
        }
        let found = shortcuts::SECTIONS.iter().any(|section| {
            section
                .entries
                .iter()
                .any(|entry| entry.keys == command.shortcut && entry.action == command.label)
        });
        if !found {
            missing.push(format!("{}（{}）", command.label, command.shortcut));
        }
    }
    assert!(
        missing.is_empty(),
        "这些带快捷键的命令没有出现在快捷键一览里：{missing:?}"
    );
}

#[test]
fn rows_are_unique() {
    let mut seen = HashSet::new();
    for section in shortcuts::SECTIONS {
        for entry in section.entries {
            let row = (section.title, entry.keys, entry.action);
            assert!(seen.insert(row), "重复的条目：{row:?}");
        }
    }
}

#[test]
fn command_shortcuts_are_unique() {
    let mut seen = HashSet::new();
    for command in commands::COMMANDS {
        if command.shortcut.is_empty() {
            continue;
        }
        assert!(
            seen.insert(command.shortcut),
            "快捷键 {} 被多个命令占用",
            command.shortcut
        );
    }
}
