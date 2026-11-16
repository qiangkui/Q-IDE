//! `Format` 的数值格式与命名格式（`Short Date`、`Currency` 等）。
//!
//! 从 `builtins.rs` 按职责拆出来；`call()` 仍在 `super` 里按名字分派。

use super::dates::format_date;
use super::dates::is_date_format;
use super::dates::parts_from_serial;
use super::*;

/// `FormatNumber` / `FormatCurrency` / `FormatPercent`：写法取本机 VB6（中文区域）实测。
///
/// 参数：`(表达式, [小数位=2], [显示前导零=vbTrue], [负数加括号=vbFalse], [千位分隔=vbTrue])`，
/// 可选参数传 `vbUseDefault`(-2) 等同于省略。
pub(super) fn format_amount(name: &str, args: &[Value], ctx: &mut Ctx) -> Result<String, String> {
    let number = args
        .first()
        .map(Value::as_float)
        .transpose()?
        .unwrap_or(0.0);
    let flag = |position: usize, default: bool| -> Result<bool, String> {
        match args.get(position) {
            Some(value) => {
                let flag = value.as_int()?;
                Ok(if flag == -2 { default } else { flag != 0 })
            }
            None => Ok(default),
        }
    };
    let digits = match args.get(1) {
        Some(value) => value.as_int()?,
        None => 2,
    };
    let digits = if digits == -2 { 2 } else { digits };
    if digits < 0 {
        ctx.failure = Some(5);
        return Err("小数位数不能是负数".into());
    }
    let leading = flag(2, true)?;
    let parens = flag(3, false)?;
    let group = flag(4, true)?;
    let digits = digits.min(15) as usize;
    let mut spec = String::new();
    if name == "formatcurrency" {
        spec.push('￥');
    }
    spec.push_str(match (group, leading) {
        (true, true) => "#,##0",
        (true, false) => "#,##",
        (false, true) => "0",
        (false, false) => "#",
    });
    if digits > 0 {
        spec.push('.');
        spec.push_str(&"0".repeat(digits));
    }
    if name == "formatpercent" {
        spec.push('%');
    }
    // 负数加括号：`FormatNumber(-1234.5, 2, , vbTrue)` 是 `(1,234.50)`。
    let spec = if parens {
        format!("{spec};({spec})")
    } else {
        spec
    };
    format_number(number, &spec)
}

/// `FormatDateTime(date, [命名格式])`：0 = 一般、1 = 长日期、2 = 短日期、3 = 长时间、4 = 短时间。
pub(super) fn format_date_named(serial: f64, named: i64, ctx: &mut Ctx) -> Result<String, String> {
    match named {
        0 => Ok(format_date_display(serial)),
        1 => Ok(format_date(serial, "yyyy年m月d日")),
        2 => Ok(format_date(serial, "yyyy/m/d")),
        3 => {
            let (_, _, _, hour, minute, second) = parts_from_serial(serial);
            Ok(format!("{hour:02}:{minute:02}:{second:02}"))
        }
        4 => {
            let (_, _, _, hour, minute, _) = parts_from_serial(serial);
            Ok(format!("{hour:02}:{minute:02}"))
        }
        other => {
            ctx.failure = Some(5);
            Err(format!("`FormatDateTime` 的命名格式 {other} 超出 0..4"))
        }
    }
}

// ---------------------------------------------------------------- 脚本设置（SaveSetting 等）

/// `Format`：支持数值格式（`0`/`#`/`.`/`,`/`%`、`E+00`、`;` 分段）与大小写格式（`>`/`<`）。
pub(super) fn format_value(value: &Value, spec: &str) -> Result<String, String> {
    // 本机 VB6 实测 `Format(Null, "0")` 是空字符串（不报错）。
    if matches!(value, Value::Null) {
        return Ok(String::new());
    }
    if spec.is_empty() {
        return Ok(value.to_display());
    }
    if let Some(text) = named_format(value, spec)? {
        return Ok(text);
    }
    if let Some(text) = named_date_format(value, spec)? {
        return Ok(text);
    }
    if !spec.contains('0') && !spec.contains('#') {
        // `@`（右对齐补空格）与 `&`（不补齐）是文本占位符：`Format(1, "@@@")` 是 `"  1"`。
        if spec.contains('@') || spec.contains('&') {
            return Ok(format_text(value, spec));
        }
        if is_date_format(spec) {
            return Ok(format_date(value.as_float()?, spec));
        }
        let text = value.to_display();
        if spec.contains('>') {
            return Ok(text.to_uppercase());
        }
        if spec.contains('<') {
            return Ok(text.to_lowercase());
        }
        // 没有占位符也没有日期/文本标记：整串按字面量输出（`Format(5, ",")` 是 `,`）。
        return Ok(format_literal(spec));
    }

    let number = value
        .as_float()
        .map_err(|_| format!("`Format` 需要数值，但得到 \"{}\"", value.to_display()))?;
    format_number(number, spec)
}

