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
fn getobject_single_argument_is_always_a_pathname() {
    // 本机 VB6 实测：单参数 `GetObject` 一律按路径/moniker 处理——
    // 路径缺失（或不是 OLE 文档）报 432、空路径报 5、类名缺失报 429、
    // 不像路径的裸名字交给 moniker 解析（语法错误按原 HRESULT 报 -2147221020）。
    let source = r#"
On Error Resume Next
Set unknown = GetObject("C:\QIDE\missing-document.unknown")
Debug.Print Err.Number
Err.Clear
Set unknown = GetObject("")
Debug.Print Err.Number
Err.Clear
Set unknown = GetObject("QIDE.Missing.Document", "QIDE.Missing.Component")
Debug.Print Err.Number
Err.Clear
Set unknown = GetObject("qide-not-a-moniker")
Debug.Print Err.Number
"#;
    assert_eq!(output(source), ["432", "5", "429", "-2147221020"]);
}

#[test]
fn omitted_arguments_bind_as_missing_optional_parameters() {
    // VB 允许 `F a, , c` 这种「跳过中间可选参数」的写法（本机 VB6 实测）：
    // 省略的 `Variant` 可选形参（没有默认值）`IsMissing` 为真，带默认值的按默认值绑定。
    let source = r#"
Function F(Optional ByVal a As Variant, Optional ByVal b As Variant) As String
    F = CStr(IsMissing(a)) & "/" & CStr(IsMissing(b))
End Function

Function H(Optional ByVal a As Variant = 5, Optional ByVal b As Variant) As String
    H = CStr(IsMissing(a)) & "/" & CStr(IsMissing(b))
End Function

Function G(Optional ByVal a As Long = 7, Optional ByVal b As Long = 9) As String
    G = CStr(a) & "/" & CStr(b) & "/" & CStr(IsMissing(a))
End Function

Sub S(ByVal a As Variant, Optional ByVal b As Variant)
    Debug.Print "S " & a & " " & CStr(IsMissing(b))
End Sub

Debug.Print F(1)
Debug.Print F(, "x")
Debug.Print F(, )
Debug.Print H(, "y")
Debug.Print H(, )
Debug.Print G(, )
S 1,
S 1, 2
"#;
    assert_eq!(
        output(source),
        [
            "False/True",
            "True/False",
            "True/True",
            "False/False",
            "False/True",
            "7/9/False",
            "S 1 True",
            "S 1 False",
        ]
    );
}

#[test]
fn omitting_a_required_argument_reports_449() {
    let source = r#"
Function Two(ByVal a As Long, ByVal b As Long) As Long
    Two = a + b
End Function

On Error Resume Next
Debug.Print Two(1, )
Debug.Print Err.Number
Err.Clear
Debug.Print Two(, 2)
Debug.Print Err.Number
"#;
    assert_eq!(output(source), ["449", "449"]);
}

#[test]
fn a_member_read_is_allowed_as_a_statement() {
    // QIDE 比 VB6 宽：`h.Self2().Value` 单独成句会求值后丢掉结果。
    // （本机 VB6 `/make` 实测这种写法编译不过，见 ROADMAP「已知遗留」。）
    let source = r#"
Class Holder
    Public Value As Long
    Public Calls As Long
    Public Function Self2() As Holder
        Calls = Calls + 1
        Set Self2 = Me
    End Function
End Class

Dim h As New Holder
h.Value = 5
h.Self2().Value
Debug.Print h.Calls
"#;
    // 打印 1 说明整条表达式确实求值了（不是被当成无操作跳过）。
    assert_eq!(output(source), ["1"]);
}

#[test]
fn chained_assignment_writes_through_methods_and_property_gets() {
    // 本机 VB6 实测（类模块 + Form_Load）：方法调用返回对象后，`= 值`/`Set = 对象`
    // 落在**返回的对象**上；`obj.Self3.Value` 不带括号也能穿过多级属性读取。
    let source = r#"
Class Holder
    Public Value As Long
    Public Other As Holder
    Public Inner As Holder

    Public Function Self2() As Holder
        Set Self2 = Me
    End Function

    Public Function Method1(ByVal n As Long) As Holder
        Method1 = Me
    End Function

    Public Property Get Self3() As Holder
        Set Self3 = Me
    End Property

    Private Sub Class_Initialize()
        Set Inner = Me
    End Sub
End Class

Dim h As New Holder
Dim e As New Holder
h.Self2().Value = 7
Debug.Print h.Value
h.Self3.Value = 9
Debug.Print h.Value
h.Self2().Inner.Value = 11
Debug.Print h.Value
Set h.Self2().Other = e
Debug.Print h.Other Is e
h.Method1(3).Value = 13
Debug.Print h.Value
"#;
    assert_eq!(output(source), ["7", "9", "11", "True", "13"]);
}

