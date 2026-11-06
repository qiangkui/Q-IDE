//! 内置函数：VB 标准库的一个实用子集 + 几个 Win32 便捷封装。

use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::interpreter::ffi;
use crate::interpreter::value::{Value, parse_number, round_half_even};

/// 内置函数执行环境。
pub struct Ctx<'a> {
    pub rng: &'a mut u64,
    pub started: Instant,
    /// 「停止脚本」的旗标：`InputBox` 这类会阻塞的内置函数要查它。
    pub interrupt: &'a AtomicBool,
    /// `Err.Number`，供无参数的 `Error()` 使用。
    pub err_number: i64,
    /// UDT 的字节数（`Len(x)`），由运行时注入。
    pub record_size: &'a dyn Fn(&str) -> Option<usize>,
    /// `Dir()` 的遍历状态。
    pub dir: &'a mut Option<DirState>,
    /// `Option Compare Text`：`StrComp`/`Filter` 不给比较方式时按它决定。
    pub compare_text: bool,
    /// 脚本运行时的命令行参数（`Command()`）。
    pub command: &'a str,
    /// 脚本视角的当前目录；相对路径按它解析，`ChDir` 修改它。
    pub cwd: &'a mut std::path::PathBuf,
    /// 内置函数失败时要用的 VB 错误号（找不到文件 53、路径错误 75 …）。
    pub failure: Option<i64>,
}

/// 把脚本里的路径变成实际路径：绝对路径原样返回，相对路径按脚本当前目录解析。
fn resolved(cwd: &std::path::Path, path: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// `Dir()` 的遍历状态：保存上一次匹配到的文件名列表与当前位置。
#[derive(Default)]
pub struct DirState {
    entries: Vec<String>,
    index: usize,
}

impl DirState {
    /// 取下一条；用完后返回空字符串（VB 里表示「没有了」）。
    fn next_entry(&mut self) -> String {
        let entry = self.entries.get(self.index).cloned().unwrap_or_default();
        self.index += 1;
        entry
    }
}

const BUILTINS: &[&str] = &[
    "msgbox",
    "inputbox",
    "shell",
    "command",
    "len",
    "mid",
    "left",
    "right",
    "ucase",
    "lcase",
    "trim",
    "ltrim",
    "rtrim",
    "instr",
    "instrrev",
    "replace",
    "chr",
    "asc",
    "val",
    "str",
    "cstr",
    "cint",
    "clng",
    "cdbl",
    "csng",
    "cbool",
    "abs",
    "int",
    "fix",
    "sgn",
    "sqr",
    "exp",
    "log",
    "round",
    "rnd",
    "hex",
    "oct",
    "space",
    "string",
    "timer",
    "sleep",
    "tickcount",
    "iif",
    "format",
    "split",
    "join",
    "array",
    "isarray",
    "isnumeric",
    "typename",
    "lbound",
    "ubound",
    "now",
    "date",
    "time",
    "dateserial",
    "timeserial",
    "dateadd",
    "datediff",
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "eof",
    "lof",
    "freefile",
    "error",
    "dir",
    "filelen",
    "filedatetime",
    "curdir",
    "kill",
    "filecopy",
    "mkdir",
    "rmdir",
    "chdir",
    "rename",
    "environ",
    "isempty",
    "isnull",
    "isobject",
    "isdate",
    "strreverse",
    "strcomp",
    "filter",
    "ascw",
    "chrw",
    "weekday",
    "weekdayname",
    "monthname",
    "datevalue",
    "timevalue",
    "randomize",
    "doevents",
];

pub fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

/// 全部内置函数名（小写），供编辑器补全使用。
pub fn names() -> &'static [&'static str] {
    BUILTINS
}

/// VB 错误号对应的说明文字（`Error()` 与 `Err.Raise` 的默认描述）。
pub fn error_text(number: i64) -> String {
    if number == 0 {
        return String::new();
    }
    let text = match number {
        3 => "无 GoSub 返回",
        5 => "无效的过程调用或参数",
        6 => "溢出",
        7 => "内存不足",
        9 => "下标越界",
        10 => "该数组为固定大小或临时锁定",
        11 => "除数为零",
        13 => "类型不匹配",
        14 => "字符串空间不足",
        17 => "无法执行所请求的操作",
        20 => "恢复时没有错误",
        28 => "堆栈空间不足",
        35 => "未定义的子过程或函数",
        48 => "加载 DLL 时出错",
        49 => "错误的 DLL 调用约定",
        51 => "内部错误",
        52 => "错误的文件名或文件号",
        53 => "找不到文件",
        54 => "错误的文件模式",
        55 => "文件已打开",
        57 => "设备 I/O 错误",
        58 => "文件已存在",
        59 => "错误的记录长度",
        61 => "磁盘已满",
        62 => "输入超出文件尾",
        63 => "错误的记录号",
        67 => "文件过多",
        68 => "设备不可用",
        70 => "权限被拒绝",
        71 => "磁盘未准备好",
        74 => "无法重命名文件",
        75 => "路径/文件访问错误",
        76 => "找不到路径",
        91 => "对象变量或 With 块变量未设置",
        94 => "无效使用 Null",
        424 => "需要对象",
        429 => "ActiveX 部件无法创建对象",
        438 => "对象不支持该属性或方法",
        449 => "参数不可选",
        450 => "参数个数错误或属性赋值无效",
        451 => "对象不是集合",
        _ => "应用程序定义或对象定义的错误",
    };
    text.to_string()
}