/// `@` / `&` 文本占位符：文本按占位符个数左补空格（VB6 实测 `Format("abc", "@@@@@")` 是 `"  abc"`、
/// `Format(1234.5, "@@@@@@")` 是 `"1234.5"`，超长时不截断）。
pub(super) fn format_text(value: &Value, spec: &str) -> String {
    // `@` 补空格到占位符个数，`&` 不补（本机 VB6 实测 `Format("abc", "&&&&")` 是 `abc`）。
    let width = spec.chars().filter(|ch| *ch == '@').count();
    let text = value.to_display();
    let length = text.chars().count();
    if length >= width {
        return text;
    }
    format!("{}{text}", " ".repeat(width - length))
}

/// 按数值格式（可含 `;` 分段）格式化一个数。
pub(super) fn format_number(number: f64, spec: &str) -> Result<String, String> {
    let sections: Vec<&str> = spec.split(';').collect();
    let (section, negative_sign) = if number < 0.0 {
        match sections.get(1) {
            Some(negative) => (*negative, false),
            None => (sections[0], true),
        }
    } else if number == 0.0 {
        (sections.get(2).copied().unwrap_or(sections[0]), false)
    } else {
        (sections[0], false)
    };

    let mut text = format_section(number.abs(), section)?;
    // 四舍五入到 -0.00 时 VB6 不带负号（实测 `Format(-0.004, "0.00")` 是 `0.00`）。
    let all_zero = !text.chars().any(|c| c.is_ascii_digit() && c != '0');
    if negative_sign && number < 0.0 && !all_zero {
        text.insert(0, '-');
    }
    Ok(text)
}

/// `Format` 的命名日期 / 时间格式（大小写不敏感），返回 `None` 表示不是这类格式。
///
/// 输出固定、不读系统区域设置，取本机 VB6（中文区域）实测的写法：`Short Date` 是
/// `2024/2/29`、`Medium Date` 是 `24-02-29`、`Long Date` 是 `2024年2月29日`、
/// `Medium Time` 是 12 小时制 + 上午/下午。
pub(super) fn named_date_format(value: &Value, spec: &str) -> Result<Option<String>, String> {
    const NAMES: &[&str] = &[
        "general date",
        "long date",
        "medium date",
        "short date",
        "long time",
        "medium time",
        "short time",
    ];
    let key = spec.trim().to_ascii_lowercase();
    if !NAMES.contains(&key.as_str()) {
        return Ok(None);
    }
    let serial = value.coerce(VbType::Date)?.as_float().map_err(|message| {
        format!(
            "`Format` 需要日期，但得到 \"{}\"：{message}",
            value.to_display()
        )
    })?;
    let (year, month, day, hour, minute, second) = parts_from_serial(serial);
    let text = match key.as_str() {
        "general date" => format_date_display(serial),
        "long date" => format!("{year}年{month}月{day}日"),
        "medium date" => format!("{:02}-{:02}-{:02}", year.rem_euclid(100), month, day),
        "short date" => format_date(serial, "yyyy/m/d"),
        "long time" => format!("{hour:02}:{minute:02}:{second:02}"),
        "medium time" => {
            let (hour12, half) = match hour {
                0 => (12, "上午"),
                1..=11 => (hour, "上午"),
                12 => (12, "下午"),
                _ => (hour - 12, "下午"),
            };
            format!("{hour12:02}:{minute:02} {half}")
        }
        _ => format!("{hour:02}:{minute:02}"),
    };
    Ok(Some(text))
}

