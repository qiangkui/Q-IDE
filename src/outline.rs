//! VB 代码大纲：从当前编辑器内容提取过程、变量与类型，并提供可折叠列表。
//!
//! 「变量」一节按作用域（模块、各个过程、各个类）再分一层，便于定位局部变量；
//! 分类与作用域分组都可以折叠。调试暂停时，在当前作用域（含模块）的变量下面
//! 补一行「值 + 类型」；这些行始终展开，不参与折叠。
//! 「监视」一节列出用户添加的表达式，调试暂停时显示各自的求值结果。
//! 「断点」一节按行号列出当前文件的断点，条件断点额外显示条件表达式。
//! 面板顶部的过滤框按名字筛选符号；过滤时符号分区强制展开、空分区隐藏。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::component::dock::DragPanel;
use gpui_kit::component::*;
use gpui_kit::prelude::{
    FluentBuilder as _, InteractiveElement as _, StatefulInteractiveElement as _,
};
use gpui_kit::*;

use crate::panel_drag::PanelDragPreview;
use crate::interpreter::ast::{ClassDef, Program, Procedure, Stmt};
use crate::interpreter::debug::{DebugSnapshot, DebugVariable};
use crate::theme::{self as ui_theme, Palette};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutlineSymbol {
    pub name: String,
    pub detail: String,
    /// 变量所在的作用域（`模块`、过程名或类名）；其它条目为空。
    pub scope: String,
    pub line: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutlineModel {
    pub procedures: Vec<OutlineSymbol>,
    pub variables: Vec<OutlineSymbol>,
    pub types: Vec<OutlineSymbol>,
    pub classes: Vec<OutlineSymbol>,
    pub enums: Vec<OutlineSymbol>,
    pub declares: Vec<OutlineSymbol>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OutlineSection {
    Breakpoints,
    Watch,
    Procedures,
    Variables,
    Types,
    Classes,
    Enums,
    Declares,
}

impl OutlineSection {
    pub const ALL: [Self; 8] = [
        Self::Breakpoints,
        Self::Watch,
        Self::Procedures,
        Self::Variables,
        Self::Types,
        Self::Classes,
        Self::Enums,
        Self::Declares,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Breakpoints => "断点",
            Self::Watch => "监视",
            Self::Procedures => "过程与函数",
            Self::Variables => "变量",
            Self::Types => "自定义类型",
            Self::Classes => "类",
            Self::Enums => "枚举",
            Self::Declares => "API 声明",
        }
    }

    fn symbols(self, model: &OutlineModel) -> &[OutlineSymbol] {
        match self {
            // 监视与断点不来自源码，由 `OutlineRender` 里的对应字段提供。
            Self::Breakpoints | Self::Watch => &[],
            Self::Procedures => &model.procedures,
            Self::Variables => &model.variables,
            Self::Types => &model.types,
            Self::Classes => &model.classes,
            Self::Enums => &model.enums,
            Self::Declares => &model.declares,
        }
    }
}

/// 大纲过滤框的匹配规则：名字包含查询串（忽略大小写）；空查询匹配全部。
pub fn matches_filter(symbol: &OutlineSymbol, query: &str) -> bool {
    let query = query.trim();
    query.is_empty() || symbol.name.to_lowercase().contains(&query.to_lowercase())
}

/// 按过滤框内容筛选符号，保持原有顺序。
pub fn filter_symbols<'a>(
    symbols: &'a [OutlineSymbol],
    query: &str,
) -> Vec<&'a OutlineSymbol> {
    symbols
        .iter()
        .filter(|symbol| matches_filter(symbol, query))
        .collect()
}

/// 「断点」一节里的一行：行号、该行源码与可选的条件。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BreakpointEntry {
    /// 1 基行号。
    pub line: usize,
    /// 该行去掉首尾空白后的源码；行号越界时为空。
    pub text: String,
    /// 条件表达式；普通断点为 `None`。
    pub condition: Option<String>,
}

/// 从断点行号、条件与源码生成「断点」一节的条目，按行号升序。
pub fn breakpoint_entries(
    lines: impl IntoIterator<Item = usize>,
    conditions: &HashMap<usize, String>,
    source: &str,
) -> Vec<BreakpointEntry> {
    let mut lines: Vec<usize> = lines.into_iter().collect();
    lines.sort_unstable();
    lines.dedup();
    lines
        .into_iter()
        .map(|line| BreakpointEntry {
            line,
            text: source
                .lines()
                .nth(line.saturating_sub(1))
                .unwrap_or_default()
                .trim()
                .to_string(),
            condition: conditions.get(&line).cloned(),
        })
        .collect()
}

