//! VB 语法脚本解释器。
//!
//! 设计目标：用 VB6/VBA 的语法写「能直接调用 Win32 API」的小脚本。
//!
//! - [`lexer`] / [`parser`]：词法与语法分析（大小写不敏感、`'` 注释、`_` 续行、`Dim/If/For/Do`）；
//! - [`runtime`]：树遍历求值，支持 `Sub`/`Function`、默认 `ByRef` 传参、数组与 `For Each`、
//!   `Type ... End Type` 记录与 `With ... End With`、静态作用域（过程只见自己的局部变量与模块级变量）、
//!   `Option Explicit`/`Option Base`/`Option Compare Binary|Text`、`Static` 局部变量，
//!   以及 `GoTo`/标签与 `On Error`/`Err`；
//! - [`ffi`]：`Declare ... Lib "user32"` 的运行时解析与调用；
//! - [`builtins`]：`Mid`/`InStr` 等内置函数；[`dialog`]：`MsgBox`/`InputBox` 的对话框；
//! - [`debug`]：断点、条件断点、暂停/继续、单步与暂停期间的监视求值；[`runtime`] 在每条语句前查询暂停请求；
//! - [`sema`]：保守的语义检查（`Option Explicit` 下的未声明变量），供编辑器做实时诊断。
//!
//! ```ignore
//! let mut lines = Vec::new();
//! interpreter::run("Debug.Print 1 + 2", |line| lines.push(line.to_string()), &flag)?;
//! ```
//!
//! 已知边界：COM 对象与 `Property Set` 尚不支持；
//! Win32 调用支持 `ByRef` / `ByVal` 结构体（字段为整型/浮点/`String * n`/标量数组；
//! `ByVal` 按 x64 ABI：≤ 8 字节进寄存器，更大的传临时副本指针）、整数参数、全 `Double`、
//! 全 `Single`，以及最多 4 个参数的整数/`Double` 混合调用，返回值可为整数或 `Single`/`Double`；
//! 不支持 `Single` 与 `Double` 混用、超过 4 个参数的混合浮点调用，
//! 结构体里的变长 `String` 与 UDT 数组字段也不能用于 Win32 调用。
//! `AddressOf 过程名` 可以把自己的过程当成回调交给 API：参数经机器码跳板进入解释器，
//! 支持最多 4 个 `Byte`/`Integer`/`Long`/`Boolean`/`LongLong`/`LongPtr`/`Single`/`Double`
//! 或 `String` 形参（`ByVal String` 是 `LPCWSTR`，`ByRef` 是 `LPWSTR *`），
//! 返回值可为整数类型（`Sub` 视为 0），不支持返回 `String`/浮点、记录与数组形参；
//! 直接作为 `Declare` 实参的跳板只在该次调用期间有效（`EnumWindows`、`InitOnceExecuteOnce` 等）；
//! 写进 UDT 字段（如 `WNDCLASSW.lpfnWndProc`）时会生成持久跳板，存活到脚本结束，
//! 因此 `RegisterClassW` + `CreateWindowExW` 这类窗口过程也能用。
//! `Type` 字段支持 `String * n`（按空格补齐/截断，`Len` 仍为 n）与定长数组
//! （`Items(2) As Long`、`Items(1 To 3) As Item`，用 `p.Items(1)` 读写元素，
//! `p.Items(0).V = 1` 也可用）；`String * n` 在结构体与文件里都占 `2n` 字节 UTF-16
//! （本解释器只支持 W 版 API，与 VB6 的 ANSI `n` 字节不同）。
//! 过程支持 `Sub`/`Function`、`Optional` 默认值、`ParamArray`（收集剩余实参为 0 基数组），
//! 以及模块级 `Property Get`/`Property Let`：`X = 值` 与 `X(下标) = 值` 会分派到同名 Let，
//! 读 `X` / `X(下标)` 会调用 Get（`Exit Property` 也可用）。
//! `Enum ... End Enum` 的成员注册为模块级常量，`Dim c As Color` 按 `Long` 处理。
//! 对象支持 `Collection`、`Dictionary`（`Scripting.Dictionary` 子集）与自定义 `Class`：
//! `Dim c As New Collection` / `Set d = New Dictionary` / `Set x = Nothing`、
//! `c.Add 元素[, 键]`、`c.Remove 下标或键`、`c.Count`、`c.Item(下标或键)`；
//! 字典另有 `d.Add 键, 值`、`d(键) = 值`、`d(键)`、`d.Item(键)`、`d.Exists(键)`、
//! `d.Keys` / `d.Items`（0 基数组，可 `For Each`）、`d.Remove 键`；键一律大小写不敏感。
//! `Class 名字 … End Class` 里可以写 `Private/Public` 变量、`Sub`/`Function`、`Property Get/Let/Set`，
//! `Class_Initialize` 就是构造函数、`Class_Terminate` 在最后一个引用消失时执行：
//! `Dim p As New 类名` / `Set p = New 类名` 建实例，`p.方法 参数`、`p.属性`、`p.属性 = 值`、
//! `Set p.属性 = 对象`、`p.字段 = 值` 都按成员的声明分派，方法里可以裸写成员名或 `Me.成员`；
//! 实例是引用语义（赋值、传参共享同一份），`TypeOf x Is 类名` 判断实例类型。
//! `Private WithEvents m As New 类名` 让类订阅另一个类的事件，`RaiseEvent 事件(实参)`
//! 会回调订阅者的 `源类名_事件名` 方法（订阅表用弱引用，对象被回收后自动失效）。
//! 简化之处：`Public`/`Private` 只做语法接受（成员都从外部可见）、没有 `Implements`、
//! 类内裸赋值 `Name = v` 不触发同名 `Property Let`（写 `Me.Name = v`）、
//! `Dim x As 类名` 需要类在同文件里先定义。
//! 错误处理支持 `On Error Resume Next`、`On Error GoTo 标签`、`On Error GoTo 0`、`GoTo 标签`、
//! `Err.Raise`/`Err.Clear` 以及 `Err.Number`/`Err.Description`/`Err.Source` 与 `Error(n)`；
//! `Resume`/`Resume Next` 的恢复点是「出错语句所在的根块语句」。
//! 与 VB 一致，整数与 `Single` 的赋值超出目标类型范围时报错误号 6（`溢出`），
//! 不会静默截断；`Variant`/`Double` 不做范围检查。
//! 数组支持 `Dim a(10)`、`a(1 To n)`、多维、`ReDim [Preserve]`、`Erase` 与 `For Each`，
//! 元素也可以是 UDT（`Dim pts(2) As Point`、`pts(0).X = 1`）；UDT 支持成员访问、嵌套记录与 `With`。
//! 数组与记录**赋值按值复制**，`ByRef` 传参（`Bump a(0)`、`Bump p.X`）仍共享同一份存储。
//! 文件 I/O 支持 `Open ... For Input/Output/Append/Binary/Random`（`As #n` 可省略，自动分配空闲文件号）、
//! `Close`、`Print #`、`Write #`、
//! `Line Input #`、`Input #`、`Get #`/`Put #`、`Seek #`，以及 `EOF`/`LOF`/`Seek`/`Loc`/`FreeFile`：
//! 界面运行脚本时会带上 `RunOptions::script_path`，`Open`/`Dir`/`FileLen` 等相对路径按脚本目录解析，
//! `CurDir`/`ChDir` 只改脚本自己的目录，`App.Path` 返回脚本所在目录。
//! - `Get #`/`Put #` 按变量的声明类型（或表达式的动态类型）读写 1/2/4/8 字节，位置是 1 基字节偏移；
//!   `Get #` 的目标可以是变量、数组元素或记录字段（`Get #1, , a(0)`、`Get #1, , p.X`、
//!   `Get #1, , p.Items(1)`），与 `ByRef` 实参共用同一套存储解析；
//!   UDT 整块读写（`Put #1, , p` / `Get #1, , p`，按 `StructLayout` 的字段偏移与对齐），
//!   `Len(p)` 给出 UDT 的字节数。
//!   `Variant` 与变长 `String` 不支持；
//! - `For Random` 是定长记录文件：`Open ... For Random As #n [Len = 记录长度]`（默认 128 字节，
//!   与 VB 一致），`Get #`/`Put #` 的位置参数与 `Seek #` 都是**记录号**，每次读写都会推进整条记录
//!   （写不足一条时补零），单次读写超过记录长度报错误号 59；
//!   `Seek(n)` 返回下一条记录号，`Loc(n)` 返回最近读写的记录号，`LOF`/`EOF` 仍是字节语义；
//! - 与结构体一致，`Boolean` 按 4 字节存；`For Binary` 下 `Loc` 返回当前字节位置；
//! - `Lock #n [, start [To end]]` / `Unlock #n [, start [To end]]` 走 `LockFileEx` 的字节范围锁
//!   （`For Binary` 下是 1 基字节偏移，`For Random` 下是记录号，与 `Get`/`Put` 一致；
//!   省略范围表示整文件，只给 `start` 表示锁到文件末尾，范围冲突报错误号 70）；
//!   `Width #n, w` 让 `Print #` 按列自动换行（0 关闭），`Print` 里可用 `Spc(n)`/`Tab(n)`；
//!   `,` 分隔符按 VB 的 14 列打印区排版（输出推进到下一个区的开头，续行时从当前列算起），
//!   `;` 分隔不补位，`Print #n, ...;` 末尾的分号表示不换行。
//!   `Write #n, ...` 按 VB 的 CSV 规则写出（字符串加引号并把内部 `"` 双写、布尔写 `#True#`、
//!   `Null` 写 `#NULL#`、字段用逗号分隔、行尾 `\r\n`），`Input #` 能把这类字段读回来；
//!   `Mid(变量, 起点[, 长度]) = 值` 是就地替换字符串的语句（不会改变字符串长度）。
//!
//! 日期时间内置 `Now`/`Date`/`Time`/`DateSerial`/`TimeSerial`/`DateAdd`/`DateDiff`/`Year`/
//! `Month`/`Day`/`Hour`/`Minute`/`Second`/`Weekday`/`WeekdayName`/`MonthName`/`DateValue`/
//! `TimeValue`，`Format` 也支持日期格式串。
//! 日常小工具常备的文件系统接口：`Dir`（带 `*`/`?` 通配符，无参续取）、`FileLen`、
//! `FileDateTime`、`CurDir`/`ChDir`、`MkDir`/`RmDir`、`FileCopy`、`Name … As`（改名）、
//! `Kill`（支持通配符），这些失败时按 VB 填 `Err.Number`（53/76/70/58/75）。
//! 另有 `Environ`、`IsEmpty`/`IsNull`/`IsObject`/`IsDate`、`StrReverse`/`StrComp`/`Filter`、
//! `AscW`/`ChrW`、`Randomize`、`DoEvents`（宿主是 GUI 程序，只让出时间片，不派发消息）。
//! `Is` 是对象同一性运算符（`If obj Is Nothing`、`a Is b`，非对象操作数报错误号 424）；
//! `Option Compare Text` 让字符串的 `=`/`<`/`>`、`Select Case`、`StrComp`/`Filter` 的默认比较
//! 都忽略大小写（默认 `Binary`，与 VB 一致）。

pub mod ast;
pub mod builtins;
pub mod debug;
pub mod dialog;
pub mod error;
pub mod ffi;
pub mod filelock;
pub mod include;
pub mod lexer;
pub mod parser;
pub mod runtime;
pub mod sema;
pub mod trampoline;
pub mod value;

