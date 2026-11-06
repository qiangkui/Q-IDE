//! VB 脚本的代码补全。
//!
//! 补全弹窗的显示、翻页与确认由编辑器自己完成，这里只负责给候选：
//! 以光标前单词为前缀的关键字、内置函数，以及当前脚本里声明过的
//! 变量/过程/类型/类/枚举。候选顺序按字母排列，`detail` 说明候选的种类。

use anyhow::Result;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui_kit::base::input::{Rope, RopeExt as _};
use gpui_kit::component::input::CompletionProvider;
use gpui_kit::{App, Task, Window};
use lsp_types::{CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse};

use crate::interpreter::{builtins, lexer};
use crate::outline::{
    OutlineModel, first_identifier, matches_ignore_ascii_case, split_declarations, split_word,
    strip_comment, strip_word,
};

/// 一次最多给出的候选数量，避免空前缀时列表过长。
const MAX_ITEMS: usize = 200;

/// `As` 后面可以写的声明类型；与解释器 `parse_declared_type` 接受的类型一致。
///
/// `Collection` / `Dictionary` 在声明里按对象处理，因此也算可选类型。
const DATA_TYPES: &[&str] = &[
    "Variant",
    "Byte",
    "Integer",
    "Long",
    "LongLong",
    "LongPtr",
    "Single",
    "Double",
    "Boolean",
    "String",
    "Object",
    "Any",
    "Collection",
    "Dictionary",
];

/// 超过这个字节数的脚本不再提取文档里的名字（解析一遍太慢）。
const MAX_SYMBOL_BYTES: usize = 400_000;

/// 文档里的一个可补全名字。
#[derive(Clone, Debug)]
pub struct SymbolItem {
    pub label: String,
    pub detail: String,
    pub kind: CompletionItemKind,
}

/// VB 脚本的补全提供者；缓存一份按源码算出来的名字表。
#[derive(Default)]
pub struct VbCompletionProvider {
    cache: RefCell<Option<(u64, Vec<SymbolItem>)>>,
}

/// 交给编辑器的补全提供者。
pub fn provider() -> Rc<dyn CompletionProvider> {
    Rc::new(VbCompletionProvider::default())
}

impl CompletionProvider for VbCompletionProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        // 注释和字符串里没有代码补全。
        let items = if is_code(text, offset) {
            let source = text.to_string();
            match member_context(text, offset) {
                // `p.` / `p.pr`：只给成员，避免关键字混进来。
                Some((base, prefix)) => {
                    let line = text.offset_to_position(offset).line as usize + 1;
                    member_candidates(&source, &base, &prefix, line)
                }
                None => {
                    let query = word_prefix(text, offset);
                    suggestions_with_symbols(&query, &self.symbols(&source))
                }
            }
        } else {
            Vec::new()
        };
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        // 输入单词字符时自动弹出候选；空格、符号、回车不弹。
        new_text
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
    }
}

impl VbCompletionProvider {
    /// 源码里的名字表；同一份源码只解析一次。
    fn symbols(&self, source: &str) -> Vec<SymbolItem> {
        if source.len() > MAX_SYMBOL_BYTES {
            return Vec::new();
        }
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        let key = hasher.finish();
        if let Some((cached, items)) = self.cache.borrow().as_ref()
            && *cached == key
        {
            return items.clone();
        }
        let items = collect_symbols(source);
        *self.cache.borrow_mut() = Some((key, items.clone()));
        items
    }
}

/// 前缀为 `query`（大小写不敏感）的关键字与内置函数候选；测试用来验证候选表。
#[cfg(test)]
pub fn suggestions(query: &str) -> Vec<CompletionItem> {
    suggestions_with_symbols(query, &[])
}

/// 前缀为 `query` 的候选：文档里的名字优先，其次是关键字与内置函数。
#[cfg(test)]
pub fn suggestions_for(source: &str, query: &str) -> Vec<CompletionItem> {
    suggestions_with_symbols(query, &collect_symbols(source))
}

