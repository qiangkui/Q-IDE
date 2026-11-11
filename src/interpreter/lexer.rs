//! VB 词法分析。
//!
//! VB 的特点：关键字大小写不敏感、`'` 或 `Rem` 注释、`:` 可以作为语句分隔。
//! 行尾 `_` 表示续行、标识符可以带 `$ % & ! # @` 类型后缀。

use crate::interpreter::error::ParseError;

/// 关键字。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kw {
    Dim,
    As,
    If,
    Then,
    Else,
    ElseIf,
    End,
    For,
    To,
    Step,
    Next,
    Do,
    Loop,
    While,
    Until,
    Wend,
    Select,
    Case,
    Is,
    Exit,
    Sub,
    Function,
    Call,
    Let,
    Set,
    Print,
    Debug,
    Const,
    ByVal,
    ByRef,
    Optional,
    Declare,
    Lib,
    Alias,
    PtrSafe,
    Public,
    Private,
    Global,
    True,
    False,
    Nothing,
    Null,
    Empty,
    And,
    Or,
    Not,
    Xor,
    Mod,
    Stop,
    Return,
    ReDim,
    Preserve,
    Each,
    In,
    Erase,
    Option,
    Explicit,
    Static,
    Type,
    Class,
    TypeOf,
    WithEvents,
    RaiseEvent,
    With,
    On,
    Error,
    GoTo,
    Resume,
    AddressOf,
    Open,
    Close,
    Input,
    Output,
    Append,
    Property,
    Enum,
    ParamArray,
}

impl Kw {
    pub fn text(self) -> &'static str {
        match self {
            Self::Dim => "Dim",
            Self::As => "As",
            Self::If => "If",
            Self::Then => "Then",
            Self::Else => "Else",
            Self::ElseIf => "ElseIf",
            Self::End => "End",
            Self::For => "For",
            Self::To => "To",
            Self::Step => "Step",
            Self::Next => "Next",
            Self::Do => "Do",
            Self::Loop => "Loop",
            Self::While => "While",
            Self::Until => "Until",
            Self::Wend => "Wend",
            Self::Select => "Select",
            Self::Case => "Case",
            Self::Is => "Is",
            Self::Exit => "Exit",
            Self::Sub => "Sub",
            Self::Function => "Function",
            Self::Call => "Call",
            Self::Let => "Let",
            Self::Set => "Set",
            Self::Print => "Print",
            Self::Debug => "Debug",
            Self::Const => "Const",
            Self::ByVal => "ByVal",
            Self::ByRef => "ByRef",
            Self::Optional => "Optional",
            Self::Declare => "Declare",
            Self::Lib => "Lib",
            Self::Alias => "Alias",
            Self::PtrSafe => "PtrSafe",
            Self::Public => "Public",
            Self::Private => "Private",
            Self::Global => "Global",
            Self::True => "True",
            Self::False => "False",
            Self::Nothing => "Nothing",
            Self::Null => "Null",
            Self::Empty => "Empty",
            Self::And => "And",
            Self::Or => "Or",
            Self::Not => "Not",
            Self::Xor => "Xor",
            Self::Mod => "Mod",
            Self::Stop => "Stop",
            Self::Return => "Return",
            Self::ReDim => "ReDim",
            Self::Preserve => "Preserve",
            Self::Each => "Each",
            Self::In => "In",
            Self::Erase => "Erase",
            Self::Option => "Option",
            Self::Explicit => "Explicit",
            Self::Static => "Static",
            Self::Type => "Type",
            Self::Class => "Class",
            Self::TypeOf => "TypeOf",
            Self::WithEvents => "WithEvents",
            Self::RaiseEvent => "RaiseEvent",
            Self::With => "With",
            Self::On => "On",
            Self::Error => "Error",
            Self::GoTo => "GoTo",
            Self::Resume => "Resume",
            Self::AddressOf => "AddressOf",
            Self::Open => "Open",
            Self::Close => "Close",
            Self::Input => "Input",
            Self::Output => "Output",
            Self::Append => "Append",
            Self::Property => "Property",
            Self::Enum => "Enum",
            Self::ParamArray => "ParamArray",
        }
    }
}

