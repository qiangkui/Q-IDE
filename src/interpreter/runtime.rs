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
struct Frame {
    locals: FastMap<String, Slot>,
    /// 函数返回值所在的槽（函数名即返回值）。
    result: Option<Slot>,
    /// 当前过程名，`Static` 局部变量按它归档。
    procedure: Option<String>,
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
    /// `Width #n, w`：`Print #` 的自动换行宽度；0 表示不换行。
    width: usize,
    /// 当前行已经写了多少字符（`;` 续行和 `,` 分区都靠它定位）。
    column: usize,
}

impl FileWriter {
    fn new(file: File) -> Self {
        Self {
            sink: BufWriter::new(file),
            width: 0,
            column: 0,
        }
    }

    fn file(&self) -> &File {
        self.sink.get_ref()
    }

    /// 写入一段文本：按 `Width #` 自动换行、`!suppress_newline` 时补换行，并跟踪列位置。
    fn write_line(&mut self, text: &str, suppress_newline: bool) -> std::io::Result<()> {
        let mut line = String::new();
        for ch in text.chars() {
            if self.width > 0 && self.column >= self.width {
                line.push('\n');
                self.column = 0;
            }
            line.push(ch);
            self.column += 1;
        }
        if !suppress_newline {
            line.push('\n');
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
    globals: FastMap<String, Slot>,
    /// 过程调用栈，只保存当前过程的局部变量。
    frames: Vec<Frame>,
    /// `Static` 局部变量：过程合 → 变量。
    statics: FastMap<String, FastMap<String, Slot>>,
    /// `Option Explicit`：未声明变量直接报错。
    option_explicit: bool,
    /// `Option Compare Text`：字符串比较忽略大小写。
    option_compare_text: bool,
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
            if globals.contains_key(name) {
                return (
                    Err(RuntimeError::new(
                        format!("枚举 `{}` 的成员 `{name}` 与前面的定义重名", def.display),
                        def.line,
                    )),
                    RunStats::default(),
                );
            }
            globals.insert(
                name.clone(),
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

impl<'p, 'o> Runtime<'p, 'o> {
    // ---------- 变量 ----------

    /// 把脚本里的路径变成实际路径：绝对路径原样返回，相对路径按脚本当前目录解析。
    fn resolve_path(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            self.cwd.join(path)
        }
    }

    fn resolve_moniker_path(&self, pathname: &str) -> String {
        if pathname.contains(':') || pathname.starts_with("new:") {
            pathname.to_string()
        } else {
            self.resolve_path(pathname).to_string_lossy().into_owned()
        }
    }

    fn find_slot(&self, name: &str) -> Option<Slot> {
        if let Some(frame) = self.frames.last()
            && let Some(slot) = frame.locals.get(name)
        {
            return Some(slot.clone());
        }
        self.globals.get(name).cloned()
    }

    /// 只读地借用槽（读变量走这条，省掉 `Slot` 克隆带来的 `Rc` 计数与释放）。
    fn lookup_slot(&self, name: &str) -> Option<&Slot> {
        if let Some(frame) = self.frames.last()
            && let Some(slot) = frame.locals.get(name)
        {
            return Some(slot);
        }
        self.globals.get(name)
    }

    /// 当前作用域：模块级写全局表，过程内写栈顶帧。
    fn scope_mut(&mut self) -> &mut FastMap<String, Slot> {
        match self.frames.last_mut() {
            Some(frame) => &mut frame.locals,
            None => &mut self.globals,
        }
    }

    fn resume_index(&self) -> Option<usize> {
        match self.frames.last() {
            Some(frame) => frame.resume_index,
            None => self.module_resume,
        }
    }

    fn set_error_handler(&mut self, handler: Option<ErrorMode>) {
        match self.frames.last_mut() {
            Some(frame) => frame.on_error = handler,
            None => self.module_error = handler,
        }
    }

    fn set_resume_index(&mut self, index: usize) {
        match self.frames.last_mut() {
            Some(frame) => frame.resume_index = Some(index),
            None => self.module_resume = Some(index),
        }
    }

    fn error_handler(&self) -> Option<ErrorMode> {
        match self.frames.last() {
            Some(frame) => frame.on_error.clone(),
            None => self.module_error.clone(),
        }
    }

    /// 进入一层语句块；返回它是否是过程 模块的根块。
    fn enter_block(&mut self) -> bool {
        match self.frames.last_mut() {
            Some(frame) => {
                let root = frame.depth == 0;
                frame.depth += 1;
                root
            }
            None => {
                let root = self.module_depth == 0;
                self.module_depth += 1;
                root
            }
        }
    }

    fn exit_block(&mut self) {
        match self.frames.last_mut() {
            Some(frame) => frame.depth = frame.depth.saturating_sub(1),
            None => self.module_depth = self.module_depth.saturating_sub(1),
        }
    }

    fn declare(&mut self, name: &str, ty: VbType, line: usize) -> Result<(), RuntimeError> {
        self.declare_slot(name, default_value(ty), ty, line)
            .map(|_| ())
    }

    /// 在当前作用域里声明变量；返回是否新建了单元。
    ///
    /// 同一条 `Dim` 语句被反复执行（例如写在循环体里）不算重复声明：
    /// VB 的声明是按过程算一次的，变量会保留上一轮的值。
    fn declare_slot(
        &mut self,
        name: &str,
        value: Value,
        ty: VbType,
        line: usize,
    ) -> Result<bool, RuntimeError> {
        let scope = self.scope_mut();
        if let Some(existing) = scope.get(name) {
            if existing.declared_at == line {
                return Ok(false);
            }
            return Err(RuntimeError::new(format!("变量 `{name}` 重复声明"), line));
        }
        scope.insert(name.to_string(), Slot::new(value, ty).at_line(line));
        Ok(true)
    }

    /// 取过程内的 `Static` 单元：首次调用时创建，之后复用。
    fn static_slot(
        &mut self,
        name: &str,
        value: Value,
        ty: VbType,
        line: usize,
    ) -> Result<Slot, RuntimeError> {
        let procedure = self
            .frames
            .last()
            .and_then(|frame| frame.procedure.clone())
            .ok_or_else(|| {
                RuntimeError::new(format!("`Static` 只能用在过程内部（`{name}`）"), line)
            })?;
        if let Some(slot) = self.statics.get(&procedure).and_then(|vars| vars.get(name)) {
            return Ok(slot.clone());
        }
        let slot = Slot::new(value, ty);
        self.statics
            .entry(procedure)
            .or_default()
            .insert(name.to_string(), slot.clone());
        Ok(slot)
    }

    fn set_var(
        &mut self,
        name: &str,
        value: Value,
        line: usize,
        from: Option<&Expr>,
    ) -> Result<(), RuntimeError> {
        match self.lookup_slot(name) {
            Some(slot) => self.store_in(slot, name, value, line, from),
            None => {
                if self.option_explicit {
                    return Err(RuntimeError::new(
                        format!("变量 `{name}` 未声明（Option Explicit）"),
                        line,
                    ));
                }
                let narrow = self.narrow_of(from);
                let slot = Slot::new(value.copy_value(), VbType::Variant);
                slot.set_narrow(narrow);
                self.scope_mut().insert(name.to_string(), slot);
                Ok(())
            }
        }
    }

    /// `&` 拼接时把 `Single` 值先按 7 位有效数字渲染成字符串（VB6 实测）；其它值原样不动。
    fn render_single(&self, expr: &Expr, value: &mut Value) {
        if let Value::Float(number) = value
            && self.static_type(expr) == Some(VbType::Single)
        {
            *value = Value::Str(crate::interpreter::value::format_single(*number));
        }
    }

    /// `Variant` 槽要记下的内容类型（`Variant` / 对象 / 推断不出来的都记为 `None`）。
    fn narrow_of(&self, from: Option<&Expr>) -> Option<VbType> {
        let ty = self.static_type(from?)?;
        (!matches!(ty, VbType::Variant | VbType::Any | VbType::Object)).then_some(ty)
    }

    /// 写入已有的单元（`set_var` 与循环计数器共用）。
    fn store_in(
        &self,
        slot: &Slot,
        name: &str,
        value: Value,
        line: usize,
        from: Option<&Expr>,
    ) -> Result<(), RuntimeError> {
        if slot.is_readonly() {
            return Err(RuntimeError::new(format!("不能给常量 `{name}` 赋值"), line));
        }
        if slot.is_array_binding() && !value.is_array() {
            return Err(RuntimeError::new(
                format!("`{name}` 是数组，不能整体赋值为普通值"),
                line,
            ));
        }
        slot.set(value)
            .map_err(|message| RuntimeError::from_coerce(message, line))?;
        // 写进 `Variant` 时记下值的 VB6 类型（实测 `v = i`（Integer）之后 `VarType(v)` 是 2）。
        if slot.ty() == VbType::Variant {
            slot.set_narrow(self.narrow_of(from));
        }
        Ok(())
    }

    // ---------- 步数 / 中断 ----------

    fn tick(&mut self, line: usize) -> Result<(), RuntimeError> {
        self.steps += 1;
        if self.steps.is_multiple_of(1024) {
            if self.interrupt.load(AtomicOrdering::Relaxed) {
                return Err(RuntimeError::new("脚本已被中断", line));
            }
            if self.steps > self.max_steps {
                return Err(RuntimeError::new(
                    "脚本执行步数超过上限，可能是死循环",
                    line,
                ));
            }
        }
        Ok(())
    }

    /// 调试模式下，在每条语句执行前把位置与变量交给界面，必要时在这里阻塞。
    fn debug_check(&mut self, line: usize) -> Result<(), RuntimeError> {
        // 监视表达式执行期间不再停下来：解释器线程本来就卡在 `before_line` 里，
        // 再次进入会等一个永远不会到来的「继续」。
        if self.evaluating || self.debug.is_none() {
            return Ok(());
        }
        let Some(control) = self.debug.clone() else {
            return Ok(());
        };
        let mut query = |request: DebugQuery<'_>| match request {
            DebugQuery::Snapshot => DebugReply::Snapshot(self.debug_snapshot(line)),
            DebugQuery::Evaluate(expression) => {
                DebugReply::Value(self.debug_evaluate(expression, line))
            }
            DebugQuery::Condition(expression) => {
                DebugReply::Condition(self.debug_condition(expression, line))
            }
        };
        if control.before_line(line, &mut query) == DebugSignal::Stop {
            return Err(RuntimeError::new(debug::STOPPED_MESSAGE, line));
        }
        Ok(())
    }

    /// 在暂停处求值一条监视表达式，给出显示文本与类型名。
    fn debug_evaluate(&mut self, expression: &str, line: usize) -> Result<DebugValue, String> {
        let value = self.debug_eval_value(expression, line)?;
        Ok(DebugValue {
            value: value.debug_preview(DEBUG_VALUE_CHARS),
            ty: value.type_name(),
        })
    }

    /// 求值条件断点的条件。
    fn debug_condition(&mut self, expression: &str, line: usize) -> Result<bool, String> {
        let value = self.debug_eval_value(expression, line)?;
        value.to_bool()
    }

    /// 调试器借一次运行时求值表达式；期间不再触发暂停检查。
    fn debug_eval_value(&mut self, expression: &str, line: usize) -> Result<Value, String> {
        let expr = crate::interpreter::parser::parse_expression(expression)
            .map_err(|error| error.message)?;
        self.evaluating = true;
        let outcome = self.eval(&expr, line);
        self.evaluating = false;
        outcome.map_err(|error| error.message.to_string())
    }

    fn debug_snapshot(&self, line: usize) -> DebugSnapshot {
        let mut variables = Vec::new();
        for (name, slot) in &self.globals {
            variables.push(DebugVariable {
                scope: "模块".to_string(),
                name: name.clone(),
                ty: slot.ty().name().to_string(),
                value: slot.value().debug_preview(DEBUG_VALUE_CHARS),
            });
        }
        let procedure = match self.frames.last() {
            Some(frame) => frame
                .procedure
                .clone()
                .unwrap_or_else(|| "过程".to_string()),
            None => "模块".to_string(),
        };
        if let Some(frame) = self.frames.last() {
            for (name, slot) in &frame.locals {
                variables.push(DebugVariable {
                    scope: procedure.clone(),
                    name: name.clone(),
                    ty: slot.ty().name().to_string(),
                    value: slot.value().debug_preview(DEBUG_VALUE_CHARS),
                });
            }
        }
        variables.sort_by(|left, right| {
            left.scope
                .cmp(&right.scope)
                .then_with(|| left.name.cmp(&right.name))
        });

        // 调用栈：从内往外走，每一层的行号由它的调用者给出。
        let mut call_stack = Vec::new();
        let mut caller_line = line;
        for frame in self.frames.iter().rev() {
            call_stack.push(debug::CallFrame {
                procedure: frame
                    .procedure
                    .clone()
                    .unwrap_or_else(|| "过程".to_string()),
                line: caller_line,
            });
            caller_line = frame.call_line;
        }
        call_stack.push(debug::CallFrame {
            procedure: "模块".to_string(),
            line: caller_line,
        });
        call_stack.reverse();

        DebugSnapshot {
            line,
            procedure,
            variables,
            call_stack,
        }
    }

    // ---------- 语句 ----------

    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<Flow, RuntimeError> {
        let is_root = self.enter_block();
        let outcome = self.exec_block_inner(stmts, is_root);
        self.exit_block();
        outcome
    }

    fn exec_block_inner(&mut self, stmts: &[Stmt], is_root: bool) -> Result<Flow, RuntimeError> {
        let mut index = 0usize;
        let mut gosub_returns = Vec::new();
        while index < stmts.len() {
            // COM 事件在语句边界分派：回调里只排队，避免重入解释器。
            self.drain_com_events()?;
            match self.exec(&stmts[index]) {
                Ok(Flow::Normal) => index += 1,
                Ok(Flow::Goto(label)) => match find_label(stmts, &label) {
                    Some(position) => index = position,
                    None if is_root => {
                        return Err(RuntimeError::new(
                            format!("找不到标签 `{label}`"),
                            stmt_line(&stmts[index]),
                        ));
                    }
                    None => return Ok(Flow::Goto(label)),
                },
                Ok(Flow::Gosub(label)) => match find_label(stmts, &label) {
                    Some(position) if is_root => {
                        gosub_returns.push(index + 1);
                        index = position;
                    }
                    Some(_) => return Ok(Flow::Gosub(label)),
                    None if is_root => {
                        return Err(RuntimeError::new(
                            format!("找不到标签 `{label}`"),
                            stmt_line(&stmts[index]),
                        ));
                    }
                    None => return Ok(Flow::Gosub(label)),
                },
                Ok(Flow::GosubReturn) if is_root => {
                    let Some(return_to) = gosub_returns.pop() else {
                        return Err(RuntimeError::new(
                            "`Return` 没有匹配的 `GoSub`",
                            stmt_line(&stmts[index]),
                        ));
                    };
                    index = return_to;
                }
                Ok(Flow::GosubReturn) => return Ok(Flow::GosubReturn),
                Ok(Flow::ResumeNext(at)) => {
                    if is_root {
                        index = at + 1;
                    } else {
                        return Ok(Flow::ResumeNext(at));
                    }
                }
                Ok(Flow::ResumeAt(at)) => {
                    if is_root {
                        index = at;
                    } else {
                        return Ok(Flow::ResumeAt(at));
                    }
                }
                Ok(flow) => return Ok(flow),
                Err(error) => match self.error_handler() {
                    Some(ErrorMode::ResumeNext) => {
                        self.last_error = Some(error);
                        index += 1;
                    }
                    Some(ErrorMode::Goto(label)) if is_root => {
                        let line = error.line;
                        self.last_error = Some(error);
                        self.set_resume_index(index);
                        match find_label(stmts, &label) {
                            Some(position) => index = position,
                            None => {
                                return Err(RuntimeError::new(
                                    format!("找不到标签 `{label}`"),
                                    line,
                                ));
                            }
                        }
                    }
                    // 嵌套块里遇到 `On Error GoTo`：交给根块统一跳转。
                    _ => return Err(error),
                },
            }
        }
        Ok(Flow::Normal)
    }

    fn exec(&mut self, stmt: &Stmt) -> Result<Flow, RuntimeError> {
        let line = stmt_line(stmt);
        if self.count_stats {
            self.stats.statements += 1;
        }
        if self.trace {
            (self.out)(&format!(
                "{} 第 {line} 行",
                crate::interpreter::TRACE_PREFIX
            ));
        }
        self.tick(line)?;
        self.debug_check(line)?;

        match stmt {
            Stmt::Dim {
                items,
                static_,
                line,
            } => {
                for item in items {
                    let (value, slot_ty) = self.declared_value(item, *line)?;
                    if *static_ {
                        let slot = self.static_slot(&item.name, value, slot_ty, *line)?;
                        let frame = self.frames.last_mut().ok_or_else(|| {
                            RuntimeError::new(
                                format!("`Static` 只能用在过程内部（`{}`）", item.name),
                                *line,
                            )
                        })?;
                        if let Some(existing) = frame.locals.get(&item.name) {
                            if existing.declared_at == *line {
                                continue;
                            }
                            return Err(RuntimeError::new(
                                format!("变量 `{}` 重复声明", item.name),
                                *line,
                            ));
                        }
                        frame.locals.insert(item.name.clone(), slot);
                    } else if !self.declare_slot(&item.name, value.clone(), slot_ty, *line)? {
                        // 同一条 `Dim` 又跑了一遍（循环体里）：变量保留，初值重新算。
                        if item.initializer.is_some() {
                            self.set_var(&item.name, value, *line, None)?;
                        }
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Redim {
                name,
                bounds,
                ty,
                preserve,
                line,
            } => {
                let dims = self.array_dims(bounds, *line)?;
                let slot = self
                    .find_slot(name)
                    .ok_or_else(|| RuntimeError::new(format!("未定义的数组 `{name}`"), *line))?;
                if slot.is_readonly() {
                    return Err(RuntimeError::new(
                        format!("不能给常量 `{name}` 赋值"),
                        *line,
                    ));
                }
                let value = slot.value();
                let array = value
                    .as_array()
                    .map_err(|message| RuntimeError::new(format!("`{name}` {message}"), *line))?;
                let mut array = array.borrow_mut();
                if !array.is_dynamic() {
                    return Err(RuntimeError::new(
                        format!("`{name}` 是静态数组，声明时已固定大小，不能用 ReDim"),
                        *line,
                    ));
                }
                if *preserve && !array.dims().is_empty() {
                    let old = array.dims();
                    if old.len() != dims.len() {
                        return Err(RuntimeError::new(
                            format!("`ReDim Preserve` 不能改变 `{name}` 的维数"),
                            *line,
                        ));
                    }
                    // 与 VB 一致：`Preserve` 只能调整最后一维的大小。
                    if old[..old.len() - 1] != dims[..dims.len() - 1] {
                        return Err(RuntimeError::new(
                            format!("`ReDim Preserve` 只能改变 `{name}` 最后一维的大小"),
                            *line,
                        )
                        .with_number(9));
                    }
                }
                array.redim(dims, *ty, *preserve);
                Ok(Flow::Normal)
            }
            Stmt::Erase { names, line } => {
                for name in names {
                    let slot = self.find_slot(name).ok_or_else(|| {
                        RuntimeError::new(format!("未定义的数组 `{name}`"), *line)
                    })?;
                    if slot.is_readonly() {
                        return Err(RuntimeError::new(
                            format!("不能给常量 `{name}` 赋值"),
                            *line,
                        ));
                    }
                    let value = slot.value();
                    let array = value.as_array().map_err(|message| {
                        RuntimeError::new(format!("`{name}` {message}"), *line)
                    })?;
                    array.borrow_mut().erase();
                }
                Ok(Flow::Normal)
            }
            Stmt::Const {
                name,
                ty,
                value,
                line,
            } => {
                let value = self.eval(value, *line)?;
                let coerced = value
                    .coerce(*ty)
                    .map_err(|message| RuntimeError::from_coerce(message, *line))?;
                self.scope_mut()
                    .insert(name.clone(), Slot::constant(coerced, *ty));
                Ok(Flow::Normal)
            }
            Stmt::Assign {
                target,
                value,
                suffix,
                line,
            } => {
                // `Property Let` 很少见：没有任何 Let 过程时跳过这次变量查找。
                if !self.lets.is_empty()
                    && self.find_slot(target).is_none()
                    && let Some(procedure) = self.lets.get(target).copied()
                {
                    return self.call_property_let(procedure, &[], value, *line);
                }
                let evaluated = self.eval(value, *line)?;
                // 只有带类型后缀（`x$ = ...`）才可能在这里隐式声明。
                if !self.option_explicit
                    && suffix.is_some()
                    && self.find_slot(target).is_none()
                    && let Some(ty) = suffix.and_then(VbType::from_suffix)
                {
                    self.declare(target, ty, *line)?;
                }
                // `Variant` 变量要记下右侧表达式的静态类型（见 `Slot::narrow_type`）。
                self.set_var(target, evaluated, *line, Some(value))?;
                Ok(Flow::Normal)
            }
            Stmt::AssignIndex {
                name,
                indices,
                value: rhs,
                line,
            } => {
                if !self.lets.is_empty()
                    && self.find_slot(name).is_none()
                    && let Some(procedure) = self.lets.get(name).copied()
                {
                    return self.call_property_let(procedure, indices, rhs, *line);
                }
                let value = self.eval(rhs, *line)?;
                let slot = self
                    .find_slot(name)
                    .ok_or_else(|| RuntimeError::new(format!("未定义的变量 `{name}`"), *line))?;
                if slot.is_readonly() {
                    return Err(RuntimeError::new(
                        format!("不能给常量 `{name}` 赋值"),
                        *line,
                    ));
                }
                let current = slot.value();
                if let Ok(entries) = current.as_dictionary() {
                    // `d("键") = 值`：字典按键写入（不存在则新增）。
                    let key = match indices.as_slice() {
                        [single] => self.eval(single, *line)?,
                        _ => {
                            return Err(RuntimeError::new(
                                "`Dictionary` 的下标只能是一个键",
                                *line,
                            ));
                        }
                    };
                    let key = VbDictionary::key_of(&key)
                        .map_err(|message| RuntimeError::new(message, *line))?;
                    let narrow = self.narrow_of(Some(rhs));
                    entries.borrow_mut().set(key, value, narrow);
                    return Ok(Flow::Normal);
                }
                if let Value::ComObject(object) = &current {
                    // `d("键") = 值`：COM 对象的默认成员（`DISPID_VALUE`）写入（VB6 实测可用）。
                    let mut index_values = Vec::with_capacity(indices.len());
                    for expr in indices {
                        index_values.push(self.eval(expr, *line)?);
                    }
                    object
                        .set_default(&index_values, &value)
                        .map_err(|failure| {
                            RuntimeError::new(failure.message, *line).with_number(failure.number)
                        })?;
                    return Ok(Flow::Normal);
                }
                let indices = self.eval_indices(indices, *line)?;
                if self.count_stats {
                    self.stats.array_accesses += 1;
                }
                let array = current
                    .as_array()
                    .map_err(|message| RuntimeError::new(format!("`{name}` {message}"), *line))?;
                // `Variant` 数组要记下元素的内容类型（`arr(0) = CByte(9)` 之后 `VarType(arr(0))` 是 17）。
                let narrow = if array.borrow().element_type() == VbType::Variant {
                    self.narrow_of(Some(rhs))
                } else {
                    None
                };
                array
                    .borrow_mut()
                    .set(&indices, value, narrow)
                    .map_err(|message| {
                        if message.contains("溢出") {
                            RuntimeError::from_coerce(format!("数组 `{name}`：{message}"), *line)
                        } else {
                            RuntimeError::new(format!("数组 `{name}`：{message}"), *line)
                                .with_number(9)
                        }
                    })?;
                Ok(Flow::Normal)
            }
            Stmt::AssignMember {
                base,
                path,
                value,
                set_op,
                line,
            } => {
                let root = self.eval(base, *line)?;
                if matches!(root, Value::ComObject(_)) {
                    // `obj.Sub.Prop = v`：中间层按属性读出来，赋值落在最后一层。
                    let mut target = root.clone();
                    for member in &path[..path.len().saturating_sub(1)] {
                        target = self.read_member(&Expr::Literal(target), member, *line)?;
                    }
                    let last = path.last().cloned().unwrap_or_default();
                    let value = self.eval(value, *line)?;
                    let Value::ComObject(object) = &target else {
                        return Err(RuntimeError::new(
                            format!("`{last}` 所在的值不是 COM 对象"),
                            *line,
                        )
                        .with_number(424));
                    };
                    object.set(&last, &value).map_err(|failure| {
                        RuntimeError::new(failure.message, *line).with_number(failure.number)
                    })?;
                    return Ok(Flow::Normal);
                }
                if let Value::Class(instance) = &root {
                    self.assign_instance_member(&instance.0, path, value, *set_op, *line)?;
                    return Ok(Flow::Normal);
                }
                if !root.is_record() {
                    return Err(RuntimeError::new("成员赋值的目标不是记录类型", *line));
                }
                // `Variant` 字段要记下右侧表达式的静态类型（`u.f = i` 之后 `VarType(u.f)` 是 2）。
                let narrow = self.narrow_of(Some(value));
                let value = self.eval(value, *line)?;
                self.assign_member(root, path, value, narrow, *line)?;
                Ok(Flow::Normal)
            }
            Stmt::AssignMemberIndex {
                base,
                path,
                indices,
                member_path,
                value,
                set_op,
                line,
            } => {
                let root = self.eval(base, *line)?;
                if matches!(root, Value::ComObject(_)) {
                    // 中间层按属性读出来（`rs.Fields`），再分两种情形：
                    // `d.Item("k") = v` 走索引属性写入，`rs.Fields("id").Value = v` 则读出中间对象再设成员。
                    let mut target = root.clone();
                    for member in &path[..path.len().saturating_sub(1)] {
                        target = self.read_member(&Expr::Literal(target), member, *line)?;
                    }
                    let Some(member) = path.last().cloned() else {
                        return Err(RuntimeError::new("缺少成员名", *line));
                    };
                    let Value::ComObject(com) = &target else {
                        return Err(RuntimeError::new("COM 索引属性的目标不是对象", *line));
                    };
                    let mut index_values = Vec::with_capacity(indices.len());
                    for expr in indices {
                        index_values.push(self.eval(expr, *line)?);
                    }
                    let value = self.eval(value, *line)?;
                    if member_path.is_empty() {
                        com.set_indexed(&member, &index_values, &value)
                            .map_err(|failure| {
                                RuntimeError::new(failure.message, *line)
                                    .with_number(failure.number)
                            })?;
                        return Ok(Flow::Normal);
                    }
                    let mut nested =
                        self.call_com_with_values(com, &member, index_values, *line)?;
                    for step in &member_path[..member_path.len() - 1] {
                        nested = self.read_member(&Expr::Literal(nested), step, *line)?;
                    }
                    let last = member_path.last().cloned().unwrap_or_default();
                    let Value::ComObject(object) = &nested else {
                        return Err(RuntimeError::new(
                            format!("`{last}` 所在的值不是 COM 对象"),
                            *line,
                        )
                        .with_number(424));
                    };
                    object.set(&last, &value).map_err(|failure| {
                        RuntimeError::new(failure.message, *line).with_number(failure.number)
                    })?;
                    return Ok(Flow::Normal);
                }
                if let Value::Class(instance) = &root {
                    self.assign_instance_index(
                        &instance.0,
                        path.first().map(String::as_str).unwrap_or_default(),
                        indices,
                        value,
                        *set_op,
                        *line,
                    )?;
                    return Ok(Flow::Normal);
                }
                let indices = self.eval_indices(indices, *line)?;
                let value = self.eval(value, *line)?;
                let (parent, last) = Self::record_parent(root, path, *line)?;
                let field = {
                    let parent = parent.borrow();
                    parent.get(&last).ok_or_else(|| {
                        RuntimeError::new(
                            format!("类型 `{}` 没有字段 `{last}`", parent.type_name()),
                            *line,
                        )
                    })?
                };
                let array = field
                    .as_array()
                    .map_err(|_| RuntimeError::new(format!("字段 `{last}` 不是数组"), *line))?;
                if member_path.is_empty() {
                    array
                        .borrow_mut()
                        .set(&indices, value, None)
                        .map_err(|message| {
                            if message.contains("溢出") {
                                RuntimeError::from_coerce(
                                    format!("字段 `{last}`：{message}"),
                                    *line,
                                )
                            } else {
                                RuntimeError::new(format!("字段 `{last}`：{message}"), *line)
                                    .with_number(9)
                            }
                        })?;
                } else {
                    let element = array.borrow().get(&indices).map_err(|message| {
                        RuntimeError::new(format!("字段 `{last}`：{message}"), *line).with_number(9)
                    })?;
                    if !element.is_record() {
                        return Err(RuntimeError::new(
                            format!("字段 `{last}` 的元素不是记录类型"),
                            *line,
                        ));
                    }
                    self.assign_member(element, member_path, value, None, *line)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::AssignWithMember { path, value, line } => {
                let Some(root) = self.with_stack.last().cloned() else {
                    return Err(RuntimeError::new(
                        "`.成员` 赋值只能出现在 `With` 块内",
                        *line,
                    ));
                };
                let value = self.eval(value, *line)?;
                match &root {
                    Value::ComObject(object) if path.len() == 1 => {
                        object.set(&path[0], &value).map_err(|failure| {
                            RuntimeError::new(failure.message, *line).with_number(failure.number)
                        })?;
                    }
                    Value::ComObject(_) => {
                        return Err(RuntimeError::new("COM 属性暂不支持链式成员赋值", *line));
                    }
                    Value::Class(instance) => {
                        self.assign_instance_member(
                            &instance.0,
                            path,
                            &Expr::Literal(value),
                            true,
                            *line,
                        )?;
                    }
                    _ => self.assign_member(root, path, value, None, *line)?,
                }
                Ok(Flow::Normal)
            }
            Stmt::WithCall { path, args, line } => {
                let Some(mut receiver) = self.with_stack.last().cloned() else {
                    return Err(RuntimeError::new(
                        "`.成员` 调用只能出现在 `With` 块内",
                        *line,
                    ));
                };
                let Some((method, parents)) = path.split_last() else {
                    return Err(RuntimeError::new("缺少成员名", *line));
                };
                for parent in parents {
                    receiver = self.read_member(&Expr::Literal(receiver), parent, *line)?;
                }
                match receiver {
                    Value::Class(instance) => {
                        self.call_class_method(&instance.0, method, args, *line)?;
                    }
                    Value::ComObject(object) => {
                        self.call_com_member(&object, method, args, *line)?;
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            format!("`.{method}` 的接收对象不支持方法调用"),
                            *line,
                        ));
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::With {
                subject,
                body,
                line,
            } => {
                let value = self.eval(subject, *line)?;
                if !value.is_record() && !value.is_object() {
                    return Err(RuntimeError::new("`With` 需要记录或对象值", *line));
                }
                self.with_stack.push(value);
                let outcome = self.exec_block(body);
                self.with_stack.pop();
                outcome
            }
            Stmt::Call {
                name,
                args,
                by_value_args,
                line,
            } => {
                self.call_with_by_value_args(name, args, by_value_args, *line)?;
                Ok(Flow::Normal)
            }
            Stmt::Set {
                target,
                value,
                line,
            } => {
                let value = self.eval(value, *line)?;
                if !value.is_collection()
                    && !value.is_dictionary()
                    && !matches!(
                        value,
                        Value::Class(_) | Value::ComObject(_) | Value::Picture(_) | Value::Empty
                    )
                {
                    return Err(RuntimeError::new(
                        "`Set` 目前支持 `Collection`、`Dictionary`、`New 类名`、COM 对象、`LoadPicture` 与 `Nothing`",
                        *line,
                    ));
                }
                if self.find_slot(target).is_none() {
                    self.declare(target, VbType::Object, *line)?;
                }
                self.connect_events_for_name(target, &value, *line)?;
                self.set_var(target, value, *line, None)?;
                Ok(Flow::Normal)
            }
            Stmt::CallMethod {
                object,
                path,
                args,
                line,
            } => {
                let method = path.last().cloned().unwrap_or_default();
                let slot = self.find_slot(object);
                let Some(slot) = slot else {
                    // `Bump .Y`（过程调用 + `With` 相对实参）与 `obj.Member` 的记号完全一样，
                    // 这里按「`object` 不是变量」回退成普通调用。
                    let call_args = if args.is_empty() {
                        vec![Expr::WithMember { path: path.clone() }]
                    } else {
                        args.clone()
                    };
                    self.call(object, &call_args, *line)?;
                    return Ok(Flow::Normal);
                };
                // `rs.Fields.Append "x", 3`：先逐层读出中间成员，再在最后一层上调用。
                let mut target = slot.value();
                for member in &path[..path.len().saturating_sub(1)] {
                    target = self.read_member(&Expr::Literal(target), member, *line)?;
                }
                if let Value::Class(instance) = &target {
                    self.call_class_method(&instance.0, &method, args, *line)?;
                    return Ok(Flow::Normal);
                }
                if let Value::ComObject(object) = &target {
                    self.call_com_member(object, &method, args, *line)?;
                    return Ok(Flow::Normal);
                }
                let mut values = Vec::with_capacity(args.len());
                for expr in args {
                    values.push(self.eval(expr, *line)?);
                }
                if let Ok(collection) = target.as_collection() {
                    match method.as_str() {
                        "add" => {
                            let Some(item) = values.first() else {
                                return Err(RuntimeError::new("`Add` 缺少要添加的元素", *line));
                            };
                            if values.len() > 2 {
                                return Err(RuntimeError::new(
                                    "`Add` 只支持「元素, 键」两个参数",
                                    *line,
                                ));
                            }
                            let key = values.get(1).map(|value| value.to_display());
                            let narrow = args.first().and_then(|expr| self.narrow_of(Some(expr)));
                            collection
                                .borrow_mut()
                                .add(item.clone(), key, narrow)
                                .map_err(|message| RuntimeError::new(message, *line))?;
                        }
                        "remove" => {
                            let Some(key) = values.first() else {
                                return Err(RuntimeError::new("`Remove` 缺少下标或键", *line));
                            };
                            collection.borrow_mut().remove(key).map_err(|message| {
                                RuntimeError::new(message, *line).with_number(9)
                            })?;
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!("`Collection` 没有方法 `{method}`"),
                                *line,
                            ));
                        }
                    }
                    return Ok(Flow::Normal);
                }
                if let Ok(entries) = target.as_dictionary() {
                    match method.as_str() {
                        "add" => {
                            if values.len() != 2 {
                                return Err(RuntimeError::new(
                                    "`Dictionary.Add` 需要「键, 值」两个参数",
                                    *line,
                                ));
                            }
                            let key = VbDictionary::key_of(&values[0])
                                .map_err(|message| RuntimeError::new(message, *line))?;
                            let narrow = args.get(1).and_then(|expr| self.narrow_of(Some(expr)));
                            entries
                                .borrow_mut()
                                .add(key, values[1].clone(), narrow)
                                .map_err(|message| RuntimeError::new(message, *line))?;
                        }
                        "remove" => {
                            let Some(key) = values.first() else {
                                return Err(RuntimeError::new("`Remove` 缺少键", *line));
                            };
                            let key = VbDictionary::key_of(key)
                                .map_err(|message| RuntimeError::new(message, *line))?;
                            entries.borrow_mut().remove(&key).map_err(|message| {
                                RuntimeError::new(message, *line).with_number(9)
                            })?;
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!("`Dictionary` 没有方法 `{method}`"),
                                *line,
                            ));
                        }
                    }
                    return Ok(Flow::Normal);
                }
                Err(RuntimeError::new(
                    format!("`{object}` 不是 `Collection` 或 `Dictionary`，不能调用 `{method}`"),
                    *line,
                ))
            }
            Stmt::Print {
                values,
                seps,
                suppress_newline,
                line,
            } => {
                let _ = suppress_newline; // `Debug.Print` 每次输出一整行
                let text = self.render_print(values, seps, 0, *line)?;
                (self.out)(&text);
                Ok(Flow::Normal)
            }
            Stmt::If {
                branches,
                otherwise,
                line,
            } => {
                for (cond, body) in branches {
                    if self.eval_condition(cond, *line)? {
                        return self.exec_block(body);
                    }
                }
                match otherwise {
                    Some(body) => self.exec_block(body),
                    None => Ok(Flow::Normal),
                }
            }
            Stmt::For {
                var,
                start,
                end,
                step,
                body,
                line,
            } => self.exec_for(var, start, end, step.as_ref(), body, *line),
            Stmt::ForEach {
                var,
                collection,
                body,
                line,
            } => self.exec_for_each(var, collection, body, *line),
            Stmt::DoLoop {
                pre,
                post,
                body,
                line,
            } => self.exec_do(pre.as_ref(), post.as_ref(), body, *line),
            Stmt::While { cond, body, line } => {
                loop {
                    self.tick(*line)?;
                    if !self.eval_condition(cond, *line)? {
                        break;
                    }
                    match self.exec_block(body)? {
                        Flow::Normal => {}
                        Flow::ExitFor | Flow::ExitDo => break,
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Select {
                subject,
                arms,
                otherwise,
                line,
            } => self.exec_select(subject, arms, otherwise.as_ref(), *line),
            Stmt::Exit { kind, .. } => Ok(match kind {
                ExitKind::For => Flow::ExitFor,
                ExitKind::Do => Flow::ExitDo,
                ExitKind::Sub | ExitKind::Function => Flow::ExitProc,
            }),
            Stmt::Return { value, line } => {
                let value = match value {
                    Some(expr) => self.eval(expr, *line)?,
                    None => Value::Empty,
                };
                if let Some(slot) = self.frames.last().and_then(|frame| frame.result.clone()) {
                    slot.set(value)
                        .map_err(|message| RuntimeError::from_coerce(message, *line))?;
                }
                Ok(Flow::ExitProc)
            }
            Stmt::GosubReturn { .. } => Ok(Flow::GosubReturn),
            Stmt::Halt { .. } => Ok(Flow::Halt),
            Stmt::Label(..) => Ok(Flow::Normal),
            Stmt::Goto(label, _) => Ok(Flow::Goto(label.clone())),
            Stmt::Gosub(label, _) => Ok(Flow::Gosub(label.clone())),
            Stmt::OnError { mode, .. } => {
                self.set_error_handler(Some(mode.clone()));
                // VB6 里执行任何 `On Error` 语句都会把 `Err` 清空（本机实测：
                // `On Error Resume Next` / `GoTo 0` / `GoTo -1` 都清）。
                self.last_error = None;
                Ok(Flow::Normal)
            }
            Stmt::Resume { target, line } => match target {
                ResumeTarget::Label(label) => Ok(Flow::Goto(label.clone())),
                ResumeTarget::Next => match self.resume_index() {
                    Some(index) => Ok(Flow::ResumeNext(index)),
                    None => Err(RuntimeError::new("`Resume Next` 之前没有发生错误", *line)),
                },
                ResumeTarget::Current => match self.resume_index() {
                    Some(index) => Ok(Flow::ResumeAt(index)),
                    None => Err(RuntimeError::new("`Resume` 之前没有发生错误", *line)),
                },
            },
            Stmt::ErrClear { .. } => {
                self.last_error = None;
                Ok(Flow::Normal)
            }
            Stmt::OpenFile {
                path,
                mode,
                number,
                record_len,
                line,
            } => {
                let path = self.eval(path, *line)?.to_display();
                let resolved = self.resolve_path(&path);
                let number = match number {
                    Some(number) => self.file_number(number, *line)?,
                    None => self.free_file_number(),
                };
                if self.files.contains_key(&number) {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 已经被占用"), *line)
                            .with_number(55),
                    );
                }
                let record_len = match record_len {
                    Some(expr) => {
                        if *mode != FileMode::Random {
                            return Err(RuntimeError::new("`Len =` 只能用于 `For Random`", *line));
                        }
                        let value = self.eval(expr, *line)?.as_int().map_err(|message| {
                            RuntimeError::new(format!("记录长度必须是整数：{message}"), *line)
                        })?;
                        if !(1..=32767).contains(&value) {
                            return Err(RuntimeError::new(
                                format!("记录长度 {value} 超出范围（1..32767）"),
                                *line,
                            ));
                        }
                        Some(value as usize)
                    }
                    None if *mode == FileMode::Random => Some(DEFAULT_RECORD_LEN),
                    None => None,
                };
                let handle = match mode {
                    FileMode::Input => {
                        let file = File::open(&resolved).map_err(|error| {
                            RuntimeError::new(format!("无法打开 `{path}`：{error}"), *line)
                                .with_number(open_error_number(&error, &resolved))
                        })?;
                        FileHandle::Reader(BufReader::new(file))
                    }
                    FileMode::Output => {
                        let file = File::create(&resolved).map_err(|error| {
                            RuntimeError::new(format!("无法创建 `{path}`：{error}"), *line)
                                .with_number(open_error_number(&error, &resolved))
                        })?;
                        FileHandle::Writer(FileWriter::new(file))
                    }
                    FileMode::Append => {
                        let file = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&resolved)
                            .map_err(|error| {
                                RuntimeError::new(format!("无法追加 `{path}`：{error}"), *line)
                                    .with_number(open_error_number(&error, &resolved))
                            })?;
                        FileHandle::Writer(FileWriter::new(file))
                    }
                    FileMode::Binary | FileMode::Random => {
                        let file = OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .truncate(false)
                            .open(&resolved)
                            .map_err(|error| {
                                RuntimeError::new(format!("无法打开 `{path}`：{error}"), *line)
                                    .with_number(open_error_number(&error, &resolved))
                            })?;
                        FileHandle::Binary(BinaryFile::new(file, record_len))
                    }
                };
                self.files.insert(number, handle);
                Ok(Flow::Normal)
            }
            Stmt::CloseFile { numbers, line } => {
                if numbers.is_empty() {
                    for (_, handle) in self.files.drain() {
                        flush_file(handle);
                    }
                    return Ok(Flow::Normal);
                }
                for expr in numbers {
                    let number = self.file_number(expr, *line)?;
                    let Some(handle) = self.files.remove(&number) else {
                        return Err(
                            RuntimeError::new(format!("文件号 #{number} 没有打开"), *line)
                                .with_number(52),
                        );
                    };
                    flush_file(handle);
                }
                Ok(Flow::Normal)
            }
            Stmt::PrintFile {
                number,
                values,
                seps,
                suppress_newline,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                // `Print #n, ...;` 之后同一行还会接着写，`,` 分区要从当前列算起。
                let start_column = match self.files.get(&number) {
                    Some(FileHandle::Writer(writer)) => writer.column,
                    _ => 0,
                };
                let text = self.render_print(values, seps, start_column, *line)?;
                let Some(FileHandle::Writer(writer)) = self.files.get_mut(&number) else {
                    return Err(writer_error(&self.files, number, *line));
                };
                writer
                    .write_line(&text, *suppress_newline)
                    .map_err(|error| {
                        RuntimeError::new(format!("写入 #{number} 失败：{error}"), *line)
                            .with_number(io_error_number(&error))
                    })?;
                Ok(Flow::Normal)
            }
            Stmt::WidthFile {
                number,
                width,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let width = self.eval(width, *line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("`Width` 的宽度必须是整数：{message}"), *line)
                })?;
                let Some(FileHandle::Writer(writer)) = self.files.get_mut(&number) else {
                    return Err(writer_error(&self.files, number, *line));
                };
                writer.width = width.max(0) as usize;
                Ok(Flow::Normal)
            }
            Stmt::WriteFile {
                number,
                values,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let mut fields = Vec::with_capacity(values.len());
                for expr in values {
                    let value = self.eval(expr, *line)?;
                    fields.push(
                        write_field(&value).map_err(|message| RuntimeError::new(message, *line))?,
                    );
                }
                let text = format!("{}\r\n", fields.join(","));
                let Some(FileHandle::Writer(writer)) = self.files.get_mut(&number) else {
                    return Err(writer_error(&self.files, number, *line));
                };
                writer.write_raw(&text).map_err(|error| {
                    RuntimeError::new(format!("写入 #{number} 失败：{error}"), *line)
                        .with_number(io_error_number(&error))
                })?;
                Ok(Flow::Normal)
            }
            Stmt::MidAssign {
                target,
                start,
                length,
                value,
                line,
            } => {
                let Some(slot) = self.find_slot(target) else {
                    return Err(RuntimeError::new(format!("未定义的变量 `{target}`"), *line));
                };
                let start = self.eval(start, *line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("`Mid` 的起点必须是整数：{message}"), *line)
                })?;
                let length = match length {
                    Some(expr) => Some(self.eval(expr, *line)?.as_int().map_err(|message| {
                        RuntimeError::new(format!("`Mid` 的长度必须是整数：{message}"), *line)
                    })?),
                    None => None,
                };
                if start < 1 {
                    return Err(RuntimeError::new(
                        format!("`Mid` 的起点 {start} 无效（从 1 开始）"),
                        *line,
                    )
                    .with_number(5));
                }
                let text = self.eval(value, *line)?.to_display();
                let mut chars: Vec<char> = slot.value().to_display().chars().collect();
                let begin = (start - 1) as usize;
                // VB 的 `Mid` 语句只替换已有字符，不会把字符串变长；起点越界时什么都不做。
                if begin >= chars.len() {
                    return Ok(Flow::Normal);
                }
                let available = chars.len() - begin;
                let count = match length {
                    Some(count) => (count.max(0) as usize).min(available),
                    None => available,
                };
                for (offset, ch) in text.chars().take(count).enumerate() {
                    chars[begin + offset] = ch;
                }
                slot.set(Value::Str(chars.into_iter().collect()))
                    .map_err(|message| RuntimeError::new(message, *line))?;
                Ok(Flow::Normal)
            }
            Stmt::RaiseEvent { name, args, line } => {
                let Some(instance) = self.current_instance() else {
                    return Err(RuntimeError::new("`RaiseEvent` 只能用在类的方法里", *line));
                };
                // 实参在「发事件的这一侧」求值，再按值传给订阅者的处理器。
                let mut values = Vec::with_capacity(args.len());
                for expr in args {
                    values.push(self.eval(expr, *line)?);
                }
                let class_name = instance.borrow().key().to_string();
                let handler_name = format!("{class_name}_{name}");
                let key = Rc::as_ptr(&instance) as usize;
                let subscribers = self.events.remove(&key).unwrap_or_default();
                let mut alive = Vec::with_capacity(subscribers.len());
                for weak in subscribers {
                    let Some(sink) = weak.upgrade() else {
                        continue;
                    };
                    let sink_class = sink.borrow().key().to_string();
                    let handler = self
                        .classes
                        .get(&sink_class)
                        .and_then(|def| def.method(&handler_name));
                    if let Some(handler) = handler {
                        let locals = Self::instance_locals(&sink);
                        self.call_procedure_values(handler, values.clone(), locals)?;
                    }
                    alive.push(weak);
                }
                self.events.insert(key, alive);
                Ok(Flow::Normal)
            }
            Stmt::LockFile {
                number,
                unlock,
                start,
                end,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let start = self.eval_position(start, *line)?;
                let end = self.eval_position(end, *line)?;
                let Some(handle) = self.files.get(&number) else {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 没有打开"), *line)
                            .with_number(52),
                    );
                };
                let (file, record_len) = match handle {
                    FileHandle::Reader(reader) => (reader.get_ref(), None),
                    FileHandle::Writer(writer) => (writer.file(), None),
                    FileHandle::Binary(binary) => {
                        (&binary.file, binary.random.then_some(binary.record_len))
                    }
                };
                let (offset, length) = Self::lock_span(start, end, record_len, number, *line)?;
                match filelock::lock_range(file, offset, length, *unlock) {
                    Ok(()) => Ok(Flow::Normal),
                    Err(filelock::LockFailure::Busy) => Err(RuntimeError::new(
                        format!("文件号 #{number} 上这段范围已被锁定"),
                        *line,
                    )
                    .with_number(70)),
                    Err(filelock::LockFailure::Io(error)) => Err(RuntimeError::new(
                        format!(
                            "{} #{number} 失败：{error}",
                            if *unlock { "解锁" } else { "锁定" }
                        ),
                        *line,
                    )
                    .with_number(io_error_number(&error))),
                }
            }
            Stmt::LineInput {
                number,
                target,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let text = {
                    let Some(FileHandle::Reader(reader)) = self.files.get_mut(&number) else {
                        return Err(RuntimeError::new(
                            format!("文件号 #{number} 不是以 Input 打开"),
                            *line,
                        ));
                    };
                    let mut buffer = String::new();
                    reader.read_line(&mut buffer).map_err(|error| {
                        RuntimeError::new(format!("读取 #{number} 失败：{error}"), *line)
                    })?;
                    buffer
                };
                let text = text.trim_end_matches(['\r', '\n']).to_string();
                self.set_var(target, Value::Str(text), *line, None)?;
                Ok(Flow::Normal)
            }
            Stmt::InputFile {
                number,
                targets,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let text = {
                    let Some(FileHandle::Reader(reader)) = self.files.get_mut(&number) else {
                        return Err(RuntimeError::new(
                            format!("文件号 #{number} 不是以 Input 打开"),
                            *line,
                        ));
                    };
                    let mut buffer = String::new();
                    reader.read_line(&mut buffer).map_err(|error| {
                        RuntimeError::new(format!("读取 #{number} 失败：{error}"), *line)
                    })?;
                    buffer
                };
                let fields = split_input_fields(&text);
                for (index, target) in targets.iter().enumerate() {
                    let value = fields
                        .get(index)
                        .map(|field| parse_input_value(field))
                        .unwrap_or(Value::Empty);
                    self.set_var(target, value, *line, None)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::GetFile {
                number,
                position,
                target,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let position = self.eval_position(position, *line)?;
                let slot = self.lvalue_slot(target, false, *line)?.ok_or_else(|| {
                    RuntimeError::new("`Get` 的目标必须是变量、数组元素或记录字段", *line)
                })?;
                let current = slot.value();
                if let Ok(record) = current.as_record() {
                    // UDT：按结构体布局整块读写。
                    let type_name = Self::record_type_name(&record);
                    let size = ffi::record_size(&type_name, &self.types)
                        .map_err(|message| RuntimeError::new(format!("`Get` {message}"), *line))?;
                    let bytes = self.binary_read(number, position, size, *line)?;
                    ffi::decode_record(&type_name, &bytes, &record, &self.types).map_err(
                        |message| {
                            RuntimeError::new(format!("读取 #{number} 失败：{message}"), *line)
                        },
                    )?;
                    return Ok(Flow::Normal);
                }
                // 变长 `String`：VB6 实测没有长度前缀，`Get` 按变量当前长度（字节）读，
                // 当前是空串就一个字节也不读（文件位置不动）。
                if slot.ty() == VbType::Str {
                    let len = match current {
                        Value::Str(text) => text.len(),
                        _ => 0,
                    };
                    if len == 0 {
                        return Ok(Flow::Normal);
                    }
                    let bytes = self.binary_read(number, position, len, *line)?;
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    slot.set(Value::Str(text))
                        .map_err(|message| RuntimeError::new(message, *line))?;
                    return Ok(Flow::Normal);
                }
                // `Variant` 变量：VB6 实测读的是「2 字节 VARTYPE + 内容」的带标记布局，
                // 读回来之后 `VarType` 也变成标记里的类型（`Get` 一个 Integer 之后是 2）。
                if slot.ty() == VbType::Variant {
                    let mut tag = [0u8; 2];
                    let header = self.binary_read(number, position, 2, *line)?;
                    tag.copy_from_slice(&header[..2]);
                    let code = i16::from_le_bytes(tag);
                    let (value, narrow) = self.read_variant(code, number, *line)?;
                    slot.set(value)
                        .map_err(|message| RuntimeError::new(message, *line))?;
                    slot.set_narrow(narrow);
                    return Ok(Flow::Normal);
                }
                let size = ffi::binary_width(slot.ty())
                    .map_err(|message| RuntimeError::new(format!("`Get` {message}"), *line))?;
                let bytes = self.binary_read(number, position, size, *line)?;
                let value = ffi::decode_scalar(slot.ty(), &bytes).map_err(|message| {
                    RuntimeError::new(format!("读取 #{number} 失败：{message}"), *line)
                })?;
                slot.set(value)
                    .map_err(|message| RuntimeError::new(message, *line))?;
                Ok(Flow::Normal)
            }
            Stmt::PutFile {
                number,
                position,
                value,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let position = self.eval_position(position, *line)?;
                let bytes = self.encode_put(value, *line)?;
                self.binary_write(number, position, &bytes, *line)?;
                Ok(Flow::Normal)
            }
            Stmt::SeekFile {
                number,
                position,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let position = self.eval(position, *line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("文件位置必须是整数：{message}"), *line)
                })?;
                self.binary_file_mut(number, *line)?
                    .move_to(position, *line)?;
                Ok(Flow::Normal)
            }
            Stmt::ErrRaise {
                number,
                source,
                description,
                line,
            } => {
                let number = self.eval(number, *line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("错误号必须是整数：{message}"), *line)
                })?;
                let source = match source {
                    Some(expr) => Some(std::sync::Arc::from(
                        self.eval(expr, *line)?.to_display().as_str(),
                    )),
                    None => None,
                };
                let description = match description {
                    Some(expr) => self.eval(expr, *line)?.to_display(),
                    None => builtins::error_text(number),
                };
                Err(RuntimeError::new(description, *line)
                    .with_number(number)
                    .with_source(source))
            }
        }
    }

    fn exec_for(
        &mut self,
        var: &str,
        start: &Expr,
        end: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        let start_value = self.eval(start, line)?;
        let end_value = self.eval(end, line)?;
        let step_expression = match step {
            Some(expr) => Some(self.eval(expr, line)?),
            None => None,
        };
        let step_value = match &step_expression {
            Some(value) => value
                .as_float()
                .map_err(|message| RuntimeError::new(message, line))?,
            None => 1.0,
        };
        if step_value == 0.0 {
            return Err(RuntimeError::new("`For` 的 `Step` 不能为 0", line));
        }

        // 起止与步长都是整数时走整数循环：没有累加误差，边界判断与 VB 完全一致。
        if let (Some(mut current), Some(limit), Some(step)) = (
            exact_int(&start_value),
            exact_int(&end_value),
            step_expression.as_ref().map_or(Some(1), exact_int),
        ) {
            // 计数器单元只在进入循环时解析一次（VB 的 `For` 计数也是按帧偏移直取）。
            let mut counter_slot = self.find_slot(var);
            let counter_type = counter_slot
                .as_ref()
                .map_or(VbType::Variant, |slot| slot.ty());
            loop {
                self.tick(line)?;
                if self.count_stats {
                    self.stats.statements += 1;
                }
                let in_range = if step > 0 {
                    current <= limit
                } else {
                    current >= limit
                };
                if !in_range {
                    break;
                }
                match &counter_slot {
                    Some(slot) => self.store_in(slot, var, Value::Int(current), line, None)?,
                    None => {
                        self.set_var(var, Value::Int(current), line, None)?;
                        counter_slot = self.find_slot(var);
                    }
                }
                match self.exec_block(body)? {
                    Flow::Normal | Flow::ExitDo => {}
                    Flow::ExitFor => return Ok(Flow::Normal),
                    other => return Ok(other),
                }
                // 计数器是 `Variant` / `LongLong` 时，i64 溢出就结束循环，避免回绕成死循环；
                // 声明成 `Integer` / `Long` / `Byte` 时按该类型的范围检查并报错误号 6（见下）。
                let Some(next) = current.checked_add(step) else {
                    break;
                };
                // 声明成整数类型时按该类型的范围检查：VB6 在 `Next` 里溢出报错误号 6。
                if let Some((min, max)) = integer_bounds(counter_type)
                    && (next < min || next > max)
                {
                    return Err(RuntimeError::new("溢出", line).with_number(6));
                }
                current = next;
            }
            // VB 的 `For` 走完时计数器停在「第一个不满足条件的值」（`For i = 1 To 4` 之后 `i` 是 5）。
            if let Some(slot) = &counter_slot {
                self.store_in(slot, var, Value::Int(current), line, None)?;
            }
            return Ok(Flow::Normal);
        }

        let mut current = start_value
            .as_float()
            .map_err(|message| RuntimeError::new(message, line))?;
        let limit = end_value
            .as_float()
            .map_err(|message| RuntimeError::new(message, line))?;
        // 累积误差与步长同阶：按步长的相对量给容差，而不是固定 1e-9。
        let tolerance = step_value.abs() * 1e-9;
        let mut counter_slot = self.find_slot(var);

        loop {
            self.tick(line)?;
            if self.count_stats {
                self.stats.statements += 1;
            }
            let in_range = if step_value > 0.0 {
                current <= limit + tolerance
            } else {
                current >= limit - tolerance
            };
            if !in_range {
                break;
            }
            match &counter_slot {
                Some(slot) => self.store_in(slot, var, number_value(current), line, None)?,
                None => {
                    self.set_var(var, number_value(current), line, None)?;
                    counter_slot = self.find_slot(var);
                }
            }
            match self.exec_block(body)? {
                Flow::Normal | Flow::ExitDo => {}
                Flow::ExitFor => break,
                other => return Ok(other),
            }
            current += step_value;
        }
        // 与整数循环一致：正常结束后计数器停在越界后的那个值。
        if let Some(slot) = &counter_slot {
            self.store_in(slot, var, number_value(current), line, None)?;
        }
        Ok(Flow::Normal)
    }

    fn exec_for_each(
        &mut self,
        var: &str,
        collection: &Expr,
        body: &[Stmt],
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        let value = self.eval(collection, line)?;
        if let Ok(items) = value.as_collection() {
            let snapshot = items.borrow().values_with_types();
            for (item, narrow) in snapshot {
                self.tick(line)?;
                self.set_iteration_var(var, item, narrow, line)?;
                match self.exec_block(body)? {
                    Flow::Normal | Flow::ExitDo => {}
                    Flow::ExitFor => break,
                    other => return Ok(other),
                }
            }
            return Ok(Flow::Normal);
        }
        if let Ok(entries) = value.as_dictionary() {
            // `For Each` 遍历 Scripting.Dictionary 时取「键」，按插入顺序（本机 VB6 实测）；
            // 要遍历值请写 `For Each v In d.Items`。
            let snapshot = entries.borrow().keys();
            for item in snapshot {
                self.tick(line)?;
                self.set_var(var, item, line, None)?;
                match self.exec_block(body)? {
                    Flow::Normal | Flow::ExitDo => {}
                    Flow::ExitFor => break,
                    other => return Ok(other),
                }
            }
            return Ok(Flow::Normal);
        }
        if let Value::ComObject(object) = &value {
            // COM 集合：走 `_NewEnum`（`IEnumVARIANT`）取元素快照，再按普通集合遍历。
            let mut enumerator = object.items().map_err(|failure| {
                RuntimeError::new(failure.message, line).with_number(failure.number)
            })?;
            let mut snapshot = Vec::new();
            while let Some(item) = enumerator.next().map_err(|failure| {
                RuntimeError::new(failure.message, line).with_number(failure.number)
            })? {
                snapshot.push(item);
            }
            for item in snapshot {
                self.tick(line)?;
                self.set_iteration_var(var, item, None, line)?;
                match self.exec_block(body)? {
                    Flow::Normal | Flow::ExitDo => {}
                    Flow::ExitFor => break,
                    other => return Ok(other),
                }
            }
            return Ok(Flow::Normal);
        }
        let array = value.as_array().map_err(|message| {
            RuntimeError::new(
                format!("`For Each` 需要数组、Collection、Dictionary 或 COM 集合：{message}"),
                line,
            )
        })?;
        // 先取快照，避免循环体里改写数组时长时间持有借用。
        let (items, element) = {
            let array = array.borrow();
            if array.dims().is_empty() {
                return Err(RuntimeError::new("`For Each` 的数组尚未初始化", line));
            }
            if array.dims().len() > 1 {
                return Err(RuntimeError::new("`For Each` 只支持一维数组", line));
            }
            // VB6 实测：循环变量取**元素声明类型**（Integer 数组 → 2、Byte 数组 → 17、String → 8）；
            // `Variant` 数组取每个元素记下的内容类型（`Array(CByte(1), 2)` 的第一个是 17）。
            let element = array.element_type();
            let element = (!matches!(element, VbType::Variant | VbType::Any)).then_some(element);
            (array.values_with_types(), element)
        };

        // 循环变量的槽只查一次；`Variant` 变量每轮把元素类型记进去（见 `Slot::narrow_type`）。
        if self.find_slot(var).is_none() {
            self.set_var(var, Value::Empty, line, None)?;
        }
        let loop_slot = self.find_slot(var);
        for (item, narrowed) in items {
            self.tick(line)?;
            match &loop_slot {
                Some(slot) => {
                    self.store_in(slot, var, item, line, None)?;
                    if slot.ty() == VbType::Variant {
                        slot.set_narrow(element.or(narrowed));
                    }
                }
                None => self.set_var(var, item, line, None)?,
            }
            match self.exec_block(body)? {
                Flow::Normal | Flow::ExitDo => {}
                Flow::ExitFor => break,
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    /// `For Each` 的循环变量：把元素（连同内容类型）写进去。
    fn set_iteration_var(
        &mut self,
        var: &str,
        item: Value,
        narrow: Option<VbType>,
        line: usize,
    ) -> Result<(), RuntimeError> {
        self.set_var(var, item, line, None)?;
        if let Some(slot) = self.find_slot(var)
            && slot.ty() == VbType::Variant
        {
            slot.set_narrow(narrow);
        }
        Ok(())
    }

    /// `WithEvents x As Object` 的变量 `Set` 到 COM 对象时挂接它的事件。
    ///
    /// VB6 的处理过程名是 `变量_事件`，所以这里只登记脚本里真的写了处理过程的事件名。
    fn connect_com_events(
        &mut self,
        object: &com::ComObject,
        variable: &str,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let prefix = format!("{variable}_");
        let mut handlers = Vec::new();
        for name in self.procedures.keys() {
            if let Some(event) = name.strip_prefix(&prefix)
                && !event.is_empty()
            {
                handlers.push(event.to_string());
            }
        }
        if handlers.is_empty() {
            // 没写任何处理过程：不需要挂接，也不算错误。
            return Ok(());
        }
        match object.connect_events(variable, &handlers) {
            Ok(events) => {
                self.com_events.push(ComEventOwner {
                    instance: None,
                    field: variable.to_string(),
                    events,
                });
                Ok(())
            }
            Err(failure) => {
                Err(RuntimeError::new(failure.message, line).with_number(failure.number))
            }
        }
    }

    /// `Set 名字 = COM 对象`：按「模块级 `WithEvents` 变量」或「当前类的 `WithEvents` 字段」挂接事件。
    fn connect_events_for_name(
        &mut self,
        name: &str,
        value: &Value,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let Value::ComObject(object) = value else {
            return Ok(());
        };
        // 类字段：方法体的局部变量表里就是实例字段，所以要查当前类的声明。
        if let Some(instance) = self.current_instance() {
            let class_name = instance.borrow().key().to_string();
            if let Some(def) = self.classes.get(&class_name).copied() {
                let with_events = def
                    .fields
                    .iter()
                    .any(|field| field.name == name && field.with_events);
                if with_events {
                    return self.connect_field_events(object, def, name, &instance, line);
                }
            }
        }
        if self.com_event_vars.contains(name) {
            return self.connect_com_events(object, name, line);
        }
        Ok(())
    }

    /// 类字段上的 `WithEvents`：处理过程是同类的 `字段_事件`。
    fn connect_field_events(
        &mut self,
        object: &com::ComObject,
        def: &'p ClassDef,
        name: &str,
        instance: &Rc<RefCell<VbInstance>>,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let prefix = format!("{name}_");
        let handlers: Vec<String> = def
            .methods
            .iter()
            .filter_map(|method| method.name.strip_prefix(prefix.as_str()))
            .filter(|event| !event.is_empty())
            .map(str::to_string)
            .collect();
        if handlers.is_empty() {
            return Ok(());
        }
        match object.connect_events(name, &handlers) {
            Ok(events) => {
                self.com_events.push(ComEventOwner {
                    instance: Some(Rc::downgrade(instance)),
                    field: name.to_string(),
                    events,
                });
                Ok(())
            }
            Err(failure) => {
                Err(RuntimeError::new(failure.message, line).with_number(failure.number))
            }
        }
    }

    /// 分派已排队的 COM 事件（在语句边界调用，避免在 COM 回调里重入解释器）。
    fn drain_com_events(&mut self) -> Result<(), RuntimeError> {
        if self.com_events.is_empty() {
            return Ok(());
        }
        let mut pending = Vec::new();
        for owner in &self.com_events {
            for event in owner.events.drain() {
                pending.push((owner.instance.clone(), owner.field.clone(), event));
            }
        }
        for (instance, field, event) in pending {
            let handler_name = format!("{field}_{}", event.event);
            let mut values = Vec::with_capacity(event.args.len());
            for arg in event.args {
                values.push(match arg {
                    com::EventArg::Empty => Value::Empty,
                    com::EventArg::Null => Value::Null,
                    com::EventArg::Int(number) => Value::Int(number),
                    com::EventArg::Float(number) => Value::Float(number),
                    com::EventArg::Bool(flag) => Value::Bool(flag),
                    com::EventArg::Str(text) => Value::Str(text),
                    com::EventArg::Object(pointer) => {
                        match com::ComObject::from_raw_owned(pointer) {
                            Some(object) => Value::ComObject(object),
                            None => Value::Empty,
                        }
                    }
                });
            }
            match instance {
                // 类字段上的 `WithEvents`：在类里找 `字段_事件` 方法，用实例的局部变量调用。
                Some(weak_owner) => {
                    let Some(owner) = weak_owner.upgrade() else {
                        continue;
                    };
                    let class_name = owner.borrow().key().to_string();
                    let Some(handler) = self
                        .classes
                        .get(&class_name)
                        .and_then(|def| def.method(&handler_name))
                    else {
                        continue;
                    };
                    let locals = Self::instance_locals(&owner);
                    actions_pad_values(&mut values, handler.params.len());
                    self.call_procedure_values(handler, values, locals)?;
                }
                None => {
                    let Some(procedure) = self.procedures.get(&handler_name).copied() else {
                        continue;
                    };
                    actions_pad_values(&mut values, procedure.params.len());
                    self.call_procedure_values(procedure, values, FastMap::default())?;
                }
            }
        }
        Ok(())
    }

    // ---------- 文件 ----------
    fn file_number(&mut self, expr: &Expr, line: usize) -> Result<i32, RuntimeError> {
        let value = self
            .eval(expr, line)?
            .as_int()
            .map_err(|message| RuntimeError::new(format!("文件号必须是整数：{message}"), line))?;
        i32::try_from(value)
            .map_err(|_| RuntimeError::new(format!("文件号 {value} 超出范围"), line))
    }

    /// 下一个空闲文件号（`FreeFile` 与省略 `As #n` 的 `Open` 共用）。
    fn free_file_number(&self) -> i32 {
        let mut number = 1i32;
        while self.files.contains_key(&number) {
            number += 1;
        }
        number
    }

    /// `Get`/`Put` 的位置参数（1 基，和 VB 一致；省略时用游标）。
    fn eval_position(
        &mut self,
        position: &Option<Expr>,
        line: usize,
    ) -> Result<Option<i64>, RuntimeError> {
        match position {
            Some(expr) => {
                let value = self.eval(expr, line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("文件位置必须是整数：{message}"), line)
                })?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// `Put #` 的写入值：UDT 按结构体布局，标量按声明类型（变量）或动态类型（表达式）写，
    /// 变长 `String` 直接写字符字节（没有长度前缀，见 VB6 实测）。
    fn encode_put(&mut self, expr: &Expr, line: usize) -> Result<Vec<u8>, RuntimeError> {
        let value = self.eval(expr, line)?;
        if let Ok(record) = value.as_record() {
            let type_name = Self::record_type_name(&record);
            return ffi::encode_record(&type_name, &record.borrow(), &self.types)
                .map_err(|message| RuntimeError::new(format!("`Put` {message}"), line));
        }
        let declared = match expr {
            Expr::Var(name) => self.find_slot(name).map(|slot| slot.ty()),
            // `Put #1, , CByte(65)`：转换函数的结果宽度由函数决定，不能按动态类型猜
            // （`CByte` 的值在解释器里是 `Int`，猜成 Integer 会写 2 字节）。
            Expr::Call { name, .. } => conversion_type(name),
            _ => None,
        };
        let ty = match declared {
            Some(VbType::Variant | VbType::Object | VbType::Any) | None => {
                Self::dynamic_type(&value)
            }
            Some(ty) => ty,
        };
        // `Variant` 变量：VB6 实测写的是「2 字节 VARTYPE + 内容」的带标记布局
        // （`v = 5` 写 4 字节、`v = "AB"` 写 6 字节、`v = True` 写 4 字节）。
        if matches!(declared, Some(VbType::Variant)) {
            let narrow = self
                .static_type(expr)
                .filter(|ty| !matches!(ty, VbType::Variant | VbType::Any | VbType::Object));
            return Self::encode_variant(narrow.unwrap_or(ty), &value);
        }
        // 变长 `String`：VB6 实测直接写字符字节、没有长度前缀（VB6 用系统代码页，QIDE 统一 UTF-8）。
        if ty == VbType::Str {
            return Ok(value.to_display().into_bytes());
        }
        ffi::encode_scalar(ty, &value)
            .map_err(|message| RuntimeError::new(format!("`Put` {message}"), line))
    }

    /// `Put #` 写 `Variant` 时的带标记布局：2 字节 VARTYPE + 内容（本机 VB6 逐字节比对过）。
    ///
    /// 不支持的类型（对象、数组…）报错误号 458，与 VB6 一致。
    fn encode_variant(ty: VbType, value: &Value) -> Result<Vec<u8>, RuntimeError> {
        let number = builtins::vartype_of(ty);
        let mut bytes = vec![0u8; 2];
        let code = match (&value, ty) {
            (Value::Empty, _) => 0,
            (Value::Null, _) => 1,
            _ => number,
        };
        bytes[..2].copy_from_slice(&(code as i16).to_le_bytes());
        match code {
            0 | 1 => {}
            // `Boolean` 在 VB6 的 Variant 里是 2 字节（`0B 00 FF FF`），不是 Win32 的 4 字节 BOOL。
            11 => {
                let flag: i16 = match value {
                    Value::Bool(true) => -1,
                    _ => 0,
                };
                bytes.extend_from_slice(&flag.to_le_bytes());
            }
            // `String`：2 字节长度前缀 + 字节（QIDE 用 UTF-8，长度按字节数）。
            8 => {
                let text = value.to_display();
                let raw = text.as_bytes();
                let len = u16::try_from(raw.len()).map_err(|_| {
                    RuntimeError::new("`Put` 的字符串太长（超过 65535 字节）", 0).with_number(458)
                })?;
                bytes.extend_from_slice(&len.to_le_bytes());
                bytes.extend_from_slice(raw);
            }
            _ => bytes.extend_from_slice(&ffi::encode_scalar(ty, value).map_err(|message| {
                RuntimeError::new(format!("`Put` {message}"), 0).with_number(458)
            })?),
        }
        Ok(bytes)
    }

    /// 读回 `Put #` 写下的带标记 `Variant`：按 2 字节 VARTYPE 决定后面跟多少字节。
    fn read_variant(
        &mut self,
        code: i16,
        number: i32,
        line: usize,
    ) -> Result<(Value, Option<VbType>), RuntimeError> {
        let (ty, narrow) = match code {
            0 => return Ok((Value::Empty, None)),
            1 => return Ok((Value::Null, None)),
            2 => (VbType::Integer, VbType::Integer),
            3 => (VbType::Long, VbType::Long),
            4 => (VbType::Single, VbType::Single),
            5 => (VbType::Double, VbType::Double),
            6 => (VbType::Currency, VbType::Currency),
            7 => (VbType::Date, VbType::Date),
            17 => (VbType::Byte, VbType::Byte),
            11 => {
                let raw = self.binary_read(number, None, 2, line)?;
                let flag = i16::from_le_bytes([raw[0], raw[1]]) != 0;
                return Ok((Value::Bool(flag), Some(VbType::Boolean)));
            }
            8 => {
                let header = self.binary_read(number, None, 2, line)?;
                let len = u16::from_le_bytes([header[0], header[1]]) as usize;
                let raw = self.binary_read(number, None, len, line)?;
                let text = String::from_utf8_lossy(&raw).into_owned();
                return Ok((Value::Str(text), Some(VbType::Str)));
            }
            other => {
                return Err(RuntimeError::new(
                    format!("`Get` 读到不支持的 Variant 类型标记 {other}"),
                    line,
                )
                .with_number(458));
            }
        };
        let size = ffi::binary_width(ty)
            .map_err(|message| RuntimeError::new(format!("`Get` {message}"), line))?;
        let bytes = self.binary_read(number, None, size, line)?;
        let value = ffi::decode_scalar(ty, &bytes).map_err(|message| {
            RuntimeError::new(format!("读取 #{number} 失败：{message}"), line)
        })?;
        Ok((value, Some(narrow)))
    }

    /// `Lock #` 的范围换算：`For Random` 下位置是记录号，其它模式是 1 基字节偏移。
    fn lock_span(
        start: Option<i64>,
        end: Option<i64>,
        record_len: Option<usize>,
        number: i32,
        line: usize,
    ) -> Result<(u64, u64), RuntimeError> {
        let unit = record_len.unwrap_or(1) as u64;
        let offset = match start {
            Some(start) => {
                if start < 1 {
                    return Err(RuntimeError::new(
                        format!("`#{number}` 的起始位置 {start} 无效（从 1 开始）"),
                        line,
                    )
                    .with_number(63));
                }
                (start as u64 - 1) * unit
            }
            None => 0,
        };
        let length = match end {
            Some(end) => {
                let first = start.unwrap_or(1);
                if end < first {
                    return Err(RuntimeError::new(
                        format!("`#{number}` 的结束位置 {end} 小于起始位置 {first}"),
                        line,
                    )
                    .with_number(63));
                }
                ((end - first) as u64 + 1) * unit
            }
            // 只给起始位置：一直锁到文件末尾。
            None => u64::MAX,
        };
        Ok((offset, length))
    }

    /// 记录的类型名（小写，和 `types` 表的键一致）。
    fn record_type_name(record: &Rc<RefCell<VbRecord>>) -> String {
        record.borrow().type_name().to_ascii_lowercase()
    }

    /// 值的动态类型：小数持 `Double`、布尔按 `Boolean`、字符串持 `String`，其余按 `Long`。
    fn dynamic_type(value: &Value) -> VbType {
        match value {
            Value::Float(_) => VbType::Double,
            Value::Currency(_) => VbType::Currency,
            Value::Date(_) => VbType::Date,
            Value::Bool(_) => VbType::Boolean,
            Value::Str(_) => VbType::Str,
            Value::Int(_) => VbType::Long,
            _ => VbType::Variant,
        }
    }

    fn binary_file_mut(
        &mut self,
        number: i32,
        line: usize,
    ) -> Result<&mut BinaryFile, RuntimeError> {
        match self.files.get_mut(&number) {
            Some(FileHandle::Binary(file)) => Ok(file),
            Some(_) => Err(RuntimeError::new(
                format!("文件号 #{number} 不是以 Binary 或 Random 打开"),
                line,
            )
            .with_number(54)),
            None => {
                Err(RuntimeError::new(format!("文件号 #{number} 没有打开"), line).with_number(52))
            }
        }
    }

    fn binary_read(
        &mut self,
        number: i32,
        position: Option<i64>,
        size: usize,
        line: usize,
    ) -> Result<Vec<u8>, RuntimeError> {
        let file = self.binary_file_mut(number, line)?;
        if let Some(position) = position {
            file.move_to(position, line)?;
        }
        let advance = file.advance(size, line)?;
        file.file.seek(SeekFrom::Start(file.pos)).map_err(|error| {
            RuntimeError::new(format!("定位 #{number} 失败：{error}"), line).with_number(57)
        })?;
        let mut buffer = vec![0u8; size];
        file.file.read_exact(&mut buffer).map_err(|error| {
            RuntimeError::new(format!("读取 #{number} 失败：{error}"), line).with_number(62)
        })?;
        file.pos += advance as u64;
        Ok(buffer)
    }

    fn binary_write(
        &mut self,
        number: i32,
        position: Option<i64>,
        bytes: &[u8],
        line: usize,
    ) -> Result<(), RuntimeError> {
        let file = self.binary_file_mut(number, line)?;
        if let Some(position) = position {
            file.move_to(position, line)?;
        }
        let advance = file.advance(bytes.len(), line)?;
        file.file.seek(SeekFrom::Start(file.pos)).map_err(|error| {
            RuntimeError::new(format!("定位 #{number} 失败：{error}"), line).with_number(57)
        })?;
        file.file.write_all(bytes).map_err(|error| {
            RuntimeError::new(format!("写入 #{number} 失败：{error}"), line).with_number(57)
        })?;
        if advance > bytes.len() {
            let padding = vec![0u8; advance - bytes.len()];
            file.file.write_all(&padding).map_err(|error| {
                RuntimeError::new(format!("写入 #{number} 失败：{error}"), line).with_number(57)
            })?;
        }
        file.pos += advance as u64;
        Ok(())
    }

    /// `EOF(n)` / `LOF(n)` / `FreeFile()`。
    fn file_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if name == "freefile" {
            return Ok(Value::Int(i64::from(self.free_file_number())));
        }
        let Some(expr) = args.first() else {
            return Err(RuntimeError::new(format!("`{name}` 缺少第 1 个参数"), line));
        };
        let number = self.file_number(expr, line)?;
        match name {
            "eof" => {
                let Some(handle) = self.files.get_mut(&number) else {
                    return Err(RuntimeError::new(
                        format!("文件号 #{number} 没有打开"),
                        line,
                    ));
                };
                match handle {
                    FileHandle::Reader(reader) => {
                        let buffer = reader.fill_buf().map_err(|error| {
                            RuntimeError::new(format!("读取 #{number} 失败：{error}"), line)
                        })?;
                        Ok(Value::Bool(buffer.is_empty()))
                    }
                    FileHandle::Binary(file) => {
                        let size = file.file.metadata().map(|meta| meta.len()).unwrap_or(0);
                        Ok(Value::Bool(file.pos >= size))
                    }
                    FileHandle::Writer(_) => Ok(Value::Bool(true)),
                }
            }
            "seek" | "loc" => {
                let Some(handle) = self.files.get_mut(&number) else {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 没有打开"), line)
                            .with_number(52),
                    );
                };
                match handle {
                    FileHandle::Binary(file) => {
                        let value = if name == "loc" {
                            file.location()
                        } else {
                            file.position()
                        };
                        Ok(Value::Int(value))
                    }
                    FileHandle::Reader(reader) => {
                        // BufReader 的逻辑位置 = 底层位置 - 尚未消费的缓冲长度。
                        let raw = reader.get_ref().stream_position().unwrap_or(0);
                        let buffered = reader.buffer().len() as u64;
                        Ok(Value::Int((raw.saturating_sub(buffered)) as i64 + 1))
                    }
                    FileHandle::Writer(writer) => {
                        let raw = writer.file().stream_position().unwrap_or(0);
                        Ok(Value::Int(raw as i64 + 1))
                    }
                }
            }
            _ => {
                let Some(handle) = self.files.get(&number) else {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 没有打开"), line)
                            .with_number(52),
                    );
                };
                let file = match handle {
                    FileHandle::Reader(reader) => reader.get_ref(),
                    FileHandle::Writer(writer) => writer.file(),
                    FileHandle::Binary(binary) => &binary.file,
                };
                let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
                Ok(Value::Int(size as i64))
            }
        }
    }

    /// 求维度表达式并换算成 `(下界, 元素数)`。
    fn array_dims(
        &mut self,
        bounds: &[(Expr, Expr)],
        line: usize,
    ) -> Result<Vec<(i32, usize)>, RuntimeError> {
        let mut dims = Vec::with_capacity(bounds.len());
        let mut total = 1usize;
        for (lower, upper) in bounds {
            let lower_value = self.eval(lower, line)?.as_int().map_err(|message| {
                RuntimeError::new(format!("数组下界必须是整数：{message}"), line)
            })?;
            let upper_value = self.eval(upper, line)?.as_int().map_err(|message| {
                RuntimeError::new(format!("数组上界必须是整数：{message}"), line)
            })?;
            if lower_value > upper_value {
                return Err(RuntimeError::new(
                    format!("数组下界 {lower_value} 不能大于上界 {upper_value}"),
                    line,
                ));
            }
            let lower_value = i32::try_from(lower_value)
                .map_err(|_| RuntimeError::new(format!("数组下界 {lower_value} 超出范围"), line))?;
            // 上界可以接近 `i64::MAX`，用 `i128` 算元素数，避免相减时溢出。
            let count = i128::from(upper_value) - i128::from(lower_value) + 1;
            let count = usize::try_from(count).unwrap_or(usize::MAX);
            total = total.saturating_mul(count);
            if total > MAX_ARRAY_ELEMENTS {
                return Err(RuntimeError::new(
                    format!("数组元素超过上限（{MAX_ARRAY_ELEMENTS} 个）"),
                    line,
                ));
            }
            dims.push((lower_value, count));
        }
        Ok(dims)
    }

    /// 求一组下标表达式，统一转成 `i32`。
    /// 求值一组下标。常见 1–2 维走栈上缓冲，不再每次访问数组都分配 `Vec`。
    fn eval_indices(&mut self, exprs: &[Expr], line: usize) -> Result<IndexBuf, RuntimeError> {
        if exprs.len() <= INLINE_DIMS {
            let mut indices = IndexBuf {
                inline: [0; INLINE_DIMS],
                len: exprs.len(),
                heap: Vec::new(),
            };
            for (position, expr) in exprs.iter().enumerate() {
                indices.inline[position] = self.eval_index(expr, line)?;
            }
            Ok(indices)
        } else {
            let mut heap = Vec::with_capacity(exprs.len());
            for expr in exprs {
                heap.push(self.eval_index(expr, line)?);
            }
            Ok(IndexBuf {
                inline: [0; INLINE_DIMS],
                len: exprs.len(),
                heap,
            })
        }
    }

    fn eval_index(&mut self, expr: &Expr, line: usize) -> Result<i32, RuntimeError> {
        let value = self.eval(expr, line)?.as_int().map_err(|message| {
            RuntimeError::new(format!("数组下标必须是整数：{message}"), line).with_number(9)
        })?;
        i32::try_from(value).map_err(|_| {
            RuntimeError::new(format!("数组下标 {value} 超出范围"), line).with_number(9)
        })
    }

    // ---------- 类（`Class ... End Class`）----------

    /// `Dim` 条目的初始值：`New` → 对象，`As 记录` → 新建记录，`As 类` → 空对象，数组 → 分配。
    fn declared_value(
        &mut self,
        item: &Declared,
        line: usize,
    ) -> Result<(Value, VbType), RuntimeError> {
        // `Dim arr As Integer() = {1, 2, 3}`：声明时就带上初值。
        if let Some(initializer) = &item.initializer {
            let value = self.eval(initializer, line)?;
            return Ok((value, VbType::Variant));
        }
        if let Some(kind) = &item.new_object {
            let value = match kind {
                NewObject::Collection => Value::collection(),
                NewObject::Dictionary => Value::dictionary(),
                NewObject::Class(name) => self.instantiate(name, line)?,
            };
            return Ok((value, VbType::Object));
        }
        if let Some(bounds) = &item.bounds {
            let dims = self.array_dims(bounds, line)?;
            let dynamic = bounds.is_empty();
            return Ok(match &item.record {
                Some(type_name) => {
                    let template = self.new_record(type_name, line)?;
                    let value = Value::array_records(dims, dynamic, template.type_name(), template);
                    (value, VbType::Variant)
                }
                // 数组的元素类型存在数组里，槽本身持 Variant 保存。
                None => (Value::array_new(dims, item.ty, dynamic), VbType::Variant),
            });
        }
        if let Some(type_name) = &item.record {
            if self.is_class(type_name) {
                // `Dim p As Class1`：`Set p = New Class1` 之前是空对象。
                return Ok((Value::Empty, VbType::Object));
            }
            return Ok((self.new_record(type_name, line)?, VbType::Variant));
        }
        Ok((default_value(item.ty), item.ty))
    }

    fn is_class(&self, name: &str) -> bool {
        self.classes.contains_key(&name.to_ascii_lowercase())
    }

    /// 当前正在执行的方法所属的类（不在方法里时返回 `None`）。
    fn current_class(&self) -> Option<&'p ClassDef> {
        let instance = self.current_instance()?;
        let instance = instance.borrow();
        self.classes.get(instance.key()).copied()
    }

    /// 当前方法里的 `Me` 实例。
    fn current_instance(&self) -> Option<Rc<RefCell<VbInstance>>> {
        let frame = self.frames.last()?;
        match frame.locals.get("me").map(Slot::value) {
            Some(Value::Class(instance)) => Some(instance.0),
            _ => None,
        }
    }

    /// 裸调用同类成员时沿用的局部变量（实例变量 + `Me`）。
    fn current_instance_locals(&self) -> FastMap<String, Slot> {
        match self.current_instance() {
            Some(instance) => Self::instance_locals(&instance),
            None => FastMap::default(),
        }
    }

    /// 执行 `Class_Terminate`：把实例变量注入帧，再跑成员过程（错误只能吞掉）。
    fn terminate_instance(&mut self, instance: &VbInstance) {
        let Some(def) = self.classes.get(instance.key()).copied() else {
            return;
        };
        let Some(terminate) = def.method("class_terminate") else {
            return;
        };
        let mut locals = FastMap::with_capacity_and_hasher(
            instance.fields().len(),
            std::hash::BuildHasherDefault::default(),
        );
        for field in instance.fields() {
            locals.insert(
                field.name.clone(),
                Slot::from_cell(field.cell.clone(), field.ty),
            );
        }
        let _ = self.call_user_with_locals(terminate, &[], locals, terminate.line);
    }

    /// `New 类名`：按类定义建实例（实例变量取默认值），再调用 `Class_Initialize`。
    fn instantiate(&mut self, class_name: &str, line: usize) -> Result<Value, RuntimeError> {
        let key = class_name.to_ascii_lowercase();
        let Some(def) = self.classes.get(&key).copied() else {
            return Err(
                RuntimeError::new(format!("未定义的类 `{class_name}`"), line).with_number(429),
            );
        };
        let instance = Rc::new(RefCell::new(VbInstance::new(def.display.clone())));
        // 字段声明先克隆出来，避免在 `self` 可变借用期间还持有类定义的引用。
        let fields = def.fields.clone();
        for item in &fields {
            let (value, ty) = self.declared_value(item, line)?;
            instance
                .borrow_mut()
                .set_field(item.name.clone(), ty, value);
        }
        if let Some(init) = def.method("class_initialize") {
            let locals = Self::instance_locals(&instance);
            self.call_user_with_locals(init, &[], locals, line)?;
        }
        // `WithEvents` 字段若是对象，立刻订阅它的事件。
        for item in &fields {
            if !item.with_events {
                continue;
            }
            let source = instance
                .borrow()
                .field(&item.name)
                .map(|field| field.cell.borrow().clone());
            if let Some(Value::Class(source)) = source {
                Self::register_event_sink(&mut self.events, &source.0, &instance);
            }
        }
        Ok(Value::Class(ClassRef(instance)))
    }

    /// 把 `sink` 登记为 `source` 的事件订阅者（弱引用，不延长生命周期）。
    fn register_event_sink(
        events: &mut HashMap<usize, Vec<Weak<RefCell<VbInstance>>>>,
        source: &Rc<RefCell<VbInstance>>,
        sink: &Rc<RefCell<VbInstance>>,
    ) {
        let key = Rc::as_ptr(source) as usize;
        let entry = events.entry(key).or_default();
        let sink_weak = Rc::downgrade(sink);
        let already =
            entry
                .iter()
                .any(|existing| match (existing.upgrade(), sink_weak.upgrade()) {
                    (Some(a), Some(b)) => Rc::ptr_eq(&a, &b),
                    _ => false,
                });
        if !already {
            entry.push(sink_weak);
        }
    }

    /// 类方法执行时的起始局部变量：实例变量（共享存储）+ `Me`。
    fn instance_locals(instance: &Rc<RefCell<VbInstance>>) -> FastMap<String, Slot> {
        let borrowed = instance.borrow();
        let fields = borrowed.fields();
        // 预分配容量，避免字段较多时反复扩容。
        let mut locals = FastMap::with_capacity_and_hasher(
            fields.len() + 1,
            std::hash::BuildHasherDefault::default(),
        );
        for field in fields {
            locals.insert(
                field.name.clone(),
                Slot::from_cell_with_narrow(field.cell.clone(), field.ty, field.narrow.clone()),
            );
        }
        drop(borrowed);
        locals.insert(
            "me".into(),
            Slot::new(Value::Class(ClassRef(instance.clone())), VbType::Object),
        );
        locals
    }

    /// `obj.Method(args)`：带上实例变量执行成员过程。
    fn call_class_method(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let def = {
            let instance = instance.borrow();
            self.classes.get(instance.key()).copied()
        };
        let Some(def) = def else {
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(format!("未定义的类 `{display}`"), line));
        };
        let method = def.method(name);
        let implemented = method.is_none();
        let Some(method) = method.or_else(|| def.implemented_method(name)) else {
            let display = instance.borrow().type_name().to_string();
            return Err(
                RuntimeError::new(format!("类 `{display}` 没有成员 `{name}`"), line)
                    .with_number(438),
            );
        };
        if !implemented {
            self.ensure_class_member_access(instance, name, method.is_private, line)?;
        }
        let locals = Self::instance_locals(instance);
        self.call_user_with_locals(method, args, locals, line)
    }

    fn ensure_class_member_access(
        &self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        is_private: bool,
        line: usize,
    ) -> Result<(), RuntimeError> {
        if !is_private {
            return Ok(());
        }
        let class_name = instance.borrow().key().to_string();
        if self
            .current_class()
            .is_some_and(|current| current.name == class_name)
        {
            return Ok(());
        }
        Err(RuntimeError::new(
            format!("类成员 `{name}` 是 `Private`，只能在所属类中访问"),
            line,
        )
        .with_number(438))
    }

    /// `obj.成员`：无参成员过程优先当属性读，其次读实例变量。
    fn read_instance_member(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let def = {
            let instance = instance.borrow();
            self.classes.get(instance.key()).copied()
        };
        if let Some(def) = def
            && let Some(method) = def.method(name).or_else(|| def.implemented_method(name))
        {
            if def.method(name).is_some() {
                self.ensure_class_member_access(instance, name, method.is_private, line)?;
            }
            if method.params.is_empty() {
                let locals = Self::instance_locals(instance);
                return self.call_user_with_locals(method, &[], locals, line);
            }
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(
                format!(
                    "类 `{display}` 的成员 `{name}` 需要 {} 个参数（写成 `.{name}(...)`）",
                    method.params.len()
                ),
                line,
            )
            .with_number(438));
        }
        if let Some(field) = instance.borrow().field(name) {
            let is_private = def.is_some_and(|def| def.field_is_private(name));
            self.ensure_class_member_access(instance, name, is_private, line)?;
            // `Variant` 字段的内容类型也一并交给 `result_narrow`（`VarType(obj.Field)` 用）。
            if field.ty == VbType::Variant {
                self.result_narrow = field.narrow.get();
            }
            return Ok(field.cell.borrow().clone());
        }
        let display = instance.borrow().type_name().to_string();
        Err(RuntimeError::new(format!("类 `{display}` 没有成员 `{name}`"), line).with_number(438))
    }

    /// `obj.成员 = 值` / `Set obj.成员 = 对象`：有对应 Property 就走它，否则直接写实例变量。
    fn assign_instance_member(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        path: &[String],
        value: &Expr,
        set_op: bool,
        line: usize,
    ) -> Result<(), RuntimeError> {
        if path.len() > 1 {
            // `obj.Inner.X = 值`：先把第一层取出来，再交给记录赋值。
            let inner = self.read_instance_member(instance, &path[0], line)?;
            if !inner.is_record() {
                return Err(RuntimeError::new(
                    format!("`{}` 不是记录类型，不能继续访问 `{}`", path[0], path[1]),
                    line,
                ));
            }
            let value = self.eval(value, line)?;
            return self.assign_member(inner, &path[1..], value, None, line);
        }
        let name = path
            .first()
            .ok_or_else(|| RuntimeError::new("缺少成员名", line))?;
        let def = {
            let instance = instance.borrow();
            self.classes.get(instance.key()).copied()
        };
        if let Some(def) = def {
            let setter = if set_op {
                def.property_set(name)
                    .or_else(|| def.implemented_property_set(name))
            } else {
                def.property_let(name)
                    .or_else(|| def.implemented_property_let(name))
            };
            if let Some(setter) = setter {
                let direct = if set_op {
                    def.property_set(name).is_some()
                } else {
                    def.property_let(name).is_some()
                };
                if direct {
                    self.ensure_class_member_access(instance, name, setter.is_private, line)?;
                }
                let locals = Self::instance_locals(instance);
                return self
                    .call_user_with_locals(setter, std::slice::from_ref(value), locals, line)
                    .map(|_| ());
            }
        }
        if let Some(field) = instance.borrow().field(name) {
            let is_private = def.is_some_and(|def| def.field_is_private(name));
            self.ensure_class_member_access(instance, name, is_private, line)?;
            // `Variant` 字段要记下右侧表达式的静态类型（`c.P = i` 之后 `VarType(c.P)` 是 2）。
            let narrow = self.narrow_of(Some(value));
            let value = self
                .eval(value, line)?
                .coerce(field.ty)
                .map_err(|message| RuntimeError::from_coerce(message, line))?;
            let with_events = def
                .and_then(|def| {
                    def.fields
                        .iter()
                        .find(|item| item.name == *name)
                        .map(|item| item.with_events)
                })
                .unwrap_or(false);
            if with_events && let Value::Class(source) = &value {
                Self::register_event_sink(&mut self.events, &source.0, instance);
            }
            if with_events
                && let Value::ComObject(object) = &value
                && let Some(def) = def
            {
                // 类字段上的 `WithEvents`：处理过程是同类的 `字段_事件`。
                self.connect_field_events(object, def, name, instance, line)?;
            }
            *field.cell.borrow_mut() = value;
            field.narrow.set(if field.ty == VbType::Variant {
                narrow
            } else {
                None
            });
            return Ok(());
        }
        if let Some(def) = def
            && let Some(method) = def.method(name).or_else(|| def.implemented_method(name))
        {
            if def.method(name).is_some() {
                self.ensure_class_member_access(instance, name, method.is_private, line)?;
            }
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(
                format!("类 `{display}` 的 `{name}` 是只读成员（缺少 `Property Let`）"),
                line,
            )
            .with_number(438));
        }
        let display = instance.borrow().type_name().to_string();
        Err(RuntimeError::new(format!("类 `{display}` 没有成员 `{name}`"), line).with_number(438))
    }

    /// `obj.Prop(下标) = 值` / `Set obj.Prop(下标) = 对象`：交给带参数的 `Property Let/Set`。
    fn assign_instance_index(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        indices: &[Expr],
        value: &Expr,
        set_op: bool,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let def = {
            let instance = instance.borrow();
            self.classes.get(instance.key()).copied()
        };
        let Some(def) = def else {
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(format!("未定义的类 `{display}`"), line));
        };
        let display = || instance.borrow().type_name().to_string();
        let setter = if set_op {
            def.property_set(name)
                .or_else(|| def.implemented_property_set(name))
        } else {
            def.property_let(name)
                .or_else(|| def.implemented_property_let(name))
        };
        let Some(setter) = setter else {
            return Err(RuntimeError::new(
                format!("类 `{}` 没有可写的 `{name}(下标)`", display()),
                line,
            )
            .with_number(438));
        };
        let direct = if set_op {
            def.property_set(name).is_some()
        } else {
            def.property_let(name).is_some()
        };
        if direct {
            self.ensure_class_member_access(instance, name, setter.is_private, line)?;
        }
        let mut args: Vec<Expr> = indices.to_vec();
        args.push(value.clone());
        let locals = Self::instance_locals(instance);
        self.call_user_with_locals(setter, &args, locals, line)?;
        Ok(())
    }

    // ---------- 记录 / With ----------

    /// `Dim p As Point`：按类型定义创建记录，字段取默认值。
    fn new_record(&self, type_name: &str, line: usize) -> Result<Value, RuntimeError> {
        let def = self
            .types
            .get(type_name)
            .copied()
            .ok_or_else(|| RuntimeError::new(format!("未定义的类型 `{type_name}`"), line))?;
        self.build_record(def, 0, line)
    }

    fn build_record(
        &self,
        def: &'p TypeDef,
        depth: usize,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if depth > 16 {
            return Err(RuntimeError::new(
                format!("类型 `{}` 嵌套过深（可能是循环定义）", def.name),
                line,
            ));
        }
        let mut fields = Vec::with_capacity(def.fields.len());
        for field in &def.fields {
            let value = match (&field.record, field.array_bounds) {
                (Some(name), bounds) => {
                    let nested =
                        self.types.get(name).copied().ok_or_else(|| {
                            RuntimeError::new(format!("未定义的类型 `{name}`"), line)
                        })?;
                    let template = self.build_record(nested, depth + 1, line)?;
                    match bounds {
                        Some(bounds) => {
                            let display = template
                                .as_record()
                                .map(|record| record.borrow().type_name().to_string())
                                .unwrap_or_else(|_| name.clone());
                            Self::array_field(bounds, field.ty, Some((display, template)), line)?
                        }
                        None => template,
                    }
                }
                (None, Some(bounds)) => Self::array_field(bounds, field.ty, None, line)?,
                (None, None) => match field.fixed_len {
                    Some(len) => Value::Str(pad_fixed("", len)),
                    None => default_value(field.ty),
                },
            };
            let record_field = match field.fixed_len {
                Some(len) => RecordField::fixed(field.name.clone(), field.ty, value, len),
                None => RecordField::new(field.name.clone(), field.ty, value),
            };
            fields.push(record_field);
        }
        Ok(Value::record(def.display.clone(), fields))
    }

    /// 记录里的定长数组字段：`(下界, 上界)` → `VbArray`。
    fn array_field(
        bounds: (i64, i64),
        ty: VbType,
        record: Option<(String, Value)>,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let count = (bounds.1 - bounds.0 + 1).max(0) as usize;
        if count > MAX_ARRAY_ELEMENTS {
            return Err(RuntimeError::new(
                format!("数组字段超过上限（{MAX_ARRAY_ELEMENTS} 个元素）"),
                line,
            ));
        }
        let lower = i32::try_from(bounds.0)
            .map_err(|_| RuntimeError::new(format!("数组下界 {} 超出范围", bounds.0), line))?;
        let dims = vec![(lower, count)];
        let array = match record {
            Some((name, template)) => VbArray::new_records(dims, false, name, template),
            None => VbArray::new(dims, ty, false),
        };
        Ok(Value::Array(Rc::new(RefCell::new(array))))
    }

    fn read_member(&mut self, base: &Expr, name: &str, line: usize) -> Result<Value, RuntimeError> {
        // 内置 `App` 对象：`App.Path` 是脚本所在目录。
        if let Expr::Var(root) = base
            && root == "app"
            && self.find_slot("app").is_none()
        {
            return match name {
                "path" => Ok(Value::Str(self.script_dir.display().to_string())),
                _ => Err(RuntimeError::new(format!("`App` 没有属性 `{name}`"), line)),
            };
        }

        // 内置 `Err` 对象：`Err.Number` / `Err.Description` / `Err.Source`。
        if let Expr::Var(root) = base
            && root == "err"
            && self.find_slot("err").is_none()
        {
            return Ok(match name {
                "number" => Value::Int(
                    self.last_error
                        .as_ref()
                        .map(|error| error.number)
                        .unwrap_or(0),
                ),
                "description" => Value::Str(
                    self.last_error
                        .as_ref()
                        .map(|error| error.message.to_string())
                        .unwrap_or_default(),
                ),
                "source" => Value::Str(
                    self.last_error
                        .as_ref()
                        .and_then(|error| error.source.as_deref())
                        .map(str::to_string)
                        .unwrap_or_else(|| "vb".to_string()),
                ),
                _ => return Err(RuntimeError::new(format!("`Err` 没有属性 `{name}`"), line)),
            });
        }

        let value = self.eval(base, line)?;
        if let Ok(collection) = value.as_collection() {
            return match name {
                "count" => Ok(Value::Int(collection.borrow().count() as i64)),
                _ => Err(RuntimeError::new(
                    format!("`Collection` 没有属性 `{name}`"),
                    line,
                )),
            };
        }
        if let Ok(entries) = value.as_dictionary() {
            let entries = entries.borrow();
            return match name {
                "count" => Ok(Value::Int(entries.count() as i64)),
                "keys" => Ok(Value::array(entries.keys())),
                "items" => Ok(Value::array(entries.values())),
                _ => Err(RuntimeError::new(
                    format!("`Dictionary` 没有属性 `{name}`"),
                    line,
                )),
            };
        }
        if let Value::Class(instance) = &value {
            return self.read_instance_member(&instance.0, name, line);
        }
        if let Value::ComObject(object) = &value {
            return object.get(name).map_err(|failure| {
                RuntimeError::new(failure.message, line).with_number(failure.number)
            });
        }
        if matches!(value, Value::Empty) {
            return Err(RuntimeError::new(
                format!("对象变量还没有 `Set`，不能访问 `.{name}`"),
                line,
            )
            .with_number(91));
        }
        let record = value
            .as_record()
            .map_err(|_| RuntimeError::new(format!("值不是记录类型，不能访问 `.{name}`"), line))?;
        let record = record.borrow();
        record.get(name).ok_or_else(|| {
            RuntimeError::new(
                format!("类型 `{}` 没有字段 `{name}`", record.type_name()),
                line,
            )
        })
    }

    /// `With` 块里的 `.X` / `.A.B` 读取。
    fn read_with_member(&mut self, path: &[String], line: usize) -> Result<Value, RuntimeError> {
        let Some(root) = self.with_stack.last().cloned() else {
            return Err(RuntimeError::new("`.成员` 只能出现在 `With` 块内", line));
        };
        let Some((last, parents)) = path.split_last() else {
            return Err(RuntimeError::new("缺少成员名", line));
        };
        let mut current = root;
        for parent in parents {
            current = self.read_member(&Expr::Literal(current), parent, line)?;
        }
        self.read_member(&Expr::Literal(current), last, line)
    }

    /// 沿成员路径定位到「最后一层记录 + 字段名」。
    fn record_parent(
        root: Value,
        path: &[String],
        line: usize,
    ) -> Result<(Rc<RefCell<VbRecord>>, String), RuntimeError> {
        let (last, parents) = path
            .split_last()
            .ok_or_else(|| RuntimeError::new("缺少成员名", line))?;
        let mut current = root;
        for step in parents {
            let record = current
                .as_record()
                .map_err(|_| RuntimeError::new(format!("`{step}` 所在的值不是记录类型"), line))?;
            current = {
                let record = record.borrow();
                record.get(step).ok_or_else(|| {
                    RuntimeError::new(
                        format!("类型 `{}` 没有字段 `{step}`", record.type_name()),
                        line,
                    )
                })?
            };
        }
        let record = current
            .as_record()
            .map_err(|_| RuntimeError::new(format!("不能访问 `.{last}`：不是记录类型"), line))?;
        Ok((record, last.clone()))
    }

    /// 沿成员路径定位到最后一层记录，再写入字段。
    fn assign_member(
        &mut self,
        root: Value,
        path: &[String],
        value: Value,
        narrow: Option<VbType>,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let (parent, last) = Self::record_parent(root, path, line)?;
        parent
            .borrow_mut()
            .set_with_narrow(&last, value, narrow)
            .map_err(|message| RuntimeError::from_coerce(message, line))
    }

    /// `Print` / `Print #` 的文本渲染：处理 `,` 的 14 列分区与 `Spc`/`Tab` 补位。
    ///
    /// `start_column` 是当前行已经写出的字符数（`Print #n, ...;` 续行时不为 0）。
    fn render_print(
        &mut self,
        values: &[Expr],
        seps: &[PrintSep],
        start_column: usize,
        line: usize,
    ) -> Result<String, RuntimeError> {
        let mut text = String::new();
        let mut column = start_column;
        for (index, expr) in values.iter().enumerate() {
            if index > 0 && matches!(seps.get(index - 1), Some(PrintSep::Comma)) {
                // `,` 把输出推进到下一个打印区的开头；正好站在区边界上时跳到再下一个。
                let next = (column / PRINT_ZONE + 1) * PRINT_ZONE;
                text.extend(std::iter::repeat_n(' ', next - column));
                column = next;
            }
            if let Expr::Call { name, args } = expr
                && (name == "spc" || name == "tab")
                && self.find_slot(name).is_none()
            {
                let Some(argument) = args.first() else {
                    return Err(RuntimeError::new(format!("`{name}` 缺少第 1 个参数"), line));
                };
                let target = self.eval(argument, line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("`{name}` 的参数必须是整数：{message}"), line)
                })?;
                let pad = if name == "spc" {
                    target.max(0) as usize
                } else {
                    // `Tab(n)`：移到第 n 列（1 基）。已经越过目标列时 VB6 会先换行再到第 n 列
                    // （实测 `"abc"; Tab(2); "z"` 是 `abc` 换行后 ` z`）。
                    let wanted = (target.max(1) - 1) as usize;
                    if wanted >= column {
                        let pad = wanted - column;
                        text.extend(std::iter::repeat_n(' ', pad));
                        column = wanted;
                        continue;
                    }
                    text.push('\n');
                    text.extend(std::iter::repeat_n(' ', wanted));
                    column = wanted;
                    continue;
                };
                text.extend(std::iter::repeat_n(' ', pad));
                column += pad;
                continue;
            }
            let value = self.eval(expr, line)?;
            // `Single` 按 7 位有效数字打印（VB6 实测）；其余类型仍走 `to_display`。
            let rendered = match &value {
                Value::Float(number) if self.static_type(expr) == Some(VbType::Single) => {
                    crate::interpreter::value::format_single(*number)
                }
                // VB6 实测：`Debug.Print 对象` / `Print #1, 对象` 报 450。
                Value::ComObject(_) => {
                    return Err(RuntimeError::new(
                        "对象没有可用的默认属性，不能转换为字符串",
                        line,
                    )
                    .with_number(450));
                }
                _ => value.to_display(),
            };
            // 边写边记列位置，避免每个值都重新数一遍整段文本（`Print` 参数很多时是 O(n²)）。
            column += rendered.chars().count();
            text.push_str(&rendered);
        }
        Ok(text)
    }

    fn exec_do(
        &mut self,
        pre: Option<&(bool, Expr)>,
        post: Option<&(bool, Expr)>,
        body: &[Stmt],
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        loop {
            self.tick(line)?;
            if let Some((positive, cond)) = pre
                && !self.cond_matches(*positive, cond, line)?
            {
                break;
            }
            match self.exec_block(body)? {
                Flow::Normal => {}
                Flow::ExitFor | Flow::ExitDo => break,
                other => return Ok(other),
            }
            if let Some((positive, cond)) = post
                && !self.cond_matches(*positive, cond, line)?
            {
                break;
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_select(
        &mut self,
        subject: &Expr,
        arms: &[(Vec<CasePattern>, Vec<Stmt>)],
        otherwise: Option<&Vec<Stmt>>,
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        let subject = self.eval(subject, line)?;
        for (patterns, body) in arms {
            for pattern in patterns {
                let matched = match pattern {
                    CasePattern::Value(expr) => {
                        let value = self.eval(expr, line)?;
                        compare_values(&subject, &value, self.option_compare_text)
                            .map_err(|message| RuntimeError::new(message, line))?
                            == Ordering::Equal
                    }
                    CasePattern::Is(op, expr) => {
                        let value = self.eval(expr, line)?;
                        let ordering = compare_values(&subject, &value, self.option_compare_text)
                            .map_err(|message| RuntimeError::new(message, line))?;
                        match op {
                            BinOp::Eq => ordering == Ordering::Equal,
                            BinOp::NotEq => ordering != Ordering::Equal,
                            BinOp::Lt => ordering == Ordering::Less,
                            BinOp::Gt => ordering == Ordering::Greater,
                            BinOp::LtEq => ordering != Ordering::Greater,
                            BinOp::GtEq => ordering != Ordering::Less,
                            _ => false,
                        }
                    }
                    CasePattern::Range(first, last) => {
                        let low = self.eval(first, line)?;
                        let high = self.eval(last, line)?;
                        compare_values(&subject, &low, self.option_compare_text)
                            .map_err(|message| RuntimeError::new(message, line))?
                            != Ordering::Less
                            && compare_values(&subject, &high, self.option_compare_text)
                                .map_err(|message| RuntimeError::new(message, line))?
                                != Ordering::Greater
                    }
                };
                if matched {
                    return self.exec_block(body);
                }
            }
        }
        match otherwise {
            Some(body) => self.exec_block(body),
            None => Ok(Flow::Normal),
        }
    }

    fn eval_condition(&mut self, cond: &Expr, line: usize) -> Result<bool, RuntimeError> {
        self.eval(cond, line)?
            .to_bool()
            .map_err(|message| RuntimeError::from_coerce(message, line))
    }

    fn cond_matches(
        &mut self,
        positive: bool,
        cond: &Expr,
        line: usize,
    ) -> Result<bool, RuntimeError> {
        let value = self.eval_condition(cond, line)?;
        Ok(if positive { value } else { !value })
    }

    // ---------- 表达异 ----------

    fn eval(&mut self, expr: &Expr, line: usize) -> Result<Value, RuntimeError> {
        if self.count_stats {
            self.stats.expressions += 1;
        }
        self.tick(line)?;
        match expr {
            Expr::Literal(value) => Ok(value.clone()),
            Expr::TypedLiteral { value, .. } => Ok(value.clone()),
            Expr::ArrayLiteral(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.eval(item, line)?);
                }
                Ok(Value::array(values))
            }
            Expr::NamedArg { .. } => Err(RuntimeError::new(
                "命名参数仅支持 COM Automation 方法调用",
                line,
            )),
            Expr::Var(name) => {
                if self.count_stats {
                    self.stats.variable_reads += 1;
                }
                if let Some(slot) = self.lookup_slot(name) {
                    return Ok(slot.value());
                }
                // `vbTrue` / `vbCrLf` 这类 VB 标准常量（VB6 里是 VBA 库的全局常量）。
                if let Some(value) = builtins::vb_constant(name) {
                    return Ok(value);
                }
                if self.procedures.contains_key(name)
                    || self.declares.contains_key(name)
                    || builtins::is_builtin(name)
                {
                    return self.call(name, &[], line);
                }
                Err(RuntimeError::new(format!("未定义的变量 `{name}`"), line))
            }
            Expr::Call { name, args } => self.call(name, args, line),
            Expr::Member { base, name } => self.read_member(base, name, line),
            Expr::MemberIndex {
                base,
                name,
                indices,
            } => {
                let root = self.eval(base, line)?;
                if let Value::Class(instance) = &root {
                    return self.call_class_method(&instance.0, name, indices, line);
                }
                if let Value::ComObject(object) = &root {
                    return self.call_com_member(object, name, indices, line);
                }
                if let Ok(collection) = root.as_collection() {
                    let key = match indices.as_slice() {
                        [single] => self.eval(single, line)?,
                        _ => {
                            return Err(RuntimeError::new(
                                "`Collection.Item` 需要一个下标或键",
                                line,
                            ));
                        }
                    };
                    let (value, narrow) = collection
                        .borrow()
                        .item_with_type(&key)
                        .map_err(|message| RuntimeError::new(message, line).with_number(9))?;
                    self.result_narrow = narrow;
                    return Ok(value);
                }
                if let Ok(entries) = root.as_dictionary() {
                    let key = match indices.as_slice() {
                        [single] => self.eval(single, line)?,
                        _ => {
                            return Err(RuntimeError::new("`Dictionary` 的方法需要一个键", line));
                        }
                    };
                    let key = VbDictionary::key_of(&key)
                        .map_err(|message| RuntimeError::new(message, line))?;
                    return match name.as_str() {
                        "exists" => Ok(Value::Bool(entries.borrow().exists(&key))),
                        "item" => entries
                            .borrow()
                            .get(&key)
                            .map_err(|message| RuntimeError::new(message, line).with_number(9)),
                        _ => Err(RuntimeError::new(
                            format!("`Dictionary` 没有方法 `{name}`"),
                            line,
                        )),
                    };
                }
                let indices = self.eval_indices(indices, line)?;
                let field = {
                    let record = root.as_record().map_err(|_| {
                        RuntimeError::new(format!("值不是记录类型，不能访问 `.{name}`"), line)
                    })?;
                    let record = record.borrow();
                    record.get(name).ok_or_else(|| {
                        RuntimeError::new(
                            format!("类型 `{}` 没有字段 `{name}`", record.type_name()),
                            line,
                        )
                    })?
                };
                let array = field
                    .as_array()
                    .map_err(|_| RuntimeError::new(format!("字段 `{name}` 不是数组"), line))?;
                let value = array.borrow().get(&indices).map_err(|message| {
                    RuntimeError::new(format!("字段 `{name}`：{message}"), line).with_number(9)
                })?;
                Ok(value)
            }
            Expr::WithMember { path } => self.read_with_member(path, line),
            Expr::NewObject(kind) => Ok(match kind {
                NewObject::Collection => Value::collection(),
                NewObject::Dictionary => Value::dictionary(),
                NewObject::Class(name) => self.instantiate(name, line)?,
            }),
            Expr::AddressOf(name) => {
                self.resolve_callback(name, line)?;
                Ok(Value::FuncPtr(name.clone()))
            }
            Expr::TypeOf { expr, class_name } => {
                let value = self.eval(expr, line)?;
                let matched = match &value {
                    Value::Class(instance) => instance
                        .0
                        .borrow()
                        .type_name()
                        .eq_ignore_ascii_case(class_name),
                    Value::ComObject(_) => class_name == "object",
                    Value::Empty => false,
                    _ => {
                        return Err(
                            RuntimeError::new("`TypeOf` 只能用于对象", line).with_number(424)
                        );
                    }
                };
                Ok(Value::Bool(matched))
            }
            Expr::Unary { op, expr } => {
                let value = self.eval(expr, line)?;
                let before = match &value {
                    Value::Int(number) => Some(*number),
                    _ => None,
                };
                let negated = match op {
                    UnOp::Negate => match value {
                        Value::Float(number) => Value::Float(-number),
                        Value::Date(number) => Value::Date(-number),
                        Value::Currency(number) => {
                            Value::Currency(number.checked_neg().ok_or_else(|| {
                                RuntimeError::new("数值溢出", line).with_number(6)
                            })?)
                        }
                        other => {
                            let number = other
                                .as_int()
                                .map_err(|message| RuntimeError::new(message, line))?;
                            Value::Int(number.wrapping_neg())
                        }
                    },
                    UnOp::Not => match value {
                        Value::Bool(flag) => Value::Bool(!flag),
                        other => {
                            let number = other
                                .as_int()
                                .map_err(|message| RuntimeError::new(message, line))?;
                            Value::Int(!number)
                        }
                    },
                };
                // `-i`（`i As Integer` = -32768）在 VB6 里报错误号 6（实测）。
                if *op == UnOp::Negate
                    && let (Some(number), Value::Int(after)) = (before, &negated)
                    && fits_i16(number)
                    && !fits_i16(*after)
                    && self.static_type(expr) == Some(VbType::Integer)
                {
                    return Err(RuntimeError::new("溢出", line).with_number(6));
                }
                Ok(negated)
            }
            Expr::Binary { op, left, right } => {
                let mut left_value = self.eval(left, line)?;
                let mut right_value = self.eval(right, line)?;
                // 拼字符串时 `Single` 也按 7 位有效数字写（VB6 实测 `"v=" & CSng(1.23456789)` 是 `v=1.234568`）；
                // 只有真的在拼浮点数时才去查静态类型，字符串拼接的热路径不受影响。
                if *op == BinOp::Concat {
                    self.render_single(left, &mut left_value);
                    self.render_single(right, &mut right_value);
                }
                let mut overflow = None;
                let value = apply_binary(
                    *op,
                    left_value,
                    right_value,
                    line,
                    self.option_compare_text,
                    &mut overflow,
                )?;
                // VB6 的 Integer / Long 运算溢出报错误号 6（实测）：`apply_binary` 只在
                // 「操作数都装得进窄类型、结果却装不进去」时给候选，这里再核实一下静态类型。
                if let Some(narrow) = overflow
                    && Self::binary_static_type(
                        *op,
                        self.static_type(left),
                        self.static_type(right),
                    ) == Some(narrow)
                {
                    return Err(RuntimeError::new("溢出", line).with_number(6));
                }
                Ok(value)
            }
        }
    }

    // ---------- 调用 ----------

    fn call(&mut self, name: &str, args: &[Expr], line: usize) -> Result<Value, RuntimeError> {
        self.call_with_by_value_args(name, args, &[], line)
    }

    /// `CallByName(对象, "成员", 调用方式[, 实参…])`：按名字做后期绑定。
    ///
    /// 语义逐条来自本机 VB6 实测：名字不区分大小写；`VbGet` 取值、`VbLet` 赋值、
    /// `VbSet` 给对象、`VbMethod` 调方法；实参按原表达式传入，所以 `ByRef` 形参照样能改回调用方
    /// （`CallByName(o, "RefArg", VbMethod, x)` 之后 `x` 也会变）。
    fn call_by_name(&mut self, args: &[Expr], line: usize) -> Result<Value, RuntimeError> {
        let Some((object_expr, rest)) = args.split_first() else {
            return Err(
                RuntimeError::new("`CallByName` 需要对象、成员名和调用方式", line).with_number(449),
            );
        };
        if rest.len() < 2 {
            return Err(
                RuntimeError::new("`CallByName` 需要对象、成员名和调用方式", line).with_number(449),
            );
        }
        // `Nothing`（声明成对象的变量）与空 `Variant` 在 VB6 里错号不同：5 / 424。
        let object_is_object = matches!(self.static_type(object_expr), Some(VbType::Object));
        let object = self.eval(object_expr, line)?;
        let name = self.eval(&rest[0], line)?.to_display().to_lowercase();
        let call_type = self
            .eval(&rest[1], line)?
            .coerce(VbType::Long)
            .map_err(|message| RuntimeError::from_coerce(message, line))?;
        let call_args = &rest[2..];
        let Value::Int(call_type) = call_type else {
            return Err(RuntimeError::new("错误的调用方式", line).with_number(438));
        };
        match &object {
            Value::Class(instance) => {
                self.call_by_name_on_instance(&instance.0, &name, call_type, call_args, line)
            }
            Value::ComObject(object) => match call_type {
                1 => self.call_com_member(object, &name, call_args, line),
                2 if call_args.is_empty() => self.call_com_member(object, &name, call_args, line),
                4 | 8 => self.com_put_member(object, &name, call_args, line),
                _ => self.call_com_member(object, &name, call_args, line),
            },
            Value::Empty if object_is_object => {
                Err(RuntimeError::new(builtins::error_text(5), line).with_number(5))
            }
            _ => Err(RuntimeError::new(builtins::error_text(424), line).with_number(424)),
        }
    }

    /// `CallByName` 落在 QIDE 自己的类实例上（`VbGet` / `VbLet` / `VbSet` / `VbMethod`）。
    fn call_by_name_on_instance(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        call_type: i64,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let def = {
            let borrowed = instance.borrow();
            self.classes.get(borrowed.key()).copied()
        };
        let Some(def) = def else {
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(format!("未定义的类 `{display}`"), line));
        };
        match call_type {
            // `VbMethod`
            1 => self.call_class_method(instance, name, args, line),
            // `VbGet`
            2 => {
                let method = def.method(name).or_else(|| def.implemented_method(name));
                if let Some(method) = method
                    && !method.is_get
                {
                    // `Function`/`Sub` 不能用 `VbGet` 取（VB6 实测 `Sub` 是 450，带参数的 `Function` 也是 450）。
                    return Err(RuntimeError::new(builtins::error_text(450), line).with_number(450));
                }
                if method.is_some() && !args.is_empty() {
                    return self.call_class_method(instance, name, args, line);
                }
                self.read_instance_member(instance, name, line)
            }
            // `VbLet` / `VbSet`
            4 | 8 => {
                let set_op = call_type == 8;
                let Some(value) = args.first() else {
                    return Err(RuntimeError::new(builtins::error_text(446), line).with_number(446));
                };
                if args.len() > 1 {
                    return Err(RuntimeError::new(builtins::error_text(451), line).with_number(451));
                }
                self.assign_by_name(instance, def, name, value, set_op, line)
            }
            _ => Err(RuntimeError::new(builtins::error_text(438), line).with_number(438)),
        }
    }

    /// `CallByName` 的 `VbLet` / `VbSet` 分支：有 `Property` 就走它，否则写实例变量。
    fn assign_by_name(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        def: &'p ClassDef,
        name: &str,
        value: &Expr,
        set_op: bool,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let setter = if set_op {
            def.property_set(name)
                .or_else(|| def.implemented_property_set(name))
        } else {
            def.property_let(name)
                .or_else(|| def.implemented_property_let(name))
        };
        if let Some(setter) = setter {
            let direct = if set_op {
                def.property_set(name).is_some()
            } else {
                def.property_let(name).is_some()
            };
            if direct {
                self.ensure_class_member_access(instance, name, setter.is_private, line)?;
            }
            let locals = Self::instance_locals(instance);
            self.call_user_with_locals(setter, std::slice::from_ref(value), locals, line)?;
            return Ok(Value::Empty);
        }
        if let Some(field) = instance.borrow().field(name) {
            let is_private = def.field_is_private(name);
            self.ensure_class_member_access(instance, name, is_private, line)?;
            // VB6 实测：对象类型的字段用 `VbLet` 赋值报 91，用 `VbSet` 给普通字段报 451。
            if !set_op && field.ty == VbType::Object {
                return Err(RuntimeError::new(builtins::error_text(91), line).with_number(91));
            }
            if set_op && !matches!(field.ty, VbType::Object | VbType::Variant) {
                return Err(RuntimeError::new(builtins::error_text(451), line).with_number(451));
            }
            let value = self
                .eval(value, line)?
                .coerce(field.ty)
                .map_err(|message| RuntimeError::from_coerce(message, line))?;
            let with_events = def
                .fields
                .iter()
                .find(|item| item.name == *name)
                .is_some_and(|item| item.with_events);
            if with_events && let Value::Class(source) = &value {
                Self::register_event_sink(&mut self.events, &source.0, instance);
            }
            *field.cell.borrow_mut() = value;
            return Ok(Value::Empty);
        }
        // 只剩只读成员（`Property Get` / `Function` / `Sub`）：VB6 实测 `VbSet` 给只有 `Get` 的属性是 450。
        if def.method(name).is_some() || def.implemented_method(name).is_some() {
            return Err(RuntimeError::new(builtins::error_text(450), line).with_number(450));
        }
        Err(RuntimeError::new(builtins::error_text(438), line).with_number(438))
    }

    /// COM 对象的 `VbLet` / `VbSet`：`IDispatch::Invoke` 的属性写入。
    fn com_put_member(
        &mut self,
        object: &com::ComObject,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let Some(value) = args.first() else {
            return Err(RuntimeError::new(builtins::error_text(446), line).with_number(446));
        };
        let value = self.eval(value, line)?;
        object.set(name, &value).map_err(|failure| {
            RuntimeError::new(failure.message, line).with_number(failure.number)
        })?;
        Ok(Value::Empty)
    }

    /// `VarPtr(变量)` / `ObjPtr(对象)`：返回一个 `Long` 形式的「地址」。
    ///
    /// QIDE 的值是解释器内部结构，没有稳定的原生地址；这里给的是**进程内唯一且稳定**的标识：
    /// `VarPtr` 取变量共享单元的地址（同一变量不变、不同变量不同），
    /// `ObjPtr` 取对象实例（`Rc`）的地址（同一对象相等、不同对象不等）。
    /// 本机 VB6 实测：两者 `VarType` 都是 3（`Long`），`ObjPtr(Nothing)` 是 0。
    fn pointer_of(
        &mut self,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let Some(expr) = args.first() else {
            return Err(
                RuntimeError::new(format!("`{name}` 缺少第 1 个参数"), line).with_number(449)
            );
        };
        if name == "varptr" {
            let Some(slot) = self.lvalue_slot(expr, false, line)? else {
                return Err(
                    RuntimeError::new("`VarPtr` 需要变量、数组元素或记录字段", line)
                        .with_number(449),
                );
            };
            return Ok(Value::Int(std::rc::Rc::as_ptr(&slot.cell) as usize as i64));
        }
        let value = self.eval(expr, line)?;
        let pointer = match &value {
            Value::Empty => 0,
            Value::Class(instance) => std::rc::Rc::as_ptr(&instance.0) as usize as i64,
            Value::Collection(items) => std::rc::Rc::as_ptr(items) as usize as i64,
            Value::Dictionary(entries) => std::rc::Rc::as_ptr(entries) as usize as i64,
            Value::Picture(picture) => std::rc::Rc::as_ptr(picture) as usize as i64,
            Value::Record(record) => std::rc::Rc::as_ptr(record) as usize as i64,
            Value::ComObject(object) => object.identity() as i64,
            _ => {
                return Err(RuntimeError::new("`ObjPtr` 需要一个对象", line).with_number(424));
            }
        };
        Ok(Value::Int(pointer))
    }

    fn call_com_member(
        &mut self,
        object: &com::ComObject,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let values = self.com_call_arguments(args, line)?;
        let call_args = Self::com_call_args(&values);
        let outcome = object.call_with_argument_modes(name, &call_args);
        Self::finish_com_call(values, outcome, line)
    }

    /// `obj(实参…)`：走 COM 的**默认成员**（`DISPID_VALUE`），`d("k")` 等同于 `d.Item("k")`。
    fn call_com_default(
        &mut self,
        object: &com::ComObject,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let values = self.com_call_arguments(args, line)?;
        let call_args = Self::com_call_args(&values);
        let outcome = object.call_default(&call_args);
        Self::finish_com_call(values, outcome, line)
    }

    /// 用**已经求好值**的实参调用 COM 成员（`rs.Fields("id")` 这种链式里中间层用得上）。
    fn call_com_with_values(
        &mut self,
        object: &com::ComObject,
        name: &str,
        values: Vec<Value>,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let arguments: Vec<ComArgument> = values
            .into_iter()
            .map(|value| (None, value, false, None))
            .collect();
        let call_args = Self::com_call_args(&arguments);
        let outcome = object.call_with_argument_modes(name, &call_args);
        Self::finish_com_call(arguments, outcome, line)
    }

    /// 逐个求值 COM 调用的实参，并记下可写回的变量槽（`ByRef` 形参写回用）。
    fn com_call_arguments(
        &mut self,
        args: &[Expr],
        line: usize,
    ) -> Result<Vec<ComArgument>, RuntimeError> {
        let mut values = Vec::with_capacity(args.len());
        for expr in args {
            let (argument_name, argument_expr) = match expr {
                Expr::NamedArg { name, value } => (Some(name.clone()), value.as_ref()),
                _ => (None, expr),
            };
            let value = self.eval(argument_expr, line)?;
            let slot = match argument_expr {
                Expr::Var(_) | Expr::Call { .. } => self.lvalue_slot(argument_expr, false, line)?,
                _ => None,
            };
            let by_ref = slot
                .as_ref()
                .is_some_and(|slot| !slot.is_readonly() && !value.is_array());
            values.push((argument_name, value, by_ref, slot));
        }
        Ok(values)
    }

    fn com_call_args(values: &[ComArgument]) -> Vec<(Option<String>, Value, bool)> {
        values
            .iter()
            .map(|(name, value, by_ref, _)| (name.clone(), value.clone(), *by_ref))
            .collect()
    }

    fn finish_com_call(
        values: Vec<ComArgument>,
        outcome: Result<(Value, Vec<(usize, Value)>), com::ComFailure>,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let (result, updated) = outcome.map_err(|failure| {
            RuntimeError::new(failure.message, line).with_number(failure.number)
        })?;
        for (index, value) in updated {
            if let Some(slot) = values[index].3.as_ref() {
                slot.set(value)
                    .map_err(|message| RuntimeError::from_coerce(message, line))?;
            }
        }
        Ok(result)
    }

    /// 表达式的 VB6 静态类型（只用于 `VarType` / `TypeName` / `Hex` / `Oct` 这类要看类型的地方）。
    ///
    /// 值本身只带动态类型（整数一律是 `i64`），所以这里按语法推断：字面量宽度、变量/参数的声明类型、
    /// 一元与二元运算的类型提升、转换函数与用户过程的返回类型。推断不出来时返回 `None`，
    /// 调用方退回按值判断（大部分内置函数、对象成员、数组元素都属于这一类）。规则逐条来自本机 VB6 实测。
    fn static_type(&self, expr: &Expr) -> Option<VbType> {
        match expr {
            Expr::TypedLiteral { ty, .. } => Some(*ty),
            Expr::Literal(value) => match value {
                // 整数字面量：装得进 Integer 就是 Integer，否则 Long（实测 `VarType(32767)`=2、`VarType(32768)`=3）。
                Value::Int(number) => Some(if i16::try_from(*number).is_ok() {
                    VbType::Integer
                } else {
                    VbType::Long
                }),
                Value::Float(_) => Some(VbType::Double),
                Value::Currency(_) => Some(VbType::Currency),
                Value::Date(_) => Some(VbType::Date),
                Value::Bool(_) => Some(VbType::Boolean),
                Value::Str(_) => Some(VbType::Str),
                _ => None,
            },
            Expr::Var(name) => {
                if let Some(slot) = self.lookup_slot(name) {
                    // 数组按 `vbArray + 元素类型码` 报，交给按值判断。
                    if slot.value().is_array() {
                        return None;
                    }
                    if slot.ty() == VbType::Variant {
                        // `Variant` 变量按赋值时记下的内容类型回答（`v = i` 之后是 Integer）。
                        return slot.narrow_type();
                    }
                    return Some(slot.ty());
                }
                // `VarType(vbTrue)` 是 3（Long）、`VarType(vbCrLf)` 是 8。
                builtins::vb_constant_type(name)
            }
            // `VarType(u.f)`：记录里 `As Variant` 的字段按写入时记下的内容类型回答。
            // `VarType(obj.Prop)`：类成员的返回类型来自 `Property Get` / `Function` 的声明，
            // `VarType(obj.Field)`：`Variant` 字段按写入时记下的内容类型回答。
            Expr::Member { base, name } => {
                let root = self.static_base_value(base)?;
                if let Value::Class(instance) = &root {
                    let borrowed = instance.0.borrow();
                    if let Some(def) = self.classes.get(borrowed.key()).copied()
                        && let Some(method) =
                            def.method(name).or_else(|| def.implemented_method(name))
                    {
                        return (method.ret != VbType::Variant).then_some(method.ret);
                    }
                    let (field_ty, narrow) = borrowed.field_narrow(name)?;
                    return if field_ty == VbType::Variant {
                        narrow
                    } else {
                        Some(field_ty)
                    };
                }
                let record = root.as_record().ok()?;
                let borrowed = record.borrow();
                let (ty, narrow) = borrowed.field_types(name)?;
                if ty == VbType::Variant {
                    narrow
                } else {
                    Some(ty)
                }
            }
            // `With u: VarType(.f)`：与 `u.f` 同样处理。
            Expr::WithMember { path } => {
                let root = self.with_stack.last().cloned()?;
                let mut value = root;
                for (index, name) in path.iter().enumerate() {
                    let record = value.as_record().ok()?;
                    let borrowed = record.borrow();
                    if index + 1 == path.len() {
                        let (ty, narrow) = borrowed.field_types(name)?;
                        return if ty == VbType::Variant {
                            narrow
                        } else {
                            Some(ty)
                        };
                    }
                    value = borrowed.get(name)?;
                }
                None
            }
            // `Not` 保持操作数类型（实测 `Not CByte(1)`=17、`Not 1`=2）；
            // `-32768` 实测是 Long，因为 `32768` 本身已是 Long。
            Expr::Unary { expr, .. } => self.static_type(expr),
            Expr::Binary { op, left, right } => {
                Self::binary_static_type(*op, self.static_type(left), self.static_type(right))
            }
            Expr::Call { name, args } => {
                if let Some(ty) = conversion_type(name) {
                    return Some(ty);
                }
                // `VarType(arr(0))`：`Variant` 数组要取元素记下的内容类型
                // （`picked(0) = CByte(9)` 之后是 17），字面量下标才能静态查。
                if let Some(ty) = self.array_element_static_type(name, args) {
                    return Some(ty);
                }
                match name.as_str() {
                    // `VarType(CVar(1))` 实测是 2：`CVar` 不改类型。
                    "cvar" | "cvdate" | "abs" | "int" | "fix" => {
                        args.first().and_then(|arg| self.static_type(arg))
                    }
                    "hex" | "oct" | "str" | "chr" => Some(VbType::Str),
                    "sgn" => Some(VbType::Integer),
                    _ => self.procedures.get(name).map(|procedure| procedure.ret),
                }
            }
            _ => None,
        }
    }

    /// `static_type` 用的「静态基值」：只认变量与字面量下标的数组元素（`u`、`recs(0)`）。
    fn static_base_value(&self, expr: &Expr) -> Option<Value> {
        match expr {
            Expr::Var(name) => Some(self.lookup_slot(name)?.value()),
            Expr::Call { name, args } => {
                let array = self.lookup_slot(name)?.value();
                if !array.is_array() {
                    return None;
                }
                let mut indices = Vec::with_capacity(args.len());
                for arg in args {
                    match arg {
                        Expr::Literal(Value::Int(number)) => {
                            indices.push(i32::try_from(*number).ok()?);
                        }
                        _ => return None,
                    }
                }
                let array = array.as_array().ok()?;
                array.borrow().get(&indices).ok()
            }
            _ => None,
        }
    }

    /// `arr(0)` 这类**字面量下标**的元素内容类型（只有 `Variant` 数组记了内容类型）。
    fn array_element_static_type(&self, name: &str, args: &[Expr]) -> Option<VbType> {
        let slot = self.find_slot(name)?;
        let value = slot.value();
        if value.is_array() {
            let array = value.as_array().ok()?;
            let array = array.borrow();
            if array.element_type() != VbType::Variant {
                return None;
            }
            let mut indices = Vec::with_capacity(args.len());
            for arg in args {
                match arg {
                    Expr::Literal(Value::Int(number)) => {
                        indices.push(i32::try_from(*number).ok()?);
                    }
                    _ => return None,
                }
            }
            return array.narrow_at(&indices);
        }
        // `VarType(d("k"))`：字典也记了每个键的内容类型，字面量键才静态查。
        if value.is_dictionary()
            && let [Expr::Literal(Value::Str(key))] = args
        {
            let entries = value.as_dictionary().ok()?;
            return entries.borrow().narrow(key);
        }
        None
    }

    /// 二元运算结果类型（比较运算给 Boolean，`/` 与 `^` 给 Double，字符串拼接给 String）。
    fn binary_static_type(
        op: BinOp,
        left: Option<VbType>,
        right: Option<VbType>,
    ) -> Option<VbType> {
        let (left, right) = (left?, right?);
        // `Variant`（含装 `Null` 的变量）在运算里会传播 `Null`，类型推断不出来，交给按值判断
        // （例如 `TypeName(Null = Null)` 是 "Null"）。
        if matches!(left, VbType::Variant | VbType::Any | VbType::Object)
            || matches!(right, VbType::Variant | VbType::Any | VbType::Object)
        {
            return None;
        }
        if matches!(
            op,
            BinOp::Eq
                | BinOp::NotEq
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::LtEq
                | BinOp::GtEq
                | BinOp::Like
                | BinOp::Is
        ) {
            return Some(VbType::Boolean);
        }
        // `+` 与 `&` 遇到字符串都拼接（实测 `VarType("a" & "b")` 与 `VarType("a" + "b")` 都是 String）。
        if op == BinOp::Concat
            || (op == BinOp::Add && (left == VbType::Str || right == VbType::Str))
        {
            return Some(VbType::Str);
        }
        match op {
            // 浮点除法与乘方一律 Double（实测 `VarType(1 / 2)`=5、`VarType(2 ^ 3)`=5）。
            BinOp::Divide | BinOp::Power => Some(VbType::Double),
            _ => promote_numeric(left, right),
        }
    }

    fn call_with_by_value_args(
        &mut self,
        name: &str,
        args: &[Expr],
        by_value_args: &[bool],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if self.count_stats {
            self.stats.calls += 1;
        }
        // `IsMissing` 判定的是"实参有没有省略"，只能从参数槽读，所以在分派前单独处理。
        if name == "ismissing" {
            let missing = match args.first() {
                Some(Expr::Var(var)) => self.find_slot(var).is_some_and(|slot| slot.is_missing()),
                _ => false,
            };
            return Ok(Value::Bool(missing));
        }
        // `VarType` / `TypeName` 要看表达式的**静态类型**（VB6 实测：`Dim n As Integer` 是 2 / "Integer"，
        // `Byte` 是 17、`Single` 是 4，`1 + 1` 也是 2）。值本身不带窄类型信息，所以按语法推断。
        // 推断不出来（大部分内置函数、对象成员、数组）时退回按值判断。
        if matches!(name, "vartype" | "typename")
            && let Some(expr) = args.first()
        {
            if let Some(ty) = self.static_type(expr)
                && !matches!(ty, VbType::Variant | VbType::Any | VbType::Object)
            {
                return Ok(if name == "vartype" {
                    Value::Int(builtins::vartype_of(ty))
                } else {
                    Value::Str(ty.name().to_string())
                });
            }
            // 调用/成员表达式的内容类型只有求值后才知道（返回 `Variant` 的函数、`IIf`/`Choose`/`Switch`、
            // `Collection.Item`），求值时会把类型记进 `result_narrow`
            // （本机 VB6 实测：`IIf(True, i, s)` 是 2、`IIf(False, i, s)` 是 4、`VarType(c.Item(1))` 是 2）。
            if matches!(
                expr,
                Expr::Call { .. }
                    | Expr::Member { .. }
                    | Expr::MemberIndex { .. }
                    | Expr::WithMember { .. }
            ) {
                self.result_narrow = None;
                let value = self.eval(expr, line)?;
                if let Some(ty) = self.result_narrow.take()
                    && !matches!(ty, VbType::Variant | VbType::Any | VbType::Object)
                {
                    return Ok(if name == "vartype" {
                        Value::Int(builtins::vartype_of(ty))
                    } else {
                        Value::Str(ty.name().to_string())
                    });
                }
                return Ok(if name == "vartype" {
                    Value::Int(builtins::vartype_code_of(&value))
                } else {
                    Value::Str(value.type_name())
                });
            }
        }
        // `CStr` / `Str` 拿不到表达式的类型，`Single` 要先按 7 位有效数字渲染再交给内置函数
        // （VB6 实测：`CStr(CSng(1.23456789))` 是 `1.234568`、`Str(CSng(0.5))` 是 `" .5"`）。
        if matches!(name, "cstr" | "str")
            && let Some(expr) = args.first()
            && self.static_type(expr) == Some(VbType::Single)
        {
            let text = match self.eval(expr, line)? {
                Value::Float(number) => crate::interpreter::value::format_single(number),
                Value::Null => return Ok(Value::Null),
                other => other.to_display(),
            };
            return Ok(Value::Str(if name == "str" {
                builtins::str_number_text(text, true)
            } else {
                text
            }));
        }
        if matches!(name, "createobject" | "getobject") {
            if let Some(message) = &self.com_error {
                return Err(RuntimeError::new(message.clone(), line).with_number(429));
            }
            let mut values = Vec::with_capacity(args.len());
            for expr in args {
                values.push(self.eval(expr, line)?);
            }
            if name == "createobject" {
                if values.len() != 1 {
                    return Err(
                        RuntimeError::new("`CreateObject` 需要一个 ProgID 参数", line)
                            .with_number(450),
                    );
                }
                let prog_id = values[0].to_display();
                return com::create_object(&prog_id)
                    .map(Value::ComObject)
                    .map_err(|message| RuntimeError::new(message, line).with_number(429));
            }
            let active_prog_id = match values.as_slice() {
                [path]
                    if {
                        let pathname = path.to_display();
                        is_moniker_path(&pathname, &self.cwd)
                    } =>
                {
                    let pathname = self.resolve_moniker_path(&path.to_display());
                    return com::get_object(&pathname, None)
                        .map(Value::ComObject)
                        .map_err(|message| RuntimeError::new(message, line).with_number(429));
                }
                [Value::Empty, class] => Some(class.to_display()),
                [path, class] if path.to_display().is_empty() => {
                    let prog_id = class.to_display();
                    if prog_id.is_empty() {
                        return Err(RuntimeError::new(
                            "`GetObject` 的路径和 ProgID 不能同时为空",
                            line,
                        )
                        .with_number(5));
                    }
                    return com::create_object(&prog_id)
                        .map(Value::ComObject)
                        .map_err(|message| RuntimeError::new(message, line).with_number(429));
                }
                [class] if class.to_display().is_empty() => {
                    return Err(
                        RuntimeError::new("`GetObject` 缺少 ProgID 参数", line).with_number(450)
                    );
                }
                [class] => Some(class.to_display()),
                [path, class] => {
                    let pathname = path.to_display();
                    let pathname = self.resolve_moniker_path(&pathname);
                    return com::get_object(&pathname, Some(&class.to_display()))
                        .map(Value::ComObject)
                        .map_err(|message| RuntimeError::new(message, line).with_number(429));
                }
                [path, class, ..] => {
                    let _ = (path, class);
                    return Err(RuntimeError::new(
                        "`GetObject` 最多接受路径和 ProgID 两个参数",
                        line,
                    )
                    .with_number(450));
                }
                _ => {
                    return Err(RuntimeError::new(
                        "`GetObject` 需要 ProgID，可写 `GetObject(\"ProgID\")` 或 `GetObject(, \"ProgID\")`",
                        line,
                    )
                    .with_number(450));
                }
            };
            let Some(prog_id) = active_prog_id else {
                return Err(
                    RuntimeError::new("`GetObject` 缺少 ProgID 参数", line).with_number(450)
                );
            };
            return com::get_active_object(&prog_id)
                .map(Value::ComObject)
                .map_err(|message| RuntimeError::new(message, line).with_number(429));
        }
        if let Some(slot) = self.find_slot(name) {
            let value = slot.value();
            if value.is_array() {
                return self.index_array(name, &value, args, line);
            }
            if let Ok(entries) = value.as_dictionary() {
                // `d("键")`：按键读值。
                let key = match args {
                    [single] => self.eval(single, line)?,
                    _ => {
                        return Err(RuntimeError::new("`Dictionary` 的下标只能是一个键", line));
                    }
                };
                let key = VbDictionary::key_of(&key)
                    .map_err(|message| RuntimeError::new(message, line))?;
                return entries
                    .borrow()
                    .get(&key)
                    .map_err(|message| RuntimeError::new(message, line).with_number(9));
            }
            if let Ok(collection) = value.as_collection() {
                // `c(1)` / `c("键")`：`Collection` 的默认成员就是 `Item`（VB6 实测可用）。
                let key = match args {
                    [single] => self.eval(single, line)?,
                    _ => {
                        return Err(RuntimeError::new(
                            "`Collection` 的下标只能是一个下标或键",
                            line,
                        ));
                    }
                };
                let (item, narrow) = collection
                    .borrow()
                    .item_with_type(&key)
                    .map_err(|message| RuntimeError::new(message, line).with_number(9))?;
                self.result_narrow = narrow;
                return Ok(item);
            }
            if let Value::ComObject(object) = &value {
                // `d("k")`：COM 对象的默认成员（`DISPID_VALUE`），等同 `d.Item("k")`（VB6 实测）。
                return self.call_com_default(object, args, line);
            }
        }
        if matches!(name, "eof" | "lof" | "freefile" | "seek" | "loc") {
            return self.file_builtin(name, args, line);
        }
        // 类方法里可以裸调用同类的成员（VB 的隐式 `Me.`）。
        if let Some(def) = self.current_class()
            && let Some(method) = def.method(name)
        {
            let locals = self.current_instance_locals();
            return self.call_user_with_locals_and_by_value_args(
                method,
                args,
                locals,
                by_value_args,
                line,
            );
        }
        if let Some(procedure) = self.procedures.get(name).copied() {
            return self.call_user_with_by_value_args(procedure, args, by_value_args, line);
        }
        if let Some(declare) = self.declares.get(name).copied() {
            return self.call_declare(declare, args, by_value_args, line);
        }
        // `CallByName` 要拿到原始实参表达式（`ByRef` 形参得能写回调用方的槽），所以放在内置函数分派之前。
        if name == "callbyname" {
            return self.call_by_name(args, line);
        }
        // `VarPtr` / `ObjPtr` 也要原始表达式：前者取变量的存储单元，后者取对象实例的身份。
        if name == "varptr" || name == "objptr" {
            return self.pointer_of(name, args, line);
        }
        if builtins::is_builtin(name) {
            // `Hex` / `Oct` 的输出宽度跟着实参的静态类型走（`Hex(-1)` 是 16 位、`Hex(-1&)` 是 32 位）。
            let arg_type = if matches!(name, "hex" | "oct") {
                args.first().and_then(|expr| self.static_type(expr))
            } else {
                None
            };
            // `Array(CByte(1), 2)` 要把每个实参的类型一起记进数组（`For Each` / `VarType` 用）；
            // `IIf`/`Choose`/`Switch` 返回的是其中一个实参，要用它的类型回答 `VarType`。
            let arg_types: Vec<Option<VbType>> = if name == "array" {
                args.iter().map(|expr| self.narrow_of(Some(expr))).collect()
            } else if matches!(name, "iif" | "choose" | "switch") {
                args.iter().map(|expr| self.narrow_of(Some(expr))).collect()
            } else {
                Vec::new()
            };
            let mut values = Vec::with_capacity(args.len());
            for expr in args {
                values.push(self.eval(expr, line)?);
            }
            let types = &self.types;
            let size_of = |name: &str| ffi::record_size(name, types).ok();
            let mut ctx = builtins::Ctx {
                rng: &mut self.rng,
                started: self.started,
                interrupt: self.interrupt,
                err_number: self
                    .last_error
                    .as_ref()
                    .map(|error| error.number)
                    .unwrap_or(0),
                record_size: &size_of,
                dir: &mut self.dir,
                compare_text: self.option_compare_text,
                allow_ui: self.allow_ui,
                failure: None,
                command: &self.command_line,
                cwd: &mut self.cwd,
                strings: &mut self.strings,
                arg_type,
                arg_types: &arg_types,
                result_narrow: None,
            };
            let outcome = builtins::call(name, values, &mut ctx);
            let failure = ctx.failure;
            self.result_narrow = ctx.result_narrow;
            return outcome.map_err(|message| {
                let error = RuntimeError::from_coerce(message, line);
                match failure {
                    Some(number) => error.with_number(number),
                    None => error,
                }
            });
        }
        Err(RuntimeError::new(
            format!("未定义的函数或过程 `{name}`"),
            line,
        ))
    }

    /// `a(0)`、`m(1, 2)`：数组元素读取（写操作在 `Stmt::AssignIndex` 里处理）。
    fn index_array(
        &mut self,
        name: &str,
        array: &Value,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if self.count_stats {
            self.stats.array_accesses += 1;
        }
        let indices = self.eval_indices(args, line)?;
        let array = array
            .as_array()
            .map_err(|message| RuntimeError::new(format!("`{name}` {message}"), line))?;
        let value = array.borrow().get(&indices).map_err(|message| {
            RuntimeError::new(format!("数组 `{name}`：{message}"), line).with_number(9)
        })?;
        Ok(value)
    }

    /// `x = 值` / `x(下标) = 值` 分派到同合 `Property Let`。
    fn call_property_let(
        &mut self,
        procedure: &'p Procedure,
        prefix: &[Expr],
        value: &Expr,
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        let mut args: Vec<Expr> = prefix.to_vec();
        args.push(value.clone());
        if args.len() != procedure.params.len() {
            return Err(RuntimeError::new(
                format!(
                    "`Property Let {}` 需要 {} 个参数，实际给了 {}",
                    procedure.name,
                    procedure.params.len(),
                    args.len()
                ),
                line,
            ));
        }
        self.call_user(procedure, &args, line)?;
        Ok(Flow::Normal)
    }

    fn call_user(
        &mut self,
        procedure: &'p Procedure,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        self.call_user_with_locals(procedure, args, FastMap::default(), line)
    }

    fn call_user_with_by_value_args(
        &mut self,
        procedure: &'p Procedure,
        args: &[Expr],
        by_value_args: &[bool],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        self.call_user_with_locals_and_by_value_args(
            procedure,
            args,
            FastMap::default(),
            by_value_args,
            line,
        )
    }

    /// 执行过程：`locals` 是预先注入的局部变量（类方法用它注入实例变量与 `Me`）。
    fn call_user_with_locals(
        &mut self,
        procedure: &'p Procedure,
        args: &[Expr],
        locals: FastMap<String, Slot>,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        self.call_user_with_locals_and_by_value_args(procedure, args, locals, &[], line)
    }

    fn call_user_with_locals_and_by_value_args(
        &mut self,
        procedure: &'p Procedure,
        args: &[Expr],
        mut locals: FastMap<String, Slot>,
        by_value_args: &[bool],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if self.frames.len() >= self.max_depth {
            return Err(RuntimeError::new(
                format!("调用层次过深（上限 {} 层）", self.max_depth),
                line,
            ));
        }
        // `max_depth` 是逻辑上的层数；真正先到顶的往往是物理栈（debug 构建每层几十 KB）。
        // 与其让进程被栈溢出直接终止，不如在这里报 VB 的错误号 28。
        if stack_headroom() < MIN_CALL_STACK {
            return Err(RuntimeError::new("调用层次过深（栈空间不足）", line).with_number(28));
        }
        if args.len() > procedure.params.len()
            && !procedure.params.iter().any(|param| param.param_array)
        {
            // VB6 实测：实参比形参多是 450（`CallByName(o, "Bump", VbMethod, 1, 2)`）。
            return Err(RuntimeError::new(
                format!(
                    "过程 `{}` 只接受 {} 个参数，实际传入 {} 个",
                    procedure.name,
                    procedure.params.len(),
                    args.len()
                ),
                line,
            )
            .with_number(450));
        }
        // 参数与返回值都要塞进局部表：先按数量预留，省掉逐次扩容的哈希搬迁。
        locals.reserve(procedure.params.len() + 1);
        for (index, param) in procedure.params.iter().enumerate() {
            if param.param_array {
                // `ParamArray xs()`：把剩下的实参打包成 0 基 Variant 数组（连每个元素的内容类型一起记）。
                let mut values = Vec::new();
                for expr in args.iter().skip(index) {
                    let value = self.eval(expr, line)?;
                    let narrow = self.narrow_of(Some(expr));
                    values.push((value, narrow));
                }
                locals.insert(
                    param.name.clone(),
                    Slot::new(Value::array_with_types(values), VbType::Variant),
                );
                continue;
            }
            let slot = match args.get(index) {
                Some(expr) => {
                    let slot = self.bind_argument(
                        expr,
                        param,
                        by_value_args.get(index).copied().unwrap_or(false),
                        line,
                    )?;
                    // `ByVal v As Variant` 也要记下内容类型（VB6 实测 `Narrow(i)` 是 2、`Narrow(CSng(1.5))` 是 4）。
                    // `ByRef` 传进来的就是调用方那个槽，标注已经在它身上了，不能覆盖。
                    if slot.ty() == VbType::Variant
                        && !param.is_array
                        && !(param.by_ref && !by_value_args.get(index).copied().unwrap_or(false))
                    {
                        slot.set_narrow(self.narrow_of(Some(expr)));
                    }
                    slot
                }
                None if param.optional => self.bind_default(param, line)?.omitted_optional(),
                None => {
                    // VB6 实测：缺少必填实参是 449（`CallByName(o, "Total", VbMethod, 3)`）。
                    return Err(RuntimeError::new(
                        format!("过程 `{}` 缺少参数 `{}`", procedure.name, param.name),
                        line,
                    )
                    .with_number(449));
                }
            };
            locals.insert(param.name.clone(), slot);
        }

        let result = if procedure.is_function {
            let slot = Slot::new(Value::Empty, procedure.ret);
            locals.insert(procedure.name.clone(), slot.clone());
            Some(slot)
        } else {
            None
        };

        self.frames.push(Frame {
            locals,
            result: result.clone(),
            procedure: Some(procedure.name.clone()),
            call_line: line,
            ..Frame::default()
        });
        // `With` 是词法作用域：进入过程时把调用方的 With 栈摘下来，返回时还原。
        let outer_with = std::mem::take(&mut self.with_stack);
        let outcome = self.exec_block(&procedure.body);
        self.with_stack = outer_with;
        self.frames.pop();
        outcome?;

        // 返回 `Variant` 的函数：内容类型只有执行完才知道（函数体里 `F = i` 时记在了结果槽上），
        // 顺手交给 `result_narrow`，`VarType(F())` 才能报 `Integer`。
        if procedure.ret == VbType::Variant
            && let Some(slot) = &result
        {
            self.result_narrow = slot.narrow_type();
        }
        Ok(result.map(|slot| slot.value()).unwrap_or(Value::Empty))
    }

    /// 绑定一个实参：变量、数组元素与记录字段按引用共享存储，其余按值传入。
    fn bind_argument(
        &mut self,
        expr: &Expr,
        param: &Param,
        force_by_value: bool,
        line: usize,
    ) -> Result<Slot, RuntimeError> {
        if param.by_ref && !force_by_value {
            if let Some(slot) = self.lvalue_slot(expr, param.is_array, line)? {
                return Ok(slot);
            }
            if let Expr::Var(name) = expr {
                return Err(RuntimeError::new(format!("未定义的变量 `{name}`"), line));
            }
        }
        let value = self.eval(expr, line)?;
        Self::bind_value(value, param, line)
    }

    /// 可赋值目标的存储单元：变量、数组元素、记录字段（含 UDT 数组字段的元素）。
    ///
    /// `ByRef` 实参和 `Get #` 的目标共用这一套解析，保证 `Bump a(0)` 与
    /// `Get #1, , a(0)` 落在同一份存储上。
    fn lvalue_slot(
        &mut self,
        expr: &Expr,
        want_array: bool,
        line: usize,
    ) -> Result<Option<Slot>, RuntimeError> {
        if want_array {
            return Ok(match expr {
                Expr::Var(name) => self.find_slot(name),
                _ => None,
            });
        }
        match expr {
            Expr::Var(name) => Ok(self.find_slot(name)),
            Expr::Call { name, args } => {
                let Some(slot) = self.find_slot(name) else {
                    return Ok(None);
                };
                let value = slot.value();
                if !value.is_array() {
                    return Ok(None);
                }
                let indices = self.eval_indices(args, line)?;
                let array = value
                    .as_array()
                    .map_err(|message| RuntimeError::new(format!("`{name}` {message}"), line))?;
                let (cell, ty) = array.borrow().element_cell(&indices).map_err(|message| {
                    RuntimeError::new(format!("数组 `{name}`：{message}"), line).with_number(9)
                })?;
                Ok(Some(Slot::from_cell(cell, ty)))
            }
            Expr::MemberIndex {
                base,
                name,
                indices,
            } => {
                let root = self.eval(base, line)?;
                if let Value::Class(instance) = &root {
                    // `obj.Prop(1)`：实例数组字段的元素，或带下标的 Property Get 的存储。
                    let field = instance.0.borrow().field(name);
                    if let Some(field) = field {
                        let is_private = {
                            let borrowed = instance.0.borrow();
                            self.classes
                                .get(borrowed.key())
                                .is_some_and(|def| def.field_is_private(name))
                        };
                        self.ensure_class_member_access(&instance.0, name, is_private, line)?;
                        let value = field.cell.borrow().clone();
                        let indices = self.eval_indices(indices, line)?;
                        let array = value.as_array().map_err(|_| {
                            RuntimeError::new(format!("成员 `{name}` 不是数组"), line)
                        })?;
                        let (cell, ty) =
                            array.borrow().element_cell(&indices).map_err(|message| {
                                RuntimeError::new(format!("成员 `{name}`：{message}"), line)
                                    .with_number(9)
                            })?;
                        return Ok(Some(Slot::from_cell(cell, ty)));
                    }
                    return Ok(None);
                }
                let indices = self.eval_indices(indices, line)?;
                let field = {
                    let record = root.as_record().map_err(|_| {
                        RuntimeError::new(format!("值不是记录类型，不能访问 `.{name}`"), line)
                    })?;
                    let record = record.borrow();
                    record.get(name).ok_or_else(|| {
                        RuntimeError::new(
                            format!("类型 `{}` 没有字段 `{name}`", record.type_name()),
                            line,
                        )
                    })?
                };
                let array = field
                    .as_array()
                    .map_err(|_| RuntimeError::new(format!("字段 `{name}` 不是数组"), line))?;
                let (cell, ty) = array.borrow().element_cell(&indices).map_err(|message| {
                    RuntimeError::new(format!("字段 `{name}`：{message}"), line).with_number(9)
                })?;
                Ok(Some(Slot::from_cell(cell, ty)))
            }
            Expr::Member { base, name } => {
                let root = self.eval(base, line)?;
                if let Value::Class(instance) = &root {
                    let field = instance.0.borrow().field(name);
                    if let Some(field) = field {
                        let is_private = {
                            let borrowed = instance.0.borrow();
                            self.classes
                                .get(borrowed.key())
                                .is_some_and(|def| def.field_is_private(name))
                        };
                        self.ensure_class_member_access(&instance.0, name, is_private, line)?;
                        return Ok(Some(Slot::from_cell(field.cell, field.ty)));
                    }
                    return Ok(None);
                }
                let (parent, last) = Self::record_parent(root, std::slice::from_ref(name), line)?;
                Ok(Some(Self::field_slot(&parent, &last, line)?))
            }
            Expr::WithMember { path } => match self.with_stack.last().cloned() {
                Some(root) => {
                    let (parent, last) = Self::record_parent(root, path, line)?;
                    Ok(Some(Self::field_slot(&parent, &last, line)?))
                }
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// 记录字段的存储单元。
    fn field_slot(
        parent: &Rc<RefCell<VbRecord>>,
        name: &str,
        line: usize,
    ) -> Result<Slot, RuntimeError> {
        let parent = parent.borrow();
        let field = parent.fields().iter().find(|field| field.name == name);
        let Some(field) = field else {
            return Err(RuntimeError::new(
                format!("类型 `{}` 没有字段 `{name}`", parent.type_name()),
                line,
            ));
        };
        Ok(Slot::from_cell_with_narrow(
            field.cell(),
            field.ty,
            field.narrow(),
        ))
    }

    fn bind_default(&mut self, param: &Param, line: usize) -> Result<Slot, RuntimeError> {
        let value = match &param.default {
            Some(expr) => self.eval(expr, line)?,
            None if param.is_array => Value::array_new(Vec::new(), param.ty, true),
            None => Value::Empty,
        };
        Self::bind_value(value, param, line)
    }

    fn bind_value(value: Value, param: &Param, line: usize) -> Result<Slot, RuntimeError> {
        if param.is_array {
            // `ByVal arr()` 得到一份独立副本；外层 `ByRef` 分支会共享存储。
            value.as_array().map_err(|_| {
                RuntimeError::new(
                    format!("参数 `{}` 需要数组，实际传入了普通值", param.name),
                    line,
                )
            })?;
            Ok(Slot::new(value.copy_value(), VbType::Variant))
        } else {
            let coerced = value
                .coerce(param.ty)
                .map_err(|message| RuntimeError::from_coerce(message, line))?;
            Ok(Slot::new(coerced, param.ty))
        }
    }

    fn call_declare(
        &mut self,
        declare: &'p Declare,
        args: &[Expr],
        by_value_args: &[bool],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let mut values = Vec::with_capacity(declare.params.len());
        for (index, param) in declare.params.iter().enumerate() {
            let Some(expr) = args.get(index) else {
                values.push(ffi::Arg::ByVal(Value::Empty));
                continue;
            };
            if param.by_ref
                && !by_value_args.get(index).copied().unwrap_or(false)
                && let Expr::Var(name) = expr
            {
                match self.find_slot(name) {
                    Some(slot) => {
                        values.push(ffi::Arg::ByRef(slot));
                        continue;
                    }
                    None => {
                        return Err(RuntimeError::new(format!("未定义的变量 `{name}`"), line));
                    }
                }
            }
            values.push(ffi::Arg::ByVal(self.eval(expr, line)?));
        }

        // 结构体里存着的 `AddressOf`：API 可能把指针保存下来以后再回调，
        // 所以这里换成整次脚本运行期间都有效的跳板。
        for value in &values {
            match value {
                ffi::Arg::ByVal(value) => self.materialize_record_callbacks(value, line)?,
                ffi::Arg::ByRef(slot) => {
                    self.materialize_record_callbacks(&slot.value(), line)?;
                }
                ffi::Arg::Callback(_) => {}
            }
        }

        // `AddressOf` 实参：现场生成跳板，调用返回后立刻释放。
        let mut contexts: Vec<Box<CallbackCtx>> = Vec::new();
        let mut trampolines: Vec<Trampoline> = Vec::new();
        // 先记下要换成跳板的实参位置，循环里才好修改 `values`。
        let callbacks: Vec<(usize, String)> = values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| match value {
                ffi::Arg::ByVal(Value::FuncPtr(name)) => Some((index, name.clone())),
                _ => None,
            })
            .collect();
        for (index, name) in callbacks {
            let declared = declare.params[index].ty;
            if !matches!(
                declared,
                VbType::Long
                    | VbType::LongLong
                    | VbType::LongPtr
                    | VbType::Variant
                    | VbType::Any
                    | VbType::Object
            ) {
                return Err(RuntimeError::new(
                    format!(
                        "`{}` 的参数 `{}` 是 {}，`AddressOf` 只能传给整数/指针参数",
                        declare.symbol,
                        declare.params[index].name,
                        declared.name()
                    ),
                    line,
                ));
            }
            let procedure = self.resolve_callback(&name, line)?;
            contexts.push(Box::new(CallbackCtx::new(
                self as *mut Self as *mut (),
                self.generation,
                procedure,
            )));
            let context = &**contexts.last().expect("刚压入") as *const CallbackCtx as usize;
            let thunk = callback_thunk_address(procedure.ret);
            let trampoline = Trampoline::new(context, thunk, procedure.params.len())
                .map_err(|message| RuntimeError::new(message, line))?;
            values[index] = ffi::Arg::Callback(trampoline.address());
            trampolines.push(trampoline);
        }

        // 类型表克隆一份（只复制共享引用），这样 `ffi::call` 期间不再借用 `self`，
        // 回调才能安全地重新进入解释器。
        let types = self.types.clone();
        let outcome =
            ffi::call(declare, &values, &types).map_err(|message| RuntimeError::new(message, line));
        // 回调里出的错跨不过 FFI 边界，调用返回后在这里补报。
        for context in &contexts {
            if let Some(error) = context.error.take() {
                return Err(error);
            }
        }
        for callback in &self.persistent_callbacks {
            if let Some(error) = callback
                .context
                .as_ref()
                .and_then(|context| context.error.take())
            {
                return Err(error);
            }
        }
        outcome
    }

    /// `AddressOf 过程名`：找到回调目标并检查形状是否受支持。
    fn resolve_callback(&self, name: &str, line: usize) -> Result<&'p Procedure, RuntimeError> {
        let Some(procedure) = self.procedures.get(name).copied() else {
            return Err(RuntimeError::new(
                format!("`AddressOf` 找不到过程或函数 `{name}`"),
                line,
            ));
        };
        if procedure.params.len() > crate::interpreter::trampoline::MAX_ABI_ARGS {
            return Err(RuntimeError::new(
                format!(
                    "回调 `{}` 有 {} 个参数，`AddressOf` 最多支持 {} 个",
                    procedure.name,
                    procedure.params.len(),
                    crate::interpreter::trampoline::MAX_ABI_ARGS
                ),
                line,
            ));
        }
        for param in &procedure.params {
            if param.is_array || param.param_array {
                return Err(RuntimeError::new(
                    format!(
                        "回调 `{}` 的数组形参 `{}` 暂不支持",
                        procedure.name, param.name
                    ),
                    line,
                ));
            }
            if param.record.is_some() && !param.by_ref {
                return Err(RuntimeError::new(
                    format!(
                        "回调 `{}` 的 UDT 形参 `{}` 必须声明为 ByRef",
                        procedure.name, param.name
                    ),
                    line,
                ));
            }
            if !matches!(
                param.ty,
                VbType::Byte
                    | VbType::Integer
                    | VbType::Long
                    | VbType::Currency
                    | VbType::Boolean
                    | VbType::LongLong
                    | VbType::LongPtr
                    | VbType::Single
                    | VbType::Double
                    | VbType::Date
                    | VbType::Str
                    | VbType::Any
            ) {
                return Err(RuntimeError::new(
                    format!(
                        "回调 `{}` 的形参 `{}` 是 {}，暂不支持",
                        procedure.name,
                        param.name,
                        param.ty.name()
                    ),
                    line,
                ));
            }
        }
        Ok(procedure)
    }

    /// 把 `AddressOf` 换成持久的跳板地址；同一过程只生成一次。
    fn persistent_callback_address(
        &mut self,
        name: &str,
        line: usize,
    ) -> Result<usize, RuntimeError> {
        let key = name.to_ascii_lowercase();
        if let Some(trampoline) = self
            .persistent_callbacks
            .iter()
            .find(|item| item.name == key)
            .and_then(|callback| callback.trampoline.as_ref())
        {
            return Ok(trampoline.address());
        }
        let procedure = self.resolve_callback(name, line)?;
        let context = Box::new(CallbackCtx::new(
            self as *mut Self as *mut (),
            self.generation,
            procedure,
        ));
        let pointer = &*context as *const CallbackCtx as usize;
        let trampoline = Trampoline::new(
            pointer,
            callback_thunk_address(procedure.ret),
            procedure.params.len(),
        )
        .map_err(|message| RuntimeError::new(message, line))?;
        let address = trampoline.address();
        self.persistent_callbacks.push(PersistentCallback {
            name: key,
            context: Some(context),
            trampoline: Some(trampoline),
        });
        Ok(address)
    }

    /// 结构体字段里的 `AddressOf` 换成真实地址：这类回调可能在 `Declare` 返回之后才被调用。
    fn materialize_record_callbacks(
        &mut self,
        value: &Value,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let Value::Record(record) = value else {
            return Ok(());
        };
        let fields: Vec<(String, Value)> = {
            let record = record.borrow();
            record
                .fields()
                .iter()
                .map(|field| (field.name.clone(), field.value()))
                .collect()
        };
        for (name, field) in fields {
            match field {
                Value::FuncPtr(procedure) => {
                    let address = self.persistent_callback_address(&procedure, line)?;
                    record
                        .borrow_mut()
                        .set(&name, Value::Int(address as i64))
                        .map_err(|message| RuntimeError::from_coerce(message, line))?;
                }
                Value::Record(_) => self.materialize_record_callbacks(&field, line)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// 从原生回调里进入 VB 过程：实参已经按形参类型还原成值。
    fn call_callback_with_params(
        &mut self,
        procedure: &'p Procedure,
        values: Vec<Value>,
    ) -> Result<(Value, Vec<Value>), RuntimeError> {
        self.call_procedure_values_with_params(procedure, values, FastMap::default(), true)
    }

    /// 按「已求值的实参」调用过程；`locals` 是预先注入的局部变量（类方法用它注入实例变量）。
    fn call_procedure_values(
        &mut self,
        procedure: &'p Procedure,
        values: Vec<Value>,
        locals: FastMap<String, Slot>,
    ) -> Result<Value, RuntimeError> {
        self.call_procedure_values_with_params(procedure, values, locals, false)
            .map(|(value, _)| value)
    }

    fn call_procedure_values_with_params(
        &mut self,
        procedure: &'p Procedure,
        values: Vec<Value>,
        mut locals: FastMap<String, Slot>,
        capture_params: bool,
    ) -> Result<(Value, Vec<Value>), RuntimeError> {
        let line = procedure.line;
        if self.frames.len() >= self.max_depth {
            return Err(RuntimeError::new(
                format!("调用层次过深（上限 {} 层）", self.max_depth),
                line,
            ));
        }
        if stack_headroom() < MIN_CALL_STACK {
            return Err(RuntimeError::new("调用层次过深（栈空间不足）", line).with_number(28));
        }
        for (index, param) in procedure.params.iter().enumerate() {
            let value = match values.get(index) {
                Some(value) => value.copy_value(),
                None => Value::Empty,
            };
            locals.insert(param.name.clone(), Self::bind_value(value, param, line)?);
        }
        let result = if procedure.is_function {
            let slot = Slot::new(Value::Empty, procedure.ret);
            locals.insert(procedure.name.clone(), slot.clone());
            Some(slot)
        } else {
            None
        };

        self.frames.push(Frame {
            locals,
            result: result.clone(),
            procedure: Some(procedure.name.clone()),
            call_line: line,
            ..Frame::default()
        });
        let outer_with = std::mem::take(&mut self.with_stack);
        let outcome = self.exec_block(&procedure.body);
        self.with_stack = outer_with;
        let param_values = if capture_params {
            self.frames.last().map_or_else(Vec::new, |frame| {
                procedure
                    .params
                    .iter()
                    .map(|param| {
                        frame
                            .locals
                            .get(&param.name)
                            .map(Slot::value)
                            .unwrap_or(Value::Empty)
                    })
                    .collect()
            })
        } else {
            Vec::new()
        };
        self.frames.pop();
        outcome?;
        Ok((
            result.map(|slot| slot.value()).unwrap_or(Value::Empty),
            param_values,
        ))
    }
}

/// 一次 `Declare` 调用里的回调上下文；`AddressOf` 跳板通过它找回解释器和 VB 过程。
struct CallbackCtx {
    /// `Runtime<'p, 'o>` 擦掉生命周期后的指针，只在本次调用期间有效。
    runtime: *mut (),
    /// 防止旧回调遇到栈地址复用时误入后续的运行时。
    generation: u64,
    /// 要调用的 VB 过程（AST 活得比运行时久）。
    procedure: *const Procedure,
    /// 回调里发生的错误：跨不过 FFI 边界，先存下来等调用结束再报。
    error: RefCell<Option<RuntimeError>>,
    /// String 回调结果须活到调用方复制完返回值。
    strings: RefCell<Vec<Box<[u16]>>>,
}

impl CallbackCtx {
    fn new(runtime: *mut (), generation: u64, procedure: &Procedure) -> Self {
        Self {
            runtime,
            generation,
            procedure: procedure as *const Procedure,
            error: RefCell::new(None),
            strings: RefCell::new(Vec::new()),
        }
    }

    fn fail(&self, error: RuntimeError) {
        let mut slot = self.error.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    }
}

fn callback_runtime_is_active(context: &CallbackCtx) -> bool {
    ACTIVE_RUNTIME.with(|slot| {
        let active = slot.get();
        runtime_identity_matches(
            active.pointer,
            active.generation,
            context.runtime,
            context.generation,
        )
    })
}

pub(crate) fn runtime_identity_matches(
    active_pointer: *mut (),
    active_generation: u64,
    callback_pointer: *mut (),
    callback_generation: u64,
) -> bool {
    active_pointer == callback_pointer && active_generation == callback_generation
}

/// 一个在整次脚本运行期间都有效的回调：API 可能把指针存下来，之后再调用。
///
/// 脚本结束（包括出错/停止）时窗口可能还没销毁，系统之后仍可能回调：
/// 这两个字段会在 [`Runtime`] 的 `Drop` 里泄漏掉，回调入口再检查运行时是否还活着。
struct PersistentCallback {
    name: String,
    context: Option<Box<CallbackCtx>>,
    trampoline: Option<Trampoline>,
}

/// 跳板转发入口：`RCX` 是上下文，`RDX` 指向原始 GP/XMM 参数快照。
type CallbackThunk = unsafe extern "system" fn(
    *mut CallbackCtx,
    *const crate::interpreter::trampoline::CallbackRegisters,
) -> usize;
type CallbackSingleThunk = unsafe extern "system" fn(
    *mut CallbackCtx,
    *const crate::interpreter::trampoline::CallbackRegisters,
) -> f32;
type CallbackDoubleThunk = unsafe extern "system" fn(
    *mut CallbackCtx,
    *const crate::interpreter::trampoline::CallbackRegisters,
) -> f64;
type CallbackStringThunk = unsafe extern "system" fn(
    *mut CallbackCtx,
    *const crate::interpreter::trampoline::CallbackRegisters,
) -> *const u16;

fn callback_thunk_address(ret: VbType) -> usize {
    match ret {
        VbType::Single => callback_single_thunk as CallbackSingleThunk as usize,
        VbType::Double | VbType::Date => callback_double_thunk as CallbackDoubleThunk as usize,
        VbType::Str => callback_string_thunk as CallbackStringThunk as usize,
        _ => callback_thunk as CallbackThunk as usize,
    }
}

/// 机器码跳板的转发入口：参数快照按过程形参类型解码。
unsafe extern "system" fn callback_thunk(
    ctx: *mut CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> usize {
    if ctx.is_null() {
        return 0;
    }
    // SAFETY: 跳板由 `Trampoline` 生成，传入的就是 `call_declare` 栈上或持久回调表里的上下文。
    let context = unsafe { &*ctx };
    // 运行时已经结束（窗口可能还活着）：不再进入解释器，直接返回 0。
    if !callback_runtime_is_active(context) {
        return 0;
    }
    // 回调里 panic 会穿过 extern "system" 直接终止进程，这里先兜住。
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch_callback(context, registers)
    })) {
        Ok(Ok(value)) => match if let Value::Currency(_) = value {
            value.as_currency()
        } else {
            value.as_int()
        } {
            Ok(value) => value as usize,
            Err(message) => {
                context.fail(RuntimeError::new(
                    format!("回调返回值不可转换为整数：{message}"),
                    0,
                ));
                0
            }
        },
        Ok(Err(error)) => {
            context.fail(error);
            0
        }
        Err(_) => {
            context.fail(RuntimeError::new("回调内部出错（解释器异常）", 0));
            0
        }
    }
}

unsafe extern "system" fn callback_single_thunk(
    ctx: *mut CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> f32 {
    let Some(context) = (unsafe { ctx.as_ref() }) else {
        return 0.0;
    };
    if !callback_runtime_is_active(context) {
        return 0.0;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch_callback(context, registers)
    })) {
        Ok(Ok(value)) => match value.as_float() {
            Ok(value) => value as f32,
            Err(message) => {
                context.fail(RuntimeError::new(
                    format!("回调返回值不可转换为 Single：{message}"),
                    0,
                ));
                0.0
            }
        },
        Ok(Err(error)) => {
            context.fail(error);
            0.0
        }
        Err(_) => {
            context.fail(RuntimeError::new("回调内部出错（解释器异常）", 0));
            0.0
        }
    }
}