pub fn call(name: &str, args: Vec<Value>, ctx: &mut Ctx) -> Result<Value, String> {
    let text = |index: usize| -> Result<String, String> {
        args.get(index)
            .map(Value::to_display)
            .ok_or_else(|| format!("`{name}` 缺少第 {} 个参数", index + 1))
    };
    let number = |index: usize| -> Result<i64, String> {
        args.get(index)
            .ok_or_else(|| format!("`{name}` 缺少第 {} 个参数", index + 1))?
            .as_int()
    };

    Ok(match name {
        "msgbox" => {
            match args.first().map(Value::to_display) {
                None => Value::Empty,
                Some(prompt) => {
                    let buttons = args.get(1).map(Value::as_int).transpose()?.unwrap_or(0);
                    let title = args
                        .get(2)
                        .map(Value::to_display)
                        .unwrap_or_else(|| "脚本".into());
                    let result = crate::interpreter::dialog::message_box(
                        &prompt,
                        &title,
                        buttons,
                        ctx.interrupt,
                    )?;
                    Value::Int(result)
                }
            }
        }
        "inputbox" => match args.first().map(Value::to_display) {
            None => Value::Empty,
            Some(prompt) => {
                let title = args
                    .get(1)
                    .map(Value::to_display)
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| "脚本".into());
                let default = args.get(2).map(Value::to_display).unwrap_or_default();
                Value::Str(crate::interpreter::dialog::input_box(
                    &prompt,
                    &title,
                    &default,
                    ctx.interrupt,
                )?)
            }
        },
        "command" => Value::Str(ctx.command.to_string()),
        "shell" => {
            let command = text(0)?;
            let style = args.get(1).map(Value::as_int).transpose()?.unwrap_or(1);
            // VB 的 windowstyle：0 隐藏、1 常规、2 最小化、3 最大化。
            Value::Int(ffi::shell(&command, style as i32)?)
        }
        "len" => match args.first().and_then(|value| value.as_record().ok()) {
            Some(record) => {
                let name = record.borrow().type_name().to_ascii_lowercase();
                match (ctx.record_size)(&name) {
                    Some(size) => Value::Int(size as i64),
                    None => return Err(format!("`Len` 不认识类型 `{name}`")),
                }
            }
            None => Value::Int(text(0)?.chars().count() as i64),
        },
        "mid" => {
            let source: Vec<char> = text(0)?.chars().collect();
            let start = number(1)?.max(1) as usize;
            let length = match args.get(2) {
                Some(value) => value.as_int()?.max(0) as usize,
                None => usize::MAX,
            };
            let begin = (start - 1).min(source.len());
            let end = begin.saturating_add(length).min(source.len());
            Value::Str(source[begin..end].iter().collect())
        }
        "left" => {
            let source: Vec<char> = text(0)?.chars().collect();
            let count = number(1)?.max(0) as usize;
            Value::Str(source[..count.min(source.len())].iter().collect())
        }
        "right" => {
            let source: Vec<char> = text(0)?.chars().collect();
            let count = number(1)?.max(0) as usize;
            let begin = source.len().saturating_sub(count);
            Value::Str(source[begin..].iter().collect())
        }
        "ucase" => Value::Str(text(0)?.to_uppercase()),
        "lcase" => Value::Str(text(0)?.to_lowercase()),
        "trim" => Value::Str(text(0)?.trim().to_string()),
        "ltrim" => Value::Str(text(0)?.trim_start().to_string()),
        "rtrim" => Value::Str(text(0)?.trim_end().to_string()),
        "instr" => {
            let (start, haystack, needle) = match args.len() {
                0 | 1 => return Err("`instr` 需要至少两个参数".into()),
                2 => (1usize, text(0)?, text(1)?),
                _ => (number(0)?.max(1) as usize, text(1)?, text(2)?),
            };
            let chars: Vec<char> = haystack.chars().collect();
            let begin = (start - 1).min(chars.len());
            let found = chars[begin..]
                .windows(needle.chars().count().max(1))
                .position(|window| window.iter().copied().eq(needle.chars()));
            Value::Int(found.map(|index| (begin + index + 1) as i64).unwrap_or(0))
        }
        "instrrev" => {
            let haystack = text(0)?;
            let needle = text(1)?;
            Value::Int(
                haystack
                    .rfind(&needle)
                    .map(|byte_index| haystack[..byte_index].chars().count() as i64 + 1)
                    .unwrap_or(0),
            )
        }
        "replace" => Value::Str(text(0)?.replace(&text(1)?, &text(2)?)),
        "chr" => {
            let code = number(0)? as u32;
            Value::Str(char::from_u32(code).unwrap_or('\u{fffd}').to_string())
        }
        "asc" => Value::Int(text(0)?.chars().next().map(|c| c as i64).unwrap_or(0)),
        "val" => Value::Float(leading_number(&text(0)?).unwrap_or(0.0)),
        "str" => Value::Str(text(0)?),
        "cstr" => Value::Str(text(0)?),
        "cint" => Value::Int(round_half_even(text(0)?.parse::<f64>().unwrap_or_else(|_| {
            args.first().and_then(|value| value.as_int().ok()).unwrap_or(0) as f64
        })) as i64),
        "clng" => Value::Int(args.first().map(|v| v.as_int()).transpose()?.unwrap_or(0)),
        "cdbl" | "csng" => Value::Float(args.first().map(|v| v.as_float()).transpose()?.unwrap_or(0.0)),
        "cbool" => Value::Bool(args.first().map(|v| v.to_bool()).transpose()?.unwrap_or(false)),
        "abs" => match args.first() {
            Some(Value::Float(value)) => Value::Float(value.abs()),
            _ => Value::Int(number(0)?.abs()),
        },
        "int" => Value::Int(args.first().map(|v| v.as_float()).transpose()?.unwrap_or(0.0).floor() as i64),
        "fix" => Value::Int(args.first().map(|v| v.as_float()).transpose()?.unwrap_or(0.0).trunc() as i64),
        "sgn" => {
            let value = args.first().map(|v| v.as_float()).transpose()?.unwrap_or(0.0);
            Value::Int(if value > 0.0 {
                1
            } else if value < 0.0 {
                -1
            } else {
                0
            })
        }
        "sqr" => Value::Float(args.first().map(|v| v.as_float()).transpose()?.unwrap_or(0.0).sqrt()),
        "exp" => Value::Float(args.first().map(|v| v.as_float()).transpose()?.unwrap_or(0.0).exp()),
        "log" => Value::Float(args.first().map(|v| v.as_float()).transpose()?.unwrap_or(0.0).ln()),
        "round" => {
            let value = args.first().map(|v| v.as_float()).transpose()?.unwrap_or(0.0);
            let digits = args.get(1).map(Value::as_int).transpose()?.unwrap_or(0).max(0) as i32;
            let factor = 10f64.powi(digits);
            Value::Float(round_half_even(value * factor) / factor)
        }
        "rnd" => {
            let mut state = *ctx.rng;
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *ctx.rng = state;
            Value::Float((state % 1_000_003) as f64 / 1_000_003.0)
        }
        "hex" => Value::Str(format!("{:X}", number(0)? as u32)),
        "oct" => Value::Str(format!("{:o}", number(0)? as u32)),
        "space" => Value::Str(" ".repeat(number(0)?.max(0) as usize)),
        "string" => {
            let count = number(0)?.max(0) as usize;
            let ch = text(1)?.chars().next().unwrap_or(' ');
            Value::Str(std::iter::repeat_n(ch, count).collect())
        }
        "timer" => {
            let _ = ctx.started;
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs_f64())
                .unwrap_or(0.0);
            Value::Float(seconds % 86_400.0)
        }
        "sleep" => {
            ffi::sleep_ms(number(0)?.max(0) as u32);
            Value::Empty
        }
        "tickcount" => Value::Int(ffi::tick_count() as i64),
        "error" => {
            let number = match args.first() {
                Some(_) => number(0)?,
                None => ctx.err_number,
            };
            Value::Str(error_text(number))
        }
        "iif" => {
            let condition = args.first().map(|v| v.to_bool()).transpose()?.unwrap_or(false);
            if condition {
                args.get(1).cloned().unwrap_or(Value::Empty)
            } else {
                args.get(2).cloned().unwrap_or(Value::Empty)
            }
        }
        "format" => {
            let value = args.first().cloned().unwrap_or(Value::Empty);
            let spec = args.get(1).map(Value::to_display).unwrap_or_default();
            Value::Str(format_value(&value, &spec)?)
        }
        "split" => {
            let source = text(0)?;
            let delimiter = args.get(1).map(Value::to_display).unwrap_or_else(|| " ".into());
            let limit = match args.get(2) {
                Some(value) => value.as_int()?.max(0) as usize,
                None => 0,
            };
            Value::array(split_parts(&source, &delimiter, limit))
        }
        "join" => {
            let array = args
                .first()
                .ok_or_else(|| "`join` 缺少第 1 个参数".to_string())?
                .as_array()?;
            let delimiter = args.get(1).map(Value::to_display).unwrap_or_default();
            let text = {
                let array = array.borrow();
                if array.dims().is_empty() {
                    return Err("`Join` 的数组尚未初始化".into());
                }
                if array.dims().len() > 1 {
                    return Err("`Join` 只支持一维数组".into());
                }
                array
                    .values()
                    .iter()
                    .map(Value::to_display)
                    .collect::<Vec<_>>()
                    .join(&delimiter)
            };
            Value::Str(text)
        }
        "array" => Value::array(args),
        "isarray" => Value::Bool(args.first().is_some_and(Value::is_array)),
        "isnumeric" => Value::Bool(args.first().is_some_and(Value::is_numeric_like)),
        "typename" => Value::Str(
            args.first()
                .map(Value::type_name)
                .unwrap_or_default(),
        ),
        "lbound" | "ubound" => {            let array = args
                .first()
                .ok_or_else(|| format!("`{name}` 缺少第 1 个参数"))?
                .as_array()?;
            let dimension = match args.get(1) {
                Some(value) => value.as_int()?.max(1) as usize,
                None => 1,
            };
            let array = array.borrow();
            let Some((lower, count)) = array.dims().get(dimension - 1).copied() else {
                return Err(if array.dims().is_empty() {
                    "数组尚未初始化（需要先 ReDim）".to_string()
                } else {
                    format!("数组没有第 {dimension} 维")
                });
            };
            Value::Int(if name == "lbound" {
                i64::from(lower)
            } else {
                i64::from(lower) + count as i64 - 1
            })
        }
        "now" => Value::Float(now_serial()),
        "date" => Value::Float(now_serial().floor()),
        "time" => Value::Float(now_serial().fract()),
        "dateserial" => Value::Float(serial_from_ymd(number(0)?, number(1)?, number(2)?)?),
        "timeserial" => {
            let seconds = number(0)? * 3600 + number(1)? * 60 + number(2)?;
            Value::Float(seconds as f64 / 86_400.0)
        }
        "dateadd" => {
            let interval = text(0)?.to_ascii_lowercase();
            let amount = number(1)?;
            let serial = match args.get(2) {
                Some(value) => value.as_float()?,
                None => now_serial(),
            };
            Value::Float(date_add(&interval, amount, serial)?)
        }
        "datediff" => {
            let interval = text(0)?.to_ascii_lowercase();
            let first = args.get(1).map(Value::as_float).transpose()?.unwrap_or(0.0);
            let second = args.get(2).map(Value::as_float).transpose()?.unwrap_or(0.0);
            Value::Int(date_diff(&interval, first, second)?)
        }
        "year" | "month" | "day" | "hour" | "minute" | "second" => {
            let serial = args.first().map(Value::as_float).transpose()?.unwrap_or(0.0);
            let (year, month, day, hour, minute, second) = parts_from_serial(serial);
            Value::Int(match name {
                "year" => year,
                "month" => month,
                "day" => day,
                "hour" => hour,
                "minute" => minute,
                _ => second,
            })
        }
        // ---------- 文件系统 / 环境 ----------
        "dir" => dir_value(&args, ctx)?,
        "filelen" => file_len_value(&args, ctx)?,
        "filedatetime" => file_datetime_value(&args, ctx)?,
        "curdir" => Value::Str(ctx.cwd.display().to_string()),
        "environ" => Value::Str(
            args.first()
                .and_then(|value| std::env::var(value.to_display()).ok())
                .unwrap_or_default(),
        ),
        "kill" => {
            kill_value(&args, ctx)?;
            Value::Empty
        }
        "filecopy" => {
            let source = args.first().map(Value::to_display).unwrap_or_default();
            let destination = args.get(1).map(Value::to_display).unwrap_or_default();
            let from = resolved(ctx.cwd, &source);
            let to = resolved(ctx.cwd, &destination);
            std::fs::copy(&from, &to).map_err(|error| {
                ctx.failure = Some(io_number(&error));
                format!("`FileCopy` 无法复制 `{source}` → `{destination}`：{error}")
            })?;
            Value::Empty
        }
        "mkdir" => {
            let path = args.first().map(Value::to_display).unwrap_or_default();
            let target = resolved(ctx.cwd, &path);
            std::fs::create_dir(&target).map_err(|error| {
                ctx.failure = Some(io_number(&error));
                format!("`MkDir` 无法创建 `{path}`：{error}")
            })?;
            Value::Empty
        }
        "rmdir" => {
            let path = args.first().map(Value::to_display).unwrap_or_default();
            let target = resolved(ctx.cwd, &path);
            std::fs::remove_dir(&target).map_err(|error| {
                ctx.failure = Some(io_number(&error));
                format!("`RmDir` 无法删除 `{path}`：{error}")
            })?;
            Value::Empty
        }
        "chdir" => {
            let path = args.first().map(Value::to_display).unwrap_or_default();
            let target = resolved(ctx.cwd, &path);
            if !target.is_dir() {
                ctx.failure = Some(76);
                return Err(format!("`ChDir` 找不到路径 `{path}`"));
            }
            // 只改脚本视角的当前目录，不动进程 CWD，避免影响编辑器本身。
            *ctx.cwd = target;
            Value::Empty
        }
        "rename" => {
            let from = args.first().map(Value::to_display).unwrap_or_default();
            let to = args.get(1).map(Value::to_display).unwrap_or_default();
            let source = resolved(ctx.cwd, &from);
            let destination = resolved(ctx.cwd, &to);
            std::fs::rename(&source, &destination).map_err(|error| {
                ctx.failure = Some(io_number(&error));
                format!("`Name` 无法把 `{from}` 改名为 `{to}`：{error}")
            })?;
            Value::Empty
        }
        // ---------- 类型判断 ----------
        "isempty" => Value::Bool(matches!(args.first(), None | Some(Value::Empty))),
        "isnull" => Value::Bool(matches!(args.first(), Some(Value::Null))),
        "isobject" => Value::Bool(args.first().is_some_and(Value::is_object)),
        "isdate" => Value::Bool(args.first().is_some_and(is_date)),
        // ---------- 字符串补充 ----------
        "strreverse" => Value::Str(text(0)?.chars().rev().collect()),
        "strcomp" => str_comp_value(&args, ctx.compare_text)?,
        "filter" => filter_value(&args, ctx.compare_text)?,
        "ascw" => Value::Int(
            text(0)?
                .chars()
                .next()
                .map(|ch| i64::from(ch as u32 as u16 as i16))
                .unwrap_or(0),
        ),
        "chrw" => Value::Str(
            char::from_u32((number(0)? as u32) & 0xFFFF)
                .unwrap_or('\u{FFFD}')
                .to_string(),
        ),
        // ---------- 日期补充 ----------
        "weekday" => {
            let serial = args.first().map(Value::as_float).transpose()?.unwrap_or_else(now_serial);
            let first_day = args.get(1).map(Value::as_int).transpose()?.unwrap_or(1);
            Value::Int(weekday_of(serial, first_day))
        }
        "weekdayname" => {
            let weekday = number(0)?;
            let abbreviate = args.get(1).map(Value::to_bool).transpose()?.unwrap_or(false);
            let first_day = args.get(2).map(Value::as_int).transpose()?.unwrap_or(1);
            if !(1..=7).contains(&weekday) {
                return Err(format!("`WeekdayName` 的星期 {weekday} 超出 1..7"));
            }
            let first = if first_day == 0 { 1 } else { first_day };
            let index = ((first - 1) + (weekday - 1)).rem_euclid(7) as usize;
            let name = WEEKDAY_NAMES[index];
            Value::Str(if abbreviate { name[..3].to_string() } else { name.to_string() })
        }
        "monthname" => {
            let month = number(0)?;
            let abbreviate = args.get(1).map(Value::to_bool).transpose()?.unwrap_or(false);
            if !(1..=12).contains(&month) {
                return Err(format!("`MonthName` 的月份 {month} 超出 1..12"));
            }
            let name = MONTH_NAMES[(month - 1) as usize];
            Value::Str(if abbreviate { name[..3].to_string() } else { name.to_string() })
        }
        "datevalue" => {
            let raw = text(0)?;
            match parse_date_text(&raw) {
                Some(serial) => Value::Float(serial),
                None => return Err(format!("`DateValue` 无法识别日期 \"{raw}\"")),
            }
        }
        "timevalue" => match args.first() {
            Some(Value::Float(_) | Value::Int(_)) => {
                Value::Float(args[0].as_float()?.fract())
            }
            _ => {
                let raw = text(0)?;
                match parse_time_text(&raw) {
                    Some(serial) => Value::Float(serial),
                    None => return Err(format!("`TimeValue` 无法识别时间 \"{raw}\"")),
                }
            }
        },
        // ---------- 运行时杂项 ----------
        "randomize" => {
            *ctx.rng = match args.first() {
                Some(value) if !matches!(value, Value::Empty) => value.as_float()? as u64,
                _ => SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos() as u64)
                    .unwrap_or(0x9E37_79B9_7F4A_7C15),
            }
            .max(1);
            Value::Empty
        }
        "doevents" => {
            // 宿主是 GPUI 应用，脚本里派发本进程消息会和编辑器自己的事件循环打架，
            // 这里只让出一次时间片，效果等价于「别把界面卡死」。
            ffi::sleep_ms(0);
            Value::Int(0)
        }
        _ => return Err(format!("未实现的内置函数 `{name}`")),
    })
}

const WEEKDAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const MONTH_NAMES: [&str; 12] = [
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

/// I/O 错误 → VB 错误号：文件不存在 53、路径不存在 76、权限 70、已存在 58，其余 75。
fn io_number(error: &std::io::Error) -> i64 {
    match error.raw_os_error() {
        Some(2) => 53,
        Some(3) => 76,
        Some(5) => 70,
        Some(183) => 58,
        _ => match error.kind() {
            std::io::ErrorKind::NotFound => 53,
            std::io::ErrorKind::PermissionDenied => 70,
            std::io::ErrorKind::AlreadyExists => 58,
            _ => 75,
        },
    }
}

/// 把路径拆成 `(目录, 文件名掩码)`。
fn split_pattern(pattern: &std::path::Path) -> (std::path::PathBuf, String) {
    let parent = match pattern.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let mask = pattern
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "*".into());
    (parent, mask)
}

/// VB 的通配符匹配：`*` 任意串、`?` 单个字符，大小写不敏感。
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some(p);
                resume = n;
                p += 1;
            }
            Some(&ch) if ch == '?' || ch == name[n] => {
                p += 1;
                n += 1;
            }
            _ => {
                let Some(position) = star else {
                    return false;
                };
                p = position + 1;
                resume += 1;
                n = resume;
            }
        }
    }
    while matches!(pattern.get(p), Some('*')) {
        p += 1;
    }
    p == pattern.len()
}