/// 前缀为 `query`（大小写不敏感）的候选；前缀为空时给出全部。
///
/// 同名的候选只保留一个，文档里声明的名字优先（变量可以遮蔽同名内置函数）。
/// 类型名与内置函数同名时两个候选都保留（`String` 既是类型也是内置函数）。
pub fn suggestions_with_symbols(query: &str, symbols: &[SymbolItem]) -> Vec<CompletionItem> {
    let query = query.to_lowercase();

    let mut seen = HashSet::new();
    // 文档里的名字与关键字会遮蔽同名类型；内置函数不会。
    let mut declared = HashSet::new();
    let mut items = Vec::new();
    for symbol in symbols {
        let key = symbol.label.to_lowercase();
        if key.starts_with(&query) && seen.insert(key.clone()) {
            declared.insert(key);
            items.push(item(&symbol.label, symbol.kind, &symbol.detail));
        }
    }
    for name in lexer::KEYWORDS {
        let key = name.to_lowercase();
        if key.starts_with(&query) && seen.insert(key.clone()) {
            declared.insert(key);
            let label = lexer::keyword_display(name).unwrap_or(name);
            items.push(item(label, CompletionItemKind::KEYWORD, "关键字"));
        }
    }
    for name in builtins::names() {
        if name.starts_with(&query) && seen.insert((*name).to_string()) {
            items.push(item(name, CompletionItemKind::FUNCTION, "内置函数"));
        }
    }
    for name in DATA_TYPES {
        let key = name.to_lowercase();
        if key.starts_with(&query) && !declared.contains(&key) {
            items.push(item(
                name,
                CompletionItemKind::TYPE_PARAMETER,
                "数据类型",
            ));
        }
    }

    sort_items(&mut items);
    items.truncate(MAX_ITEMS);
    items
}

fn sort_items(items: &mut [CompletionItem]) {
    items.sort_by(|left, right| {
        left.label
            .to_lowercase()
            .cmp(&right.label.to_lowercase())
            .then(left.detail.cmp(&right.detail))
    });
}

// ---------- 成员补全（`p.`） ----------

/// 光标处是不是 `base.` / `base.prefix`；返回 (基名, 已输入的前缀)。
pub fn member_context(text: &Rope, offset: usize) -> Option<(String, String)> {
    let prefix = word_prefix(text, offset);
    let dot = offset.checked_sub(prefix.len())?;
    if dot == 0 {
        return None;
    }
    if text.chars_at(dot).reversed().next() != Some('.') {
        return None;
    }
    let mut start = dot - 1;
    for ch in text.chars_at(start).reversed() {
        if ch.is_alphanumeric() || ch == '_' {
            start -= ch.len_utf8();
        } else {
            break;
        }
    }
    let base = text.slice(start..dot - 1).to_string();
    (!base.is_empty()).then_some((base, prefix))
}

/// `base.prefix` 的成员候选：UDT 字段、类字段/方法、集合与字典的方法。
///
/// 只看文本，不依赖整份脚本能否解析，所以边写边补全也能用。
pub fn member_candidates(
    source: &str,
    base: &str,
    prefix: &str,
    cursor_line: usize,
) -> Vec<CompletionItem> {
    let symbols = if base.eq_ignore_ascii_case("me") {
        enclosing_class(source, cursor_line)
            .and_then(|class| class_members(source, &class))
            .unwrap_or_default()
    } else {
        match find_base_kind(source, base) {
            Some(BaseKind::NewCollection) => collection_members(),
            Some(BaseKind::NewDictionary) => dictionary_members(),
            Some(BaseKind::Named(type_name)) => type_fields(source, &type_name)
                .or_else(|| class_members(source, &type_name))
                .unwrap_or_default(),
            None => Vec::new(),
        }
    };

    let query = prefix.to_lowercase();
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for symbol in &symbols {
        let key = symbol.label.to_lowercase();
        if key.starts_with(&query) && seen.insert(key) {
            items.push(item(&symbol.label, symbol.kind, &symbol.detail));
        }
    }
    sort_items(&mut items);
    items.truncate(MAX_ITEMS);
    items
}

/// `base` 的声明种类。
enum BaseKind {
    /// `Dim x As 类型名`（UDT 或类）。
    Named(String),
    NewCollection,
    NewDictionary,
}

/// 找出 `base` 的声明：`Dim`/`Private` 字段、过程参数、`As New …`。
fn find_base_kind(source: &str, base: &str) -> Option<BaseKind> {
    for raw in source.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        let mut statement = line;
        let mut declared = false;
        while let Some((word, rest)) = split_word(statement) {
            if matches_ignore_ascii_case(
                word,
                &["dim", "static", "const", "public", "private", "friend", "global"],
            ) {
                declared = true;
                statement = rest.trim_start();
            } else {
                break;
            }
        }

        // 过程/属性签名：只看参数表。
        if let Some(rest) = strip_word(statement, "sub")
            .or_else(|| strip_word(statement, "function"))
            .or_else(|| strip_word(statement, "property"))
        {
            if let Some(kind) = parameter_kind(rest, base) {
                return Some(kind);
            }
            continue;
        }

        if !declared {
            continue;
        }
        for part in split_declarations(statement) {
            if let Some(kind) = declarator_kind(part, base) {
                return Some(kind);
            }
        }
    }
    None
}

