//! 树遍历解释执行。

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::rc::Weak;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::time::Instant;

use crate::interpreter::RunOptions;
use crate::interpreter::ast::*;
use crate::interpreter::builtins;
use crate::interpreter::com::{self, ApartmentGuard};
use crate::interpreter::debug::{
    self, DebugControl, DebugQuery, DebugReply, DebugSignal, DebugSnapshot, DebugValue,
    DebugVariable,
};
use crate::interpreter::error::RuntimeError;
use crate::interpreter::ffi;
use crate::interpreter::filelock;
use crate::interpreter::hasher::FastMap;
use crate::interpreter::trampoline::Trampoline;
use crate::interpreter::value::{
    ClassRef, RecordField, Value, VbArray, VbDictionary, VbInstance, VbRecord, VbType,
    default_value, fits_directly, pad_fixed,
};

mod calls;
mod classes;
mod control;
mod expr;
mod files;
mod records;
mod statements;
mod variables;

mod callbacks;
mod helpers;

use callbacks::*;
use helpers::*;

// 测试会用到：glob 引入的名字对外不可见，这里显式再导出一次（`qide-run` 那个测试目标用不到它）。
#[allow(unused_imports)]
pub(crate) use callbacks::runtime_identity_matches;

/// 单个数组允许的最大元素数，避免脚本一次分配过多内存。
const MAX_ARRAY_ELEMENTS: usize = 1_000_000;

/// 递归调用前要求当前线程还剩多少栈空间。
///
/// debug 构建里每层用户调用要占几十 KB（`exec` 的栈帧很大），而调用方可能跑在
/// 标准库默认 2 MiB 的线程栈上。栈到底时进程会被直接终止（0xC00000FD），
/// 界面上看不到任何提示，所以提前留出 1 MiB 余量报错误号 28。
const MIN_CALL_STACK: usize = 1024 * 1024;

/// 当前线程还剩多少字节的栈（Windows 下用 `GetCurrentThreadStackLimits` 取栈底）。
#[cfg(windows)]
fn stack_headroom() -> usize {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
    }
    let mut low = 0usize;
    let mut high = 0usize;
    // SAFETY: 两个指针都指向本函数的局部变量，调用期间一直有效。
    unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
    // 取一个栈上地址当作当前栈指针；栈从高地址往低地址生长。
    let here = &low as *const usize as usize;
    here.saturating_sub(low)
}

/// 非 Windows 平台拿不到栈界，交给 `max_depth` 兜底。
#[cfg(not(windows))]
fn stack_headroom() -> usize {
    usize::MAX
}
static NEXT_RUNTIME_GENERATION: AtomicU64 = AtomicU64::new(1);

/// `Print` 的打印区宽度：`,` 会把输出推进到下一个 14 列区的开头（VB 的 `Spc`/`Tab` 同款排版）。
const PRINT_ZONE: usize = 14;

pub(crate) fn is_moniker_path(value: &str, cwd: &Path) -> bool {
    value.starts_with("file:")
        || value.contains('\\')
        || value.contains('/')
        || value.contains(':')
        || value.contains('!')
        || cwd.join(value).is_file()
}

/// 调试面板里每个变量最多显示多少个字符。
const DEBUG_VALUE_CHARS: usize = 120;

