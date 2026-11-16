//! VB 语法脚本解释器。
//!
//! 设计目标：以 VB6 语法子集写「能直接调用 Win32 API」的小脚本，并提供少量 Win64 API 类型扩展。
//!
//! - [`lexer`] / [`parser`]：词法与语法分析（大小写不敏感、`'` 注释、`_` 续行、`Dim/If/For/Do`、
//!   `Like`/`Eqv`/`Imp`、`GoSub`/`Return` 和整数枚举常量表达式）；
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
//! COM Automation 支持 `CreateObject`、活动对象/新对象/moniker `GetObject`、属性与
//! 位置/命名参数调用、链式成员、标量/字符串 `ByRef`、任意维数与强类型 `SAFEARRAY` 封送、
//! `For Each` 活枚举器与 `WithEvents` 事件；仍不支持 UDT 记录封送（VB6 自己的标准模块也做不到）。
//! Win32 调用支持 `ByRef` / `ByVal` 结构体（字段为整型/浮点/`String * n`/标量或 UDT 数组；
//! `ByVal` 按 x64 ABI：≤ 8 字节进寄存器，更大的传临时副本指针）、最多 64 个整数与
//! `Single`/`Double`/`Date` 混合的标量参数，返回值可为整数或 `Single`/`Double`/`Date`；
//! 结构体里的变长 `String` 仍不能用于 Win32 调用。
//! `AddressOf 过程名` 可以把自己的过程当成回调交给 API：参数经机器码跳板进入解释器，
//! 支持最多 64 个 `Byte`/`Integer`/`Long`/`Boolean`/`LongLong`/`LongPtr`/`Currency`/`Single`/`Double`/`Date`
//! 或 `String` 形参（`ByVal String` 是 `LPCWSTR`，`ByRef` 是 `LPWSTR *`），
//! 返回值可为整数、`Single`、`Double`/`Date` 或 NUL 结尾的 `String`（`Sub` 视为 0）；
//! 支持 ByRef UDT 形参，不支持数组形参；
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
//! `Enum ... End Enum` 的成员注册为模块级常量，显式值支持整数常量表达式，
//! `Dim c As Color` 按 `Long` 处理。
//! 对象支持 `Collection`、`Dictionary`（`Scripting.Dictionary` 子集）与自定义 `Class`：
//! `Dim c As New Collection` / `Set d = New Dictionary` / `Set x = Nothing`、
//! `c.Add 元素[, 键]`、`c.Remove 下标或键`、`c.Count`、`c.Item(下标或键)`；
//! 字典另有 `d.Add 键, 值`、`d(键) = 值`、`d(键)`、`d.Item(键)`、`d.Exists(键)`、
//! `d.Keys` / `d.Items`（0 基数组，可 `For Each`）、`d.Remove 键`；`For Each k In d` 直接给出键、
//! 按插入顺序（与 VB6 的 `Scripting.Dictionary` 一致）；键一律大小写不敏感。
//! `Class 名字 … End Class` 里可以写 `Private/Public` 变量、`Sub`/`Function`、`Property Get/Let/Set`，
//! `Class_Initialize` 就是构造函数、`Class_Terminate` 在最后一个引用消失时执行：
//! `Dim p As New 类名` / `Set p = New 类名` 建实例，`p.方法 参数`、`p.属性`、`p.属性 = 值`、
//! `Set p.属性 = 对象`、`p.字段 = 值` 都按成员的声明分派，方法里可以裸写成员名或 `Me.成员`；
//! 实例是引用语义（赋值、传参共享同一份），`TypeOf x Is 类名` 判断实例类型。
//! `Private WithEvents m As New 类名` 让类订阅另一个类的事件，`RaiseEvent 事件(实参)`
//! 会回调订阅者的 `源类名_事件名` 方法（订阅表用弱引用，对象被回收后自动失效）。
//! 类成员字段未标注时默认为 `Private`，方法和属性未标注时默认为 `Public`；显式访问修饰符生效，
//! 同类成员可以访问私有成员。类可用 `Implements 接口类` 声明接口，并以私有
//! `接口类_成员名` 方法/属性实现接口；语法检查会报出漏实现/多写的成员与签名不一致的实现
//! （过程形式、参数、返回类型都要与接口相同，参数名可不同；规则按本机 VB6 实测）。
//! 当前还没有独立的接口变量类型（不能声明 `Dim x As 接口名` 走接口分派）。
//! 简化之处：
//! 类内裸赋值 `Name = v` 不触发同名 `Property Let`（写 `Me.Name = v`）、
//! `Dim x As 类名` 需要类在同文件里先定义。
//! 错误处理支持 `On Error Resume Next`、`On Error GoTo 标签`、`On Error GoTo 0`、`On Error GoTo -1`、
//! `GoTo 标签`、`Err.Raise`/`Err.Clear` 以及 `Err.Number`/`Err.Description`/`Err.Source` 与 `Error(n)`；
//! 执行任何 `On Error` 语句都会清空 `Err`（本机 VB6 实测），`Resume`/`Resume Next` 的恢复点是「出错语句所在的根块语句」。
//! 与 VB 一致，整数与 `Single` 的赋值超出目标类型范围时报错误号 6（`溢出`），
//! 不会静默截断；`Variant`/`Double` 不做范围检查。
//! `CInt`/`CLng` 按银行家舍入并检查目标类型范围（越界错误号 6），`CBool` 与 `If` 条件里的字符串
//! 只接受 `True`/`False` 与数字文本（其余错误号 13）；`Str` 给非负数加前导空格，`Val` 认识
//! `&H`/`&O` 前缀；`Double` 按 VB6 的 15 位有效数字显示（`0.1 + 0.2` 打印 `0.3`）。
//! `For` 在起止值与步长都是整数时按整数精确步进；`Space`/`String` 与定长字段都有大小上限。
//! 过程递归会同时看 `max_depth` 与线程剩余栈空间，栈不够时报错误号 28 而不是让进程被终止；
//! 宿主应当用 [`SCRIPT_STACK_SIZE`] 开脚本线程（见 [`spawn_script_thread`]）。
//! `Date` 以独立值类型表示，内部存储 OLE Automation 日期序列，边界上按 Double/`VT_DATE` 封送。
//! 数组支持 `Dim a(10)`、`a(1 To n)`、多维、`ReDim [Preserve]`（只保留最后一维的数据）、`Erase` 与 `For Each`，
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
//! `AscW`/`ChrW`、`Randomize`、`DoEvents`（宿主是 GUI 程序，不派发本进程消息，但会把排队的
//! COM 事件当场执行完，与 VB6 的消息循环一致）。
//! `Is` 是对象同一性运算符（`If obj Is Nothing`、`a Is b`，非对象操作数报错误号 424）；
//! `Option Compare Text` 让字符串的 `=`/`<`/`>`、`Select Case`、`StrComp`/`Filter` 的默认比较
//! 都忽略大小写（默认 `Binary`，与 VB 一致）。

