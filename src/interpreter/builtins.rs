//! 内置函数：VB 标准库的一个实用子集 + 几个 Win32 便捷封装。

use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::interpreter::ffi;
use crate::interpreter::value::{
    Value, VbArray, VbType, parse_number, parse_radix_text, round_half_even,
};

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

/// `QBColor` 的 16 个经典颜色（Long，`r + g*256 + b*65536`；本机 VB6 实测 0/1/7/15）。
const QB_COLORS: [i64; 16] = [
    0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xC0C0C0, 0x808080,
    0xFF0000, 0x00FF00, 0xFFFF00, 0x0000FF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
];

/// VB 标准常量（VB6 里来自 VBA 库）：名字一律小写，比较不分大小写。
///
/// 只收脚本里常用的那些：布尔/默认值、控制字符、比较方式、日期格式、一周第一天与首周规则、
/// `MsgBox` 的标志与返回值、`VarType` 的类型码。
const VB_CONSTANTS: &[(&str, i64)] = &[
    ("vbtrue", -1),
    ("vbfalse", 0),
    ("vbusdefault", -2),
    ("vbobjecterror", -2147221504),
    ("vbbinarycompare", 0),
    ("vbtextcompare", 1),
    ("vbdatabasecompare", 2),
    ("vbgeneraldate", 0),
    ("vblongdate", 1),
    ("vbshortdate", 2),
    ("vblongtime", 3),
    ("vbshorttime", 4),
    ("vbusesystem", 0),
    ("vbsunday", 1),
    ("vbmonday", 2),
    ("vbtuesday", 3),
    ("vbwednesday", 4),
    ("vbthursday", 5),
    ("vbfriday", 6),
    ("vbsaturday", 7),
    ("vbsystemdayofweek", 0),
    ("vbfirstjan1", 1),
    ("vbfirstfourdays", 2),
    ("vbfirstfullweek", 3),
    ("vbokonly", 0),
    ("vbokcancel", 1),
    ("vbabortretryignore", 2),
    ("vbyesnocancel", 3),
    ("vbyesno", 4),
    ("vbretrycancel", 5),
    ("vbcritical", 16),
    ("vbquestion", 32),
    ("vbexclamation", 48),
    ("vbinformation", 64),
    ("vbdefaultbutton1", 0),
    ("vbdefaultbutton2", 256),
    ("vbdefaultbutton3", 512),
    ("vbdefaultbutton4", 768),
    ("vbapplicationmodal", 0),
    ("vbsystemmodal", 4096),
    ("vbok", 1),
    ("vbcancel", 2),
    ("vbabort", 3),
    ("vbretry", 4),
    ("vbignore", 5),
    ("vbyes", 6),
    ("vbno", 7),
    ("vbempty", 0),
    ("vbnull", 1),
    ("vbinteger", 2),
    ("vblong", 3),
    ("vbsingle", 4),
    ("vbdouble", 5),
    ("vbcurrency", 6),
    ("vbdate", 7),
    ("vbstring", 8),
    ("vbobject", 9),
    ("vberror", 10),
    ("vbboolean", 11),
    ("vbvariant", 12),
    ("vbdataobject", 13),
    ("vbdecimal", 14),
    ("vbbyte", 17),
    ("vblonglong", 20),
    ("vbuserdefinedtype", 36),
    ("vbnormal", 0),
    ("vbreadonly", 1),
    ("vbhidden", 2),
    ("vbsystem", 4),
    ("vbvolume", 8),
    ("vbdirectory", 16),
    ("vbarchive", 32),
    ("vbalias", 64),
    ("vbarray", 8192),
    ("vbmethod", 1),
    ("vbget", 2),
    ("vblet", 4),
    ("vbset", 8),
];

/// 字符串型 VB 常量（控制字符）。
const VB_STRING_CONSTANTS: &[(&str, &str)] = &[
    ("vbcr", "\r"),
    ("vblf", "\n"),
    ("vbcrlf", "\r\n"),
    ("vbtab", "\t"),
    ("vbnullchar", "\0"),
    ("vbnullstring", ""),
    ("vbback", "\u{8}"),
    ("vbformfeed", "\u{c}"),
    ("vbverticaltab", "\u{b}"),
];