/// 变量的存储单元。`ByRef` 传参时共享同一个单元。
#[derive(Clone)]
pub struct Slot {
    cell: Rc<RefCell<Value>>,
    ty: VbType,
    /// `Const` 声明的单元不允许再赋值。
    readonly: bool,
    /// 声明时就是数组（`Dim a(2)`、`Dim a()`），不能整体赋成标量。
    array_binding: bool,
    /// 省略掉的可选参数（`IsMissing` 用它判定）；一旦写入就清掉。
    missing: std::cell::Cell<bool>,
    /// 由哪一行 `Dim` 声明出来的；0 表示参数、全局或 `Const` 这类非 `Dim` 来源。
    ///
    /// 循环体里的 `Dim` 每轮都会执行到，靠它区分「同一条声明又跑了一遍」和
    /// 「同一过程里重复声明了两个同名变量」。
    declared_at: usize,
    /// `Variant` 槽里当前值的 VB6 类型（见 [`Slot::narrow_type`]）。
    ///
    /// 用 `Rc<Cell<_>>` 而不是 `Cell`：`Slot` 是克隆出去用的（`find_slot` 返回克隆），
    /// 存储共享、标注也必须共享，否则写在克隆上的类型就丢了。
    /// 注意：**不能**改成「按需分配」（`RefCell<Option<Rc<..>>>`）——克隆后两边各自的 `None`
    /// 会让「先克隆、再在克隆上记类型」的路径丢标注，`VarType`/`Get`/`Put` 的窄类型用例会挂。
    narrow: Rc<std::cell::Cell<Option<VbType>>>,
}

impl Slot {
    pub fn new(value: Value, ty: VbType) -> Self {
        Self::with_readonly(value, ty, false)
    }

    /// `Const` 用的只读单元。
    pub fn constant(value: Value, ty: VbType) -> Self {
        Self::with_readonly(value, ty, true)
    }

    /// 共享一个已有的值单元（数组元素持 `ByRef` 传参时使用）。
    pub fn from_cell(cell: Rc<RefCell<Value>>, ty: VbType) -> Self {
        Self::from_cell_with_narrow(cell, ty, Rc::new(std::cell::Cell::new(None)))
    }

    /// 共享值单元与内容类型标注（UDT 的 `Variant` 字段当 `ByRef` 实参时用）。
    pub fn from_cell_with_narrow(
        cell: Rc<RefCell<Value>>,
        ty: VbType,
        narrow: Rc<std::cell::Cell<Option<VbType>>>,
    ) -> Self {
        Self {
            cell,
            ty,
            readonly: false,
            array_binding: false,
            missing: std::cell::Cell::new(false),
            declared_at: 0,
            narrow,
        }
    }

    fn with_readonly(value: Value, ty: VbType, readonly: bool) -> Self {
        let array_binding = value.is_array();
        Self {
            cell: Rc::new(RefCell::new(value)),
            ty,
            readonly,
            array_binding,
            missing: std::cell::Cell::new(false),
            declared_at: 0,
            narrow: Rc::new(std::cell::Cell::new(None)),
        }
    }

    /// 标记为「调用时省略的可选参数」，供 `IsMissing` 判定。
    fn omitted_optional(self) -> Self {
        self.missing.set(true);
        self
    }

    pub fn is_missing(&self) -> bool {
        self.missing.get()
    }

    /// 记下这条声明来自哪一行。
    fn at_line(mut self, line: usize) -> Self {
        self.declared_at = line;
        self
    }

    pub fn is_readonly(&self) -> bool {
        self.readonly
    }

    pub fn is_array_binding(&self) -> bool {
        self.array_binding
    }

    pub fn value(&self) -> Value {
        self.cell.borrow().clone()
    }

    pub fn ty(&self) -> VbType {
        self.ty
    }

    /// `Variant` 槽里当前值的 VB6 类型（`v = i`（Integer）之后是 `Integer`）。
    ///
    /// 值本身只带动态类型，所以赋值时把右侧表达式的静态类型记在这里；`VarType` / `TypeName`
    /// 用它把 `Integer`/`Long`、`Single`/`Double` 区分开。每次写入都会先清空（`set`），
    /// 不会留下过期信息。
    pub fn narrow_type(&self) -> Option<VbType> {
        self.narrow.get()
    }

    fn set_narrow(&self, ty: Option<VbType>) {
        self.narrow.set(ty);
    }

    pub fn set(&self, value: Value) -> Result<(), String> {
        if self.readonly {
            return Err("不能给常量赋值".into());
        }
        self.missing.set(false);
        self.narrow.set(None);
        if fits_directly(self.ty, &value) {
            *self.cell.borrow_mut() = value;
            return Ok(());
        }
        let coerced = value.coerce(self.ty)?;
        *self.cell.borrow_mut() = coerced;
        Ok(())
    }
}

