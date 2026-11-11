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
fn module_level_declarations_are_globals_and_main_still_runs() {
    let source = r#"
Option Explicit
Private counter As Long
Private greeting As String
Private Const STEP_SIZE As Long = 2

Sub Main()
    counter = counter + STEP_SIZE
    greeting = "次数"
    Debug.Print greeting & counter
End Sub
"#;
    assert_eq!(output(source), ["次数2"]);
}

#[test]
fn address_of_accepts_four_parameter_callback() {
    let source = r#"
Function Four(ByVal a As Long, ByVal b As Long, ByVal c As Long, ByVal d As Long) As Long
    Four = a + b + c + d
End Function
Debug.Print AddressOf Four
"#;
    assert_eq!(output(source), ["AddressOf four"]);
}

#[test]
fn address_of_four_parameter_callback_receives_arguments() {
    let source = r#"
Declare PtrSafe Function EnumLanguageGroupLocalesW Lib "kernel32" (ByVal proc As LongPtr, ByVal group As Long, ByVal flags As Long, ByVal param As LongPtr) As Long
Private g_group As Long
Private g_locale As Long
Private g_name As String
Private g_param As LongPtr

Function OnGroupLocale(ByVal group As Long, ByVal locale As Long, ByVal name As String, ByVal param As LongPtr) As Long
    If Len(name) > 0 Then
        g_group = group
        g_locale = locale
        g_name = name
        g_param = param
        OnGroupLocale = 0
    Else
        OnGroupLocale = 1
    End If
End Function

EnumLanguageGroupLocalesW AddressOf OnGroupLocale, 1, 0, 123
Debug.Print g_group = 1
Debug.Print g_locale <> 0
Debug.Print Len(g_name) > 0
Debug.Print g_param = 123
"#;
    assert_eq!(output(source), ["True", "True", "True", "True"]);
}

#[test]
fn strptr_points_at_a_wide_string_buffer() {
    let source = r#"
Declare PtrSafe Function lstrlenW Lib "kernel32" (ByVal text As LongPtr) As Long
Private Type Holder
    Ptr As LongPtr
End Type

Dim h As Holder
h.Ptr = StrPtr("你好abc")
Debug.Print lstrlenW(h.Ptr)
Debug.Print lstrlenW(StrPtr("abc"))
"#;
    assert_eq!(output(source), ["5", "3"]);
}

#[test]
fn declare_supports_fourteen_parameters() {
    let source = r#"
Declare PtrSafe Function CreateFontW Lib "gdi32" (ByVal nHeight As Long, ByVal nWidth As Long, ByVal nEscapement As Long, ByVal nOrientation As Long, ByVal fnWeight As Long, ByVal fdwItalic As Long, ByVal fdwUnderline As Long, ByVal fdwStrikeOut As Long, ByVal fdwCharSet As Long, ByVal fdwOutputPrecision As Long, ByVal fdwClipPrecision As Long, ByVal fdwQuality As Long, ByVal fdwPitchAndFamily As Long, ByVal lpszFace As String) As LongPtr
Declare PtrSafe Function DeleteObject Lib "gdi32" (ByVal hObject As LongPtr) As Long
Dim h As LongPtr
h = CreateFontW(-16, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, "微软雅黑")
Debug.Print h <> 0
Debug.Print DeleteObject(h) <> 0
"#;
    assert_eq!(output(source), ["True", "True"]);
}

#[test]
fn address_of_stored_in_a_record_gets_a_persistent_trampoline() {
    let source = r#"
Private Type WNDCLASSW
    style As Long
    lpfnWndProc As LongPtr
    cbClsExtra As Long
    cbWndExtra As Long
    hInstance As LongPtr
    hIcon As LongPtr
    hCursor As LongPtr
    hbrBackground As LongPtr
    lpszMenuName As LongPtr
    lpszClassName As LongPtr
End Type

Declare PtrSafe Function GetModuleHandleW Lib "kernel32" (ByVal name As LongPtr) As LongPtr
Declare PtrSafe Function DefWindowProcW Lib "user32" (ByVal hWnd As LongPtr, ByVal msg As Long, ByVal wParam As LongPtr, ByVal lParam As LongPtr) As LongPtr
Declare PtrSafe Function RegisterClassW Lib "user32" (ByRef wc As WNDCLASSW) As Long
Declare PtrSafe Function UnregisterClassW Lib "user32" (ByVal name As LongPtr, ByVal instance As LongPtr) As Long
Declare PtrSafe Function CreateWindowExW Lib "user32" (ByVal exStyle As Long, ByVal className As LongPtr, ByVal windowName As String, ByVal style As Long, ByVal x As Long, ByVal y As Long, ByVal w As Long, ByVal h As Long, ByVal parent As LongPtr, ByVal menu As LongPtr, ByVal instance As LongPtr, ByVal param As LongPtr) As LongPtr
Declare PtrSafe Function DestroyWindow Lib "user32" (ByVal hWnd As LongPtr) As Long

Private g_calls As Long

Function DummyProc(ByVal hWnd As LongPtr, ByVal msg As Long, ByVal wParam As LongPtr, ByVal lParam As LongPtr) As LongPtr
    g_calls = g_calls + 1
    DummyProc = DefWindowProcW(hWnd, msg, wParam, lParam)
End Function

Dim wc As WNDCLASSW
Dim hInst As LongPtr
Dim hWnd As LongPtr
Dim className As String
className = "QideInterpreterCallbackTest"
hInst = GetModuleHandleW(0)
wc.lpfnWndProc = AddressOf DummyProc
wc.hInstance = hInst
wc.lpszClassName = StrPtr(className)
Debug.Print RegisterClassW(wc) <> 0
hWnd = CreateWindowExW(0, StrPtr(className), "test", 0, 0, 0, 100, 100, 0, 0, hInst, 0)
Debug.Print hWnd <> 0
DestroyWindow hWnd
Debug.Print g_calls > 0
Debug.Print UnregisterClassW(StrPtr(className), hInst) <> 0
"#;
    assert_eq!(output(source), ["True", "True", "True", "True"]);
}

#[test]
fn hex_literals_accept_type_suffix() {
    assert_eq!(output("Debug.Print &HFFFF& And &HFF"), ["255"]);
    assert_eq!(output("Debug.Print &H10000& \\ &H100&"), ["256"]);
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