/// `Format` 的命名格式（大小写不敏感），返回 `None` 表示不是命名格式。
///
/// 命名格式不读系统区域设置：`Currency` 固定用 `￥`（本机 VB6 中文系统实测是 `￥1.50`），
/// 千位分隔符固定用 `,`。
pub(super) fn named_format(value: &Value, spec: &str) -> Result<Option<String>, String> {
    let key = spec.trim().to_ascii_lowercase();
    let number = |spec: &str| -> Result<Option<String>, String> {
        let number = value
            .as_float()
            .map_err(|_| format!("`Format` 需要数值，但得到 \"{}\"", value.to_display()))?;
        Ok(Some(format_number(number, spec)?))
    };
    match key.as_str() {
        "general number" => Ok(Some(value.to_display())),
        "fixed" => number("0.00"),
        "standard" => number("#,##0.00"),
        "percent" => number("0.00%"),
        "scientific" => number("0.00E+00"),
        "currency" => number("￥#,##0.00"),
        "yes/no" => Ok(Some(choice_text(value, "No", "Yes")?)),
        "true/false" => Ok(Some(choice_text(value, "False", "True")?)),
        "on/off" => Ok(Some(choice_text(value, "Off", "On")?)),
        _ => Ok(None),
    }
}

/// `Format` 的 `Yes/No`、`True/False`、`On/Off`：0 取第一个词，非 0 取第二个。
pub(super) fn choice_text(value: &Value, zero: &str, nonzero: &str) -> Result<String, String> {
    Ok(if value.as_float()? == 0.0 {
        zero.to_string()
    } else {
        nonzero.to_string()
    })
}

/// 按单个数字格式段（`#,##0.00`）格式化一个非负数。
pub(super) fn format_section(value: f64, section: &str) -> Result<String, String> {
    if section.is_empty() {
        return Ok(String::new());
    }
    let chars = parse_format_chars(section);

    // 本机 VB6 实测：段首的 `!` 让这一段的数字填充失效——整段按字面量输出，`!` 本身丢掉。
    if chars
        .first()
        .is_some_and(|item| !item.literal && item.ch == '!')
    {
        return Ok(chars
            .iter()
            .filter(|item| item.ch != '!')
            .map(|item| item.ch)
            .collect());
    }

    let placeholder = |item: &FormatChar| !item.literal && matches!(item.ch, '0' | '#');
    let decimal = chars
        .iter()
        .position(|item| !item.literal && item.ch == '.');
    let int_end = decimal.unwrap_or(chars.len());
    let frac_start = decimal.map_or(chars.len(), |index| index + 1);
    let int_placeholders: Vec<usize> = (0..int_end).filter(|&i| placeholder(&chars[i])).collect();
    let frac_placeholders: Vec<usize> = (frac_start..chars.len())
        .filter(|&i| placeholder(&chars[i]))
        .collect();
    if int_placeholders.is_empty() && frac_placeholders.is_empty() {
        return Ok(chars.iter().map(|item| item.ch).collect());
    }

    // 指数写法（`0.00E+00`）：尾数交给 `render_scientific`，前面的字面量原样保留。
    // `E` 可能在小数点后面，所以要在整段里找。
    if let Some(exponent) = chars
        .iter()
        .position(|item| !item.literal && matches!(item.ch, 'E' | 'e'))
    {
        return render_section_scientific(value, &chars, exponent);
    }

    let last_placeholder = frac_placeholders
        .last()
        .or(int_placeholders.last())
        .copied();
    // `%` 只有出现在最后一个占位符后面才乘 100（`%0` 不乘，本机 VB6 实测）。
    let percent = last_placeholder.is_some_and(|last| {
        chars[last + 1..]
            .iter()
            .any(|item| !item.literal && item.ch == '%')
    });
    let value = if percent { value * 100.0 } else { value };

    // 整数占位符后面、小数点之前的每个逗号表示除以 1000（`Format(1234567, "0,")` 是 `1235`）。
    let mut scale = 0u32;
    if let Some(last) = int_placeholders.last() {
        for item in &chars[last + 1..int_end] {
            if !item.literal && item.ch == ',' {
                scale += 1;
            }
        }
    }
    let value = if scale > 0 {
        value / 1000f64.powi(scale as i32)
    } else {
        value
    };

    // 整数占位符之间的逗号是千位分组（`#,##0` 对 1234567 是 `1,234,567`）。
    let grouped = match (int_placeholders.first(), int_placeholders.last()) {
        (Some(first), Some(last)) => chars[*first..=*last]
            .iter()
            .any(|item| !item.literal && item.ch == ','),
        _ => false,
    };

    let decimals = frac_placeholders.len();
    let (integer_digits, fraction_digits) = round_decimal(value, decimals);
    let min_integer = int_placeholders
        .iter()
        .filter(|&&index| chars[index].ch == '0')
        .count();
    let mut digits = integer_digits;
    if digits == "0" && min_integer == 0 {
        // `#` 没有最少位数时 0 不显示（`Format(0, "#.##")` 是 `.`）。
        digits.clear();
    }
    while digits.chars().count() < min_integer {
        digits.insert(0, '0');
    }

    let mut output = String::with_capacity(section.len() + 4);
    if grouped {
        // 分组时按整串数字每三位插逗号：格式串里的逗号位置与占位符个数都不再限制位数。
        let first = int_placeholders.first().copied().unwrap_or(0);
        let last = int_placeholders.last().copied().unwrap_or(first);
        push_literals(&mut output, &chars[..first]);
        if !digits.is_empty() {
            output.push_str(&group_thousands(&digits));
        }
        push_literals(&mut output, &chars[last + 1..int_end]);
    } else {
        push_positional_section(
            &mut output,
            &chars,
            &int_placeholders,
            &frac_placeholders,
            decimal,
            &digits,
            &fraction_digits,
        );
        return Ok(output);
    }

    if decimal.is_some() {
        output.push('.');
        push_fraction_literals(
            &mut output,
            &chars,
            frac_start,
            &frac_placeholders,
            &fraction_digits,
        );
    }
    Ok(output)
}