/// 查一个 VB 标准常量（名字不分大小写），没查到时返回 `None`。
pub fn vb_constant(name: &str) -> Option<Value> {
    let key = name.to_ascii_lowercase();
    if let Some((_, number)) = VB_CONSTANTS.iter().find(|(name, _)| *name == key) {
        return Some(Value::Int(*number));
    }
    VB_STRING_CONSTANTS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, text)| Value::Str((*text).to_string()))
}

/// VB 标准常量的类型（本机 VB6 实测：`VarType(vbTrue)` 是 3（Long）、`VarType(vbCrLf)` 是 8）。
pub fn vb_constant_type(name: &str) -> Option<VbType> {
    let key = name.to_ascii_lowercase();
    if VB_STRING_CONSTANTS.iter().any(|(name, _)| *name == key) {
        return Some(VbType::Str);
    }
    VB_CONSTANTS
        .iter()
        .any(|(name, _)| *name == key)
        .then_some(VbType::Long)
}

/// `FormatNumber` / `FormatCurrency` / `FormatPercent`：写法取本机 VB6（中文区域）实测。
///
/// 参数：`(表达式, [小数位=2], [显示前导零=vbTrue], [负数加括号=vbFalse], [千位分隔=vbTrue])`，
/// 可选参数传 `vbUseDefault`(-2) 等同于省略。
fn format_amount(name: &str, args: &[Value], ctx: &mut Ctx) -> Result<String, String> {
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
fn format_date_named(serial: f64, named: i64, ctx: &mut Ctx) -> Result<String, String> {
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

/// `SaveSetting` / `GetSetting` / `DeleteSetting` 的存储文件。
///
/// VB6 写注册表 `HKCU\Software\VB and VBA Program Settings`；QIDE 不碰注册表，
/// 写 `%APPDATA%\gpui-first\vb-settings.txt`（每行 `应用 节 键 值`，用制表符分隔、反斜杠转义）。
fn settings_path() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("LOCALAPPDATA"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("gpui-first").join("vb-settings.txt")
}

/// 转义/还原设置文件里的字段（`\`、制表符、换行）。
fn escape_setting(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "")
}

fn unescape_setting(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('t') => output.push('\t'),
            Some('n') => output.push('\n'),
            Some(other) => output.push(other),
            None => output.push('\\'),
        }
    }
    output
}

/// 设置文件是「读整份 → 改 → 写回整份」，多个线程同时写会互相丢数据（脚本线程与单元测试都会碰它），
/// 所以读改写整段串行化。
static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 读改写设置文件的互斥体； poisoned 时也照常使用（数据只是文件，不会内存不安全）。
fn lock_settings() -> std::sync::MutexGuard<'static, ()> {
    SETTINGS_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// 读出全部设置项（读不到文件就是空）。
fn read_settings() -> Vec<(String, String, String, String)> {
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                return None;
            }
            Some((
                unescape_setting(parts[0]),
                unescape_setting(parts[1]),
                unescape_setting(parts[2]),
                unescape_setting(parts.get(3).copied().unwrap_or("")),
            ))
        })
        .collect()
}

fn write_settings(entries: &[(String, String, String, String)]) {
    let text: String = entries
        .iter()
        .map(|(app, section, key, value)| {
            format!(
                "{}\t{}\t{}\t{}\n",
                escape_setting(app),
                escape_setting(section),
                escape_setting(key),
                escape_setting(value)
            )
        })
        .collect();
    let path = settings_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, text);
}

/// 键名比较不分大小写（与 VB6 的注册表语义一致）。
fn same_setting(
    entry: &(String, String, String, String),
    app: &str,
    section: &str,
    key: &str,
) -> bool {
    entry.0.eq_ignore_ascii_case(app)
        && entry.1.eq_ignore_ascii_case(section)
        && entry.2.eq_ignore_ascii_case(key)
}