/// 一条监视表达式的求值状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum WatchState {
    /// 没有暂停，或还没有求值。
    #[default]
    Inactive,
    /// 已经交给解释器，等结果。
    Pending,
    /// 求值成功：值与类型。
    Ready { value: String, ty: String },
    /// 求值失败。
    Failed(String),
}

/// 「监视」一节里的一行：表达式文本 + 求值状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchEntry {
    pub expression: String,
    pub state: WatchState,
}

pub type ToggleSectionAction = Rc<dyn Fn(OutlineSection, &mut Window, &mut App)>;
/// 点击标题栏关闭按钮的回调：收起代码大纲。
pub type ClosePanelAction = Rc<dyn Fn(&mut Window, &mut App)>;
/// 折叠/展开某个作用域分组，参数是作用域名（分组的键）。
pub type ToggleScopeAction = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub type GoToLineAction = Rc<dyn Fn(usize, &mut Window, &mut App)>;

impl OutlineModel {
    pub fn from_source(source: &str) -> Self {
        match crate::interpreter::parse(source) {
            Ok(program) => Self::from_program(&program),
            Err(_) => scan_incomplete_source(source),
        }
    }

    fn from_program(program: &Program) -> Self {
        let mut model = Self::default();

        for procedure in &program.procedures {
            model.procedures.push(procedure_symbol(procedure, None));
            collect_procedure_variables(procedure, &procedure.name, &mut model.variables);
        }
        for declare in &program.declares {
            model.declares.push(OutlineSymbol {
                name: declare.name.clone(),
                detail: if declare.is_function {
                    "Function".to_string()
                } else {
                    "Sub".to_string()
                },
                line: declare.line,
                ..Default::default()
            });
        }
        for ty in &program.types {
            model.types.push(OutlineSymbol {
                name: ty.display.clone(),
                detail: format!("{} 个字段", ty.fields.len()),
                line: ty.line,
                ..Default::default()
            });
        }
        for class in &program.classes {
            model.classes.push(OutlineSymbol {
                name: class.display.clone(),
                detail: format!("{} 个成员", class.fields.len() + class.methods.len()),
                line: class.line,
                ..Default::default()
            });
            for method in &class.methods {
                model
                    .procedures
                    .push(procedure_symbol(method, Some(&class.display)));
                collect_procedure_variables(
                    method,
                    &format!("{}.{}", class.display, method.name),
                    &mut model.variables,
                );
            }
            collect_class_variables(class, &mut model.variables);
        }
        for enumeration in &program.enums {
            model.enums.push(OutlineSymbol {
                name: enumeration.display.clone(),
                detail: format!("{} 个成员", enumeration.members.len()),
                line: enumeration.line,
                ..Default::default()
            });
        }

        collect_variables(&program.main, "模块", &mut model.variables);
        sort_symbols(&mut model);
        model
    }
}

impl OutlineSymbol {
    fn scope_label(&self) -> &str {
        if self.scope.is_empty() {
            UNCLASSIFIED_SCOPE
        } else {
            &self.scope
        }
    }
}

/// 归不到任何作用域的变量（正常情况下不会出现）。
const UNCLASSIFIED_SCOPE: &str = "未分类";

/// 变量按作用域分组：组按首次出现的顺序排列，组内保持原有顺序。
pub(crate) fn group_by_scope(symbols: &[OutlineSymbol]) -> Vec<(&str, Vec<&OutlineSymbol>)> {
    let mut groups: Vec<(&str, Vec<&OutlineSymbol>)> = Vec::new();
    for symbol in symbols {
        let scope = symbol.scope_label();
        match groups.iter_mut().find(|(name, _)| *name == scope) {
            Some((_, items)) => items.push(symbol),
            None => groups.push((scope, vec![symbol])),
        }
    }
    groups
}

fn procedure_symbol(procedure: &Procedure, class: Option<&str>) -> OutlineSymbol {
    let kind = if procedure.is_let {
        "Property Let"
    } else if procedure.is_set {
        "Property Set"
    } else if procedure.is_function {
        "Function"
    } else {
        "Sub"
    };
    let name = class.map_or_else(
        || procedure.name.clone(),
        |class| format!("{class}.{}", procedure.name),
    );
    OutlineSymbol {
        name,
        detail: format!("{kind} · {} 个参数", procedure.params.len()),
        line: procedure.line,
        ..Default::default()
    }
}

fn collect_procedure_variables(
    procedure: &Procedure,
    owner: &str,
    variables: &mut Vec<OutlineSymbol>,
) {
    for parameter in &procedure.params {
        variables.push(OutlineSymbol {
            name: parameter.name.clone(),
            detail: "参数".to_string(),
            scope: owner.to_string(),
            line: procedure.line,
        });
    }
    collect_variables(&procedure.body, owner, variables);
}