/// 运算符与标点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Plus,
    Minus,
    Star,
    Slash,
    Backslash,
    Caret,
    Amp,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    LParen,
    RParen,
    /// `{`（数组字面量 `{1, 2, 3}` 的开头）。
    LBrace,
    /// `}`。
    RBrace,
    Comma,
    /// `;`（`Print` 的分隔符，不做补位）。
    Semicolon,
    Dot,
    /// `#`（文件号前缀）。
    Hash,
}

impl Op {
    pub fn text(self) -> &'static str {
        match self {
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::Backslash => "\\",
            Self::Caret => "^",
            Self::Amp => "&",
            Self::Eq => "=",
            Self::NotEq => "<>",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::LtEq => "<=",
            Self::GtEq => ">=",
            Self::LParen => "(",
            Self::RParen => ")",
            Self::LBrace => "{",
            Self::RBrace => "}",
            Self::Comma => ",",
            Self::Semicolon => ";",
            Self::Dot => ".",
            Self::Hash => "#",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    Kw(Kw),
    Op(Op),
    /// 换行，语句结束。
    Newline,
    /// `:`，也是语句分隔，但与换行不同（单行 `If` 靠它区分）。
    Colon,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    /// 标识符的原始拼写（关键字/符号为空）。`Declare` 依赖它保留导出名的正确大小写。
    pub text: String,
    /// 标识符的类型后缀（`$ % & ! # @`），用于 `Dim s$` 之类的声明。
    pub suffix: Option<char>,
}

impl Token {
    fn new(kind: TokenKind, line: usize) -> Self {
        Self {
            kind,
            line,
            text: String::new(),
            suffix: None,
        }
    }

    fn with_suffix(kind: TokenKind, line: usize, text: &str, suffix: Option<char>) -> Self {
        Self {
            kind,
            line,
            text: text.to_string(),
            suffix,
        }
    }

    /// 便于在错误信息里展示当前 token。
    pub fn describe(&self) -> String {
        match &self.kind {
            TokenKind::Ident(name) => format!("标识符 `{name}`"),
            TokenKind::Int(value) => format!("数字 `{value}`"),
            TokenKind::Float(value) => format!("数字 `{value}`"),
            TokenKind::Str(value) => format!("字符串 \"{value}\""),
            TokenKind::Kw(kw) => format!("关键字 `{}`", kw.text()),
            TokenKind::Op(op) => format!("符号 `{}`", op.text()),
            TokenKind::Newline => "换行".into(),
            TokenKind::Colon => "`:`".into(),
            TokenKind::Eof => "文件结尾".into(),
        }
    }
}

/// 全部关键字（小写），供编辑器补全使用。
///
/// 与 [`keyword`] 的匹配表保持一致；`keyword_list_covers_every_keyword` 测试会检查
/// 这里的每一项都能被词法器识别。
pub const KEYWORDS: &[&str] = &[
    "dim", "as", "if", "then", "else", "elseif", "end", "for", "to", "step", "next", "do",
    "loop", "while", "until", "wend", "select", "case", "is", "exit", "sub", "function", "call",
    "let", "set", "print", "debug", "const", "byval", "byref", "optional", "declare", "lib",
    "alias", "ptrsafe", "public", "private", "global", "true", "false", "nothing", "null",
    "empty", "and", "or", "not", "xor", "mod", "stop", "return", "redim", "preserve", "each",
    "in", "erase", "option", "explicit", "static", "type", "class", "typeof", "withevents",
    "raiseevent", "with", "on", "error", "goto", "resume", "addressof", "open", "close", "input",
    "output", "append", "property", "enum", "paramarray",
];

/// 判断一个词（小写）是否是关键字；编辑器高亮也用它。
pub fn is_keyword(word: &str) -> bool {
    keyword(word).is_some()
}

/// 关键字在编辑器里显示的写法，例如 `dim` → `Dim`、`elseif` → `ElseIf`。
pub fn keyword_display(word: &str) -> Option<&'static str> {
    keyword(word).map(Kw::text)
}