/// 带通配符的路径展开成实际文件列表；没有通配符时原样返回。
fn expand_paths(cwd: &std::path::Path, pattern: &str) -> Vec<String> {
    if !pattern.contains(['*', '?']) {
        return vec![resolved(cwd, pattern).display().to_string()];
    }
    let (parent, mask) = split_pattern(&resolved(cwd, pattern));
    let Ok(reader) = std::fs::read_dir(&parent) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = reader
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| !kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| wildcard_match(&mask, name))
        .map(|name| parent.join(name).display().to_string())
        .collect();
    paths.sort();
    paths
}

/// `Dir([路径[, 属性]])`：带路径时重建列表，无参时取下一条。
fn dir_value(args: &[Value], ctx: &mut Ctx) -> Result<Value, String> {
    let pattern = match args.first() {
        None | Some(Value::Empty) => None,
        Some(value) => Some(value.to_display()),
    };
    let Some(pattern) = pattern else {
        let Some(state) = ctx.dir.as_mut() else {
            return Err("`Dir` 需要先用路径调用一次".into());
        };
        return Ok(Value::Str(state.next_entry()));
    };
    let (parent, mask) = split_pattern(&resolved(ctx.cwd, &pattern));
    let attributes = args.get(1).map(Value::as_int).transpose()?.unwrap_or(0);
    let include_dirs = attributes & 16 != 0;
    let reader = std::fs::read_dir(&parent).map_err(|error| {
        ctx.failure = Some(io_number(&error));
        format!("`Dir` 无法打开 {}：{error}", parent.display())
    })?;
    let mut entries = Vec::new();
    for entry in reader.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if is_dir && !include_dirs {
            continue;
        }
        if wildcard_match(&mask, &name) {
            entries.push(name);
        }
    }
    entries.sort();
    let mut state = DirState {
        entries,
        index: 0,
    };
    let first = state.next_entry();
    *ctx.dir = Some(state);
    Ok(Value::Str(first))
}

