//! 内置函数：VB 标准库的一个实用子集 + 几个 Win32 便捷封装。

use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::interpreter::ffi;
use crate::interpreter::value::{
    Value, VbArray, VbType, parse_number, parse_radix_text, round_half_even,
};

mod constants;
mod dates;
mod files;
mod format;
mod settings;
mod strings;

// 旧的 `builtins::名字` 路径继续可用（跨模块调用点不用改）。
use constants::QB_COLORS;
pub use constants::vb_constant;
pub use constants::vb_constant_type;
use dates::MONTH_NAMES;
use dates::WEEKDAY_ABBREVIATIONS;
use dates::WEEKDAY_NAMES;
use dates::date_add;
use dates::date_diff;
use dates::date_part;
pub use dates::format_date_display;
use dates::is_date;
use dates::leading_number;
use dates::now_serial;
pub use dates::parse_date_text;
pub use dates::parse_date_value;
use dates::parse_time_text;
use dates::parts_from_serial;
use dates::serial_from_normalized;
use dates::two_digit_year;
use dates::weekday_of;
use files::dir_value;
use files::file_datetime_value;
use files::file_len_value;
use files::get_attr_value;
use files::io_number;
use files::kill_value;
use files::set_attr_value;
use format::format_amount;
use format::format_date_named;
use format::format_value;
use settings::delete_setting;
use settings::get_setting;
use settings::looks_like_picture;
use settings::save_setting;
use settings::settings_array;
use settings::settings_in_section;
use strings::chars_match_at;
use strings::compare_is_text;
use strings::filter_value;
use strings::find_substring;
use strings::replace_substring;
use strings::split_parts;
use strings::str_comp_value;

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
    /// 是否允许弹窗；headless 宿主为 `false`，`MsgBox`/`InputBox` 直接返回默认值。
    pub allow_ui: bool,
    /// 脚本运行时的命令行参数（`Command()`）。
    pub command: &'a str,
    /// 脚本视角的当前目录；相对路径按它解析，`ChDir` 修改它。
    pub cwd: &'a mut std::path::PathBuf,
    /// 内置函数失败时要用的 VB 错误号（找不到文件 53、路径错误 75 …）。
    pub failure: Option<i64>,
    /// `StrPtr` 返回的 UTF-16 缓冲区；必须活到脚本结束，指针才一直有效。
    pub strings: &'a mut Vec<Vec<u16>>,
    /// 第一个实参的**静态类型**（只有 `Hex` / `Oct` 需要：输出宽度跟着参数类型走）。
    pub arg_type: Option<VbType>,
    /// 全部实参的静态类型（`Array(...)` 记元素类型、`IIf`/`Choose`/`Switch` 记结果类型都要用）。
    pub arg_types: &'a [Option<VbType>],
    /// 返回实参本身的内置函数（`IIf`/`Choose`/`Switch`）把选中项的内容类型记在这里，
    /// 供 `VarType` / `TypeName` 回答（`Value` 本身不带窄类型）。
    pub result_narrow: Option<VbType>,
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
    "createobject",
    "getobject",
    "msgbox",
    "inputbox",
    "shell",
    "command",
    "len",
    "lenb",
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
    "rgb",
    "qbcolor",
    "beep",
    "formatnumber",
    "formatcurrency",
    "formatpercent",
    "formatdatetime",
    "getsetting",
    "savesetting",
    "deletesetting",
    "getallsettings",
    "loadpicture",
    "str",
    "cstr",
    "cint",
    "clng",
    "cbyte",
    "cvar",
    "vartype",
    "cdbl",
    "csng",
    "ccur",
    "cdate",
    "cbool",
    "abs",
    "int",
    "fix",
    "sgn",
    "sqr",
    "exp",
    "log",
    "sin",
    "cos",
    "tan",
    "atn",
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
    "choose",
    "switch",
    "strconv",
    "partition",
    "ismissing",
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
    "datepart",
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "eof",
    "lof",
    "fileattr",
    "input",
    "freefile",
    "error",
    "dir",
    "filelen",
    "filedatetime",
    "getattr",
    "setattr",
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
    "strptr",
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
    BUILTINS.contains(&name) || crate::interpreter::finance::BUILTINS.contains(&name)
}

/// 全部内置函数名（小写），供编辑器补全使用。
pub fn names() -> Vec<&'static str> {
    let mut names = BUILTINS.to_vec();
    names.extend_from_slice(crate::interpreter::finance::BUILTINS);
    names
}

/// VB 错误号对应的说明文字（`Error()` 与 `Err.Raise` / `Error n` 的默认描述）。
///
/// 文案来自本机 VB6 p-code 实测的 `Error$(n)`（**运行时**表），与 VB6 IDE 编译对话框用的
/// RT_STRING 文案不同（例如 3 号运行时是「不执行 GoSub 而返回」，IDE 是「无 GoSub 返回」）；
/// 未知错误号返回 VB6 的兜底文案「定义的应用程序或对象错误」。
pub fn error_text(number: i64) -> String {
    if number == 0 {
        return String::new();
    }
    let text = match number {
        3 => "不执行 GoSub 而返回",
        5 => "无效的过程调用或参数",
        6 => "溢出",
        7 => "超出内存",
        9 => "下标越界",
        10 => "该数组被固定或临时锁定",
        11 => "除数为零",
        13 => "类型不匹配",
        14 => "超出字符串空间",
        16 => "表达式太复杂",
        17 => "不能执行请求的操作",
        18 => "发生用户中断",
        20 => "无错误恢复",
        28 => "堆栈空间溢出",
        35 => "Sub 或 Function 未定义",
        47 => "太多 DLL 应用程序客户",
        48 => "加载 DLL 错误",
        49 => "DLL 调用约定错误",
        51 => "内部错误",
        52 => "错误文件名或数",
        53 => "文件未找到",
        54 => "错误文件模式",
        55 => "文件已经打开",
        57 => "设备 I/O 错误",
        58 => "文件已经存在",
        59 => "错误记录长度",
        61 => "磁盘满",
        62 => "输入超出文件尾",
        63 => "错误记录号",
        67 => "太多文件",
        68 => "设备不可用",
        70 => "拒绝的权限",
        71 => "磁盘未准备好",
        74 => "不能用不同驱动器重命名",
        75 => "路径/文件访问错误",
        76 => "未找到路径",
        91 => "未设置对象变量或 With block 变量",
        92 => "For 循环未初始化",
        93 => "无效格式字符串",
        94 => "使用 Null 无效",
        95 => "定义的应用程序或对象错误",
        96 => "无法吸收对象的事件，因为引发的事件已经达到它的对象接收器所支持的最大数目。",
        97 => "在不是定义类实例的对象中不能调用友元函数",
        98 => "属性或方法调用不能包括对私有对象的引用，无论是作为参数或者作为返回值。",
        424 => "需要对象",
        429 => "ActiveX 部件不能创建对象",
        438 => "对象不支持该属性或方法",
        446 => "对象不支持命名参数",
        449 => "参数不是可选的",
        450 => "错误的参数数或无效属性分配",
        // VB6 自己的 451 资源串是坏的（`属性使得未定义过程Property let …`），这里按含义写。
        451 => "属性 Let 过程未定义，或属性 Get 过程没有返回对象",
        452 => "无效的序号",
        453 => "未找到指定的 DLL 函数",
        454 => "未找到代码源",
        455 => "代码源锁定错误",
        457 => "该键已同该集合中的一个元素关联。",
        458 => "变量使用了 Visual Basic 不支持的 Automation 类型",
        459 => "对象或类不支持事件设置",
        460 => "无效剪贴板格式",
        461 => "未找到方法或数据成员",
        462 => "远程服务器不存在或不可用",
        463 => "在本地机器上未注册类",
        _ => "定义的应用程序或对象错误",
    };
    text.to_string()
}