fn collect_class_variables(class: &ClassDef, variables: &mut Vec<OutlineSymbol>) {
    for field in &class.fields {
        variables.push(OutlineSymbol {
            name: field.name.clone(),
            detail: "字段".to_string(),
            scope: class.display.clone(),
            line: class.line,
        });
    }
}

fn collect_variables(stmts: &[Stmt], owner: &str, variables: &mut Vec<OutlineSymbol>) {
    for stmt in stmts {
        match stmt {
            Stmt::Dim { items, line, .. } => {
                for item in items {
                    variables.push(OutlineSymbol {
                        name: item.name.clone(),
                        detail: if owner == "模块" {
                            "模块变量".to_string()
                        } else {
                            "局部变量".to_string()
                        },
                        scope: owner.to_string(),
                        line: *line,
                    });
                }
            }
            Stmt::Redim { name, line, .. } => variables.push(OutlineSymbol {
                name: name.clone(),
                detail: "数组".to_string(),
                scope: owner.to_string(),
                line: *line,
            }),
            Stmt::For {
                var, body, line, ..
            }
            | Stmt::ForEach {
                var, body, line, ..
            } => {
                variables.push(OutlineSymbol {
                    name: var.clone(),
                    detail: "循环变量".to_string(),
                    scope: owner.to_string(),
                    line: *line,
                });
                collect_variables(body, owner, variables);
            }
            Stmt::With { body, .. }
            | Stmt::DoLoop { body, .. }
            | Stmt::While { body, .. } => collect_variables(body, owner, variables),
            Stmt::If {
                branches,
                otherwise,
                ..
            } => {
                for (_, body) in branches {
                    collect_variables(body, owner, variables);
                }
                if let Some(body) = otherwise {
                    collect_variables(body, owner, variables);
                }
            }
            Stmt::Select {
                arms, otherwise, ..
            } => {
                for (_, body) in arms {
                    collect_variables(body, owner, variables);
                }
                if let Some(body) = otherwise {
                    collect_variables(body, owner, variables);
                }
            }
            _ => {}
        }
    }
}

fn sort_symbols(model: &mut OutlineModel) {
    for symbols in [
        &mut model.procedures,
        &mut model.variables,
        &mut model.types,
        &mut model.classes,
        &mut model.enums,
        &mut model.declares,
    ] {
        symbols.sort_by(|left, right| {
            left.line
                .cmp(&right.line)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });
    }
}

fn scan_incomplete_source(source: &str) -> OutlineModel {
    let mut model = OutlineModel::default();
    // 代码解析失败时只能按 `End Sub`/`End Class` 粗配作用域，用来给变量归类。
    let mut scopes: Vec<String> = Vec::new();

    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }

        let mut statement = line;
        let mut had_visibility = false;
        while let Some((modifier, rest)) = split_word(statement) {
            if !matches_ignore_ascii_case(
                modifier,
                &["public", "private", "friend", "static", "global"],
            ) {
                break;
            }
            had_visibility = true;
            statement = rest.trim_start();
        }

        if let Some(rest) = strip_word(statement, "end")
            && matches_ignore_ascii_case(
                rest.trim(),
                &["sub", "function", "property", "class", "type", "enum"],
            )
        {
            scopes.pop();
            continue;
        }

        if let Some(rest) = strip_word(statement, "function") {
            enter_named_scope(&mut scopes, &mut model.procedures, rest, "Function", line_number);
            continue;
        }
        if let Some(rest) = strip_word(statement, "sub") {
            enter_named_scope(&mut scopes, &mut model.procedures, rest, "Sub", line_number);
            continue;
        }
        if let Some(rest) = strip_word(statement, "property") {
            let rest = rest.trim_start();
            let (_, rest) = split_word(rest).unwrap_or(("", rest));
            enter_named_scope(&mut scopes, &mut model.procedures, rest, "Property", line_number);
            continue;
        }
        if let Some(rest) = strip_word(statement, "class") {
            enter_named_scope(&mut scopes, &mut model.classes, rest, "Class", line_number);
            continue;
        }
        if let Some(rest) = strip_word(statement, "type") {
            enter_named_scope(&mut scopes, &mut model.types, rest, "Type", line_number);
            continue;
        }
        if let Some(rest) = strip_word(statement, "enum") {
            enter_named_scope(&mut scopes, &mut model.enums, rest, "Enum", line_number);
            continue;
        }

        let declaration = ["dim", "static", "const"]
            .into_iter()
            .find_map(|keyword| strip_word(statement, keyword));
        if let Some(rest) = declaration {
            for name in declaration_names(rest) {
                model.variables.push(OutlineSymbol {
                    name,
                    detail: "变量".to_string(),
                    scope: current_scope(&scopes),
                    line: line_number,
                });
            }
            continue;
        }

        if had_visibility && !statement.is_empty() {
            for name in declaration_names(statement) {
                model.variables.push(OutlineSymbol {
                    name,
                    detail: "变量".to_string(),
                    scope: current_scope(&scopes),
                    line: line_number,
                });
            }
            continue;
        }

        if let Some(rest) = strip_word(statement, "for") {
            let rest = strip_word(rest.trim_start(), "each").unwrap_or(rest.trim_start());
            if let Some(name) = first_identifier(rest) {
                model.variables.push(OutlineSymbol {
                    name,
                    detail: "循环变量".to_string(),
                    scope: current_scope(&scopes),
                    line: line_number,
                });
            }
        }
    }

    sort_symbols(&mut model);
    model
}

