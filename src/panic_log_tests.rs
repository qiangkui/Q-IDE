//! 崩溃日志的纯逻辑测试：时间格式化与报告文本（真正写文件的部分依赖 `%APPDATA%`，不在这里测）。

use crate::panic_log;

#[test]
fn format_utc_converts_known_epoch_seconds() {
    assert_eq!(panic_log::format_utc(0), "1970-01-01 00:00:00");
    assert_eq!(panic_log::format_utc(86_399), "1970-01-01 23:59:59");
    assert_eq!(panic_log::format_utc(951_782_400), "2000-02-29 00:00:00");
    // 闰日之后一天、跨年、以及一个普通时刻。
    assert_eq!(panic_log::format_utc(1_709_164_800), "2024-02-29 00:00:00");
    assert_eq!(panic_log::format_utc(1_735_689_599), "2024-12-31 23:59:59");
    assert_eq!(panic_log::format_utc(1_700_000_000), "2023-11-14 22:13:20");
}

#[test]
fn report_contains_version_location_and_message() {
    let report = panic_log::format_report(
        "0.1.0",
        1_700_000_000,
        Some(("src/app/editor.rs", 42)),
        "index out of bounds",
    );
    assert!(report.starts_with("=== 2023-11-14 22:13:20 ===\n"));
    assert!(report.contains("版本 0.1.0\n"));
    assert!(report.contains("位置 src/app/editor.rs:42\n"));
    assert!(report.contains("消息 index out of bounds\n"));
    assert!(report.ends_with("\n\n"), "条目之间要留空行");
}

#[test]
fn report_without_location_says_so() {
    let report = panic_log::format_report("0.1.0", 0, None, "boom");
    assert!(report.contains("位置 未知位置\n"));
}

#[test]
fn log_path_lives_next_to_the_layout_file() {
    // 只校验形状：CI 上没有 `%APPDATA%`（或与桌面不同的取值）时允许为 `None`。
    if let Some(path) = panic_log::log_path() {
        assert!(path.ends_with(std::path::Path::new("gpui-first").join("crash.log")));
    }
}
