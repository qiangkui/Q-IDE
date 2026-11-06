//! VB 递归下降语法分析。

use crate::interpreter::ast::*;
use crate::interpreter::error::ParseError;
use crate::interpreter::lexer::{Kw, Op, Token, TokenKind, tokenize};
use crate::interpreter::value::{VbType, Value};

/// 解析一段 VB 源码。
pub fn parse(source: &str) -> Result<Program, ParseError> {
    let tokens = tokenize(source)?;
    Parser {
        tokens,
        pos: 0,
        option_explicit: false,
        option_base: 0,
        option_compare_text: false,
        record_types: Vec::new(),
        enum_types: Vec::new(),
        class_types: Vec::new(),
    }
    .parse_program()
}

/// 解析单个表达式（调试器「监视」用）。表达式后面不能再有其他内容。
pub fn parse_expression(source: &str) -> Result<Expr, ParseError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        option_explicit: false,
        option_base: 0,
        option_compare_text: false,
        record_types: Vec::new(),
        enum_types: Vec::new(),
        class_types: Vec::new(),
    };
    let expr = parser.parse_expr()?;
    parser.skip_newlines();
    if !parser.at_eof() {
        return Err(ParseError::new(
            format!("表达式后面还有多余的内容：{}", parser.token().describe()),
            parser.line(),
        ));
    }
    Ok(expr)
}

/// 代码块（`If` / 循环 / 过程体）的结束标记。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    ElseIf,
    Else,
    EndIf,
    Next,
    Loop,
    Wend,
    Case,
    EndSelect,
    EndSub,
    EndFunction,
    EndWith,
    EndProperty,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// `Option Explicit`。
    option_explicit: bool,
    /// `Option Base`：数组默认下界。
    option_base: i64,
    /// `Option Compare Text`：字符串比较忽略大小写。
    option_compare_text: bool,
    /// 已解析的 `Type ... End Type`，供 `As 类型名` 查表。
    record_types: Vec<TypeDef>,
    /// 已解析的 `Enum` 名字：`Dim c As Color` 按 `Long` 处理。
    enum_types: Vec<String>,
    /// 已解析的 `Class` 名字：`Dim p As Point3D` 按对象处理。
    class_types: Vec<String>,
}

impl Parser {
    // ---------- token 辅助 ----------

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_at(&self, offset: usize) -> &TokenKind {
        let index = (self.pos + offset).min(self.tokens.len() - 1);
        &self.tokens[index].kind
    }

    fn token(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn line(&self) -> usize {
        self.tokens[self.pos].line
    }

    fn advance(&mut self) -> TokenKind {
        let kind = self.tokens[self.pos].kind.clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        kind
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    fn at_newline(&self) -> bool {
        matches!(self.peek(), TokenKind::Newline)
    }

    fn at_statement_end(&self) -> bool {
        matches!(self.peek(), TokenKind::Newline | TokenKind::Colon | TokenKind::Eof)
    }

    fn at_kw(&self, kw: Kw) -> bool {
        matches!(self.peek(), TokenKind::Kw(k) if *k == kw)
    }

    fn at_ident(&self, name: &str) -> bool {
        matches!(self.peek(), TokenKind::Ident(n) if n.eq_ignore_ascii_case(name))
            && self.token().suffix.is_none()
    }

    fn at_ident_pair(&self, first: &str, second: Kw) -> bool {
        self.at_ident(first) && matches!(self.peek_at(1), TokenKind::Kw(k) if *k == second)
    }

    fn eat_kw(&mut self, kw: Kw) -> bool {
        if self.at_kw(kw) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, kw: Kw, what: &str) -> Result<(), ParseError> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            Err(ParseError::new(
                format!("{what}，但遇到 {}", self.token().describe()),
                self.line(),
            ))
        }
    }

    fn at_op(&self, op: Op) -> bool {
        matches!(self.peek(), TokenKind::Op(o) if *o == op)
    }

    fn eat_op(&mut self, op: Op) -> bool {
        if self.at_op(op) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_op(&mut self, op: Op, what: &str) -> Result<(), ParseError> {
        if self.eat_op(op) {
            Ok(())
        } else {
            Err(ParseError::new(
                format!("{what}，但遇到 {}", self.token().describe()),
                self.line(),
            ))
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                self.advance();
                Ok(name)
            }
            _ => Err(ParseError::new(
                format!("需要{what}，但遇到 {}", self.token().describe()),
                self.line(),
            )),
        }
    }

    /// 读一个标识符，同时给出它的类型后缀（`x%`、`s$`）。
    fn expect_ident_typed(&mut self, what: &str) -> Result<(String, Option<char>), ParseError> {
        let suffix = self.token().suffix;
        let name = self.expect_ident(what)?;
        Ok((name, suffix))
    }