fn declaration_names(text: &str) -> Vec<String> {
    split_declarations(text)
        .into_iter()
        .filter_map(first_identifier)
        .collect()
}

/// 记录一个具名条目，并把它压入作用域栈（类里的过程拼成 `类名.过程名`）。
fn enter_named_scope(
    scopes: &mut Vec<String>,
    symbols: &mut Vec<OutlineSymbol>,
    text: &str,
    detail: &str,
    line: usize,
) {
    let Some(name) = first_identifier(text) else {
        return;
    };
    let scope = match scopes.last() {
        Some(parent) => format!("{parent}.{name}"),
        None => name.clone(),
    };
    symbols.push(OutlineSymbol {
        name,
        detail: detail.to_string(),
        line,
        ..Default::default()
    });
    scopes.push(scope);
}

fn current_scope(scopes: &[String]) -> String {
    scopes.last().cloned().unwrap_or_else(|| "模块".to_string())
}

pub(crate) fn split_declarations(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut chars = text.char_indices().peekable();

    while let Some((byte, ch)) = chars.next() {
        match ch {
            '"' if in_string && chars.peek().is_some_and(|(_, next)| *next == '"') => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '(' if !in_string => depth += 1,
            ')' if !in_string => depth = depth.saturating_sub(1),
            ',' if !in_string && depth == 0 => {
                parts.push(text[start..byte].trim());
                start = byte + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(text[start..].trim());
    parts
}

pub(crate) fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut chars = line.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' if in_string && chars.peek().is_some_and(|(_, next)| *next == '"') => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '\'' if !in_string => return &line[..index],
            _ => {}
        }
    }
    line
}

pub(crate) fn split_word(text: &str) -> Option<(&str, &str)> {
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (end > 0).then(|| (&text[..end], &text[end..]))
}

pub(crate) fn strip_word<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let head = text.get(..word.len())?;
    if !head.eq_ignore_ascii_case(word) {
        return None;
    }
    let rest = &text[word.len()..];
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest)
    } else {
        None
    }
}

pub(crate) fn matches_ignore_ascii_case(value: &str, options: &[&str]) -> bool {
    options.iter().any(|option| value.eq_ignore_ascii_case(option))
}

pub(crate) fn first_identifier(text: &str) -> Option<String> {
    let text = text.trim_start();
    let end = text
        .char_indices()
        .find_map(|(index, ch)| {
            (!(ch.is_alphanumeric() || ch == '_')).then_some(index)
        })
        .unwrap_or(text.len());
    (end > 0).then(|| text[..end].to_string())
}

/// 调试会话在侧栏里的状态：暂停时给「变量」里的每个变量补一行值与类型。
#[derive(Clone, Copy)]
pub struct DebugView<'a> {
    pub paused: bool,
    /// 暂停时的行号与变量；还没有快照时为 `None`。
    pub snapshot: Option<&'a DebugSnapshot>,
}

impl<'a> DebugView<'a> {
    /// 这个变量当前的值与类型；不在当前作用域里则返回 `None`。
    fn variable_of(&self, symbol: &OutlineSymbol) -> Option<&'a DebugVariable> {
        if !self.paused {
            return None;
        }
        let snapshot = self.snapshot?;
        let module = symbol.scope.is_empty() || symbol.scope == "模块";
        snapshot.variables.iter().find(|variable| {
            variable.name.eq_ignore_ascii_case(&symbol.name)
                && if module {
                    variable.scope == "模块"
                } else {
                    variable.scope != "模块" && scope_matches(&symbol.scope, &variable.scope)
                }
        })
    }
}