#[derive(Default)]
struct Frame<'p> {
    /// 局部变量表。键用 `Cow<'p, str>`：形参名借自 AST（`Borrowed`，每次调用不分配）；
    /// `Dim` / `Const` / 隐式声明的名字来自语句本身、活不过 `'p`，只能 `Owned`；
    /// 类方法调用时从实例字段搬过来的名字同样是 `Owned`。
    locals: FastMap<std::borrow::Cow<'p, str>, Slot>,
    /// 函数返回值所在的槽（函数名即返回值）。
    result: Option<Slot>,
    /// 当前过程名，`Static` 局部变量按它归档。
    ///
    /// 借用 AST 里的名字（`&'p str`）：每次调用都会 push 一个 `Frame`，用 `String` 就是每次调用
    /// 一次分配 + 拷贝；过程名在整次运行里不会变，借出去就够了。
    procedure: Option<&'p str>,
    /// 调用这一帧的那条语句所在行（1 基）：用来拼调用栈。
    call_line: usize,
    /// 当前 `On Error` 处理方式。
    on_error: Option<ErrorMode>,
    /// 出错语句在根块里的下标（供 `Resume` 使用）。
    resume_index: Option<usize>,
    /// 已进入的语句块层数，0 表示过程/模块的根块。
    depth: usize,
}

/// `eval_indices` 的栈上缓冲上限；超过这么多维才退回堆分配（VB 最多允许 60 维）。
const INLINE_DIMS: usize = 8;

/// 数组下标求值结果：常见 1–2 维不碰堆。
struct IndexBuf {
    inline: [i32; INLINE_DIMS],
    len: usize,
    /// 只有维数超过 `INLINE_DIMS` 时才非空。
    heap: Vec<i32>,
}

impl std::ops::Deref for IndexBuf {
    type Target = [i32];

