//! 语义检查：在语法树上做一遍保守的名字解析。
//!
//! 只在脚本写了 `Option Explicit` 时启用（与运行时的严格模式一致），提前报告
//! 「未声明的变量」和「未定义的函数或过程」。`With` 块里的裸成员可能是对象的
//! 属性，整块跳过；调用点如果名字同时是变量（可能是数组下标）也不报，避免误报。

use std::collections::HashSet;

use crate::interpreter::ast::*;
use crate::interpreter::builtins;

/// 一条语义问题。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// 1 基行号，与解释器的错误行号一致。
    pub line: usize,
    pub message: String,
}

/// 检查整份程序。没有 `Option Explicit` 时只做 `Implements` 检查。
pub fn analyze(program: &Program) -> Vec<Issue> {
    if !program.option_explicit {
        return check_implements(program);
    }

    let mut checker = Checker {
        globals: HashSet::new(),
        callables: HashSet::new(),
        reported: HashSet::new(),
        issues: Vec::new(),
    };
    // 内置函数名可以当值用（`x = Timer`），不当作未声明变量。
    for name in builtins::names() {
        checker.globals.insert((*name).to_string());
        checker.callables.insert((*name).to_string());
    }
    for procedure in &program.procedures {
        checker.callables.insert(procedure.name.clone());
    }
    for class in &program.classes {
        for method in &class.methods {
            checker.callables.insert(method.name.clone());
        }
    }
    for declare in &program.declares {
        checker.callables.insert(declare.name.clone());
    }
    checker.globals.insert("err".to_string());
    // `App.Path` 是运行时内置对象。
    checker.globals.insert("app".to_string());
    collect_declarations(&program.main, &mut checker.globals);
    for def in &program.enums {
        for (name, _) in &def.members {
            checker.globals.insert(name.clone());
        }
    }

    let globals = checker.globals.clone();
    checker.check_block(&program.main, &globals);

    for procedure in &program.procedures {
        checker.check_procedure(procedure, &[]);
    }
    for class in &program.classes {
        let fields = class
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect::<Vec<_>>();
        for method in &class.methods {
            checker.check_procedure(method, &fields);
        }
    }

    let mut issues = check_implements(program);
    issues.extend(checker.issues);
    issues
}

/// `Implements` 的成员一致性检查。
///
/// 接口类的每个公开成员都要求在实现类里存在 `接口名_成员名`（VB6 的约定），
/// 反向也要求实现类里不存在接口没有的 `接口名_xxx`。成员的签名也必须与接口一致：
/// 过程形式（`Sub` / `Function` / `Property Get|Let|Set`）、参数个数、每个参数的
/// 类型、传递方式、数组形式、可选性与默认值、返回类型都要相同；参数名可以不同。
/// 以上规则按本机 VB6 实测（不一致时 VB6 报「过程声明与同名事件或过程的描述不匹配」）。
///
/// 这条检查与 `Option Explicit` 无关：VB6 无论有没有该选项都会在编译期报错。
fn check_implements(program: &Program) -> Vec<Issue> {
    let mut issues = Vec::new();
    for class in &program.classes {
        for interface in &class.implements {
            // 解析阶段已经校验过接口类存在，这里再兜一次底。
            let Some(def) = program.classes.iter().find(|def| &def.name == interface) else {
                continue;
            };
            let interface_members = def
                .methods
                .iter()
                .filter(|method| !method.is_private)
                .collect::<Vec<_>>();
            let members = interface_members
                .iter()
                .map(|method| method.name.clone())
                .collect::<HashSet<_>>();
            for member in &interface_members {
                let expected = format!("{interface}_{}", member.name);
                let Some(implementation) =
                    class.methods.iter().find(|method| method.name == expected)
                else {
                    issues.push(Issue {
                        line: class.line,
                        message: format!(
                            "类 `{}` 没有实现接口 `{interface}` 的成员 `{}`（应写 `{}`）",
                            class.name, member.name, expected
                        ),
                    });
                    continue;
                };
                if let Some(reason) = signature_mismatch(member, implementation) {
                    issues.push(Issue {
                        line: implementation.line,
                        message: format!(
                            "`{expected}` 的签名与接口 `{interface}` 的成员 `{}` 不一致：{reason}",
                            member.name
                        ),
                    });
                }
            }
            let prefix = format!("{interface}_");
            for method in &class.methods {
                let Some(rest) = method.name.strip_prefix(&prefix) else {
                    continue;
                };
                if members.contains(rest) {
                    continue;
                }
                issues.push(Issue {
                    line: method.line,
                    message: format!("接口 `{interface}` 没有成员 `{rest}`"),
                });
            }
        }
    }
    issues
}

