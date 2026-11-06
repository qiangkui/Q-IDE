//! 输出面板选区逻辑的测试。
//!
//! 与资源管理器一样放在独立文件里：写在 `output.rs` 的 `mod tests` 中会触发
//! rustc 的宏展开递归上限。

use std::time::Duration;

use crate::output::{
    OutputFilter, OutputLevel, OutputLine, TextPosition, TextSelection, can_group, display_text,
    line_number_prefix, selection_range, selection_text,
};

fn lines(texts: &[&str]) -> Vec<OutputLine> {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| OutputLine {
            level: OutputLevel::Info,
            text: (*text).to_string(),
            at: Duration::from_millis(index as u64 * 10),
            repeat: 1,
        })
        .collect()
}

fn at(line: usize, byte: usize) -> TextPosition {
    TextPosition::new(line, byte)
}

#[test]
fn selecting_part_of_a_line_keeps_only_that_part() {
    let log = lines(&["cargo build 结束"]);
    let selection = TextSelection::new(at(0, 0), at(0, 11));

    assert_eq!(selection_text(&log, selection), "cargo build");
}

#[test]
fn selecting_backwards_uses_reading_order() {
    let log = lines(&["hello", "world"]);
    let selection = TextSelection::new(at(1, 5), at(0, 4));

    assert_eq!(selection.ordered(), (at(0, 4), at(1, 5)));
    assert_eq!(selection_text(&log, selection), "o\nworld");
}

#[test]
fn selecting_whole_lines_joins_them_with_newlines() {
    let log = lines(&["一", "二", "三"]);
    let selection = TextSelection::new(at(0, 0), at(2, "三".len()));

    assert_eq!(selection_text(&log, selection), "一\n二\n三");
}

#[test]
fn selecting_an_empty_first_line_keeps_the_line_break() {
    let log = lines(&["", "脚本执行结束"]);
    let selection = TextSelection::new(at(0, 0), at(1, 6));

    assert_eq!(selection_text(&log, selection), "\n脚本");
}

#[test]
fn collapsed_selection_selects_nothing() {
    let log = lines(&["cargo build", "cargo test"]);
    let selection = TextSelection::new(at(1, 3), at(1, 3));

    assert!(selection.is_collapsed());
    assert_eq!(selection_text(&log, selection), "");
}

#[test]
fn out_of_range_positions_are_clamped_to_char_boundaries() {
    let text = "脚本执行结束";
    let log = lines(&[text]);

    // 排版结果里的末尾位置可能超过行尾，也不能切在多字节字符中间。
    assert_eq!(
        selection_text(&log, TextSelection::new(at(0, 0), at(0, usize::MAX))),
        text
    );
    assert_eq!(selection_range(at(0, 0), at(0, 6), 0, text), Some(0..6));
    assert_eq!(
        selection_range(at(0, 0), at(0, 7), 0, text),
        Some(0..6),
        "7 不是字符边界，要退回到上一个边界"
    );
}

#[test]
fn lines_outside_the_selection_have_no_range() {
    let selection = TextSelection::new(at(1, 0), at(1, 1));
    let (start, end) = selection.ordered();

    assert_eq!(selection_range(start, end, 0, "a"), None);
    assert_eq!(selection_range(start, end, 1, "b"), Some(0..1));
    assert_eq!(selection_range(start, end, 2, "c"), None);
}

#[test]
fn repeated_errors_are_grouped_but_other_levels_are_not() {
    let error = OutputLine {
        level: OutputLevel::Error,
        text: "第 3 行运行时错误: 类型不匹配".to_string(),
        at: Duration::ZERO,
        repeat: 1,
    };

    assert!(can_group(&error, OutputLevel::Error, &error.text));
    assert!(!can_group(&error, OutputLevel::Error, "另一条错误"));
    assert!(
        !can_group(&error, OutputLevel::Warning, &error.text),
        "级别不同不合并"
    );

    let info = OutputLine {
        level: OutputLevel::Info,
        ..error.clone()
    };
    assert!(
        !can_group(&info, OutputLevel::Info, &info.text),
        "普通信息不合并"
    );

    let warning = OutputLine {
        level: OutputLevel::Warning,
        ..error.clone()
    };
    assert!(can_group(&warning, OutputLevel::Warning, &warning.text));
}

fn line_at(level: OutputLevel, text: &str) -> OutputLine {
    OutputLine {
        level,
        text: text.to_string(),
        at: Duration::ZERO,
        repeat: 1,
    }
}

#[test]
fn an_empty_filter_keeps_every_line() {
    let filter = OutputFilter::default();
    assert!(!filter.is_active());
    assert!(filter.matches(&line_at(OutputLevel::Info, "随便什么")));
    assert!(filter.matches(&line_at(OutputLevel::Error, "错误")));
}

#[test]
fn the_keyword_filter_ignores_case_and_surrounding_spaces() {
    let filter = OutputFilter {
        text: "  错误 ".to_string(),
        only_errors: false,
    };
    assert!(filter.is_active());
    assert!(filter.matches(&line_at(
        OutputLevel::Info,
        "第 3 行运行时错误: 类型不匹配"
    )));
    assert!(!filter.matches(&line_at(OutputLevel::Info, "脚本执行结束")));
}

#[test]
fn the_only_errors_filter_keeps_warnings_and_errors() {
    let filter = OutputFilter {
        text: String::new(),
        only_errors: true,
    };
    assert!(filter.is_active());
    assert!(filter.matches(&line_at(OutputLevel::Error, "错误")));
    assert!(filter.matches(&line_at(OutputLevel::Warning, "警告")));
    assert!(!filter.matches(&line_at(OutputLevel::Info, "信息")));
    assert!(!filter.matches(&line_at(OutputLevel::Success, "成功")));
    assert!(!filter.matches(&line_at(OutputLevel::Command, "> 运行脚本")));
}

#[test]
fn both_conditions_must_hold_when_combined() {
    let filter = OutputFilter {
        text: "类型".to_string(),
        only_errors: true,
    };
    assert!(filter.matches(&line_at(OutputLevel::Error, "类型不匹配")));
    assert!(!filter.matches(&line_at(OutputLevel::Error, "找不到文件")));
    assert!(!filter.matches(&line_at(OutputLevel::Info, "类型：整数")));
}

#[test]
fn display_text_appends_the_repeat_count() {
    let mut line = OutputLine {
        level: OutputLevel::Error,
        text: "出错了".to_string(),
        at: Duration::ZERO,
        repeat: 1,
    };
    assert_eq!(display_text(&line), "出错了");
    line.repeat = 3;
    assert_eq!(display_text(&line), "出错了（重复 3 次）");
}

#[test]
fn error_lines_parse_their_line_number() {
    assert_eq!(line_number_prefix("第 12 行运行时错误: 类型不匹配"), Some(12));
    assert_eq!(line_number_prefix("第 3 行语法错误: 缺少 `Then`"), Some(3));
    assert_eq!(line_number_prefix("脚本执行结束"), None);
    assert_eq!(line_number_prefix("第 行错误"), None);
    assert_eq!(line_number_prefix("第 12行错误"), None);
}