unsafe extern "system" fn callback_double_thunk(
    ctx: *mut CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> f64 {
    let Some(context) = (unsafe { ctx.as_ref() }) else {
        return 0.0;
    };
    if !callback_runtime_is_active(context) {
        return 0.0;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch_callback(context, registers)
    })) {
        Ok(Ok(value)) => match value.as_float() {
            Ok(value) => value,
            Err(message) => {
                context.fail(RuntimeError::new(
                    format!("回调返回值不可转换为 Double：{message}"),
                    0,
                ));
                0.0
            }
        },
        Ok(Err(error)) => {
            context.fail(error);
            0.0
        }
        Err(_) => {
            context.fail(RuntimeError::new("回调内部出错（解释器异常）", 0));
            0.0
        }
    }
}

unsafe extern "system" fn callback_string_thunk(
    ctx: *mut CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> *const u16 {
    let Some(context) = (unsafe { ctx.as_ref() }) else {
        return std::ptr::null();
    };
    if !callback_runtime_is_active(context) {
        return std::ptr::null();
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch_callback(context, registers)
    })) {
        Ok(Ok(value)) => {
            let mut text: Vec<u16> = value.to_display().encode_utf16().collect();
            text.push(0);
            let text = text.into_boxed_slice();
            let pointer = text.as_ptr();
            context.strings.borrow_mut().push(text);
            pointer
        }
        Ok(Err(error)) => {
            context.fail(error);
            std::ptr::null()
        }
        Err(_) => {
            context.fail(RuntimeError::new("回调内部出错（解释器异常）", 0));
            std::ptr::null()
        }
    }
}