#[test]
fn chained_assignment_covers_object_fields_and_parameterized_properties() {
    // 本机 VB6 实测：对象字段链（`h.Inner.Value`）、带参数的 `Property Get` 链
    // （`h.PropChain(2).Value`）、无参属性带空括号（`h.Self3().Value`）都按同一规则写入。
    let source = r#"
Class Holder
    Public Value As Long
    Public Other As Holder
    Public Inner As Holder

    Public Property Get Self3() As Holder
        Set Self3 = Me
    End Property

    Public Property Get PropChain(ByVal n As Long) As Holder
        Set PropChain = Me
    End Property

    Private Sub Class_Initialize()
        Set Inner = Me
        Set Other = Me
    End Sub
End Class

Dim h As New Holder
Dim e As New Holder
h.Inner.Value = 21
Debug.Print h.Value
h.Self3().Value = 27
Debug.Print h.Value
h.PropChain(2).Value = 25
Debug.Print h.Value
Set h.Inner.Other = e
Debug.Print h.Other Is e
"#;
    assert_eq!(output(source), ["21", "27", "25", "True"]);
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
            "2024/2/29 12:34:56",
            "Date",
            "Date",
            "Date",
            "2024/3/1 12:34:56",
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
fn with_assigns_through_property_let_and_chained_members() {
    // 本机 VB6 实测：`With` 里 `.Value = 10` 走 `Property Let`、`Set .Child = e` 走对象赋值，
    // `.Inner.Value = 31` 与 `.Kiddie(2).Value = 33` 都先读/调用中间层再在返回对象上赋值。
    let source = r#"
Class Holder
    Public Value As Long
    Public Child As Holder
    Public Inner As Holder

    Public Property Get Kiddie(ByVal n As Long) As Holder
        Set Kiddie = Me
    End Property

    Private Sub Class_Initialize()
        Set Inner = Me
    End Sub
End Class

Class Counter
    Private m As Long

    Public Property Get Value() As Long
        Value = m
    End Property

    Public Property Let Value(ByVal v As Long)
        m = v
    End Property
End Class

Dim c As New Counter
Dim h As New Holder
Dim e As New Holder
With c
    .Value = 10
End With
Debug.Print c.Value
With h
    Set .Child = e
End With
Debug.Print h.Child Is e
With h
    .Inner.Value = 31
End With
Debug.Print h.Value
With h
    .Kiddie(2).Value = 33
End With
Debug.Print h.Value
"#;
    assert_eq!(output(source), ["10", "True", "31", "33"]);
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
    assert_eq!(lines, ["Date", "2024/2/29 12:34:56"]);
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
    items.borrow_mut().set(&[1], entry_value(11), None).unwrap();
    items.borrow_mut().set(&[2], entry_value(22), None).unwrap();
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
fn callbyname_dispatches_get_let_set_and_method_like_vb6() {
    // 期望值全部来自本机 VB6 实测（`CallByName` 的错误号逐条比对过）。
    let source = r#"
Class Payload
    Public Tag As String
End Class
Class Widget
    Public Count As Long
    Public Label2 As String
    Public Inner As Payload
    Public Property Get Title() As String
        Title = "T:" & CStr(Count)
    End Property
    Public Property Let Title(ByVal value As String)
        Count = Count + 1
        Label2 = value
    End Property
    Public Function Total(ByVal left As Long, ByVal right As Long) As Long
        Total = left + right
    End Function
    Public Sub Bump()
        Count = Count + 1
    End Sub
    Public Function RefArg(x As Long) As Long
        x = x + 100
        RefArg = x
    End Function
    Private Property Get Secret() As String
        Secret = "s"
    End Property
End Class
Dim widget As New Widget
Dim payload As New Payload
Dim result As Variant
widget.Count = 7
Debug.Print CallByName(widget, "Title", VbGet)
CallByName widget, "Title", VbLet, "hello"
Debug.Print widget.Count
Debug.Print widget.Label2
Debug.Print CallByName(widget, "Total", VbMethod, 3, 4)
CallByName widget, "Bump", VbMethod
Debug.Print widget.Count
Debug.Print CallByName(widget, "Count", VbGet)
CallByName widget, "Count", VbLet, 42
Debug.Print widget.Count
Debug.Print CallByName(widget, "count", VbGet)
payload.Tag = "inner-tag"
CallByName widget, "Inner", VbSet, payload
Debug.Print widget.Inner.Tag
Debug.Print CallByName(widget, "Inner", VbGet) Is payload
Dim counter As Long
counter = 5
Debug.Print CallByName(widget, "RefArg", VbMethod, counter)
Debug.Print counter
result = CallByName(widget, "Bump", VbMethod)
Debug.Print IsEmpty(result)
On Error Resume Next
result = Empty
Err.Clear
result = CallByName(widget, "Nope", VbGet)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Title", 99)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Secret", VbGet)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Bump", VbGet)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Label2", VbGet)
Debug.Print Err.Number
Err.Clear
CallByName widget, "Title", VbSet, "x"
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Count", VbMethod)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Total", VbMethod, 3)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "", VbGet)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Bump", VbMethod, 1, 2)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Title", VbLet)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Total", VbGet, 3, 4)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Count", VbSet, 5)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Count", VbLet, "abc")
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Inner", VbLet, payload)
Debug.Print Err.Number
result = Empty
Err.Clear
result = CallByName(widget, "Count", VbLet, 9, 10)
Debug.Print Err.Number
Dim missingObject As Widget
result = Empty
Err.Clear
result = CallByName(missingObject, "Count", VbGet)
Debug.Print Err.Number
Dim numberVar As Variant
numberVar = 5
result = Empty
Err.Clear
result = CallByName(numberVar, "Count", VbGet)
Debug.Print Err.Number
"#;
    assert_eq!(
        output(source),
        [
            "T:7",
            "8",
            "hello",
            "7",
            "9",
            "9",
            "42",
            "42",
            "inner-tag",
            "True",
            "105",
            "105",
            "True",
            "438",
            "438",
            "438",
            "450",
            "0",
            "450",
            "438",
            "449",
            "438",
            "450",
            "446",
            "450",
            "451",
            "13",
            "91",
            "451",
            "5",
            "424",
        ]
    );
}