fn get_setting(app: &str, section: &str, key: &str) -> Option<String> {
    read_settings()
        .into_iter()
        .find(|entry| same_setting(entry, app, section, key))
        .map(|entry| entry.3)
}

fn save_setting(app: &str, section: &str, key: &str, value: &str) {
    let _guard = lock_settings();
    let mut entries = read_settings();
    match entries
        .iter_mut()
        .find(|entry| same_setting(entry, app, section, key))
    {
        Some(entry) => entry.3 = value.to_string(),
        None => entries.push((
            app.to_string(),
            section.to_string(),
            key.to_string(),
            value.to_string(),
        )),
    }
    write_settings(&entries);
}

/// `LoadPicture` 支持的文件头：BMP / PNG / JPEG / GIF / ICO / WMF / EMF。
///
/// QIDE 不解码图像，只按幻数判断「像不像图片」——VB6 对内容无效的文件报错误号 481。
fn looks_like_picture(bytes: &[u8]) -> bool {
    bytes.starts_with(b"BM")
        || bytes.starts_with(b"\x89PNG")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || bytes.starts_with(&[0x00, 0x00, 0x01, 0x00])
        || bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A])
        || bytes.starts_with(&[0x01, 0x00, 0x00, 0x00, 0x6C, 0x00, 0x00, 0x00])
}

/// `GetAllSettings`：某一节里的全部键值对，保持写入顺序。
fn settings_in_section(app: &str, section: &str) -> Vec<(String, String)> {
    read_settings()
        .into_iter()
        .filter(|entry| entry.0.eq_ignore_ascii_case(app) && entry.1.eq_ignore_ascii_case(section))
        .map(|entry| (entry.2, entry.3))
        .collect()
}

/// `GetAllSettings` 的返回值：0 基 `(行, 列)` 二维 `String` 数组。
fn settings_array(entries: &[(String, String)]) -> VbArray {
    let mut array = VbArray::new(vec![(0, entries.len()), (0, 2)], VbType::Str, true);
    for (row, (key, value)) in entries.iter().enumerate() {
        let row = row as i32;
        let _ = array.set(&[row, 0], Value::Str(key.clone()), None);
        let _ = array.set(&[row, 1], Value::Str(value.clone()), None);
    }
    array
}

/// `DeleteSetting 应用[, 节[, 键]]`：省略越靠后的参数，删除范围越大。
fn delete_setting(app: &str, section: Option<&str>, key: Option<&str>) {
    let _guard = lock_settings();
    let mut entries = read_settings();
    entries.retain(|entry| {
        if !entry.0.eq_ignore_ascii_case(app) {
            return true;
        }
        if let Some(section) = section
            && !entry.1.eq_ignore_ascii_case(section)
        {
            return true;
        }
        if let Some(key) = key
            && !entry.2.eq_ignore_ascii_case(key)
        {
            return true;
        }
        false
    });
    write_settings(&entries);
}

/// 星期名（索引 0 = 星期日）：本机 VB6（中文区域）实测是中文全称 + `周X` 缩写。
const WEEKDAY_NAMES: [&str; 7] = [
    "星期日",
    "星期一",
    "星期二",
    "星期三",
    "星期四",
    "星期五",
    "星期六",
];

const WEEKDAY_ABBREVIATIONS: [&str; 7] = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

/// 月份名（索引 0 = 一月）：全称 `一月`…`十二月`，缩写 `1月`…`12月`。
const MONTH_NAMES: [&str; 12] = [
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
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| !kind.is_dir())
                .unwrap_or(false)
        })
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
    let mut state = DirState { entries, index: 0 };
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

/// `SetAttr` 允许设置的属性位：`vbReadOnly` / `vbHidden` / `vbSystem` / `vbArchive`。
const SET_ATTR_MASK: i64 = 1 | 2 | 4 | 32;