/// `(a As A, base As B) As R` 里 `base` 的类型。
fn parameter_kind(text: &str, base: &str) -> Option<BaseKind> {
    let open = text.find('(')?;
    let close = text.find(')')?;
    let params = text.get(open + 1..close)?;
    split_declarations(params)
        .into_iter()
        .find_map(|part| declarator_kind(part, base))
}

/// `base(10) As Foo` / `ByVal base As New Dictionary` 这样的单个声明。
fn declarator_kind(part: &str, base: &str) -> Option<BaseKind> {
    let mut part = part.trim();
    while let Some((word, rest)) = split_word(part) {
        if matches_ignore_ascii_case(word, &["byval", "byref", "optional", "paramarray"]) {
            part = rest.trim_start();
        } else {
            break;
        }
    }

    let name = first_identifier(part)?;
    if !name.eq_ignore_ascii_case(base) {
        return None;
    }

    let mut rest = part[name.len()..].trim_start();
    // 跳过数组下标：`x(10) As Long`。
    if let Some(inner) = rest.strip_prefix('(')
        && let Some(close) = inner.find(')')
    {
        rest = inner[close + 1..].trim_start();
    }

    let after_as = strip_word(rest, "as")?.trim_start();
    if let Some(new_rest) = strip_word(after_as, "new") {
        let kind = first_identifier(new_rest).unwrap_or_default();
        return Some(match kind.to_ascii_lowercase().as_str() {
            "collection" => BaseKind::NewCollection,
            "dictionary" => BaseKind::NewDictionary,
            other => BaseKind::Named(other.to_string()),
        });
    }
    Some(BaseKind::Named(first_identifier(after_as)?))
}

/// `Type 名字 ... End Type` 的字段。
fn type_fields(source: &str, type_name: &str) -> Option<Vec<SymbolItem>> {
    let mut inside = false;
    let mut items = Vec::new();
    for raw in source.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if !inside {
            if let Some(rest) = strip_word(line, "type")
                && first_identifier(rest.trim_start())
                    .is_some_and(|name| name.eq_ignore_ascii_case(type_name))
            {
                inside = true;
            }
            continue;
        }
        if strip_word(line, "end").is_some_and(|rest| matches_ignore_ascii_case(rest.trim(), &["type"]))
        {
            break;
        }
        if let Some((name, detail)) = field_of(line) {
            items.push(SymbolItem {
                label: name,
                detail,
                kind: CompletionItemKind::VARIABLE,
            });
        }
    }
    inside.then_some(items)
}

/// `Class 名字 ... End Class` 的字段与方法（跳过方法体）。
fn class_members(source: &str, class_name: &str) -> Option<Vec<SymbolItem>> {
    let mut inside = false;
    let mut in_method = false;
    let mut items = Vec::new();
    for raw in source.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if !inside {
            if let Some(rest) = strip_word(line, "class")
                && first_identifier(rest.trim_start())
                    .is_some_and(|name| name.eq_ignore_ascii_case(class_name))
            {
                inside = true;
            }
            continue;
        }

        if let Some(rest) = strip_word(line, "end") {
            let end = rest.trim();
            if matches_ignore_ascii_case(end, &["class"]) {
                break;
            }
            if matches_ignore_ascii_case(end, &["sub", "function", "property"]) {
                in_method = false;
            }
            continue;
        }
        if in_method {
            continue;
        }

        let mut statement = line;
        while let Some((word, rest)) = split_word(statement) {
            if matches_ignore_ascii_case(word, &["public", "private", "friend", "static"]) {
                statement = rest.trim_start();
            } else {
                break;
            }
        }

        if let Some(rest) = strip_word(statement, "sub").or_else(|| strip_word(statement, "function"))
        {
            if let Some(name) = first_identifier(rest) {
                items.push(method_item(name, "方法"));
            }
            in_method = true;
            continue;
        }
        if let Some(rest) = strip_word(statement, "property") {
            let rest = rest.trim_start();
            let (_, after_kind) = split_word(rest).unwrap_or(("", rest));
            if let Some(name) = first_identifier(after_kind) {
                items.push(method_item(name, "属性"));
            }
            in_method = true;
            continue;
        }
        if let Some((name, detail)) = field_of(statement) {
            items.push(SymbolItem {
                label: name,
                detail,
                kind: CompletionItemKind::VARIABLE,
            });
        }
    }
    inside.then_some(items)
}