/// 把寄存器里的实参还原成 VB 值并执行过程。
unsafe fn dispatch_callback(
    context: &CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> Result<Value, RuntimeError> {
    if registers.is_null() {
        return Err(RuntimeError::new("回调缺少参数寄存器", 0));
    }
    let registers = unsafe { &*registers };
    // SAFETY: 上下文里的指针在 `ffi::call` 期间一直有效；`call_declare` 在调用外部函数时
    // 不持有 `self` 的任何引用（类型表是提前克隆的），所以这里重新借出 `&mut` 不会
    // 和数据里的其它引用重叠。
    let runtime = unsafe { &mut *(context.runtime as *mut Runtime<'static, 'static>) };
    let procedure = unsafe { &*context.procedure };
    let mut values = Vec::with_capacity(procedure.params.len());
    let mut record_args = Vec::new();
    for (index, param) in procedure.params.iter().enumerate() {
        let raw = if !param.by_ref
            && matches!(param.ty, VbType::Single | VbType::Double | VbType::Date)
        {
            registers.float[index] as usize
        } else {
            registers.integer[index]
        };
        if let Some(record_name) = &param.record {
            if !param.by_ref {
                return Err(RuntimeError::new(
                    format!(
                        "回调 `{}` 的 UDT 形参 `{}` 必须声明为 ByRef",
                        procedure.name, param.name
                    ),
                    procedure.line,
                ));
            }
            if raw == 0 {
                return Err(RuntimeError::new(
                    format!("回调 `{}` 的记录指针为空", procedure.name),
                    procedure.line,
                ));
            }
            let size = ffi::record_size(record_name, &runtime.types)
                .map_err(|message| RuntimeError::new(message, procedure.line))?;
            let value = runtime.new_record(record_name, procedure.line)?;
            let record = value
                .as_record()
                .map_err(|message| RuntimeError::new(message, procedure.line))?;
            let bytes = unsafe { std::slice::from_raw_parts(raw as *const u8, size) };
            ffi::decode_record(record_name, bytes, &record, &runtime.types)
                .map_err(|message| RuntimeError::new(message, procedure.line))?;
            record_args.push((index, raw as *mut u8, record_name.clone()));
            values.push(value);
            continue;
        }
        match callback_arg(param, raw) {
            Ok(value) => values.push(value),
            Err(message) => {
                return Err(RuntimeError::new(
                    format!("回调 `{}`：{message}", procedure.name),
                    procedure.line,
                ));
            }
        }
    }
    let (result, params) = runtime.call_callback_with_params(procedure, values)?;
    for (index, pointer, type_name) in record_args {
        let value = params
            .get(index)
            .ok_or_else(|| RuntimeError::new("回调记录参数丢失", procedure.line))?;
        let record = value
            .as_record()
            .map_err(|message| RuntimeError::new(message, procedure.line))?;
        let bytes = ffi::encode_record(&type_name, &record.borrow(), &runtime.types)
            .map_err(|message| RuntimeError::new(message, procedure.line))?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
        }
    }
    for (index, param) in procedure.params.iter().enumerate() {
        if !param.by_ref || param.record.is_some() {
            continue;
        }
        let value = params
            .get(index)
            .ok_or_else(|| RuntimeError::new("回调参数丢失", procedure.line))?;
        write_callback_arg(param, registers.integer[index], value, context)
            .map_err(|message| RuntimeError::new(message, procedure.line))?;
    }
    Ok(result)
}