/// `GetAttr(path)`：返回文件的属性位。
///
/// 本机 VB6 实测返回的就是 Win32 属性位（新建文件常带 `0x2000`「未编入索引」，例如 8224），
/// `vbReadOnly` 1 / `vbHidden` 2 / `vbSystem` 4 / `vbDirectory` 16 / `vbArchive` 32 与系统位一一对应。
fn get_attr_value(args: &[Value], ctx: &mut Ctx) -> Result<Value, String> {
    let path = args.first().map(Value::to_display).unwrap_or_default();
    let target = resolved(ctx.cwd, &path);
    let metadata = std::fs::metadata(&target).map_err(|error| {
        ctx.failure = Some(io_number(&error));
        format!("`GetAttr` 无法读取 `{path}`：{error}")
    })?;
    Ok(Value::Int(attribute_bits(&metadata) as i64))
}

#[cfg(windows)]
fn attribute_bits(metadata: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt as _;
    // `FILE_ATTRIBUTE_NORMAL`（0x80）只在「没有其它属性」时由系统给出，VB6 的 `GetAttr`
    // 把它报成 `vbNormal`（0），所以要清掉这一位（本机 VB6 实测 `SetAttr vbNormal` 后是 0）。
    metadata.file_attributes() & !0x80
}

#[cfg(not(windows))]
fn attribute_bits(metadata: &std::fs::Metadata) -> u32 {
    let mut bits = 0;
    if metadata.permissions().readonly() {
        bits |= 1;
    }
    if metadata.is_dir() {
        bits |= 16;
    }
    bits
}

/// `SetAttr path, attributes`：只接受 `vbReadOnly`/`vbHidden`/`vbSystem`/`vbArchive` 的组合，
/// 其它位（`vbDirectory`、`vbVolume` 等）按 VB6 报错误号 5；属性不是数值报 13。
fn set_attr_value(args: &[Value], ctx: &mut Ctx) -> Result<(), String> {
    let path = args.first().map(Value::to_display).unwrap_or_default();
    let attributes = match args.get(1).map(Value::as_int) {
        Some(Ok(value)) => value,
        Some(Err(message)) => {
            ctx.failure = Some(13);
            return Err(format!("`SetAttr` 的属性参数无效：{message}"));
        }
        None => 0,
    };
    if attributes & !SET_ATTR_MASK != 0 {
        ctx.failure = Some(5);
        return Err(format!(
            "`SetAttr` 不支持属性位 {attributes}（只能组合 vbReadOnly/vbHidden/vbSystem/vbArchive）"
        ));
    }
    let target = resolved(ctx.cwd, &path);
    set_file_attributes(&target, attributes as u32).map_err(|error| {
        ctx.failure = Some(match error.raw_os_error() {
            Some(2 | 3) => 53,
            Some(5) => 70,
            _ => 5,
        });
        format!("`SetAttr` 无法修改 `{path}` 的属性：{error}")
    })
}