/// 在当前模型里找 `name` 的定义。
///
/// `line` 是光标所在行（1 基）：优先选当前过程里的局部变量/参数，其次是模块变量，
/// 最后才是过程、API 声明、类型、类、枚举。`p.Bump` 这种成员调用会退而匹配
/// `类名.Bump`。
pub fn definition<'a>(model: &'a OutlineModel, name: &str, line: usize) -> Option<&'a OutlineSymbol> {
    let matches = |symbol: &&OutlineSymbol| symbol.name.eq_ignore_ascii_case(name);
    let in_current_scope =
        |symbol: &&OutlineSymbol| matches(symbol) && Some(symbol.scope.as_str()) == enclosing_scope(model, line).as_deref();

    if let Some(symbol) = model.variables.iter().find(in_current_scope) {
        return Some(symbol);
    }
    if let Some(symbol) = model
        .variables
        .iter()
        .find(|symbol| matches(symbol) && symbol.scope == "模块")
    {
        return Some(symbol);
    }
    if let Some(symbol) = model.variables.iter().find(|symbol| matches(symbol)) {
        return Some(symbol);
    }

    let named = model
        .procedures
        .iter()
        .chain(&model.declares)
        .chain(&model.types)
        .chain(&model.classes)
        .chain(&model.enums)
        .find(|symbol| matches(symbol));
    if named.is_some() {
        return named;
    }

    // `p.Bump`：大纲里类方法写作 `Box.Bump`，按后缀再找一次。
    let suffix = format!(".{name}");
    model
        .procedures
        .iter()
        .find(|symbol| symbol.name.to_lowercase().ends_with(&suffix.to_lowercase()))
}

/// 找出 `name` 在源码里出现的每一处（整词、忽略大小写，注释里的不算）。
///
/// 返回 `(行号, 该行去掉首尾空白后的文本)`，供输出面板列出、点击跳转。
pub fn references(source: &str, name: &str) -> Vec<(usize, String)> {
    if name.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (index, raw) in source.lines().enumerate() {
        if contains_word(strip_comment(raw), name) {
            found.push((index + 1, raw.trim().to_string()));
        }
    }
    found
}

/// `text` 里是否有一处完整的 `name`（前后不能是字母数字或 `_`）。
fn contains_word(text: &str, name: &str) -> bool {
    let text = text.to_lowercase();
    let target = name.to_lowercase();
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';

    let mut start = 0;
    while let Some(offset) = text[start..].find(&target) {
        let at = start + offset;
        let end = at + target.len();
        let before_ok = text[..at].chars().next_back().is_none_or(|ch| !is_word(ch));
        let after_ok = text[end..].chars().next().is_none_or(|ch| !is_word(ch));
        if before_ok && after_ok {
            return true;
        }
        start = end;
    }
    false
}

/// 「转到行」里输入的内容：纯数字行号，或者要跳转的符号名。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GotoTarget {
    Line(usize),
    Symbol(String),
}

/// 解析「转到行 / 符号」的输入；空输入返回 `None`。
pub fn goto_target(text: &str) -> Option<GotoTarget> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(line) = text.parse::<usize>() {
        return (line > 0).then_some(GotoTarget::Line(line));
    }
    Some(GotoTarget::Symbol(text.to_string()))
}

/// 光标行所属的过程名（开始行不晚于 `line` 的最后一个过程）。
fn enclosing_scope(model: &OutlineModel, line: usize) -> Option<String> {
    model
        .procedures
        .iter()
        .filter(|symbol| symbol.line <= line)
        .max_by_key(|symbol| symbol.line)
        .map(|symbol| symbol.name.clone())
}

/// 大纲里的类方法写作 `类名.方法名`，运行时记录的过程名只有方法名。
fn scope_matches(symbol_scope: &str, variable_scope: &str) -> bool {
    symbol_scope.eq_ignore_ascii_case(variable_scope)
        || symbol_scope
            .rsplit_once('.')
            .is_some_and(|(_, method)| method.eq_ignore_ascii_case(variable_scope))
}

/// 大纲的渲染参数。
pub struct OutlineRender<'a> {
    /// Dock 内由容器决定宽度时为 `None`。
    pub width: Option<f32>,
    pub palette: Palette,
    pub model: &'a OutlineModel,
    pub collapsed_sections: &'a HashSet<OutlineSection>,
    /// 「变量」里被人工展开的作用域分组；不在集合里的分组默认收起。
    pub expanded_scopes: &'a HashSet<String>,
    pub vb_supported: bool,
    /// 正在进行的调试会话；暂停时把变量值写进「变量」分组。
    pub debug: Option<DebugView<'a>>,
    /// 「监视」一节里的表达式与暂停时的求值结果。
    pub watches: &'a [WatchEntry],
    /// 「断点」一节里的当前文件断点。
    pub breakpoints: &'a [BreakpointEntry],
    /// 过滤框内容；空串表示不过滤。
    pub filter: &'a str,
    pub toggle_section: ToggleSectionAction,
    pub toggle_scope: ToggleScopeAction,
    pub close_panel: ClosePanelAction,
    pub go_to_line: GoToLineAction,
}