fn keyword(word: &str) -> Option<Kw> {
    Some(match word {
        "dim" => Kw::Dim,
        "as" => Kw::As,
        "if" => Kw::If,
        "then" => Kw::Then,
        "else" => Kw::Else,
        "elseif" => Kw::ElseIf,
        "end" => Kw::End,
        "for" => Kw::For,
        "to" => Kw::To,
        "step" => Kw::Step,
        "next" => Kw::Next,
        "do" => Kw::Do,
        "loop" => Kw::Loop,
        "while" => Kw::While,
        "until" => Kw::Until,
        "wend" => Kw::Wend,
        "select" => Kw::Select,
        "case" => Kw::Case,
        "is" => Kw::Is,
        "exit" => Kw::Exit,
        "sub" => Kw::Sub,
        "function" => Kw::Function,
        "call" => Kw::Call,
        "let" => Kw::Let,
        "set" => Kw::Set,
        "print" => Kw::Print,
        "debug" => Kw::Debug,
        "const" => Kw::Const,
        "byval" => Kw::ByVal,
        "byref" => Kw::ByRef,
        "optional" => Kw::Optional,
        "declare" => Kw::Declare,
        "lib" => Kw::Lib,
        "alias" => Kw::Alias,
        "ptrsafe" => Kw::PtrSafe,
        "public" => Kw::Public,
        "private" => Kw::Private,
        "global" => Kw::Global,
        "true" => Kw::True,
        "false" => Kw::False,
        "nothing" => Kw::Nothing,
        "null" => Kw::Null,
        "empty" => Kw::Empty,
        "and" => Kw::And,
        "or" => Kw::Or,
        "not" => Kw::Not,
        "xor" => Kw::Xor,
        "mod" => Kw::Mod,
        "stop" => Kw::Stop,
        "return" => Kw::Return,
        "redim" => Kw::ReDim,
        "preserve" => Kw::Preserve,
        "each" => Kw::Each,
        "in" => Kw::In,
        "erase" => Kw::Erase,
        "option" => Kw::Option,
        "explicit" => Kw::Explicit,
        "static" => Kw::Static,
        "type" => Kw::Type,
        "class" => Kw::Class,
        "typeof" => Kw::TypeOf,
        "withevents" => Kw::WithEvents,
        "raiseevent" => Kw::RaiseEvent,
        "with" => Kw::With,
        "on" => Kw::On,
        "error" => Kw::Error,
        "goto" => Kw::GoTo,
        "resume" => Kw::Resume,
        "addressof" => Kw::AddressOf,
        "open" => Kw::Open,
        "close" => Kw::Close,
        "input" => Kw::Input,
        "output" => Kw::Output,
        "append" => Kw::Append,
        "property" => Kw::Property,
        "enum" => Kw::Enum,
        "paramarray" => Kw::ParamArray,
        _ => return None,
    })
}

/// 把源码切成 token 流。
pub fn tokenize(source: &str) -> Result<Vec<Token>, ParseError> {
    tokenize_mapped(source, &[])
}

