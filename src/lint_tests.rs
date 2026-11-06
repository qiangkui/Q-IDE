//! VB 实时语法检查的测试。

use crate::lint::analyze;

#[test]
fn valid_program_has_no_issue() {
    assert!(analyze("Dim x As Long\nx = 1\nMsgBox x\n").issues.is_empty());
}

#[test]
fn empty_source_is_valid() {
    assert!(analyze("").issues.is_empty());
}

#[test]
fn missing_expression_reports_its_line() {
    let issue = analyze("Dim x As Long\nx = \n")
        .issues
        .into_iter()
        .next()
        .expect("应当报语法错误");
    assert_eq!(issue.line, 2);
    assert!(!issue.message.is_empty());
}

#[test]
fn undeclared_variable_is_reported_live() {
    let issues = analyze("Option Explicit\nx = 1\n").issues;
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].line, 2);
    assert!(issues[0].message.contains("`x`"));
}