/// 各分区共用的调色板与回调。
struct SectionContext<'a> {
    palette: Palette,
    debug: Option<DebugView<'a>>,
    expanded_scopes: &'a HashSet<String>,
    watches: &'a [WatchEntry],
    breakpoints: &'a [BreakpointEntry],
    /// 正在过滤：符号分区强制展开、空分区隐藏。
    filtering: bool,
    toggle_section: ToggleSectionAction,
    toggle_scope: ToggleScopeAction,
    go_to_line: GoToLineAction,
}

pub type DragStartAction = Rc<dyn Fn(&mut App)>;

pub fn render(
    props: OutlineRender<'_>,
    show_header: bool,
    drag: Option<DragPanel>,
    on_drag_start: Option<DragStartAction>,
    filter_input: Option<AnyElement>,
) -> AnyElement {
    let OutlineRender {
        width,
        palette,
        model,
        collapsed_sections,
        expanded_scopes,
        vb_supported,
        debug,
        watches,
        breakpoints,
        filter,
        toggle_section,
        toggle_scope,
        close_panel,
        go_to_line,
    } = props;
    let filtering = !filter.trim().is_empty();
    let context = SectionContext {
        palette,
        debug,
        expanded_scopes,
        watches,
        breakpoints,
        filtering,
        toggle_section,
        toggle_scope,
        go_to_line,
    };

    let sections = OutlineSection::ALL.into_iter().filter_map(|section| {
        let collapsed = collapsed_sections.contains(&section);
        match section {
            OutlineSection::Watch => Some(render_section(
                section,
                &[],
                watches.len(),
                collapsed,
                &context,
            )),
            OutlineSection::Breakpoints => Some(render_section(
                section,
                &[],
                breakpoints.len(),
                collapsed,
                &context,
            )),
            _ => {
                let symbols = section.symbols(model);
                if filtering {
                    let filtered: Vec<OutlineSymbol> = filter_symbols(symbols, filter)
                        .into_iter()
                        .cloned()
                        .collect();
                    if filtered.is_empty() {
                        return None;
                    }
                    // 过滤时强制展开，否则匹配项还藏在折叠的分区里。
                    Some(render_section(section, &filtered, filtered.len(), false, &context))
                } else {
                    Some(render_section(
                        section,
                        symbols,
                        symbols.len(),
                        collapsed,
                        &context,
                    ))
                }
            }
        }
    });

    let body = if vb_supported {
        v_flex().children(sections).into_any_element()
    } else {
        div()
            .p(px(12.0))
            .text_xs()
            .text_color(rgb(palette.muted_text))
            .child("代码大纲目前支持 VB 脚本")
            .into_any_element()
    };

    let mut panel = v_flex()
        .h_full()
        .bg(rgb(palette.sidebar_bg))
        .rounded(px(ui_theme::PANE_RADIUS))
        .border_1()
        .border_color(rgba(palette.border))
        .overflow_hidden();
    panel = match width {
        Some(width) => panel.w(px(width)).flex_shrink_0(),
        None => panel.w_full(),
    };

    let title = div()
        .id("outline-title")
        .flex_1()
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .child("代码大纲");
    let title = if let Some(drag) = drag {
        title.on_drag(drag, move |drag, offset, _, cx| {
            cx.stop_propagation();
            if let Some(on_drag_start) = on_drag_start.clone() {
                on_drag_start(cx);
            }
            drag.set_drag_offset(offset);
            drag.set_preview_size(size(px(96.0), px(30.0)));
            cx.new(|_| PanelDragPreview::new("代码大纲"))
        })
    } else {
        title
    };

    panel = panel.when(show_header, |this| {
        this.child(
            h_flex()
                .h(px(ui_theme::EXPLORER_HEADER_HEIGHT))
                .px(px(16.0))
                .items_center()
                .justify_between()
                .text_xs()
                .text_color(rgb(palette.sidebar_text))
                .child(title)
                .child(close_button(close_panel, palette)),
        )
    });
    if let Some(filter_input) = filter_input {
        panel = panel.child(filter_input);
    }
    panel
        .child(
            div()
                .id("outline-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .bg(rgb(palette.panel_bg))
                .child(body),
        )
        .into_any_element()
}

fn render_section(
    section: OutlineSection,
    symbols: &[OutlineSymbol],
    count: usize,
    collapsed: bool,
    context: &SectionContext<'_>,
) -> AnyElement {
    let palette = context.palette;
    let mut group = v_flex().w_full().child(section_header(
        section,
        count,
        collapsed,
        palette,
        context.toggle_section.clone(),
    ));

    if !collapsed {
        if section == OutlineSection::Watch {
            group = render_watch_rows(group, context);
        } else if section == OutlineSection::Breakpoints {
            group = render_breakpoint_rows(group, context);
        } else if symbols.is_empty() {
            group = group.child(empty_row(24.0, palette));
        } else if section == OutlineSection::Variables {
            // 变量再按作用域分一层：每个作用域一个小标题，下面挂它的变量。
            for (scope_index, (scope, items)) in group_by_scope(symbols).into_iter().enumerate() {
                // 过滤时作用域分组强制展开。
                let scope_collapsed =
                    !context.filtering && !context.expanded_scopes.contains(scope);
                group = group.child(scope_header(
                    scope,
                    items.len(),
                    scope_collapsed,
                    palette,
                    context.toggle_scope.clone(),
                ));
                if scope_collapsed {
                    continue;
                }
                for (index, symbol) in items.into_iter().enumerate() {
                    let variable = context
                        .debug
                        .and_then(|debug| debug.variable_of(symbol));
                    group = group.child(symbol_row(
                        format!("outline-symbol-{section:?}-{scope_index}-{index}"),
                        symbol,
                        32.0,
                        palette,
                        context.go_to_line.clone(),
                    ));
                    // 暂停时在每个变量下面补一行「值 + 类型」，始终展开、不折叠。
                    if let Some(variable) = variable {
                        group = group.child(value_row(
                            format!("outline-value-{section:?}-{scope_index}-{index}"),
                            variable,
                            32.0,
                            palette,
                        ));
                    }
                }
            }
        } else {
            for (index, symbol) in symbols.iter().enumerate() {
                group = group.child(symbol_row(
                    format!("outline-symbol-{section:?}-{index}"),
                    symbol,
                    24.0,
                    palette,
                    context.go_to_line.clone(),
                ));
            }
        }
    }

    group.into_any_element()
}

/// 标题栏右侧的关闭按钮。
fn close_button(close_panel: ClosePanelAction, palette: Palette) -> impl IntoElement {
    div()
        .id("outline-close-panel")
        .flex_shrink_0()
        .size(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(rgba(palette.hover)))
        .child(
            Icon::new(IconName::PanelLeftClose)
                .xsmall()
                .text_color(rgb(palette.sidebar_text)),
        )
        .on_click(move |_, window, cx| close_panel(window, cx))
}

fn section_header(
    section: OutlineSection,
    count: usize,
    collapsed: bool,
    palette: Palette,
    toggle_section: ToggleSectionAction,
) -> impl IntoElement {
    h_flex()
        .id(format!("outline-section-{section:?}"))
        .w_full()
        .h(px(28.0))
        .px(px(8.0))
        .gap_1()
        .items_center()
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(rgba(palette.hover)))
        .on_click(move |_, window, cx| toggle_section(section, window, cx))
        .child(
            Icon::new(if collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronDown
            })
            .xsmall()
            .text_color(rgb(palette.muted_text)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(rgb(palette.sidebar_text))
                .child(section.title()),
        )
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(count.to_string()),
        )
}