/// 格式串里的一个字符：`literal` 表示它被 `\` 或 `"` 转义过，不参与占位符解释。
#[derive(Clone, Copy)]
pub(super) struct FormatChar {
    ch: char,
    literal: bool,
}

/// 拆出格式串的字符并处理转义：`\x` 与 `"…"` 里的字符按字面量输出。
pub(super) fn parse_format_chars(section: &str) -> Vec<FormatChar> {
    let mut chars = Vec::with_capacity(section.chars().count());
    let mut iter = section.chars();
    while let Some(ch) = iter.next() {
        match ch {
            '\\' => {
                if let Some(next) = iter.next() {
                    chars.push(FormatChar {
                        ch: next,
                        literal: true,
                    });
                }
            }
            '"' => {
                for inner in iter.by_ref() {
                    if inner == '"' {
                        break;
                    }
                    chars.push(FormatChar {
                        ch: inner,
                        literal: true,
                    });
                }
            }
            _ => chars.push(FormatChar { ch, literal: false }),
        }
    }
    chars
}

/// 没有占位符时的 `Format`：整串按字面量输出（段首 `!` 的怪癖与数值格式一致）。
pub(super) fn format_literal(spec: &str) -> String {
    parse_format_chars(spec)
        .iter()
        .filter(|item| item.ch != '!')
        .map(|item| item.ch)
        .collect()
}

/// 原样输出一批字符（转义已处理）；未转义的逗号只用于分组/缩放，不显示。
pub(super) fn push_literals(output: &mut String, chars: &[FormatChar]) {
    for item in chars {
        if !item.literal && item.ch == ',' {
            continue;
        }
        output.push(item.ch);
    }
}

/// 科学计数法的分段渲染：`0.00E+00` 前面的字面量、尾数、指数与后面的字面量。
pub(super) fn render_section_scientific(
    value: f64,
    chars: &[FormatChar],
    exponent: usize,
) -> Result<String, String> {
    let placeholder = |item: &FormatChar| !item.literal && matches!(item.ch, '0' | '#');
    let first = (0..exponent)
        .find(|&i| placeholder(&chars[i]))
        .ok_or_else(|| "`Format` 的指数格式缺少数字占位符".to_string())?;
    let last = (0..exponent)
        .rfind(|&i| placeholder(&chars[i]))
        .unwrap_or(first);
    let mantissa: String = chars[first..=last]
        .iter()
        .filter(|item| placeholder(item) || (!item.literal && matches!(item.ch, '.' | ',')))
        .map(|item| item.ch)
        .collect();
    let spec_text: String = chars[exponent..].iter().map(|item| item.ch).collect();
    let spec = exponent_spec(&spec_text)?;
    let (int_format, frac_format) = match mantissa.split_once('.') {
        Some((int, frac)) => (int, frac),
        None => (mantissa.as_str(), ""),
    };
    let (integer, fraction, exponent_text) =
        render_scientific(value, int_format, frac_format, &spec);

    let mut output = String::with_capacity(chars.len() + 8);
    push_literals(&mut output, &chars[..first]);
    output.push_str(&integer);
    if mantissa.contains('.') {
        output.push('.');
        output.push_str(&fraction);
    }
    output.push_str(&exponent_text);
    Ok(output)
}

