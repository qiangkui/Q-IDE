//! 日期时间：序列值换算、`CDate`/`DateValue`/`TimeValue` 解析、`DateAdd`/`DateDiff`/`DatePart`。
//!
//! 从 `builtins.rs` 按职责拆出来；`call()` 仍在 `super` 里按名字分派。

use super::*;

/// 星期名（索引 0 = 星期日）：本机 VB6（中文区域）实测是中文全称 + `周X` 缩写。
pub(super) const WEEKDAY_NAMES: [&str; 7] = [
    "星期日",
    "星期一",
    "星期二",
    "星期三",
    "星期四",
    "星期五",
    "星期六",
];

pub(super) const WEEKDAY_ABBREVIATIONS: [&str; 7] =
    ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

/// 月份名（索引 0 = 一月）：全称 `一月`…`十二月`，缩写 `1月`…`12月`。
pub(super) const MONTH_NAMES: [&str; 12] = [
    "一月",
    "二月",
    "三月",
    "四月",
    "五月",
    "六月",
    "七月",
    "八月",
    "九月",
    "十月",
    "十一月",
    "十二月",
];

/// Unix 秒 → VB 日期序列（UTC）。
pub(super) fn serial_from_unix(seconds: f64) -> f64 {
    let epoch = (days_from_civil(1970, 1, 1) - ole_epoch_days()) as f64;
    epoch + seconds / 86_400.0
}

/// 本地时区相对 UTC 的偏移（天），取整到分钟，避免亚秒抖动。
pub(super) fn local_offset_days() -> f64 {
    let utc_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    let offset = now_serial() - serial_from_unix(utc_now);
    (offset * 1440.0).round() / 1440.0
}

/// 星期几：默认 1=星期日 … 7=星期六，`first_day` 指定一周的第一天。
pub(super) fn weekday_of(serial: f64, first_day: i64) -> i64 {
    // 日期序列 0 = 1899-12-30 = 星期六。
    let sunday_based = (serial.floor() as i64 + 6).rem_euclid(7) + 1;
    let first = if first_day == 0 { 1 } else { first_day };
    ((sunday_based - first).rem_euclid(7)) + 1
}

/// 识别 `2024-01-31`、`2024/1/31`、`1/31/2024`、`24-2-29`、`2024年1月31日` 这类日期文本。
///
/// 三个数字段依次按「年-月-日」「月-日-年」「日-月-年」尝试，取第一个合法组合
/// （本机 VB6 实测：`CDate("24-2-29")` 是 2024-02-29、`DateValue("3/4/2024")` 是 2024-03-04、
/// `DateValue("13/11/2024")` 是 2024-11-13）；年份不超过两位时按 1930/2000 规则展开。
pub fn parse_date_text(text: &str) -> Option<f64> {
    let cleaned = text.trim().replace(['年', '月'], "-").replace('日', "");
    let parts: Vec<i64> = cleaned
        .split(['-', '/', '.'])
        .filter(|part| !part.trim().is_empty())
        .filter_map(|part| part.trim().parse::<i64>().ok())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let candidates = [
        (two_digit_year(parts[0]), parts[1], parts[2]),
        (parts[2], parts[0], parts[1]),
        (parts[2], parts[1], parts[0]),
    ];
    candidates
        .into_iter()
        .find_map(|(year, month, day)| serial_from_ymd(year, month, day).ok())
}

pub fn parse_date_value(text: &str) -> Option<f64> {
    let text = text.trim();
    if let Some(serial) = parse_date_text(text) {
        return Some(serial);
    }
    if let Some(serial) = parse_time_text(text) {
        return Some(serial);
    }
    for (index, ch) in text.char_indices() {
        if ch.is_whitespace() || ch == 'T' || ch == 't' {
            let (date, time) = text.split_at(index);
            let date = parse_date_text(date)?;
            let time = parse_time_text(time.trim_start_matches([' ', 'T', 't']))?;
            return Some(date + time);
        }
    }
    parse_number(text)
}