pub mod ast;
pub mod builtins;
pub mod com;
pub mod debug;
pub mod dialog;
pub mod error;
pub mod ffi;
pub mod filelock;
pub mod finance;
pub mod hasher;
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

/// 运行脚本的线程至少需要多大的栈。
///
/// 过程递归到 [`RunOptions::max_depth`] 时每层要占几十 KB（debug 构建下的
/// `Runtime::exec` 每层约 80 KB），标准库给新线程的默认栈只有 2 MiB，
/// 深递归的脚本会直接触发栈溢出、让整个进程退出（0xC00000FD），
/// 而不是报一个「调用层次过深」的错误。宿主在后台线程跑脚本时用这个值开线程。
pub const SCRIPT_STACK_SIZE: usize = 64 * 1024 * 1024;

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
    /// `#Include` 在脚本目录里找不到时再搜索的目录。
    ///
    /// `None` 时用可执行文件旁的 `assets/`：IDE 里是程序自带的公共定义，
    /// 而 DLL / 命令行宿主里 `current_exe()` 指向别的程序，应当显式传进来。
    pub include_dir: Option<PathBuf>,
    /// 是否允许 `MsgBox` / `InputBox` 弹窗。
    ///
    /// headless 宿主（命令行、外部 agent）设 `false`：`MsgBox` 直接返回 0（`vbOK`），
    /// `InputBox` 返回空串，脚本不会因为等不到人点按钮而卡死。
    pub allow_ui: bool,
    /// 是否累计执行频次统计（`qide-run --stats`）；默认关闭，避免给热路径加无谓工作。
    pub count_stats: bool,
    /// 是否逐条输出执行轨迹（`qide-run --trace`）：每执行一条语句就写一行 `» 第 N 行`。
    pub trace: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_steps: 20_000_000,
            max_depth: 200,
            command_line: String::new(),
            script_path: None,
            include_dir: None,
            allow_ui: true,
            count_stats: false,
            trace: false,
        }
    }
}