fn write_callback_arg(
    param: &Param,
    raw: usize,
    value: &Value,
    context: &CallbackCtx,
) -> Result<(), String> {
    if raw == 0 {
        return Err(format!("ByRef 参数 `{}` 的指针为空", param.name));
    }
    if param.ty == VbType::Str {
        let mut text: Vec<u16> = value.to_display().encode_utf16().collect();
        text.push(0);
        let text = text.into_boxed_slice();
        let pointer = text.as_ptr();
        context.strings.borrow_mut().push(text);
        unsafe {
            (raw as *mut *const u16).write(pointer);
        }
        return Ok(());
    }
    let value = value.coerce(param.ty)?;
    unsafe {
        match param.ty {
            VbType::Byte => (raw as *mut u8).write(value.as_int()? as u8),
            VbType::Integer => (raw as *mut i16).write(value.as_int()? as i16),
            VbType::Boolean => {
                (raw as *mut i32).write(if value.to_bool()? { -1 } else { 0 });
            }
            VbType::Long => (raw as *mut i32).write(value.as_int()? as i32),
            VbType::Single => (raw as *mut f32).write(value.as_float()? as f32),
            VbType::Double => (raw as *mut f64).write(value.as_float()?),
            VbType::Date => (raw as *mut f64).write(value.as_float()?),
            VbType::Currency => (raw as *mut i64).write(value.as_currency()?),
            _ => (raw as *mut i64).write(value.as_int()?),
        }
    }
    Ok(())
}