/// 同 [`tokenize`]，但允许把物理行映射成逻辑行号（`#Include` 展开用）。
pub fn tokenize_mapped(source: &str, lines: &[usize]) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    let mut physical_line = 1usize;
    let mut line = logical_line(lines, physical_line);
    let mut at_statement_start = true;

    while index < chars.len() {
        let current = chars[index];
        match current {
            '\r' => index += 1,
            '\n' => {
                tokens.push(Token::new(TokenKind::Newline, line));
                physical_line += 1;
                line = logical_line(lines, physical_line);
                index += 1;
                at_statement_start = true;
            }
            ' ' | '\t' => index += 1,
            '\'' => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
            }
            ':' => {
                tokens.push(Token::new(TokenKind::Colon, line));
                index += 1;
                at_statement_start = true;
            }
            '_' if is_line_continuation(&chars, index) => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
                if index < chars.len() {
                    index += 1;
                    physical_line += 1;
                    line = logical_line(lines, physical_line);
                }
            }
            '"' => {
                let (text, next) = scan_string(&chars, index, line)?;
                tokens.push(Token::new(TokenKind::Str(text), line));
                index = next;
                at_statement_start = false;
            }
            '&' if matches!(chars.get(index + 1), Some('h' | 'H' | 'o' | 'O')) => {
                let (value, next) = scan_radix_number(&chars, index, line)?;
                tokens.push(Token::new(TokenKind::Int(value), line));
                index = next;
                at_statement_start = false;
            }
            c if c.is_ascii_digit() || (c == '.' && matches!(chars.get(index + 1), Some(d) if d.is_ascii_digit())) => {
                let (kind, next) = scan_number(&chars, index, line)?;
                tokens.push(Token::new(kind, line));
                index = next;
                at_statement_start = false;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = index;
                while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                    index += 1;
                }
                let word: String = chars[start..index].iter().collect();
                // 类型后缀，例如 `Left$`、`x%`，去掉后缀但保留名字。
                let mut suffix = None;
                if let Some(next) = chars.get(index).copied()
                    && matches!(next, '$' | '%' | '&' | '!' | '#' | '@')
                {
                    suffix = Some(next);
                    index += 1;
                }
                let lower = word.to_ascii_lowercase();
                if lower == "rem" && at_statement_start {
                    while index < chars.len() && chars[index] != '\n' {
                        index += 1;
                    }
                    continue;
                }
                match keyword(&lower) {
                    Some(kw) => tokens.push(Token::new(TokenKind::Kw(kw), line)),
                    None => tokens.push(Token::with_suffix(
                        TokenKind::Ident(lower),
                        line,
                        &word,
                        suffix,
                    )),
                }
                at_statement_start = false;
            }
            _ => {
                let Some((op, width)) = scan_operator(&chars, index) else {
                    return Err(ParseError::new(format!("无法识别的字符 `{current}`"), line));
                };
                tokens.push(Token::new(TokenKind::Op(op), line));
                index += width;
                at_statement_start = false;
            }
        }
    }

    tokens.push(Token::new(TokenKind::Eof, line));
    Ok(tokens)
}

/// 物理行 → 逻辑行号；映射表用完后沿用最后一项（EOF 行）。
fn logical_line(lines: &[usize], physical: usize) -> usize {
    lines
        .get(physical.saturating_sub(1))
        .copied()
        .or_else(|| lines.last().copied())
        .unwrap_or(physical)
}

fn is_line_continuation(chars: &[char], index: usize) -> bool {
    let mut cursor = index + 1;
    while cursor < chars.len() && (chars[cursor] == ' ' || chars[cursor] == '\t') {
        cursor += 1;
    }
    cursor >= chars.len() || chars[cursor] == '\n' || chars[cursor] == '\r'
}

fn scan_string(chars: &[char], start: usize, line: usize) -> Result<(String, usize), ParseError> {
    let mut text = String::new();
    let mut index = start + 1;
    loop {
        match chars.get(index) {
            None => return Err(ParseError::new("字符串没有闭合", line)),
            Some('"') => {
                if matches!(chars.get(index + 1), Some('"')) {
                    text.push('"');
                    index += 2;
                } else {
                    return Ok((text, index + 1));
                }
            }
            Some(c) => {
                text.push(*c);
                index += 1;
            }
        }
    }
}

fn scan_radix_number(
    chars: &[char],
    start: usize,
    line: usize,
) -> Result<(i64, usize), ParseError> {
    let radix = if matches!(chars[start + 1], 'h' | 'H') { 16 } else { 8 };
    let mut index = start + 2;
    let mut digits = String::new();
    while index < chars.len() && chars[index].is_ascii_alphanumeric() {
        digits.push(chars[index]);
        index += 1;
    }
    if digits.is_empty() {
        return Err(ParseError::new("进制字面量缺少数字", line));
    }
    // `&HFFFF&`：进制字面量后面可以跟类型后缀（`%`/`&`/`!`/`#`/`@`），值不变。
    if matches!(chars.get(index), Some('%' | '&' | '!' | '#' | '@')) {
        index += 1;
    }
    let value = i64::from_str_radix(&digits, radix)
        .map_err(|_| ParseError::new(format!("无法解析数字 `{digits}`"), line))?;
    Ok((value, index))
}

