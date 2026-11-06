//! 命令面板的命令清单与匹配测试。

use std::collections::HashSet;

use crate::commands::{COMMANDS, best_command};

#[test]
fn every_command_id_is_unique() {
    let mut seen = HashSet::new();
    for command in COMMANDS {
        assert!(seen.insert(command.id), "重复的命令 id：{}", command.id);
    }
}

#[test]
fn every_command_label_is_unique() {
    let mut seen = HashSet::new();
    for command in COMMANDS {
        assert!(seen.insert(command.label), "重复的命令名：{}", command.label);
    }
}

#[test]
fn an_exact_label_wins() {
    assert_eq!(best_command("保存").map(|command| command.id), Some("file.save"));
    assert_eq!(
        best_command("保存全部").map(|command| command.id),
        Some("file.save-all")
    );
    assert_eq!(
        best_command("调试运行").map(|command| command.id),
        Some("run.debug")
    );
}

#[test]
fn a_prefix_also_matches() {
    assert_eq!(best_command("另存").map(|command| command.id), Some("file.save-as"));
    assert_eq!(
        best_command("全屏").map(|command| command.id),
        Some("view.fullscreen")
    );
}

#[test]
fn surrounding_whitespace_is_ignored() {
    assert_eq!(
        best_command("  运行脚本  ").map(|command| command.id),
        Some("run.start")
    );
}

#[test]
fn an_empty_query_matches_nothing() {
    assert!(best_command("").is_none());
    assert!(best_command("   ").is_none());
}

#[test]
fn an_unknown_query_matches_nothing() {
    assert!(best_command("zzzz").is_none());
}