fn scope_header(
    scope: &str,
    count: usize,
    collapsed: bool,
    palette: Palette,
    toggle_scope: ToggleScopeAction,
) -> impl IntoElement {
    let key = scope.to_string();
    h_flex()
        .id(format!("outline-scope-{scope}"))
        .w_full()
        .h(px(22.0))
        .pl(px(16.0))
        .pr(px(8.0))
        .gap_1()
        .items_center()
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(rgba(palette.hover)))
        .on_click(move |_, window, cx| toggle_scope(key.clone(), window, cx))
        .child(
            Icon::new(if collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronDown
            })
            .xsmall()
            .text_color(rgb(palette.muted_text)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(scope.to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(count.to_string()),
        )
}

/// 「断点」一节的内容：每条断点一行（行号 + 源码），条件断点再挂一行条件。
fn render_breakpoint_rows(mut group: Div, context: &SectionContext<'_>) -> Div {
    let palette = context.palette;
    if context.breakpoints.is_empty() {
        return group.child(
            div()
                .pl(px(24.0))
                .pr(px(8.0))
                .py(px(3.0))
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child("（无，按 F9 在行号栏切换断点）"),
        );
    }

    for (index, entry) in context.breakpoints.iter().enumerate() {
        let line = entry.line;
        let go_to_line = context.go_to_line.clone();
        let conditional = entry.condition.is_some();
        group = group.child(
            h_flex()
                .id(format!("outline-breakpoint-{index}"))
                .w_full()
                .min_h(px(24.0))
                .pl(px(24.0))
                .pr(px(8.0))
                .gap_2()
                .items_center()
                .cursor(CursorStyle::PointingHand)
                .hover(move |this| this.bg(rgba(palette.hover)))
                .on_click(move |_, window, cx| go_to_line(line, window, cx))
                .child(
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(rgb(if conditional {
                            palette.warning
                        } else {
                            palette.error
                        }))
                        .child(format!("● {}", entry.line)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.sidebar_text))
                        .child(if entry.text.is_empty() {
                            "（空行）".to_string()
                        } else {
                            entry.text.clone()
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(rgb(if conditional {
                            palette.warning
                        } else {
                            palette.muted_text
                        }))
                        .child(if conditional { "条件" } else { "" }),
                ),
        );
        if let Some(condition) = &entry.condition {
            group = group.child(
                div()
                    .id(format!("outline-breakpoint-condition-{index}"))
                    .w_full()
                    .min_h(px(20.0))
                    .pl(px(40.0))
                    .pr(px(8.0))
                    .truncate()
                    .text_xs()
                    .text_color(rgb(palette.warning))
                    .child(format!("条件：{condition}")),
            );
        }
    }
    group
}

/// 「监视」一节的内容：每条表达式一行，下面再挂一行求值结果。
fn render_watch_rows(mut group: Div, context: &SectionContext<'_>) -> Div {
    let palette = context.palette;
    if context.watches.is_empty() {
        return group.child(
            div()
                .pl(px(24.0))
                .pr(px(8.0))
                .py(px(3.0))
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child("从「运行 → 添加监视…」添加表达式"),
        );
    }

    for (index, entry) in context.watches.iter().enumerate() {
        group = group.child(
            h_flex()
                .id(format!("outline-watch-{index}"))
                .w_full()
                .min_h(px(24.0))
                .pl(px(24.0))
                .pr(px(8.0))
                .gap_1()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.sidebar_text))
                        .child(entry.expression.clone()),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(rgb(palette.muted_text))
                        .child(match &entry.state {
                            WatchState::Inactive => "暂停时求值".to_string(),
                            WatchState::Pending => "求值中…".to_string(),
                            WatchState::Ready { ty, .. } => ty.clone(),
                            WatchState::Failed(_) => "错误".to_string(),
                        }),
                ),
        );
        match &entry.state {
            WatchState::Ready { value, .. } => {
                group = group.child(
                    div()
                        .id(format!("outline-watch-value-{index}"))
                        .w_full()
                        .min_h(px(20.0))
                        .pl(px(32.0))
                        .pr(px(8.0))
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted_text))
                        .child(format!("= {value}")),
                );
            }
            WatchState::Failed(message) => {
                group = group.child(
                    div()
                        .id(format!("outline-watch-error-{index}"))
                        .w_full()
                        .pl(px(32.0))
                        .pr(px(8.0))
                        .text_xs()
                        .text_color(rgb(palette.error))
                        .child(message.clone()),
                );
            }
            WatchState::Inactive | WatchState::Pending => {}
        }
    }
    group
}

