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
fn getobject_detects_existing_relative_files_as_paths() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let temp_dir = std::env::temp_dir().join(format!(
        "qide-getobject-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&temp_dir).unwrap();
    std::fs::write(temp_dir.join("document.dat"), []).unwrap();

    assert!(crate::interpreter::runtime::is_moniker_path(
        "document.dat",
        &temp_dir
    ));
    assert!(!crate::interpreter::runtime::is_moniker_path(
        "QIDE.Missing.Component",
        &temp_dir
    ));
    assert!(crate::interpreter::runtime::is_moniker_path(
        "document.dat!Item1",
        &temp_dir
    ));

    std::fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn currency_supports_fixed_point_coercion_arithmetic_and_suffix() {
    let source = r#"
Dim amount As Currency
amount = CCur(1.23456)
Debug.Print amount
Debug.Print TypeName(amount)
Debug.Print amount + CCur(2.5)
Debug.Print amount * 2
Debug.Print TypeName(CCur(1) / CCur(3))
Dim tagged@
tagged = 3.125
Debug.Print TypeName(tagged)
Debug.Print tagged
On Error Resume Next
amount = CCur(1e20)
Debug.Print Err.Number
"#;
    assert_eq!(
        output(source),
        [
            "1.2346", "Currency", "3.7346", "2.4692", "Double", "Currency", "3.125", "6"
        ]
    );
}

#[test]
fn currency_binary_encoding_preserves_the_scaled_integer() {
    use crate::interpreter::ffi::{binary_width, decode_scalar, encode_scalar};
    use crate::interpreter::value::{Value, VbType};

    let value = Value::Currency(-12_345);
    let bytes = encode_scalar(VbType::Currency, &value).unwrap();
    assert_eq!(binary_width(VbType::Currency).unwrap(), 8);
    assert_eq!(bytes, (-12_345_i64).to_le_bytes());
    assert_eq!(decode_scalar(VbType::Currency, &bytes).unwrap(), value);
}

#[test]
fn date_is_a_distinct_type_with_serial_arithmetic_and_literals() {
    let source = r#"
Dim dateValue As Date
dateValue = #2024-02-29 12:34:56#
Debug.Print TypeName(dateValue)
Debug.Print dateValue
Debug.Print TypeName(DateSerial(2024, 2, 29))
Debug.Print TypeName(TimeValue("12:34:56"))
Debug.Print TypeName(dateValue + 1)
Debug.Print dateValue + 1
Debug.Print TypeName(dateValue - DateSerial(2024, 2, 29))
Debug.Print Format(dateValue, "yyyy-mm-dd hh:nn:ss")
Debug.Print TypeName(CDate("2024-02-29"))
Debug.Print IsDate(dateValue)
Debug.Print IsNumeric(dateValue)
Dim serial As Double
serial = dateValue
Debug.Print TypeName(serial)
"#;
    assert_eq!(
        output(source),
        [
            "Date",
            "2024-02-29 12:34:56",
            "Date",
            "Date",
            "Date",
            "2024-03-01 12:34:56",
            "Double",
            "2024-02-29 12:34:56",
            "Date",
            "True",
            "False",
            "Double"
        ]
    );
}

#[test]
fn date_binary_encoding_uses_ole_automation_double() {
    use crate::interpreter::ffi::{binary_width, decode_scalar, encode_scalar};
    use crate::interpreter::value::{Value, VbType};

    let value = Value::Date(45_000.5);
    let bytes = encode_scalar(VbType::Date, &value).unwrap();
    assert_eq!(binary_width(VbType::Date).unwrap(), 8);
    assert_eq!(bytes, 45_000.5_f64.to_le_bytes());
    assert_eq!(decode_scalar(VbType::Date, &bytes).unwrap(), value);
}

#[test]
fn vb6_like_eqv_imp_and_gosub_are_supported() {
    let source = r#"
Debug.Print "QIDE" Like "Q*"
Debug.Print "Abc123" Like "[A-Z]bc###"
Debug.Print "Abc" Like "[!A-Z]*"
Debug.Print (True Eqv True)
Debug.Print (True Eqv False)
Debug.Print (True Imp False)
Debug.Print (False Imp True)
Debug.Print (1 Eqv 1)
Debug.Print (1 Imp 2)
Debug.Print (True Or False And False)
Debug.Print (Not True Or False)
GoSub AddValue
Debug.Print value
GoTo Done
AddValue:
value = 42
Return
Done:
SubFlow:
GoSub NestedFlow
Debug.Print "after nested"
GoTo EndFlow
NestedFlow:
Debug.Print "nested"
Return
EndFlow:
"#;
    assert_eq!(
        output(source),
        [
            "True",
            "True",
            "False",
            "True",
            "False",
            "False",
            "True",
            "-1",
            "-2",
            "True",
            "False",
            "42",
            "nested",
            "after nested"
        ]
    );
}

#[test]
fn with_supports_object_members_and_method_calls() {
    let source = r#"
Class Counter
    Public Value As Long
    Public Sub Add(ByVal amount As Long)
        Me.Value = Me.Value + amount
    End Sub
End Class
Dim counter As New Counter
With counter
    .Value = 10
    .Add 5
    .Value = .Value + 2
End With
Debug.Print counter.Value
"#;
    assert_eq!(output(source), ["17"]);
}

#[test]
fn enum_members_accept_constant_expressions() {
    let source = r#"
Enum Access
    Read = 1
    Write = Read * 2
    Execute = Write Or 4
    All = Execute + 1
End Enum
Debug.Print Read
Debug.Print Write
Debug.Print Execute
Debug.Print All
"#;
    assert_eq!(output(source), ["1", "2", "6", "7"]);
}

#[test]
fn classes_implement_vb6_interface_methods_and_properties() {
    let source = r#"
Class ICalculator
    Public Function Add(ByVal left As Long, ByVal right As Long) As Long
    End Function
    Public Property Get Total() As Long
    End Property
    Public Property Let Total(ByVal value As Long)
    End Property
End Class
Class Calculator
    Implements ICalculator
    Private mTotal As Long
    Private Function ICalculator_Add(ByVal left As Long, ByVal right As Long) As Long
        ICalculator_Add = left + right
    End Function
    Private Property Get ICalculator_Total() As Long
        ICalculator_Total = mTotal
    End Property
    Private Property Let ICalculator_Total(ByVal value As Long)
        mTotal = value
    End Property
End Class
Dim calculator As New Calculator
Debug.Print calculator.Add(20, 22)
calculator.Total = 7
Debug.Print calculator.Total
"#;
    assert_eq!(output(source), ["42", "7"]);
}

#[test]
fn write_and_input_preserve_date_variant_type() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let path = std::env::temp_dir().join(format!(
        "qide-date-{}-{}.txt",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let source = format!(
        r#"
Dim original As Date
original = #2024-02-29 12:34:56#
Open "{path}" For Output As #1
Write #1, original
Close #1
Dim loaded As Variant
Open "{path}" For Input As #2
Input #2, loaded
Close #2
Debug.Print TypeName(loaded)
Debug.Print loaded
"#,
        path = path.display()
    );
    let mut lines = Vec::new();
    let flag = AtomicBool::new(false);
    let result = run(&source, |line| lines.push(line.to_string()), &flag);
    let _ = std::fs::remove_file(path);
    result.expect("Date 应能通过 Write # / Input # 往返");
    assert_eq!(lines, ["Date", "2024-02-29 12:34:56"]);
}

#[test]
fn win32_struct_layout_round_trips_udt_arrays() {
    use std::collections::HashMap;

    use crate::interpreter::ast::{TypeDef, TypeField};
    use crate::interpreter::ffi::{decode_record, encode_record};
    use crate::interpreter::value::{RecordField, Value, VbType};

    let entry = TypeDef {
        name: "entry".into(),
        display: "Entry".into(),
        fields: vec![TypeField {
            name: "value".into(),
            ty: VbType::Long,
            record: None,
            fixed_len: None,
            array_bounds: None,
        }],
        line: 1,
    };
    let parent = TypeDef {
        name: "parent".into(),
        display: "Parent".into(),
        fields: vec![
            TypeField {
                name: "items".into(),
                ty: VbType::Variant,
                record: Some("entry".into()),
                fixed_len: None,
                array_bounds: Some((1, 2)),
            },
            TypeField {
                name: "tail".into(),
                ty: VbType::Long,
                record: None,
                fixed_len: None,
                array_bounds: None,
            },
        ],
        line: 1,
    };
    let mut definitions = HashMap::new();
    definitions.insert("entry".into(), &entry);
    definitions.insert("parent".into(), &parent);

    let entry_value = |value| {
        Value::record(
            "Entry",
            vec![RecordField::new("value", VbType::Long, Value::Int(value))],
        )
    };
    let item_array = Value::array_records(vec![(1, 2)], false, "Entry", entry_value(0));
    let items = item_array.as_array().unwrap();
    items.borrow_mut().set(&[1], entry_value(11)).unwrap();
    items.borrow_mut().set(&[2], entry_value(22)).unwrap();
    let source = Value::record(
        "Parent",
        vec![
            RecordField::new("items", VbType::Variant, item_array),
            RecordField::new("tail", VbType::Long, Value::Int(33)),
        ],
    );
    let bytes = encode_record(
        "parent",
        &source.as_record().unwrap().borrow(),
        &definitions,
    )
    .unwrap();

    let target_array = Value::array_records(vec![(1, 2)], false, "Entry", entry_value(0));
    let target = Value::record(
        "Parent",
        vec![
            RecordField::new("items", VbType::Variant, target_array),
            RecordField::new("tail", VbType::Long, Value::Int(0)),
        ],
    );
    decode_record("parent", &bytes, &target.as_record().unwrap(), &definitions).unwrap();
    let target_record = target.as_record().unwrap();
    let items = target_record
        .borrow()
        .get("items")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        items
            .borrow()
            .get(&[1])
            .unwrap()
            .as_record()
            .unwrap()
            .borrow()
            .get("value"),
        Some(Value::Int(11))
    );
    assert_eq!(
        items
            .borrow()
            .get(&[2])
            .unwrap()
            .as_record()
            .unwrap()
            .borrow()
            .get("value"),
        Some(Value::Int(22))
    );
    assert_eq!(target_record.borrow().get("tail"), Some(Value::Int(33)));
}

#[test]
fn class_private_members_are_only_accessible_from_their_class() {
    let source = r#"
Class Box
    Dim hidden As Long
    Private values(1) As Long
    Private Property Get Secret() As Long
        Secret = hidden
    End Property
    Private Sub SetSecret(ByVal value As Long)
        hidden = value
    End Sub
    Function Reveal() As Long
        SetSecret 42
        values(0) = hidden
        Reveal = Me.Secret + values(0)
    End Function
End Class
Sub Mutate(ByRef target As Long)
    target = target + 1
End Sub
Dim box As New Box
Debug.Print box.Reveal()
On Error Resume Next
Debug.Print box.hidden
Debug.Print Err.Number
box.hidden = 1
Debug.Print Err.Number
box.SetSecret 1
Debug.Print Err.Number
Debug.Print box.Secret
Debug.Print Err.Number
Debug.Print box.values(0)
Debug.Print Err.Number
Mutate box.hidden
Debug.Print Err.Number
Mutate box.values(0)
Debug.Print Err.Number
"#;
    assert_eq!(
        output(source),
        ["84", "438", "438", "438", "438", "438", "438", "438"]
    );
}

#[test]
fn createobject_reports_activation_errors_as_vb_error_429() {
    let source = r#"
On Error Resume Next
Set unknown = CreateObject("QIDE.Missing.Component")
Debug.Print Err.Number
On Error GoTo 0
On Error Resume Next
Set unknown = GetObject(, "QIDE.Missing.Component")
Debug.Print Err.Number
On Error GoTo 0
On Error Resume Next
Set unknown = GetObject("", "QIDE.Missing.Component")
Debug.Print Err.Number
On Error GoTo 0
On Error Resume Next
Set unknown = GetObject("")
Debug.Print Err.Number
"#;
    assert_eq!(output(source), ["429", "429", "429", "450"]);
}

#[test]
fn getobject_routes_paths_to_moniker_activation() {
    let source = r#"
On Error Resume Next
Set unknown = GetObject("QIDE.Missing.Document", "QIDE.Missing.Component")
Debug.Print Err.Number
On Error GoTo 0
On Error Resume Next
Set unknown = GetObject("C:\QIDE\missing-document.unknown")
Debug.Print Err.Number
"#;
    assert_eq!(output(source), ["429", "429"]);
}

#[test]
fn parser_preserves_com_named_arguments() {
    use crate::interpreter::ast::{Expr, Stmt};

    let program = crate::interpreter::parse("Dim result = obj.Method(2, first:=1, second:=\"x\")")
        .expect("命名参数语法应可解析");
    let Stmt::Dim { items, .. } = &program.main[0] else {
        panic!("应解析为变量声明");
    };
    let Some(Expr::MemberIndex { indices, .. }) = &items[0].initializer else {
        panic!("应解析为成员调用");
    };
    assert!(matches!(
        &indices[1],
        Expr::NamedArg { name, value }
            if name == "first" && matches!(value.as_ref(), Expr::Literal(_))
    ));
    assert!(matches!(&indices[0], Expr::Literal(_)));
    assert!(matches!(
        &indices[2],
        Expr::NamedArg { name, value }
            if name == "second" && matches!(value.as_ref(), Expr::Literal(_))
    ));
    assert!(
        crate::interpreter::parse("Dim result = obj.Method(first:=1, 2)").is_err(),
        "位置参数不能跟在命名参数之后"
    );
}

#[test]
#[ignore = "requires the Windows Script Host COM server"]
fn createobject_invokes_a_registered_dispatch_object() {
    let source = r#"
Dim shell As Object
Set shell = CreateObject("WScript.Shell")
Debug.Print Len(shell.ExpandEnvironmentStrings("%COMSPEC%")) > 0
Dim environmentPattern As String
environmentPattern = "%COMSPEC%"
Debug.Print Len(shell.ExpandEnvironmentStrings(environmentPattern)) > 0
Debug.Print environmentPattern = "%COMSPEC%"
Set shell = GetObject("", "WScript.Shell")
Debug.Print Len(shell.ExpandEnvironmentStrings("%COMSPEC%")) > 0
Set shell = GetObject("new:WScript.Shell")
Debug.Print Len(shell.CurrentDirectory) > 0
Dim currentDirectory As String
currentDirectory = shell.CurrentDirectory
shell.CurrentDirectory = currentDirectory
Debug.Print Len(shell.CurrentDirectory) > 0
Debug.Print shell Is shell
Dim fso As Object
Set fso = CreateObject("Scripting.FileSystemObject")
Dim path As String
path = "."
Debug.Print Len(fso.GetAbsolutePathName(Path:=path)) > 0
Dim dictionary As Object
Set dictionary = CreateObject("Scripting.Dictionary")
Dim sourceValues As Variant
sourceValues = Array("alpha", 42, True)
dictionary.Add "payload", sourceValues
Debug.Print UBound(sourceValues)
Debug.Print sourceValues(1)
Dim returned As Variant
returned = dictionary.Items()
Debug.Print UBound(returned)
Dim payload As Variant
payload = returned(0)
Debug.Print UBound(payload)
Debug.Print payload(0)
Debug.Print payload(1)
Debug.Print payload(2)
"#;
    assert_eq!(
        output(source),
        [
            "True", "True", "True", "True", "True", "True", "True", "True", "2", "42", "0", "2",
            "alpha", "42", "True"
        ]
    );
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
fn persistent_callback_from_previous_runtime_is_never_reactivated() {
    let runtime = std::ptr::without_provenance_mut::<()>(0x1000);
    assert!(crate::interpreter::runtime::runtime_identity_matches(
        runtime, 7, runtime, 7
    ));
    assert!(!crate::interpreter::runtime::runtime_identity_matches(
        runtime, 7, runtime, 8
    ));
}

#[test]
fn address_of_callback_supports_four_parameters_and_string_return() {
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
    let source =
        "Sub Main()\n    Dim arr As Integer() = {3, 1, 2}\n    Debug.Print arr(0)\nEnd Sub\nMain";
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
    let error =
        run(source, |line| lines.push(line.to_string()), &flag).expect_err("重复声明应当报错");
    assert!(
        error.to_string().contains("重复声明"),
        "错误信息应当说明重复声明：{error}"
    );
}

#[test]
fn call_supports_explicit_byval_for_a_byref_parameter() {
    let source = r#"
Sub ReplaceText(ByRef text As String)
    text = "callee"
End Sub

Dim original As String = "caller"
Call ReplaceText(ByVal original)
Debug.Print original

Call ReplaceText(ByVal "literal")
Debug.Print original
"#;
    assert_eq!(output(source), ["caller", "caller"]);
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

/// 运行一段应当失败的脚本，取出运行时错误号。
fn runtime_error_number(source: &str) -> i64 {
    let mut lines = Vec::new();
    let flag = AtomicBool::new(false);
    let error = run(source, |line| lines.push(line.to_string()), &flag).expect_err("应当报错");
    match error {
        crate::interpreter::Error::Runtime(error) => error.number,
        other => panic!("应当是运行时错误：{other}"),
    }
}

#[test]
fn longlong_division_overflow_reports_vb_error_6() {
    // `LongLong` 最小值 `\ -1` 与 `Mod -1` 在 Rust 里会 panic，解释器应当报错误号 6。
    for operator in ["\\", "Mod"] {
        let source = format!(
            "Dim x As LongLong\nx = -9223372036854775807 - 1\nDebug.Print x {operator} -1"
        );
        assert_eq!(runtime_error_number(&source), 6, "整数溢出应当报错误号 6");
    }
}

#[test]
fn huge_array_bounds_report_an_error_instead_of_panicking() {
    let source = "Dim a() As Long\nReDim a(-2147483648 To 9223372036854775807)";
    assert_eq!(runtime_error_number(source), 5, "越界尺寸应当报参数错误");
}

#[test]
fn redim_preserve_keeps_rows_when_growing_the_last_dimension() {
    let source = r#"
Dim a() As Long
ReDim a(0 To 1, 0 To 2)
a(0, 0) = 1
a(0, 1) = 2
a(0, 2) = 3
a(1, 0) = 4
a(1, 1) = 5
a(1, 2) = 6
ReDim Preserve a(0 To 1, 0 To 4)
Debug.Print a(0, 0)
Debug.Print a(0, 2)
Debug.Print a(1, 0)
Debug.Print a(1, 2)
Debug.Print a(1, 4)
"#;
    assert_eq!(output(source), ["1", "3", "4", "6", "0"]);
}

#[test]
fn redim_preserve_rejects_changing_a_leading_dimension() {
    let source = "Dim a() As Long\nReDim a(0 To 1, 0 To 2)\nReDim Preserve a(0 To 2, 0 To 2)";
    assert_eq!(runtime_error_number(source), 9, "只能调整最后一维");
}

#[test]
fn redim_preserve_on_a_fresh_dynamic_array_is_allowed() {
    let source = r#"
Dim a() As Long
ReDim Preserve a(1 To 3)
a(2) = 7
Debug.Print a(2)
Debug.Print UBound(a)
"#;
    assert_eq!(output(source), ["7", "3"]);
}

#[test]
fn instr_finds_an_empty_needle_at_the_start_position() {
    assert_eq!(output(r#"Debug.Print InStr("abc", "")"#), ["1"]);
    assert_eq!(output(r#"Debug.Print InStr(2, "abc", "")"#), ["2"]);
    assert_eq!(output(r#"Debug.Print InStr(5, "abc", "")"#), ["0"]);
    assert_eq!(output(r#"Debug.Print InStr("abcabc", "bc")"#), ["2"]);
    assert_eq!(output(r#"Debug.Print InStr(3, "abcabc", "bc")"#), ["5"]);
}

#[test]
fn boolean_converts_to_integer_like_vb() {
    // VB 的 `True` 转成整数是 -1（与 `CDbl(True)` 一致）。
    assert_eq!(output("Debug.Print True + 1"), ["0"]);
    assert_eq!(output("Debug.Print CInt(True)"), ["-1"]);
    assert_eq!(output("Debug.Print CLng(True)"), ["-1"]);
    assert_eq!(output("Debug.Print CDbl(True)"), ["-1"]);
    assert_eq!(output("Debug.Print True And 2"), ["2"]);
}

#[test]
fn deeply_nested_expressions_report_a_parse_error() {
    let source = format!("Debug.Print {}1{}", "(".repeat(4000), ")".repeat(4000));
    let error = crate::interpreter::parse(&source).expect_err("应当报嵌套过深");
    assert!(error.message.contains("嵌套"), "错误信息：{error}");
}

#[test]
fn deeply_nested_blocks_report_a_parse_error() {
    let mut source = String::new();
    for _ in 0..4000 {
        source.push_str("If True Then\n");
    }
    source.push_str("Debug.Print 1\n");
    for _ in 0..4000 {
        source.push_str("End If\n");
    }
    let error = crate::interpreter::parse(&source).expect_err("应当报嵌套过深");
    assert!(error.message.contains("嵌套"), "错误信息：{error}");
}

#[test]
fn str_prefixes_positive_numbers_like_vb() {
    assert_eq!(output("Debug.Print \"[\" & Str(5) & \"]\""), ["[ 5]"]);
    assert_eq!(output("Debug.Print \"[\" & Str(-5) & \"]\""), ["[-5]"]);
    assert_eq!(output("Debug.Print \"[\" & Str(1.5) & \"]\""), ["[ 1.5]"]);
    // 布尔值不带前导空格。
    assert_eq!(output("Debug.Print \"[\" & Str(True) & \"]\""), ["[True]"]);
}

#[test]
fn val_parses_radix_prefixes_and_stops_at_unparsable_text() {
    assert_eq!(output(r#"Debug.Print Val("&HFF")"#), ["255"]);
    assert_eq!(output(r#"Debug.Print Val("&O17")"#), ["15"]);
    assert_eq!(output(r#"Debug.Print Val("-12.5abc")"#), ["-12.5"]);
    assert_eq!(output(r#"Debug.Print Val(" 42")"#), ["42"]);
    assert_eq!(output(r#"Debug.Print Val("abc")"#), ["0"]);
}

#[test]
fn cint_and_clng_round_bankers_and_check_the_target_range() {
    assert_eq!(output("Debug.Print CInt(2.5)"), ["2"]);
    assert_eq!(output("Debug.Print CInt(3.5)"), ["4"]);
    assert_eq!(output("Debug.Print CLng(-2.5)"), ["-2"]);
    assert_eq!(
        runtime_error_number("Debug.Print CInt(40000)"),
        6,
        "超出 Integer 范围应当报溢出"
    );
    assert_eq!(
        runtime_error_number("Debug.Print CLng(3000000000)"),
        6,
        "超出 Long 范围应当报溢出"
    );
    assert_eq!(
        runtime_error_number(r#"Debug.Print CInt("abc")"#),
        13,
        "非数字文本应当报类型不匹配"
    );
}

#[test]
fn boolean_context_rejects_non_numeric_strings() {
    assert_eq!(output(r#"Debug.Print CBool("True")"#), ["True"]);
    assert_eq!(output(r#"Debug.Print CBool("0")"#), ["False"]);
    assert_eq!(
        runtime_error_number(r#"If "abc" Then Debug.Print 1"#),
        13,
        "非数字文本不能再当作真"
    );
    assert_eq!(
        runtime_error_number(r#"If "" Then Debug.Print 1"#),
        13,
        "空字符串在 VB 里也是类型不匹配"
    );
}

#[test]
fn float_display_uses_fifteen_significant_digits() {
    assert_eq!(output("Debug.Print 0.1 + 0.2"), ["0.3"]);
    assert_eq!(output("Debug.Print 1 / 3"), ["0.333333333333333"]);
    assert_eq!(output("Debug.Print 1E15"), ["1E+15"]);
    assert_eq!(output("Debug.Print 0.0001"), ["1E-04"]);
    assert_eq!(output("Debug.Print 0.001"), ["0.001"]);
}

#[test]
fn space_and_string_reject_huge_lengths() {
    assert_eq!(
        runtime_error_number("Debug.Print Len(Space(20000000))"),
        14,
        "超长字符串应当报字符串空间不足"
    );
    assert_eq!(
        runtime_error_number("Debug.Print Len(String(20000000, \"x\"))"),
        14
    );
}

#[test]
fn for_loop_uses_exact_integer_steps() {
    assert_eq!(
        output("Dim i\nFor i = 0 To 10 Step 3\nDebug.Print i\nNext i"),
        ["0", "3", "6", "9"]
    );
    // 超过 2^53 的整数步进：浮点累加会原地打转，整数路径必须精确。
    let source = "Dim i As LongLong\nFor i = 9007199254740990 To 9007199254740992\nDebug.Print i\nNext i";
    assert_eq!(
        output(source),
        ["9007199254740990", "9007199254740991", "9007199254740992"]
    );
}

#[test]
fn dictionary_keeps_its_index_consistent_after_removals() {
    let source = r#"
Dim d
Set d = New Dictionary
Dim i As Long
For i = 1 To 5
    d.Add "k" & i, i
Next i
d.Remove "k3"
Debug.Print d.Count
Debug.Print d("k4")
Debug.Print d.Exists("K3")
d("k4") = 40
Debug.Print d("K4")
Debug.Print Join(d.Keys, ",")
"#;
    assert_eq!(output(source), ["4", "4", "False", "40", "k1,k2,k4,k5"]);
}

#[test]
fn record_field_limits_are_reported_as_parse_errors() {
    let long_string = format!("Type T\n    S As String * {}\nEnd Type", 70_000);
    assert!(
        crate::interpreter::parse(&long_string).is_err(),
        "定长字符串超过 65535 应当报错"
    );
    let huge_array = "Type T\n    Items(1 To 5000000) As Long\nEnd Type";
    assert!(
        crate::interpreter::parse(huge_array).is_err(),
        "定长数组字段过大应当报错"
    );
}

#[test]
fn headless_options_skip_dialogs_instead_of_blocking() {
    // headless 宿主（命令行 / 外部 agent）不能卡在对话框上：直接给默认返回值。
    let source = "Debug.Print MsgBox(\"你好\")\nDebug.Print \"[\" & InputBox(\"名字\") & \"]\"";
    let mut lines = Vec::new();
    let flag = AtomicBool::new(false);
    crate::interpreter::run_with(
        source,
        |line| lines.push(line.to_string()),
        &flag,
        crate::interpreter::RunOptions {
            allow_ui: false,
            ..crate::interpreter::RunOptions::default()
        },
    )
    .expect("脚本应当执行成功");
    assert_eq!(lines, ["0", "[]"]);
}

#[test]
fn include_dir_can_replace_the_bundled_assets_folder() {
    use std::path::PathBuf;

    let dir = std::env::temp_dir().join(format!("qide_include_dir_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    std::fs::write(dir.join("shared.vb"), "Private Const FROM_DIR As Long = 7\n")
        .expect("写 include 文件");
    let script_dir = dir.join("scripts");
    std::fs::create_dir_all(&script_dir).expect("建脚本目录");
    let script = script_dir.join("main.vb");
    let source = "#Include \"shared.vb\"\nDebug.Print FROM_DIR\n";
    std::fs::write(&script, source).expect("写脚本");

    let mut lines = Vec::new();
    let flag = AtomicBool::new(false);
    crate::interpreter::run_with(
        source,
        |line| lines.push(line.to_string()),
        &flag,
        crate::interpreter::RunOptions {
            script_path: Some(script.clone()),
            include_dir: Some(PathBuf::from(&dir)),
            ..crate::interpreter::RunOptions::default()
        },
    )
    .expect("脚本应当执行成功");
    assert_eq!(lines, ["7"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn implicit_conversions_use_vb_error_numbers() {
    assert_eq!(
        runtime_error_number("Dim x As Long\nx = \"abc\""),
        13,
        "字符串不是数字时报类型不匹配"
    );
    assert_eq!(
        runtime_error_number("Dim b As Boolean\nb = \"abc\""),
        13,
        "字符串不是布尔文本时报类型不匹配"
    );
    assert_eq!(
        runtime_error_number("Dim x As Long\nx = Null"),
        94,
        "Null 不能转换为数值"
    );
    assert_eq!(
        runtime_error_number("Dim x As Long\nx = 3000000000"),
        6,
        "超出 Long 范围报溢出"
    );
}

/// 用界面里真正的脚本线程入口跑脚本（`spawn_script_thread` 的栈与命名都走同一条路）。
fn run_on_script_stack(source: &str) -> Vec<String> {
    let source = source.to_string();
    let (sender, receiver) = std::sync::mpsc::channel();
    crate::interpreter::spawn_script_thread(move || {
        let flag = AtomicBool::new(false);
        let mut lines = Vec::new();
        let result = run(&source, |line| lines.push(line.to_string()), &flag);
        let _ = sender.send((result.is_ok(), lines));
    })
    .expect("线程应当能启动");
    let (ok, lines) = receiver.recv().expect("脚本线程应当回传结果");
    assert!(ok, "脚本应当执行成功");
    lines
}

#[test]
fn recursion_fits_in_the_script_thread_stack() {
    // 递归到调用层次上限时，不能把 2 MiB 的脚本线程栈撑爆（进程会直接退出）。
    let source = "\
Function Down(n As Long) As Long
    If n <= 0 Then
        Down = 0
    Else
        Down = Down(n - 1)
    End If
End Function
Debug.Print Down(190)
";
    assert_eq!(run_on_script_stack(source), ["0"]);
}

#[test]
fn nested_blocks_fit_in_the_script_thread_stack() {
    let mut source = String::from("Dim n As Long\n");
    for _ in 0..40 {
        source.push_str("If n = 0 Then\n");
    }
    source.push_str("Debug.Print 1\n");
    for _ in 0..40 {
        source.push_str("End If\n");
    }
    assert_eq!(run_on_script_stack(&source), ["1"]);
}

#[test]
fn deep_recursion_on_a_small_stack_reports_out_of_stack_space() {
    // 宿主如果没按 `SCRIPT_STACK_SIZE` 开线程（比如直接 `std::thread::spawn`），
    // 解释器也要报错而不是让进程被栈溢出终止。
    let source = "\
Function Down(n As Long) As Long
    If n <= 0 Then
        Down = 0
    Else
        Down = Down(n - 1)
    End If
End Function
Debug.Print Down(190)
";
    let message = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let flag = AtomicBool::new(false);
            match run(source, |_| {}, &flag) {
                Ok(()) => panic!("栈不够时应当报错"),
                Err(error) => error.to_string(),
            }
        })
        .expect("线程应当能启动")
        .join()
        .expect("脚本线程不该崩溃");
    assert!(
        message.contains("栈空间不足") || message.contains("调用层次过深"),
        "错误信息：{message}"
    );
}