    fn expect_string(&mut self, what: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            TokenKind::Str(text) => {
                self.advance();
                Ok(text)
            }
            _ => Err(ParseError::new(
                format!("需要{what}，但遇到 {}", self.token().describe()),
                self.line(),
            )),
        }
    }

    fn skip_separators(&mut self) {
        while matches!(self.peek(), TokenKind::Newline | TokenKind::Colon) {
            self.advance();
        }
    }

    /// 括号内的换行（VB.NET 式的隐式续行）：`(ByVal a As Long,` 换行后继续。
    fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokenKind::Newline) {
            self.advance();
        }
    }

    fn eat_line_end(&mut self) {
        while matches!(self.peek(), TokenKind::Colon) {
            self.advance();
        }
        if self.at_newline() {
            self.advance();
        }
    }

    // ---------- 顶层 ----------

    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut program = Program::default();
        loop {
            self.skip_separators();
            if self.at_eof() {
                break;
            }

            // `Option Explicit` / `Option Base n`。
            if self.eat_kw(Kw::Option) {
                self.parse_option()?;
                continue;
            }

            // 允许 `Public` / `Private` / `Global` 修饰。
            while matches!(
                self.peek(),
                TokenKind::Kw(Kw::Public | Kw::Private | Kw::Global)
            ) {
                self.advance();
            }

            match self.peek() {
                TokenKind::Kw(Kw::Type) => {
                    program.types.push(self.parse_typedef()?);
                }
                TokenKind::Kw(Kw::Class) => {
                    program.classes.push(self.parse_class()?);
                }
                TokenKind::Kw(Kw::Declare) => {
                    program.declares.push(self.parse_declare()?);
                }
                TokenKind::Kw(Kw::Sub) => {
                    program.procedures.push(self.parse_procedure(false)?);
                }
                TokenKind::Kw(Kw::Function) => {
                    program.procedures.push(self.parse_procedure(true)?);
                }
                TokenKind::Kw(Kw::Property) => {
                    program.procedures.push(self.parse_property()?);
                }
                TokenKind::Kw(Kw::Enum) => {
                    program.enums.push(self.parse_enum()?);
                }
                _ => {
                    let before = self.pos;
                    let stmt = self.parse_statement()?;
                    program.main.push(stmt);
                    if self.pos == before {
                        return Err(ParseError::new("无法解析的语句", self.line()));
                    }
                }
            }
        }
        program.option_explicit = self.option_explicit;
        program.option_compare_text = self.option_compare_text;
        Ok(program)
    }

    /// `Option Explicit` / `Option Base 0|1` / `Option Compare Binary|Text`。
    fn parse_option(&mut self) -> Result<(), ParseError> {
        let line = self.line();
        if self.eat_kw(Kw::Explicit) {
            self.option_explicit = true;
            self.eat_line_end();
            return Ok(());
        }
        if self.at_ident("compare") {
            self.advance();
            let text = if self.at_ident("text") {
                true
            } else if self.at_ident("binary") {
                false
            } else {
                return Err(ParseError::new(
                    "`Option Compare` 只支持 `Binary` 或 `Text`",
                    line,
                ));
            };
            self.advance();
            self.option_compare_text = text;
            self.eat_line_end();
            return Ok(());
        }
        if matches!(self.peek(), TokenKind::Ident(name) if name == "base") {
            self.advance();
            let value = match self.peek().clone() {
                TokenKind::Int(value @ (0 | 1)) => {
                    self.advance();
                    value
                }
                _ => {
                    return Err(ParseError::new(
                        "`Option Base` 只支持 0 或 1",
                        line,
                    ));
                }
            };
            self.option_base = value;
            self.eat_line_end();
            return Ok(());
        }
        Err(ParseError::new(
            "只支持 `Option Explicit`、`Option Base 0|1` 与 `Option Compare Binary|Text`",
            line,
        ))
    }

    /// `New Collection` / `New Dictionary` / `New 类名`（调用方已吃掉 `New`）。
    fn parse_new_object(&mut self) -> Result<NewObject, ParseError> {
        if self.at_ident("collection") {
            self.advance();
            return Ok(NewObject::Collection);
        }
        if self.at_ident("dictionary") {
            self.advance();
            return Ok(NewObject::Dictionary);
        }
        let name = self.expect_ident("类名")?;
        Ok(NewObject::Class(name))
    }

    /// `Class 名字 ... End Class`：只要实例变量声明与 `Sub`/`Function`/`Property`。
    fn parse_class(&mut self) -> Result<ClassDef, ParseError> {
        let line = self.line();
        self.expect_kw(Kw::Class, "需要 Class")?;
        let display = self.token().text.clone();
        let name = self.expect_ident("类名")?;
        self.eat_line_end();
        let mut def = ClassDef {
            name,
            display,
            line,
            fields: Vec::new(),
            methods: Vec::new(),
        };
        loop {
            self.skip_separators();
            if self.at_eof() {
                return Err(ParseError::new("`Class` 缺少 `End Class`", line));
            }
            if self.at_kw(Kw::End) && matches!(self.peek_at(1), TokenKind::Kw(Kw::Class)) {
                self.advance();
                self.advance();
                break;
            }
            // `Public` / `Private` 修饰对解释器没有实质区别（成员都从外部可见）。
            while matches!(
                self.peek(),
                TokenKind::Kw(Kw::Public | Kw::Private | Kw::Global)
            ) {
                self.advance();
            }
            match self.peek() {
                TokenKind::Kw(Kw::Dim) => {
                    let line = self.line();
                    self.advance();
                    if let Stmt::Dim { items, .. } = self.parse_dim(line, false)? {
                        def.fields.extend(items);
                    }
                }
                TokenKind::Kw(Kw::WithEvents) => {
                    // `Private WithEvents m As Foo`：接收 Foo 的事件。
                    let line = self.line();
                    self.advance();
                    self.eat_kw(Kw::Dim);
                    if let Stmt::Dim { items, .. } = self.parse_dim(line, false)? {
                        def.fields.extend(items.into_iter().map(|item| Declared {
                            with_events: true,
                            ..item
                        }));
                    }
                }
                TokenKind::Kw(Kw::Sub) => def.methods.push(self.parse_procedure(false)?),
                TokenKind::Kw(Kw::Function) => def.methods.push(self.parse_procedure(true)?),
                TokenKind::Kw(Kw::Property) => def.methods.push(self.parse_property()?),
                TokenKind::Kw(Kw::Static) => {
                    self.advance();
                    if !matches!(self.peek(), TokenKind::Kw(Kw::Sub | Kw::Function)) {
                        return Err(ParseError::new(
                            "`Class` 里只支持变量声明与 Sub/Function/Property",
                            self.line(),
                        ));
                    }
                }
                TokenKind::Ident(_) => {
                    // `Private x As Long`（不带 `Dim` 的成员声明）。
                    let line = self.line();
                    if let Stmt::Dim { items, .. } = self.parse_dim(line, false)? {
                        def.fields.extend(items);
                    }
                }
                _ => {
                    return Err(ParseError::new(
                        "`Class` 里只支持变量声明与 Sub/Function/Property",
                        self.line(),
                    ));
                }
            }
        }
        self.class_types.push(def.name.clone());
        Ok(def)
    }

    /// `Type Point ... End Type`。
    fn parse_typedef(&mut self) -> Result<TypeDef, ParseError> {        let line = self.line();
        self.expect_kw(Kw::Type, "需要 Type")?;
        let display = self.token().text.clone();
        let name = self.expect_ident("类型名")?;
        self.eat_line_end();

        let mut fields = Vec::new();
        loop {
            self.skip_separators();
            if self.at_eof() {
                return Err(ParseError::new("`Type` 缺少 `End Type`", line));
            }
            if self.at_kw(Kw::End) && matches!(self.peek_at(1), TokenKind::Kw(Kw::Type)) {
                self.advance();
                self.advance();
                break;
            }
            let (field_name, _suffix) = self.expect_ident_typed("字段名")?;
            let array_bounds = if self.at_op(Op::LParen) {
                // `Items(2) As Long` / `Items(1 To 3) As Long`：定长数组字段。
                self.advance();
                let first = self.expect_int_literal("数组字段的上界")?;
                let bounds = if self.eat_kw(Kw::To) {
                    let upper = self.expect_int_literal("数组字段的上界")?;
                    (first, upper)
                } else {
                    (0, first)
                };
                self.expect_op(Op::RParen, "数组字段需要 `)`")?;
                if bounds.1 < bounds.0 {
                    return Err(ParseError::new(
                        format!("数组字段的下界 {} 大于上界 {}", bounds.0, bounds.1),
                        self.line(),
                    ));
                }
                Some(bounds)
            } else {
                None
            };
            self.expect_kw(Kw::As, "字段需要 `As 类型`")?;
            let (ty, record) = self.parse_declared_type()?;
            let fixed_len = if self.eat_op(Op::Star) {
                if ty != VbType::Str || record.is_some() {
                    return Err(ParseError::new("只有 `String * n` 支持定长", self.line()));
                }
                let len = self.expect_int_literal("定长字符串的长度")?;
                if len < 0 {
                    return Err(ParseError::new("定长字符串的长度不能为负", self.line()));
                }
                Some(len as usize)
            } else {
                None
            };
            fields.push(TypeField {
                name: field_name,
                ty,
                record,
                fixed_len,
                array_bounds,
            });
            self.eat_line_end();
        }

        let def = TypeDef {
            name,
            display,
            fields,
            line,
        };
        self.record_types.push(def.clone());
        Ok(def)
    }

    fn parse_declare(&mut self) -> Result<Declare, ParseError> {
        let line = self.line();
        self.expect_kw(Kw::Declare, "需要 Declare")?;
        let _ = self.eat_kw(Kw::PtrSafe);
        let is_function = if self.eat_kw(Kw::Function) {
            true
        } else {
            self.expect_kw(Kw::Sub, "Declare 后需要 Function 或 Sub")?;
            false
        };
        let symbol_default = self.token().text.clone();
        let (name, suffix) = self.expect_ident_typed("函数名")?;
        self.expect_kw(Kw::Lib, "Declare 需要 `Lib \"库名\"`")?;
        let lib = self.expect_string("库名")?;
        let symbol = if self.eat_kw(Kw::Alias) {
            self.expect_string("导出名")?
        } else {
            symbol_default
        };
        let params = if self.at_op(Op::LParen) {
            self.parse_params()?
        } else {
            Vec::new()
        };
        let ret = if self.eat_kw(Kw::As) {
            self.parse_type()?
        } else if is_function {
            suffix
                .and_then(VbType::from_suffix)
                .unwrap_or(VbType::Long)
        } else {
            VbType::Variant
        };
        Ok(Declare {
            name,
            lib,
            symbol,
            params,
            ret,
            is_function,
            line,
        })
    }

    fn parse_procedure(&mut self, is_function: bool) -> Result<Procedure, ParseError> {
        let line = self.line();
        if is_function {
            self.expect_kw(Kw::Function, "需要 Function")?;
        } else {
            self.expect_kw(Kw::Sub, "需要 Sub")?;
        }
        let (name, suffix) = self.expect_ident_typed("过程名")?;
        let params = if self.at_op(Op::LParen) {
            self.parse_params()?
        } else {
            Vec::new()
        };
        let ret = if is_function && self.eat_kw(Kw::As) {
            self.parse_type()?
        } else if is_function {
            suffix
                .and_then(VbType::from_suffix)
                .unwrap_or(VbType::Variant)
        } else {
            VbType::Variant
        };
        self.eat_line_end();
        let body = self.parse_block(&[Stop::EndSub, Stop::EndFunction])?;
        self.expect_kw(Kw::End, "过程缺少 `End Sub` / `End Function`")?;
        match self.peek() {
            TokenKind::Kw(Kw::Sub | Kw::Function) => {
                self.advance();
            }
            _ => {
                return Err(ParseError::new(
                    "过程缺少 `End Sub` / `End Function`",
                    self.line(),
                ));
            }
        }
        Ok(Procedure {
            name,
            params,
            body,
            is_function,
            is_let: false,
            is_set: false,
            ret,
            line,
        })
    }

    /// `Property Get Name(...) As T ... End Property`、
    /// `Property Let Name(args..., 值) ... End Property`、`Property Set Name(args..., 对象) ... End Property`。
    fn parse_property(&mut self) -> Result<Procedure, ParseError> {
        let line = self.line();
        self.expect_kw(Kw::Property, "需要 Property")?;
        let mut is_set = false;
        let is_get = if self.at_ident("get") {
            self.advance();
            true
        } else if self.eat_kw(Kw::Let) {
            false
        } else if self.eat_kw(Kw::Set) {
            is_set = true;
            false
        } else {
            return Err(ParseError::new(
                "`Property` 后需要 `Get`、`Let` 或 `Set`",
                line,
            ));
        };
        let (name, suffix) = self.expect_ident_typed("属性名")?;
        let params = if self.at_op(Op::LParen) {
            self.parse_params()?
        } else {
            Vec::new()
        };
        if is_get {
            let ret = if self.eat_kw(Kw::As) {
                self.parse_type()?
            } else {
                suffix
                    .and_then(VbType::from_suffix)
                    .unwrap_or(VbType::Variant)
            };
            self.eat_line_end();
            let body = self.parse_block(&[Stop::EndProperty])?;
            self.expect_property_end("`Property Get` 缺少 `End Property`")?;
            return Ok(Procedure {
                name,
                params,
                body,
                is_function: true,
                is_let: false,
                is_set: false,
                ret,
                line,
            });
        }
        if params.is_empty() {
            return Err(ParseError::new(
                if is_set {
                    "`Property Set` 至少需要一个接收赋值的参数"
                } else {
                    "`Property Let` 至少需要一个接收赋值的参数"
                },
                line,
            ));
        }
        self.eat_line_end();
        let body = self.parse_block(&[Stop::EndProperty])?;
        self.expect_property_end(if is_set {
            "`Property Set` 缺少 `End Property`"
        } else {
            "`Property Let` 缺少 `End Property`"
        })?;
        Ok(Procedure {
            name,
            params,
            body,
            is_function: false,
            is_let: true,
            is_set,
            ret: VbType::Variant,
            line,
        })
    }

    fn expect_property_end(&mut self, what: &str) -> Result<(), ParseError> {
        self.expect_kw(Kw::End, what)?;
        self.expect_kw(Kw::Property, what)
    }

    /// `Enum Color ... End Enum`。
    fn parse_enum(&mut self) -> Result<EnumDef, ParseError> {
        let line = self.line();
        self.expect_kw(Kw::Enum, "需要 Enum")?;
        let display = self.token().text.clone();
        let name = self.expect_ident("枚举名")?;
        self.eat_line_end();

        let mut members = Vec::new();
        let mut next = 0i64;
        loop {
            self.skip_separators();
            if self.at_eof() {
                return Err(ParseError::new("`Enum` 缺少 `End Enum`", line));
            }
            if self.at_kw(Kw::End) && matches!(self.peek_at(1), TokenKind::Kw(Kw::Enum)) {
                self.advance();
                self.advance();
                break;
            }
            let member = self.expect_ident("枚举成员名")?;
            let value = if self.eat_op(Op::Eq) {
                let negative = self.eat_op(Op::Minus);
                let TokenKind::Int(value) = self.peek().clone() else {
                    return Err(ParseError::new("枚举成员的值必须是整数字面量", self.line()));
                };
                self.advance();
                if negative { -value } else { value }
            } else {
                next
            };
            next = value.saturating_add(1);
            members.push((member, value));
            self.eat_line_end();
        }
        if members.is_empty() {
            return Err(ParseError::new("`Enum` 至少需要一个成员", line));
        }
        self.enum_types.push(name);
        Ok(EnumDef {
            display,
            members,
            line,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        self.expect_op(Op::LParen, "参数列表需要 `(`")?;
        self.skip_newlines();
        let mut params = Vec::new();
        if self.eat_op(Op::RParen) {
            return Ok(params);
        }
        loop {
            self.skip_newlines();
            let optional = self.eat_kw(Kw::Optional);
            let param_array = self.eat_kw(Kw::ParamArray);
            let by_ref = if self.eat_kw(Kw::ByVal) {
                false
            } else {
                let _ = self.eat_kw(Kw::ByRef);
                true
            };
            let (name, suffix) = self.expect_ident_typed("参数名")?;
            let mut is_array = param_array;
            if self.at_op(Op::LParen) {
                // 数组参数 `a()`：与调用方共享同一个数组。
                self.advance();
                self.expect_op(Op::RParen, "数组参数需要 `)`")?;
                is_array = true;
            }
            if param_array && !is_array {
                return Err(ParseError::new(
                    "`ParamArray` 参数需要写成 `xs()`",
                    self.line(),
                ));
            }
            let (ty, record) = if self.eat_kw(Kw::As) {
                self.parse_declared_type()?
            } else {
                (
                    suffix
                        .and_then(VbType::from_suffix)
                        .unwrap_or(VbType::Variant),
                    None,
                )
            };
            let default = if self.at_op(Op::Eq) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            params.push(Param {
                name,
                by_ref,
                ty,
                optional,
                default,
                is_array,
                param_array,
                record,
            });
            self.skip_newlines();
            if param_array && self.at_op(Op::Comma) {
                return Err(ParseError::new(
                    "`ParamArray` 必须是最后一个参数",
                    self.line(),
                ));
            }
            if !self.eat_op(Op::Comma) {
                break;
            }
            self.skip_newlines();
        }
        self.expect_op(Op::RParen, "参数列表需要 `)`")?;
        Ok(params)
    }

    fn parse_type(&mut self) -> Result<VbType, ParseError> {
        Ok(self.parse_declared_type()?.0)
    }

    /// 需要整数字面量的位置（`Enum` 值、`String * n`、定长数组字段）。
    fn expect_int_literal(&mut self, what: &str) -> Result<i64, ParseError> {
        match self.peek().clone() {
            TokenKind::Int(value) => {
                self.advance();
                Ok(value)
            }
            _ => Err(ParseError::new(
                format!("{what}必须是整数字面量，但遇到 {}", self.token().describe()),
                self.line(),
            )),
        }
    }

    /// 解析类型名；返回基础类型，若是 UDT 再带上类型名。
    fn parse_declared_type(&mut self) -> Result<(VbType, Option<String>), ParseError> {
        let line = self.line();
        let name = self.expect_ident("类型名")?;
        if let Some(ty) = VbType::parse(&name) {
            return Ok((ty, None));
        }
        if self.record_types.iter().any(|def| def.name == name) {
            return Ok((VbType::Variant, Some(name)));
        }
        if self.enum_types.contains(&name) {
            return Ok((VbType::Long, None));
        }
        // `Class` 声明的类型按对象处理（`Set` + `New` 才能拿到实例）。
        if self.class_types.contains(&name) {
            return Ok((VbType::Object, None));
        }
        // `Collection` / `Dictionary` 没有独立的类型，按对象处理。
        if name == "collection" || name == "dictionary" {
            return Ok((VbType::Object, None));
        }
        Err(ParseError::new(format!("未知类型 `{name}`"), line))
    }

    // ---------- 语句块 ----------

    fn at_stop(&self, stops: &[Stop]) -> bool {
        match self.peek() {
            TokenKind::Kw(Kw::ElseIf) => stops.contains(&Stop::ElseIf),
            TokenKind::Kw(Kw::Else) => stops.contains(&Stop::Else),
            TokenKind::Kw(Kw::Next) => stops.contains(&Stop::Next),
            TokenKind::Kw(Kw::Loop) => stops.contains(&Stop::Loop),
            TokenKind::Kw(Kw::Wend) => stops.contains(&Stop::Wend),
            TokenKind::Kw(Kw::Case) => stops.contains(&Stop::Case),
            TokenKind::Kw(Kw::End) => match self.peek_at(1) {
                TokenKind::Kw(Kw::If) => stops.contains(&Stop::EndIf),
                TokenKind::Kw(Kw::Select) => stops.contains(&Stop::EndSelect),
                TokenKind::Kw(Kw::Sub) => stops.contains(&Stop::EndSub),
                TokenKind::Kw(Kw::Function) => stops.contains(&Stop::EndFunction),
                TokenKind::Kw(Kw::Property) => stops.contains(&Stop::EndProperty),
                TokenKind::Kw(Kw::With) => stops.contains(&Stop::EndWith),
                _ => false,
            },
            TokenKind::Eof => true,
            _ => false,
        }
    }

    fn parse_block(&mut self, stops: &[Stop]) -> Result<Vec<Stmt>, ParseError> {
        let mut stmts = Vec::new();
        loop {
            self.skip_separators();
            if self.at_stop(stops) {
                break;
            }
            if self.at_eof() {
                break;
            }
            let before = self.pos;
            stmts.push(self.parse_statement()?);
            if self.pos == before {
                return Err(ParseError::new("无法解析的语句", self.line()));
            }
        }
        Ok(stmts)
    }

    // ---------- 语句 ----------

    fn parse_statement(&mut self) -> Result<Stmt, ParseError> {
        let line = self.line();

        // 标签：`Again:`
        if let TokenKind::Ident(name) = self.peek().clone()
            && matches!(self.peek_at(1), TokenKind::Colon)
        {
            self.advance();
            self.advance();
            return Ok(Stmt::Label(name, line));
        }
        if self.eat_kw(Kw::On) {
            return self.parse_on_error(line);
        }
        if self.eat_kw(Kw::GoTo) {
            let label = self.expect_ident("标签名")?;
            return Ok(Stmt::Goto(label, line));
        }
        if self.eat_kw(Kw::Resume) {
            let target = if self.at_statement_end() {
                ResumeTarget::Current
            } else if self.eat_kw(Kw::Next) {
                ResumeTarget::Next
            } else {
                ResumeTarget::Label(self.expect_ident("标签名")?)
            };
            return Ok(Stmt::Resume { target, line });
        }

        if self.at_op(Op::Dot) {
            return self.parse_with_member(line);
        }
        if self.eat_kw(Kw::With) {
            let subject = self.parse_expr()?;
            self.eat_line_end();
            let body = self.parse_block(&[Stop::EndWith])?;
            self.expect_kw(Kw::End, "`With` 缺少 `End With`")?;
            self.expect_kw(Kw::With, "`With` 缺少 `End With`")?;
            return Ok(Stmt::With {
                subject,
                body,
                line,
            });
        }
        if self.eat_kw(Kw::Let) {
            return self.parse_assignment(line);
        }
        if self.eat_kw(Kw::Dim) {
            return self.parse_dim(line, false);
        }
        if self.eat_kw(Kw::Static) {
            return self.parse_dim(line, true);
        }
        if self.eat_kw(Kw::ReDim) {
            return self.parse_redim(line);
        }
        if self.eat_kw(Kw::Erase) {
            let mut names = vec![self.expect_ident("数组名")?];
            while self.eat_op(Op::Comma) {
                names.push(self.expect_ident("数组名")?);
            }
            return Ok(Stmt::Erase { names, line });
        }
        if self.eat_kw(Kw::Open) {
            return self.parse_open(line);
        }
        if self.eat_kw(Kw::Close) {
            return self.parse_close(line);
        }
        if self.at_ident("get") && matches!(self.peek_at(1), TokenKind::Op(Op::Hash)) {
            self.advance();
            return self.parse_get(line);
        }
        if self.at_ident("put") && matches!(self.peek_at(1), TokenKind::Op(Op::Hash)) {
            self.advance();
            return self.parse_put(line);
        }
        if self.at_ident("seek") && matches!(self.peek_at(1), TokenKind::Op(Op::Hash)) {
            self.advance();
            return self.parse_seek(line);
        }
        if self.at_ident("width") && matches!(self.peek_at(1), TokenKind::Op(Op::Hash)) {
            self.advance();
            self.expect_op(Op::Hash, "`Width` 需要 `#`")?;
            let number = self.parse_expr()?;
            self.expect_op(Op::Comma, "`Width` 需要逗号")?;
            let width = self.parse_expr()?;
            return Ok(Stmt::WidthFile {
                number,
                width,
                line,
            });
        }
        // `Write #1, a, b`：CSV 风格写出（字符串带引号、字段用逗号分隔、行尾 CRLF）。
        if self.at_ident("write") && matches!(self.peek_at(1), TokenKind::Op(Op::Hash)) {
            self.advance();
            self.expect_op(Op::Hash, "`Write` 需要 `#`")?;
            let number = self.parse_expr()?;
            let mut values = Vec::new();
            if self.eat_op(Op::Comma) && !self.at_statement_end() {
                values.push(self.parse_expr()?);
                while self.eat_op(Op::Comma) {
                    values.push(self.parse_expr()?);
                }
            }
            return Ok(Stmt::WriteFile {
                number,
                values,
                line,
            });
        }
        if (self.at_ident("lock") || self.at_ident("unlock"))
            && matches!(self.peek_at(1), TokenKind::Op(Op::Hash))
        {
            let unlock = self.at_ident("unlock");
            self.advance();
            self.expect_op(Op::Hash, "`Lock`/`Unlock` 需要 `#`")?;
            let number = self.parse_expr()?;
            let mut start = None;
            let mut end = None;
            if self.eat_op(Op::Comma) && !self.at_statement_end() {
                start = Some(self.parse_expr()?);
                if self.eat_kw(Kw::To) {
                    end = Some(self.parse_expr()?);
                }
            }
            return Ok(Stmt::LockFile {
                number,
                unlock,
                start,
                end,
                line,
            });        }
        if self.at_ident_pair("line", Kw::Input) {
            self.advance();
            self.advance();
            self.expect_op(Op::Hash, "`Line Input` 需要 `#`")?;
            let number = self.parse_expr()?;
            self.expect_op(Op::Comma, "`Line Input` 需要逗号")?;
            let target = self.expect_ident("变量名")?;
            return Ok(Stmt::LineInput {
                number,
                target,
                line,
            });
        }
        if self.at_kw(Kw::Input) && matches!(self.peek_at(1), TokenKind::Op(Op::Hash)) {
            self.advance();
            self.advance();
            let number = self.parse_expr()?;
            self.expect_op(Op::Comma, "`Input #` 需要逗号")?;
            let mut targets = vec![self.expect_ident("变量名")?];
            while self.eat_op(Op::Comma) {
                targets.push(self.expect_ident("变量名")?);
            }
            return Ok(Stmt::InputFile {
                number,
                targets,
                line,
            });
        }
        if self.eat_kw(Kw::Const) {
            return self.parse_const(line);
        }
        if self.eat_kw(Kw::Print) {
            if self.at_op(Op::Hash) {
                self.advance();
                let number = self.parse_expr()?;
                self.expect_op(Op::Comma, "`Print #` 需要逗号")?;
                let (values, seps, suppress_newline) = self.parse_print_values()?;
                return Ok(Stmt::PrintFile {
                    number,
                    values,
                    seps,
                    suppress_newline,
                    line,
                });
            }
            return self.parse_print(line);
        }
        if self.eat_kw(Kw::Debug) {
            self.expect_op(Op::Dot, "`Debug` 后需要 `.`")?;
            self.expect_kw(Kw::Print, "只支持 `Debug.Print`")?;
            return self.parse_print(line);
        }
        if self.eat_kw(Kw::If) {
            return self.parse_if(line);
        }
        if self.eat_kw(Kw::For) {
            return self.parse_for(line);
        }
        if self.eat_kw(Kw::Do) {
            return self.parse_do(line);
        }
        if self.eat_kw(Kw::While) {
            return self.parse_while(line);
        }
        if self.eat_kw(Kw::Select) {
            return self.parse_select(line);
        }
        if self.eat_kw(Kw::Exit) {
            return self.parse_exit(line);
        }
        if self.eat_kw(Kw::Return) {
            let value = if self.at_statement_end() {
                None
            } else {
                Some(self.parse_expr()?)
            };
            return Ok(Stmt::Return { value, line });
        }
        if self.eat_kw(Kw::RaiseEvent) {
            let name = self.expect_ident("事件名")?;
            let args = if self.at_op(Op::LParen) {
                self.parse_paren_args()?
            } else if self.at_statement_end() {
                Vec::new()
            } else {
                let mut args = vec![self.parse_expr()?];
                while self.eat_op(Op::Comma) {
                    args.push(self.parse_expr()?);
                }
                args
            };
            return Ok(Stmt::RaiseEvent { name, args, line });
        }
        if self.eat_kw(Kw::Stop) {
            return Ok(Stmt::Halt { line });
        }
        if self.eat_kw(Kw::Call) {
            let name = self.expect_ident("过程名")?;
            let args = if self.at_op(Op::LParen) {
                self.parse_paren_args()?
            } else {
                Vec::new()
            };
            return Ok(Stmt::Call { name, args, line });
        }
        if self.at_kw(Kw::Set) {
            self.advance();
            let target = self.expect_ident("对象变量名")?;
            // `Set obj.Member = x` / `Set obj.Item(1) = x`：对象属性的赋值。
            if self.at_op(Op::Dot) {
                let mut path = Vec::new();
                while self.eat_op(Op::Dot) {
                    path.push(self.expect_ident("成员名")?);
                }
                let indices = if self.at_op(Op::LParen) {
                    self.parse_paren_args()?
                } else {
                    Vec::new()
                };
                if !indices.is_empty() {
                    let mut member_path = Vec::new();
                    while self.eat_op(Op::Dot) {
                        member_path.push(self.expect_ident("成员名")?);
                    }
                    self.expect_op(Op::Eq, "`Set` 需要 `=`")?;
                    let value = self.parse_expr()?;
                    return Ok(Stmt::AssignMemberIndex {
                        base: Expr::Var(target),
                        path,
                        indices,
                        member_path,
                        value,
                        set_op: true,
                        line,
                    });
                }
                self.expect_op(Op::Eq, "`Set` 需要 `=`")?;
                let value = self.parse_expr()?;
                return Ok(Stmt::AssignMember {
                    base: Expr::Var(target),
                    path,
                    value,
                    set_op: true,
                    line,
                });
            }
            self.expect_op(Op::Eq, "`Set` 需要 `=`")?;
            let value = self.parse_expr()?;
            return Ok(Stmt::Set {
                target,
                value,
                line,
            });
        }
        if self.eat_kw(Kw::End) {
            // 裸 End：结束脚本。`End If/Sub/Function/Select` 由块解析器处理。
            return Ok(Stmt::Halt { line });
        }

        self.parse_ident_statement(line)
    }

    /// `On Error Resume Next` / `On Error GoTo 标签` / `On Error GoTo 0`。
    fn parse_on_error(&mut self, line: usize) -> Result<Stmt, ParseError> {
        self.expect_kw(Kw::Error, "`On` 后需要 `Error`")?;
        if self.eat_kw(Kw::Resume) {
            self.expect_kw(Kw::Next, "只支持 `On Error Resume Next`")?;
            return Ok(Stmt::OnError {
                mode: ErrorMode::ResumeNext,
                line,
            });
        }
        self.expect_kw(Kw::GoTo, "`On Error` 后需要 `GoTo` 或 `Resume Next`")?;
        if let TokenKind::Int(0) = self.peek() {
            self.advance();
            return Ok(Stmt::OnError {
                mode: ErrorMode::Disabled,
                line,
            });
        }
        let label = self.expect_ident("标签名")?;
        Ok(Stmt::OnError {
            mode: ErrorMode::Goto(label),
            line,
        })
    }

    /// `Err.Raise 编号[, 来源][, 描述]`，空参数可以省略，多余的帮助参数会被忽略。
    fn parse_err_raise(&mut self, line: usize) -> Result<Stmt, ParseError> {
        if self.at_statement_end() {
            return Err(ParseError::new("`Err.Raise` 需要错误号", line));
        }
        let number = self.parse_expr()?;
        let mut source = None;
        let mut description = None;
        let mut index = 0usize;
        while self.eat_op(Op::Comma) {
            if self.at_statement_end() || self.at_op(Op::Comma) {
                index += 1;
                continue;
            }
            let expr = self.parse_expr()?;
            match index {
                0 => source = Some(expr),
                1 => description = Some(expr),
                _ => {}
            }
            index += 1;
        }
        if index > 4 {
            return Err(ParseError::new("`Err.Raise` 最多 5 个参数", line));
        }
        Ok(Stmt::ErrRaise {
            number,
            source,
            description,
            line,
        })
    }

    /// `With` 块里的 `.X = 1`（也允许 `.A.B = 1`）。
    fn parse_with_member(&mut self, line: usize) -> Result<Stmt, ParseError> {        let mut path = Vec::new();
        while self.eat_op(Op::Dot) {
            path.push(self.expect_ident("成员名")?);
        }
        self.expect_op(Op::Eq, "`With` 块内只支持 `.成员 = 值` 赋值")?;
        let value = self.parse_expr()?;
        Ok(Stmt::AssignWithMember { path, value, line })
    }

    /// 尝试把 `成员路径 = 值` 解析成成员赋值；不是赋值时回退到原位置。
    fn try_member_assignment(
        &mut self,
        base: Expr,
        line: usize,
    ) -> Result<Option<Stmt>, ParseError> {
        let start = self.pos;
        let mut path = Vec::new();
        while self.eat_op(Op::Dot) {
            path.push(self.expect_ident("成员名")?);
        }
        if path.is_empty() {
            self.pos = start;
            return Ok(None);
        }
        if self.at_op(Op::LParen) {
            // `p.Items(1) = 5` / `p.Items(1).Sub = 5`：记录里数组字段的元素赋值。
            let indices = self.parse_paren_args()?;
            let mut member_path = Vec::new();
            while self.eat_op(Op::Dot) {
                member_path.push(self.expect_ident("成员名")?);
            }
            if !self.at_op(Op::Eq) {
                // `obj.Method(1)` 这种成员调用：交回上层按调用处理。
                self.pos = start;
                return Ok(None);
            }
            self.expect_op(Op::Eq, "数组元素赋值需要 `=`")?;
            let value = self.parse_expr()?;
            return Ok(Some(Stmt::AssignMemberIndex {
                base,
                path,
                indices,
                member_path,
                value,
                set_op: false,
                line,
            }));
        }
        if !self.eat_op(Op::Eq) {
            // `.Y` 更可能是 `With` 相对成员的实参（`Bump .Y`）。
            self.pos = start;
            return Ok(None);
        }
        let value = self.parse_expr()?;
        Ok(Some(Stmt::AssignMember {
            base,
            path,
            value,
            set_op: false,
            line,
        }))
    }

    fn parse_ident_statement(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let (name, suffix) = self.expect_ident_typed("语句")?;
        // `Name 旧路径 As 新路径`：VB 的改名语句（`As` 不是表达式的一部分，得单独解析）。
        if name == "name"
            && !matches!(
                self.peek(),
                TokenKind::Op(Op::Eq) | TokenKind::Op(Op::LParen) | TokenKind::Op(Op::Dot)
            )
        {
            let from = self.parse_expr()?;
            self.expect_kw(Kw::As, "`Name` 需要 `As`")?;
            let to = self.parse_expr()?;
            return Ok(Stmt::Call {
                name: "rename".into(),
                args: vec![from, to],
                line,
            });
        }
        if self.eat_op(Op::Eq) {
            let value = self.parse_expr()?;
            return Ok(Stmt::Assign {
                target: name,
                value,
                suffix,
                line,
            });
        }
        // `Err.Clear` / `Err.Raise`
        if name == "err" && self.at_op(Op::Dot) {
            let start = self.pos;
            self.advance();
            if let TokenKind::Ident(member) = self.peek().clone() {
                self.advance();
                if member.eq_ignore_ascii_case("clear") && self.at_statement_end() {
                    return Ok(Stmt::ErrClear { line });
                }
                if member.eq_ignore_ascii_case("raise") {
                    return self.parse_err_raise(line);
                }
            }
            self.pos = start;
        }
        // `p.X = 1` / `p.Items(1) = 5`：成员赋值优先解析（`=` 同时是相等比较，
        // 必须先按「成员路径 + 等号」判断，不能先当表达式解析）。
        if self.at_op(Op::Dot)
            && let Some(stmt) = self.try_member_assignment(Expr::Var(name.clone()), line)?
        {
            return Ok(stmt);
        }
        // `c.Add x, key` / `obj.SomeMethod arg`：成员方法调用。
        if self.at_op(Op::Dot) {
            let start = self.pos;
            self.advance();
            if let TokenKind::Ident(method) = self.peek().clone() {
                self.advance();
                let mut args = Vec::new();
                if !self.at_statement_end() {
                    if self.at_op(Op::LParen) {
                        args = self.parse_paren_args()?;
                    } else {
                        args.push(self.parse_expr()?);
                        while self.eat_op(Op::Comma) {
                            args.push(self.parse_expr()?);
                        }
                    }
                }
                return Ok(Stmt::CallMethod {
                    object: name,
                    method,
                    args,
                    line,
                });
            }
            self.pos = start;
        }        // `a(0) = 1` / `a(0).X = 1` / `Mid(s, 2, 3) = "x"` / 普通调用
        if self.at_op(Op::LParen) {
            let args = self.parse_paren_args()?;
            if self.eat_op(Op::Eq) {
                let value = self.parse_expr()?;
                if name == "mid" {
                    // `Mid(s, 2, 3) = "x"`：就地替换，第一个实参必须是变量。
                    if !(2..=3).contains(&args.len()) {
                        return Err(ParseError::new(
                            "`Mid` 语句需要 2 或 3 个参数：`Mid(变量, 起点[, 长度]) = 值`",
                            line,
                        ));
                    }
                    let Expr::Var(target) = &args[0] else {
                        return Err(ParseError::new(
                            "`Mid` 语句的第一个参数必须是字符串变量",
                            line,
                        ));
                    };
                    return Ok(Stmt::MidAssign {
                        target: target.clone(),
                        start: args[1].clone(),
                        length: args.get(2).cloned(),
                        value,
                        line,
                    });
                }
                return Ok(Stmt::AssignIndex {
                    name,
                    indices: args,
                    value,
                    line,
                });
            }
            if self.at_op(Op::Dot)
                && let Some(stmt) = self.try_member_assignment(
                    Expr::Call {
                        name: name.clone(),
                        args: args.clone(),
                    },
                    line,
                )?
            {
                return Ok(stmt);
            }
            return Ok(Stmt::Call { name, args, line });
        }
        // 无括号的过程调用：`MsgBox "hi", 0`；也可以是完全无参的 `Foo`。
        let mut args = Vec::new();
        if self.at_statement_end() {
            return Ok(Stmt::Call { name, args, line });
        }
        args.push(self.parse_expr()?);
        while self.eat_op(Op::Comma) {
            args.push(self.parse_expr()?);
        }
        Ok(Stmt::Call { name, args, line })
    }

    fn parse_assignment(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let (target, suffix) = self.expect_ident_typed("变量名")?;
        self.expect_op(Op::Eq, "赋值需要 `=`")?;
        let value = self.parse_expr()?;
        Ok(Stmt::Assign {
            target,
            value,
            suffix,
            line,
        })
    }

    fn parse_dim(&mut self, line: usize, static_: bool) -> Result<Stmt, ParseError> {
        let mut items = Vec::new();
        loop {
            let (name, suffix) = self.expect_ident_typed("变量名")?;
            let bounds = if self.at_op(Op::LParen) {
                Some(self.parse_bounds()?)
            } else {
                None
            };
            let (ty, record, new_object) = if self.eat_kw(Kw::As) {
                if self.at_ident("new") {
                    // `Dim c As New Collection` / `Dim p As New Point3D`
                    self.advance();
                    (VbType::Object, None, Some(self.parse_new_object()?))
                } else {
                    let (ty, record) = self.parse_declared_type()?;
                    (ty, record, None)
                }
            } else {
                (
                    suffix
                        .and_then(VbType::from_suffix)
                        .unwrap_or(VbType::Variant),
                    None,
                    None,
                )
            };
            items.push(Declared {
                name,
                ty,
                record,
                bounds,
                new_object,
                with_events: false,
            });
            if !self.eat_op(Op::Comma) {
                break;
            }
        }
        Ok(Stmt::Dim {
            items,
            static_,
            line,
        })
    }

    fn parse_redim(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let preserve = self.eat_kw(Kw::Preserve);
        let name = self.expect_ident("数组名")?;
        let bounds = self.parse_bounds()?;
        if bounds.is_empty() {
            return Err(ParseError::new("`ReDim` 需要至少一个维度，例如 `ReDim a(10)`", line));
        }
        let ty = if self.eat_kw(Kw::As) {
            Some(self.parse_type()?)
        } else {
            None
        };
        Ok(Stmt::Redim {
            name,
            bounds,
            ty,
            preserve,
            line,
        })
    }

    /// `(10)`、`(1 To 10, 0 To 3)`、`()`：解析维度上下界。
    fn parse_bounds(&mut self) -> Result<Vec<(Expr, Expr)>, ParseError> {
        self.expect_op(Op::LParen, "数组需要 `(`")?;
        self.skip_newlines();
        let mut bounds = Vec::new();
        if self.eat_op(Op::RParen) {
            return Ok(bounds);
        }
        loop {
            let first = self.parse_expr()?;
            let (lower, upper) = if self.eat_kw(Kw::To) {
                (first, self.parse_expr()?)
            } else {
                (Expr::Literal(Value::Int(self.option_base)), first)
            };
            bounds.push((lower, upper));
            self.skip_newlines();
            if !self.eat_op(Op::Comma) {
                break;
            }
            self.skip_newlines();
        }
        self.expect_op(Op::RParen, "数组维度需要 `)`")?;
        Ok(bounds)
    }

    fn parse_const(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let (name, suffix) = self.expect_ident_typed("常量名")?;
        let ty = if self.eat_kw(Kw::As) {
            self.parse_type()?
        } else {
            suffix
                .and_then(VbType::from_suffix)
                .unwrap_or(VbType::Variant)
        };
        self.expect_op(Op::Eq, "常量需要 `=`")?;
        let value = self.parse_expr()?;
        Ok(Stmt::Const { name, ty, value, line })
    }

    fn parse_print(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let (values, seps, suppress_newline) = self.parse_print_values()?;
        Ok(Stmt::Print {
            values,
            seps,
            suppress_newline,
            line,
        })
    }

    /// `Print` / `Print #` 的值列表：`,` 与 `;` 都能分隔，末尾的 `;` 表示不换行。
    fn parse_print_values(&mut self) -> Result<(Vec<Expr>, Vec<PrintSep>, bool), ParseError> {
        let mut values = Vec::new();
        let mut seps = Vec::new();
        let mut suppress_newline = false;
        while !self.at_statement_end() {
            values.push(self.parse_expr()?);
            if self.eat_op(Op::Comma) {
                seps.push(PrintSep::Comma);
                suppress_newline = false;
                continue;
            }
            if self.eat_op(Op::Semicolon) {
                seps.push(PrintSep::Semicolon);
                suppress_newline = true;
                continue;
            }
            break;
        }
        // 分隔符个数是 `values - 1`；末尾那个只影响换行。
        if suppress_newline && seps.len() == values.len() {
            seps.pop();
        } else {
            suppress_newline = false;
        }
        Ok((values, seps, suppress_newline))
    }

    /// `Open 路径 For Input|Output|Append|Binary|Random [As #n] [Len = 记录长度]`。
    fn parse_open(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let path = self.parse_expr()?;
        self.expect_kw(Kw::For, "`Open` 需要 `For`")?;
        let mode = if self.eat_kw(Kw::Output) {
            FileMode::Output
        } else if self.eat_kw(Kw::Input) {
            FileMode::Input
        } else if self.eat_kw(Kw::Append) {
            FileMode::Append
        } else if self.at_ident("binary") || self.at_ident("random") {
            let name = self.expect_ident("模式")?;
            if name.eq_ignore_ascii_case("binary") {
                FileMode::Binary
            } else {
                FileMode::Random
            }
        } else {
            return Err(ParseError::new(
                "`Open` 的模式只支持 Input / Output / Append / Binary / Random",
                line,
            ));
        };
        // `As #n` 可以省略：运行时自动分配一个空闲文件号（与 `FreeFile` 相同）。
        let number = if self.eat_kw(Kw::As) {
            self.expect_op(Op::Hash, "`As` 后需要 `#`")?;
            Some(self.parse_expr()?)
        } else {
            None
        };
        let record_len = if self.at_ident("len") {
            self.expect_ident("Len")?;
            self.expect_op(Op::Eq, "`Len` 后需要 `=`")?;
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Stmt::OpenFile {
            path,
            mode,
            number,
            record_len,
            line,
        })
    }

    /// `Close [#n, ...]`，不带参数时关闭全部文件。
    fn parse_close(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let mut numbers = Vec::new();
        if !self.at_statement_end() {
            loop {
                self.eat_op(Op::Hash);
                numbers.push(self.parse_expr()?);
                if !self.eat_op(Op::Comma) {
                    break;
                }
            }
        }
        Ok(Stmt::CloseFile { numbers, line })
    }

    /// `Get #n, [位置], 变量`。
    fn parse_get(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let (number, position) = self.parse_file_position()?;
        let target = self.parse_expr()?;
        Ok(Stmt::GetFile {
            number,
            position,
            target,
            line,
        })
    }

    /// `Put #n, [位置], 表达式`。
    fn parse_put(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let (number, position) = self.parse_file_position()?;
        let value = self.parse_expr()?;
        Ok(Stmt::PutFile {
            number,
            position,
            value,
            line,
        })
    }

    /// `Seek #n, 位置`。
    fn parse_seek(&mut self, line: usize) -> Result<Stmt, ParseError> {
        self.expect_op(Op::Hash, "`Seek` 需要 `#`")?;
        let number = self.parse_expr()?;
        self.expect_op(Op::Comma, "`Seek` 需要逗号")?;
        let position = self.parse_expr()?;
        Ok(Stmt::SeekFile {
            number,
            position,
            line,
        })
    }

    /// `Get`/`Put` 的公共前缀 `#n, [位置],`；位置省略时返回 `None`。
    fn parse_file_position(&mut self) -> Result<(Expr, Option<Expr>), ParseError> {
        self.expect_op(Op::Hash, "需要 `#`")?;
        let number = self.parse_expr()?;
        self.expect_op(Op::Comma, "文件号后需要逗号")?;
        if self.eat_op(Op::Comma) {
            return Ok((number, None));
        }
        let position = self.parse_expr()?;
        self.expect_op(Op::Comma, "位置后需要逗号")?;
        Ok((number, Some(position)))
    }

    fn parse_inline_statements(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut stmts = Vec::new();
        loop {
            if self.at_eof() || self.at_newline() || self.at_kw(Kw::Else) {
                break;
            }
            if matches!(self.peek(), TokenKind::Colon) {
                self.advance();
                continue;
            }
            let before = self.pos;
            stmts.push(self.parse_statement()?);
            if self.pos == before {
                break;
            }
            if matches!(self.peek(), TokenKind::Colon) {
                continue;
            }
            break;
        }
        Ok(stmts)
    }

    fn parse_if(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let cond = self.parse_expr()?;
        self.expect_kw(Kw::Then, "`If` 后需要 `Then`")?;

        // 单行形式：`If a Then b Else c`
        if !self.at_newline() && !self.at_eof() {
            let body = self.parse_inline_statements()?;
            let otherwise = if self.eat_kw(Kw::Else) {
                Some(self.parse_inline_statements()?)
            } else {
                None
            };
            return Ok(Stmt::If {
                branches: vec![(cond, body)],
                otherwise,
                line,
            });
        }

        self.eat_line_end();
        let mut branches = vec![(
            cond,
            self.parse_block(&[Stop::ElseIf, Stop::Else, Stop::EndIf])?,
        )];
        let mut otherwise = None;
        loop {
            if self.eat_kw(Kw::ElseIf) {
                let cond = self.parse_expr()?;
                self.expect_kw(Kw::Then, "`ElseIf` 后需要 `Then`")?;
                self.eat_line_end();
                let body = self.parse_block(&[Stop::ElseIf, Stop::Else, Stop::EndIf])?;
                branches.push((cond, body));
                continue;
            }
            if self.eat_kw(Kw::Else) {
                self.eat_line_end();
                otherwise = Some(self.parse_block(&[Stop::EndIf])?);
            }
            break;
        }
        self.expect_kw(Kw::End, "`If` 块缺少 `End If`")?;
        self.expect_kw(Kw::If, "`If` 块缺少 `End If`")?;
        Ok(Stmt::If {
            branches,
            otherwise,
            line,
        })
    }

    fn parse_for(&mut self, line: usize) -> Result<Stmt, ParseError> {
        if self.eat_kw(Kw::Each) {
            let var = self.expect_ident("循环变量")?;
            self.expect_kw(Kw::In, "`For Each` 需要 `In`")?;
            let collection = self.parse_expr()?;
            self.eat_line_end();
            let body = self.parse_block(&[Stop::Next])?;
            let next_line = self.line();
            self.expect_kw(Kw::Next, "`For Each` 缺少 `Next`")?;
            if matches!(self.peek(), TokenKind::Ident(_)) && self.line() == next_line {
                self.advance();
            }
            return Ok(Stmt::ForEach {
                var,
                collection,
                body,
                line,
            });
        }
        let var = self.expect_ident("循环变量")?;
        self.expect_op(Op::Eq, "`For` 需要 `=`")?;
        let start = self.parse_expr()?;
        self.expect_kw(Kw::To, "`For` 需要 `To`")?;
        let end = self.parse_expr()?;
        let step = if self.eat_kw(Kw::Step) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.eat_line_end();
        let body = self.parse_block(&[Stop::Next])?;
        let next_line = self.line();
        self.expect_kw(Kw::Next, "`For` 缺少 `Next`")?;
        // `Next i`：只有同一行上的标识符才是循环变量。
        if matches!(self.peek(), TokenKind::Ident(_)) && self.line() == next_line {
            self.advance();
        }
        Ok(Stmt::For {
            var,
            start,
            end,
            step,
            body,
            line,
        })
    }

    fn parse_do(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let mut pre = None;
        if self.eat_kw(Kw::While) {
            pre = Some((true, self.parse_expr()?));
        } else if self.eat_kw(Kw::Until) {
            pre = Some((false, self.parse_expr()?));
        }
        self.eat_line_end();
        let body = self.parse_block(&[Stop::Loop])?;
        self.expect_kw(Kw::Loop, "`Do` 缺少 `Loop`")?;
        let mut post = None;
        if self.eat_kw(Kw::While) {
            post = Some((true, self.parse_expr()?));
        } else if self.eat_kw(Kw::Until) {
            post = Some((false, self.parse_expr()?));
        }
        Ok(Stmt::DoLoop {
            pre,
            post,
            body,
            line,
        })
    }

    fn parse_while(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let cond = self.parse_expr()?;
        self.eat_line_end();
        let body = self.parse_block(&[Stop::Wend])?;
        self.expect_kw(Kw::Wend, "`While` 缺少 `Wend`")?;
        Ok(Stmt::While { cond, body, line })
    }

    fn parse_select(&mut self, line: usize) -> Result<Stmt, ParseError> {
        self.expect_kw(Kw::Case, "`Select` 后需要 `Case`")?;
        let subject = self.parse_expr()?;
        self.eat_line_end();

        let mut arms = Vec::new();
        let mut otherwise = None;
        loop {
            self.skip_separators();
            if self.at_stop(&[Stop::EndSelect]) || self.at_eof() {
                break;
            }
            if !self.eat_kw(Kw::Case) {
                return Err(ParseError::new(
                    format!("`Select Case` 里需要 `Case`，但遇到 {}", self.token().describe()),
                    self.line(),
                ));
            }
            if self.eat_kw(Kw::Else) {
                self.eat_line_end();
                otherwise = Some(self.parse_block(&[Stop::Case, Stop::EndSelect])?);
                continue;
            }
            let mut patterns = Vec::new();
            loop {
                if self.eat_kw(Kw::Is) {
                    let op = self.parse_comparison_op()?;
                    patterns.push(CasePattern::Is(op, self.parse_expr()?));
                } else {
                    let first = self.parse_expr()?;
                    if self.eat_kw(Kw::To) {
                        patterns.push(CasePattern::Range(first, self.parse_expr()?));
                    } else {
                        patterns.push(CasePattern::Value(first));
                    }
                }
                if !self.eat_op(Op::Comma) {
                    break;
                }
            }
            self.eat_line_end();
            let body = self.parse_block(&[Stop::Case, Stop::EndSelect])?;
            arms.push((patterns, body));
        }
        self.expect_kw(Kw::End, "`Select Case` 缺少 `End Select`")?;
        self.expect_kw(Kw::Select, "`Select Case` 缺少 `End Select`")?;
        Ok(Stmt::Select {
            subject,
            arms,
            otherwise,
            line,
        })
    }

    fn parse_comparison_op(&mut self) -> Result<BinOp, ParseError> {
        let op = match self.peek() {
            TokenKind::Op(op) => *op,
            _ => {
                return Err(ParseError::new(
                    format!("`Case Is` 后需要比较运算符，但遇到 {}", self.token().describe()),
                    self.line(),
                ));
            }
        };
        let bin = match op {
            Op::Eq => BinOp::Eq,
            Op::NotEq => BinOp::NotEq,
            Op::Lt => BinOp::Lt,
            Op::Gt => BinOp::Gt,
            Op::LtEq => BinOp::LtEq,
            Op::GtEq => BinOp::GtEq,
            _ => {
                return Err(ParseError::new(
                    format!("`Case Is` 不支持运算符 `{}`", op.text()),
                    self.line(),
                ));
            }
        };
        self.advance();
        Ok(bin)
    }

    fn parse_exit(&mut self, line: usize) -> Result<Stmt, ParseError> {
        let kind = match self.peek() {
            TokenKind::Kw(Kw::For) => ExitKind::For,
            TokenKind::Kw(Kw::Do) => ExitKind::Do,
            TokenKind::Kw(Kw::Sub) => ExitKind::Sub,
            TokenKind::Kw(Kw::Function) => ExitKind::Function,
            TokenKind::Kw(Kw::Property) => ExitKind::Sub,
            _ => {
                return Err(ParseError::new(
                    format!(
                        "`Exit` 后需要 For/Do/Sub/Function/Property，但遇到 {}",
                        self.token().describe()
                    ),
                    line,
                ));
            }
        };
        self.advance();
        Ok(Stmt::Exit { kind, line })
    }

    // ---------- 表达式 ----------

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary(0)
    }

    fn peek_binary(&self) -> Option<(BinOp, u8)> {
        match self.peek() {
            TokenKind::Op(op) => match op {
                Op::Plus => Some((BinOp::Add, 5)),
                Op::Minus => Some((BinOp::Subtract, 5)),
                Op::Star => Some((BinOp::Multiply, 8)),
                Op::Slash => Some((BinOp::Divide, 8)),
                Op::Backslash => Some((BinOp::IntDivide, 7)),
                Op::Caret => Some((BinOp::Power, 9)),
                Op::Amp => Some((BinOp::Concat, 4)),
                Op::Eq => Some((BinOp::Eq, 3)),
                Op::NotEq => Some((BinOp::NotEq, 3)),
                Op::Lt => Some((BinOp::Lt, 3)),
                Op::Gt => Some((BinOp::Gt, 3)),
                Op::LtEq => Some((BinOp::LtEq, 3)),
                Op::GtEq => Some((BinOp::GtEq, 3)),
                _ => None,
            },
            TokenKind::Kw(Kw::Mod) => Some((BinOp::Modulo, 6)),
            TokenKind::Kw(Kw::And) => Some((BinOp::And, 2)),
            TokenKind::Kw(Kw::Or) => Some((BinOp::Or, 1)),
            TokenKind::Kw(Kw::Xor) => Some((BinOp::Xor, 1)),
            // `obj Is Nothing` / `a Is b`：对象同一性，优先级与比较运算相同。
            TokenKind::Kw(Kw::Is) => Some((BinOp::Is, 3)),
            _ => None,
        }
    }

    fn parse_binary(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        while let Some((op, bp)) = self.peek_binary() {
            if bp < min_bp {
                break;
            }
            self.advance();
            let right_bp = if op == BinOp::Power { bp } else { bp + 1 };
            let right = self.parse_binary(right_bp)?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.eat_op(Op::Minus) {
            return Ok(Expr::Unary {
                op: UnOp::Negate,
                expr: Box::new(self.parse_binary(9)?),
            });
        }
        if self.eat_op(Op::Plus) {
            return self.parse_unary();
        }
        if self.eat_kw(Kw::Not) {
            return Ok(Expr::Unary {
                op: UnOp::Not,
                expr: Box::new(self.parse_binary(3)?),
            });
        }
        if self.eat_kw(Kw::TypeOf) {
            // `TypeOf 对象 Is 类名`：按 4 级优先级解析操作数，免得把 `Is` 吃掉。
            let target = self.parse_binary(4)?;
            self.expect_kw(Kw::Is, "`TypeOf` 需要 `Is 类名`")?;
            let class_name = self.expect_ident("类名")?;
            return Ok(Expr::TypeOf {
                expr: Box::new(target),
                class_name,
            });
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let mut expr = match self.peek().clone() {
            TokenKind::Int(value) => {
                self.advance();
                Expr::Literal(Value::Int(value))
            }
            TokenKind::Float(value) => {
                self.advance();
                Expr::Literal(Value::Float(value))
            }
            TokenKind::Str(text) => {
                self.advance();
                Expr::Literal(Value::Str(text))
            }
            TokenKind::Kw(Kw::True) => {
                self.advance();
                Expr::Literal(Value::Bool(true))
            }
            TokenKind::Kw(Kw::False) => {
                self.advance();
                Expr::Literal(Value::Bool(false))
            }
            TokenKind::Kw(Kw::Null) => {
                self.advance();
                Expr::Literal(Value::Null)
            }
            TokenKind::Kw(Kw::Empty | Kw::Nothing) => {
                self.advance();
                Expr::Literal(Value::Empty)
            }
            TokenKind::Ident(name) if name == "new" => {
                self.advance();
                Expr::NewObject(self.parse_new_object()?)
            }
            TokenKind::Ident(name) => {
                self.advance();
                if self.at_op(Op::LParen) {
                    let args = self.parse_paren_args()?;
                    Expr::Call { name, args }
                } else {
                    Expr::Var(name)
                }
            }
            TokenKind::Kw(Kw::AddressOf) => {
                self.advance();
                let (name, _) = self.expect_ident_typed("过程名")?;
                Expr::AddressOf(name)
            }
            // `Error(5)`：`Error` 是 `On Error` 的关键字，带括号时当内置函数用。
            TokenKind::Kw(Kw::Error) if matches!(self.peek_at(1), TokenKind::Op(Op::LParen)) => {
                self.advance();
                let args = self.parse_paren_args()?;
                Expr::Call {
                    name: "error".into(),
                    args,
                }
            }
            TokenKind::Op(Op::LParen) => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect_op(Op::RParen, "括号没有闭合")?;
                expr
            }
            TokenKind::Op(Op::Dot) => {
                // `With` 块里的相对成员：`.X`、`.A.B`。
                let mut path = Vec::new();
                while self.eat_op(Op::Dot) {
                    path.push(self.expect_ident("成员名")?);
                }
                Expr::WithMember { path }
            }
            _ => {
                return Err(ParseError::new(
                    format!("需要表达式，但遇到 {}", self.token().describe()),
                    self.line(),
                ));
            }
        };

        // `p.X`、`l.A.X`：记录成员访问（`p.Items(1)` 是数组字段的元素）。
        while self.eat_op(Op::Dot) {
            let name = self.expect_ident("成员名")?;
            let base = Box::new(expr);
            expr = if self.at_op(Op::LParen) {
                let indices = self.parse_paren_args()?;
                Expr::MemberIndex {
                    base,
                    name,
                    indices,
                }
            } else {
                Expr::Member { base, name }
            };
        }
        Ok(expr)
    }

    fn parse_paren_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        self.expect_op(Op::LParen, "调用需要 `(`")?;
        self.skip_newlines();
        let mut args = Vec::new();
        if self.eat_op(Op::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            self.skip_newlines();
            if !self.eat_op(Op::Comma) {
                break;
            }
            self.skip_newlines();
        }
        self.expect_op(Op::RParen, "调用需要 `)`")?;
        Ok(args)
    }
}
