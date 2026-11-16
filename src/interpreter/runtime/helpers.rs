//! `Runtime` 用到的自由函数：类型提升、值比较、`Input` 解析、`Like` 匹配等。
//! 都不依赖运行时状态，拆出来让 `mod.rs` 专注于执行逻辑。

use super::*;

pub(super) fn stmt_line(stmt: &Stmt) -> usize {
    match stmt {
        Stmt::Dim { line, .. }
        | Stmt::Redim { line, .. }
        | Stmt::Erase { line, .. }
        | Stmt::Const { line, .. }
        | Stmt::Assign { line, .. }
        | Stmt::AssignIndex { line, .. }
        | Stmt::AssignMember { line, .. }
        | Stmt::AssignMemberIndex { line, .. }
        | Stmt::AssignWithMember { line, .. }
        | Stmt::WithCall { line, .. }
        | Stmt::OnError { line, .. }
        | Stmt::Resume { line, .. }
        | Stmt::ErrClear { line }
        | Stmt::ErrRaise { line, .. }
        | Stmt::With { line, .. }
        | Stmt::Label(_, line)
        | Stmt::Goto(_, line)
        | Stmt::Gosub(_, line)
        | Stmt::GosubReturn { line }
        | Stmt::OpenFile { line, .. }
        | Stmt::CloseFile { line, .. }
        | Stmt::PrintFile { line, .. }
        | Stmt::LineInput { line, .. }
        | Stmt::InputFile { line, .. }
        | Stmt::GetFile { line, .. }
        | Stmt::PutFile { line, .. }
        | Stmt::SeekFile { line, .. }
        | Stmt::WidthFile { line, .. }
        | Stmt::WriteFile { line, .. }
        | Stmt::MidAssign { line, .. }
        | Stmt::RaiseEvent { line, .. }
        | Stmt::LockFile { line, .. }
        | Stmt::Set { line, .. }
        | Stmt::CallMethod { line, .. }
        | Stmt::Call { line, .. }
        | Stmt::Eval { line, .. }
        | Stmt::Print { line, .. }
        | Stmt::If { line, .. }
        | Stmt::For { line, .. }
        | Stmt::ForEach { line, .. }
        | Stmt::DoLoop { line, .. }
        | Stmt::While { line, .. }
        | Stmt::Select { line, .. }
        | Stmt::Exit { line, .. }
        | Stmt::Return { line, .. }
        | Stmt::Halt { line } => *line,
    }
}

/// 事件实参按处理过程的形参个数对齐：多余的丢掉、缺少的补 `Empty`
/// （`WithEvents` 的处理过程可以只接自己关心的前几个参数）。
pub(super) fn actions_pad_values(values: &mut Vec<Value>, params: usize) {
    values.truncate(params);
    while values.len() < params {
        values.push(Value::Empty);
    }
}

/// 在同一个语句块里查找标签位置。
pub(super) fn find_label(stmts: &[Stmt], label: &str) -> Option<usize> {
    stmts
        .iter()
        .position(|stmt| matches!(stmt, Stmt::Label(name, _) if name == label))
}

pub(super) fn flush_file(handle: FileHandle) {
    if let FileHandle::Writer(mut writer) = handle {
        let _ = writer.sink.flush();
    }
}

/// 转换函数（`CByte` / `CInt` …）的结果声明类型：`Put #` 用它决定写几个字节。
/// 数值类型提升（本机 VB6 实测）：Byte+Byte→Byte、Byte+Integer→Integer、Integer+Long→Long、
/// Integer+Single→Single、Long+Single→Double、Single+Double→Double、Currency 吸收 Single/Double、
/// Date 吸收所有数值（日期算术）。
pub(super) fn promote_numeric(left: VbType, right: VbType) -> Option<VbType> {
    pub(super) fn rank(ty: VbType) -> Option<u8> {
        Some(match ty {
            VbType::Byte => 0,
            VbType::Integer | VbType::Boolean => 1,
            VbType::Long => 2,
            VbType::LongLong | VbType::LongPtr => 3,
            VbType::Single => 4,
            VbType::Double => 5,
            VbType::Currency => 6,
            VbType::Date => 7,
            _ => return None,
        })
    }
    let (left_rank, right_rank) = (rank(left)?, rank(right)?);
    // Long/LongLong 与 Single 混算退到 Double（Integer + Single 仍是 Single）。
    let single_with_long = |single: u8, other: u8| single == 4 && (2..=3).contains(&other);
    if single_with_long(left_rank, right_rank) || single_with_long(right_rank, left_rank) {
        return Some(VbType::Double);
    }
    Some(match left_rank.max(right_rank) {
        0 => VbType::Byte,
        1 => VbType::Integer,
        2 => VbType::Long,
        3 => VbType::LongLong,
        4 => VbType::Single,
        5 => VbType::Double,
        6 => VbType::Currency,
        _ => VbType::Date,
    })
}