/// 不做千位分组时的逐字符排版：整数占位符右对齐填数字，小数占位符左对齐，
/// 其它字符按位置原样输出（逗号只用于分组/缩放，不直接显示）。
pub(super) fn push_positional_section(
    output: &mut String,
    chars: &[FormatChar],
    int_placeholders: &[usize],
    frac_placeholders: &[usize],
    decimal: Option<usize>,
    digits: &str,
    fraction_digits: &str,
) {
    let total_int = int_placeholders.len();
    let digit_count = digits.chars().count();
    // 位数比占位符多时，多出来的数字接在最左边那个占位符前面。
    let extra = digit_count.saturating_sub(total_int);
    let fraction_limit = {
        let min = frac_placeholders
            .iter()
            .filter(|&&index| chars[index].ch == '0')
            .count();
        fraction_digits
            .trim_end_matches('0')
            .chars()
            .count()
            .max(min)
    };
    let mut int_index = 0usize;
    let mut frac_index = 0usize;
    for (index, item) in chars.iter().enumerate() {
        if Some(index) == decimal {
            // 没有整数占位符（`.00`）时整数部分也要显示（`Format(123, ".00")` 是 `123.00`），
            // 但值为 0 时 `digits` 已经是空的。
            if int_placeholders.is_empty() {
                output.push_str(digits);
            }
            output.push('.');
            continue;
        }
        if int_index < int_placeholders.len() && int_placeholders[int_index] == index {
            if int_index == 0 && extra > 0 {
                output.push_str(&digits.chars().take(extra).collect::<String>());
            }
            let digit_index = digit_count as isize - (total_int - int_index) as isize;
            if digit_index >= 0 && (digit_index as usize) < digit_count {
                output.push(digits.chars().nth(digit_index as usize).unwrap_or('0'));
            }
            int_index += 1;
            continue;
        }
        if frac_index < frac_placeholders.len() && frac_placeholders[frac_index] == index {
            if frac_index < fraction_limit && frac_index < fraction_digits.chars().count() {
                output.push(fraction_digits.chars().nth(frac_index).unwrap_or('0'));
            }
            frac_index += 1;
            continue;
        }
        if !item.literal && item.ch == ',' {
            continue;
        }
        output.push(item.ch);
    }
}

/// 千位分组时的小数部分：占位符左对齐填数字，字面量原样保留，逗号不显示。
pub(super) fn push_fraction_literals(
    output: &mut String,
    chars: &[FormatChar],
    start: usize,
    frac_placeholders: &[usize],
    fraction_digits: &str,
) {
    let min = frac_placeholders
        .iter()
        .filter(|&&index| chars[index].ch == '0')
        .count();
    let fraction_limit = fraction_digits
        .trim_end_matches('0')
        .chars()
        .count()
        .max(min);
    let mut frac_index = 0usize;
    for (index, item) in chars.iter().enumerate().skip(start) {
        if !item.literal && item.ch == ',' {
            continue;
        }
        if frac_index < frac_placeholders.len() && frac_placeholders[frac_index] == index {
            if frac_index < fraction_limit {
                output.push(fraction_digits.chars().nth(frac_index).unwrap_or('0'));
            }
            frac_index += 1;
            continue;
        }
        output.push(item.ch);
    }
}

/// 把非负数按 `0`/`#` 的位数渲染成 (整数部分, 小数部分)。
pub(super) fn render_fixed(value: f64, int_format: &str, frac_format: &str) -> (String, String) {
    let decimals = frac_format
        .chars()
        .filter(|ch| matches!(ch, '0' | '#'))
        .count();
    let min_decimals = frac_format.chars().filter(|ch| *ch == '0').count();
    let min_int = int_format.chars().filter(|ch| *ch == '0').count();
    let grouped = int_format.contains(',');

    // 与 VB6 一致：`Format` 逢五进位，而 `Round` 是银行家舍入。
    let (int_text, frac_text) = round_decimal(value, decimals);

    let mut integer = int_text.trim_start_matches('0').to_string();
    while integer.chars().count() < min_int {
        integer.insert(0, '0');
    }
    if grouped && !integer.is_empty() {
        integer = group_thousands(&integer);
    }

    let mut fraction = frac_text;
    while fraction.chars().count() > min_decimals && fraction.ends_with('0') {
        fraction.pop();
    }
    (integer, fraction)
}