/// `VarType` 的返回值（VB 的 `vb*` 常量），只按**值的动态类型**判断。数组是 `vbArray + 元素类型码`。
///
/// 变量的**声明**类型在 [`crate::interpreter::runtime`] 的调用分派里优先使用：
/// `Dim n As Integer` 的 `VarType(n)` 是 2。但值本身不带窄类型信息，所以字面量与表达式
/// （`VarType(1)`、`VarType(i * 2)`）仍按 `Long` 报 3，`Variant` 里装的整数也一样。
pub(crate) fn vartype_code_of(value: &Value) -> i64 {
    vartype_code(Some(value))
}

fn vartype_code(value: Option<&Value>) -> i64 {
    let Some(value) = value else {
        return 0;
    };
    match value {
        Value::Empty => 0,
        Value::Null => 1,
        // `VarType(Nothing)` 是 9（vbObject），与真正的空值 `Empty` 不同（本机 VB6 实测）。
        Value::Nothing => 9,
        Value::Int(_) => 3,
        Value::Float(_) => 5,
        Value::Currency(_) => 6,
        Value::Date(_) => 7,
        Value::Str(_) => 8,
        Value::Bool(_) => 11,
        Value::Array(array) => 8192 + vartype_of(array.borrow().element_type()),
        Value::FuncPtr(_) => 3,
        Value::Record(_)
        | Value::Class(_)
        | Value::ComObject(_)
        | Value::Collection(_)
        | Value::Dictionary(_)
        | Value::Picture(_) => 9,
    }
}

/// `Hex` / `Oct` 的文本：宽度跟着参数的静态类型走。
///
/// 本机 VB6 实测：`Hex(-1)` 是 16 位的 `FFFF`（`-1` 是 Integer）、`Hex(-1&)` 是 32 位的 `FFFFFFFF`、
/// `Hex(-32768)` 是 `FFFF8000`（`32768` 已是 Long，取负后仍是 Long）、正数一律不补零。
fn radix_text(value: i64, ty: Option<VbType>, radix: u32) -> String {
    if ty == Some(VbType::Integer) {
        let narrow = value as i16;
        return match radix {
            8 => format!("{:o}", narrow as u16),
            _ => format!("{:X}", narrow as u16),
        };
    }
    let wide = value as i32 as u32;
    match radix {
        8 => format!("{wide:o}"),
        _ => format!("{wide:X}"),
    }
}

/// 声明类型对应的 `VarType` 类型码。
///
/// 数组拼 8192 时用它，`VarType(变量)` 也用它——值本身不带窄类型信息，
/// 只能靠变量的声明类型区分 Integer / Byte / Single。
pub(crate) fn vartype_of(ty: VbType) -> i64 {
    match ty {
        VbType::Byte => 17,
        VbType::Integer => 2,
        VbType::Long | VbType::LongPtr => 3,
        VbType::LongLong => 20,
        VbType::Single => 4,
        VbType::Double => 5,
        VbType::Currency => 6,
        VbType::Date => 7,
        VbType::Boolean => 11,
        VbType::Str => 8,
        VbType::Object => 9,
        VbType::Variant | VbType::Any => 12,
    }
}

/// `StrConv`：实现大小写、全角与「宽/窄」恒等变换。
///
/// `vbUnicode`(64) / `vbFromUnicode`(128) 对 Rust 的 Unicode 字符串是恒等变换；
/// VB6 的字节级转换语义在 QIDE 里没有对应物，这里按原样返回。
fn strconv(source: &str, conversion: i64) -> Result<String, String> {
    // 1/2 是大小写位，3 由这两位组合出「首字母大写」，不必单列。
    const SUPPORTED: i64 = 1 | 2 | 4 | 8 | 64 | 128;
    if conversion < 0 || conversion & !SUPPORTED != 0 {
        return Err(format!("无效的 `StrConv` 转换方式 {conversion}"));
    }
    let mut text = match conversion & 3 {
        1 => source.to_uppercase(),
        2 => source.to_lowercase(),
        3 => proper_case(source),
        _ => source.to_string(),
    };
    if conversion & 4 != 0 {
        text = text.chars().map(to_fullwidth).collect();
    }
    Ok(text)
}

/// `vbProperCase`：只有**空白**才是词边界（本机 VB6 实测 `hello,world/x-y` → `Hello,world/x-y`，
/// 逗号与斜杠后面的字母保持小写），并且只有词首那个字符本身是字母时才大写
/// （`123abc` → `123abc`、`a-b` → `A-b`）。
fn proper_case(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut at_word_start = true;
    for ch in source.chars() {
        if ch.is_whitespace() {
            out.push(ch);
            at_word_start = true;
        } else if ch.is_alphanumeric() {
            if at_word_start {
                out.extend(ch.to_uppercase());
            } else {
                out.extend(ch.to_lowercase());
            }
            at_word_start = false;
        } else {
            out.push(ch);
            at_word_start = false;
        }
    }
    out
}

fn to_fullwidth(ch: char) -> char {
    match ch {
        ' ' => '\u{3000}',
        '\u{21}'..='\u{7e}' => char::from_u32(ch as u32 + 0xFEE0).unwrap_or(ch),
        _ => ch,
    }
}