/// 过程的形式。接口实现要求两侧形式相同（`Function` 不能顶替 `Property Get`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcedureKind {
    Sub,
    Function,
    Get,
    Let,
    Set,
}

fn procedure_kind(procedure: &Procedure) -> ProcedureKind {
    if procedure.is_set {
        ProcedureKind::Set
    } else if procedure.is_let {
        ProcedureKind::Let
    } else if procedure.is_function {
        if procedure.is_get {
            ProcedureKind::Get
        } else {
            ProcedureKind::Function
        }
    } else {
        ProcedureKind::Sub
    }
}

fn kind_name(kind: ProcedureKind) -> &'static str {
    match kind {
        ProcedureKind::Sub => "Sub",
        ProcedureKind::Function => "Function",
        ProcedureKind::Get => "Property Get",
        ProcedureKind::Let => "Property Let",
        ProcedureKind::Set => "Property Set",
    }
}

/// 比较接口成员与实现成员的签名；一致返回 `None`，否则返回可直接拼进诊断的原因。
fn signature_mismatch(interface: &Procedure, implementation: &Procedure) -> Option<String> {
    let expected_kind = procedure_kind(interface);
    let actual_kind = procedure_kind(implementation);
    if expected_kind != actual_kind {
        return Some(format!(
            "声明形式应为 `{}`，实际是 `{}`",
            kind_name(expected_kind),
            kind_name(actual_kind)
        ));
    }
    if interface.params.len() != implementation.params.len() {
        return Some(format!(
            "参数个数应为 {} 个，实际是 {} 个",
            interface.params.len(),
            implementation.params.len()
        ));
    }
    for (index, (expected, actual)) in interface
        .params
        .iter()
        .zip(&implementation.params)
        .enumerate()
    {
        let position = index + 1;
        // 同一个参数可能有多处不一致，只报最先发现的一处，避免刷屏。
        let category = if expected.param_array != actual.param_array {
            Some("ParamArray 形式")
        } else if expected.is_array != actual.is_array {
            Some("数组形式")
        } else if expected.by_ref != actual.by_ref {
            Some("传递方式")
        } else if expected.ty != actual.ty || !record_names_equal(&expected.record, &actual.record)
        {
            Some("类型")
        } else if expected.optional != actual.optional {
            Some("可选性")
        } else if !defaults_equal(expected.default.as_ref(), actual.default.as_ref()) {
            Some("默认值")
        } else {
            None
        };
        if let Some(category) = category {
            return Some(format!(
                "第 {position} 个参数{category}不一致（接口 `{}`，实现 `{}`）",
                parameter_display(expected),
                parameter_display(actual)
            ));
        }
    }
    if matches!(expected_kind, ProcedureKind::Function | ProcedureKind::Get)
        && interface.ret != implementation.ret
    {
        return Some(format!(
            "返回类型应为 `{}`，实际是 `{}`",
            interface.ret.name(),
            implementation.ret.name()
        ));
    }
    None
}

fn parameter_type_name(param: &Param) -> String {
    match &param.record {
        Some(record) => record.clone(),
        None => param.ty.name().to_string(),
    }
}

/// 参数在诊断里的显示形式，例如 `ByVal w As Long`、`ByRef xs() As Long`。
fn parameter_display(param: &Param) -> String {
    let mut text = String::new();
    let mode = if param.by_ref { "ByRef " } else { "ByVal " };
    text.push_str(mode);
    if param.param_array {
        text.push_str("ParamArray ");
    }
    text.push_str(&param.name);
    if param.is_array {
        text.push_str("()");
    }
    text.push_str(" As ");
    text.push_str(&parameter_type_name(param));
    if param.optional {
        text.push_str("（可选）");
    }
    text
}

fn record_names_equal(left: &Option<String>, right: &Option<String>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        (None, None) => true,
        _ => false,
    }
}

/// 默认值按语法树比较：`= 5` 与 `= 6` 算不同（本机 VB6 实测也拒绝）。
fn defaults_equal(left: Option<&Expr>, right: Option<&Expr>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => format!("{left:?}") == format!("{right:?}"),
        (None, None) => true,
        _ => false,
    }
}