pub use error::{Error, ParseError};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// 运行限制，避免脚本把编辑器拖死。
#[derive(Clone, Debug)]
pub struct RunOptions {
    /// 最多执行多少条语句/表达式。
    pub max_steps: u64,
    /// 过程调用的最大嵌套层数。
    pub max_depth: usize,
    /// `Command()` 返回的命令行参数。
    pub command_line: String,
    /// 脚本文件路径：文件操作里的相对路径按它所在目录解析，`App.Path` 也返回这个目录。
    pub script_path: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_steps: 20_000_000,
            max_depth: 200,
            command_line: String::new(),
            script_path: None,
        }
    }
}

/// 解析并执行脚本；`out` 接收 `Print` / `Debug.Print` 的每一行。
#[allow(dead_code)] // 不带运行参数时的便捷入口；界面统一走 [`run_with`]。
pub fn run<F>(source: &str, out: F, interrupt: &AtomicBool) -> Result<(), Error>
where
    F: FnMut(&str),
{
    run_with(source, out, interrupt, RunOptions::default())
}

/// 带运行限制的 [`run`]。
pub fn run_with<F>(
    source: &str,
    mut out: F,
    interrupt: &AtomicBool,
    options: RunOptions,
) -> Result<(), Error>
where
    F: FnMut(&str),
{
    let (program, map) = parse_with_source_map(source, options.script_path.as_deref())?;
    runtime::run_program(&program, &mut out, interrupt, &options, None)
        .map_err(|error| Error::Runtime(remap_runtime(error, &map)))?;
    Ok(())
}

/// 带调试器的 [`run`]：命中断点或界面请求暂停时，解释器会停下来并把
/// 行号与变量快照写进 `debug`，直到界面请求继续/单步/停止。
pub fn run_debug<F>(
    source: &str,
    mut out: F,
    interrupt: &AtomicBool,
    options: RunOptions,
    debug: Arc<debug::DebugControl>,
) -> Result<(), Error>
where
    F: FnMut(&str),
{
    let (program, map) = parse_with_source_map(source, options.script_path.as_deref())?;
    let outcome = runtime::run_program(
        &program,
        &mut out,
        interrupt,
        &options,
        Some(debug.clone()),
    )
    .map_err(|error| Error::Runtime(remap_runtime(error, &map)));
    debug.finish();
    outcome?;
    Ok(())
}

/// 只做语法检查，不执行；不展开 `#Include`（没有脚本路径，无法解析相对路径）。
#[allow(dead_code)] // 供测试与不需要 include 的轻量检查使用。
pub fn parse(source: &str) -> Result<ast::Program, ParseError> {
    parser::parse(source)
}

/// 解析脚本并展开 `#Include`，同时返回行号映射。
pub fn parse_with_source_map(
    source: &str,
    path: Option<&Path>,
) -> Result<(ast::Program, include::SourceMap), Error> {
    let expanded = include::expand(source, path)
        .map_err(|error| Error::Parse(ParseError::new(error.message, error.line)))?;
    let program = parser::parse_mapped(&expanded.text, &expanded.logical_lines)
        .map_err(|error| remap_parse(error, &expanded.map))?;
    Ok((program, expanded.map))
}

/// 解析脚本并展开 `#Include`（相对路径按 `path` 所在目录解析）。
#[allow(dead_code)] // 方便只需要语法树、不需要行号映射的调用方。
pub fn parse_with_path(source: &str, path: Option<&Path>) -> Result<ast::Program, Error> {
    parse_with_source_map(source, path).map(|(program, _)| program)
}

fn remap_parse(error: ParseError, map: &include::SourceMap) -> Error {
    let (line, message) = map.remap(error.line, &error.message);
    Error::Parse(ParseError { line, message })
}

