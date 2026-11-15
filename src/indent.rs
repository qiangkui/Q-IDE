//! VB 脚本的块级自动缩进规则。
//!
//! 编辑器（`gpui-base`）在回车时会用语言配置里的两条正则算新行缩进：
//! 行首到光标的内容匹配「增加缩进」规则时多缩进一级；光标之后的内容匹配
//! 「减少缩进」规则时少缩进一级。规则只在语法上下文是代码时生效，
//! 注释与字符串里的关键字不会触发。
//!
//! 这里单独成模块是为了能脱离界面单测正则；`app::init` 把它们装进
//! `LanguageConfig` 并注册给 `vb` / `vb6` / `vba`。

use std::sync::OnceLock;

use gpui_kit::base::input::language_config::{IndentationRules, LanguageConfig};

/// 块开始（回车后新行多缩进一级）。
///
/// 覆盖 `Sub` / `Function` / `Property`、`Type` / `Class`、`Do` / `For` / `While` /
/// `With` / `Select Case`、`Case` / `Else`，以及整行以 `Then` 结尾的
/// `If ... Then` / `ElseIf ... Then`（单行 `If ... Then x = 1` 不会触发）。
pub const VB_INCREASE: &str = r"(?i)^\s*(?:(?:public|private|friend|static)\s+)*(?:sub|function|property)\b|^\s*(?:public|private)?\s*(?:type|class)\b|^\s*(?:do|for|while|with)\b|^\s*select\s+case\b|^\s*(?:case|else)\b|\bthen\s*$";

/// 块结束（在这些关键字之前回车时新行少缩进一级）。
///
/// 对应 `End Sub` / `End Function` / `End If` / `End With` / `End Select` /
/// `End Type` / `End Class` / `End Property`、`Next` / `Loop` / `Wend` 与单独的 `End`。
pub const VB_DECREASE: &str =
    r"(?i)^\s*(?:end(?:\s+(?:sub|function|if|with|select|type|class|property))?|next|loop|wend)\b";

fn increase_regex() -> &'static regex::Regex {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    REGEX.get_or_init(|| regex::Regex::new(VB_INCREASE).expect("VB_INCREASE 必须是合法正则"))
}

fn decrease_regex() -> &'static regex::Regex {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    REGEX.get_or_init(|| regex::Regex::new(VB_DECREASE).expect("VB_DECREASE 必须是合法正则"))
}

/// 整行是不是一个块结束行（`End Sub`、`Next`…）。
///
/// 编辑器的「减少缩进」规则只拿**光标之后**的文本去匹配，所以在 `End Sub` 行尾回车时
/// 光标后面是空的、不会触发；`CodeWindow` 在 `PressEnter` 之后用这个函数补一次反缩进。
pub fn closes_block(line: &str) -> bool {
    decrease_regex().is_match(line.trim_end())
}

/// 编辑器要用的语言配置：块级缩进规则 + 默认的括号配对。
pub fn language_config() -> LanguageConfig {
    LanguageConfig::default().indentation_rules(IndentationRules::new(
        increase_regex().clone(),
        decrease_regex().clone(),
    ))
}