/// 单个回调实参：`ByRef` 时寄存器里是原生指针，`ByVal` 时就是值本身。
fn callback_arg(param: &Param, raw: usize) -> Result<Value, String> {
    if param.record.is_some() {
        return Err("暂不支持记录类型形参".into());
    }
    if param.is_array || param.param_array {
        return Err("暂不支持数组形参".into());
    }
    if matches!(param.ty, VbType::Str) {
        // `ByVal s As String` 是 `LPCWSTR`，`ByRef s As String` 是 `LPWSTR *`。
        let pointer = if param.by_ref {
            unsafe { *(raw as *const *const u16) }
        } else {
            raw as *const u16
        };
        return Ok(Value::Str(unsafe { ffi::read_wstring(pointer) }));
    }
    Ok(if param.by_ref {
        scalar_from_pointer(raw, param.ty)
    } else {
        scalar_from_register(raw, param.ty)
    })
}

/// `ByRef` 形参：按声明类型从原生指针读值。
fn scalar_from_pointer(raw: usize, ty: VbType) -> Value {
    unsafe {
        match ty {
            VbType::Byte => Value::Int(i64::from(*(raw as *const u8))),
            VbType::Integer => Value::Int(i64::from(*(raw as *const i16))),
            VbType::Boolean => Value::Bool(*(raw as *const i32) != 0),
            VbType::Long => Value::Int(i64::from(*(raw as *const i32))),
            VbType::Currency => Value::Currency(*(raw as *const i64)),
            VbType::Single => Value::Float(f64::from(*(raw as *const f32))),
            VbType::Double => Value::Float(*(raw as *const f64)),
            VbType::Date => Value::Date(*(raw as *const f64)),
            _ => Value::Int(*(raw as *const i64)),
        }
    }
}