struct Checker {
    globals: HashSet<String>,
    /// 过程、内置函数与 `Declare` 的名字。
    callables: HashSet<String>,
    /// 同一行同一条消息只报一次。
    reported: HashSet<(usize, String)>,
    issues: Vec<Issue>,
}

impl Checker {
    fn check_procedure(&mut self, procedure: &Procedure, class_fields: &[String]) {
        let mut names = self.globals.clone();
        // 函数名本身就是返回值槽，可以直接赋值。
        names.insert(procedure.name.clone());
        names.insert("me".to_string());
        for field in class_fields {
            names.insert(field.clone());
        }
        for parameter in &procedure.params {
            names.insert(parameter.name.clone());
        }
        collect_declarations(&procedure.body, &mut names);
        self.check_block(&procedure.body, &names);
    }

    fn check_block(&mut self, stmts: &[Stmt], names: &HashSet<String>) {
        for stmt in stmts {
            self.check_stmt(stmt, names);
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt, names: &HashSet<String>) {
        match stmt {
            Stmt::Dim { items, line, .. } => {
                for item in items {
                    if let Some(bounds) = &item.bounds {
                        for (lower, upper) in bounds {
                            self.check_expr(lower, names, *line);
                            self.check_expr(upper, names, *line);
                        }
                    }
                    if let Some(initializer) = &item.initializer {
                        self.check_expr(initializer, names, *line);
                    }
                }
            }
            Stmt::Const { value, line, .. } => self.check_expr(value, names, *line),
            Stmt::Redim {
                name, bounds, line, ..
            } => {
                self.check_undefined(name, names, *line, "未定义的数组");
                for (lower, upper) in bounds {
                    self.check_expr(lower, names, *line);
                    self.check_expr(upper, names, *line);
                }
            }
            Stmt::Erase {
                names: erased,
                line,
            } => {
                for name in erased {
                    self.check_undefined(name, names, *line, "未定义的数组");
                }
            }
            Stmt::Assign {
                target,
                value,
                line,
                ..
            } => {
                self.check_undefined(target, names, *line, "未声明的变量");
                self.check_expr(value, names, *line);
            }
            Stmt::AssignIndex {
                name,
                indices,
                value,
                line,
            } => {
                self.check_undefined(name, names, *line, "未声明的变量");
                for index in indices {
                    self.check_expr(index, names, *line);
                }
                self.check_expr(value, names, *line);
            }
            Stmt::AssignMember {
                base, value, line, ..
            } => {
                self.check_expr(base, names, *line);
                self.check_expr(value, names, *line);
            }
            Stmt::AssignMemberIndex {
                base,
                indices,
                value,
                line,
                ..
            } => {
                self.check_expr(base, names, *line);
                for index in indices {
                    self.check_expr(index, names, *line);
                }
                self.check_expr(value, names, *line);
            }
            Stmt::AssignWithMember {
                indices,
                value,
                line,
                ..
            } => {
                for index in indices {
                    self.check_expr(index, names, *line);
                }
                self.check_expr(value, names, *line);
            }
            Stmt::WithCall { args, line, .. } => {
                for arg in args {
                    self.check_expr(arg, names, *line);
                }
            }
            // `With` 块里的裸名字可能是对象的成员，整块跳过。
            Stmt::With { subject, line, .. } => self.check_expr(subject, names, *line),
            Stmt::Call {
                name, args, line, ..
            } => {
                self.check_call(name, names, *line);
                for arg in args {
                    self.check_expr(arg, names, *line);
                }
            }
            Stmt::CallMethod {
                object, args, line, ..
            } => {
                self.check_undefined(object, names, *line, "未声明的变量");
                for arg in args {
                    self.check_expr(arg, names, *line);
                }
            }
            Stmt::Print { values, line, .. } | Stmt::PrintFile { values, line, .. } => {
                for value in values {
                    self.check_expr(value, names, *line);
                }
            }
            Stmt::WriteFile {
                number,
                values,
                line,
            } => {
                self.check_expr(number, names, *line);
                for value in values {
                    self.check_expr(value, names, *line);
                }
            }
            Stmt::If {
                branches,
                otherwise,
                line,
            } => {
                for (condition, body) in branches {
                    self.check_expr(condition, names, *line);
                    self.check_block(body, names);
                }
                if let Some(body) = otherwise {
                    self.check_block(body, names);
                }
            }
            Stmt::For {
                var,
                start,
                end,
                step,
                body,
                line,
            } => {
                // VB 的循环变量也必须先声明（运行时同样会报未声明）。
                self.check_undefined(var, names, *line, "未声明的变量");
                self.check_expr(start, names, *line);
                self.check_expr(end, names, *line);
                if let Some(step) = step {
                    self.check_expr(step, names, *line);
                }
                self.check_block(body, names);
            }
            Stmt::ForEach {
                var,
                collection,
                body,
                line,
            } => {
                self.check_undefined(var, names, *line, "未声明的变量");
                self.check_expr(collection, names, *line);
                self.check_block(body, names);
            }
            Stmt::DoLoop {
                pre,
                post,
                body,
                line,
            } => {
                if let Some((_, condition)) = pre {
                    self.check_expr(condition, names, *line);
                }
                if let Some((_, condition)) = post {
                    self.check_expr(condition, names, *line);
                }
                self.check_block(body, names);
            }
            Stmt::While { cond, body, line } => {
                self.check_expr(cond, names, *line);
                self.check_block(body, names);
            }
            Stmt::Select {
                subject,
                arms,
                otherwise,
                line,
            } => {
                self.check_expr(subject, names, *line);
                for (patterns, body) in arms {
                    for pattern in patterns {
                        match pattern {
                            CasePattern::Value(value) => self.check_expr(value, names, *line),
                            CasePattern::Is(_, value) => self.check_expr(value, names, *line),
                            CasePattern::Range(lower, upper) => {
                                self.check_expr(lower, names, *line);
                                self.check_expr(upper, names, *line);
                            }
                        }
                    }
                    self.check_block(body, names);
                }
                if let Some(body) = otherwise {
                    self.check_block(body, names);
                }
            }
            Stmt::ErrRaise {
                number,
                source,
                description,
                line,
            } => {
                self.check_expr(number, names, *line);
                if let Some(source) = source {
                    self.check_expr(source, names, *line);
                }
                if let Some(description) = description {
                    self.check_expr(description, names, *line);
                }
            }
            Stmt::OpenFile {
                path,
                number,
                record_len,
                line,
                ..
            } => {
                self.check_expr(path, names, *line);
                if let Some(number) = number {
                    self.check_expr(number, names, *line);
                }
                if let Some(record_len) = record_len {
                    self.check_expr(record_len, names, *line);
                }
            }
            Stmt::CloseFile { numbers, line } => {
                for number in numbers {
                    self.check_expr(number, names, *line);
                }
            }
            Stmt::WidthFile {
                number,
                width,
                line,
            } => {
                self.check_expr(number, names, *line);
                self.check_expr(width, names, *line);
            }
            Stmt::MidAssign {
                target,
                start,
                length,
                value,
                line,
            } => {
                self.check_undefined(target, names, *line, "未声明的变量");
                self.check_expr(start, names, *line);
                if let Some(length) = length {
                    self.check_expr(length, names, *line);
                }
                self.check_expr(value, names, *line);
            }
            Stmt::RaiseEvent { args, line, .. } => {
                for arg in args {
                    self.check_expr(arg, names, *line);
                }
            }
            Stmt::LockFile {
                number,
                start,
                end,
                line,
                ..
            } => {
                self.check_expr(number, names, *line);
                if let Some(start) = start {
                    self.check_expr(start, names, *line);
                }
                if let Some(end) = end {
                    self.check_expr(end, names, *line);
                }
            }
            Stmt::LineInput {
                number,
                target,
                line,
            } => {
                self.check_expr(number, names, *line);
                self.check_undefined(target, names, *line, "未声明的变量");
            }
            Stmt::InputFile {
                number,
                targets,
                line,
            } => {
                self.check_expr(number, names, *line);
                for target in targets {
                    self.check_undefined(target, names, *line, "未声明的变量");
                }
            }
            Stmt::GetFile {
                number,
                position,
                target,
                line,
            } => {
                self.check_expr(number, names, *line);
                if let Some(position) = position {
                    self.check_expr(position, names, *line);
                }
                self.check_expr(target, names, *line);
            }
            Stmt::PutFile {
                number,
                position,
                value,
                line,
            } => {
                self.check_expr(number, names, *line);
                if let Some(position) = position {
                    self.check_expr(position, names, *line);
                }
                self.check_expr(value, names, *line);
            }
            Stmt::SeekFile {
                number,
                position,
                line,
            } => {
                self.check_expr(number, names, *line);
                self.check_expr(position, names, *line);
            }
            Stmt::Set {
                target,
                value,
                line,
            } => {
                self.check_undefined(target, names, *line, "未声明的变量");
                self.check_expr(value, names, *line);
            }
            Stmt::Return { value, line } => {
                if let Some(value) = value {
                    self.check_expr(value, names, *line);
                }
            }
            Stmt::Eval { expr, line } => self.check_expr(expr, names, *line),
            Stmt::Exit { .. }
            | Stmt::Label(..)
            | Stmt::Goto(..)
            | Stmt::Gosub(..)
            | Stmt::GosubReturn { .. }
            | Stmt::OnError { .. }
            | Stmt::Resume { .. }
            | Stmt::ErrClear { .. }
            | Stmt::Halt { .. } => {}
        }
    }

    fn check_expr(&mut self, expr: &Expr, names: &HashSet<String>, line: usize) {
        match expr {
            Expr::Literal(_)
            | Expr::TypedLiteral { .. }
            | Expr::NewObject(_)
            | Expr::AddressOf(_) => {}
            Expr::ArrayLiteral(items) => {
                for item in items {
                    self.check_expr(item, names, line);
                }
            }
            Expr::Var(name) => self.check_undefined(name, names, line, "未声明的变量"),
            Expr::Member { base, .. } => self.check_expr(base, names, line),
            Expr::MemberIndex { base, indices, .. } => {
                self.check_expr(base, names, line);
                for index in indices {
                    self.check_expr(index, names, line);
                }
            }
            // `With` 里的 `.X`：主体是隐藏的，无法静态判断。
            Expr::WithMember { .. } => {}
            Expr::NamedArg { value, .. } => self.check_expr(value, names, line),
            Expr::OmittedArg => {}
            Expr::TypeOf { expr, .. } => self.check_expr(expr, names, line),
            Expr::Call { name, args } => {
                self.check_call(name, names, line);
                for arg in args {
                    self.check_expr(arg, names, line);
                }
            }
            Expr::Unary { expr, .. } => self.check_expr(expr, names, line),
            Expr::Binary { left, right, .. } => {
                self.check_expr(left, names, line);
                self.check_expr(right, names, line);
            }
        }
    }

    /// 调用点：名字既不是过程/内置函数，也不是变量（可能是数组下标）时报错。
    fn check_call(&mut self, name: &str, variables: &HashSet<String>, line: usize) {
        if self.callables.contains(name) || variables.contains(name) || self.globals.contains(name)
        {
            return;
        }
        let message = format!("未定义的函数或过程 `{name}`");
        if self.reported.insert((line, message.clone())) {
            self.issues.push(Issue { line, message });
        }
    }

    fn check_undefined(&mut self, name: &str, names: &HashSet<String>, line: usize, kind: &str) {
        if names.contains(name) {
            return;
        }
        let message = match kind {
            "未定义的数组" => format!("未定义的数组 `{name}`"),
            _ => format!("变量 `{name}` 未声明（Option Explicit）"),
        };
        if self.reported.insert((line, message.clone())) {
            self.issues.push(Issue { line, message });
        }
    }
}

/// 收集这一段语句里声明的名字（含嵌套块：VB 的 `Dim` 是过程级的）。
fn collect_declarations(stmts: &[Stmt], names: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Dim { items, .. } => {
                for item in items {
                    names.insert(item.name.clone());
                }
            }
            Stmt::Const { name, .. } => {
                names.insert(name.clone());
            }
            Stmt::If {
                branches,
                otherwise,
                ..
            } => {
                for (_, body) in branches {
                    collect_declarations(body, names);
                }
                if let Some(body) = otherwise {
                    collect_declarations(body, names);
                }
            }
            Stmt::For { body, .. }
            | Stmt::ForEach { body, .. }
            | Stmt::DoLoop { body, .. }
            | Stmt::While { body, .. }
            | Stmt::With { body, .. } => collect_declarations(body, names),
            Stmt::Select {
                arms, otherwise, ..
            } => {
                for (_, body) in arms {
                    collect_declarations(body, names);
                }
                if let Some(body) = otherwise {
                    collect_declarations(body, names);
                }
            }
            _ => {}
        }
    }
}
