//! 语义检查（未声明变量）的测试。

use crate::interpreter::{parse, sema};

fn issues(source: &str) -> Vec<sema::Issue> {
    let program = parse(source).expect("测试脚本应当能解析");
    sema::analyze(&program)
}

#[test]
fn undeclared_variable_is_reported_under_option_explicit() {
    let found = issues("Option Explicit\nx = 1\n");
    assert!(
        found
            .iter()
            .any(|issue| issue.line == 2 && issue.message.contains("`x`")),
        "{found:?}"
    );
}

#[test]
fn without_option_explicit_nothing_is_reported() {
    assert!(issues("x = 1\n").is_empty());
}

#[test]
fn app_path_is_a_known_object() {
    assert!(
        issues("Option Explicit\nDebug.Print App.Path\n").is_empty(),
        "App.Path 不应被当成未声明变量"
    );
}

#[test]
fn declared_variables_and_parameters_are_accepted() {
    let found = issues(
        "Option Explicit\n\
         Dim total As Long\n\
         Sub Foo(value As Long)\n\
             Dim local As Long\n\
             total = value + local\n\
         End Sub\n",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn for_loop_variable_must_be_declared() {
    let missing = issues("Option Explicit\nFor i = 1 To 3\nNext i\n");
    assert!(
        missing
            .iter()
            .any(|issue| issue.message.contains("`i`")),
        "{missing:?}"
    );

    let declared = issues("Option Explicit\nDim i As Long\nFor i = 1 To 3\nNext i\n");
    assert!(declared.is_empty(), "{declared:?}");
}

#[test]
fn class_fields_and_me_are_known() {
    let found = issues(
        "Option Explicit\n\
         Class Box\n\
             Public Value As Long\n\
             Public Sub Bump()\n\
                 Me.Value = Value + 1\n\
             End Sub\n\
         End Class\n",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn with_block_members_are_skipped() {
    let found = issues(
        "Option Explicit\n\
         Type Point\n\
             X As Long\n\
         End Type\n\
         Dim p As Point\n\
         With p\n\
             .X = 1\n\
         End With\n",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn builtin_names_are_accepted_as_values() {
    let found = issues("Option Explicit\nDim t As Double\nt = Timer\n");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn redim_of_undeclared_array_is_reported() {
    let found = issues("Option Explicit\nReDim a(10)\n");
    assert!(
        found
            .iter()
            .any(|issue| issue.message.contains("未定义的数组")),
        "{found:?}"
    );
}

#[test]
fn a_rich_script_has_no_false_positives() {
    let found = issues(
        "Option Explicit\n\
         Type Point\n\
             X As Long\n\
             Y As Long\n\
         End Type\n\
         \n\
         Enum Color\n\
             Red\n\
             Green\n\
         End Enum\n\
         \n\
         Dim total As Long\n\
         Dim names(1 To 3) As String\n\
         Dim p As Point\n\
         Dim box As New Collection\n\
         \n\
         Sub Fill()\n\
             Dim i As Long\n\
             Dim item As String\n\
             For i = 1 To 3\n\
                 names(i) = \"n\" & i\n\
             Next i\n\
             For Each item In names\n\
                 box.Add item\n\
             Next item\n\
         End Sub\n\
         \n\
         Function Sum(a As Long, Optional b As Long = 1) As Long\n\
             Dim result As Long\n\
             result = a + b\n\
             Sum = result\n\
         End Function\n\
         \n\
         Class Counter\n\
             Private Value As Long\n\
             Public Sub Bump()\n\
                 Me.Value = Value + 1\n\
             End Sub\n\
             Public Property Get Total() As Long\n\
                 Total = Value\n\
             End Property\n\
         End Class\n\
         \n\
         total = Sum(1) + Sum(2, 3)\n\
         p.X = total\n\
         With p\n\
             .Y = 2\n\
         End With\n\
         Select Case total\n\
             Case 1 To 5\n\
                 Debug.Print \"small\", Red\n\
             Case Else\n\
                 Debug.Print total, Green\n\
         End Select\n",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn unknown_procedure_call_is_reported() {
    let found = issues("Option Explicit\nSub Main()\n    Missing 1\nEnd Sub\n");
    assert!(
        found.iter().any(|issue| {
            issue.message.contains("未定义的函数或过程") && issue.message.contains("`missing`")
        }),
        "{found:?}"
    );
}

#[test]
fn known_calls_are_accepted() {
    let found = issues(
        "Option Explicit\n\
         Declare Function MessageBoxW Lib \"user32\" (ByVal hwnd As Long) As Long\n\
         Sub Helper()\n\
         End Sub\n\
         Sub Main()\n\
             Helper\n\
             MsgBox \"hi\"\n\
         End Sub\n",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn array_indexing_is_not_reported_as_a_call() {
    let found = issues("Option Explicit\nDim a(2) As Long\nDim x As Long\na(0) = 1\nx = a(0)\n");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn each_issue_is_reported_once_per_line() {
    let found = issues("Option Explicit\nx = x + x\n");
    assert_eq!(found.len(), 1, "{found:?}");
}
