//! 抽象语法树。

use crate::interpreter::value::{VbType, Value};

/// 一个完整的 VB 脚本：`Declare` / `Sub` / `Function` 定义，加上模块级语句。
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub declares: Vec<Declare>,
    pub procedures: Vec<Procedure>,
    /// `Type ... End Type` 定义。
    pub types: Vec<TypeDef>,
    /// `Class ... End Class` 定义。
    pub classes: Vec<ClassDef>,
    /// `Enum ... End Enum` 定义。
    pub enums: Vec<EnumDef>,
    pub main: Vec<Stmt>,
    /// `Option Explicit`：要求先声明变量。
    pub option_explicit: bool,
    /// `Option Compare Text`：字符串比较忽略大小写。
    pub option_compare_text: bool,
}

/// `Class 名字 ... End Class`：实例变量 + 成员过程（`Property Get/Let` 也在 `methods` 里）。
#[derive(Clone, Debug)]
pub struct ClassDef {
    /// 小写类名（和 `types`/`procedures` 的键一致）。
    pub name: String,
    /// 源码里的写法，用于报错与 `TypeName`。
    pub display: String,
    pub line: usize,
    /// `Private x As Long` 这类实例变量。
    pub fields: Vec<Declared>,
    pub methods: Vec<Procedure>,
}

impl ClassDef {
    /// 按名字找成员过程（`Property Get` / `Function` / `Sub`，不含 `Property Let/Set`）。
    pub fn method(&self, name: &str) -> Option<&Procedure> {
        self.methods
            .iter()
            .find(|method| method.name == name && !method.is_let)
    }

    /// 按名字找 `Property Let`。
    pub fn property_let(&self, name: &str) -> Option<&Procedure> {
        self.methods
            .iter()
            .find(|method| method.name == name && method.is_let && !method.is_set)
    }

    /// 按名字找 `Property Set`。
    pub fn property_set(&self, name: &str) -> Option<&Procedure> {
        self.methods
            .iter()
            .find(|method| method.name == name && method.is_set)
    }
}

/// `Declare Function MessageBoxW Lib "user32" (...) As Long`。
#[derive(Clone, Debug)]
pub struct Declare {
    pub name: String,
    pub lib: String,
    /// `Alias` 指定的真实导出名，没有就是 `name` 原文。
    pub symbol: String,
    pub params: Vec<Param>,
    pub ret: VbType,
    pub is_function: bool,
    pub line: usize,
}

/// `Sub` / `Function` 定义。
#[derive(Clone, Debug)]
pub struct Procedure {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub is_function: bool,
    /// `Property Let`：只能通过赋值语句调用。
    pub is_let: bool,
    /// `Property Set`：只能通过 `Set x = 对象` 调用。
    pub is_set: bool,
    pub ret: VbType,
    pub line: usize,
}

/// 形参。VB 默认按引用传递（与多数语言相反）。
#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub by_ref: bool,
    pub ty: VbType,
    pub optional: bool,
    /// `Optional x As Long = 5` 里的默认值。
    pub default: Option<Expr>,
    /// `arr() As Long` 这样的数组参数。
    pub is_array: bool,
    /// `ParamArray xs()`：接收调用方剩下的所有实参。
    pub param_array: bool,
    /// `ByRef p As POINT` 里的 UDT 类型名（`Declare` 用它做结构体布局）。
    pub record: Option<String>,
}

/// `Type ... End Type` 里的一个字段。
#[derive(Clone, Debug)]
pub struct TypeField {
    pub name: String,
    pub ty: VbType,
    /// 字段本身是另一个 UDT 时的类型名。
    pub record: Option<String>,
    /// `Name As String * 8`：定长字符串的字符数。
    pub fixed_len: Option<usize>,
    /// `Items(2) As Long` / `Items(1 To 3) As Long`：定长数组字段的 `(下界, 上界)`。
    pub array_bounds: Option<(i64, i64)>,
}

/// 用户自定义类型定义。
#[derive(Clone, Debug)]
pub struct TypeDef {
    /// 规范化（小写）后的名字，用于查找。
    pub name: String,
    /// 源码里的原始拼写，`TypeName` 用它展示。
    pub display: String,
    pub fields: Vec<TypeField>,
    pub line: usize,
}

/// `Enum Color ... End Enum`：成员在运行时注册为模块级常量。
#[derive(Clone, Debug)]
pub struct EnumDef {
    /// 源码里的原始拼写。
    pub display: String,
    /// 成员（名字已小写）与值。
    pub members: Vec<(String, i64)>,
    /// 声明位置。
    pub line: usize,
}

/// `Dim` 里的一个名字：`a`、`a(10)`、`a(1 To 10)`、`a()` 或 `a As Long`。
#[derive(Clone, Debug)]
pub struct Declared {
    pub name: String,
    pub ty: VbType,
    /// `As Point` 这类自定义类型的名字。
    pub record: Option<String>,
    /// `Some([])` 表示动态数组 `a()`，`Some(bounds)` 表示带边界的数组，`None` 是标量。
    pub bounds: Option<Vec<(Expr, Expr)>>,
    /// `Dim c As New Collection`：声明时立即创建对象。
    pub new_object: Option<NewObject>,
    /// `WithEvents m As Foo`：类里的字段要接收 `Foo` 的 `RaiseEvent`。
    pub with_events: bool,
}