/// 执行轨迹行（`--trace`）的前缀：宿主靠它把轨迹与脚本输出区分开。
pub const TRACE_PREFIX: &str = "»";

/// 在后台线程跑脚本。
///
/// 用 [`SCRIPT_STACK_SIZE`] 而不是标准库默认的 2 MiB：过程递归到 `max_depth` 时
/// 每层要占几十 KB，栈不够会让进程被直接终止（0xC00000FD），界面上什么也看不到。
pub fn spawn_script_thread<F>(work: F) -> std::io::Result<std::thread::JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    std::thread::Builder::new()
        .name("vb-script".to_string())
        .stack_size(SCRIPT_STACK_SIZE)
        .spawn(work)
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
    out: F,
    interrupt: &AtomicBool,
    options: RunOptions,
) -> Result<(), Error>
where
    F: FnMut(&str),
{
    run_with_stats(source, out, interrupt, options).0
}

/// 同 [`run_with`]，另外返回执行频次统计（`qide-run --stats`）。
pub fn run_with_stats<F>(
    source: &str,
    mut out: F,
    interrupt: &AtomicBool,
    options: RunOptions,
) -> (Result<(), Error>, runtime::RunStats)
where
    F: FnMut(&str),
{
    let parsed = parse_with_include_dir(
        source,
        options.script_path.as_deref(),
        options.include_dir.as_deref(),
    );
    let (program, map) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => return (Err(error), runtime::RunStats::default()),
    };
    let (outcome, stats) = run_program_guarded_stats(|| {
        runtime::run_program_with_stats(&program, &mut out, interrupt, &options, None)
    });
    let outcome = outcome.map_err(|error| Error::Runtime(remap_runtime(error, &map)));
    (outcome, stats)
}

/// 同 [`run_program_guarded`]，但把执行统计一起带回来。
fn run_program_guarded_stats<F>(work: F) -> (Result<(), error::RuntimeError>, runtime::RunStats)
where
    F: FnOnce() -> (Result<(), error::RuntimeError>, runtime::RunStats),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(outcome) => outcome,
        Err(payload) => (
            Err(error::RuntimeError::new(
                format!(
                    "解释器内部错误（这是 Q-IDE 的缺陷，不是脚本问题）：{}",
                    panic_text(payload.as_ref())
                ),
                0,
            )),
            runtime::RunStats::default(),
        ),
    }
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
    let (program, map) = parse_with_include_dir(
        source,
        options.script_path.as_deref(),
        options.include_dir.as_deref(),
    )?;
    let outcome = run_program_guarded(|| {
        runtime::run_program(&program, &mut out, interrupt, &options, Some(debug.clone()))
    })
    .map_err(|error| Error::Runtime(remap_runtime(error, &map)));
    debug.finish();
    outcome?;
    Ok(())
}

/// 跑一遍程序；解释器内部 panic（借用冲突、下标越界等）转成普通运行时错误。
///
/// 少了这一层，panic 会让脚本线程静默结束：界面永远等不到「结束」事件、一直显示运行中，
/// 用户也看不到任何原因。捕获范围只到 `run_program`，`Runtime` 的清理照常执行。
fn run_program_guarded<F>(work: F) -> Result<(), error::RuntimeError>
where
    F: FnOnce() -> Result<(), error::RuntimeError>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(outcome) => outcome,
        Err(payload) => Err(error::RuntimeError::new(
            format!(
                "解释器内部错误（这是 Q-IDE 的缺陷，不是脚本问题）：{}",
                panic_text(payload.as_ref())
            ),
            0,
        )),
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "原因不明".to_string()
    }
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
    parse_with_include_dir(source, path, None)
}

/// 同 [`parse_with_source_map`]，但可以指定 `#Include` 的备用搜索目录（`assets/` 的替代）。
pub fn parse_with_include_dir(
    source: &str,
    path: Option<&Path>,
    include_dir: Option<&Path>,
) -> Result<(ast::Program, include::SourceMap), Error> {
    let expanded = include::expand(source, path, include_dir)
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
    error.message = std::sync::Arc::from(message.as_str());
    error
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