/// `Partition`：返回 `number` 落在哪个区间，格式与 VB6 一致（本机实测）。
///
/// 区间是 `[start + k*interval, start + k*interval + interval - 1]`，上界超过 `stop` 时截到 `stop`；
/// 两段都按 `停止值 + 1` 的位数右对齐补空格。`number` 小于 `start` 时前半段留空、
/// 后半段是 `start - 1`；大于 `stop` 时前半段是 `stop + 1`、后半段留空。
fn partition(number: i64, start: i64, stop: i64, interval: i64) -> Result<String, String> {
    const INVALID: &str = "`Partition` 的区间参数无效";
    if start < 0 || stop <= start || interval < 1 {
        return Err(INVALID.to_string());
    }
    let width = (stop.saturating_add(1)).to_string().len();
    let field = |value: i64| format!("{value:>width$}");
    let blank = " ".repeat(width);
    if number < start {
        return Ok(format!("{blank}:{}", field(start - 1)));
    }
    if number > stop {
        return Ok(format!("{}:{blank}", field(stop.saturating_add(1))));
    }
    let offset = (number - start) / interval;
    let lower = start.saturating_add(offset.saturating_mul(interval));
    let upper = lower.saturating_add(interval - 1).min(stop);
    Ok(format!("{}:{}", field(lower), field(upper)))
}