/// VB6 的整数运算溢出（错误号 6）判定的宽度：只有「两个操作数都装得进这个类型、结果却装不进去」时
/// 才需要去查静态类型确认（本机 VB6 实测：`32767 + 1`、`32767 * 2`、`2147483647 + 1`、
/// `CByte(200) + CByte(200)` 都报 6；`Dim c As Long: c + c`（30000）不报、`200 + 200` 不报）。
pub(super) fn overflow_candidate(left: i64, right: i64, result: i64) -> Option<VbType> {
    let in_i32 = |value: i64| (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&value);
    // 快路径：结果落在 `0..=255` 里就既不会 `Byte` 溢出也不会 `Integer`/`Long` 溢出
    // （热循环里的计数与累加绝大多数都走这一条，只做一次比较）。
    if fits_byte(result) {
        return None;
    }
    // `Byte`：两个操作数都装得进 Byte、结果装不进去时给候选（实测 `CByte(200) + CByte(200)` 报 6）。
    if fits_byte(left) && fits_byte(right) {
        return Some(VbType::Byte);
    }
    // 结果装得进 Integer 就不会溢出。
    if fits_i16(result) {
        return None;
    }
    if !in_i32(result) && in_i32(left) && in_i32(right) {
        return Some(VbType::Long);
    }
    if fits_i16(left) && fits_i16(right) {
        return Some(VbType::Integer);
    }
    None
}

pub(super) fn fits_i16(value: i64) -> bool {
    i16::try_from(value).is_ok()
}

pub(super) fn fits_byte(value: i64) -> bool {
    u8::try_from(value).is_ok()
}

pub(super) fn conversion_type(name: &str) -> Option<VbType> {
    Some(match name {
        "cbyte" => VbType::Byte,
        "cint" => VbType::Integer,
        "clng" => VbType::Long,
        "csng" => VbType::Single,
        "cdbl" => VbType::Double,
        "ccur" => VbType::Currency,
        "cdate" => VbType::Date,
        "cbool" => VbType::Boolean,
        "cstr" => VbType::Str,
        _ => return None,
    })
}

/// 整数类型的取值范围（`Variant` 等没有固定范围，返回 `None`）。
pub(super) fn integer_bounds(ty: VbType) -> Option<(i64, i64)> {
    match ty {
        VbType::Byte => Some((0, 255)),
        VbType::Integer => Some((i16::MIN as i64, i16::MAX as i64)),
        VbType::Long => Some((i32::MIN as i64, i32::MAX as i64)),
        _ => None,
    }
}

/// 把 IO 错误映射成 VB 错误号。
pub(super) fn io_error_number(error: &std::io::Error) -> i64 {
    match error.kind() {
        std::io::ErrorKind::NotFound => 53,
        std::io::ErrorKind::PermissionDenied => 70,
        std::io::ErrorKind::AlreadyExists => 58,
        std::io::ErrorKind::UnexpectedEof => 62,
        _ => 57,
    }
}

/// 打开文件时的错误号：父目录不存在是 VB 的 76（路径未找到），目录在但文件不在才是 53。
pub(super) fn open_error_number(error: &std::io::Error, path: &Path) -> i64 {
    if error.kind() == std::io::ErrorKind::NotFound
        && let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return 76;
    }
    io_error_number(error)
}