#[cfg(windows)]
fn set_file_attributes(path: &std::path::Path, attributes: u32) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;

    unsafe extern "system" {
        fn SetFileAttributesW(name: *const u16, attributes: u32) -> i32;
    }
    // `SetFileAttributesW` 不接受 0：`vbNormal` 要换成 FILE_ATTRIBUTE_NORMAL（0x80）。
    let flags = if attributes == 0 { 0x80 } else { attributes };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe { SetFileAttributesW(wide.as_ptr(), flags) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn set_file_attributes(path: &std::path::Path, attributes: u32) -> std::io::Result<()> {
    // 其它平台只有只读位可写。
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_readonly(attributes & 1 != 0);
    std::fs::set_permissions(path, permissions)
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
    Ok(Value::Date(serial_from_unix(seconds) + local_offset_days()))
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
    let needle = args.get(1).ok_or("`Filter` 缺少第 2 个参数")?.to_display();
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
    let array = array.borrow();
    for cell in array.cells() {
        let value = cell.borrow();
        let text = value.to_display();
        let haystack = if ignore_case {
            text.to_lowercase()
        } else {
            text
        };
        if haystack.contains(&needle) == include {
            // 只复制留下的元素，避免给整份数组做一次深拷贝。
            kept.push((*value).clone());
        }
    }
    drop(array);
    Ok(Value::array(kept))
}

/// 星期几：默认 1=星期日 … 7=星期六，`first_day` 指定一周的第一天。
fn weekday_of(serial: f64, first_day: i64) -> i64 {
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
fn parse_time_text(text: &str) -> Option<f64> {
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
fn parse_time_field(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// 拆掉末尾的 `AM` / `PM`（大小写不敏感、前面可以空一格），返回（正文, 是否 AM）。
fn split_meridiem(text: &str) -> (&str, Option<bool>) {
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

fn is_date(value: &Value) -> bool {
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
fn leading_number(text: &str) -> Option<f64> {
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
fn exponent_follows(rest: &str) -> bool {
    let rest = rest.strip_prefix(['+', '-']).unwrap_or(rest);
    rest.as_bytes().first().is_some_and(u8::is_ascii_digit)
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

/// `DateSerial` 的两位数年份规则：0–29 → 2000–2029，30–99 → 1930–1999（VB6 实测）。
fn two_digit_year(year: i64) -> i64 {
    match year {
        0..=29 => year + 2000,
        30..=99 => year + 1900,
        _ => year,
    }
}

/// `DateSerial` 允许月、日溢出（VB6 实测 `DateSerial(2024, 2, 30)` 是 2024-03-01）。
fn serial_from_normalized(year: i64, month: i64, day: i64) -> f64 {
    let index = year * 12 + (month - 1);
    let normal_year = index.div_euclid(12);
    let normal_month = index.rem_euclid(12) + 1;
    (days_from_civil(normal_year, normal_month, 1) + day - 1 - ole_epoch_days()) as f64
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
        // `y`（一年中的第几天）与 `w`（星期几）在 VB6 里都按「天」加减，`ww` 才是按周。
        "y" | "d" | "w" => serial + amount as f64,
        "ww" => serial + amount as f64 * 7.0,
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
fn date_part(interval: &str, serial: f64, first_day: i64, first_week: i64) -> Result<i64, String> {
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
fn week_one_start(year: i64, first_day: i64, first_week: i64) -> i64 {
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
fn weeks_in_year(year: i64, first_day: i64, first_week: i64) -> i64 {
    (week_one_start(year + 1, first_day, first_week) - week_one_start(year, first_day, first_week))
        / 7
}

/// 一年中的第几周。跨年时先退到上一年，`vbFirstFourDays` 还要把超出的周并到下一年的第 1 周
/// （本机 VB6 实测：`ww` 的年末表现按 `DatePart` 的参数变化，见 `docs/脚本语言.md`）。
fn week_of_year(serial: f64, first_day: i64, first_week: i64) -> i64 {
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

fn is_date_format(spec: &str) -> bool {
    spec.chars().any(|ch| {
        matches!(
            ch.to_ascii_lowercase(),
            'y' | 'm' | 'd' | 'h' | 'n' | 's' | 'w' | 'q' | 'a' | 'c'
        )
    })
}

/// 英文字母的星期/月份名（`Format` 的 `ddd`/`dddd`/`mmm`/`mmmm` 用英文，见本机 VB6 实测）。
const ENGLISH_WEEKDAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const ENGLISH_WEEKDAY_ABBREVIATIONS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const ENGLISH_MONTH_NAMES: [&str; 12] = [
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

const ENGLISH_MONTH_ABBREVIATIONS: [&str; 12] = [
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
fn format_date(serial: f64, spec: &str) -> String {
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

/// `Split` 的分段规则：源串为空时返回空数组（VB6 里 `UBound(Split(""))` 是 -1），
/// 分隔符为空或 `limit` 为 1 时整串作为唯一元素。
fn split_parts(source: &str, delimiter: &str, limit: usize) -> Vec<Value> {
    if source.is_empty() {
        return Vec::new();
    }
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

/// `Format`：支持数值格式（`0`/`#`/`.`/`,`/`%`、`E+00`、`;` 分段）与大小写格式（`>`/`<`）。
fn format_value(value: &Value, spec: &str) -> Result<String, String> {
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
fn format_text(value: &Value, spec: &str) -> String {
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
fn format_number(number: f64, spec: &str) -> Result<String, String> {
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
fn named_date_format(value: &Value, spec: &str) -> Result<Option<String>, String> {
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
fn named_format(value: &Value, spec: &str) -> Result<Option<String>, String> {
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
fn choice_text(value: &Value, zero: &str, nonzero: &str) -> Result<String, String> {
    Ok(if value.as_float()? == 0.0 {
        zero.to_string()
    } else {
        nonzero.to_string()
    })
}

/// VB 的比较方式参数：0 = 二进制、1 = 文本（不区分大小写）、2 = 数据库。
///
/// 解释器没有区域/排序规则，`2` 按二进制处理。
fn compare_is_text(value: Option<&Value>) -> Result<bool, String> {
    match value {
        None | Some(Value::Empty) => Ok(false),
        Some(value) => Ok(value.as_int()? != 0),
    }
}

fn chars_match_at(haystack: &[char], needle: &[char], index: usize, ignore_case: bool) -> bool {
    if index + needle.len() > haystack.len() {
        return false;
    }
    needle.iter().enumerate().all(|(offset, expected)| {
        let actual = haystack[index + offset];
        if ignore_case {
            actual == *expected || actual.to_lowercase().eq(expected.to_lowercase())
        } else {
            actual == *expected
        }
    })
}

/// 从 `begin`（0 基字符下标）开始找子串，返回匹配的 0 基下标。
fn find_substring(
    haystack: &[char],
    needle: &[char],
    begin: usize,
    ignore_case: bool,
) -> Option<usize> {
    if needle.is_empty() || begin >= haystack.len() {
        return None;
    }
    (begin..=haystack.len().saturating_sub(needle.len()))
        .find(|index| chars_match_at(haystack, needle, *index, ignore_case))
}

/// `Replace`：从 `start`（1 基字符位置）起替换，`count` 为负表示全部替换。
///
/// VB6 的返回值从 `start` 开始，`start` 之前的部分不会保留；空 `find` 不做任何替换。
fn replace_substring(
    source: &str,
    find: &str,
    replacement: &str,
    start: usize,
    count: i64,
    ignore_case: bool,
) -> String {
    let find: Vec<char> = find.chars().collect();
    if find.is_empty() {
        return source.to_string();
    }
    let chars: Vec<char> = source.chars().collect();
    let mut output = String::with_capacity(source.len());
    let mut index = (start - 1).min(chars.len());
    let mut replaced = 0i64;
    while index < chars.len() {
        if (count < 0 || replaced < count) && chars_match_at(&chars, &find, index, ignore_case) {
            output.push_str(replacement);
            index += find.len();
            replaced += 1;
            continue;
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

/// 按单个数字格式段（`#,##0.00`）格式化一个非负数。
fn format_section(value: f64, section: &str) -> Result<String, String> {
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
struct FormatChar {
    ch: char,
    literal: bool,
}

/// 拆出格式串的字符并处理转义：`\x` 与 `"…"` 里的字符按字面量输出。
fn parse_format_chars(section: &str) -> Vec<FormatChar> {
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
fn format_literal(spec: &str) -> String {
    parse_format_chars(spec)
        .iter()
        .filter(|item| item.ch != '!')
        .map(|item| item.ch)
        .collect()
}

/// 原样输出一批字符（转义已处理）；未转义的逗号只用于分组/缩放，不显示。
fn push_literals(output: &mut String, chars: &[FormatChar]) {
    for item in chars {
        if !item.literal && item.ch == ',' {
            continue;
        }
        output.push(item.ch);
    }
}

/// 科学计数法的分段渲染：`0.00E+00` 前面的字面量、尾数、指数与后面的字面量。
fn render_section_scientific(
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
fn push_positional_section(
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
fn push_fraction_literals(
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
fn render_fixed(value: f64, int_format: &str, frac_format: &str) -> (String, String) {
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
fn round_decimal(value: f64, decimals: usize) -> (String, String) {
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
struct ExponentSpec {
    marker: char,
    always_sign: bool,
    width: usize,
}

fn exponent_spec(text: &str) -> Result<ExponentSpec, String> {
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
fn render_scientific(
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