fn file_len_value(args: &[Value], ctx: &mut Ctx) -> Result<Value, String> {
    let path = args.first().map(Value::to_display).unwrap_or_default();
    let target = resolved(ctx.cwd, &path);
    std::fs::metadata(&target)
        .map(|metadata| Value::Int(metadata.len() as i64))
        .map_err(|error| {
            ctx.failure = Some(io_number(&error));
            format!("无法读取 `{path}`：{error}")
        })
}

/// `FileDateTime(path)`：本地时区的 VB 日期序列。
fn file_datetime_value(args: &[Value], ctx: &mut Ctx) -> Result<Value, String> {
    let path = args.first().map(Value::to_display).unwrap_or_default();
    let target = resolved(ctx.cwd, &path);
    let metadata = std::fs::metadata(&target).map_err(|error| {
        ctx.failure = Some(io_number(&error));
        format!("无法读取 `{path}`：{error}")
    })?;
    let modified = metadata.modified().map_err(|error| {
        ctx.failure = Some(75);
        format!("无法读取 `{path}` 的时间：{error}")
    })?;
    let seconds = modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    Ok(Value::Float(serial_from_unix(seconds) + local_offset_days()))
}

/// Unix 秒 → VB 日期序列（UTC）。
fn serial_from_unix(seconds: f64) -> f64 {
    let epoch = (days_from_civil(1970, 1, 1) - ole_epoch_days()) as f64;
    epoch + seconds / 86_400.0
}

