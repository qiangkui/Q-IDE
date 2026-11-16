//! 崩溃日志：把 panic 信息追加到 `%APPDATA%\gpui-first\crash.log`。
//!
//! 界面线程 panic 时进程会直接退出、什么都不留下，用户也没法回报。这里装一个
//! `panic` 钩子：先把报告写进日志文件，再交给原来的钩子（调试器里仍然能看到标准输出）。
//! 报告里只放**位置 + 消息 + 版本 + 时间**，不抓堆栈（`RUST_BACKTRACE=1` 时才由默认钩子输出）。

use std::io::Write;
use std::path::PathBuf;

/// 日志文件上限：超过就重开一份，避免长期使用无限增长。
const MAX_LOG_BYTES: u64 = 256 * 1024;

/// 崩溃日志路径：`%APPDATA%\gpui-first\crash.log`（与布局、设置文件同目录）。
pub fn log_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").map(PathBuf::from)?;
    Some(base.join("gpui-first").join("crash.log"))
}

/// 把「自 1970-01-01 UTC 起的秒数」格式化成 `2024-02-29 13:05:06`。
///
/// 自己算是为了不引日期库：`days_from_civil` 的逆运算（Howard Hinnant 的算法）。
pub fn format_utc(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    let seconds = epoch_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// 自 1970-01-01 起的天数 → (年, 月, 日)。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// 一条崩溃报告：时间、版本、位置、消息，末尾留空行分隔。
pub fn format_report(
    version: &str,
    epoch_secs: u64,
    location: Option<(&str, u32)>,
    message: &str,
) -> String {
    let place = match location {
        Some((file, line)) => format!("{file}:{line}"),
        None => "未知位置".to_string(),
    };
    format!(
        "=== {} ===\n版本 {version}\n位置 {place}\n消息 {message}\n\n",
        format_utc(epoch_secs)
    )
}

/// 装钩子：panic 时写日志再交给默认钩子。
pub fn install(version: &'static str) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = panic_message(info);
        let location = info.location().map(|spot| (spot.file(), spot.line()));
        let epoch_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        append(&format_report(version, epoch_secs, location, &message));
        previous(info);
    }));
}

/// 把一条报告追加到日志文件；文件过大时先删掉重来。写不进去就算了（不能因为记日志再 panic）。
fn append(report: &str) {
    let Some(path) = log_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(report.as_bytes());
    }
}

/// panic 载荷里的文字（`panic!("…")` 与 `panic!("{}", x)` 两种）。
fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(text) = info.payload().downcast_ref::<&str>() {
        return (*text).to_string();
    }
    if let Some(text) = info.payload().downcast_ref::<String>() {
        return text.clone();
    }
    "panic（载荷不是字符串）".to_string()
}