/// 目标文件号不能用时的错误：没打开是 VB 的 52（错误的文件名或号码），
/// 打开了但模式不对是 54（文件模式错误）。
pub(super) fn writer_error(
    files: &HashMap<i32, FileHandle>,
    number: i32,
    line: usize,
) -> RuntimeError {
    if files.contains_key(&number) {
        RuntimeError::new(format!("文件号 #{number} 不是以 Output/Append 打开"), line)
            .with_number(54)
    } else {
        RuntimeError::new(format!("文件号 #{number} 没有打开"), line).with_number(52)
    }
}

/// `Input #` 的一行：按逗号切分，支持双引号包住的字符串。
pub(super) fn split_input_fields(text: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' if !in_quotes => in_quotes = true,
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            ',' if !in_quotes => {
                fields.push(current.trim().to_string());
                current.clear();
            }
            '\r' | '\n' if !in_quotes => {
                if !current.trim().is_empty() {
                    fields.push(current.trim().to_string());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() || fields.is_empty() {
        fields.push(current.trim().to_string());
    }
    fields
}

pub(super) fn parse_input_value(text: &str) -> Value {
    let text = text.trim();
    if text.is_empty() {
        return Value::Empty;
    }
    // `Write #` 写出的 `#…#` 字段：布尔、`#NULL#` 或日期。
    if let Some(inner) = text
        .strip_prefix('#')
        .and_then(|rest| rest.strip_suffix('#'))
    {
        let inner = inner.trim();
        if inner.eq_ignore_ascii_case("true") {
            return Value::Bool(true);
        }
        if inner.eq_ignore_ascii_case("false") {
            return Value::Bool(false);
        }
        if inner.eq_ignore_ascii_case("null") {
            return Value::Null;
        }
        if let Some(serial) = builtins::parse_date_value(inner) {
            return Value::Date(serial);
        }
        return Value::Str(inner.to_string());
    }
    if let Ok(value) = text.parse::<i64>() {
        return Value::Int(value);
    }
    if let Ok(value) = text.parse::<f64>() {
        return Value::Float(value);
    }
    Value::Str(text.to_string())
}

pub(super) fn number_value(value: f64) -> Value {
    if value == value.trunc() && value.abs() < 9e15 {
        Value::Int(value as i64)
    } else {
        Value::Float(value)
    }
}

/// 值是否恰好是整数（`For` 循环用它选中整数路径）。
pub(super) fn exact_int(value: &Value) -> Option<i64> {
    match value {
        Value::Int(value) => Some(*value),
        Value::Empty => Some(0),
        Value::Bool(value) => Some(if *value { -1 } else { 0 }),
        Value::Currency(value) if value % 10_000 == 0 => Some(value / 10_000),
        Value::Float(value) if value.fract() == 0.0 && value.abs() < 9e15 => Some(*value as i64),
        _ => None,
    }
}

/// 字符串拼接：两侧都是字符串时直接拷贝，省掉 `to_display` 的临时副本。
pub(super) fn concat_text(left: &Value, right: &Value) -> String {
    // VB 的 `&` 把 `Null` 当成空串（`"a" & Null` 是 `"a"`，与 `+` 传播 Null 不同）。
    let display = |value: &Value| match value {
        Value::Null => String::new(),
        other => other.to_display(),
    };
    let (left, right) = match (left, right) {
        (Value::Str(left), Value::Str(right)) => {
            return {
                let mut text = String::with_capacity(left.len() + right.len());
                text.push_str(left);
                text.push_str(right);
                text
            };
        }
        _ => (display(left), display(right)),
    };
    let mut text = String::with_capacity(left.len() + right.len());
    text.push_str(&left);
    text.push_str(&right);
    text
}

pub(super) fn compare_values(
    left: &Value,
    right: &Value,
    compare_text: bool,
) -> Result<Ordering, String> {
    if let (Value::Currency(left), Value::Currency(right)) = (left, right) {
        return Ok(left.cmp(right));
    }
    let left_numeric = left.is_numeric_like() || matches!(left, Value::Date(_));
    let right_numeric = right.is_numeric_like() || matches!(right, Value::Date(_));
    if left_numeric && right_numeric {
        let left = left.as_float()?;
        let right = right.as_float()?;
        Ok(left.partial_cmp(&right).unwrap_or(Ordering::Equal))
    } else {
        // 两个字符串直接比较，省掉两次 `to_display` 拷贝。
        if let (Value::Str(left), Value::Str(right)) = (left, right) {
            return Ok(compare_text_strings(left, right, compare_text));
        }
        let left = left.to_display();
        let right = right.to_display();
        Ok(compare_text_strings(&left, &right, compare_text))
    }
}

/// 比较两段文本：`Option Compare Text` 下忽略大小写（整串转小写后比较）。
pub(super) fn compare_text_strings(left: &str, right: &str, compare_text: bool) -> Ordering {
    if compare_text {
        left.to_lowercase().cmp(&right.to_lowercase())
    } else {
        left.cmp(right)
    }
}

pub(super) fn currency_scaled(value: &Value) -> Result<i64, String> {
    match value {
        Value::Currency(value) => Ok(*value),
        Value::Int(value) => value
            .checked_mul(10_000)
            .ok_or_else(|| "Currency 溢出".into()),
        other => other.as_currency(),
    }
}

pub(super) fn currency_ratio(numerator: i128, denominator: i128) -> Result<i64, String> {
    if denominator == 0 {
        return Err("除数为零".into());
    }
    let negative = (numerator < 0) ^ (denominator < 0);
    let numerator = numerator.unsigned_abs();
    let denominator = denominator.unsigned_abs();
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let rounded =
        if remainder * 2 > denominator || (remainder * 2 == denominator && quotient % 2 == 1) {
            quotient + 1
        } else {
            quotient
        };
    let signed = if negative {
        -(rounded as i128)
    } else {
        rounded as i128
    };
    i64::try_from(signed).map_err(|_| "Currency 溢出".into())
}

/// `a Is b`：对象同一性判断（`Nothing` 视作空）。
pub(super) fn objects_identical(left: &Value, right: &Value) -> Result<bool, String> {
    Ok(match (left, right) {
        (Value::Nothing, Value::Nothing) => true,
        // `Empty` 不是对象：本机 VB6 实测 `Dim v: v Is Nothing` 报 424。
        (Value::Empty, _) | (_, Value::Empty) => {
            return Err("`Is` 要求两侧都是对象（Empty 不是对象）".into());
        }
        (Value::Nothing, _) | (_, Value::Nothing) => false,
        (Value::Collection(a), Value::Collection(b)) => Rc::ptr_eq(a, b),
        (Value::Dictionary(a), Value::Dictionary(b)) => Rc::ptr_eq(a, b),
        (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(&a.0, &b.0),
        (Value::ComObject(a), Value::ComObject(b)) => a == b,
        (Value::Picture(a), Value::Picture(b)) => Rc::ptr_eq(a, b),
        _ => return Err("`Is` 只能比较对象与 `Nothing`".into()),
    })
}

/// 从文本文件里读一个字符（`Input(n, #f)` 用）：按 UTF-8 首字节决定长度。
pub(super) fn read_char(reader: &mut BufReader<File>) -> std::io::Result<Option<char>> {
    use std::io::BufRead as _;
    let buffer = reader.fill_buf()?;
    if buffer.is_empty() {
        return Ok(None);
    }
    let first = buffer[0];
    let width = if first < 0x80 {
        1
    } else if first >= 0xF0 {
        4
    } else if first >= 0xE0 {
        3
    } else if first >= 0xC0 {
        2
    } else {
        1
    };
    let width = width.min(buffer.len());
    let bytes = buffer[..width].to_vec();
    reader.consume(width);
    Ok(String::from_utf8_lossy(&bytes).chars().next())
}

/// `Write #` 的单个字段：字符串加引号（内部 `"` 双写），布尔/Null 写成 `#…#`。
pub(super) fn write_field(value: &Value) -> Result<String, String> {
    Ok(match value {
        Value::Empty => String::new(),
        Value::Null => "#NULL#".into(),
        Value::Bool(true) => "#True#".into(),
        Value::Bool(false) => "#False#".into(),
        Value::Str(text) => format!("\"{}\"", text.replace('"', "\"\"")),
        Value::Int(_) | Value::Float(_) => value.to_display(),
        Value::Date(_) => format!("#{}#", value.to_display()),
        other => {
            return Err(format!("`Write #` 不支持 {} 类型的值", other.type_name()));
        }
    })
}

#[derive(Clone)]
pub(super) enum LikeToken {
    AnySequence,
    AnyCharacter,
    Digit,
    Literal(char),
    CharacterClass {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

pub(super) fn like_match(value: &str, pattern: &str, compare_text: bool) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < pattern.len() {
        match pattern[index] {
            '*' => {
                if !matches!(tokens.last(), Some(LikeToken::AnySequence)) {
                    tokens.push(LikeToken::AnySequence);
                }
                index += 1;
            }
            '?' => {
                tokens.push(LikeToken::AnyCharacter);
                index += 1;
            }
            '#' => {
                tokens.push(LikeToken::Digit);
                index += 1;
            }
            '[' => {
                let Some(end) = pattern[index + 1..].iter().position(|ch| *ch == ']') else {
                    tokens.push(LikeToken::Literal('['));
                    index += 1;
                    continue;
                };
                let end = index + 1 + end;
                let mut content = &pattern[index + 1..end];
                let negated = content.first() == Some(&'!');
                if negated {
                    content = &content[1..];
                }
                let mut ranges = Vec::new();
                let mut cursor = 0;
                while cursor < content.len() {
                    if cursor + 2 < content.len() && content[cursor + 1] == '-' {
                        ranges.push((content[cursor], content[cursor + 2]));
                        cursor += 3;
                    } else {
                        ranges.push((content[cursor], content[cursor]));
                        cursor += 1;
                    }
                }
                if ranges.is_empty() {
                    tokens.push(LikeToken::Literal('['));
                    index += 1;
                } else {
                    tokens.push(LikeToken::CharacterClass { negated, ranges });
                    index = end + 1;
                }
            }
            literal => {
                tokens.push(LikeToken::Literal(literal));
                index += 1;
            }
        }
    }

    pub(super) fn equal_char(left: char, right: char, compare_text: bool) -> bool {
        if compare_text {
            left.to_lowercase().eq(right.to_lowercase())
        } else {
            left == right
        }
    }

    pub(super) fn matches(
        tokens: &[LikeToken],
        chars: &[char],
        token_index: usize,
        char_index: usize,
        compare_text: bool,
        memo: &mut HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(result) = memo.get(&(token_index, char_index)) {
            return *result;
        }
        let result = if token_index == tokens.len() {
            char_index == chars.len()
        } else {
            match &tokens[token_index] {
                LikeToken::AnySequence => {
                    matches(
                        tokens,
                        chars,
                        token_index + 1,
                        char_index,
                        compare_text,
                        memo,
                    ) || (char_index < chars.len()
                        && matches(
                            tokens,
                            chars,
                            token_index,
                            char_index + 1,
                            compare_text,
                            memo,
                        ))
                }
                _ if char_index >= chars.len() => false,
                LikeToken::AnyCharacter => matches(
                    tokens,
                    chars,
                    token_index + 1,
                    char_index + 1,
                    compare_text,
                    memo,
                ),
                LikeToken::Digit => {
                    chars[char_index].is_ascii_digit()
                        && matches(
                            tokens,
                            chars,
                            token_index + 1,
                            char_index + 1,
                            compare_text,
                            memo,
                        )
                }
                LikeToken::Literal(expected) => {
                    equal_char(chars[char_index], *expected, compare_text)
                        && matches(
                            tokens,
                            chars,
                            token_index + 1,
                            char_index + 1,
                            compare_text,
                            memo,
                        )
                }
                LikeToken::CharacterClass { negated, ranges } => {
                    let ch = chars[char_index];
                    let contained = ranges.iter().any(|(start, end)| {
                        let (ch, start, end) = if compare_text {
                            (
                                ch.to_lowercase().next().unwrap_or(ch),
                                start.to_lowercase().next().unwrap_or(*start),
                                end.to_lowercase().next().unwrap_or(*end),
                            )
                        } else {
                            (ch, *start, *end)
                        };
                        start <= ch && ch <= end
                    });
                    (contained != *negated)
                        && matches(
                            tokens,
                            chars,
                            token_index + 1,
                            char_index + 1,
                            compare_text,
                            memo,
                        )
                }
            }
        };
        memo.insert((token_index, char_index), result);
        result
    }

    matches(
        &tokens,
        &value.chars().collect::<Vec<_>>(),
        0,
        0,
        compare_text,
        &mut HashMap::new(),
    )
}

pub(super) fn apply_binary(
    op: BinOp,
    left: Value,
    right: Value,
    line: usize,
    compare_text: bool,
    overflow: &mut Option<VbType>,
) -> Result<Value, RuntimeError> {
    let fail = |message: String| RuntimeError::from_coerce(message, line);

    // VB6 实测：`"x" & 对象` 报 450（对象没有可用的默认属性）。
    if matches!(op, BinOp::Concat)
        && (matches!(left, Value::ComObject(_)) || matches!(right, Value::ComObject(_)))
    {
        return Err(
            RuntimeError::new("对象没有可用的默认属性，不能转换为字符串", line).with_number(450),
        );
    }

    // `Null` 参与算术或比较时结果仍是 `Null`（VB 的传播规则，`&` 例外：当成空串）。
    if matches!(
        op,
        BinOp::Add
            | BinOp::Subtract
            | BinOp::Multiply
            | BinOp::Divide
            | BinOp::IntDivide
            | BinOp::Modulo
            | BinOp::Power
            | BinOp::Eq
            | BinOp::NotEq
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::LtEq
            | BinOp::GtEq
            | BinOp::Like
    ) && (matches!(left, Value::Null) || matches!(right, Value::Null))
    {
        return Ok(Value::Null);
    }

    // 最常见的 `Long op Long`：先走整数快路径，省掉下面逐层 `matches!` 与 `as_int` 的 Result 开销。
    // 语义与后面的通用分支完全一致（溢出仍是错误号 6，除零仍是 11）。
    if let (Value::Int(left), Value::Int(right)) = (&left, &right)
        && matches!(
            op,
            BinOp::Add | BinOp::Subtract | BinOp::Multiply | BinOp::IntDivide | BinOp::Modulo
        )
    {
        let computed = match op {
            BinOp::Add => left.checked_add(*right),
            BinOp::Subtract => left.checked_sub(*right),
            BinOp::Multiply => left.checked_mul(*right),
            BinOp::IntDivide if *right == 0 => {
                return Err(fail("除数为零".into()).with_number(11));
            }
            BinOp::Modulo if *right == 0 => {
                return Err(fail("除数为零".into()).with_number(11));
            }
            BinOp::IntDivide => left.checked_div(*right),
            _ => left.checked_rem(*right),
        };
        let result = computed.ok_or_else(|| fail("数值溢出".into()).with_number(6))?;
        // Integer / Long 的溢出由调用方查静态类型确认（这里只记录「可能溢出」）。
        *overflow = overflow_candidate(*left, *right, result);
        return Ok(Value::Int(result));
    }

    Ok(match op {
        BinOp::Concat => Value::Str(concat_text(&left, &right)),
        BinOp::Add if matches!((&left, &right), (Value::Str(_), Value::Str(_))) => {
            Value::Str(concat_text(&left, &right))
        }
        BinOp::Is => Value::Bool(
            objects_identical(&left, &right).map_err(|message| fail(message).with_number(424))?,
        ),
        BinOp::Eq
        | BinOp::NotEq
        | BinOp::Lt
        | BinOp::Gt
        | BinOp::LtEq
        | BinOp::GtEq
        | BinOp::Like => {
            if op == BinOp::Like {
                Value::Bool(like_match(
                    &left.to_display(),
                    &right.to_display(),
                    compare_text,
                ))
            } else {
                let ordering = compare_values(&left, &right, compare_text).map_err(fail)?;
                Value::Bool(match op {
                    BinOp::Eq => ordering == Ordering::Equal,
                    BinOp::NotEq => ordering != Ordering::Equal,
                    BinOp::Lt => ordering == Ordering::Less,
                    BinOp::Gt => ordering == Ordering::Greater,
                    BinOp::LtEq => ordering != Ordering::Greater,
                    _ => ordering != Ordering::Less,
                })
            }
        }
        BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Eqv | BinOp::Imp => {
            if let (Value::Bool(left), Value::Bool(right)) = (&left, &right) {
                Value::Bool(match op {
                    BinOp::And => *left && *right,
                    BinOp::Or => *left || *right,
                    BinOp::Xor => *left ^ *right,
                    BinOp::Eqv => *left == *right,
                    BinOp::Imp => !*left || *right,
                    _ => unreachable!(),
                })
            } else {
                let left = left.as_int().map_err(fail)?;
                let right = right.as_int().map_err(fail)?;
                Value::Int(match op {
                    BinOp::And => left & right,
                    BinOp::Or => left | right,
                    BinOp::Xor => left ^ right,
                    BinOp::Eqv => !(left ^ right),
                    BinOp::Imp => (!left) | right,
                    _ => unreachable!(),
                })
            }
        }
        BinOp::Divide => {
            let divisor = right.as_float().map_err(fail)?;
            if divisor == 0.0 {
                return Err(fail("除数为零".into()).with_number(11));
            }
            Value::Float(left.as_float().map_err(fail)? / divisor)
        }
        BinOp::IntDivide => {
            let divisor = right.as_int().map_err(fail)?;
            if divisor == 0 {
                return Err(fail("除数为零".into()).with_number(11));
            }
            // `LongLong` 最小值除以 -1 会溢出，VB 报错误号 6 而不是崩溃。
            let quotient = left
                .as_int()
                .map_err(fail)?
                .checked_div(divisor)
                .ok_or_else(|| fail("数值溢出".into()).with_number(6))?;
            Value::Int(quotient)
        }
        BinOp::Modulo => {
            let divisor = right.as_int().map_err(fail)?;
            if divisor == 0 {
                return Err(fail("除数为零".into()).with_number(11));
            }
            let remainder = left
                .as_int()
                .map_err(fail)?
                .checked_rem(divisor)
                .ok_or_else(|| fail("数值溢出".into()).with_number(6))?;
            Value::Int(remainder)
        }
        BinOp::Power => Value::Float(
            left.as_float()
                .map_err(fail)?
                .powf(right.as_float().map_err(fail)?),
        ),
        BinOp::Add if matches!(left, Value::Date(_)) || matches!(right, Value::Date(_)) => {
            Value::Date(left.as_float().map_err(fail)? + right.as_float().map_err(fail)?)
        }
        BinOp::Subtract if matches!((&left, &right), (Value::Date(_), Value::Date(_))) => {
            Value::Float(left.as_float().map_err(fail)? - right.as_float().map_err(fail)?)
        }
        BinOp::Subtract if matches!(left, Value::Date(_)) => {
            Value::Date(left.as_float().map_err(fail)? - right.as_float().map_err(fail)?)
        }
        BinOp::Add | BinOp::Subtract | BinOp::Multiply => {
            if matches!(left, Value::Date(_))
                || matches!(right, Value::Date(_))
                || matches!(left, Value::Float(_))
                || matches!(right, Value::Float(_))
            {
                let left = left.as_float().map_err(fail)?;
                let right = right.as_float().map_err(fail)?;
                Value::Float(match op {
                    BinOp::Multiply => left * right,
                    BinOp::Subtract => left - right,
                    _ => left + right,
                })
            } else if matches!(left, Value::Currency(_)) || matches!(right, Value::Currency(_)) {
                let left = currency_scaled(&left).map_err(fail)?;
                let right = currency_scaled(&right).map_err(fail)?;
                let result = match op {
                    BinOp::Add => i128::from(left) + i128::from(right),
                    BinOp::Subtract => i128::from(left) - i128::from(right),
                    BinOp::Multiply => {
                        let product = i128::from(left) * i128::from(right);
                        return Ok(Value::Currency(
                            currency_ratio(product, 10_000)
                                .map_err(|message| fail(message).with_number(6))?,
                        ));
                    }
                    _ => unreachable!(),
                };
                Value::Currency(
                    i64::try_from(result)
                        .map_err(|_| fail("Currency 溢出".into()).with_number(6))?,
                )
            } else {
                let left = left.as_int().map_err(fail)?;
                let right = right.as_int().map_err(fail)?;
                let computed = match op {
                    BinOp::Multiply => left.checked_mul(right),
                    BinOp::Subtract => left.checked_sub(right),
                    _ => left.checked_add(right),
                };
                let result = computed.ok_or_else(|| fail("数值溢出".into()).with_number(6))?;
                // Integer / Long 的溢出由调用方查静态类型确认（这里只记录「可能溢出」）。
                *overflow = overflow_candidate(left, right, result);
                Value::Int(result)
            }
        }
    })
}