/// 本地时区相对 UTC 的偏移（天），取整到分钟，避免亚秒抖动。
fn local_offset_days() -> f64 {
    let utc_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    let offset = now_serial() - serial_from_unix(utc_now);
    (offset * 1440.0).round() / 1440.0
}

/// `Kill`：支持 `*`/`?` 通配符；一个都没匹配到时按 VB 报「找不到文件」。
fn kill_value(args: &[Value], ctx: &mut Ctx) -> Result<(), String> {
    let pattern = args.first().map(Value::to_display).unwrap_or_default();
    let paths = expand_paths(ctx.cwd, &pattern);
    if paths.is_empty() {
        ctx.failure = Some(53);
        return Err(format!("`Kill` 找不到文件 `{pattern}`"));
    }
    for path in paths {
        std::fs::remove_file(&path).map_err(|error| {
            ctx.failure = Some(io_number(&error));
            format!("`Kill` 无法删除 `{path}`：{error}")
        })?;
    }
    Ok(())
}

/// `StrComp(a, b[, compare])`：`compare` 省略或为 -1 时按 `Option Compare`，`1` 忽略大小写。
fn str_comp_value(args: &[Value], compare_text: bool) -> Result<Value, String> {
    let first = args.first().ok_or("`StrComp` 缺少第 1 个参数")?;
    let second = args.get(1).ok_or("`StrComp` 缺少第 2 个参数")?;
    if matches!(first, Value::Null) || matches!(second, Value::Null) {
        return Ok(Value::Null);
    }
    let compare = args.get(2).map(Value::as_int).transpose()?.unwrap_or(-1);
    let ignore_case = match compare {
        -1 => compare_text,
        1 => true,
        _ => false,
    };
    let left = first.to_display();
    let right = second.to_display();
    let ordering = if ignore_case {
        left.to_lowercase().cmp(&right.to_lowercase())
    } else {
        left.cmp(&right)
    };
    Ok(Value::Int(match ordering {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

/// `Filter(数组, 子串[, include[, compare]])`：返回 0 基的新数组。
fn filter_value(args: &[Value], compare_text: bool) -> Result<Value, String> {
    let array = args
        .first()
        .ok_or("`Filter` 缺少第 1 个参数")?
        .as_array()
        .map_err(|message| format!("`Filter` {message}"))?;
    let needle = args
        .get(1)
        .ok_or("`Filter` 缺少第 2 个参数")?
        .to_display();
    let include = match args.get(2) {
        Some(value) => value.to_bool()?,
        None => true,
    };
    let compare = args.get(3).map(Value::as_int).transpose()?.unwrap_or(-1);
    // -1 = 按 `Option Compare`；0 = 二进制；1 = 忽略大小写。
    let ignore_case = match compare {
        -1 => compare_text,
        1 => true,
        _ => false,
    };
    let needle = if ignore_case {
        needle.to_lowercase()
    } else {
        needle
    };
    let mut kept = Vec::new();
    for value in array.borrow().values() {
        let text = value.to_display();
        let haystack = if ignore_case { text.to_lowercase() } else { text };
        if haystack.contains(&needle) == include {
            kept.push(value);
        }
    }
    Ok(Value::array(kept))
}

/// 星期几：默认 1=星期日 … 7=星期六，`first_day` 指定一周的第一天。
fn weekday_of(serial: f64, first_day: i64) -> i64 {
    // 日期序列 0 = 1899-12-30 = 星期六。
    let sunday_based = (serial.floor() as i64 + 6).rem_euclid(7) + 1;
    let first = if first_day == 0 { 1 } else { first_day };
    ((sunday_based - first).rem_euclid(7)) + 1
}

/// 识别 `2024-01-31`、`2024/1/31`、`1/31/2024`、`2024年1月31日` 这类日期文本。
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
    let (year, month, day) = if parts[0] > 31 {
        (parts[0], parts[1], parts[2])
    } else if parts[2] > 31 {
        (parts[2], parts[0], parts[1])
    } else {
        return None;
    };
    serial_from_ymd(year, month, day).ok()
}

/// 识别 `12:34`、`12:34:56.5` 这类时间文本。
fn parse_time_text(text: &str) -> Option<f64> {
    let mut parts = text.trim().split(':');
    let hour: i64 = parts.next()?.trim().parse().ok()?;
    let minute: i64 = parts.next().unwrap_or("0").trim().parse().ok()?;
    let second: f64 = parts.next().unwrap_or("0").trim().parse().ok()?;
    Some((hour * 3600 + minute * 60) as f64 / 86_400.0 + second / 86_400.0)
}

fn is_date(value: &Value) -> bool {
    match value {
        Value::Float(_) | Value::Int(_) | Value::Bool(_) => true,
        Value::Str(text) => parse_date_text(text).is_some() || parse_time_text(text).is_some(),
        _ => false,
    }
}

/// 取字符串开头的数字部分（`Val` 的语义）。
fn leading_number(text: &str) -> Option<f64> {
    let trimmed = text.trim_start();
    let mut end = 0usize;
    let mut seen_digit = false;
    let mut seen_dot = false;
    let mut seen_exponent = false;
    for (index, ch) in trimmed.char_indices() {
        match ch {
            '0'..='9' => seen_digit = true,
            '+' | '-' if index == 0 => {}
            '.' if !seen_dot && !seen_exponent => seen_dot = true,
            'e' | 'E' if seen_digit && !seen_exponent => seen_exponent = true,
            _ => break,
        }
        end = index + ch.len_utf8();
    }
    if !seen_digit {
        return None;
    }
    parse_number(&trimmed[..end])
}

// ---------------------------------------------------------------- 日期时间

/// 公历日期 → 自 1970-01-01 起的天数（Howard Hinnant 的算法）。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// 天数 → 公历日期。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
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

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        _ => 28,
    }
}

/// VB 的日期序列以 1899-12-30 为 0。
fn ole_epoch_days() -> i64 {
    days_from_civil(1899, 12, 30)
}

fn serial_from_ymd(year: i64, month: i64, day: i64) -> Result<f64, String> {
    if !(1..=12).contains(&month) {
        return Err(format!("月份 {month} 超出 1..12"));
    }
    let last = days_in_month(year, month);
    if day < 1 || day > last {
        return Err(format!("日期 {day} 超出 {year}-{month} 的范围 1..{last}"));
    }
    Ok((days_from_civil(year, month, day) - ole_epoch_days()) as f64)
}

fn serial_from_parts(
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
fn parts_from_serial(serial: f64) -> (i64, i64, i64, i64, i64, i64) {
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
fn now_serial() -> f64 {
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

fn date_add(interval: &str, amount: i64, serial: f64) -> Result<f64, String> {
    Ok(match interval {
        "yyyy" => add_months(serial, amount * 12),
        "q" => add_months(serial, amount * 3),
        "m" => add_months(serial, amount),
        "ww" => serial + amount as f64 * 7.0,
        "d" => serial + amount as f64,
        "h" => serial + amount as f64 / 24.0,
        "n" => serial + amount as f64 / 1440.0,
        "s" => serial + amount as f64 / 86_400.0,
        other => return Err(format!("`DateAdd` 不支持间隔 `{other}`")),
    })
}

fn add_months(serial: f64, months: i64) -> f64 {
    let (year, month, day, hour, minute, second) = parts_from_serial(serial);
    let index = year * 12 + (month - 1) + months;
    let new_year = index.div_euclid(12);
    let new_month = index.rem_euclid(12) + 1;
    let new_day = day.min(days_in_month(new_year, new_month));
    serial_from_parts(new_year, new_month, new_day, hour, minute, second, 0)
}

fn date_diff(interval: &str, first: f64, second: f64) -> Result<i64, String> {
    let (year1, month1, _, _, _, _) = parts_from_serial(first);
    let (year2, month2, _, _, _, _) = parts_from_serial(second);
    Ok(match interval {
        "yyyy" => year2 - year1,
        "m" => (year2 - year1) * 12 + (month2 - month1),
        "d" => (second.floor() - first.floor()) as i64,
        "ww" => (second.floor() - first.floor()) as i64 / 7,
        "h" => ((second - first) * 24.0) as i64,
        "n" => ((second - first) * 1440.0) as i64,
        "s" => ((second - first) * 86_400.0) as i64,
        other => return Err(format!("`DateDiff` 不支持间隔 `{other}`")),
    })
}

fn is_date_format(spec: &str) -> bool {
    spec.chars()
        .any(|ch| matches!(ch.to_ascii_lowercase(), 'y' | 'd' | 'h' | 'n' | 's'))
}

/// `Format(date, "yyyy-mm-dd hh:nn:ss")` 的日期格式子集。
fn format_date(serial: f64, spec: &str) -> String {
    let (year, month, day, hour, minute, second) = parts_from_serial(serial);
    let chars: Vec<char> = spec.chars().collect();
    let mut output = String::new();
    let mut index = 0usize;
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
        let lower = ch.to_ascii_lowercase();
        let run = chars[index..]
            .iter()
            .take_while(|next| next.to_ascii_lowercase() == lower)
            .count();
        let value = match lower {
            'y' => Some(if run >= 4 {
                format!("{year:04}")
            } else {
                format!("{:02}", year.rem_euclid(100))
            }),
            'm' => Some(if run >= 2 {
                format!("{month:02}")
            } else {
                format!("{month}")
            }),
            'd' => Some(if run >= 2 {
                format!("{day:02}")
            } else {
                format!("{day}")
            }),
            'h' => Some(if run >= 2 {
                format!("{hour:02}")
            } else {
                format!("{hour}")
            }),
            'n' => Some(if run >= 2 {
                format!("{minute:02}")
            } else {
                format!("{minute}")
            }),
            's' => Some(if run >= 2 {
                format!("{second:02}")
            } else {
                format!("{second}")
            }),
            _ => None,
        };
        match value {
            Some(text) => {
                output.push_str(&text);
                index += run;
            }
            None => {
                output.push(ch);
                index += 1;
            }
        }
    }
    output
}

/// `Split` 的分段规则：分隔符为空或 `limit` 为 1 时整串作为唯一元素。
fn split_parts(source: &str, delimiter: &str, limit: usize) -> Vec<Value> {
    if delimiter.is_empty() || limit == 1 {
        return vec![Value::Str(source.to_string())];
    }
    let parts: Vec<&str> = if limit == 0 {
        source.split(delimiter).collect()
    } else {
        source.splitn(limit, delimiter).collect()
    };
    parts
        .into_iter()
        .map(|part| Value::Str(part.to_string()))
        .collect()
}

/// `Format`：支持数值格式（`0`/`#`/`.`/`,`/`%`、`;` 分段）与大小写格式（`>`/`<`）。
fn format_value(value: &Value, spec: &str) -> Result<String, String> {
    if spec.is_empty() {
        return Ok(value.to_display());
    }
    if !spec.contains('0') && !spec.contains('#') {
        if is_date_format(spec) {
            return Ok(format_date(value.as_float()?, spec));
        }
        let text = value.to_display();
        return if spec.contains('>') {
            Ok(text.to_uppercase())
        } else if spec.contains('<') {
            Ok(text.to_lowercase())
        } else {
            Err(format!("`Format` 暂不支持格式 \"{spec}\""))
        };
    }

    let number = value
        .as_float()
        .map_err(|_| format!("`Format` 需要数值，但得到 \"{}\"", value.to_display()))?;
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
    if negative_sign && number < 0.0 {
        text.insert(0, '-');
    }
    Ok(text)
}

/// 按单个数字格式段（`#,##0.00`）格式化一个非负数。
fn format_section(value: f64, section: &str) -> Result<String, String> {
    if section.is_empty() {
        return Ok(String::new());
    }

    let chars: Vec<char> = section.chars().collect();
    let first = chars.iter().position(|ch| matches!(ch, '0' | '#'));
    let last = chars.iter().rposition(|ch| matches!(ch, '0' | '#'));
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(unquote(section));
    };

    let prefix = unquote(&chars[..first].iter().collect::<String>());
    let suffix = unquote(&chars[last + 1..].iter().collect::<String>());
    if suffix.starts_with(',') {
        return Err("`Format` 暂不支持千位缩放（末尾逗号）".into());
    }

    let core: String = chars[first..=last].iter().collect();
    if let Some(bad) = core
        .chars()
        .find(|ch| !matches!(ch, '0' | '#' | ',' | '.'))
    {
        return Err(format!("`Format` 不支持格式字符 `{bad}`"));
    }

    let value = if prefix.contains('%') || suffix.contains('%') {
        value * 100.0
    } else {
        value
    };

    let (int_format, frac_format) = match core.split_once('.') {
        Some((int, frac)) => (int, frac),
        None => (core.as_str(), ""),
    };
    let decimals = frac_format
        .chars()
        .filter(|ch| matches!(ch, '0' | '#'))
        .count();
    let min_decimals = frac_format.chars().filter(|ch| *ch == '0').count();
    let min_int = int_format.chars().filter(|ch| *ch == '0').count();
    let grouped = int_format.contains(',');

    let factor = 10f64.powi(decimals as i32);
    let rounded = round_half_even(value * factor) / factor;
    let text = format!("{rounded:.decimals$}");
    let (int_text, frac_text) = match text.split_once('.') {
        Some((int, frac)) => (int.to_string(), frac.to_string()),
        None => (text.clone(), String::new()),
    };

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

    let mut output = String::with_capacity(section.len() + 4);
    output.push_str(&prefix);
    output.push_str(&integer);
    if !frac_format.is_empty() {
        output.push('.');
        output.push_str(&fraction);
    }
    output.push_str(&suffix);
    Ok(output)
}

/// 从右往左每三位插入千位分隔符。
fn group_thousands(digits: &str) -> String {
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

/// 去掉格式里的引号/反斜杠转义，得到字面量文本。
fn unquote(text: &str) -> String {
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                for inner in chars.by_ref() {
                    if inner == '"' {
                        break;
                    }
                    output.push(inner);
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    output.push(next);
                }
            }
            _ => output.push(ch),
        }
    }
    output
}