#[test]
fn variant_narrow_types_survive_parameters_and_udt_fields_like_vb6() {
    // 期望值全部来自本机 VB6 实测：`ByVal v As Variant` 要保留实参的窄类型，
    // UDT 里 `As Variant` 的字段也要保留（`u.f = i` 之后 `VarType(u.f)` 是 2）。
    let source = r#"
Type NarrowRec
    f As Variant
End Type
Function Narrow(ByVal v As Variant) As Long
    Narrow = VarType(v)
End Function
Function NarrowRef(v As Variant) As Long
    NarrowRef = VarType(v)
End Function
Function SingleBack(ByVal v As Single) As Single
    SingleBack = v
End Function
Dim i As Integer
Dim s As Single
Dim byt As Byte
i = 5
s = CSng(1.5)
byt = 3
Debug.Print Narrow(i)
Debug.Print Narrow(s)
Debug.Print Narrow(byt)
Debug.Print Narrow(40000)
Debug.Print Narrow(CSng(1.5))
Debug.Print Narrow(CByte(1))
Debug.Print NarrowRef(i)
Debug.Print NarrowRef(s)
Debug.Print VarType(Array(i, s, byt))
Dim u As NarrowRec
u.f = i
Debug.Print VarType(u.f)
u.f = s
Debug.Print VarType(u.f)
u.f = byt
Debug.Print VarType(u.f)
u.f = 40000
Debug.Print VarType(u.f)
Dim recs(1) As NarrowRec
recs(0).f = i
Debug.Print VarType(recs(0).f)
Dim mn As Variant
mn = i
Debug.Print VarType(mn)
Dim fn As Variant
fn = SingleBack(1.5)
Debug.Print VarType(fn)
"#;
    assert_eq!(
        output(source),
        [
            "2", "4", "17", "3", "4", "17", "2", "4", "8204", "2", "4", "17", "3", "2", "2", "4",
        ]
    );
}