/// 非负数四舍五入到 `decimals` 位小数，返回 (整数部分, 恰好 `decimals` 位的小数部分)。
///
/// 取舍在**最短往返十进制表示**上做：VB6 的 `Format(1.005, "0.00")` 是 `1.01`，
/// 而 f64 里的 1.005 略小于十进制的 1.005（直接乘 100 取整会得到 1.00）。
pub(super) fn round_decimal(value: f64, decimals: usize) -> (String, String) {
    if !value.is_finite() {
        return match format!("{value:.decimals$}").split_once('.') {
            Some((int, frac)) => (int.to_string(), frac.to_string()),
            None => (value.to_string(), String::new()),
        };
    }
    // Rust 的 `Display` 给的是最短往返表示，且不用指数写法。
    let text = format!("{value}");
    let (int_text, frac_text) = match text.split_once('.') {
        Some((int, frac)) => (int, frac),
        None => (text.as_str(), ""),
    };
    let mut point = int_text.chars().count();
    let mut digits: Vec<u8> = int_text
        .bytes()
        .chain(frac_text.bytes())
        .map(|byte| byte - b'0')
        .collect();
    if frac_text.len() > decimals {
        let round_up = digits[point + decimals] >= 5;
        digits.truncate(point + decimals);
        if round_up {
            let mut index = digits.len();
            loop {
                if index == 0 {
                    digits.insert(0, 1);
                    point += 1;
                    break;
                }
                index -= 1;
                if digits[index] == 9 {
                    digits[index] = 0;
                } else {
                    digits[index] += 1;
                    break;
                }
            }
        }
    } else {
        digits.resize(point + decimals, 0);
    }
    let text_of = |slice: &[u8]| slice.iter().map(|digit| (b'0' + digit) as char).collect();
    (text_of(&digits[..point]), text_of(&digits[point..]))
}

/// 科学计数法格式里 `E`/`e` 后面的写法：`E+00` 表示总带符号、至少两位指数。
pub(super) struct ExponentSpec {
    marker: char,
    always_sign: bool,
    width: usize,
}

pub(super) fn exponent_spec(text: &str) -> Result<ExponentSpec, String> {
    let marker = text.chars().next().unwrap_or('E');
    let rest: String = text.chars().skip(1).collect();
    let (always_sign, digits) = match rest.strip_prefix('+') {
        Some(digits) => (true, digits),
        None => (false, rest.strip_prefix('-').unwrap_or(&rest)),
    };
    if digits.is_empty() || digits.chars().any(|ch| ch != '0') {
        return Err(format!("`Format` 不支持指数写法 \"{text}\""));
    }
    Ok(ExponentSpec {
        marker,
        always_sign,
        width: digits.chars().count(),
    })
}

/// 科学计数法：返回 (尾数整数部分, 尾数小数部分, 指数文本)。
pub(super) fn render_scientific(
    value: f64,
    int_format: &str,
    frac_format: &str,
    spec: &ExponentSpec,
) -> (String, String, String) {
    let mut exponent = if value == 0.0 {
        0
    } else {
        value.log10().floor() as i32
    };
    let mut scaled = if value == 0.0 {
        0.0
    } else {
        value / 10f64.powi(exponent)
    };
    let (mut integer, mut fraction) = render_fixed(scaled, int_format, frac_format);
    // 四舍五入后尾数可能进位到 10，需要把指数提高一位再算。
    if integer.trim_start_matches('0').chars().count() > 1 {
        exponent += 1;
        scaled /= 10.0;
        let rendered = render_fixed(scaled, int_format, frac_format);
        integer = rendered.0;
        fraction = rendered.1;
    }
    let sign = if exponent < 0 {
        "-"
    } else if spec.always_sign {
        "+"
    } else {
        ""
    };
    let digits = format!("{:0width$}", exponent.abs(), width = spec.width);
    (integer, fraction, format!("{}{sign}{digits}", spec.marker))
}

/// 从右往左每三位插入千位分隔符。
pub(super) fn group_thousands(digits: &str) -> String {
    let total = digits.chars().count();
    let mut output = String::with_capacity(total + total / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (total - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(ch);
    }
    output
}