fn remap_runtime(mut error: error::RuntimeError, map: &include::SourceMap) -> error::RuntimeError {
    let (line, message) = map.remap(error.line, &error.message);
    error.line = line;
    error.message = message;
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn run_lines(source: &str) -> Result<Vec<String>, Error> {
        let mut lines = Vec::new();
        let flag = AtomicBool::new(false);
        run(source, |line| lines.push(line.to_string()), &flag)?;
        Ok(lines)
    }

    fn output(source: &str) -> Vec<String> {
        run_lines(source).expect("脚本应当执行成功")
    }

    #[test]
    fn arithmetic_precedence() {
        assert_eq!(output("Debug.Print 1 + 2 * 3"), ["7"]);
        assert_eq!(output("Debug.Print (1 + 2) * 3"), ["9"]);
        assert_eq!(output("Debug.Print 2 ^ 3 ^ 2"), ["512"]);
        assert_eq!(output("Debug.Print -2 ^ 2"), ["-4"]);
        assert_eq!(output("Debug.Print 7 \\ 2"), ["3"]);
        assert_eq!(output("Debug.Print 7 Mod 2"), ["1"]);
    }

    #[test]
    fn strings_are_case_insensitive_and_concatenated() {
        assert_eq!(
            output("Dim name As String\nname = \"GPUI\"\nDebug.Print \"hello, \" & name"),
            ["hello, GPUI"]
        );
        assert_eq!(output("Debug.Print 1 & 2"), ["12"]);
    }

    #[test]
    fn if_elseif_else_and_single_line() {
        let source = r#"
Dim score As Long
score = 85
If score >= 90 Then
    Debug.Print "A"
ElseIf score >= 80 Then
    Debug.Print "B"
Else
    Debug.Print "C"
End If
If score > 60 Then Debug.Print "pass" Else Debug.Print "fail"
"#;
        assert_eq!(output(source), ["B", "pass"]);
    }

    #[test]
    fn loops_with_step_and_exit() {
        let source = r#"
Dim total As Long
Dim i As Long
For i = 1 To 5
    total = total + i
Next i
Debug.Print total

Dim j As Long
j = 0
Do While j < 10
    j = j + 2
    If j = 6 Then Exit Do
Loop
Debug.Print j

Dim k As Long
k = 0
While k < 3
    k = k + 1
Wend
Debug.Print k
"#;
        assert_eq!(output(source), ["15", "6", "3"]);
    }

    #[test]
    fn select_case_patterns() {
        let source = r#"
Dim code As Long
code = 3
Select Case code
    Case 1
        Debug.Print "one"
    Case 2, 3, 4
        Debug.Print "few"
    Case Is > 10
        Debug.Print "many"
    Case Else
        Debug.Print "other"
End Select
"#;
        assert_eq!(output(source), ["few"]);
    }

    #[test]
    fn functions_byref_and_recursion() {
        let source = r#"
Function Factorial(ByVal n As Long) As Long
    If n <= 1 Then
        Factorial = 1
    Else
        Factorial = n * Factorial(n - 1)
    End If
End Function

Sub Bump(x As Long)
    x = x + 1
End Sub

Dim value As Long
value = 5
Bump value
Debug.Print value
Debug.Print Factorial(5)
"#;
        assert_eq!(output(source), ["6", "120"]);
    }

    #[test]
    fn builtins_behave_like_vb() {
        assert_eq!(output("Debug.Print Mid(\"abcdef\", 2, 3)"), ["bcd"]);
        assert_eq!(output("Debug.Print Left(\"abcdef\", 2)"), ["ab"]);
        assert_eq!(output("Debug.Print Right(\"abcdef\", 2)"), ["ef"]);
        assert_eq!(output("Debug.Print InStr(\"abcdef\", \"cd\")"), ["3"]);
        assert_eq!(output("Debug.Print UCase(\"abc\")"), ["ABC"]);
        assert_eq!(output("Debug.Print Len(\"中文\")"), ["2"]);
        assert_eq!(output("Debug.Print Val(\" 12abc\")"), ["12"]);
        assert_eq!(output("Debug.Print Hex(255)"), ["FF"]);
    }

    #[test]
    fn format_covers_numbers_percent_and_sections() {
        assert_eq!(output("Debug.Print Format(1234.5, \"#,##0.00\")"), ["1,234.50"]);
        assert_eq!(output("Debug.Print Format(0.256, \"0.0%\")"), ["25.6%"]);
        assert_eq!(output("Debug.Print Format(-12.3, \"0.00;(0.00)\")"), ["(12.30)"]);
        assert_eq!(output("Debug.Print Format(7, \"0000\")"), ["0007"]);
        assert_eq!(output("Debug.Print Format(1.5, \"0.##\")"), ["1.5"]);
        assert_eq!(output("Debug.Print Format(\"abc\", \">\")"), ["ABC"]);
        assert_eq!(output("Debug.Print Format(12.345, \"0.0\")"), ["12.3"]);
    }

    #[test]
    fn split_join_and_array_indexing() {
        let source = r#"
Dim parts As Variant
parts = Split("a,b,c", ",")
Debug.Print UBound(parts)
Debug.Print LBound(parts)
Debug.Print parts(0)
Debug.Print Join(parts, "-")
parts(1) = "B"
Debug.Print Join(parts, "-")
Debug.Print IsArray(parts)
Debug.Print Join(Split("a b  c", " "), "|")
"#;
        assert_eq!(
            output(source),
            ["2", "0", "a", "a-b-c", "a-B-c", "True", "a|b||c"]
        );
    }

    #[test]
    fn array_index_out_of_range_reports_line() {
        let error = run_lines("Dim items As Variant\nitems = Split(\"a,b\", \",\")\nDebug.Print items(5)\n")
            .unwrap_err();
        assert!(error.message().contains("超出"), "{error}");
        assert_eq!(error.line(), 3);
    }

    #[test]
    fn arrays_declare_and_index() {
        let source = r#"
Dim a(1 To 3) As Long
Dim i As Long
For i = 1 To 3
    a(i) = i * 10
Next i
Debug.Print a(1)
Debug.Print a(3)
Debug.Print LBound(a)
Debug.Print UBound(a)

Dim scores(2) As Long
Debug.Print scores(0)
Debug.Print UBound(scores)
Debug.Print TypeName(scores)

Dim neg(-2 To 2) As Long
neg(-2) = 9
Debug.Print neg(-2)
Debug.Print LBound(neg)
"#;
        assert_eq!(
            output(source),
            ["10", "30", "1", "3", "0", "2", "Long()", "9", "-2"]
        );
    }

    #[test]
    fn multi_dimensional_arrays() {
        let source = r#"
Dim grid(1 To 2, 1 To 3) As Long
grid(2, 3) = 42
Debug.Print grid(2, 3)
Debug.Print UBound(grid, 1)
Debug.Print UBound(grid, 2)
grid(1, 1) = 7
Debug.Print grid(1, 1)
"#;
        assert_eq!(output(source), ["42", "2", "3", "7"]);
    }

    #[test]
    fn redim_preserve_and_erase() {
        let source = r#"
Dim a() As Long
ReDim a(2)
a(0) = 1
a(1) = 2
a(2) = 3
ReDim Preserve a(4)
Debug.Print a(0) & a(1) & a(2)
Debug.Print UBound(a)

Dim b() As Long
ReDim b(1)
Erase b
"#;
        assert_eq!(output(source), ["123", "4"]);
    }

    #[test]
    fn erase_static_array_resets_values() {
        let source = r#"
Dim a(1) As Long
a(0) = 5
a(1) = 6
Erase a
Debug.Print a(0) & a(1)
Debug.Print UBound(a)
"#;
        assert_eq!(output(source), ["00", "1"]);
    }

    #[test]
    fn erase_dynamic_array_requires_redim() {
        let error = run_lines("Dim a() As Long\nReDim a(1)\nErase a\nDebug.Print UBound(a)\n")
            .unwrap_err();
        assert!(error.message().contains("尚未初始化"), "{error}");
        assert_eq!(error.line(), 4);
    }

    #[test]
    fn static_array_cannot_be_redimmed() {
        let error = run_lines("Dim a(2) As Long\nReDim a(5)\n").unwrap_err();
        assert!(error.message().contains("静态数组"), "{error}");
        assert_eq!(error.line(), 2);
    }

    #[test]
    fn for_each_iterates_array_items() {
        let source = r#"
Dim items As Variant
items = Split("a,b,c", ",")
Dim item As Variant
Dim joined As String
For Each item In items
    joined = joined & item
Next
Debug.Print joined

Dim total As Long
For Each item In Array(1, 2, 3)
    total = total + item
Next item
Debug.Print total
"#;
        assert_eq!(output(source), ["abc", "6"]);
    }

    #[test]
    fn array_assignment_copies_elements() {
        let source = r#"
Dim a As Variant
Dim b As Variant
a = Split("x,y", ",")
b = a
b(0) = "Z"
Debug.Print a(0)
Debug.Print b(0)

c = a
c(1) = "W"
Debug.Print a(1)
Debug.Print c(1)

ReDim b(4)
Debug.Print UBound(a)
Erase b
Debug.Print UBound(a)
"#;
        assert_eq!(output(source), ["x", "Z", "y", "W", "1", "1"]);
    }

    #[test]
    fn byval_array_parameter_gets_a_copy() {
        let source = r#"
Sub Mutate(ByVal arr() As Long)
    arr(0) = 99
End Sub

Dim a(1) As Long
a(0) = 1
Mutate a
Debug.Print a(0)
"#;
        assert_eq!(output(source), ["1"]);
    }

    #[test]
    fn array_parameters_are_shared_by_reference() {
        let source = r#"
Sub Fill(arr() As Long, count As Long)
    Dim i As Long
    For i = 0 To count - 1
        arr(i) = i + 1
    Next i
End Sub

Function Total(arr() As Long) As Long
    Dim i As Long
    For i = LBound(arr) To UBound(arr)
        Total = Total + arr(i)
    Next i
End Function

Dim a(0 To 3) As Long
Fill a, 4
Debug.Print a(3)
Debug.Print Total(a)
"#;
        assert_eq!(output(source), ["4", "10"]);
    }

    #[test]
    fn array_element_type_is_enforced() {
        let source = r#"
Dim a(1) As Long
a(0) = 1.7
Debug.Print a(0)
Dim s(1) As String
s(0) = 42
Debug.Print s(0)
"#;
        assert_eq!(output(source), ["2", "42"]);
    }

    #[test]
    fn array_element_is_passed_by_ref() {
        let source = r#"
Sub Bump(x As Long)
    x = x + 1
End Sub

Sub NoBump(ByVal x As Long)
    x = x + 1
End Sub

Dim a(2) As Long
a(0) = 5
Bump a(0)
Debug.Print a(0)
NoBump a(1)
Debug.Print a(1)
"#;
        assert_eq!(output(source), ["6", "0"]);
    }

    #[test]
    fn variant_can_switch_between_array_and_scalar() {
        let source = r#"
Dim v As Variant
v = Split("a,b", ",")
Debug.Print IsArray(v)
v = 5
Debug.Print IsArray(v)
Debug.Print v
"#;
        assert_eq!(output(source), ["True", "False", "5"]);
    }

    #[test]
    fn array_whole_assignment_is_rejected() {
        let error = run_lines("Dim a(2) As Long\na = 5\n").unwrap_err();
        assert!(error.message().contains("数组"), "{error}");
        assert_eq!(error.line(), 2);
    }

    #[test]
    fn type_suffix_declares_storage() {
        let source = r#"
Dim s$
Dim d#
s = 12
d = 1.5
Debug.Print TypeName(s)
Debug.Print s
Debug.Print TypeName(d)
Debug.Print IsNumeric("12.5")
Debug.Print IsNumeric("abc")
"#;
        assert_eq!(
            output(source),
            ["String", "12", "Double", "True", "False"]
        );
    }

    #[test]
    fn optional_parameter_defaults_apply() {
        let source = r#"
Function Greet(name As String, Optional greeting As String = "Hello") As String
    Greet = greeting & ", " & name
End Function

Debug.Print Greet("VB")
Debug.Print Greet("VB", "Hi")
"#;
        assert_eq!(output(source), ["Hello, VB", "Hi, VB"]);
    }

    #[test]
    fn type_definitions_and_members() {
        let source = r#"
Type Point
    X As Long
    Y As Long
End Type

Dim p As Point
p.X = 3
p.Y = 4
Debug.Print p.X + p.Y
Debug.Print TypeName(p)
Debug.Print IsNumeric(p.X)

Dim origin As Point
Debug.Print origin.X
"#;
        assert_eq!(output(source), ["7", "Point", "True", "0"]);
    }

    #[test]
    fn nested_records_and_with_block() {
        let source = r#"
Type Point
    X As Long
    Y As Long
End Type
Type Line
    A As Point
    B As Point
End Type

Dim l As Line
l.A.X = 1
l.B.Y = 2
Debug.Print l.A.X + l.B.Y

With l.A
    .X = 10
    .Y = 20
    .X = .X + 1
End With
Debug.Print l.A.X + l.A.Y
Debug.Print l.A.X
"#;
        assert_eq!(output(source), ["3", "31", "11"]);
    }

    #[test]
    fn record_assignment_copies_by_value() {
        let source = r#"
Type Point
    X As Long
End Type

Dim a As Point
Dim b As Point
a.X = 1
b = a
b.X = 2
Debug.Print a.X
Debug.Print b.X
"#;
        assert_eq!(output(source), ["1", "2"]);
    }

    #[test]
    fn record_fields_can_be_passed_by_ref() {
        let source = r#"
Type Point
    X As Long
    Y As Long
End Type
Type Line
    A As Point
    B As Point
End Type

Sub Bump(v As Long)
    v = v + 1
End Sub

Dim p As Point
p.X = 5
Bump p.X
Debug.Print p.X

With p
    Bump .Y
End With
Debug.Print p.Y

Dim l As Line
With l.A
    .X = 10
    Bump .X
End With
Debug.Print l.A.X
Bump l.B.Y
Debug.Print l.B.Y
"#;
        assert_eq!(output(source), ["6", "1", "11", "1"]);
    }

    #[test]
    fn byval_record_parameter_gets_a_copy() {
        let source = r#"
Type Point
    X As Long
End Type

Sub Reset(ByVal p As Point)
    p.X = 99
End Sub

Dim p As Point
p.X = 1
Reset p
Debug.Print p.X
"#;
        assert_eq!(output(source), ["1"]);
    }

    #[test]
    fn record_fields_use_declared_type() {
        let source = r#"
Type Item
    Name As String
    Count As Long
End Type
Dim it As Item
it.Name = 12
it.Count = 2.6
Debug.Print it.Name
Debug.Print it.Count
"#;
        assert_eq!(output(source), ["12", "3"]);
    }

    #[test]
    fn records_pass_by_reference() {
        let source = r#"
Type Point
    X As Long
    Y As Long
End Type

Sub Flip(p As Point)
    Dim t As Long
    t = p.X
    p.X = p.Y
    p.Y = t
End Sub

Dim p As Point
p.X = 1
p.Y = 2
Flip p
Debug.Print p.X & "," & p.Y
"#;
        assert_eq!(output(source), ["2,1"]);
    }

    #[test]
    fn unknown_member_reports_line() {
        let error = run_lines(
            "Type Point\n    X As Long\nEnd Type\nDim p As Point\nDebug.Print p.Z\n",
        )
        .unwrap_err();
        assert!(error.message().contains("没有字段"), "{error}");
        assert_eq!(error.line(), 5);
    }

    #[test]
    fn with_requires_a_record_value() {
        let error = run_lines("Dim x As Long\nWith x\n    .Y = 1\nEnd With\n").unwrap_err();
        assert!(error.message().contains("With"), "{error}");
        assert_eq!(error.line(), 2);
    }

    #[test]
    fn with_block_is_not_visible_inside_callees() {
        let source = r#"
Type Point
    X As Long
End Type

Sub Stray()
    .X = 1
End Sub

Dim p As Point
With p
    Stray
End With
"#;
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("With"), "{error}");
        assert_eq!(error.line(), 7);
    }

    #[test]
    fn udt_arrays_support_members_and_copies() {
        let source = r#"
Type Point
    X As Long
    Y As Long
End Type

Dim pts(2) As Point
Dim i As Long
For i = 0 To UBound(pts)
    pts(i).X = i * 10
    pts(i).Y = i
Next i
Debug.Print pts(2).X + pts(2).Y
Debug.Print TypeName(pts)
Debug.Print IsArray(pts)

Dim copy As Variant
copy = pts
copy(1).X = 99
Debug.Print pts(1).X
Debug.Print copy(1).X

Dim dyn() As Point
ReDim dyn(1)
dyn(0).X = 7
Debug.Print dyn(0).X
Debug.Print LBound(dyn)
"#;
        assert_eq!(
            output(source),
            ["22", "Point()", "True", "10", "99", "7", "0"]
        );
    }

    #[test]
    fn udt_array_elements_reject_plain_values() {
        let error =
            run_lines("Type P\n    X As Long\nEnd Type\nDim a(1) As P\na(0) = 5\n").unwrap_err();
        assert!(error.message().contains("必须是"), "{error}");
        assert_eq!(error.line(), 5);
    }

    #[test]
    fn goto_and_labels() {
        let source = r#"
Dim i As Long
i = 0
Again:
i = i + 1
If i < 3 Then GoTo Again
Debug.Print i
"#;
        assert_eq!(output(source), ["3"]);
    }

    #[test]
    fn goto_can_leave_a_loop() {
        let source = r#"
Dim i As Long
For i = 1 To 5
    If i = 2 Then GoTo Done
Next i
Done:
Debug.Print i
"#;
        assert_eq!(output(source), ["2"]);
    }

    #[test]
    fn goto_unknown_label_reports_line() {
        let error = run_lines("Debug.Print 1\nGoTo Nowhere\n").unwrap_err();
        assert!(error.message().contains("找不到标签"), "{error}");
        assert_eq!(error.line(), 2);
    }

    #[test]
    fn on_error_resume_next_records_err() {
        let source = r#"
Dim x As Long
On Error Resume Next
x = 1 / 0
Debug.Print Err.Number <> 0
Debug.Print Err.Description
Err.Clear
Debug.Print Err.Number
Debug.Print x
"#;
        assert_eq!(output(source), ["True", "除数为 0", "0", "0"]);
    }

    #[test]
    fn on_error_goto_handler_and_resume_next() {
        let source = r#"
Sub Main()
    Dim hits As Long
    On Error GoTo Handler
    Dim x As Long
    x = 1 / 0
    Debug.Print "after"
    Exit Sub
Handler:
    hits = hits + 1
    If hits > 1 Then Exit Sub
    Debug.Print "handled: " & Err.Description
    Resume Next
End Sub
"#;
        assert_eq!(output(source), ["handled: 除数为 0", "after"]);
    }

    #[test]
    fn on_error_goto_0_disables_the_handler() {
        let error = run_lines(
            "Sub Main()\n    On Error Resume Next\n    On Error GoTo 0\n    Dim x As Long\n    x = 1 / 0\nEnd Sub\n",
        )
        .unwrap_err();
        assert!(error.message().contains("除数为 0"), "{error}");
        assert_eq!(error.line(), 5);
    }

    #[test]
    fn err_is_per_script_and_clears() {
        let source = r#"
Err.Clear
Debug.Print Err.Number
Debug.Print Len(Err.Description)
"#;
        assert_eq!(output(source), ["0", "0"]);
    }

    #[test]
    fn file_io_roundtrip() {
        let path = std::env::temp_dir().join("vb_interp_roundtrip.txt");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Output As #1
Print #1, "hello"
Print #1, 42
Close #1

Dim n As Long
n = FreeFile
Open "{display}" For Input As #n
Dim first As String
Line Input #n, first
Debug.Print first
Debug.Print EOF(n)
Debug.Print LOF(n) > 0
Close #n
"#
        );
        assert_eq!(output(&source), ["hello", "False", "True"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn input_hash_reads_numbers_and_strings() {
        let path = std::env::temp_dir().join("vb_interp_input.txt");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Output As #1
Print #1, "10,20,hello"
Print #1, "3,4,""quoted, value"""
Close

Open "{display}" For Input As #1
Dim a As Long
Dim b As Long
Dim s As String
Input #1, a, b, s
Debug.Print a + b
Debug.Print s
Input #1, a, b, s
Debug.Print a * b
Debug.Print s
Close
"#
        );
        assert_eq!(output(&source), ["30", "hello", "12", "quoted, value"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_errors_are_reported() {
        let path = std::env::temp_dir().join("vb_interp_missing_xyz.txt");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!("Open \"{display}\" For Input As #1\n");
        let error = run_lines(&source).unwrap_err();
        assert!(error.message().contains("无法打开"), "{error}");
        assert_eq!(error.line(), 1);
    }

    #[test]
    fn binary_put_get_roundtrip() {
        let path = std::env::temp_dir().join("vb_interp_binary.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Dim l As Long
Dim d As Double
Dim i As Integer
l = 305419896
d = 3.5
i = -2
Open "{display}" For Binary As #1
Put #1, , l
Put #1, , d
Put #1, , i
Debug.Print LOF(1)
Debug.Print Seek(1)
Debug.Print EOF(1)
Seek #1, 1
Debug.Print Seek(1)
Dim rl As Long
Dim rd As Double
Dim ri As Integer
Get #1, , rl
Get #1, , rd
Get #1, , ri
Debug.Print rl
Debug.Print rd
Debug.Print ri
Debug.Print EOF(1)
Close #1
"#
        );
        assert_eq!(
            output(&source),
            ["14", "15", "True", "1", "305419896", "3.5", "-2", "True"]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn binary_get_uses_explicit_position() {
        let path = std::env::temp_dir().join("vb_interp_binary_pos.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Dim v As Long
Dim back As Long
v = 12345
Open "{display}" For Binary As #1
Put #1, 3, v
Debug.Print LOF(1)
Get #1, 3, back
Debug.Print back
Debug.Print Seek(1)
Close #1
"#
        );
        assert_eq!(output(&source), ["6", "12345", "7"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn binary_get_rejects_unsupported_types() {
        let path = std::env::temp_dir().join("vb_interp_binary_bad.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            "Dim s As String\nOpen \"{display}\" For Binary As #1\nPut #1, , s\n"
        );
        let error = run_lines(&source).unwrap_err();
        assert!(error.message().contains("String"), "{error}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn long_overflow_reports_vb_error_6() {
        let source = r#"
Dim n As Long
On Error Resume Next
n = 2147483647
n = n + 1
Debug.Print Err.Number
Debug.Print Err.Description
n = 300
Dim b As Byte
b = n
Debug.Print Err.Number
"#;
        assert_eq!(output(source), ["6", "溢出", "6"]);
    }

    #[test]
    fn err_raise_sets_number_source_and_description() {
        let source = r#"
On Error Resume Next
Err.Raise 53, "存储", "自定义错误"
Debug.Print Err.Number
Debug.Print Err.Description
Debug.Print Err.Source
Err.Clear
Debug.Print Err.Number
Err.Raise 11
Debug.Print Err.Number
Debug.Print Err.Description
Err.Clear
Debug.Print Error(5)
Debug.Print Len(Error())
"#;
        assert_eq!(
            output(source),
            [
                "53",
                "自定义错误",
                "存储",
                "0",
                "11",
                "除数为零",
                "无效的过程调用或参数",
                "0"
            ]
        );
    }

    #[test]
    fn runtime_errors_carry_vb_numbers() {
        let source = r#"
On Error Resume Next
Dim x As Long
x = 1 / 0
Debug.Print Err.Number
On Error Resume Next
Dim a(2) As Long
x = a(5)
Debug.Print Err.Number
On Error Resume Next
Open "Z:\nope\nothing.txt" For Input As #1
Debug.Print Err.Number
"#;
        assert_eq!(output(source), ["11", "9", "53"]);
    }

    #[test]
    fn unhandled_err_raise_stops_with_its_description() {
        let error = run_lines("Err.Raise 53\n").unwrap_err();
        assert!(error.message().contains("找不到文件"), "{error}");
        assert_eq!(error.line(), 1);
    }

    #[test]
    fn err_raise_accepts_skipped_and_extra_arguments() {
        let source = r#"
On Error Resume Next
Err.Raise 5, , "只有描述"
Debug.Print Err.Number
Debug.Print Err.Description
Err.Clear
Err.Raise 5, "src", "desc", "help.chm", 1000
Debug.Print Err.Source
Debug.Print Err.Description
"#;
        assert_eq!(output(source), ["5", "只有描述", "src", "desc"]);
    }

    #[test]
    fn binary_get_reads_existing_bytes() {
        let path = std::env::temp_dir().join("vb_interp_binary_read.dat");
        std::fs::write(&path, [1u8, 0, 0, 0, 0xFF, 0, 0, 0]).unwrap();
        let display = path.display().to_string();
        let source = format!(
            r#"
Dim a As Long
Dim b As Long
Open "{display}" For Binary As #1
Get #1, , a
Get #1, , b
Debug.Print a
Debug.Print b
Debug.Print LOF(1)
Close #1
"#
        );
        assert_eq!(output(&source), ["1", "255", "8"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn get_and_put_require_binary_mode() {
        let path = std::env::temp_dir().join("vb_interp_not_binary.txt");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            "Dim v As Long\nOpen \"{display}\" For Output As #1\nPut #1, , v\nClose #1\n"
        );
        let error = run_lines(&source).unwrap_err();
        assert!(error.message().contains("Binary"), "{error}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn date_serial_functions_work() {
        let source = r#"
Dim d As Double
d = DateSerial(2024, 5, 6)
Debug.Print Format(d, "yyyy-mm-dd")
Debug.Print Year(d)
Debug.Print Month(d)
Debug.Print Day(d)
Debug.Print Format(DateAdd("d", 2, d), "yyyy-mm-dd")
Debug.Print DateDiff("d", d, DateSerial(2024, 5, 16))
Debug.Print Format(DateAdd("m", 9, DateSerial(2024, 1, 31)), "yyyy-mm-dd")
Debug.Print Format(DateSerial(1899, 12, 30), "yyyy-mm-dd")
"#;
        assert_eq!(
            output(source),
            [
                "2024-05-06",
                "2024",
                "5",
                "6",
                "2024-05-08",
                "10",
                "2024-10-31",
                "1899-12-30"
            ]
        );
    }

    #[test]
    fn now_and_time_format_work() {
        let source = r#"
Debug.Print Year(Now()) >= 2024
Debug.Print Len(Format(Now(), "yyyy-mm-dd hh:nn:ss")) = 19
Debug.Print TimeSerial(13, 30, 0) > 0.5
"#;
        assert_eq!(output(source), ["True", "True", "True"]);
    }

    #[test]
    fn date_errors_are_reported() {
        let error = run_lines("Debug.Print DateSerial(2024, 13, 1)\n").unwrap_err();
        assert!(error.message().contains("月份"), "{error}");
        let error = run_lines("Debug.Print DateSerial(2023, 2, 29)\n").unwrap_err();
        assert!(error.message().contains("日期"), "{error}");
        let error = run_lines("Debug.Print DateAdd(\"x\", 1, 0)\n").unwrap_err();
        assert!(error.message().contains("间隔"), "{error}");
    }

    #[test]
    fn procedures_use_static_scoping() {
        // 过程看不到调用者的局部变量（旧实现是动态作用域，会意外看到）。
        let source = r#"
Sub Inner()
    Debug.Print x
End Sub

Sub Outer()
    Dim x As Long
    x = 42
    Inner
End Sub

Outer
"#;
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("未定义的变量 `x`"), "{error}");

        // 模块级变量对所有过程可见。
        let source = r#"
Dim total As Long

Sub Add(n As Long)
    total = total + n
End Sub

Add 2
Add 3
Debug.Print total
"#;
        assert_eq!(output(source), ["5"]);
    }

    #[test]
    fn option_explicit_requires_declaration() {
        let source = "Option Explicit\nDim x As Long\nx = 3\nDebug.Print x\n";
        assert_eq!(output(source), ["3"]);

        let error = run_lines("Option Explicit\ny = 3\n").unwrap_err();
        assert!(error.message().contains("未声明"), "{error}");
        assert_eq!(error.line(), 2);

        let error =
            run_lines("Option Explicit\nSub Foo()\n    z = 1\nEnd Sub\nFoo\n").unwrap_err();
        assert!(error.message().contains("未声明"), "{error}");
        assert_eq!(error.line(), 3);
    }

    #[test]
    fn option_base_changes_default_lower_bound() {
        let source = r#"
Option Base 1
Dim a(3)
Debug.Print LBound(a)
Debug.Print UBound(a)
Dim b(1 To 2)
Debug.Print LBound(b)
"#;
        assert_eq!(output(source), ["1", "3", "1"]);
    }

    #[test]
    fn static_locals_keep_value_between_calls() {
        let source = r#"
Function Counter() As Long
    Static n As Long
    n = n + 1
    Counter = n
End Function

Debug.Print Counter()
Debug.Print Counter()
Debug.Print Counter()
"#;
        assert_eq!(output(source), ["1", "2", "3"]);
    }

    #[test]
    fn declared_scalars_start_at_default_value() {
        let source = "Dim n As Long\nDim s As String\nDim b As Boolean\nDebug.Print n\nDebug.Print Len(s)\nDebug.Print b\n";
        assert_eq!(output(source), ["0", "0", "False"]);
    }

    #[test]
    fn constants_are_read_only() {
        let error = run_lines("Const max = 10\nmax = 11\n").unwrap_err();
        assert!(error.message().contains("常量"), "{error}");
        assert_eq!(error.line(), 2);
    }

    #[test]
    fn sub_main_runs_when_no_top_level_code() {
        let source = r#"
Sub Main()
    Debug.Print "from main"
End Sub
"#;
        assert_eq!(output(source), ["from main"]);
    }

    #[test]
    fn syntax_error_reports_line() {
        let error = run_lines("Dim x As Long\nx = \n").unwrap_err();
        assert_eq!(error.line(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn multiline_declare_and_arguments_are_allowed() {
        let source = r#"
Declare Function GetModuleHandleW Lib "kernel32" (
    ByVal name As LongPtr
) As LongPtr

Dim handle As LongPtr
handle = GetModuleHandleW(
    0
)
Debug.Print handle <> 0
"#;
        assert_eq!(output(source), ["True"]);
    }

    #[cfg(windows)]
    #[test]
    fn declare_can_call_win32() {        let source = r#"
Declare Function GetTickCount Lib "kernel32" () As Long
Declare Function GetModuleHandleW Lib "kernel32" (ByVal name As LongPtr) As LongPtr
Declare Function lstrlenW Lib "kernel32" (ByVal text As String) As Long

Dim ticks As Long
ticks = GetTickCount()
Debug.Print ticks > 0

Dim handle As LongPtr
handle = GetModuleHandleW(0)
Debug.Print handle <> 0

Debug.Print lstrlenW("hello")
"#;
        assert_eq!(output(source), ["True", "True", "5"]);
    }

    #[cfg(windows)]
    #[test]
    fn unknown_library_reports_error() {
        let source = "Declare Function Nope Lib \"no_such_lib_xyz\" () As Long\nDebug.Print Nope()";
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("无法加载库"), "{error}");
    }

    #[cfg(windows)]
    #[test]
    fn declare_accepts_struct_by_ref() {
        let source = r#"
Type RECT
    Left As Long
    Top As Long
    Right As Long
    Bottom As Long
End Type

Declare Function SetRect Lib "user32" (
    ByRef rect As RECT,
    ByVal left As Long,
    ByVal top As Long,
    ByVal right As Long,
    ByVal bottom As Long
) As Long

Dim r As RECT
Debug.Print SetRect(r, 1, 2, 30, 40) <> 0
Debug.Print r.Right - r.Left
Debug.Print r.Bottom - r.Top
"#;
        assert_eq!(output(source), ["True", "29", "38"]);
    }

    #[cfg(windows)]
    #[test]
    fn declare_accepts_systemtime_struct() {
        let source = r#"
Type SYSTEMTIME
    Year As Integer
    Month As Integer
    DayOfWeek As Integer
    Day As Integer
    Hour As Integer
    Minute As Integer
    Second As Integer
    Milliseconds As Integer
End Type

Declare Sub GetLocalTime Lib "kernel32" (ByRef t As SYSTEMTIME)

Dim t As SYSTEMTIME
GetLocalTime t
Debug.Print t.Year > 2000
Debug.Print t.Month >= 1
Debug.Print t.Month <= 12
"#;
        assert_eq!(output(source), ["True", "True", "True"]);
    }

    #[cfg(windows)]
    #[test]
    fn struct_parameter_accepts_byref_and_byval() {
        // `ByVal` 结构体：按 x64 ABI 放进寄存器或临时副本。
        let source = r#"
Type RECT
    Left As Long
    Top As Long
    Right As Long
    Bottom As Long
End Type

Type POINT
    X As Long
    Y As Long
End Type

Declare Function SetRect Lib "user32" (ByRef rect As RECT, ByVal left As Long, ByVal top As Long, ByVal right As Long, ByVal bottom As Long) As Long
Declare Function PtInRect Lib "user32" (ByRef rect As RECT, ByVal pt As POINT) As Long

Dim r As RECT
Debug.Print SetRect(r, 1, 2, 11, 12)
Debug.Print r.Right
Dim p As POINT
p.X = 5
p.Y = 5
Debug.Print PtInRect(r, p) <> 0

p.X = 50
Debug.Print PtInRect(r, p) <> 0
"#;
        assert_eq!(output(source), ["1", "11", "True", "False"]);
    }

    #[cfg(windows)]
    #[test]
    fn struct_parameter_requires_record_value() {
        let source = r#"
Type SYSTEMTIME
    Year As Integer
    Month As Integer
    DayOfWeek As Integer
    Day As Integer
    Hour As Integer
    Minute As Integer
    Second As Integer
    Milliseconds As Integer
End Type

Declare Sub GetLocalTime Lib "kernel32" (ByRef t As SYSTEMTIME)

Dim not_a_struct As Long
GetLocalTime not_a_struct
"#;
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("SYSTEMTIME"), "{error}");
    }

    #[cfg(windows)]
    #[test]
    fn declare_supports_double_arguments_and_returns() {
        let source = r#"
Declare Function pow Lib "msvcrt" (ByVal x As Double, ByVal y As Double) As Double

Debug.Print pow(2, 10)
Debug.Print Abs(pow(9, 0.5) - 3) < 0.000001
"#;
        assert_eq!(output(source), ["1024", "True"]);
    }

    #[cfg(windows)]
    #[test]
    fn declare_supports_single_and_mixed_arguments() {
        // 全 `Single` 参数与返回值。
        let source = r#"
Declare Function powf Lib "msvcrt" (ByVal x As Single, ByVal y As Single) As Single
Debug.Print Abs(powf(2, 3) - 8) < 0.0001
"#;
        assert_eq!(output(source), ["True"]);

        // 整数与 `Double` 混合：`ldexp(x, exp)`。
        let source = r#"
Declare Function ldexp Lib "msvcrt" (ByVal x As Double, ByVal e As Long) As Double
Debug.Print ldexp(1.5, 3)
"#;
        assert_eq!(output(source), ["12"]);

        // 超过 4 个参数的混合调用仍明确报错。
        let source = r#"
Declare Function pow Lib "msvcrt" (ByVal a As Long, ByVal b As Long, ByVal c As Long, ByVal d As Long, ByVal e As Double) As Long
Debug.Print pow(1, 2, 3, 4, 5)
"#;
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("混合"), "{error}");
    }

    #[test]
    fn runtime_error_reports_line() {
        let error = run_lines("Dim x As Long\nx = 1 / 0").unwrap_err();
        assert!(error.message().contains("除数为 0"), "{error}");
        assert_eq!(error.line(), 2);
    }

    #[test]
    fn infinite_loop_is_stopped_by_step_limit() {
        let mut lines = Vec::new();
        let flag = AtomicBool::new(false);
        let options = RunOptions {
            max_steps: 4096,
            ..RunOptions::default()
        };
        let error = run_with("Do\nLoop", |line| lines.push(line.to_string()), &flag, options)
            .unwrap_err();
        assert!(error.message().contains("步数"), "{error}");
    }

    #[test]
    fn interrupt_flag_stops_script() {
        let flag = AtomicBool::new(true);
        let mut lines = Vec::new();
        let error = run("Do\nLoop", |line| lines.push(line.to_string()), &flag).unwrap_err();
        assert!(error.message().contains("中断"), "{error}");
    }

    #[test]
    fn enum_members_become_constants() {
        let source = r#"
Enum Color
    Red
    Green = 5
    Blue
    Black = -1
End Enum

Dim c As Color
c = Blue
Debug.Print Red
Debug.Print Green
Debug.Print c
Debug.Print Black
Debug.Print TypeName(c)
"#;
        assert_eq!(output(source), ["0", "5", "6", "-1", "Long"]);
    }

    #[test]
    fn duplicate_enum_members_are_rejected() {
        let error = run_lines("Enum A\n    X = 1\nEnd Enum\nEnum B\n    X = 2\nEnd Enum\n")
            .unwrap_err();
        assert!(error.message().contains("重名"), "{error}");
        let error = run_lines("Enum A\nEnd Enum\n").unwrap_err();
        assert!(error.message().contains("至少需要一个成员"), "{error}");
    }

    #[test]
    fn paramarray_collects_remaining_arguments() {
        let source = r#"
Function Sum(ParamArray xs()) As Long
    Dim i As Long
    Dim total As Long
    For i = LBound(xs) To UBound(xs)
        total = total + xs(i)
    Next i
    Sum = total
End Function

Sub Tagged(prefix As String, ParamArray xs())
    Dim i As Long
    For i = LBound(xs) To UBound(xs)
        Debug.Print prefix & xs(i)
    Next i
End Sub

Debug.Print Sum(1, 2, 3)
Debug.Print Sum()
Debug.Print UBound(Array(1, 2)) < 0
Tagged "n=", 7, 8
"#;
        assert_eq!(output(source), ["6", "0", "False", "n=7", "n=8"]);
    }

    #[test]
    fn paramarray_must_be_last() {
        let error = run_lines("Sub S(ParamArray xs(), y As Long)\nEnd Sub\n").unwrap_err();
        assert!(error.message().contains("最后一个参数"), "{error}");
    }

    #[test]
    fn property_get_and_let_wrap_a_module_variable() {
        let source = r#"
Dim stored As Long

Property Get Total() As Long
    Total = stored
    Exit Property
End Property

Property Let Total(v As Long)
    stored = v * 2
End Property

Total = 21
Debug.Print Total
Debug.Print stored
"#;
        assert_eq!(output(source), ["42", "42"]);
    }

    #[test]
    fn indexed_property_get_and_let() {
        let source = r#"
Dim items(2) As Long

Property Get Item(index As Long) As Long
    Item = items(index)
End Property

Property Let Item(index As Long, v As Long)
    items(index) = v
End Property

Item(1) = 99
Debug.Print Item(1)
Item(2) = Item(1) + 1
Debug.Print Item(2)
"#;
        assert_eq!(output(source), ["99", "100"]);
    }

    #[test]
    fn with_events_receives_raise_event() {
        let source = r#"
Dim g_log As String

Class Timer
    Public Name As String

    Public Sub Tick(ByVal n As Long)
        RaiseEvent Ticked(n)
    End Sub
End Class

Class Monitor
    Public WithEvents mTimer As New Timer

    Private Sub Timer_Ticked(ByVal n As Long)
        g_log = g_log & mTimer.Name & ":" & n & ";"
    End Sub

    Public Sub Start(ByVal label As String)
        mTimer.Name = label
        mTimer.Tick 1
        mTimer.Tick 2
    End Sub
End Class

Dim m As New Monitor
m.Start "t"
Debug.Print g_log
"#;
        assert_eq!(output(source), ["t:1;t:2;"]);
    }

    #[test]
    fn class_terminate_runs_when_the_last_reference_goes_away() {
        let source = r#"
Dim g_log As String

Class Res
    Public Name As String

    Private Sub Class_Terminate()
        g_log = g_log & Name & ";"
    End Sub
End Class

Dim a As New Res
a.Name = "a"
Dim b As New Res
b.Name = "b"
Dim alias_b As Res
Set alias_b = b
Set b = Nothing
Debug.Print g_log
Set alias_b = Nothing
Debug.Print g_log
Dim c As New Res
c.Name = "c"
Set c = Nothing
Debug.Print g_log
Dim d As New Res
d.Name = "d"
Set d = New Res
d.Name = "e"
Debug.Print g_log
Debug.Print TypeOf d Is Res
"#;
        assert_eq!(
            output(source),
            ["", "b;", "b;c;", "b;c;d;", "True"]
        );
    }

    #[test]
    fn property_set_assigns_object_properties() {
        let source = r#"
Class Item
    Public Name As String
End Class

Class Holder
    Private mChild As Variant
    Private mSlot As Variant

    Public Property Get Child() As Variant
        Child = mChild
    End Property

    Public Property Set Child(ByVal value As Variant)
        Set mChild = value
    End Property

    Public Property Get Slot(ByVal index As Long) As Variant
        Slot = mSlot
    End Property

    Public Property Set Slot(ByVal index As Long, ByVal value As Variant)
        Set mSlot = value
    End Property
End Class

Dim h As New Holder
Dim it As New Item
it.Name = "box"
Set h.Child = it
Debug.Print h.Child.Name
Debug.Print TypeOf h.Child Is Item
Debug.Print TypeOf h.Child Is Holder
Debug.Print TypeOf it Is Item
Set h.Slot(1) = it
Debug.Print h.Slot(1).Name
Dim blank As Item
Debug.Print TypeOf blank Is Item
"#;
        assert_eq!(
            output(source),
            ["box", "True", "False", "True", "box", "False"]
        );

        // `TypeOf` 用在非对象上会报 424。
        let source = r#"
Class Item
End Class
On Error Resume Next
Debug.Print TypeOf 1 Is Item
Debug.Print Err.Number
"#;
        assert_eq!(output(source), ["424"]);
    }

    #[test]
    fn binary_put_get_udt_roundtrip() {
        let path = std::env::temp_dir().join("vb_interp_udt.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Type Point
    X As Long
    Y As Long
    Tag As Integer
End Type

Dim p As Point
p.X = 10
p.Y = -20
p.Tag = 7
Debug.Print Len(p)

Open "{display}" For Binary As #1
Put #1, , p
Debug.Print LOF(1)
Close #1

Dim q As Point
Open "{display}" For Binary As #1
Get #1, , q
Debug.Print q.X
Debug.Print q.Y
Debug.Print q.Tag
Debug.Print EOF(1)
Close #1
"#
        );
        assert_eq!(output(&source), ["12", "12", "10", "-20", "7", "True"]);

        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw.len(), 12, "{raw:?}");
        assert_eq!(&raw[0..4], &10i32.to_le_bytes());
        assert_eq!(&raw[4..8], &(-20i32).to_le_bytes());
        assert_eq!(&raw[8..10], &7i16.to_le_bytes());
        assert_eq!(&raw[10..12], &[0, 0]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn binary_put_get_nested_udt() {
        let path = std::env::temp_dir().join("vb_interp_udt_nested.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Type Inner
    A As Long
End Type

Type Outer
    I As Inner
    B As Double
End Type

Dim o As Outer
o.I.A = 5
o.B = 1.5

Open "{display}" For Binary As #1
Put #1, , o
Debug.Print LOF(1)
Close #1

Dim r As Outer
Open "{display}" For Binary As #1
Get #1, , r
Debug.Print r.I.A
Debug.Print r.B
Close #1
"#
        );
        assert_eq!(output(&source), ["16", "5", "1.5"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn binary_get_udt_reads_at_explicit_position() {
        let path = std::env::temp_dir().join("vb_interp_udt_pos.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Type Pair
    A As Long
    B As Long
End Type

Dim p As Pair
p.A = 11
p.B = 22

Open "{display}" For Binary As #1
Put #1, 3, p
Debug.Print LOF(1)
Dim q As Pair
Get #1, 3, q
Debug.Print q.A + q.B
Close #1
"#
        );
        assert_eq!(output(&source), ["10", "33"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn udt_fixed_length_string_field() {
        let path = std::env::temp_dir().join("vb_interp_udt_fixed.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Type FixedRec
    Name As String * 4
    Value As Long
End Type

Dim r As FixedRec
r.Name = "abc"
r.Value = 7
Debug.Print Len(r.Name)
Debug.Print "[" & r.Name & "]"
Debug.Print Len(r)

Open "{display}" For Binary As #1
Put #1, , r
Debug.Print LOF(1)
Close #1

Dim s As FixedRec
Open "{display}" For Binary As #1
Get #1, , s
Debug.Print "[" & s.Name & "]"
Debug.Print s.Name = "abc"
Debug.Print Trim(s.Name) = "abc"
Debug.Print s.Value
s.Name = "abcdef"
Debug.Print Len(s.Name)
Debug.Print s.Name
Close #1
"#
        );
        assert_eq!(
            output(&source),
            ["4", "[abc ]", "12", "12", "[abc ]", "False", "True", "7", "4", "abcd"]
        );

        // `String * 4` 按 UTF-16 写 8 字节：`abc` + 空格（定长字符串用空格补齐）。
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw.len(), 12, "{raw:?}");
        assert_eq!(u16::from_le_bytes([raw[0], raw[1]]), u16::from(b'a'));
        assert_eq!(u16::from_le_bytes([raw[2], raw[3]]), u16::from(b'b'));
        assert_eq!(u16::from_le_bytes([raw[4], raw[5]]), u16::from(b'c'));
        assert_eq!(u16::from_le_bytes([raw[6], raw[7]]), u16::from(b' '));
        assert_eq!(&raw[8..12], &7i32.to_le_bytes());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn udt_array_field_supports_index_read_and_write() {
        let path = std::env::temp_dir().join("vb_interp_udt_array.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Type Buffer
    Data(2) As Long
    Count As Long
End Type

Dim b As Buffer
b.Data(0) = 5
b.Data(1) = 6
b.Data(2) = 7
b.Count = b.Data(0) + b.Data(2)
Debug.Print b.Count
Debug.Print LBound(b.Data)
Debug.Print UBound(b.Data)
Debug.Print Len(b)

Open "{display}" For Binary As #1
Put #1, , b
Debug.Print LOF(1)
Close #1

Dim c As Buffer
Open "{display}" For Binary As #1
Get #1, , c
Debug.Print c.Data(1)
Debug.Print c.Count
Close #1
"#
        );
        assert_eq!(output(&source), ["12", "0", "2", "16", "16", "6", "12"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn udt_array_field_with_explicit_lower_bound() {
        let source = r#"
Type Shaded
    V(1 To 3) As Integer
End Type

Dim t As Shaded
t.V(1) = 10
t.V(3) = 30
Debug.Print LBound(t.V)
Debug.Print UBound(t.V)
Debug.Print t.V(1) + t.V(3)
Debug.Print Len(t)
"#;
        assert_eq!(output(source), ["1", "3", "40", "6"]);
    }

    #[test]
    fn udt_layout_combines_fixed_strings_and_arrays() {
        let source = r#"
Type Mixed
    Tag As String * 8
    Flags(1) As Integer
    Count As Long
End Type

Dim m As Mixed
Debug.Print Len(m)
Debug.Print LBound(m.Flags)
Debug.Print UBound(m.Flags)
"#;
        // `String * 8` 占 16 字节（对齐 2）；`Integer` 数组占 4 字节（偏移 16）；
        // `Long` 落在偏移 20，总长按 4 字节对齐到 24。
        assert_eq!(output(source), ["24", "0", "1"]);
    }

    #[test]
    fn udt_array_of_records_supports_member_assignment() {
        let source = r#"
Type Item
    V As Long
End Type

Type Box
    Items(1) As Item
    Total As Long
End Type

Dim b As Box
b.Items(0).V = 7
b.Items(1).V = 8
b.Total = b.Items(0).V + b.Items(1).V
Debug.Print b.Total
Debug.Print TypeName(b.Items)
"#;
        assert_eq!(output(source), ["15", "Item()"]);
    }

    #[test]
    fn get_accepts_array_element_and_record_field_targets() {
        let path = std::env::temp_dir().join("vb_interp_get_targets.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Type Rec
    X As Long
    Y As Long
End Type

Dim a(2) As Long
Dim r As Rec

Open "{display}" For Binary As #1
Put #1, , 111
Put #1, , 222
Put #1, , 333
Seek #1, 1
Get #1, , a(0)
Get #1, , a(2)
Get #1, , r.X
Close #1

Debug.Print a(0)
Debug.Print a(1)
Debug.Print a(2)
Debug.Print r.X

Open "{display}" For Binary As #1
Get #1, , r.Y
Debug.Print r.Y
Close #1
"#
        );
        assert_eq!(output(&source), ["111", "0", "222", "333", "111"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn get_accepts_udt_array_field_target() {
        let path = std::env::temp_dir().join("vb_interp_get_field_target.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Type Holder
    V(1) As Long
End Type

Dim h As Holder
Open "{display}" For Binary As #1
Put #1, , 9
Put #1, , 10
Seek #1, 1
Get #1, , h.V(1)
Debug.Print h.V(0)
Debug.Print h.V(1)
Close #1
"#
        );
        assert_eq!(output(&source), ["0", "9"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn get_rejects_non_variable_target() {
        let path = std::env::temp_dir().join("vb_interp_get_bad_target.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!("Open \"{display}\" For Binary As #1\nGet #1, , 5\n");
        let error = run_lines(&source).unwrap_err();
        assert!(error.message().contains("Get"), "{error}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn print_hash_wraps_at_width() {
        let path = std::env::temp_dir().join("vb_interp_width.txt");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Output As #1
Width #1, 5
Print #1, "abcdefgh"
Width #1, 0
Print #1, "abcdefgh"
Close #1
"#
        );
        assert_eq!(output(&source), Vec::<String>::new());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "abcde\nfgh\nabcdefgh\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn print_supports_spc_tab_and_trailing_semicolon() {
        let path = std::env::temp_dir().join("vb_interp_spc.txt");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Output As #1
Print #1, "a"; Spc(3); "b"
Print #1, "x"; 10; Tab(6); "y";
Close #1

Debug.Print "a"; Spc(2); "b"
Debug.Print "c", "d"
Debug.Print "e",
"#
        );
        assert_eq!(output(&source), ["a  b", "c             d", "e"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "a   b\nx10  y");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn print_comma_moves_to_the_next_fourteen_column_zone() {
        let source = r#"
Debug.Print "a", "b"
Debug.Print "12345678901234", "x"
Debug.Print "1234567890123", "x"
Debug.Print "123456789012345", "x"
Debug.Print "a", "b", "c"
"#;
        let lines = output(source);
        assert_eq!(lines[0], format!("a{}b", " ".repeat(13)));
        assert_eq!(lines[1], format!("12345678901234{}x", " ".repeat(14)));
        assert_eq!(lines[2], format!("1234567890123{}x", " "));
        assert_eq!(lines[3], format!("123456789012345{}x", " ".repeat(13)));
        assert_eq!(
            lines[4],
            format!("a{}b{}c", " ".repeat(13), " ".repeat(13))
        );
    }

    #[test]
    fn print_file_zones_count_the_current_column() {
        let path = std::env::temp_dir().join("vb_interp_zones.txt");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Output As #1
Print #1, "a";
Print #1, "b", "c"
Close #1
"#
        );
        assert_eq!(output(&source), Vec::<String>::new());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, format!("ab{}c\n", " ".repeat(12)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lock_and_unlock_a_file() {
        let path = std::env::temp_dir().join("vb_interp_lock.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Binary As #1
Lock #1
Unlock #1
Debug.Print EOF(1)
Close #1
"#
        );
        assert_eq!(output(&source), ["True"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn class_supports_fields_properties_methods_and_initialize() {
        let source = r#"
Class Counter
    Private mTotal As Long
    Private mStep As Long

    Private Sub Class_Initialize()
        mStep = 1
    End Sub

    Public Property Get Total() As Long
        Total = mTotal
    End Property

    Public Property Let Total(ByVal value As Long)
        mTotal = value
    End Property

    Public Property Get Stride() As Long
        Stride = mStep
    End Property

    Public Property Let Stride(ByVal value As Long)
        mStep = value
    End Property

    Public Sub Bump()
        mTotal = mTotal + mStep
    End Sub

    Public Function Twice(ByVal extra As Long) As Long
        Twice = mTotal * 2 + extra
    End Function

    Public Function Describe() As String
        Describe = "total=" & mTotal & " step=" & mStep
    End Function
End Class

Dim c As New Counter
Debug.Print c.Total
c.Bump
c.Bump
Debug.Print c.Total
c.Total = 10
Debug.Print c.Total
c.Stride = 5
c.Bump
Debug.Print c.Total
Debug.Print c.Twice(1)
Debug.Print c.Describe()
Debug.Print TypeName(c)
Debug.Print IsObject(c)
"#;
        assert_eq!(
            output(source),
            [
                "0",
                "2",
                "10",
                "15",
                "31",
                "total=15 step=5",
                "Counter",
                "True",
            ]
        );
    }

    #[test]
    fn class_instances_are_references_and_pass_by_ref() {
        let source = r#"
Class Box
    Public Value As Long

    Public Sub Bump(ByVal amount As Long)
        Value = Value + amount
    End Sub
End Class

Sub Apply(ByRef target As Box)
    target.Bump 10
End Sub

Dim a As New Box
Dim b As Box
Set b = a
b.Value = 5
a.Bump 1
Debug.Print a.Value
Debug.Print b.Value
Apply a
Debug.Print a.Value
Set b = New Box
Debug.Print a.Value
Debug.Print b.Value
"#;
        assert_eq!(output(source), ["6", "6", "16", "16", "0"]);
    }

    #[test]
    fn class_errors_are_reported() {
        let error = run_lines("Dim p As New Missing\n").unwrap_err();
        assert!(error.message().contains("未定义的类"), "{error}");

        let source = r#"
Class Thing
    Public Value As Long
    Public Property Get Name() As String
        Name = "thing"
    End Property
End Class
Dim t As New Thing
Debug.Print t.Nope
"#;
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("没有成员"), "{error}");

        let source = r#"
Class Thing
    Public Value As Long
End Class
Dim t As New Thing
On Error Resume Next
Debug.Print t.Nope
Debug.Print Err.Number
"#;
        assert_eq!(output(source), ["438"]);

        let source = r#"
Class Thing
    Public Property Get Name() As String
        Name = "thing"
    End Property
End Class
Dim t As New Thing
t.Name = "other"
"#;
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("只读"), "{error}");

        let error = run_lines("Dim t As New Thing\nt.Value\n").unwrap_err();
        assert!(error.message().contains("未定义的类"), "{error}");
    }

    #[test]
    fn class_methods_can_use_me_and_sibling_members() {
        let source = r#"
Class Chain
    Public Value As Long

    Public Sub Advance()
        Me.Value = Me.Value + 1
    End Sub

    Public Sub Grow(ByVal times As Long)
        Dim i As Long
        For i = 1 To times
            Advance
        Next i
    End Sub
End Class

Dim c As New Chain
c.Grow 3
Debug.Print c.Value
"#;
        assert_eq!(output(source), ["3"]);
    }

    #[test]
    fn class_indexed_properties_read_and_write() {
        let source = r#"
Class Bag
    Private mData(2) As Long

    Public Property Get Item(ByVal index As Long) As Long
        Item = mData(index)
    End Property

    Public Property Let Item(ByVal index As Long, ByVal value As Long)
        mData(index) = value
    End Property
End Class

Dim b As New Bag
b.Item(1) = 42
b.Item(2) = 7
Debug.Print b.Item(1)
Debug.Print b.Item(2)
Debug.Print b.Item(0)
"#;
        assert_eq!(output(source), ["42", "7", "0"]);
    }

    #[test]
    fn is_operator_compares_object_identity() {
        let source = r#"
Class Box
    Public Value As Long
End Class

Dim a As Box
Dim b As Box
Debug.Print a Is Nothing
Set a = New Box
Debug.Print a Is Nothing
Set b = a
Debug.Print a Is b
b.Value = 7
Debug.Print a.Value
Set b = New Box
Debug.Print a Is b
Dim c As New Collection
Dim d As Collection
Debug.Print c Is d
Set d = c
Debug.Print c Is d
On Error Resume Next
Debug.Print 1 Is 2
Debug.Print Err.Number
"#;
        assert_eq!(
            output(source),
            ["True", "False", "True", "7", "False", "False", "True", "424"]
        );
    }

    #[test]
    fn write_hash_pairs_with_input_hash() {
        let path = std::env::temp_dir().join("vb_interp_write.txt");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Output As #1
Write #1, 1, "a,b", True, 2.5
Write #1, "he said ""hi"""
Write #1, Empty, Null, False
Close #1

Open "{display}" For Input As #2
Dim n As Long
Dim s As String
Dim flag As Boolean
Dim f As Double
Input #2, n, s, flag, f
Debug.Print n
Debug.Print s
Debug.Print flag
Debug.Print f
Close #2
"#
        );
        assert_eq!(output(&source), ["1", "a,b", "True", "2.5"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "1,\"a,b\",#True#,2.5\r\n\"he said \"\"hi\"\"\"\r\n,#NULL#,#False#\r\n"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mid_statement_replaces_in_place() {
        let source = r#"
Dim s As String
s = "abcdef"
Mid(s, 2, 3) = "XY"
Debug.Print s
Mid(s, 4) = "12345"
Debug.Print s
Mid(s, 2) = "123456789"
Debug.Print s
Mid(s, 99) = "zzz"
Debug.Print s
"#;
        assert_eq!(output(source), ["aXYdef", "aXY123", "a12345", "a12345"]);
    }

    #[test]
    fn option_compare_text_makes_string_comparisons_case_insensitive() {
        let source = r#"
Option Compare Text
Debug.Print "abc" = "ABC"
Debug.Print "abc" < "ABD"
Select Case "abc"
Case "ABC"
    Debug.Print "matched"
Case Else
    Debug.Print "missed"
End Select
Debug.Print StrComp("a", "A")
Debug.Print StrComp("a", "A", 0)
"#;
        assert_eq!(output(source), ["True", "True", "matched", "0", "1"]);

        let source = r#"
Debug.Print "abc" = "ABC"
Debug.Print StrComp("a", "A")
"#;
        assert_eq!(output(source), ["False", "1"]);
    }

    #[test]
    fn class_member_that_needs_arguments_says_so() {
        let source = r#"
Class Bag
    Private mData(2) As Long

    Public Property Get Item(ByVal index As Long) As Long
        Item = mData(index)
    End Property
End Class

Dim b As New Bag
On Error Resume Next
Debug.Print b.Item
Debug.Print Err.Number
Debug.Print Err.Description
"#;
        let lines = output(source);
        assert_eq!(lines[0], "438");
        assert!(lines[1].contains("需要"), "{lines:?}");
    }

    #[test]
    fn file_system_statements_and_builtins_roundtrip() {
        let folder = std::env::temp_dir().join("vb_interp_fs_round");
        let file = folder.join("data.txt");
        let copy = folder.join("copy.txt");
        let renamed = folder.join("moved.txt");
        let _ = std::fs::remove_dir_all(&folder);
        let source = format!(
            r#"
MkDir "{folder}"
Open "{file}" For Output As #1
Print #1, "hello"
Close #1
Debug.Print FileLen("{file}")
FileCopy "{file}", "{copy}"
Debug.Print FileLen("{copy}")
Name "{copy}" As "{renamed}"
Debug.Print FileLen("{renamed}")
Debug.Print Len(Dir("{folder}\*.txt")) > 0
Kill "{renamed}"
Debug.Print Dir("{folder}\*.txt") = "data.txt"
Kill "{folder}\*.txt"
Debug.Print Dir("{folder}\*.txt") = ""
RmDir "{folder}"
Debug.Print Len(CurDir()) > 0
"#,
            folder = folder.display(),
            file = file.display(),
            copy = copy.display(),
            renamed = renamed.display(),
        );
        assert_eq!(
            output(&source),
            ["6", "6", "6", "True", "True", "True", "True"]
        );
        assert!(!folder.exists());
    }

    #[test]
    fn dir_enumerates_matching_files() {
        let folder = std::env::temp_dir().join("vb_interp_dir_list");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        for name in ["a.txt", "b.txt", "c.dat"] {
            std::fs::write(folder.join(name), b"x").unwrap();
        }
        let source = format!(
            r#"
Dim name As String
Dim found As String
name = Dir("{folder}\*.txt")
Do While name <> ""
    found = found & name & ";"
    name = Dir()
Loop
Debug.Print found
Debug.Print Dir("{folder}\*.log") = ""
"#,
            folder = folder.display()
        );
        assert_eq!(output(&source), ["a.txt;b.txt;", "True"]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn date_string_and_type_helpers() {
        let source = r#"
Debug.Print Weekday(DateSerial(2024, 1, 31))
Debug.Print WeekdayName(4)
Debug.Print MonthName(2)
Debug.Print DateValue("2024-01-31") = DateSerial(2024, 1, 31)
Debug.Print DateValue("1/31/2024") = DateSerial(2024, 1, 31)
Debug.Print TimeValue("12:00:00")
Debug.Print IsEmpty(Empty)
Debug.Print IsNull(Null)
Debug.Print IsDate("2024-01-31")
Debug.Print IsDate("not a date")
Debug.Print StrReverse("abc")
Debug.Print StrComp("a", "A")
Debug.Print StrComp("a", "A", 1)
Debug.Print AscW("A")
Debug.Print ChrW(66)
Debug.Print Join(Filter(Split("apple,banana,avocado", ","), "an"), "|")
"#;
        assert_eq!(
            output(source),
            [
                "4", "Wednesday", "February", "True", "True", "0.5", "True", "True", "True",
                "False", "cba", "1", "0", "65", "B", "banana",
            ]
        );
    }

    #[test]
    fn file_errors_carry_vb_numbers() {
        let source = r#"
On Error Resume Next
Dim size As Long
size = FileLen("nope_xyz.txt")
Debug.Print Err.Number
Err.Clear
Kill "nope_xyz.txt"
Debug.Print Err.Number
Err.Clear
MkDir "Z:\nope\nope"
Debug.Print Err.Number
"#;
        let lines = output(source);
        assert_eq!(lines[0], "53");
        assert_eq!(lines[1], "53");
        assert!(lines[2].parse::<i64>().unwrap() >= 70, "{lines:?}");
    }

    #[test]
    fn randomize_and_doevents_are_accepted() {
        let source = r#"
Randomize 42
Debug.Print Rnd() >= 0
Randomize
Debug.Print Rnd() >= 0
DoEvents
Debug.Print "done"
"#;
        assert_eq!(output(source), ["True", "True", "done"]);
    }

    #[test]
    fn lock_requires_an_open_file() {
        let error = run_lines("Lock #3\n").unwrap_err();
        assert!(error.message().contains("没有打开"), "{error}");
        assert_eq!(error.line(), 1);
    }

    #[test]
    fn lock_and_unlock_cover_only_the_given_range() {
        let path = std::env::temp_dir().join("vb_interp_lock_range.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Binary As #1
Open "{display}" For Binary As #2
Lock #1, 1 To 10
On Error Resume Next
Lock #2, 5 To 15
Debug.Print Err.Number
Err.Clear
Lock #2, 11 To 20
Debug.Print Err.Number
Unlock #2, 11 To 20
Err.Clear
Unlock #1, 1 To 10
Lock #2, 5 To 15
Debug.Print Err.Number
Unlock #2, 5 To 15
Close
"#
        );
        assert_eq!(output(&source), ["70", "0", "0"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lock_range_uses_record_numbers_for_random_files() {
        let path = std::env::temp_dir().join("vb_interp_lock_records.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Random As #1 Len = 8
Open "{display}" For Binary As #2
Lock #1, 2 To 3
On Error Resume Next
Lock #2, 1 To 8
Debug.Print Err.Number
Err.Clear
Lock #2, 9 To 16
Debug.Print Err.Number
Err.Clear
Lock #2, 25 To 32
Debug.Print Err.Number
Close
"#
        );
        assert_eq!(output(&source), ["0", "70", "0"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lock_range_rejects_bad_positions() {
        let path = std::env::temp_dir().join("vb_interp_lock_bad.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Open "{display}" For Binary As #1
On Error Resume Next
Lock #1, 5 To 2
Debug.Print Err.Number
Err.Clear
Lock #1, 0
Debug.Print Err.Number
Close
"#
        );
        assert_eq!(output(&source), ["63", "63"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn collection_supports_add_count_item_and_remove() {
        let source = r#"
Dim c As New Collection
c.Add "a"
c.Add 42, "answer"
Debug.Print c.Count
Debug.Print c.Item(1)
Debug.Print c.Item(2)
Debug.Print c.Item("ANSWER")
c.Remove 1
Debug.Print c.Count
Debug.Print c.Item(1)
Debug.Print TypeName(c)
"#;
        assert_eq!(
            output(source),
            ["2", "a", "42", "42", "1", "42", "Collection"]
        );
    }

    #[test]
    fn set_and_for_each_over_collection() {
        let source = r#"
Dim c As Collection
Set c = New Collection

Dim i As Long
For i = 1 To 3
    c.Add i * 10
Next i

Dim total As Long
Dim v As Variant
For Each v In c
    total = total + v
Next v
Debug.Print total

Set c = Nothing
Debug.Print Len(c)
"#;
        assert_eq!(output(source), ["60", "0"]);
    }

    #[test]
    fn collection_errors_are_reported() {
        let error =
            run_lines("Dim c As New Collection\nc.Add 1, \"k\"\nc.Add 2, \"K\"\n").unwrap_err();
        assert!(error.message().contains("已经存在"), "{error}");

        let error = run_lines("Dim c As New Collection\nDebug.Print c.Item(1)\n").unwrap_err();
        assert!(error.message().contains("超出范围"), "{error}");

        let error = run_lines("Dim x As Long\nx.Add 1\n").unwrap_err();
        assert!(error.message().contains("不是 `Collection`"), "{error}");
    }

    #[test]
    fn dictionary_supports_add_item_exists_and_remove() {
        let source = r#"
Dim d As New Dictionary
d.Add "one", 1
d.Add "two", 2
d("three") = 3
d("one") = 11

Debug.Print d.Count
Debug.Print d("one")
Debug.Print d.Item("two")
Debug.Print d.Exists("THREE")
Debug.Print d.Exists("four")
Debug.Print TypeName(d)

d.Remove "two"
Debug.Print d.Count
Debug.Print d.Exists("two")
"#;
        assert_eq!(
            output(source),
            ["3", "11", "2", "True", "False", "Dictionary", "2", "False"]
        );
    }

    #[test]
    fn dictionary_keys_items_and_for_each() {
        let source = r#"
Dim d As New Dictionary
d.Add "a", 10
d.Add "b", 20

Dim total As Long
Dim v As Variant
For Each v In d
    total = total + v
Next v
Debug.Print total

Dim k As Variant
Dim names As String
For Each k In d.Keys
    names = names & k
Next k
Debug.Print names

Debug.Print Join(d.Items, ",")
"#;
        assert_eq!(output(source), ["30", "ab", "10,20"]);
    }

    #[test]
    fn set_dictionary_works() {
        let source = r#"
Dim d As Dictionary
Set d = New Dictionary
d("k") = "v"
Debug.Print d("k")
Debug.Print d.Count
"#;
        assert_eq!(output(source), ["v", "1"]);
    }

    #[test]
    fn dictionary_errors_are_reported() {
        let error = run_lines("Dim d As New Dictionary\nd.Add \"k\", 1\nd.Add \"K\", 2\n")
            .unwrap_err();
        assert!(error.message().contains("已经有键"), "{error}");

        let error = run_lines("Dim d As New Dictionary\nDebug.Print d(\"missing\")\n")
            .unwrap_err();
        assert!(error.message().contains("没有键"), "{error}");
    }

    #[test]
    fn random_file_records_use_record_numbers() {
        let path = std::env::temp_dir().join("vb_interp_random.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Type Rec
    Id As Long
    Score As Integer
End Type
Dim r As Rec
Open "{display}" For Random As #1 Len = 8
r.Id = 1
r.Score = 10
Put #1, , r
r.Id = 2
r.Score = 20
Put #1, , r
Debug.Print LOF(1)
Debug.Print Seek(1)
Debug.Print Loc(1)
Seek #1, 1
Debug.Print Seek(1)
Get #1, , r
Debug.Print r.Id
Debug.Print r.Score
Debug.Print Loc(1)
Get #1, 2, r
Debug.Print r.Id
Debug.Print r.Score
Close #1
"#
        );
        assert_eq!(
            output(&source),
            ["16", "3", "2", "1", "1", "10", "1", "2", "20"]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn random_file_defaults_to_128_byte_records() {
        let path = std::env::temp_dir().join("vb_interp_random_default.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Dim v As Long
Dim back As Long
v = 4242
Open "{display}" For Random As #1
Put #1, , v
Put #1, , v
Debug.Print LOF(1)
Debug.Print Seek(1)
Debug.Print Loc(1)
Seek #1, 2
Get #1, , back
Debug.Print back
Close #1
"#
        );
        assert_eq!(output(&source), ["256", "3", "2", "4242"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn random_file_rejects_oversized_record() {
        let path = std::env::temp_dir().join("vb_interp_random_small.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Dim v As Long
On Error Resume Next
Open "{display}" For Random As #1 Len = 2
v = 7
Put #1, , v
Debug.Print Err.Number
Debug.Print Err.Description
"#
        );
        let lines = output(&source);
        assert_eq!(lines[0], "59");
        assert!(lines[1].contains("记录长度"), "{:?}", lines[1]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn len_clause_requires_random_mode() {
        let path = std::env::temp_dir().join("vb_interp_random_len.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!("Open \"{display}\" For Binary As #1 Len = 8\n");
        let error = run_lines(&source).unwrap_err();
        assert!(error.message().contains("Len"), "{error}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn random_file_sequential_scalar_records() {
        let path = std::env::temp_dir().join("vb_interp_random_seq.dat");
        let _ = std::fs::remove_file(&path);
        let display = path.display().to_string();
        let source = format!(
            r#"
Dim v As Long
Dim back As Long
Open "{display}" For Random As #1 Len = 4
v = 11
Put #1, , v
v = 22
Put #1, , v
Debug.Print LOF(1)
Seek #1, 1
Get #1, , back
Debug.Print back
Get #1, , back
Debug.Print back
Debug.Print EOF(1)
Close #1
"#
        );
        assert_eq!(output(&source), ["8", "11", "22", "True"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn address_of_runs_three_argument_callback() {
        let source = r#"
Declare PtrSafe Function InitOnceExecuteOnce Lib "kernel32" (ByRef InitOnce As LongLong, ByVal InitFn As LongPtr, ByVal Parameter As LongLong, ByRef Context As LongLong) As Long
Dim g_hits As Long
Dim g_param As LongLong
Function Once(ByRef once As LongLong, ByVal param As LongLong, ByRef ctx As LongLong) As Long
    g_hits = g_hits + 1
    g_param = param
    If ctx = 0 Then
        Once = 1
    Else
        Once = 0
    End If
End Function
Dim once As LongLong
Dim ctx As LongLong
Dim ok As Long
once = 0
ctx = 0
ok = InitOnceExecuteOnce(once, AddressOf Once, 1234, ctx)
Debug.Print ok <> 0
Debug.Print g_hits
Debug.Print g_param
Dim again As LongLong
Dim callback As LongPtr
again = 0
g_hits = 0
callback = AddressOf Once
ok = InitOnceExecuteOnce(again, callback, 99, ctx)
Debug.Print ok <> 0
Debug.Print g_hits
Debug.Print g_param
"#;
        assert_eq!(output(source), ["True", "1", "1234", "True", "1", "99"]);
    }

    #[test]
    fn address_of_passes_string_argument_to_callback() {
        let source = r#"
Declare PtrSafe Function EnumSystemLocalesW Lib "kernel32" (ByVal proc As LongPtr, ByVal flags As Long) As Long
Dim g_count As Long
Dim g_first As String
Function OnLocale(ByVal name As String) As Long
    g_count = g_count + 1
    If g_count = 1 Then g_first = name
    OnLocale = 1
End Function
Dim ok As Long
ok = EnumSystemLocalesW(AddressOf OnLocale, 0)
Debug.Print ok <> 0
Debug.Print g_count > 0
Debug.Print Len(g_first) > 0
"#;
        assert_eq!(output(source), ["True", "True", "True"]);
    }

    #[test]
    fn address_of_reports_callback_errors() {
        let source = r#"
Declare PtrSafe Function InitOnceExecuteOnce Lib "kernel32" (ByRef InitOnce As LongLong, ByVal InitFn As LongPtr, ByVal Parameter As LongLong, ByRef Context As LongLong) As Long
Function Boom(ByVal param As LongLong) As Long
    Boom = 1 / 0
End Function
Dim once As LongLong
Dim ctx As LongLong
once = 0
ctx = 0
InitOnceExecuteOnce once, AddressOf Boom, 0, ctx
"#;
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("除数"), "{error}");
        assert_eq!(error.line(), 4);
    }

    #[test]
    fn address_of_rejects_unsupported_targets() {
        let error = run_lines("Debug.Print AddressOf Missing\n").unwrap_err();
        assert!(error.message().contains("找不到"), "{error}");

        let source = r#"
Declare PtrSafe Function InitOnceExecuteOnce Lib "kernel32" (ByVal a As LongPtr, ByVal b As LongPtr) As Long
Function TooMany(a As Long, b As Long, c As Long, d As Long, e As Long) As Long
    TooMany = 0
End Function
InitOnceExecuteOnce AddressOf TooMany, 0
"#;
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("最多支持 4 个"), "{error}");

        let source = r#"
Declare PtrSafe Function MessageBoxA Lib "user32" (ByVal hwnd As LongPtr, ByVal text As String, ByVal title As String, ByVal flags As Long) As Long
Function Never(a As Long) As Long
    Never = 0
End Function
MessageBoxA 0, AddressOf Never, "t", 0
"#;
        let error = run_lines(source).unwrap_err();
        assert!(error.message().contains("只能传给"), "{error}");

        let error = run_lines(
            "Function Never(a As Long) As Long\nNever = 0\nEnd Function\nDebug.Print AddressOf Never + 1\n",
        )
        .unwrap_err();
        assert!(error.message().contains("只能传给"), "{error}");
    }
}