#[test]
fn put_and_get_write_variants_as_a_tagged_layout_like_vb6() {
    // 本机 VB6 实测：`Put #` 一个 `Variant` 变量写的是「2 字节 VARTYPE + 内容」，
    // `Get #` 读回同样的布局并把变量类型改成标记里的类型（逐字节比对过写入文件）。
    use std::time::{SystemTime, UNIX_EPOCH};

    let path = std::env::temp_dir().join(format!(
        "qide-variant-{}-{}.bin",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let source = format!(
        r#"
Dim n As Integer
Dim v As Variant
n = FreeFile
Open "{path}" For Binary As #n
v = 5
Put #n, , v
Debug.Print Loc(n)
v = "AB"
Put #n, , v
Debug.Print Loc(n)
v = CCur(1.5)
Put #n, , v
Debug.Print Loc(n)
v = True
Put #n, , v
Debug.Print Loc(n)
v = CDbl(2.5)
Put #n, , v
Debug.Print Loc(n)
v = CDate(2)
Put #n, , v
Debug.Print Loc(n)
v = CSng(1.5)
Put #n, , v
Debug.Print Loc(n)
v = Empty
Put #n, , v
Debug.Print Loc(n)
Debug.Print LOF(n)
Dim w As Variant
Get #n, 1, w
Debug.Print w
Debug.Print VarType(w)
Debug.Print Loc(n)
Seek n, 5
Get #n, , w
Debug.Print w
Debug.Print VarType(w)
Debug.Print Loc(n)
Get #n, , w
Debug.Print w
Debug.Print VarType(w)
Debug.Print Loc(n)
Seek n, 21
Get #n, , w
Debug.Print w
Debug.Print VarType(w)
Seek n, 25
Get #n, , w
Debug.Print w
Debug.Print VarType(w)
Close #n
"#,
        path = path.display()
    );
    let mut lines = Vec::new();
    let flag = AtomicBool::new(false);
    let result = run(&source, |line| lines.push(line.to_string()), &flag);
    let bytes = std::fs::read(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    result.expect("Variant 应能通过 Put # / Get # 往返");
    assert_eq!(
        lines,
        [
            "4", "10", "20", "24", "34", "44", "50", "52",
            "52", // 写：每次写到的位置
            "5", "2", "4", // 读回 Integer
            "AB", "8", "10", // 读回 String
            "1.5", "6", "20", // 读回 Currency
            "True", "11", // 读回 Boolean
            "2.5", "5", // 读回 Double
        ]
    );
    // 文件布局：`02 00 05 00 | 08 00 02 00 41 42 | 06 00 …`
    assert_eq!(
        &bytes[..12],
        &[
            0x02, 0x00, 0x05, 0x00, 0x08, 0x00, 0x02, 0x00, 0x41, 0x42, 0x06, 0x00
        ]
    );
    assert_eq!(bytes.len(), 52);
}

#[test]
fn var_type_reports_narrow_types_through_calls_and_members_like_vb6() {
    // 期望值全部来自本机 VB6 实测：函数返回 `Variant`、`IIf`/`Choose`/`Switch` 都按**选中的那个值**报告，
    // 类成员（`Property Get` 的返回类型、`Variant` 字段的内容类型）、`ParamArray` 元素、`Collection.Item` 同理。
    let source = r#"
Class Holder
    Public P As Variant
    Public Property Get PG() As Integer
        PG = 7
    End Property
    Public Property Get PS() As Single
        PS = CSng(1.5)
    End Property
    Public Property Get PV() As Variant
        Dim i As Integer
        i = 9
        PV = i
    End Property
End Class
Type Rec
    f As Variant
End Type
Function FI() As Integer
    FI = 5
End Function
Function FS() As Single
    FS = CSng(1.5)
End Function
Function FB() As Byte
    FB = 3
End Function
Function FD() As Double
    FD = 2.5
End Function
Function FV() As Variant
    Dim i As Integer
    i = 9
    FV = i
End Function
Function ShowParam(ParamArray xs()) As String
    ShowParam = CStr(VarType(xs(0))) & "/" & CStr(VarType(xs(1)))
End Function
Dim i As Integer
Dim s As Single
Dim b As Byte
i = 5
s = CSng(1.5)
b = 3
Debug.Print VarType(FI())
Debug.Print VarType(FS())
Debug.Print VarType(FB())
Debug.Print VarType(FD())
Debug.Print VarType(FV())
Debug.Print VarType(IIf(True, i, s))
Debug.Print VarType(IIf(False, i, s))
Debug.Print VarType(IIf(True, b, i))
Debug.Print VarType(Choose(2, i, s))
Debug.Print VarType(Switch(False, i, True, s))
Dim holder As New Holder
Debug.Print VarType(holder.PG)
Debug.Print VarType(holder.PS)
Debug.Print VarType(holder.PV)
holder.P = i
Debug.Print VarType(holder.P)
Dim record As Rec
record.f = i
With record
    Debug.Print VarType(.f)
End With
Debug.Print ShowParam(i, s)
Debug.Print VarType(Mid("abc", 1))
Debug.Print VarType(RGB(1, 2, 3))
Dim assigned As Variant
assigned = FI()
Debug.Print VarType(assigned)
Dim items As New Collection
items.Add i
Debug.Print VarType(items.Item(1))
Debug.Print VarType(items(1))
"#;
    assert_eq!(
        output(source),
        [
            "2", "4", "17", "5", "2", "2", "4", "17", "4", "4", "2", "4", "2", "2", "2", "2/4",
            "8", "3", "2", "2", "2",
        ]
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
    // `GetObject("")` 在 VB6 里是 5（路径为空，本机实测）。
    assert_eq!(output(source), ["429", "429", "429", "5"]);
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
    // 类名缺失 429；路径缺失 432（VB6 实测，本机逐条对比过）。
    assert_eq!(output(source), ["429", "432"]);
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
dictionary.Add "k", "v"
Debug.Print dictionary.Item("k")
Debug.Print dictionary("k")
dictionary("w") = "z"
Debug.Print dictionary("w")
Debug.Print CallByName(dictionary, "Item", VbGet, "k")
Dim absent As Variant
absent = dictionary.Item("nope")
Debug.Print dictionary.Exists("nope")
On Error Resume Next
dictionary.Add "k", "again"
Debug.Print Err.Number
Err.Clear
absent = dictionary.NoSuchMember
Debug.Print Err.Number
Err.Clear
On Error GoTo 0
Dim fresh As Object
Set fresh = CreateObject("Scripting.Dictionary")
fresh.Add "a", 1
fresh.Add "b", 2
Dim key As Variant
For Each key In fresh
    Debug.Print key
Next
Debug.Print TypeName(fresh)
"#;
    let lines = output(source);
    assert_eq!(
        lines[..15],
        [
            "True", "True", "True", "True", "True", "True", "True", "True", "2", "42", "0", "2",
            "alpha", "42", "True"
        ]
    );
    // 参数化属性（`Item("k")`）、默认成员（`d("k")` / `d("w") = "z"`）、错误号、`For Each`、`TypeName`。
    assert_eq!(
        lines[15..],
        [
            "v",
            "v",
            "z",
            "v",
            "True",
            "457",
            "438",
            "a",
            "b",
            "Dictionary"
        ]
    );
}

#[test]
#[ignore = "requires the Windows Script Host COM server"]
fn for_each_over_object_without_new_enum_matches_vb6() {
    // 本机 VB6 实测（`On Error Resume Next` 下）：`WScript.Shell` 没有 `_NewEnum`，
    // 但 `For Each` 的循环体照样跑**一次**——此时循环变量是 `Empty`（`IsObject` 为假、`IsEmpty` 为真）、
    // `Err.Number` 已经是 438，随后 `Next` 报 92（「For 循环未初始化」）；循环体里写了 `Exit For` 也一样报 92。
    let source = r#"
Dim sh As Object, x As Variant, n As Long
Set sh = CreateObject("WScript.Shell")
On Error Resume Next
Err.Clear
For Each x In sh
    n = n + 1
    Debug.Print n
    Debug.Print TypeName(x)
    Debug.Print IsObject(x)
    Debug.Print IsEmpty(x)
    Debug.Print Err.Number
Next x
Debug.Print n
Debug.Print Err.Number
Err.Clear
For Each x In sh
    n = n + 1
    Exit For
Next x
Debug.Print n - 1
Debug.Print Err.Number
"#;
    assert_eq!(
        output(source),
        ["1", "Empty", "False", "True", "438", "1", "92", "1", "92"]
    );
}

#[test]
#[ignore = "requires the Windows Script Host COM server"]
fn com_arrays_keep_their_element_type_and_dimensions() {
    // 本机 VB6 实测：数组经 COM 往返后元素类型与各维边界都不变
    // （`Long()` 的 `VarType` 是 8195、`String()` 8200、`Byte()` 8209、`Boolean()` 8203，`Variant(1 To 2, 0 To 2)` 是 8204）。
    let source = r#"
Dim dictionary As Object
Set dictionary = CreateObject("Scripting.Dictionary")
Dim cache As Variant
Dim longs(2) As Long
longs(0) = 7
dictionary.Add "longs", longs
cache = dictionary("longs")
Debug.Print VarType(cache) & " " & TypeName(cache) & " " & cache(0)
Dim bytes(1) As Byte
bytes(0) = 200
dictionary.Add "bytes", bytes
cache = dictionary("bytes")
Debug.Print VarType(cache) & " " & TypeName(cache) & " " & cache(0)
Dim flags(1) As Boolean
flags(0) = True
dictionary.Add "flags", flags
cache = dictionary("flags")
Debug.Print VarType(cache) & " " & cache(0)
Dim grid(1 To 2, 0 To 2) As Variant
grid(2, 1) = 21
dictionary.Add "grid", grid
cache = dictionary("grid")
Debug.Print VarType(cache) & " " & LBound(cache, 1) & UBound(cache, 1) & LBound(cache, 2) & UBound(cache, 2) & " " & cache(2, 1)
Dim cube(1 To 2, 1 To 2, 1 To 2) As Variant
cube(2, 2, 2) = 5
dictionary.Add "cube", cube
cache = dictionary("cube")
Debug.Print LBound(cache, 3) & UBound(cache, 3) & " " & cache(2, 2, 2)
"#;
    assert_eq!(
        output(source),
        [
            "8195 Long() 7",
            "8209 Byte() 200",
            "8203 True",
            "8204 1202 21",
            "12 5"
        ]
    );
}

#[test]
#[ignore = "requires Microsoft ADODB (msado15.dll)"]
fn com_chained_members_work_with_adodb_recordsets() {
    // 本机 VB6 实测同一段脚本输出 `01 1 alpha` / `Field`：
    // `rs.Fields.Append "id", 3` 是关键字成员 + 两层路径调用，`rs.Fields("id").Value = 1` 是链式成员赋值。
    let source = r#"
Dim recordset As Object
Set recordset = CreateObject("ADODB.Recordset")
recordset.Fields.Append "id", 3
recordset.Fields.Append "name", 200, 50
recordset.Open
recordset.AddNew
recordset.Fields("id").Value = 1
recordset.Fields("name").Value = "alpha"
recordset.Update
recordset.MoveFirst
Dim rows As Variant
rows = recordset.GetRows(1)
Debug.Print LBound(rows, 1) & UBound(rows, 1) & " " & rows(0, 0) & " " & rows(1, 0)
Debug.Print TypeName(recordset.Fields("id"))
recordset.Close
"#;
    assert_eq!(output(source), ["01 1 alpha", "Field"]);
}

#[test]
#[ignore = "requires Microsoft ADODB (msado15.dll)"]
fn with_block_writes_through_com_chained_members() {
    // 本机 VB6 实测（`python tools/vb6_probe.py --script`）同一段脚本输出 `id=5`。
    let source = r#"
Dim recordset As Object
Set recordset = CreateObject("ADODB.Recordset")
recordset.Fields.Append "id", 3
recordset.Open
recordset.AddNew
With recordset
    .Fields("id").Value = 5
    .Update
End With
recordset.MoveFirst
Debug.Print "id=" & recordset.Fields("id").Value
recordset.Close
"#;
    assert_eq!(output(source), ["id=5"]);
}

#[test]
#[ignore = "requires Microsoft ADODB (msado15.dll)"]
fn com_get_rows_returns_a_two_dimensional_array() {
    // `GetRows` 的二维数组：第 1 维是字段、第 2 维是行（与 VB6 一致，游标停在更新后的那一行）。
    let source = r#"
Dim recordset As Object
Set recordset = CreateObject("ADODB.Recordset")
recordset.Fields.Append "id", 3
recordset.Fields.Append "name", 200, 50
recordset.Open
recordset.AddNew
recordset.Fields("id").Value = 1
recordset.Fields("name").Value = "alpha"
recordset.Update
recordset.AddNew
recordset.Fields("id").Value = 2
recordset.Fields("name").Value = "beta"
recordset.Update
Debug.Print recordset.RecordCount
Dim rows As Variant
rows = recordset.GetRows()
Debug.Print LBound(rows, 1) & UBound(rows, 1) & LBound(rows, 2) & UBound(rows, 2)
Debug.Print rows(0, 0) & " " & rows(1, 0)
Debug.Print VarType(rows)
recordset.Close
"#;
    assert_eq!(output(source), ["2", "0100", "2 beta", "8204"]);
}

#[test]
#[ignore = "requires Microsoft ADODB (msado15.dll)"]
fn with_events_receives_com_events() {
    // `WithEvents` 变量 `Set` 到 COM 对象后挂接它的事件，处理过程名是 `变量_事件`。
    // 这里连一个不存在的 Provider：ADO 会先发 `WillConnect`，随后 `Open` 报 3706。
    let source = r#"
Dim WithEvents connection As Object
Dim calls As Long

Sub connection_WillConnect(ByVal connectionString As Variant, ByVal userID As Variant, ByVal password As Variant, ByVal options As Long, ByVal status As Long, ByVal target As Object)
    calls = calls + 1
End Sub

Sub connection_ConnectComplete(ByVal failure As Object, ByVal status As Long, ByVal target As Object)
    calls = calls + 1
End Sub

Set connection = CreateObject("ADODB.Connection")
On Error Resume Next
connection.Open "Provider=NoSuchProvider;"
Debug.Print Err.Number
Debug.Print calls > 0
Debug.Print "done"
"#;
    assert_eq!(output(source), ["3706", "True", "done"]);
}

#[test]
#[ignore = "requires Microsoft ADODB (msado15.dll)"]
fn with_events_works_on_class_fields() {
    // 类字段上的 `WithEvents` 用同类的私有过程 `字段_事件` 接收（VB6 里 `WithEvents` 本来就写在类里）。
    let source = r#"
Class Watcher
    Private WithEvents connection As Object
    Public calls As Long

    Public Sub Attach()
        Set connection = CreateObject("ADODB.Connection")
    End Sub

    Public Sub Connect()
        On Error Resume Next
        connection.Open "Provider=NoSuchProvider;"
    End Sub

    Private Sub connection_WillConnect(ByVal connectionString As Variant, ByVal userID As Variant, ByVal password As Variant, ByVal options As Long, ByVal status As Long, ByVal target As Object)
        calls = calls + 1
    End Sub
End Class

Dim watcher As Watcher
Set watcher = New Watcher
watcher.Attach
watcher.Connect
DoEvents
Debug.Print watcher.calls > 0
Debug.Print "done"
"#;
    assert_eq!(output(source), ["True", "done"]);
}

#[test]
#[ignore = "requires the Windows Script Host COM server"]
fn com_objects_do_not_convert_to_strings() {
    // 本机 VB6 实测：`CStr` / `&` / `Print` 用到一个没有默认属性的对象时都报 450。
    let source = r#"
Dim dictionary As Object
Set dictionary = CreateObject("Scripting.Dictionary")
On Error Resume Next
Dim text As String
text = CStr(dictionary)
Debug.Print Err.Number
Err.Clear
text = "prefix" & dictionary
Debug.Print Err.Number
Err.Clear
text = dictionary
Debug.Print Err.Number
Err.Clear
Debug.Print dictionary
Debug.Print Err.Number
"#;
    assert_eq!(output(source), ["450", "450", "450", "450"]);
}

#[test]
#[ignore = "requires the Windows Script Host COM server"]
fn for_each_over_a_com_collection_uses_a_live_enumerator() {
    // `For Each` 走 `_NewEnum` 的活枚举器（VB6 同样如此），不是进循环前取一份快照：
    // 循环体里往集合里加的元素，服务器自己的枚举器看得见，所以这里会多打印一个 `c`。
    let source = r#"
Dim dictionary As Object
Set dictionary = CreateObject("Scripting.Dictionary")
dictionary.Add "a", 1
dictionary.Add "b", 2
Dim key As Variant
For Each key In dictionary
    Debug.Print "got " & key
    If key = "a" Then dictionary.Add "c", 3
Next
Debug.Print dictionary.Count
"#;
    assert_eq!(output(source), ["got a", "got b", "got c", "3"]);
}

#[test]
#[ignore = "requires the Windows Script Host COM server"]
fn com_error_numbers_match_vb6() {
    // 本机 VB6 实测（`tools/vb6_probe.py --script`）：系统包装码**原样**当 `Long` 报（含负号），
    // `E_INVALIDARG` 报 5，服务器自己的 `0x800Axxxx` 取低 16 位。
    let source = r#"
Dim sh As Object
Dim fso As Object
Set sh = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")
On Error Resume Next
sh.Run "nonexistent-xyz.exe"
Debug.Print Err.Number
Err.Clear
sh.Run ""
Debug.Print Err.Number
Err.Clear
sh.Exec "C:\Windows"
Debug.Print Err.Number
Err.Clear
sh.Run "\\nonexistent-host-qide\share\a.exe"
Debug.Print Err.Number
Err.Clear
fso.GetFile "C:\nonexistent-xyz\a.txt"
Debug.Print Err.Number
Err.Clear
fso.OpenTextFile "C:\nonexistent-xyz\a.txt"
Debug.Print Err.Number
"#;
    assert_eq!(
        output(source),
        ["-2147024894", "5", "-2147024891", "-2147024843", "53", "76"]
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
fn member_names_accept_keywords_and_multi_level_paths() {
    // `obj.` 之后的成员名可以是关键字（VB6 里 `rs.Fields.Append`、`f.Type`、`rs.Open` 都合法），
    // 语句级的成员路径也不限一层。
    for source in [
        "Sub Main()\n    rs.Fields.Append \"id\", 3\nEnd Sub",
        "Sub Main()\n    f.Type = 1\nEnd Sub",
        "Sub Main()\n    rs.Open\nEnd Sub",
        "Sub Main()\n    rs.Fields(\"id\").Value = 1\nEnd Sub",
        "Sub Main()\n    Debug.Print f.Type\nEnd Sub",
        "Sub Main()\n    With f\n        .Type = 1\n    End With\nEnd Sub",
        "Sub Main()\n    With rs\n        .Fields(\"id\").Value = 1\n    End With\nEnd Sub",
        "Sub Main()\n    With f\n        Set .Child = obj\n    End With\nEnd Sub",
    ] {
        assert!(
            crate::interpreter::parse(source).is_ok(),
            "应当能解析：{source}"
        );
    }
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
        let source =
            format!("Dim x As LongLong\nx = -9223372036854775807 - 1\nDebug.Print x {operator} -1");
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

/// 循环结束后计数器停在「第一个不满足条件的值」（本机 VB6 实测）。
#[test]
fn for_leaves_the_counter_at_the_first_failing_value() {
    let cases = [
        ("For i = 1 To 4\nNext i", "5"),
        ("For i = 1 To 10 Step 3\nNext i", "13"),
        ("For i = 5 To 1 Step -2\nNext i", "-1"),
        ("For i = 5 To 1\nNext i", "5"),
        ("For i = 1 To 10\nIf i = 4 Then Exit For\nNext i", "4"),
    ];
    for (loop_source, expected) in cases {
        let source = format!("Dim i As Long\n{loop_source}\nDebug.Print i");
        assert_eq!(output(&source), [expected], "{loop_source}");
    }
}

/// `For` 的计数器溢出按 VB6 报错误号 6（`Integer` 到 32767、`Long` 到 2147483647 之后再加一步）。
#[test]
fn for_counter_overflow_reports_error_six() {
    let source = r#"
Dim i As Integer, n As Long
On Error Resume Next
For i = 32767 To 32767
    n = n + 1
Next i
Debug.Print n
Debug.Print Err.Number
Err.Clear
For i = -32768 To -32768
    n = n + 1
Next i
Debug.Print n
Debug.Print Err.Number
"#;
    assert_eq!(output(source), ["1", "6", "2", "0"]);
}

/// `Loc(n)` 在二进制模式下返回「最后一个读写过的字节」的 1 基位置（VB6 实测：Seek 2 后 Get 一个字节 → 2）。
#[test]
fn loc_reports_the_last_byte_accessed() {
    let dir = std::env::temp_dir().join(format!("qide-loc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("loc.dat"), [1u8, 2, 3]).unwrap();
    let source = r#"
Dim fn As Integer, a As Byte
fn = FreeFile
Open "loc.dat" For Binary As #fn
Debug.Print Loc(fn)
Debug.Print LOF(fn)
Seek #fn, 2
Get #fn, , a
Debug.Print a
Debug.Print Loc(fn)
Close #fn
"#;
    let interrupt = AtomicBool::new(false);
    let mut lines = Vec::new();
    let options = crate::interpreter::RunOptions {
        script_path: Some(dir.join("loc.vb")),
        allow_ui: false,
        ..crate::interpreter::RunOptions::default()
    };
    crate::interpreter::run_with(
        source,
        |line| lines.push(line.to_string()),
        &interrupt,
        options,
    )
    .expect("脚本应当执行成功");
    assert_eq!(lines, ["0", "3", "2", "2"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `Put #` / `Get #` 的变长 `String`：VB6 实测没有长度前缀，`Get` 按变量当前长度（字节）读。
#[test]
fn binary_strings_are_written_without_a_length_prefix() {
    let dir = std::env::temp_dir().join(format!("qide-strbin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = r#"
Dim fn As Integer, s As String, out As String
s = "AB"
fn = FreeFile
Open "str.dat" For Binary As #fn
Put #fn, , s
Put #fn, , "CD"
Put #fn, , ""
Debug.Print LOF(fn)
Close #fn
Open "str.dat" For Binary As #fn
out = "ABCD"
Get #fn, 1, out
Debug.Print out
out = ""
Get #fn, , out
Debug.Print "[" & out & "]"
Debug.Print Loc(fn)
Close #fn
"#;
    let interrupt = AtomicBool::new(false);
    let mut lines = Vec::new();
    let options = crate::interpreter::RunOptions {
        script_path: Some(dir.join("str.vb")),
        allow_ui: false,
        ..crate::interpreter::RunOptions::default()
    };
    crate::interpreter::run_with(
        source,
        |line| lines.push(line.to_string()),
        &interrupt,
        options,
    )
    .expect("脚本应当执行成功");
    assert_eq!(lines, ["4", "ABCD", "[]", "4"]);
    assert_eq!(
        std::fs::read(dir.join("str.dat")).unwrap(),
        b"ABCD",
        "变长字符串没有长度前缀"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `Error n` 语句：写入 `Err.Number` 与 VB 的默认描述；`Error$(n)` / `Error(n)` 取同一套文案。
/// 期望值全部来自本机 VB6 p-code 实测（`Error 40080` 的描述是兜底文案）。
#[test]
fn error_statement_uses_the_default_vb_description() {
    let source = r#"
On Error Resume Next
Error 5
Debug.Print Err.Number
Debug.Print Err.Description
Err.Clear
Error 40080
Debug.Print Err.Number
Debug.Print Err.Description
Err.Clear
Debug.Print Error(5)
Debug.Print Error$(5)
Debug.Print Error(40080)
Debug.Print Len(Error(0))
"#;
    assert_eq!(
        output(source),
        [
            "5",
            "无效的过程调用或参数",
            "40080",
            "定义的应用程序或对象错误",
            "无效的过程调用或参数",
            "无效的过程调用或参数",
            "定义的应用程序或对象错误",
            "0"
        ]
    );
}

/// 接口的 `Property Set` / `Get` / `Let` 分派：本机 VB6 实测（`Implements` + `接口名_属性` 三个访问器，
/// `Set h.Payload = t` / `h.Title = "x"` / 读回）与 QIDE 行为一致，都走实现类的私有成员。
#[test]
fn interface_property_set_dispatches_to_the_implementing_class() {
    let source = r#"
Class IHolder
    Public Property Get Payload() As Object
    End Property
    Public Property Set Payload(ByVal value As Object)
    End Property
    Public Property Get Title() As String
    End Property
    Public Property Let Title(ByVal value As String)
    End Property
End Class

Class Holder
    Implements IHolder
    Private mPayload As Object
    Private mTitle As String
    Private Property Get IHolder_Payload() As Object
        Set IHolder_Payload = mPayload
    End Property
    Private Property Set IHolder_Payload(ByVal value As Object)
        Set mPayload = value
    End Property
    Private Property Get IHolder_Title() As String
        IHolder_Title = mTitle
    End Property
    Private Property Let IHolder_Title(ByVal value As String)
        mTitle = value
    End Property
End Class

Class Thing
    Public Name As String
End Class

Dim h As IHolder
Dim t As Thing
Dim hh As Holder
Set hh = New Holder
Set h = hh
Set t = New Thing
t.Name = "abc"
Set h.Payload = t
h.Title = "hello"
Debug.Print h.Payload.Name
Debug.Print h.Title
Debug.Print hh.Title
"#;
    assert_eq!(output(source), ["abc", "hello", "hello"]);
}

/// `Hex` / `Oct` 的宽度跟着实参的静态类型走（全部为本机 VB6 实测值）。
#[test]
fn hex_and_oct_widths_follow_the_static_type() {
    let source = r#"
Dim i As Integer, l As Long, b As Byte
i = -1
l = -1
b = 200
Debug.Print Hex(i) & " " & Hex(l) & " " & Hex(b)
Debug.Print Oct(i) & " " & Oct(l)
Debug.Print Hex(-1) & " " & Hex(-1&) & " " & Hex(-200) & " " & Hex(-32768)
Debug.Print Oct(-1) & " " & Oct(-1&)
Debug.Print Hex(1) & " " & Hex(255) & " " & Hex(1 + 1) & " " & Hex(1 - 2)
"#;
    assert_eq!(
        output(source),
        [
            "FFFF FFFFFFFF C8",
            "177777 37777777777",
            "FFFF FFFFFFFF FF38 FFFF8000",
            "177777 37777777777",
            "1 FF 2 FFFF"
        ]
    );
}

/// 整数运算溢出报错误号 6（全部为本机 VB6 实测：`32767 + 1`、`32767 * 2`、`2147483647 + 1`、
/// `2147483647 * 2`、`-i`（Integer 的 -32768）都报 6；
/// `-32768 - 1`（字面量已是 Long）、`c + c`（Long 30000）、`200 + 200`、`32767 Mod -1` 都不报）。
///
/// `Byte` 的运算溢出（VB6 里 `CByte(200) + CByte(200)` 也报 6）不做检查：类型推断要多查一次
/// 静态类型，热路径上不划算，见 `docs/脚本语言.md` 的已知边界。
#[test]
fn integer_arithmetic_overflow_reports_error_six() {
    let source = r#"
Dim v As Variant, i As Integer, c As Long
On Error Resume Next
v = 32767 + 1
Debug.Print Err.Number
Err.Clear
v = 32767 * 2
Debug.Print Err.Number
Err.Clear
v = 2147483647 + 1
Debug.Print Err.Number
Err.Clear
v = 2147483647 * 2
Debug.Print Err.Number
Err.Clear
i = -32768
v = -i
Debug.Print Err.Number
Err.Clear
v = -32768 - 1
Debug.Print Err.Number
Err.Clear
c = 30000
v = c + c
Debug.Print Err.Number
Debug.Print v
v = 200 + 200
Debug.Print Err.Number
Debug.Print v
v = 32767 Mod -1
Debug.Print Err.Number
Debug.Print v
"#;
    assert_eq!(
        output(source),
        [
            "6", "6", "6", "6", "6", "0", "0", "60000", "0", "400", "0", "0"
        ]
    );
}

/// 下标超过栈上缓冲上限（8 维）时退回堆分配的分支也要正确。
#[test]
fn nine_dimension_array_indexing_round_trips() {
    let source = r#"
Dim a() As Long
ReDim a(1 To 2, 1 To 2, 1 To 2, 1 To 2, 1 To 2, 1 To 2, 1 To 2, 1 To 2, 1 To 2)
a(2, 2, 2, 2, 2, 2, 2, 2, 2) = 42
Debug.Print a(2, 2, 2, 2, 2, 2, 2, 2, 2)
Debug.Print a(1, 1, 1, 1, 1, 1, 1, 1, 1)
Debug.Print UBound(a, 9)
"#;
    assert_eq!(output(source), ["42", "0", "2"]);
}

/// 数组元素是强类型时，写入仍要按元素类型检查（快路径不能绕过溢出）。
#[test]
fn array_element_assignment_still_checks_the_element_type() {
    assert_eq!(
        runtime_error_number("Dim a(1) As Integer\na(0) = 40000"),
        6,
        "数组元素溢出应当报错误号 6"
    );
    assert_eq!(
        output("Dim a(1) As String\na(0) = 5\nDebug.Print a(0)"),
        ["5"],
        "字符串数组元素仍按元素类型转换"
    );
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
    // 定点写法的小数位不超过 15 位就保持定点（VB6 实测 `CStr(0.0001)` 是 `0.0001`）。
    assert_eq!(output("Debug.Print 0.0001"), ["0.0001"]);
    assert_eq!(output("Debug.Print 0.001"), ["0.001"]);
    assert_eq!(output("Debug.Print 1.5E-14"), ["0.000000000000015"]);
    assert_eq!(
        output("Debug.Print 1.23456789012345E-11"),
        ["1.23456789012345E-11"]
    );
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
    let source =
        "Dim i As LongLong\nFor i = 9007199254740990 To 9007199254740992\nDebug.Print i\nNext i";
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
    std::fs::write(
        dir.join("shared.vb"),
        "Private Const FROM_DIR As Long = 7\n",
    )
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

#[test]
fn record_and_class_types_can_be_declared_after_their_use() {
    let source = "\
Option Explicit
Dim u As LaterType
u.a = 5
Dim p As Point3D
Set p = New Point3D
p.X = 7
Debug.Print u.a & \",\" & p.X
Type LaterType
    a As Long
End Type
Class Point3D
    Public X As Long
End Class
";
    assert_eq!(output(source), ["5,7"]);
}

#[test]
fn implements_can_reference_an_interface_declared_later() {
    let source = "\
Option Explicit
Dim g As Greeter
Set g = New Hello
Debug.Print g.Greet(\"world\")
Class Hello
    Implements Greeter
    Private Function Greeter_Greet(n As String) As String
        Greeter_Greet = \"hi \" & n
    End Function
End Class
Class Greeter
    Public Function Greet(n As String) As String
    End Function
End Class
";
    assert_eq!(output(source), ["hi world"]);
}

#[test]
fn unknown_type_error_keeps_the_source_spelling() {
    let flag = AtomicBool::new(false);
    let error = run("Dim x As NoSuchType\n", |_| {}, &flag).expect_err("类型不存在时应当报错");
    assert!(error.message().contains("NoSuchType"), "{error}");
}

#[test]
fn record_types_cannot_contain_themselves() {
    let flag = AtomicBool::new(false);
    let direct = run("Type SelfRef\n    me As SelfRef\nEnd Type\n", |_| {}, &flag)
        .expect_err("自引用应当报错");
    assert!(direct.message().contains("包含自身"), "{direct}");

    let mutual = "\
Type Outer
    inner As Inner
End Type
Type Inner
    outer As Outer
End Type
";
    let error = run(mutual, |_| {}, &flag).expect_err("互相引用应当报错");
    assert!(error.message().contains("包含自身"), "{error}");
}
