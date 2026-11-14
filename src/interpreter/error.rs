//! 解释器错误：解析错误与运行时错误都带行号。

use std::fmt;

/// 语法错误（词法 / 语法分析阶段）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
}

impl ParseError {
    pub fn new(message: impl Into<String>, line: usize) -> Self {
        Self {
            message: message.into(),
            line,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "第 {} 行语法错误: {}", self.line, self.message)
    }
}

/// 运行时错误（求值阶段）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
    pub line: usize,
    /// VB 错误号，供 `Err.Number` 与 `Error()` 使用。
    pub number: i64,
    /// `Err.Raise` 传入的错误来源（`Err.Source`）。
    pub source: Option<String>,
}

impl RuntimeError {
    /// 默认错误号 5：VB 的「无效的过程调用或参数」。
    pub fn new(message: impl Into<String>, line: usize) -> Self {
        Self {
            message: message.into(),
            line,
            number: 5,
            source: None,
        }
    }

    /// 把值转换类错误转成运行时错误，并按 VB 约定补上错误号：
    /// 溢出 6、类型不匹配 13、无效使用 `Null` 94，其余为 5。
    pub fn from_coerce(message: impl Into<String>, line: usize) -> Self {
        let message = message.into();
        let number = if message.contains("溢出") {
            6
        } else if message.contains("类型不匹配") {
            13
        } else if message.contains("Null 不能") || message.contains("无效使用 Null") {
            94
        } else {
            5
        };
        Self::new(message, line).with_number(number)
    }

    pub fn with_number(mut self, number: i64) -> Self {
        self.number = number;
        self
    }

    pub fn with_source(mut self, source: Option<String>) -> Self {
        self.source = source;
        self
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "第 {} 行运行时错误: {}", self.line, self.message)
    }
}

/// 解释器的统一错误类型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Parse(ParseError),
    Runtime(RuntimeError),
}

impl Error {
    /// 错误所在的行号，供界面定位。
    pub fn line(&self) -> usize {
        match self {
            Self::Parse(error) => error.line,
            Self::Runtime(error) => error.line,
        }
    }

    #[allow(dead_code)] // 供测试与界面显示错误文本。
    pub fn message(&self) -> &str {
        match self {
            Self::Parse(error) => &error.message,
            Self::Runtime(error) => &error.message,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::Runtime(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<ParseError> for Error {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<RuntimeError> for Error {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}