/// `X As Long` / `Name As String * 8` / `Items(2) As Long` 这样的字段。
fn field_of(statement: &str) -> Option<(String, String)> {
    let name = first_identifier(statement)?;
    let mut rest = statement[name.len()..].trim_start();
    if let Some(inner) = rest.strip_prefix('(')
        && let Some(close) = inner.find(')')
    {
        rest = inner[close + 1..].trim_start();
    }
    let detail = strip_word(rest, "as")
        .and_then(|after| first_identifier(after.trim_start()))
        .map_or_else(|| "字段".to_string(), |ty| format!("字段 · {ty}"));
    Some((name, detail))
}

fn method_item(name: String, detail: &str) -> SymbolItem {
    SymbolItem {
        label: name,
        detail: detail.to_string(),
        kind: CompletionItemKind::METHOD,
    }
}

fn collection_members() -> Vec<SymbolItem> {
    ["Add", "Remove", "Item", "Count"]
        .into_iter()
        .map(|name| method_item(name.to_string(), "Collection 方法"))
        .collect()
}

fn dictionary_members() -> Vec<SymbolItem> {
    ["Add", "Remove", "Exists", "Keys", "Items", "Count"]
        .into_iter()
        .map(|name| method_item(name.to_string(), "Dictionary 方法"))
        .collect()
}

/// 光标行所在的类（`Me.` 用）；扫描到光标行为止。
fn enclosing_class(source: &str, cursor_line: usize) -> Option<String> {
    let mut current = None;
    for raw in source.lines().take(cursor_line) {
        let line = strip_comment(raw).trim();
        if let Some(rest) = strip_word(line, "class") {
            current = first_identifier(rest.trim_start());
        } else if strip_word(line, "end")
            .is_some_and(|rest| matches_ignore_ascii_case(rest.trim(), &["class"]))
        {
            current = None;
        }
    }
    current
}

/// 从源码里取出可以补全的名字（走代码大纲的解析，语法不完整时也会尽量给）。
fn collect_symbols(source: &str) -> Vec<SymbolItem> {
    let model = OutlineModel::from_source(source);
    let mut items = Vec::new();

    for symbol in &model.procedures {
        items.push(SymbolItem {
            label: symbol.name.clone(),
            detail: symbol.detail.clone(),
            kind: CompletionItemKind::FUNCTION,
        });
    }
    for symbol in &model.variables {
        let detail = if symbol.scope.is_empty() || symbol.scope == "模块" {
            symbol.detail.clone()
        } else {
            format!("{} · {}", symbol.detail, symbol.scope)
        };
        items.push(SymbolItem {
            label: symbol.name.clone(),
            detail,
            kind: CompletionItemKind::VARIABLE,
        });
    }
    for symbol in &model.types {
        items.push(SymbolItem {
            label: symbol.name.clone(),
            detail: symbol.detail.clone(),
            kind: CompletionItemKind::STRUCT,
        });
    }
    for symbol in &model.classes {
        items.push(SymbolItem {
            label: symbol.name.clone(),
            detail: symbol.detail.clone(),
            kind: CompletionItemKind::CLASS,
        });
    }
    for symbol in &model.enums {
        items.push(SymbolItem {
            label: symbol.name.clone(),
            detail: symbol.detail.clone(),
            kind: CompletionItemKind::ENUM,
        });
    }
    for symbol in &model.declares {
        items.push(SymbolItem {
            label: symbol.name.clone(),
            detail: symbol.detail.clone(),
            kind: CompletionItemKind::FUNCTION,
        });
    }
    items.retain(|symbol| !symbol.label.is_empty());
    items
}

fn item(label: &str, kind: CompletionItemKind, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail.to_string()),
        ..Default::default()
    }
}

/// 光标前的标识符片段（不包含光标后面的部分）。
pub(crate) fn word_prefix(text: &Rope, offset: usize) -> String {
    let mut length = 0usize;
    for ch in text.chars_at(offset).reversed() {
        if ch.is_alphanumeric() || ch == '_' {
            length += ch.len_utf8();
        } else {
            break;
        }
    }
    text.slice(offset - length..offset).to_string()
}

/// 光标是否在普通代码位置：不在 `'` 注释里，也不在未闭合的字符串里。
pub(crate) fn is_code(text: &Rope, offset: usize) -> bool {
    let mut start = offset;
    for ch in text.chars_at(offset).reversed() {
        if ch == '\n' {
            break;
        }
        start -= ch.len_utf8();
    }

    let mut in_string = false;
    for ch in text.slice(start..offset).chars() {
        match ch {
            '"' => in_string = !in_string,
            '\'' if !in_string => return false,
            _ => {}
        }
    }
    !in_string
}