/// `New` 能创建的对象种类。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewObject {
    Collection,
    Dictionary,
    /// `New 类名`
    Class(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Negate,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    IntDivide,
    Modulo,
    Power,
    Concat,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    Xor,
    /// `a Is b` / `obj Is Nothing`：对象同一性。
    Is,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Literal(Value),
    Var(String),
    /// `p.X`：读取记录的字段。
    Member {
        base: Box<Expr>,
        name: String,
    },
    /// `p.Items(1)`：读取记录里数组字段的元素。
    MemberIndex {
        base: Box<Expr>,
        name: String,
        indices: Vec<Expr>,
    },
    /// `With` 块内的 `.X` / `.A.B`。
    WithMember {
        path: Vec<String>,
    },
    /// `New Collection` / `New Dictionary`：新建空对象。
    NewObject(NewObject),
    /// `AddressOf 过程名`：取过程的回调入口（只能传给 `Declare` 的指针参数）。
    AddressOf(String),
    /// `TypeOf 表达式 Is 类名`：判断对象是不是指定类的实例。
    TypeOf {
        expr: Box<Expr>,
        class_name: String,
    },
    Call { name: String, args: Vec<Expr> },
    Unary { op: UnOp, expr: Box<Expr> },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
}

/// `Select Case` 的一个分支条件。
#[derive(Clone, Debug)]
pub enum CasePattern {
    /// `Case 1`
    Value(Expr),
    /// `Case Is > 5`
    Is(BinOp, Expr),
    /// `Case 1 To 5`
    Range(Expr, Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitKind {
    For,
    Do,
    Sub,
    Function,
}

/// `On Error` 的三种形式。
#[derive(Clone, Debug)]
pub enum ErrorMode {
    ResumeNext,
    /// `On Error GoTo 标签`。
    Goto(String),
    /// `On Error GoTo 0`。
    Disabled,
}

/// `Resume` / `Resume Next` / `Resume 标签`。
#[derive(Clone, Debug)]
pub enum ResumeTarget {
    /// `Resume`：重新执行出错语句。
    Current,
    /// `Resume Next`：继续执行下一条。
    Next,
    Label(String),
}

/// `Open ... For <mode> As #n` 的打开模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    Input,
    Output,
    Append,
    /// `For Binary`：随机读写字节。
    Binary,
    /// `For Random`：按定长记录读写。
    Random,
}

/// `Print` 里两个表达式之间的分隔符。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintSep {
    /// `,`：按当前语句类型的补位规则（`Debug.Print` 两空格，`Print #` 不补位）。
    Comma,
    /// `;`：不补位。
    Semicolon,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Dim {
        items: Vec<Declared>,
        /// `Static x As Long`：过程内跨调用保留。
        static_: bool,
        line: usize,
    },
    /// `ReDim [Preserve] a(10)`。
    Redim {
        name: String,
        bounds: Vec<(Expr, Expr)>,
        ty: Option<VbType>,
        preserve: bool,
        line: usize,
    },
    /// `Erase a, b`。
    Erase {
        names: Vec<String>,
        line: usize,
    },
    Assign {
        target: String,
        value: Expr,
        /// `x% = 1` 这类隐式声明时使用的类型后缀。
        suffix: Option<char>,
        line: usize,
    },
    /// `a(0) = 1`、`m(1, 2) = 3`：给数组元素赋值。
    AssignIndex {
        name: String,
        indices: Vec<Expr>,
        value: Expr,
        line: usize,
    },
    /// `p.X = 1`、`l.A.X = 2`、`a(0).X = 3`：给记录字段赋值。
    AssignMember {
        base: Expr,
        path: Vec<String>,
        value: Expr,
        /// `Set` 语句：走 `Property Set`，或直接给对象字段赋对象。
        set_op: bool,
        line: usize,
    },
    /// `p.Items(1) = 5`：给记录里数组字段的元素赋值（`Items(1).Sub = 5` 走 `member_path`）。
    AssignMemberIndex {
        base: Expr,
        path: Vec<String>,
        indices: Vec<Expr>,
        member_path: Vec<String>,
        value: Expr,
        /// `Set` 语句：走带下标的 `Property Set`。
        set_op: bool,
        line: usize,
    },
    /// `With` 块内的 `.X = 1`。
    AssignWithMember {
        path: Vec<String>,
        value: Expr,
        line: usize,
    },
    /// `With subject ... End With`。
    With {
        subject: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    Call {
        name: String,
        args: Vec<Expr>,
        line: usize,
    },
    /// `Print` / `Debug.Print`。
    Print {
        values: Vec<Expr>,
        /// 每个间隔的分隔符（`values.len() - 1` 个）。
        seps: Vec<PrintSep>,
        /// 末尾是 `;` 时不换行（只对 `Print #` 有效）。
        suppress_newline: bool,
        line: usize,
    },
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        otherwise: Option<Vec<Stmt>>,
        line: usize,
    },
    For {
        var: String,
        start: Expr,
        end: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
        line: usize,
    },
    /// `For Each item In array`。
    ForEach {
        var: String,
        collection: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    DoLoop {
        pre: Option<(bool, Expr)>,
        post: Option<(bool, Expr)>,
        body: Vec<Stmt>,
        line: usize,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    Select {
        subject: Expr,
        arms: Vec<(Vec<CasePattern>, Vec<Stmt>)>,
        otherwise: Option<Vec<Stmt>>,
        line: usize,
    },
    Exit {
        kind: ExitKind,
        line: usize,
    },
    /// `标签:`
    Label(String, usize),
    /// `GoTo 标签`。
    Goto(String, usize),
    /// `On Error ...`。
    OnError {
        mode: ErrorMode,
        line: usize,
    },
    /// `Resume` / `Resume Next` / `Resume 标签`。
    Resume {
        target: ResumeTarget,
        line: usize,
    },
    /// `Err.Clear`。
    ErrClear {
        line: usize,
    },
    /// `Err.Raise 编号[, 来源][, 描述]`。
    ErrRaise {
        number: Expr,
        source: Option<Expr>,
        description: Option<Expr>,
        line: usize,
    },
    /// `Open 路径 For Input|Output|Append|Binary|Random [As #n] [Len = 记录长度]`。
    OpenFile {
        path: Expr,
        mode: FileMode,
        /// 省略 `As #n` 时为 `None`：运行时自动取一个空闲文件号（等同 `FreeFile`）。
        number: Option<Expr>,
        /// `Len = 记录长度`（`For Random` 用）。
        record_len: Option<Expr>,
        line: usize,
    },
    /// `Close [#n, ...]`。
    CloseFile {
        numbers: Vec<Expr>,
        line: usize,
    },
    /// `Print #n, ...`。
    PrintFile {
        number: Expr,
        values: Vec<Expr>,
        seps: Vec<PrintSep>,
        suppress_newline: bool,
        line: usize,
    },
    /// `Width #n, 宽度`：`Print #` 的自动换行宽度（0 表示不换行）。
    WidthFile {
        number: Expr,
        width: Expr,
        line: usize,
    },
    /// `Write #n, ...`：CSV 风格写入（字符串加引号、字段用逗号分隔、行尾 CRLF）。
    WriteFile {
        number: Expr,
        values: Vec<Expr>,
        line: usize,
    },
    /// `Mid(变量, start[, length]) = 值`：就地替换字符串的一段。
    MidAssign {
        target: String,
        start: Expr,
        length: Option<Expr>,
        value: Expr,
        line: usize,
    },
    /// `RaiseEvent 事件名(实参...)`：把事件发给订阅了这个实例的 `WithEvents` 对象。
    RaiseEvent {
        name: String,
        args: Vec<Expr>,
        line: usize,
    },
    /// `Lock #n [, start [To end]]` / `Unlock #n [, start [To end]]`。
    ///
    /// 范围是 1 基的：`For Binary` 下是字节偏移，`For Random` 下是记录号；省略时锁整个文件。
    LockFile {
        number: Expr,
        unlock: bool,
        start: Option<Expr>,
        end: Option<Expr>,
        line: usize,
    },
    /// `Line Input #n, 变量`。
    LineInput {
        number: Expr,
        target: String,
        line: usize,
    },
    /// `Input #n, 变量, ...`。
    InputFile {
        number: Expr,
        targets: Vec<String>,
        line: usize,
    },
    /// `Get #n, [位置], 变量`（Binary / Random 模式）。目标可以是变量、数组元素或记录字段。
    GetFile {
        number: Expr,
        position: Option<Expr>,
        target: Expr,
        line: usize,
    },
    /// `Put #n, [位置], 表达式`（Binary / Random 模式）。
    PutFile {
        number: Expr,
        position: Option<Expr>,
        value: Expr,
        line: usize,
    },
    /// `Seek #n, 位置`（Binary 模式是 1 基字节偏移，Random 模式是记录号）。
    SeekFile {
        number: Expr,
        position: Expr,
        line: usize,
    },
    /// `Set x = New Collection` / `Set x = Nothing`。
    Set {
        target: String,
        value: Expr,
        line: usize,
    },
    /// `c.Add x[, key]` / `c.Remove i`：集合方法调用。
    CallMethod {
        object: String,
        method: String,
        args: Vec<Expr>,
        line: usize,
    },
    Return {
        value: Option<Expr>,
        line: usize,
    },
    /// `Const x = 1`：声明并赋值（解释器不强制只读）。
    Const {
        name: String,
        ty: VbType,
        value: Expr,
        line: usize,
    },
    /// `Stop`，中断执行。
    Halt {
        line: usize,
    },
}
