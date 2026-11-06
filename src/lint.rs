//! VB 脚本的实时检查：语法错误 + `Option Explicit` 下的语义问题。
//!
//! 菜单里的「检查语法」只在手动触发时报告一次；这里把同一套解析器接到编辑框上，
//! 内容一变就重新检查，把发现的问题挂成编辑器诊断（波浪线 + 悬停提示）。

use gpui_kit::Context;
use gpui_kit::base::input::{Diagnostic, DiagnosticSeverity, EditorState, Position, RopeExt as _};

use crate::interpreter::{parse, sema};

/// 一次最多挂多少个诊断，避免大脚本把编辑器压垮。
const MAX_DIAGNOSTICS: usize = 100;

/// 超过这个字节数的脚本不再随输入实时检查：大文件每敲一个字都重新解析会卡。
/// 这种文件仍可以用菜单「运行 → 检查语法」手动检查。
const MAX_LIVE_CHECK_BYTES: usize = 400_000;

/// 一次检查发现的语法错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxIssue {
    /// 出错行号（1 基，与解释器的错误信息一致）。
    pub line: usize,
    pub message: String,
}

/// 一次检查的完整结果。
pub struct Analysis {
    /// 语法错误或语义问题（按行去重）。
    pub issues: Vec<SyntaxIssue>,
    /// 解析成功时的语法树，供「检查语法」显示统计信息。
    pub program: Option<crate::interpreter::ast::Program>,
}

/// 解析 + 语义检查：语法错误只报第一处，语义问题按行去重后全部给出。
pub fn analyze(source: &str) -> Analysis {
    match parse(source) {
        Ok(program) => Analysis {
            issues: sema::analyze(&program)
                .into_iter()
                .map(|issue| SyntaxIssue {
                    line: issue.line,
                    message: issue.message,
                })
                .collect(),
            program: Some(program),
        },
        Err(error) => Analysis {
            issues: vec![SyntaxIssue {
                line: error.line,
                message: error.message,
            }],
            program: None,
        },
    }
}

/// 重新检查编辑器内容并刷新诊断，返回发现的问题。
pub fn refresh(editor: &mut EditorState, cx: &mut Context<EditorState>) -> Vec<SyntaxIssue> {
    let text = editor.text().clone();
    if text.len() > MAX_LIVE_CHECK_BYTES {
        clear(editor, cx);
        return Vec::new();
    }

    let issues = analyze(&text.to_string()).issues;

    let Some(diagnostics) = editor.diagnostics_mut() else {
        return issues;
    };
    diagnostics.reset(&text);
    for issue in issues.iter().take(MAX_DIAGNOSTICS) {
        // 行号来自解析器/语义分析，正常情况下一定落在文本范围内，这里再兜一次底。
        let row = issue
            .line
            .saturating_sub(1)
            .min(text.lines_len().saturating_sub(1));
        let characters = text.slice_line(row).chars().count() as u32;
        diagnostics.push(
            Diagnostic::new(
                Position::new(row as u32, 0)..Position::new(row as u32, characters),
                issue.message.clone(),
            )
            .with_severity(DiagnosticSeverity::Error)
            .with_source("vb"),
        );
    }
    cx.notify();
    issues
}

/// 清空编辑器的诊断（例如切换到非 VB 文件时）。
pub fn clear(editor: &mut EditorState, cx: &mut Context<EditorState>) {
    if let Some(diagnostics) = editor.diagnostics_mut() {
        diagnostics.clear();
        cx.notify();
    }
}