fn scan_number(
    chars: &[char],
    start: usize,
    line: usize,
) -> Result<(TokenKind, usize), ParseError> {
    let mut index = start;
    let mut text = String::new();
    let mut is_float = false;

    while index < chars.len() && chars[index].is_ascii_digit() {
        text.push(chars[index]);
        index += 1;
    }
    if matches!(chars.get(index), Some('.')) {
        is_float = true;
        text.push('.');
        index += 1;
        while index < chars.len() && chars[index].is_ascii_digit() {
            text.push(chars[index]);
            index += 1;
        }
    }
    if matches!(chars.get(index), Some('e' | 'E')) {
        let mut cursor = index + 1;
        let mut exponent = String::new();
        if matches!(chars.get(cursor), Some('+' | '-')) {
            exponent.push(chars[cursor]);
            cursor += 1;
        }
        let mut digits = String::new();
        while cursor < chars.len() && chars[cursor].is_ascii_digit() {
            digits.push(chars[cursor]);
            cursor += 1;
        }
        if !digits.is_empty() {
            is_float = true;
            text.push('e');
            text.push_str(&exponent);
            text.push_str(&digits);
            index = cursor;
        }
    }

    if is_float {
        let value: f64 = text
            .parse()
            .map_err(|_| ParseError::new(format!("无法解析数字 `{text}`"), line))?;
        Ok((TokenKind::Float(value), index))
    } else {
        let value: i64 = text
            .parse()
            .map_err(|_| ParseError::new(format!("整数 `{text}` 超出范围"), line))?;
        Ok((TokenKind::Int(value), index))
    }
}

fn scan_operator(chars: &[char], index: usize) -> Option<(Op, usize)> {
    let current = chars[index];
    let next = chars.get(index + 1).copied();
    let pair = match (current, next) {
        ('<', Some('>')) => Some((Op::NotEq, 2)),
        ('<', Some('=')) => Some((Op::LtEq, 2)),
        ('>', Some('=')) => Some((Op::GtEq, 2)),
        _ => None,
    };
    if let Some(result) = pair {
        return Some(result);
    }
    let op = match current {
        '+' => Op::Plus,
        '-' => Op::Minus,
        '*' => Op::Star,
        '/' => Op::Slash,
        '\\' => Op::Backslash,
        '^' => Op::Caret,
        '&' => Op::Amp,
        '=' => Op::Eq,
        '<' => Op::Lt,
        '>' => Op::Gt,
        '(' => Op::LParen,
        ')' => Op::RParen,
        '{' => Op::LBrace,
        '}' => Op::RBrace,
        ',' => Op::Comma,
        ';' => Op::Semicolon,
        '.' => Op::Dot,
        '#' => Op::Hash,
        _ => return None,
    };
    Some((op, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn keywords_are_case_insensitive() {
        let tokens = kinds("dIm X aS LoNg");
        assert_eq!(tokens[0], TokenKind::Kw(Kw::Dim));
        assert_eq!(tokens[1], TokenKind::Ident("x".into()));
        assert_eq!(tokens[2], TokenKind::Kw(Kw::As));
    }

    #[test]
    fn comments_and_continuation() {
        let tokens = kinds("' 注释\nx = 1 _\n  + 2\nRem 另一行注释\ny = 3");
        assert_eq!(
            tokens
                .iter()
                .filter(|token| **token == TokenKind::Newline)
                .count(),
            3
        );
        assert!(!tokens.iter().any(|token| matches!(token, TokenKind::Ident(name) if name == "rem")));
    }

    #[test]
    fn strings_numbers_and_suffix() {
        let tokens = kinds("\"a\"\"b\" &H10 &o7 1.5 Left$ x%");
        assert_eq!(tokens[0], TokenKind::Str("a\"b".into()));
        assert_eq!(tokens[1], TokenKind::Int(16));
        assert_eq!(tokens[2], TokenKind::Int(7));
        assert_eq!(tokens[3], TokenKind::Float(1.5));
        assert_eq!(tokens[4], TokenKind::Ident("left".into()));
        assert_eq!(tokens[5], TokenKind::Ident("x".into()));
    }
}