/// 识别 `12:34`、`12:34:56`、`1:2:3 PM`、`9:05am` 这类时间文本。
///
/// 本机 VB6 实测：不收小数秒（`TimeValue("12:34:56.5")` 报 13）、时分秒超范围报 13；
/// `AM`/`PM` 后缀可以紧贴也可以空一格，且只在 12 小时制内起作用
/// （`TimeValue("13:00 PM")` 是 13:00:00，后缀被忽略）。
pub(super) fn parse_time_text(text: &str) -> Option<f64> {
    let (body, am) = split_meridiem(text);
    let mut parts = body.split(':');
    let hour = parse_time_field(parts.next()?)?;
    let minute = match parts.next() {
        Some(part) => parse_time_field(part)?,
        None => 0,
    };
    let second = match parts.next() {
        Some(part) => parse_time_field(part)?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    let hour = match am {
        Some(true) if hour == 12 => 0,
        Some(true) => hour,
        Some(false) if hour < 12 => hour + 12,
        Some(false) => hour,
        None => hour,
    };
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some((hour * 3600 + minute * 60 + second) as f64 / 86_400.0)
}

/// 时间字段只认十进制数字；空串、符号、小数点都算非法。
pub(super) fn parse_time_field(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// 拆掉末尾的 `AM` / `PM`（大小写不敏感、前面可以空一格），返回（正文, 是否 AM）。
pub(super) fn split_meridiem(text: &str) -> (&str, Option<bool>) {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let am = if lower.ends_with("am") {
        true
    } else if lower.ends_with("pm") {
        false
    } else {
        return (trimmed, None);
    };
    let body = trimmed[..trimmed.len() - 2].trim_end();
    (body, Some(am))
}

pub(super) fn is_date(value: &Value) -> bool {
    match value {
        Value::Date(_) | Value::Float(_) | Value::Int(_) | Value::Bool(_) => true,
        Value::Str(text) => parse_date_value(text).is_some(),
        _ => false,
    }
}

/// 取字符串开头的数字部分（`Val` 的语义）。
///
/// 支持前导空白、正负号，以及 VB 的 `&H`（十六进制）、`&O` / `&`（八进制）前缀；
/// 遇到第一个不能解释的字符就停下（`Val("12abc")` 是 12、`Val("1.2.3")` 是 1.2）。
pub(super) fn leading_number(text: &str) -> Option<f64> {
    let trimmed = text.trim_start();
    let (negative, body) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    // 符号后面还可以有空格（VB6 实测 `Val("  -  1")` 是 -1）。
    let body = body.trim_start();
    if body.starts_with('&') {
        // 取补码规则与进制字面量一致（`Val("&HFFFF")` 是 -1、`Val("&HFFFF&")` 是 65535）；
        // `&H` 后面没有合法数字时 VB 也返回 0。
        let value = parse_radix_text(body).map_or(0.0, |(value, _)| value as f64);
        return Some(if negative { -value } else { value });
    }
    let mut end = 0usize;
    let mut seen_digit = false;
    let mut seen_dot = false;
    let mut seen_exponent = false;
    for (index, ch) in body.char_indices() {
        match ch {
            '0'..='9' => seen_digit = true,
            '.' if !seen_dot && !seen_exponent => seen_dot = true,
            // `Val("1e")`、`Val("1E+")` 都只取前面的 1：指数后面没有数字就不算指数。
            // `D` 也是 VB 的指数标记（`Val("1D5")` 是 100000）。
            'e' | 'E' | 'd' | 'D'
                if seen_digit && !seen_exponent && exponent_follows(&body[index + 1..]) =>
            {
                seen_exponent = true;
            }
            '+' | '-' if seen_exponent => {}
            _ => break,
        }
        end = index + ch.len_utf8();
    }
    if !seen_digit {
        // `Val("-")`、`Val("abc")` 都是 0。
        return None;
    }
    let text = body[..end].replace(['d', 'D'], "e");
    let value = parse_number(&text)?;
    Some(if negative { -value } else { value })
}

/// 指数标记后面是否真的有数字（`Val("1e")` 的 `e` 不算指数）。
pub(super) fn exponent_follows(rest: &str) -> bool {
    let rest = rest.strip_prefix(['+', '-']).unwrap_or(rest);
    rest.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

// ---------------------------------------------------------------- 日期时间

/// 公历日期 → 自 1970-01-01 起的天数（Howard Hinnant 的算法）。
pub(super) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// 天数 → 公历日期。
pub(super) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

pub(super) fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub(super) fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        _ => 28,
    }
}

/// VB 的日期序列以 1899-12-30 为 0。
pub(super) fn ole_epoch_days() -> i64 {
    days_from_civil(1899, 12, 30)
}

pub(super) fn serial_from_ymd(year: i64, month: i64, day: i64) -> Result<f64, String> {
    if !(1..=12).contains(&month) {
        return Err(format!("月份 {month} 超出 1..12"));
    }
    let last = days_in_month(year, month);
    if day < 1 || day > last {
        return Err(format!("日期 {day} 超出 {year}-{month} 的范围 1..{last}"));
    }
    Ok((days_from_civil(year, month, day) - ole_epoch_days()) as f64)
}

/// `DateSerial` 的两位数年份规则：0–29 → 2000–2029，30–99 → 1930–1999（VB6 实测）。
pub(super) fn two_digit_year(year: i64) -> i64 {
    match year {
        0..=29 => year + 2000,
        30..=99 => year + 1900,
        _ => year,
    }
}

/// `DateSerial` 允许月、日溢出（VB6 实测 `DateSerial(2024, 2, 30)` 是 2024-03-01）。
pub(super) fn serial_from_normalized(year: i64, month: i64, day: i64) -> f64 {
    let index = year * 12 + (month - 1);
    let normal_year = index.div_euclid(12);
    let normal_month = index.rem_euclid(12) + 1;
    (days_from_civil(normal_year, normal_month, 1) + day - 1 - ole_epoch_days()) as f64
}

pub(super) fn serial_from_parts(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    millisecond: i64,
) -> f64 {
    (days_from_civil(year, month, day) - ole_epoch_days()) as f64
        + (hour * 3600 + minute * 60 + second) as f64 / 86_400.0
        + millisecond as f64 / 86_400_000.0
}

/// 日期序列 → `(年, 月, 日, 时, 分, 秒)`。
pub(super) fn parts_from_serial(serial: f64) -> (i64, i64, i64, i64, i64, i64) {
    let days = serial.floor();
    let fraction = serial - days;
    let (year, month, day) = civil_from_days(days as i64 + ole_epoch_days());
    let total_ms = ((fraction * 86_400_000.0).round() as i64).rem_euclid(86_400_000);
    let total_seconds = total_ms / 1000;
    (
        year,
        month,
        day,
        (total_seconds / 3600).rem_euclid(24),
        (total_seconds / 60).rem_euclid(60),
        total_seconds.rem_euclid(60),
    )
}

/// 当前本地时间的日期序列；非 Windows 用 UTC。
pub(super) fn now_serial() -> f64 {
    if let Some(parts) = ffi::local_time() {
        return serial_from_parts(
            i64::from(parts.year),
            i64::from(parts.month),
            i64::from(parts.day),
            i64::from(parts.hour),
            i64::from(parts.minute),
            i64::from(parts.second),
            i64::from(parts.millisecond),
        );
    }
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    let days = (seconds / 86_400.0).floor() as i64;
    let rest = seconds - days as f64 * 86_400.0;
    let (year, month, day) = civil_from_days(days);
    serial_from_parts(
        year,
        month,
        day,
        (rest / 3600.0) as i64,
        ((rest % 3600.0) / 60.0) as i64,
        (rest % 60.0) as i64,
        0,
    )
}

pub(super) fn date_add(interval: &str, amount: i64, serial: f64) -> Result<f64, String> {
    Ok(match interval {
        "yyyy" => add_months(serial, amount * 12),
        "q" => add_months(serial, amount * 3),
        "m" => add_months(serial, amount),
        // `y`（一年中的第几天）与 `w`（星期几）在 VB6 里都按「天」加减，`ww` 才是按周。
        "y" | "d" | "w" => serial + amount as f64,
        "ww" => serial + amount as f64 * 7.0,
        "h" => serial + amount as f64 / 24.0,
        "n" => serial + amount as f64 / 1440.0,
        "s" => serial + amount as f64 / 86_400.0,
        other => return Err(format!("`DateAdd` 不支持间隔 `{other}`")),
    })
}

pub(super) fn add_months(serial: f64, months: i64) -> f64 {
    let (year, month, day, hour, minute, second) = parts_from_serial(serial);
    let index = year * 12 + (month - 1) + months;
    let new_year = index.div_euclid(12);
    let new_month = index.rem_euclid(12) + 1;
    let new_day = day.min(days_in_month(new_year, new_month));
    serial_from_parts(new_year, new_month, new_day, hour, minute, second, 0)
}

pub(super) fn date_diff(interval: &str, first: f64, second: f64) -> Result<i64, String> {
    let (year1, month1, _, _, _, _) = parts_from_serial(first);
    let (year2, month2, _, _, _, _) = parts_from_serial(second);
    Ok(match interval {
        "yyyy" => year2 - year1,
        // 季度差按「绝对季度序号」相减：`DateDiff("q", #2024-03-31#, #2024-04-01#)` 是 1。
        "q" => (year2 * 4 + (month2 - 1) / 3) - (year1 * 4 + (month1 - 1) / 3),
        "m" => (year2 - year1) * 12 + (month2 - month1),
        // `y`（一年中的第几天）按天算；`w`（星期几）在 `DateDiff` 里与 `ww` 一样按周算（VB6 实测）。
        "y" | "d" => (second.floor() - first.floor()) as i64,
        "w" | "ww" => (second.floor() - first.floor()) as i64 / 7,
        "h" => ((second - first) * 24.0) as i64,
        "n" => ((second - first) * 1440.0) as i64,
        "s" => ((second - first) * 86_400.0) as i64,
        other => return Err(format!("`DateDiff` 不支持间隔 `{other}`")),
    })
}

/// `DatePart` 取日期的一部分；`first_day`/`first_week` 对应 VB 的
/// `vbDayOfWeek`（1 = 星期日）与 `vbFirstWeekOfYear`（1 = 含 1 月 1 日的那一周）。
pub(super) fn date_part(
    interval: &str,
    serial: f64,
    first_day: i64,
    first_week: i64,
) -> Result<i64, String> {
    let (year, month, day, hour, minute, second) = parts_from_serial(serial);
    Ok(match interval {
        "yyyy" => year,
        "q" => (month - 1) / 3 + 1,
        "m" => month,
        "y" => serial.floor() as i64 - serial_from_ymd(year, 1, 1)? as i64 + 1,
        "d" => day,
        "w" => weekday_of(serial, first_day),
        "ww" => week_of_year(serial, first_day, first_week),
        "h" => hour,
        "n" => minute,
        "s" => second,
        other => return Err(format!("`DatePart` 不支持间隔 `{other}`")),
    })
}

/// 某一年第 1 周第一天的日期（自 1970-01-01 起的天数）。
///
/// `first_week`：0/1 = 含 1 月 1 日的那一周，2 = 新年里至少有 4 天的那一周，
/// 3 = 新年里完整的第一周。
pub(super) fn week_one_start(year: i64, first_day: i64, first_week: i64) -> i64 {
    let jan1 = days_from_civil(year, 1, 1);
    // 1970-01-01 是星期四：`(days + 4) % 7 + 1` 给出「星期日是 1」的星期号。
    let weekday = (jan1 + 4).rem_euclid(7) + 1;
    let first = if first_day == 0 { 1 } else { first_day };
    let offset = (weekday - first).rem_euclid(7);
    let offset = match first_week {
        2 if 7 - offset < 4 => offset - 7,
        3 if offset > 0 => offset - 7,
        _ => offset,
    };
    jan1 - offset
}

/// 一年有几周（相邻两年第 1 周起点相差几周）。
pub(super) fn weeks_in_year(year: i64, first_day: i64, first_week: i64) -> i64 {
    (week_one_start(year + 1, first_day, first_week) - week_one_start(year, first_day, first_week))
        / 7
}

/// 一年中的第几周。跨年时先退到上一年，`vbFirstFourDays` 还要把超出的周并到下一年的第 1 周
/// （本机 VB6 实测：`ww` 的年末表现按 `DatePart` 的参数变化，见 `docs/脚本语言.md`）。
pub(super) fn week_of_year(serial: f64, first_day: i64, first_week: i64) -> i64 {
    let days = serial.floor() as i64 + ole_epoch_days();
    let (mut year, _, _, _, _, _) = parts_from_serial(serial);
    let mut week = (days - week_one_start(year, first_day, first_week)).div_euclid(7) + 1;
    if week < 1 {
        year -= 1;
        week = (days - week_one_start(year, first_day, first_week)).div_euclid(7) + 1;
    }
    if first_week == 2 {
        let total = weeks_in_year(year, first_day, first_week);
        if week > total {
            week -= total;
        }
    }
    week
}

pub(super) fn is_date_format(spec: &str) -> bool {
    spec.chars().any(|ch| {
        matches!(
            ch.to_ascii_lowercase(),
            'y' | 'm' | 'd' | 'h' | 'n' | 's' | 'w' | 'q' | 'a' | 'c'
        )
    })
}

/// 英文字母的星期/月份名（`Format` 的 `ddd`/`dddd`/`mmm`/`mmmm` 用英文，见本机 VB6 实测）。
pub(super) const ENGLISH_WEEKDAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

pub(super) const ENGLISH_WEEKDAY_ABBREVIATIONS: [&str; 7] =
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

pub(super) const ENGLISH_MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub(super) const ENGLISH_MONTH_ABBREVIATIONS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `Format(date, "yyyy-mm-dd hh:nn:ss")` 的日期格式子集，占位符规则逐条来自本机 VB6 实测。
///
/// - `y` = 一年中的第几天、`yy` = 两位年、`yyyy` = 四位年；
/// - `m`/`mm` = 月，但**紧跟在 `h`/`hh` 之后时是分钟**；`mmm`/`mmmm` = 英文月份与全称；
/// - `d`/`dd` = 日，`ddd`/`dddd` = 英文星期缩写/全称，`aaa`/`aaaa` = 中文星期缩写/全称；
/// - `h`/`hh` = 小时（出现 `AM/PM` 时按 12 小时制）、`n`/`nn` = 分钟、`s`/`ss` = 秒；
/// - `w` = 星期号（星期日为 1）、`ww` = 一年中的第几周、`q` = 季度；
/// - `AM/PM`、`am/pm`、`A/P`、`a/p` 输出对应大小写的英文标记；
/// - `"…"` 与 `\x` 是字面量转义。
pub(super) fn format_date(serial: f64, spec: &str) -> String {
    let (year, month, day, hour, minute, second) = parts_from_serial(serial);
    let weekday = weekday_of(serial, 1) as usize;
    let day_of_year = serial.floor() as i64 + ole_epoch_days() - days_from_civil(year, 1, 1) + 1;
    let lowered = spec.to_ascii_lowercase();
    let twelve_hour = lowered.contains("am/pm") || lowered.contains("a/p");
    let chars: Vec<char> = spec.chars().collect();
    let mut output = String::new();
    let mut index = 0usize;
    // 上一个占位符：`m` 紧跟 `h` 才算分钟。
    let mut previous = '\0';
    while index < chars.len() {
        let ch = chars[index];
        if ch == '"' {
            index += 1;
            while index < chars.len() && chars[index] != '"' {
                output.push(chars[index]);
                index += 1;
            }
            index += 1;
            continue;
        }
        if ch == '\\'
            && let Some(next) = chars.get(index + 1)
        {
            output.push(*next);
            index += 2;
            continue;
        }
        let rest: String = chars[index..]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        // `AM/PM` / `A/P`：输出哪种大小写跟着占位符写法走（`hh am/pm` 给 `pm`）。
        let marker = if rest.starts_with("am/pm") {
            Some((5usize, if hour < 12 { "AM" } else { "PM" }))
        } else if rest.starts_with("a/p") {
            Some((3usize, if hour < 12 { "A" } else { "P" }))
        } else {
            None
        };
        if let Some((length, text)) = marker {
            if ch.is_lowercase() {
                output.push_str(&text.to_lowercase());
            } else {
                output.push_str(text);
            }
            index += length;
            previous = 'a';
            continue;
        }
        let lower = ch.to_ascii_lowercase();
        let run = chars[index..]
            .iter()
            .take_while(|next| next.to_ascii_lowercase() == lower)
            .count();
        let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
        let text = match lower {
            'y' if run == 1 => day_of_year.to_string(),
            'y' if run == 2 => format!("{:02}", year.rem_euclid(100)),
            'y' => format!("{year:04}"),
            'm' if previous == 'h' => {
                if run >= 2 {
                    format!("{minute:02}")
                } else {
                    minute.to_string()
                }
            }
            'm' if run >= 4 => ENGLISH_MONTH_NAMES[(month - 1) as usize].to_string(),
            'm' if run == 3 => ENGLISH_MONTH_ABBREVIATIONS[(month - 1) as usize].to_string(),
            'm' if run == 2 => format!("{month:02}"),
            'm' => month.to_string(),
            'd' if run >= 4 => ENGLISH_WEEKDAY_NAMES[weekday - 1].to_string(),
            'd' if run == 3 => ENGLISH_WEEKDAY_ABBREVIATIONS[weekday - 1].to_string(),
            'd' if run == 2 => format!("{day:02}"),
            'd' => day.to_string(),
            'a' if run >= 4 => WEEKDAY_NAMES[weekday - 1].to_string(),
            'a' if run == 3 => WEEKDAY_ABBREVIATIONS[weekday - 1].to_string(),
            'h' => {
                let value = if twelve_hour { hour12 } else { hour };
                if run >= 2 {
                    format!("{value:02}")
                } else {
                    value.to_string()
                }
            }
            'n' if run >= 2 => format!("{minute:02}"),
            'n' => minute.to_string(),
            's' if run >= 2 => format!("{second:02}"),
            's' => second.to_string(),
            'w' if run >= 2 => week_of_year(serial, 1, 1).to_string(),
            'w' => weekday.to_string(),
            'q' => ((month - 1) / 3 + 1).to_string(),
            // 单个 `c` 是 VB6 的 `General Date`（`Format(123, "c")` 是 `1900/5/2`）。
            'c' if run == 1 => format_date_display(serial),
            _ => {
                output.push(ch);
                index += 1;
                continue;
            }
        };
        output.push_str(&text);
        index += run;
        previous = lower;
    }
    output
}

/// `Date` 值转文本（VB6 的 `General Date`，中文区域实测）：
/// 有日期有时间的写 `2024/2/29 13:05:06`，只有日期的写 `2024/2/29`，
/// 序列 0（1899-12-30）只写时间 `0:00:00`。月、日、时不补零，分隔符是 `/`。
pub fn format_date_display(serial: f64) -> String {
    let (_, _, _, hour, minute, second) = parts_from_serial(serial);
    let has_date = serial.floor() != 0.0;
    let has_time = hour != 0 || minute != 0 || second != 0;
    match (has_date, has_time) {
        (true, true) => format_date(serial, "yyyy/m/d h:nn:ss"),
        (true, false) => format_date(serial, "yyyy/m/d"),
        (false, _) => format_date(serial, "h:nn:ss"),
    }
}