fn symbol_row(
    id: String,
    symbol: &OutlineSymbol,
    indent: f32,
    palette: Palette,
    go_to_line: GoToLineAction,
) -> impl IntoElement {
    let line = symbol.line;
    h_flex()
        .id(id)
        .w_full()
        .min_h(px(24.0))
        .pl(px(indent))
        .pr(px(8.0))
        .gap_1()
        .items_center()
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(rgba(palette.hover)))
        .on_click(move |_, window, cx| go_to_line(line, window, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(rgb(palette.sidebar_text))
                .child(symbol.name.clone()),
        )
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(symbol.detail.clone()),
        )
}

/// 调试暂停时变量下方的那一行：值与声明类型，用淡色（禁用态）显示。
fn value_row(
    id: String,
    variable: &DebugVariable,
    indent: f32,
    palette: Palette,
) -> impl IntoElement {
    h_flex()
        .id(id)
        .w_full()
        .min_h(px(20.0))
        .pl(px(indent + 8.0))
        .pr(px(8.0))
        .gap_1()
        .items_center()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(format!("值：{}", variable.value)),
        )
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_xs()
                .text_color(rgb(palette.muted_text))
                .child(format!("类型：{}", variable.ty)),
        )
}

fn empty_row(indent: f32, palette: Palette) -> impl IntoElement {
    div()
        .pl(px(indent))
        .pr(px(8.0))
        .py(px(3.0))
        .text_xs()
        .text_color(rgb(palette.muted_text))
        .child("（无）")
}
