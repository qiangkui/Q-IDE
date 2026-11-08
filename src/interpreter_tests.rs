//! VB 解释器的语言特性测试：数组字面量 `{…}` 与 `Dim … = …` 初值。

use std::sync::atomic::AtomicBool;

use crate::interpreter::run;

fn output(source: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let flag = AtomicBool::new(false);
    run(source, |line| lines.push(line.to_string()), &flag).expect("脚本应当执行成功");
    lines
}

#[test]
fn array_literal_declaration_runs() {
    let source = "Dim arr As Integer() = {64, 34, 25, 12, 22, 11, 90}";
    assert_eq!(output(&format!("{source}\nDebug.Print arr(0)")), ["64"]);
    assert_eq!(output(&format!("{source}\nDebug.Print arr(6)")), ["90"]);
    assert_eq!(output(&format!("{source}\nDebug.Print UBound(arr)")), ["6"]);
    assert_eq!(output(&format!("{source}\nDebug.Print LBound(arr)")), ["0"]);
}

#[test]
fn array_literal_items_may_be_expressions_and_strings() {
    assert_eq!(output("Dim a = {1 + 1, 2 * 3}\nDebug.Print a(1)"), ["6"]);
    assert_eq!(
        output("Dim s As String() = {\"甲\", \"乙\"}\nDebug.Print s(1)"),
        ["乙"]
    );
}

#[test]
fn vb6_dynamic_array_can_be_assigned_a_literal() {
    assert_eq!(
        output("Dim a() As Integer\na = {7, 8, 9}\nDebug.Print a(2)"),
        ["9"]
    );
}

#[test]
fn array_literal_edges_parse() {
    // 尾逗号与空字面量都按 VB.NET 的习惯接受。
    assert!(crate::interpreter::parse("Dim a = {1, 2,}").is_ok());
    assert!(crate::interpreter::parse("Dim a = {}").is_ok());
    // 括号没闭合时报错，而不是静默吞掉。
    assert!(crate::interpreter::parse("Dim a = {1, 2").is_err());
}

#[test]
fn array_literal_works_inside_procedures() {
    let source = "Sub Main()\n    Dim arr As Integer() = {3, 1, 2}\n    Debug.Print arr(0)\nEnd Sub\nMain";
    assert_eq!(output(source), ["3"]);
}

#[test]
fn procedure_parameter_accepts_type_suffix_parens() {
    // 形如 `Sub BubbleSort(ByRef arr As Integer())`：类型后面的空括号表示数组参数。
    let source = "Sub BubbleSort(ByRef arr As Integer())\n    arr(0) = 99\nEnd Sub\n\
                  Dim a As Integer() = {1, 2}\nBubbleSort a\nDebug.Print a(0)";
    assert_eq!(output(source), ["99"]);

    // 老写法 `a() As Integer` 继续可用。
    let legacy = "Sub Bump(ByRef a() As Integer)\n    a(0) = 5\nEnd Sub\n\
                  Dim x As Integer() = {1}\nBump x\nDebug.Print x(0)";
    assert_eq!(output(legacy), ["5"]);
}

#[test]
fn array_parameter_has_no_lint_issues() {
    let analysis = crate::lint::analyze("Sub BubbleSort(ByRef arr As Integer())\nEnd Sub");
    assert!(
        analysis.issues.is_empty(),
        "不该有诊断：{:?}",
        analysis.issues
    );
}

#[test]
fn dim_inside_a_loop_is_declared_once() {
    // 循环体里的 `Dim` 每轮都会执行到，但 VB 的声明按过程算一次，不该报重复声明。
    let source = r#"
Sub Test()
    Dim i As Long
    For i = 0 To 2
        Dim swapped As Boolean
        swapped = True
        Debug.Print i
    Next i
End Sub
Test
"#;
    assert_eq!(output(source), ["0", "1", "2"]);
}

#[test]
fn dim_initializer_runs_on_every_iteration() {
    let source = r#"
Sub Test()
    Dim i As Long
    For i = 1 To 3
        Dim n As Long = i * 10
        Debug.Print n
    Next i
End Sub
Test
"#;
    assert_eq!(output(source), ["10", "20", "30"]);
}

#[test]
fn two_dims_on_different_lines_still_conflict() {
    let source = "Sub Bad()\n    Dim x As Long\n    Dim x As Long\nEnd Sub\nBad";
    let mut lines = Vec::new();
    let flag = AtomicBool::new(false);
    let error = run(source, |line| lines.push(line.to_string()), &flag)
        .expect_err("重复声明应当报错");
    assert!(
        error.to_string().contains("重复声明"),
        "错误信息应当说明重复声明：{error}"
    );
}

#[test]
fn bubble_sort_written_the_ai_way_runs() {
    // AI 常用的写法：类型后缀括号、`{…}` 字面量、循环体内的 `Dim`。
    let source = r#"
Sub BubbleSort(ByRef arr As Integer())
    Dim n As Integer = UBound(arr)
    Dim i As Integer, j As Integer
    Dim tmp As Integer
    For i = 0 To n - 1
        Dim swapped As Boolean
        swapped = False
        For j = 0 To n - 1 - i
            If arr(j) > arr(j + 1) Then
                tmp = arr(j)
                arr(j) = arr(j + 1)
                arr(j + 1) = tmp
                swapped = True
            End If
        Next j
        If Not swapped Then Exit For
    Next i
End Sub

Dim data As Integer() = {64, 34, 25, 12, 22, 11, 90}
BubbleSort data
Debug.Print data(0)
Debug.Print data(6)
"#;
    assert_eq!(output(source), ["11", "90"]);
}

#[test]
fn array_literal_produces_no_lint_issues() {
    let source = "Dim arr As Integer() = {64, 34, 25, 12, 22, 11, 90}\nDebug.Print arr(0)";
    let analysis = crate::lint::analyze(source);
    assert!(
        analysis.issues.is_empty(),
        "不该有诊断：{:?}",
        analysis.issues
    );
}