pub fn call(name: &str, args: Vec<Value>, ctx: &mut Ctx) -> Result<Value, String> {
    // 财务函数单独一个模块（`finance.rs`），这里先让给它，未命中再往下走。
    if let Some(value) = crate::interpreter::finance::call(name, &args, ctx)? {
        return Ok(value);
    }
    let text = |index: usize| -> Result<String, String> {
        args.get(index)
            .map(Value::to_display)
            .ok_or_else(|| format!("`{name}` 缺少第 {} 个参数", index + 1))
    };
    // VB 的 `Null` 怎么处理看形参声明：`As String` / `As Long` 这类**非 `Variant`** 形参收到 `Null`
    // 报错误号 94（「无效使用 Null」），`Variant` 形参则把 `Null` 原样传回去。
    // 本机 VB6 实测：`Asc(Null)`=94、`Split(Null, ",")`=94、`Chr(Null)`=94、`Val(Null)`=94，
    // 而 `Trim(Null)`、`LCase(Null)`、`Right(Null, 1)`、`String(3, Null)` 都返回 `Null`。
    let strict = |index: usize| -> Result<(), String> {
        match args.get(index) {
            Some(Value::Null) => Err("无效使用 Null".into()),
            _ => Ok(()),
        }
    };
    let propagated = |index: usize| -> Option<Value> {
        matches!(args.get(index), Some(Value::Null)).then_some(Value::Null)
    };
    let number = |index: usize| -> Result<i64, String> {
        args.get(index)
            .ok_or_else(|| format!("`{name}` 缺少第 {} 个参数", index + 1))?
            .as_int()
    };

    Ok(match name {
        "msgbox" => match args.first().map(Value::to_display) {
            None => Value::Empty,
            // 无人值守：不弹窗，按「点了确定」返回 `vbOK`。
            Some(_) if !ctx.allow_ui => Value::Int(0),
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
        },
        "inputbox" => match args.first().map(Value::to_display) {
            None => Value::Empty,
            // 无人值守：不弹窗，直接返回空串（相当于用户点了取消）。
            Some(_) if !ctx.allow_ui => Value::Str(String::new()),
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
        "len" | "lenb" => {
            // `Len`/`LenB` 的形参是 `Variant`：`Len(Null)` 与 `LenB(Null)` 都返回 `Null`（本机 VB6 实测）。
            if let Some(null) = propagated(0) {
                return Ok(null);
            }
            if matches!(args.first(), Some(Value::Array(_))) {
                ctx.failure = Some(13);
                return Err("`Len` 不能用于数组".into());
            }
            match args.first().and_then(|value| value.as_record().ok()) {
                Some(record) => {
                    let name = record.borrow().type_name().to_ascii_lowercase();
                    match (ctx.record_size)(&name) {
                        Some(size) => Value::Int(size as i64),
                        None => return Err(format!("`Len` 不认识类型 `{name}`")),
                    }
                }
                // VB 的字符串是 UTF-16，`LenB` 是 `Len` 的两倍（本机 VB6 实测 `LenB("abc")`=6）。
                None => {
                    let chars = text(0)?.chars().count() as i64;
                    Value::Int(if name == "lenb" { chars * 2 } else { chars })
                }
            }
        }
        "mid" => {
            if let Some(null) = propagated(0) {
                return Ok(null);
            }
            strict(1)?;
            strict(2)?;
            let source: Vec<char> = text(0)?.chars().collect();
            let start = number(1)?;
            if start < 1 {
                ctx.failure = Some(5);
                return Err("`Mid` 的起点必须大于 0".into());
            }
            let start = start as usize;
            let length = match args.get(2) {
                Some(value) => value.as_int()?.max(0) as usize,
                None => usize::MAX,
            };
            let begin = (start - 1).min(source.len());
            let end = begin.saturating_add(length).min(source.len());
            Value::Str(source[begin..end].iter().collect())
        }
        "left" => {
            if let Some(null) = propagated(0) {
                return Ok(null);
            }
            strict(1)?;
            let source: Vec<char> = text(0)?.chars().collect();
            let count = number(1)?;
            if count < 0 {
                ctx.failure = Some(5);
                return Err("`Left` 的长度不能是负数".into());
            }
            let count = count as usize;
            Value::Str(source[..count.min(source.len())].iter().collect())
        }
        "right" => {
            if let Some(null) = propagated(0) {
                return Ok(null);
            }
            strict(1)?;
            let source: Vec<char> = text(0)?.chars().collect();
            let count = number(1)?;
            if count < 0 {
                ctx.failure = Some(5);
                return Err("`Right` 的长度不能是负数".into());
            }
            let count = count as usize;
            let begin = source.len().saturating_sub(count);
            Value::Str(source[begin..].iter().collect())
        }
        "ucase" => match propagated(0) {
            Some(null) => null,
            None => Value::Str(text(0)?.to_uppercase()),
        },
        "lcase" => match propagated(0) {
            Some(null) => null,
            None => Value::Str(text(0)?.to_lowercase()),
        },
        "trim" => match propagated(0) {
            Some(null) => null,
            None => Value::Str(text(0)?.trim().to_string()),
        },
        "ltrim" => match propagated(0) {
            Some(null) => null,
            None => Value::Str(text(0)?.trim_start().to_string()),
        },
        "rtrim" => match propagated(0) {
            Some(null) => null,
            None => Value::Str(text(0)?.trim_end().to_string()),
        },
        "instr" => {
            // `InStr([start], string1, string2, [compare])`：VB 的 compare 是 0=二进制、1=文本、
            // 2=数据库（按二进制处理）。省略时按二进制比较。
            // `Null` 的处理按形参走：字符串形参是 `Variant` 所以传播 `Null`，
            // 起点是 `As Long` 所以报 13（本机 VB6 实测 `InStr(Null, "a")`=Null、`InStr("abc", "b", Null)`=13）。
            let (start, haystack, needle, ignore_case) = match args.len() {
                0 | 1 => return Err("`instr` 需要至少两个参数".into()),
                2 => {
                    if let Some(null) = propagated(0).or_else(|| propagated(1)) {
                        return Ok(null);
                    }
                    (1i64, text(0)?, text(1)?, false)
                }
                3 => {
                    // 三参形式是 `InStr(start, string1, string2)`：先转起点，再判字符串的 `Null`。
                    strict(0)?;
                    let start = number(0)?;
                    if propagated(1).is_some() || propagated(2).is_some() {
                        return Ok(Value::Null);
                    }
                    (start, text(1)?, text(2)?, false)
                }
                _ => {
                    strict(0)?;
                    let start = number(0)?;
                    if propagated(1).is_some() || propagated(2).is_some() {
                        return Ok(Value::Null);
                    }
                    (start, text(1)?, text(2)?, compare_is_text(args.get(3))?)
                }
            };
            if start < 1 {
                ctx.failure = Some(5);
                return Err("`InStr` 的起点必须大于 0".into());
            }
            let start = start as usize;
            let chars: Vec<char> = haystack.chars().collect();
            let needle: Vec<char> = needle.chars().collect();
            // VB：子串为空时返回起点；起点越过末尾时返回 0。
            if needle.is_empty() {
                return Ok(Value::Int(if start > chars.len() {
                    0
                } else {
                    start as i64
                }));
            }
            let begin = (start - 1).min(chars.len());
            Value::Int(
                find_substring(&chars, &needle, begin, ignore_case)
                    .map(|index| index as i64 + 1)
                    .unwrap_or(0),
            )
        }
        "instrrev" => {
            // `InStrRev(string1, string2, [start], [compare])`：从 `start`（默认 -1 = 末尾）
            // 往左找，返回**起始位置**在 `start` 之前的最后一个匹配。
            // 形参都是非 `Variant`，收到 `Null` 报 94（本机 VB6 实测 `InStrRev(Null, "a")`=94）。
            strict(0)?;
            strict(1)?;
            let haystack = text(0)?;
            let needle = text(1)?;
            let chars: Vec<char> = haystack.chars().collect();
            let needle: Vec<char> = needle.chars().collect();
            let start = match args.get(2) {
                Some(value) => value.as_int()?,
                None => -1,
            };
            let ignore_case = compare_is_text(args.get(3))?;
            if start == 0 {
                ctx.failure = Some(5);
                return Err("`InStrRev` 的起点不能是 0".into());
            }
            let start = if start < 0 {
                chars.len()
            } else {
                (start as usize).min(chars.len())
            };
            if needle.is_empty() {
                return Ok(Value::Int(start as i64 + 1));
            }
            let found = (0..start)
                .rev()
                .find(|index| chars_match_at(&chars, &needle, *index, ignore_case));
            Value::Int(found.map(|index| index as i64 + 1).unwrap_or(0))
        }
        "replace" => {
            // `Replace(expression, find, replace, [start], [count], [compare])`。
            // 三个字符串形参都是非 `Variant`：任一为 `Null` 报 94（VB6 实测）。
            strict(0)?;
            strict(1)?;
            strict(2)?;
            let source = text(0)?;
            let find = text(1)?;
            let replacement = text(2)?;
            let start = match args.get(3) {
                Some(value) => value.as_int()?.max(1),
                None => 1,
            };
            let count = match args.get(4) {
                Some(value) => value.as_int()?,
                None => -1,
            };
            let ignore_case = compare_is_text(args.get(5))?;
            Value::Str(replace_substring(
                &source,
                &find,
                &replacement,
                start as usize,
                count,
                ignore_case,
            ))
        }
        "chr" => {
            strict(0)?;
            let code = number(0)? as u32;
            Value::Str(char::from_u32(code).unwrap_or('\u{fffd}').to_string())
        }
        "asc" => {
            strict(0)?;
            match text(0)?.chars().next() {
                Some(first) => Value::Int(first as i64),
                None => {
                    ctx.failure = Some(5);
                    return Err("`Asc` 的参数不能是空字符串".into());
                }
            }
        }
        "val" => {
            strict(0)?;
            Value::Float(leading_number(&text(0)?).unwrap_or(0.0))
        }
        // `RGB(r, g, b)`：分量超过 255 按 255 处理、负数报错误号 5（本机 VB6 实测）。
        "rgb" => {
            let mut color = 0i64;
            for (index, weight) in [1i64, 256, 65536].into_iter().enumerate() {
                let channel = number(index)?;
                if channel < 0 {
                    ctx.failure = Some(5);
                    return Err("`RGB` 的颜色分量不能是负数".into());
                }
                color += channel.min(255) * weight;
            }
            Value::Int(color)
        }
        "qbcolor" => {
            let index = number(0)?;
            if !(0..=15).contains(&index) {
                ctx.failure = Some(5);
                return Err(format!("`QBColor` 的颜色号 {index} 超出 0..15"));
            }
            Value::Int(QB_COLORS[index as usize])
        }
        // `Beep` 在 VB6 里是语句（`Stmt::Call` 会走到这里），响一声系统提示音。
        "beep" => {
            ffi::beep();
            Value::Empty
        }
        "formatnumber" | "formatcurrency" | "formatpercent" => {
            Value::Str(format_amount(name, &args, ctx)?)
        }
        "formatdatetime" => {
            let serial = args
                .first()
                .map(Value::as_float)
                .transpose()?
                .unwrap_or_else(now_serial);
            let named = args.get(1).map(Value::as_int).transpose()?.unwrap_or(0);
            Value::Str(format_date_named(serial, named, ctx)?)
        }
        "getsetting" => {
            let default = match args.get(3) {
                Some(value) => value.to_display(),
                None => String::new(),
            };
            Value::Str(get_setting(&text(0)?, &text(1)?, &text(2)?).unwrap_or(default))
        }
        "savesetting" => {
            let (app, section, key, value) = (text(0)?, text(1)?, text(2)?, text(3)?);
            save_setting(&app, &section, &key, &value);
            Value::Empty
        }
        "deletesetting" => {
            let app = text(0)?;
            let section = if args.len() > 1 { Some(text(1)?) } else { None };
            let key = if args.len() > 2 { Some(text(2)?) } else { None };
            delete_setting(&app, section.as_deref(), key.as_deref());
            Value::Empty
        }
        // 本机 VB6 实测：返回 0 基二维 `String` 数组（`VarType` 8200、`TypeName` 是 `String()`），
        // 列 0 是键名、列 1 是值，顺序是**写入顺序**；一节里没有键时返回 `Empty`（不是空数组）。
        // 本机 VB6 实测：文件不存在报 53、内容不是图片报 481；`LoadPicture("")` / `LoadPicture()`
        // 返回一个**空图片对象**（不是 `Nothing`，`TypeName` 仍是 `Picture`）。
        "loadpicture" => {
            let path = match args.first() {
                Some(value) => value.to_display(),
                None => String::new(),
            };
            if path.is_empty() {
                return Ok(Value::Picture(std::rc::Rc::new(
                    crate::interpreter::value::PictureValue {
                        path: String::new(),
                        bytes: Vec::new(),
                    },
                )));
            }
            let full = resolved(ctx.cwd, &path);
            let bytes = match std::fs::read(&full) {
                Ok(bytes) => bytes,
                Err(_) => {
                    ctx.failure = Some(53);
                    return Err(format!("加载图片失败：找不到文件 `{path}`"));
                }
            };
            if !looks_like_picture(&bytes) {
                ctx.failure = Some(481);
                return Err(format!("`{path}` 不是有效的图片文件"));
            }
            Value::Picture(std::rc::Rc::new(crate::interpreter::value::PictureValue {
                path,
                bytes,
            }))
        }
        "getallsettings" => {
            let app = text(0)?;
            let section = text(1)?;
            if app.is_empty() || section.is_empty() {
                return Err("`GetAllSettings` 需要应用名和节名".into());
            }
            match settings_in_section(&app, &section) {
                entries if entries.is_empty() => Value::Empty,
                entries => Value::Array(std::rc::Rc::new(std::cell::RefCell::new(settings_array(
                    &entries,
                )))),
            }
        }
        "str" => match args.first() {
            None => Value::Str(String::new()),
            // `Str(Null)` 在 VB 里传播 `Null`。
            Some(Value::Null) => Value::Null,
            Some(value) => Value::Str(str_value(value)),
        },
        "cstr" => match args.first() {
            // VB6 实测：`CStr(Null)` 报 94（「使用 Null 无效」）、`CStr(数组)` 报 13（类型不匹配），
            // 没有默认属性的对象报 450（`CStr(对象)`）。
            Some(Value::Null) => {
                ctx.failure = Some(94);
                return Err("无效使用 Null".into());
            }
            Some(Value::Array(_)) => {
                ctx.failure = Some(13);
                return Err("`CStr` 不能用于数组".into());
            }
            Some(Value::ComObject(_)) => {
                ctx.failure = Some(450);
                return Err("对象没有可用的默认属性，不能转换为字符串".into());
            }
            _ => Value::Str(text(0)?),
        },
        "cint" => Value::Int(convert_integer(
            args.first(),
            i16::MIN.into(),
            i16::MAX.into(),
            ctx,
        )?),
        "clng" => Value::Int(convert_integer(
            args.first(),
            i32::MIN.into(),
            i32::MAX.into(),
            ctx,
        )?),
        "cdbl" => {
            let value = args
                .first()
                .map(|v| v.as_float())
                .transpose()?
                .unwrap_or(0.0);
            // `CDbl("1e400")` 超出 Double 范围时报错误号 6（本机 VB6 实测）。
            if !value.is_finite() {
                ctx.failure = Some(6);
                return Err("溢出：结果超出 Double 范围".into());
            }
            Value::Float(value)
        }
        "csng" => {
            let value = args
                .first()
                .map(|v| v.as_float())
                .transpose()?
                .unwrap_or(0.0);
            // `Single` 是 32 位浮点：先窄化再存回 `f64`，后续运算才与 VB6 一致。
            let narrowed = value as f32;
            if narrowed.is_infinite() && value.is_finite() {
                ctx.failure = Some(6);
                return Err("溢出".into());
            }
            Value::Float(f64::from(narrowed))
        }
        "ccur" => {
            let value = args
                .first()
                .map(Value::as_currency)
                .unwrap_or(Ok(0))
                .map_err(|message| {
                    if message == "溢出" {
                        ctx.failure = Some(6);
                    }
                    message
                })?;
            Value::Currency(value)
        }
        "cdate" => {
            let value = args.first().cloned().unwrap_or(Value::Empty);
            value.coerce(crate::interpreter::value::VbType::Date)?
        }
        "cbool" => Value::Bool(
            args.first()
                .map(|v| v.to_bool())
                .transpose()?
                .unwrap_or(false),
        ),
        "cbyte" => {
            // VB6 把 `True` 直接存成字节 255（等同于 -1 按位截断），不走溢出检查。
            let number = match args.first() {
                Some(Value::Bool(flag)) => {
                    if *flag {
                        255
                    } else {
                        0
                    }
                }
                other => convert_integer(other, 0, 255, ctx)?,
            };
            Value::Int(number)
        }
        "cvar" => args.first().cloned().unwrap_or(Value::Empty),
        "vartype" => Value::Int(vartype_code(args.first())),
        "choose" => {
            let index = number(0)?;
            if index < 1 {
                Value::Null
            } else {
                match args.get(index as usize) {
                    Some(value) => {
                        ctx.result_narrow = ctx.arg_types.get(index as usize).copied().flatten();
                        value.clone()
                    }
                    None => Value::Null,
                }
            }
        }
        "switch" => {
            // 成对的「条件, 值」；参数个数为奇数或没有条件成立都按 VB 返回 Null。
            if !args.len().is_multiple_of(2) {
                return Err("参数个数错误".into());
            }
            let mut chosen = Value::Null;
            for (index, pair) in args.chunks(2).enumerate() {
                if pair[0].to_bool()? {
                    chosen = pair[1].clone();
                    ctx.result_narrow = ctx.arg_types.get(index * 2 + 1).copied().flatten();
                    break;
                }
            }
            chosen
        }
        "strconv" => {
            let source = text(0)?;
            let conversion = number(1)?;
            Value::Str(strconv(&source, conversion)?)
        }
        "partition" => {
            ctx.failure = Some(5);
            let result = partition(number(0)?, number(1)?, number(2)?, number(3)?);
            ctx.failure = if result.is_ok() { None } else { Some(5) };
            Value::Str(result?)
        }
        // `Abs`/`Int`/`Fix`/`Round`/`Hex`/`Oct` 接收 `Variant`，`Null` 原样返回（本机 VB6 实测）；
        // `Sgn`/`Sqr`/`Exp`/`Log`/三角函数/`Val` 是强类型，`Null` 报 94。
        "abs" => match args.first() {
            Some(Value::Null) => Value::Null,
            Some(Value::Float(value)) => Value::Float(value.abs()),
            Some(Value::Currency(value)) => {
                let Some(value) = value.checked_abs() else {
                    ctx.failure = Some(6);
                    return Err("溢出".into());
                };
                Value::Currency(value)
            }
            _ => Value::Int(number(0)?.abs()),
        },
        // `Int` / `Fix` 返回参数的**同一种类型**（VB6 实测 `VarType(Int(1.5))` 是 5、`Int(1E+20)` 原样给 1E+20）。
        "int" => int_or_fix(args.first(), true)?,
        "fix" => int_or_fix(args.first(), false)?,
        "sgn" => {
            let value = args
                .first()
                .map(|v| v.as_float())
                .transpose()?
                .unwrap_or(0.0);
            Value::Int(if value > 0.0 {
                1
            } else if value < 0.0 {
                -1
            } else {
                0
            })
        }
        // 三角函数：参数与结果都是 `Double`，`Null` 报 94（本机 VB6 实测）。
        "sin" => math_double(args.first(), f64::sin)?,
        "cos" => math_double(args.first(), f64::cos)?,
        "tan" => math_double(args.first(), f64::tan)?,
        "atn" => math_double(args.first(), f64::atan)?,
        // `Sqr` 负数、`Log` 非正数都报错误号 5，`Exp` 溢出报错误号 6（本机 VB6 实测）。
        "sqr" => {
            let value = args
                .first()
                .map(|v| v.as_float())
                .transpose()?
                .unwrap_or(0.0);
            if value < 0.0 {
                ctx.failure = Some(5);
                return Err("`Sqr` 的参数不能是负数".into());
            }
            Value::Float(value.sqrt())
        }
        "exp" => {
            let value = args
                .first()
                .map(|v| v.as_float())
                .transpose()?
                .unwrap_or(0.0)
                .exp();
            if !value.is_finite() {
                ctx.failure = Some(6);
                return Err("`Exp` 的结果超出范围".into());
            }
            Value::Float(value)
        }
        "log" => {
            let value = args
                .first()
                .map(|v| v.as_float())
                .transpose()?
                .unwrap_or(0.0);
            if value <= 0.0 {
                ctx.failure = Some(5);
                return Err("`Log` 的参数必须大于 0".into());
            }
            Value::Float(value.ln())
        }
        "round" => {
            if matches!(args.first(), Some(Value::Null)) {
                Value::Null
            } else {
                let value = args
                    .first()
                    .map(|v| v.as_float())
                    .transpose()?
                    .unwrap_or(0.0);
                let digits = args
                    .get(1)
                    .map(Value::as_int)
                    .transpose()?
                    .unwrap_or(0)
                    .max(0) as i32;
                let factor = 10f64.powi(digits);
                Value::Float(round_half_even(value * factor) / factor)
            }
        }
        "rnd" => {
            let mut state = *ctx.rng;
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *ctx.rng = state;
            Value::Float((state % 1_000_003) as f64 / 1_000_003.0)
        }
        "hex" => {
            if matches!(args.first(), Some(Value::Null)) {
                Value::Null
            } else {
                Value::Str(radix_text(number(0)?, ctx.arg_type, 16))
            }
        }
        "oct" => {
            if matches!(args.first(), Some(Value::Null)) {
                Value::Null
            } else {
                Value::Str(radix_text(number(0)?, ctx.arg_type, 8))
            }
        }
        "space" => {
            strict(0)?;
            let count = number(0)?;
            if count < 0 {
                ctx.failure = Some(5);
                return Err("`Space` 的参数不能是负数".into());
            }
            Value::Str(limited_repeat(count, ' ', ctx)?)
        }
        "string" => {
            strict(0)?;
            // 第二个形参是 `Variant`：`String(3, Null)` 返回 `Null`（VB6 实测）。
            if let Some(null) = propagated(1) {
                return Ok(null);
            }
            let count = number(0)?;
            if count < 0 {
                ctx.failure = Some(5);
                return Err("`String` 的参数不能是负数".into());
            }
            // `String(3, 65)` 是 `"AAA"`：第二个参数是数值时按字符代码处理（VB6 实测）。
            let ch = match args.get(1) {
                Some(Value::Str(text)) => text.chars().next().unwrap_or(' '),
                Some(value) => char::from_u32((value.as_int()? as u32) & 0xFF).unwrap_or(' '),
                None => ' ',
            };
            Value::Str(limited_repeat(count, ch, ctx)?)
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
            let condition = args
                .first()
                .map(|v| v.to_bool())
                .transpose()?
                .unwrap_or(false);
            // 结果就是其中一个实参，类型跟着它走（VB6 实测 `IIf(True, i, s)` 是 2、`IIf(False, i, s)` 是 4）。
            let index = if condition { 1 } else { 2 };
            ctx.result_narrow = ctx.arg_types.get(index).copied().flatten();
            args.get(index).cloned().unwrap_or(Value::Empty)
        }
        "format" => {
            let value = args.first().cloned().unwrap_or(Value::Empty);
            let spec = args.get(1).map(Value::to_display).unwrap_or_default();
            Value::Str(format_value(&value, &spec)?)
        }
        "split" => {
            strict(0)?;
            let source = text(0)?;
            let delimiter = args
                .get(1)
                .map(Value::to_display)
                .unwrap_or_else(|| " ".into());
            let limit = match args.get(2) {
                Some(value) => value.as_int()?.max(0) as usize,
                None => 0,
            };
            Value::array(split_parts(&source, &delimiter, limit))
        }
        "join" => {
            // `Join(Null, ",")` 在 VB6 里报 13（类型不匹配：`Null` 不是数组），不是 94。
            if matches!(args.first(), Some(Value::Null)) {
                ctx.failure = Some(13);
                return Err("`Join` 需要一个数组".into());
            }
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
                // 直接遍历元素存储单元，省掉整份数组的深拷贝。
                array
                    .cells()
                    .iter()
                    .map(|cell| cell.borrow().to_display())
                    .collect::<Vec<_>>()
                    .join(&delimiter)
            };
            Value::Str(text)
        }
        "array" => {
            if ctx.arg_types.is_empty() {
                Value::array(args)
            } else {
                let items = args
                    .into_iter()
                    .zip(ctx.arg_types.iter().copied())
                    .collect();
                Value::array_with_types(items)
            }
        }
        "isarray" => Value::Bool(args.first().is_some_and(Value::is_array)),
        "isnumeric" => Value::Bool(args.first().is_some_and(Value::is_numeric_like)),
        "typename" => Value::Str(args.first().map(Value::type_name).unwrap_or_default()),
        // `StrPtr(s)`：返回以 0 结尾的 UTF-16 缓冲区地址（`LongPtr`），供 `Declare` 的指针参数使用。
        "strptr" => {
            let mut buffer: Vec<u16> = text(0)?.encode_utf16().collect();
            buffer.push(0);
            ctx.strings.push(buffer);
            let pointer = ctx.strings.last().expect("刚压入").as_ptr() as i64;
            Value::Int(pointer)
        }
        "lbound" | "ubound" => {
            // 本机 VB6 实测：参数不是数组（含空 `Variant`）时报错误号 13。
            let array = args
                .first()
                .ok_or_else(|| format!("`{name}` 缺少第 1 个参数"))?
                .as_array()
                .inspect_err(|_| ctx.failure = Some(13))?;
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
        "now" => Value::Date(now_serial()),
        "date" => Value::Date(now_serial().floor()),
        "time" => Value::Date(now_serial().fract()),
        "dateserial" => {
            let year = two_digit_year(number(0)?);
            Value::Date(serial_from_normalized(year, number(1)?, number(2)?))
        }
        "timeserial" => {
            let seconds = number(0)? * 3600 + number(1)? * 60 + number(2)?;
            Value::Date(seconds as f64 / 86_400.0)
        }
        "dateadd" => {
            let interval = text(0)?.to_ascii_lowercase();
            let amount = number(1)?;
            let serial = match args.get(2) {
                Some(value) => value.as_float()?,
                None => now_serial(),
            };
            Value::Date(date_add(&interval, amount, serial)?)
        }
        "datediff" => {
            let interval = text(0)?.to_ascii_lowercase();
            let first = args.get(1).map(Value::as_float).transpose()?.unwrap_or(0.0);
            let second = args.get(2).map(Value::as_float).transpose()?.unwrap_or(0.0);
            Value::Int(date_diff(&interval, first, second)?)
        }
        "datepart" => {
            let interval = text(0)?.to_ascii_lowercase();
            let serial = args.get(1).map(Value::as_float).transpose()?.unwrap_or(0.0);
            let first_day = args.get(2).map(Value::as_int).transpose()?.unwrap_or(1);
            let first_week = args.get(3).map(Value::as_int).transpose()?.unwrap_or(1);
            Value::Int(date_part(&interval, serial, first_day, first_week)?)
        }
        "year" | "month" | "day" | "hour" | "minute" | "second" => {
            let serial = args
                .first()
                .map(Value::as_float)
                .transpose()?
                .unwrap_or(0.0);
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
        "getattr" => get_attr_value(&args, ctx)?,
        "setattr" => {
            set_attr_value(&args, ctx)?;
            Value::Empty
        }
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
        // 真正的判定在 `call_with_by_value_args` 里按槽位做（需要"参数是否省略"的信息）。
        "ismissing" => Value::Bool(false),
        "isnull" => Value::Bool(matches!(args.first(), Some(Value::Null))),
        "isobject" => Value::Bool(args.first().is_some_and(Value::is_object)),
        "isdate" => Value::Bool(args.first().is_some_and(is_date)),
        // ---------- 字符串补充 ----------
        "strreverse" => {
            strict(0)?;
            Value::Str(text(0)?.chars().rev().collect())
        }
        "strcomp" => str_comp_value(&args, ctx.compare_text)?,
        "filter" => filter_value(&args, ctx.compare_text)?,
        "ascw" => {
            strict(0)?;
            match text(0)?.chars().next() {
                Some(ch) => Value::Int(i64::from(ch as u32 as u16 as i16)),
                None => {
                    ctx.failure = Some(5);
                    return Err("`AscW` 的参数不能是空字符串".into());
                }
            }
        }
        "chrw" => {
            strict(0)?;
            Value::Str(
                char::from_u32((number(0)? as u32) & 0xFFFF)
                    .unwrap_or('\u{FFFD}')
                    .to_string(),
            )
        }
        // ---------- 日期补充 ----------
        "weekday" => {
            let serial = args
                .first()
                .map(Value::as_float)
                .transpose()?
                .unwrap_or_else(now_serial);
            let first_day = args.get(1).map(Value::as_int).transpose()?.unwrap_or(1);
            Value::Int(weekday_of(serial, first_day))
        }
        "weekdayname" => {
            let weekday = number(0)?;
            let abbreviate = args
                .get(1)
                .map(Value::to_bool)
                .transpose()?
                .unwrap_or(false);
            // `WeekdayName` 的 `firstdayofweek` 默认取系统设置：本机 VB6（中文区域）是星期一，
            // 所以 `WeekdayName(1)` 是「星期一」而不是「星期日」。
            let first_day = args.get(2).map(Value::as_int).transpose()?.unwrap_or(0);
            if !(1..=7).contains(&weekday) {
                return Err(format!("`WeekdayName` 的星期 {weekday} 超出 1..7"));
            }
            let first = if first_day == 0 { 2 } else { first_day };
            let index = ((first - 1) + (weekday - 1)).rem_euclid(7) as usize;
            Value::Str(if abbreviate {
                WEEKDAY_ABBREVIATIONS[index].to_string()
            } else {
                WEEKDAY_NAMES[index].to_string()
            })
        }
        "monthname" => {
            let month = number(0)?;
            let abbreviate = args
                .get(1)
                .map(Value::to_bool)
                .transpose()?
                .unwrap_or(false);
            if !(1..=12).contains(&month) {
                return Err(format!("`MonthName` 的月份 {month} 超出 1..12"));
            }
            Value::Str(if abbreviate {
                format!("{month}月")
            } else {
                MONTH_NAMES[(month - 1) as usize].to_string()
            })
        }
        "datevalue" => {
            let raw = text(0)?;
            match parse_date_text(&raw) {
                Some(serial) => Value::Date(serial),
                None => return Err(format!("`DateValue` 无法识别日期 \"{raw}\"")),
            }
        }
        "timevalue" => match args.first() {
            // VB6 只接受字符串与 `Date` 值：`TimeValue(0.75)` / `TimeValue(1)` 报 13，
            // 而 `TimeValue(CDate(0.75))` 是 18:00:00（本机实测）。
            Some(Value::Date(_)) => Value::Date(args[0].as_float()?.fract()),
            Some(Value::Int(_) | Value::Float(_) | Value::Currency(_) | Value::Bool(_)) => {
                ctx.failure = Some(13);
                return Err("`TimeValue` 只接受字符串与日期值".into());
            }
            _ => {
                let raw = text(0)?;
                match parse_time_text(&raw) {
                    Some(serial) => Value::Date(serial),
                    None => {
                        // VB6 对认不出的时间文本报 13（类型不匹配），不是 5。
                        ctx.failure = Some(13);
                        return Err(format!("`TimeValue` 无法识别时间 \"{raw}\""));
                    }
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
            // 运行时会在 `DoEvents` 上先分派排队的 COM 事件，再落到这里；这条分支留给
            // 直接调 `builtins::call` 的场合：只让出一次时间片（不派发本进程消息，
            // 宿主的 GPUI 事件循环不能被脚本抢占）。
            ffi::sleep_ms(0);
            Value::Int(0)
        }
        _ => return Err(format!("未实现的内置函数 `{name}`")),
    })
}

/// 单个字符串允许的最大字符数：避免 `Space(2 ^ 40)` 这类参数一次申请掉几 GB 内存。
const MAX_STRING_CHARS: i64 = 1 << 24;

/// `Space(n)` / `String(n, ch)` 的重复填充，超长时报「字符串空间不足」（错误号 14）。
fn limited_repeat(count: i64, fill: char, ctx: &mut Ctx) -> Result<String, String> {
    let count = count.max(0);
    if count > MAX_STRING_CHARS {
        ctx.failure = Some(14);
        return Err(format!("字符串长度 {count} 超过上限（{MAX_STRING_CHARS}）"));
    }
    Ok(std::iter::repeat_n(fill, count as usize).collect())
}

/// `Str(number)`：正数带一个前导空格（VB 的写法），布尔值照常给 `True` / `False`。
fn str_value(value: &Value) -> String {
    match value {
        Value::Int(_) => str_number_text(value.to_display(), false),
        Value::Float(_) | Value::Currency(_) => str_number_text(value.to_display(), true),
        // `Empty` 按数值 0 处理，与 VB 的 `Str(Empty)` 一致。
        Value::Empty => " 0".to_string(),
        other => other.to_display(),
    }
}

/// `Int` / `Fix`：返回参数的同一种类型（`Int(1.5)` 仍是 Double、`Int(5)` 仍是 Integer）。
///
/// VB6 实测：`VarType(Int(1.5))` 是 5（Double）、`Int(1E+20)` 原样给 1E+20，
/// 所以不能一律转成整数（那样超大值会被截断成 `i64::MAX`）。
fn int_or_fix(value: Option<&Value>, floor: bool) -> Result<Value, String> {
    let Some(value) = value else {
        return Ok(Value::Float(0.0));
    };
    let apply = |number: f64| {
        if floor {
            number.floor()
        } else {
            number.trunc()
        }
    };
    Ok(match value {
        Value::Null => Value::Null,
        Value::Int(number) => Value::Int(*number),
        Value::Float(number) => Value::Float(apply(*number)),
        Value::Date(number) => Value::Date(apply(*number)),
        Value::Currency(scaled) => {
            Value::Currency(apply(*scaled as f64 / 10_000.0) as i64 * 10_000)
        }
        Value::Bool(_) | Value::Str(_) | Value::Empty => Value::Float(apply(value.as_float()?)),
        other => {
            return Err(format!(
                "`Int` / `Fix` 不支持 {} 类型的参数",
                other.type_name()
            ));
        }
    })
}

/// `Sin`/`Cos`/`Tan`/`Atn`：参数按 `Double` 换算（`Null` 报 94），结果也是 `Double`。
fn math_double(value: Option<&Value>, apply: impl Fn(f64) -> f64) -> Result<Value, String> {
    let number = value.map(Value::as_float).transpose()?.unwrap_or(0.0);
    Ok(Value::Float(apply(number)))
}

/// `Str` 的数字写法：`|x| < 1` 时不写整数部分的 0（`Str(0.5)` 是 `" .5"`），正数前面加空格。
///
/// `strip_zero` 只对浮点/货币打开（`Str(0)` 仍是 `" 0"`）。
pub(crate) fn str_number_text(text: String, strip_zero: bool) -> String {
    let text = if strip_zero {
        strip_leading_zero(&text)
    } else {
        text
    };
    if text.starts_with('-') {
        text
    } else {
        format!(" {text}")
    }
}

/// 去掉 `0.5` / `-0.5` 里的前导 `0`：`".5"` / `"-.5"`。
fn strip_leading_zero(text: &str) -> String {
    if let Some(rest) = text.strip_prefix("0.") {
        format!(".{rest}")
    } else if let Some(rest) = text.strip_prefix("-0.") {
        format!("-.{rest}")
    } else {
        text.to_string()
    }
}

/// `CInt` / `CLng`：银行家舍入后检查目标类型范围，越界报错误号 6。
fn convert_integer(
    value: Option<&Value>,
    min: i64,
    max: i64,
    ctx: &mut Ctx,
) -> Result<i64, String> {
    let Some(value) = value else {
        return Ok(0);
    };
    // `CInt(Null)` / `CLng(Null)` 报 94（「无效使用 Null」），不是类型不匹配（本机 VB6 实测）。
    if matches!(value, Value::Null) {
        ctx.failure = Some(94);
        return Err("无效使用 Null".into());
    }
    let number = value.as_float().map_err(|message| {
        ctx.failure = Some(13);
        format!("类型不匹配：{message}")
    })?;
    if !number.is_finite() {
        ctx.failure = Some(6);
        return Err("溢出".into());
    }
    let rounded = round_half_even(number);
    if rounded < min as f64 || rounded > max as f64 {
        ctx.failure = Some(6);
        return Err(format!("溢出：{number} 超出目标类型范围"));
    }
    Ok(rounded as i64)
}