    fn deref(&self) -> &[i32] {
        if self.heap.is_empty() {
            &self.inline[..self.len]
        } else {
            &self.heap
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Flow {
    Normal,
    ExitFor,
    ExitDo,
    ExitProc,
    Halt,
    /// `GoTo 标签`：逐层向外查找标签。
    Goto(String),
    Gosub(String),
    GosubReturn,
    /// `Resume Next`：回到根块的第 `index + 1` 条语句。
    ResumeNext(usize),
    /// `Resume`：重新执行根块的第 `index` 条语句。
    ResumeAt(usize),
}

/// 脚本里由 `Open` 打开的文件。
enum FileHandle {
    Reader(BufReader<File>),
    Writer(FileWriter),
    /// `For Binary`：按字节随机读写。
    Binary(BinaryFile),
}

/// 文本输出文件（`For Output` / `For Append`）。
struct FileWriter {
    sink: BufWriter<File>,
    /// `Open` 时的模式码（`Output` 2 / `Append` 8），`FileAttr(n, 1)` 要原样报出去。
    mode: i64,
    /// `Width #n, w`：`Print #` 的自动换行宽度；0 表示不换行。
    width: usize,
    /// 当前行已经写了多少字符（`;` 续行和 `,` 分区都靠它定位）。
    column: usize,
}

impl FileWriter {
    fn new(file: File, mode: i64) -> Self {
        Self {
            sink: BufWriter::new(file),
            mode,
            width: 0,
            column: 0,
        }
    }

    fn file(&self) -> &File {
        self.sink.get_ref()
    }

    /// 写入一段文本：按 `Width #` 自动换行、`!suppress_newline` 时补换行，并跟踪列位置。
    fn write_line(&mut self, text: &str, suppress_newline: bool) -> std::io::Result<()> {
        // 行尾是 **CRLF**：本机 VB6 实测 `Print #1, "hello from script"` 之后 `FileLen` 是 19
        // （17 个字符 + `\r\n`）。用 `\n` 的话读回去的字节数、以及外部工具看到的换行全都不对。
        let mut line = String::new();
        for ch in text.chars() {
            if self.width > 0 && self.column >= self.width {
                line.push_str("\r\n");
                self.column = 0;
            }
            line.push(ch);
            self.column += 1;
        }
        if !suppress_newline {
            line.push_str("\r\n");
            self.column = 0;
        }
        self.sink.write_all(line.as_bytes())
    }

    /// 原样写入（`Write #` 用：不分列、不管宽度，文本自带换行）。
    fn write_raw(&mut self, text: &str) -> std::io::Result<()> {
        self.column = 0;
        self.sink.write_all(text.as_bytes())
    }
}

/// `For Binary` / `For Random` 文件的读写游标（`pos` 是 0 基字节偏移）。
struct BinaryFile {
    file: File,
    pos: u64,
    /// `For Random`：按定长记录读写，位置以记录号计。
    random: bool,
    /// `Len = n` 指定的记录长度（`For Binary` 时无意义）。
    record_len: usize,
}

/// VB 默认的随机文件记录长度。
const DEFAULT_RECORD_LEN: usize = 128;

impl BinaryFile {
    fn new(file: File, record_len: Option<usize>) -> Self {
        match record_len {
            Some(record_len) => Self {
                file,
                pos: 0,
                random: true,
                record_len,
            },
            None => Self {
                file,
                pos: 0,
                random: false,
                record_len: DEFAULT_RECORD_LEN,
            },
        }
    }

    /// 下一条记录号（随机文件）或逻辑字节位置（1 基）。
    fn position(&self) -> i64 {
        if self.random {
            self.record_number() + 1
        } else {
            self.pos as i64 + 1
        }
    }

    /// `Loc(n)`：最近一次读写过的记录号；随机文件以外是「最后一个读写过的字节的 1 基位置」。
    /// （`pos` 保存的是下一次读写的位置（0 基），所以刚读过的那一字节正好是 `pos`。）
    fn location(&self) -> i64 {
        if self.random {
            self.record_number()
        } else {
            self.pos as i64
        }
    }

    /// 游标所在的记录号（`pos` 对齐到记录边界时就是刚读写的那条）。
    fn record_number(&self) -> i64 {
        (self.pos / self.record_len as u64) as i64
    }

    fn move_to(&mut self, position: i64, line: usize) -> Result<(), RuntimeError> {
        if position < 1 {
            return Err(
                RuntimeError::new(format!("文件位置 {position} 无效（从 1 开始）"), line)
                    .with_number(63),
            );
        }
        let offset = (position - 1) as u64;
        self.pos = if self.random {
            offset.saturating_mul(self.record_len as u64)
        } else {
            offset
        };
        Ok(())
    }

    /// 一次读写要推进多少字节：随机文件按固定记录长度对齐，超出则报错误号 59。
    fn advance(&self, size: usize, line: usize) -> Result<usize, RuntimeError> {
        if self.random {
            if size > self.record_len {
                return Err(RuntimeError::new(
                    format!("记录长度 {size} 字节超过 `Len = {}`", self.record_len),
                    line,
                )
                .with_number(59));
            }
            Ok(self.record_len)
        } else {
            Ok(size)
        }
    }
}

/// 一个已挂接的 COM 事件源：模块级变量（`instance` 为 `None`）或类字段上的 `WithEvents`。
struct ComEventOwner {
    instance: Option<Weak<RefCell<VbInstance>>>,
    field: String,
    events: com::ComEvents,
}

/// COM 调用的一个实参：命名实参的名字、值、是否按 `ByRef` 传、写回用的变量槽。
type ComArgument = (Option<String>, Value, bool, Option<Slot>);

pub struct Runtime<'p, 'o> {
    generation: u64,
    procedures: HashMap<String, &'p Procedure>,
    /// `Property Let` 过程：只能通过赋值语句调用。
    lets: HashMap<String, &'p Procedure>,
    declares: HashMap<String, &'p Declare>,
    /// `Type ... End Type` 定义。
    types: HashMap<String, &'p TypeDef>,
    /// `Class ... End Class` 定义。
    classes: HashMap<String, &'p ClassDef>,
    /// 模块级变量，所有过程都能看到。
    globals: FastMap<std::borrow::Cow<'p, str>, Slot>,
    /// 过程调用栈，只保存当前过程的局部变量。
    frames: Vec<Frame<'p>>,
    /// `Static` 局部变量：过程名 → 变量（过程名借用 AST，不复制）。
    statics: FastMap<&'p str, FastMap<String, Slot>>,
    /// `Option Explicit`：未声明变量直接报错。
    option_explicit: bool,
    /// `Option Compare Text`：字符串比较忽略大小写。
    option_compare_text: bool,
    /// `DefInt A-Z` 之类的隐式类型范围（字母已转小写）。
    def_types: Vec<(char, char, VbType)>,
    /// 是否允许 `MsgBox` / `InputBox` 弹窗（headless 宿主为 `false`）。
    allow_ui: bool,
    /// `With` 嵌套栈。
    with_stack: Vec<Value>,
    /// 模块级代码的 `On Error` / `Resume` 状态（没有调用帧时使用）。
    module_error: Option<ErrorMode>,
    module_resume: Option<usize>,
    module_depth: usize,
    /// 最近一次运行时错误，供 `Err.Number` / `Err.Description` 读取。
    last_error: Option<RuntimeError>,
    /// 已打开的脚本文件：`#n` → 句柄。
    files: HashMap<i32, FileHandle>,
    out: &'o mut dyn FnMut(&str),
    interrupt: &'o AtomicBool,
    steps: u64,
    max_steps: u64,
    max_depth: usize,
    rng: u64,
    started: Instant,
    /// `Dir()` 的遍历状态：第一次带路径调用时建立，之后无参调用依次往后取。
    dir: Option<builtins::DirState>,
    /// `StrPtr` 返回的 UTF-16 缓冲区，保持到脚本结束，避免指针悬空。
    strings: Vec<Vec<u16>>,
    /// 存进结构体/变量后仍要被系统回调的 `AddressOf` 跳板，保持到脚本结束。
    persistent_callbacks: Vec<PersistentCallback>,
    /// 事件订阅表：源实例 → 订阅它的 `WithEvents` 对象（弱引用，随对象一起消失）。
    events: HashMap<usize, Vec<Weak<RefCell<VbInstance>>>>,
    /// `WithEvents` 声明的变量名（标识符已小写）：`Set` 到 COM 对象时挂接事件。
    com_event_vars: std::collections::HashSet<String>,
    /// 已挂接的 COM 事件连接；`Drop` 时自动 `Unadvise`。
    com_events: Vec<ComEventOwner>,
    /// 正在分派 COM 事件（`事件处理过程里再产生的事件`要留到下一次，避免递归分派）。
    pumping_events: bool,
    /// 调试控制器；普通运行时为 `None`。
    debug: Option<Arc<DebugControl>>,
    /// 是否累计执行频次统计（`--stats`）。
    count_stats: bool,
    /// 是否逐条输出执行轨迹（`--trace`）。
    trace: bool,
    stats: RunStats,
    /// 正在求值监视表达式：期间的语句不再次触发暂停检查。
    evaluating: bool,
    /// `Command()` 返回的命令行参数。
    command_line: String,
    /// 脚本所在目录：`App.Path` 返回它。
    script_dir: PathBuf,
    /// 脚本视角的当前目录：相对路径按它解析，`ChDir` 只改它不动进程 CWD。
    cwd: PathBuf,
    /// COM apartment initialization failure is deferred until a script requests COM.
    com_error: Option<String>,
    /// 「刚刚求值出来的那个值」的内容类型（`Variant` 里装的是 `Integer` 还是 `Long`）。
    ///
    /// `Value` 本身不带窄类型，`Slot` / 数组 / UDT 字段 / 类字段都各自记了一份；唯独
    /// **返回 `Variant` 的函数**与**返回实参的 `IIf`/`Choose`/`Switch`** 这类路径把值直接交出去，
    /// 类型信息会丢。`VarType` / `TypeName` 需要它，所以在这些路径上顺手记一笔
    /// （只有求值这几个函数时才被读写，热路径没有开销）。
    result_narrow: Option<VbType>,
}

impl Drop for Runtime<'_, '_> {
    fn drop(&mut self) {
        // 脚本结束/中止时窗口可能还活着，系统之后仍可能回调：
        // 跳板与上下文留到进程结束，回调入口会用 `ACTIVE_RUNTIME` 判断并直接返回 0。
        for callback in &mut self.persistent_callbacks {
            if let Some(context) = callback.context.take() {
                std::mem::forget(context);
            }
            if let Some(trampoline) = callback.trampoline.take() {
                std::mem::forget(trampoline);
            }
        }
    }
}

thread_local! {
    /// 当前正在执行脚本的运行时；`Class_Terminate` 靠它在实例被回收时回调解释器。
    static ACTIVE_RUNTIME: std::cell::Cell<ActiveRuntime> =
        const { std::cell::Cell::new(ActiveRuntime { pointer: std::ptr::null_mut(), generation: 0 }) };
}

#[derive(Clone, Copy, Default)]
struct ActiveRuntime {
    pointer: *mut (),
    generation: u64,
}

/// 实例被回收时调用 `Class_Terminate`（由 `VbInstance::drop` 触发）。
pub(crate) fn terminate_instance(instance: &mut VbInstance) {
    let pointer = ACTIVE_RUNTIME.with(|slot| slot.get().pointer);
    if pointer.is_null() || !instance.begin_terminate() {
        return;
    }
    // SAFETY: 只在 `run_program` 执行期间有效，回调期间 `Runtime` 仍在栈上活着。
    let runtime = unsafe { &mut *(pointer as *mut Runtime<'static, 'static>) };
    runtime.terminate_instance(instance);
}

/// 执行频次统计：`qide-run --stats` 用它回答"钱花在哪"，也用来决定优化的先后。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunStats {
    pub statements: u64,
    pub expressions: u64,
    pub calls: u64,
    pub array_accesses: u64,
    pub variable_reads: u64,
}

/// 执行一个已解析的程序；`debug` 为 `Some` 时在每条语句前检查断点与暂停请求。
pub fn run_program(
    program: &Program,
    out: &mut dyn FnMut(&str),
    interrupt: &AtomicBool,
    options: &RunOptions,
    debug: Option<Arc<DebugControl>>,
) -> Result<(), RuntimeError> {
    run_program_with_stats(program, out, interrupt, options, debug).0
}

/// 同 [`run_program`]，另外返回执行频次统计。
pub fn run_program_with_stats(
    program: &Program,
    out: &mut dyn FnMut(&str),
    interrupt: &AtomicBool,
    options: &RunOptions,
    debug: Option<Arc<DebugControl>>,
) -> (Result<(), RuntimeError>, RunStats) {
    let com_initialization = com::initialize();
    let com_error = com_initialization.as_ref().err().cloned();
    let _com_apartment: Option<ApartmentGuard> = com_initialization.ok();

    let mut procedures = HashMap::new();
    let mut lets = HashMap::new();
    for procedure in &program.procedures {
        if procedure.is_let {
            lets.insert(procedure.name.clone(), procedure);
        } else {
            procedures.insert(procedure.name.clone(), procedure);
        }
    }
    let mut declares = HashMap::new();
    for declare in &program.declares {
        declares.insert(declare.name.clone(), declare);
    }
    let mut types = HashMap::new();
    for def in &program.types {
        types.insert(def.name.clone(), def);
    }
    let mut classes = HashMap::new();
    for def in &program.classes {
        classes.insert(def.name.clone(), def);
    }

    // `Enum` 成员注册成模块级常量；重名时报错。
    let mut globals = FastMap::default();
    for def in &program.enums {
        for (name, value) in &def.members {
            if globals.contains_key(name.as_str()) {
                return (
                    Err(RuntimeError::new(
                        format!("枚举 `{}` 的成员 `{name}` 与前面的定义重名", def.display),
                        def.line,
                    )),
                    RunStats::default(),
                );
            }
            globals.insert(
                std::borrow::Cow::Borrowed(name.as_str()),
                Slot::constant(Value::Int(*value), VbType::Long),
            );
        }
    }

    // 相对路径统一按脚本所在目录解析；没有脚本路径（例如测试）时退回进程目录。
    let script_dir = options
        .script_path
        .as_deref()
        .and_then(Path::parent)
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let generation = NEXT_RUNTIME_GENERATION.fetch_add(1, AtomicOrdering::Relaxed);
    let mut runtime = Runtime {
        generation,
        procedures,
        lets,
        declares,
        types,
        classes,
        globals,
        frames: Vec::new(),
        statics: FastMap::default(),
        option_explicit: program.option_explicit,
        option_compare_text: program.option_compare_text,
        def_types: program.def_types.clone(),
        allow_ui: options.allow_ui,
        with_stack: Vec::new(),
        module_error: None,
        module_resume: None,
        module_depth: 0,
        last_error: None,
        files: HashMap::new(),
        out,
        interrupt,
        steps: 0,
        max_steps: options.max_steps.max(1024),
        max_depth: options.max_depth.max(1),
        rng: 0x9E37_79B9_7F4A_7C15,
        started: Instant::now(),
        count_stats: options.count_stats,
        trace: options.trace,
        stats: RunStats::default(),
        dir: None,
        strings: Vec::new(),
        persistent_callbacks: Vec::new(),
        events: HashMap::new(),
        com_event_vars: program
            .main
            .iter()
            .filter_map(|stmt| match stmt {
                Stmt::Dim { items, .. } => Some(items),
                _ => None,
            })
            .flatten()
            .filter(|item| item.with_events)
            .map(|item| item.name.clone())
            .collect(),
        com_events: Vec::new(),
        pumping_events: false,
        debug,
        evaluating: false,
        command_line: options.command_line.clone(),
        script_dir: script_dir.clone(),
        cwd: script_dir,
        com_error,
        result_narrow: None,
    };

    // 脚本执行期间让 `Class_Terminate` 能找到解释器；结束时先释放变量再撤回指针。
    let engine = &mut runtime as *mut Runtime<'_, '_> as *mut ();
    let active = ActiveRuntime {
        pointer: engine,
        generation,
    };
    let previous = ACTIVE_RUNTIME.with(|slot| slot.replace(active));
    // 模块级只有声明（变量/常量）时仍自动调用 `Main`；有可执行的模块级语句时按脚本顺序执行。
    let module_only_declarations = program
        .main
        .iter()
        .all(|stmt| matches!(stmt, Stmt::Dim { .. } | Stmt::Const { .. }));
    let outcome = runtime.exec_block(&program.main).and_then(|_| {
        if module_only_declarations
            && let Some(main) = program.procedures.iter().find(|procedure| {
                procedure.name == "main" && !procedure.is_function && !procedure.is_let
            })
        {
            return runtime.call_user(main, &[], main.line).map(|_| ());
        }
        Ok(())
    });
    // 先回收脚本里的变量，让它们的 `Class_Terminate` 还能回调进来。
    // 这里用 `mem::take` 把内容整体搬出来再释放：`clear()` 是边删边释放，
    // 若 `Class_Terminate` 在释放途中回调进来查变量，就会看到已经释放的表项。
    let leftovers = (
        std::mem::take(&mut runtime.globals),
        std::mem::take(&mut runtime.frames),
        std::mem::take(&mut runtime.statics),
        std::mem::take(&mut runtime.with_stack),
    );
    drop(leftovers);
    ACTIVE_RUNTIME.with(|slot| slot.set(previous));
    (outcome, runtime.stats)
}