/// `ByVal` 形参：寄存器里的值按声明类型截断。
fn scalar_from_register(raw: usize, ty: VbType) -> Value {
    match ty {
        VbType::Byte => Value::Int(i64::from(raw as u8)),
        VbType::Integer => Value::Int(i64::from(raw as u16 as i16)),
        VbType::Boolean => Value::Bool(raw as u32 != 0),
        VbType::Long => Value::Int(i64::from(raw as u32 as i32)),
        VbType::Currency => Value::Currency(raw as i64),
        VbType::Single => Value::Float(f64::from(f32::from_bits(raw as u32))),
        VbType::Double => Value::Float(f64::from_bits(raw as u64)),
        VbType::Date => Value::Date(f64::from_bits(raw as u64)),
        _ => Value::Int(raw as i64),
    }
}

fn stmt_line(stmt: &Stmt) -> usize {
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
fn actions_pad_values(values: &mut Vec<Value>, params: usize) {
    values.truncate(params);
    while values.len() < params {
        values.push(Value::Empty);
    }
}

/// 在同一个语句块里查找标签位置。
fn find_label(stmts: &[Stmt], label: &str) -> Option<usize> {
    stmts
        .iter()
        .position(|stmt| matches!(stmt, Stmt::Label(name, _) if name == label))
}

fn flush_file(handle: FileHandle) {
    if let FileHandle::Writer(mut writer) = handle {
        let _ = writer.sink.flush();
    }
}

/// 转换函数（`CByte` / `CInt` …）的结果声明类型：`Put #` 用它决定写几个字节。
/// 数值类型提升（本机 VB6 实测）：Byte+Byte→Byte、Byte+Integer→Integer、Integer+Long→Long、
/// Integer+Single→Single、Long+Single→Double、Single+Double→Double、Currency 吸收 Single/Double、
/// Date 吸收所有数值（日期算术）。
fn promote_numeric(left: VbType, right: VbType) -> Option<VbType> {
    fn rank(ty: VbType) -> Option<u8> {
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
fn overflow_candidate(left: i64, right: i64, result: i64) -> Option<VbType> {
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

fn fits_i16(value: i64) -> bool {
    i16::try_from(value).is_ok()
}

fn fits_byte(value: i64) -> bool {
    u8::try_from(value).is_ok()
}

fn conversion_type(name: &str) -> Option<VbType> {
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
fn integer_bounds(ty: VbType) -> Option<(i64, i64)> {
    match ty {
        VbType::Byte => Some((0, 255)),
        VbType::Integer => Some((i16::MIN as i64, i16::MAX as i64)),
        VbType::Long => Some((i32::MIN as i64, i32::MAX as i64)),
        _ => None,
    }
}

/// 把 IO 错误映射成 VB 错误号。
fn io_error_number(error: &std::io::Error) -> i64 {
    match error.kind() {
        std::io::ErrorKind::NotFound => 53,
        std::io::ErrorKind::PermissionDenied => 70,
        std::io::ErrorKind::AlreadyExists => 58,
        std::io::ErrorKind::UnexpectedEof => 62,
        _ => 57,
    }
}

/// 打开文件时的错误号：父目录不存在是 VB 的 76（路径未找到），目录在但文件不在才是 53。
fn open_error_number(error: &std::io::Error, path: &Path) -> i64 {
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
fn writer_error(files: &HashMap<i32, FileHandle>, number: i32, line: usize) -> RuntimeError {
    if files.contains_key(&number) {
        RuntimeError::new(format!("文件号 #{number} 不是以 Output/Append 打开"), line)
            .with_number(54)
    } else {
        RuntimeError::new(format!("文件号 #{number} 没有打开"), line).with_number(52)
    }
}

/// `Input #` 的一行：按逗号切分，支持双引号包住的字符串。
fn split_input_fields(text: &str) -> Vec<String> {
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

fn parse_input_value(text: &str) -> Value {
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

fn number_value(value: f64) -> Value {
    if value == value.trunc() && value.abs() < 9e15 {
        Value::Int(value as i64)
    } else {
        Value::Float(value)
    }
}

/// 值是否恰好是整数（`For` 循环用它选中整数路径）。
fn exact_int(value: &Value) -> Option<i64> {
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
fn concat_text(left: &Value, right: &Value) -> String {
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

fn compare_values(left: &Value, right: &Value, compare_text: bool) -> Result<Ordering, String> {
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
fn compare_text_strings(left: &str, right: &str, compare_text: bool) -> Ordering {
    if compare_text {
        left.to_lowercase().cmp(&right.to_lowercase())
    } else {
        left.cmp(right)
    }
}

fn currency_scaled(value: &Value) -> Result<i64, String> {
    match value {
        Value::Currency(value) => Ok(*value),
        Value::Int(value) => value
            .checked_mul(10_000)
            .ok_or_else(|| "Currency 溢出".into()),
        other => other.as_currency(),
    }
}

fn currency_ratio(numerator: i128, denominator: i128) -> Result<i64, String> {
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
fn objects_identical(left: &Value, right: &Value) -> Result<bool, String> {
    Ok(match (left, right) {
        (Value::Empty, Value::Empty) => true,
        (Value::Empty, _) | (_, Value::Empty) => false,
        (Value::Collection(a), Value::Collection(b)) => Rc::ptr_eq(a, b),
        (Value::Dictionary(a), Value::Dictionary(b)) => Rc::ptr_eq(a, b),
        (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(&a.0, &b.0),
        (Value::ComObject(a), Value::ComObject(b)) => a == b,
        (Value::Picture(a), Value::Picture(b)) => Rc::ptr_eq(a, b),
        _ => return Err("`Is` 只能比较对象与 `Nothing`".into()),
    })
}

/// `Write #` 的单个字段：字符串加引号（内部 `"` 双写），布尔/Null 写成 `#…#`。
fn write_field(value: &Value) -> Result<String, String> {
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
enum LikeToken {
    AnySequence,
    AnyCharacter,
    Digit,
    Literal(char),
    CharacterClass {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

fn like_match(value: &str, pattern: &str, compare_text: bool) -> bool {
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

    fn equal_char(left: char, right: char, compare_text: bool) -> bool {
        if compare_text {
            left.to_lowercase().eq(right.to_lowercase())
        } else {
            left == right
        }
    }

    fn matches(
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

fn apply_binary(
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
