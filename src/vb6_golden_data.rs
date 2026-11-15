//! 由 `tools/vb6_golden.py` 生成，请勿手工修改。
//!
//! 期望值来自本机 VB6 真编译（`CompilationType=-1`，IDE 调试用的 p-code 形态）的实际输出。

/// 表达式 → `CStr(表达式)` 的结果。
pub const CASES: &[(&str, &str)] = &[
    ("1 + 2 * 3", "7"),
    ("2 ^ 3 ^ 2", "64"),
    ("-2 ^ 2", "-4"),
    ("7 \\ 2", "3"),
    ("7 Mod 2", "1"),
    ("1 / 2", "0.5"),
    ("\"a\" & \"b\"", "ab"),
    ("\"a\" + \"b\"", "ab"),
    ("True + 1", "0"),
    ("Not 0", "-1"),
    ("5 And 3", "1"),
    ("5 Or 3", "7"),
    ("5 Xor 3", "6"),
    ("5 Eqv 3", "-7"),
    ("5 Imp 3", "-5"),
    ("CInt(2.5)", "2"),
    ("CInt(3.5)", "4"),
    ("CLng(-2.5)", "-2"),
    ("CStr(1.5)", "1.5"),
    ("CStr(True)", "True"),
    ("CSng(1.23456789)", "1.234568"),
    ("CCur(1.5)", "1.5"),
    ("CByte(200)", "200"),
    ("CDbl(\"1e3\")", "1000"),
    ("CVar(3)", "3"),
    ("VarType(1.5)", "5"),
    ("VarType(\"s\")", "8"),
    ("VarType(True)", "11"),
    ("Len(\"abc\")", "3"),
    ("Left(\"abcdef\", 2)", "ab"),
    ("Mid(\"abcdef\", 2, 3)", "bcd"),
    ("InStr(\"abcabc\", \"b\")", "2"),
    ("StrReverse(\"abc\")", "cba"),
    ("UCase(\"aB\")", "AB"),
    ("LCase(\"aB\")", "ab"),
    ("Replace(\"aXbX\", \"X\", \"-\")", "a-b-"),
    ("StrComp(\"a\", \"A\", 1)", "0"),
    ("Trim(\"  x  \")", "x"),
    ("Int(-1.5)", "-2"),
    ("Fix(-1.5)", "-1"),
    ("Round(2.5)", "2"),
    ("Abs(-3)", "3"),
    ("Sgn(-5)", "-1"),
    ("Sqr(9)", "3"),
    ("Hex(255)", "FF"),
    ("Oct(8)", "10"),
    ("Val(\"12abc\")", "12"),
    ("Str(5)", " 5"),
    ("Format(1234.5678, \"0.00\")", "1234.57"),
    ("Format(0.5, \"0.00%\")", "50.00%"),
    ("IsNumeric(\"12\")", "True"),
    ("IsNumeric(\"1a\")", "False"),
    ("IsEmpty(Empty)", "True"),
    ("Choose(2, \"a\", \"b\")", "b"),
    ("Switch(True, 1, False, 2)", "1"),
    ("2 * 3 + 4", "10"),
    ("2 + 3 * 4", "14"),
    ("1 - 2 - 3", "-4"),
    ("100 / 10 / 5", "2"),
    ("2 ^ 2 ^ 3", "64"),
    ("-3 ^ 2", "-9"),
    ("Not True", "False"),
    ("Not 5", "-6"),
    ("1 = 1", "True"),
    ("1 <> 1", "False"),
    ("1 < 2", "True"),
    ("\"a\" < \"b\"", "True"),
    ("True And False", "False"),
    ("True Or False", "True"),
    ("True Xor True", "False"),
    ("\"abc\" Like \"a*\"", "True"),
    ("\"abc\" Like \"a?\"", "False"),
    ("\"abc\" Like \"a[bd]c\"", "True"),
    ("5 \\ 2 * 2", "1"),
    ("7 Mod 3 + 1", "2"),
    ("Len(\"\")", "0"),
    ("Len(\"中文\")", "2"),
    ("Left(\"abc\", 10)", "abc"),
    ("Right(\"abc\", 10)", "abc"),
    ("Mid(\"abc\", 10)", ""),
    ("UCase(\"\")", ""),
    ("Space(3) & \"x\"", "   x"),
    ("String(3, \"ab\")", "aaa"),
    ("InStr(\"abc\", \"\")", "1"),
    ("InStrRev(\"abcabc\", \"b\")", "5"),
    ("Replace(\"aaa\", \"a\", \"bb\")", "bbbbbb"),
    ("StrComp(\"a\", \"b\")", "-1"),
    ("StrComp(\"b\", \"a\")", "1"),
    ("StrComp(\"a\", \"a\", 0)", "0"),
    ("Str(0)", " 0"),
    ("Str(0.5)", " .5"),
    ("Str(-0.5)", "-.5"),
    ("Str(1.5)", " 1.5"),
    ("CStr(0.5)", "0.5"),
    ("CStr(-0.5)", "-0.5"),
    ("CStr(1.5)", "1.5"),
    ("Trim(Str(0.5))", ".5"),
    ("Val(\"  12\")", "12"),
    ("Val(\"&H10\")", "16"),
    ("Val(\"1.5e2\")", "150"),
    ("Hex(0)", "0"),
    ("Oct(0)", "0"),
    ("Format(255, \"0.00\")", "255.00"),
    ("Format(-1.5, \"0.00\")", "-1.50"),
    ("Format(1234567, \"#,##0\")", "1,234,567"),
    ("CInt(1.5)", "2"),
    ("CInt(-1.5)", "-2"),
    ("CInt(0.5)", "0"),
    ("CLng(1.5)", "2"),
    ("CInt(32767)", "32767"),
    ("CLng(2147483647)", "2147483647"),
    ("CCur(0.0001)", "0.0001"),
    ("CCur(-0.0001)", "-0.0001"),
    ("CSng(0.1)", "0.1"),
    ("CDbl(0.1)", "0.1"),
    ("CByte(0)", "0"),
    ("CByte(255)", "255"),
    ("Abs(-1.5)", "1.5"),
    ("Int(1.9)", "1"),
    ("Fix(1.9)", "1"),
    ("Round(1.5)", "2"),
    ("Round(3.14159, 2)", "3.14"),
    ("Sgn(0)", "0"),
    ("Sqr(2)", "1.4142135623731"),
    ("Exp(0)", "1"),
    ("Log(1)", "0"),
    ("VarType(Empty)", "0"),
    ("VarType(Null)", "1"),
    ("VarType(True)", "11"),
    ("VarType(1.5)", "5"),
    ("VarType(CCur(1))", "6"),
    ("VarType(CDate(0))", "7"),
    ("IsNumeric(\"\")", "False"),
    ("IsNumeric(Empty)", "True"),
    ("IsEmpty(Empty)", "True"),
    ("IsNull(Null)", "True"),
    ("TypeName(1)", "Integer"),
    ("TypeName(1.5)", "Double"),
    ("TypeName(\"s\")", "String"),
    ("TypeName(True)", "Boolean"),
    ("Year(DateSerial(2024, 2, 29))", "2024"),
    ("Month(DateSerial(2024, 2, 29))", "2"),
    ("Day(DateSerial(2024, 2, 29))", "29"),
    ("Weekday(DateSerial(2024, 1, 1))", "2"),
    ("DateAdd(\"d\", 1, DateSerial(2024, 2, 28))", "2024/2/29"),
    ("DateDiff(\"d\", DateSerial(2024, 1, 1), DateSerial(2024, 3, 1))", "60"),
    ("CStr(CDate(0))", "0:00:00"),
    ("CStr(CDate(1))", "1899/12/31"),
    ("Hour(TimeSerial(13, 5, 9))", "13"),
    ("Minute(TimeSerial(13, 5, 9))", "5"),
    ("Second(TimeSerial(13, 5, 9))", "9"),
    ("Partition(5, 0, 99, 10)", "  0:  9"),
    ("Partition(0, 0, 99, 10)", "  0:  9"),
    ("Partition(99, 0, 99, 10)", " 90: 99"),
    ("Partition(100, 0, 99, 10)", "100:   "),
    ("Partition(0, 1, 99, 10)", "   :  0"),
    ("Partition(5, 1, 10, 1)", " 5: 5"),
    ("Partition(5, 0, 999, 1)", "   5:   5"),
    ("Partition(5, 0, 9999, 10)", "    0:    9"),
    ("Partition(5, 0, 100000, 1000)", "     0:   999"),
    ("Partition(5, 0, 100, 1000)", "  0:100"),
    ("Partition(23, 0, 99, 50)", "  0: 49"),
    ("Partition(150, 0, 99, 10)", "100:   "),
    ("Left$(\"abcdef\", 2)", "ab"),
    ("Right$(\"abcdef\", 3)", "def"),
    ("Mid$(\"abcdef\", 2)", "bcdef"),
    ("Mid$(\"abcdef\", 2, 2)", "bc"),
    ("Trim$(\"  x  \")", "x"),
    ("LTrim$(\"  x  \")", "x  "),
    ("RTrim$(\"  x  \")", "  x"),
    ("UCase$(\"aB\")", "AB"),
    ("LCase$(\"aB\")", "ab"),
    ("Space$(0)", ""),
    ("Space$(3) & \"x\"", "   x"),
    ("String$(3, \"ab\")", "aaa"),
    ("Chr$(65)", "A"),
    ("InStr(2, \"abcabc\", \"b\")", "2"),
    ("InStr(1, \"abc\", \"B\")", "0"),
    ("InStr(1, \"abc\", \"B\", 1)", "2"),
    ("InStr(1, \"aBc\", \"b\", 1)", "2"),
    ("InStr(3, \"aBcaBc\", \"b\", 1)", "5"),
    ("InStr(4, \"abc\", \"a\", 0)", "0"),
    ("InStrRev(\"aBcaBc\", \"b\")", "0"),
    ("InStrRev(\"aBcaBc\", \"b\", -1, 1)", "5"),
    ("InStrRev(\"abcabc\", \"b\", 3)", "2"),
    ("InStrRev(\"abcabc\", \"b\", 3, 0)", "2"),
    ("Replace(\"aXbX\", \"X\", \"-\", 1, -1, 1)", "a-b-"),
    ("Replace(\"aXbX\", \"x\", \"-\", 1, -1, 1)", "a-b-"),
    ("Replace(\"aXbX\", \"X\", \"-\", 3)", "b-"),
    ("Replace(\"aXbX\", \"X\", \"-\", 1, 1)", "a-bX"),
    ("Replace(\"abc\", \"\", \"-\")", "abc"),
    ("Replace(\"aaa\", \"a\", \"b\", 2)", "bb"),
    ("Round(0.5)", "0"),
    ("Round(1.5)", "2"),
    ("Round(-1.5)", "-2"),
    ("Round(-2.5)", "-2"),
    ("Round(2.675, 2)", "2.68"),
    ("Round(1.2345, 2)", "1.23"),
    ("Int(-0.5)", "-1"),
    ("Fix(-0.5)", "0"),
    ("-7 \\ 2", "-3"),
    ("7 \\ -2", "-3"),
    ("-7 Mod 2", "-1"),
    ("7 Mod -2", "1"),
    ("-7.5 \\ 2", "-4"),
    ("5.5 \\ 2", "3"),
    ("5.5 Mod 2", "0"),
    ("Int(-7 / 2)", "-4"),
    ("Hex(-1)", "FFFF"),
    ("Oct(-1)", "177777"),
    ("Hex(3.9)", "4"),
    ("Oct(3.9)", "4"),
    ("Hex(255.4)", "FF"),
    ("AscW(\"中\")", "20013"),
    ("ChrW(20013)", "中"),
    ("Asc(\"A\")", "65"),
    ("Asc(\" \")", "32"),
    ("UBound(Array())", "-1"),
    ("UBound(Array(1, 2, 3))", "2"),
    ("LBound(Array(1, 2, 3))", "0"),
    ("UBound(Split(\"a,b,c\", \",\"))", "2"),
    ("UBound(Split(\"\"))", "-1"),
    ("Join(Array(\"a\", \"b\"), \"-\")", "a-b"),
    ("Join(Array(1, 2, 3), \"+\")", "1+2+3"),
    ("UBound(Filter(Array(\"ax\", \"b\", \"ay\"), \"a\"))", "1"),
    ("Len(CStr(123))", "3"),
    ("Len(CStr(1.5))", "3"),
    ("Len(CStr(True))", "4"),
    ("IIf(True, 1, 2)", "1"),
    ("IIf(False, \"a\", \"b\")", "b"),
    ("IIf(1, \"yes\", \"no\")", "yes"),
    ("Format(1234.5678, \"General Number\")", "1234.5678"),
    ("Format(1234.5678, \"Fixed\")", "1234.57"),
    ("Format(1234.5678, \"Standard\")", "1,234.57"),
    ("Format(1234, \"Scientific\")", "1.23E+03"),
    ("Format(0.5, \"Percent\")", "50.00%"),
    ("Format(True, \"Yes/No\")", "Yes"),
    ("Format(False, \"Yes/No\")", "No"),
    ("Format(True, \"True/False\")", "True"),
    ("Format(True, \"On/Off\")", "On"),
    ("Format(1234.5678, \"##0.0\")", "1234.6"),
    ("Format(1234.5678, \"0.00E+00\")", "1.23E+03"),
    ("Format(-1234.5, \"#,##0.00;(#,##0.00)\")", "(1,234.50)"),
    ("Format(0, \"0.00\")", "0.00"),
    ("Format(0.5, \"0\")", "1"),
    ("Format(2.5, \"0\")", "3"),
    ("Format(1.5, \"0\")", "2"),
    ("Format(1234567, \"#,##0\")", "1,234,567"),
    ("Format(0.5, \"0.00%\")", "50.00%"),
    ("Format(12, \"000\")", "012"),
];

/// 会失败的语句 → `Err.Number`。
pub const ERROR_CASES: &[(&str, i64)] = &[
    ("x = 1 / 0", 11),
    ("x = 1 \\ 0", 11),
    ("x = 7 Mod 0", 11),
    ("i = 40000", 6),
    ("b = 300", 6),
    ("l = 2147483648", 6),
    ("x = CInt(40000)", 6),
    ("x = CByte(256)", 6),
    ("x = CByte(-1)", 6),
    ("x = CLng(\"abc\")", 13),
    ("x = CSng(1) / 0", 11),
    ("d = \"not a date\"", 13),
    ("x = arr(5)", 9),
    ("x = empty_arr(0)", 9),
    ("x = 1 + \"abc\"", 13),
    ("x = c.Count", 91),
    ("x = Sgn(\"abc\")", 13),
    ("Kill \"no_such_file_for_golden_test.txt\"", 53),
    ("Open \"no_such_dir_for_golden\\x.txt\" For Input As #2", 76),
    ("x = Int(\"abc\")", 13),
    ("x = Abs(\"abc\")", 13),
    ("Print #9, \"x\"", 52),
    ("Seek #9, 1", 52),
    ("x = Partition(5, 0, 99, 0)", 5),
    ("x = Partition(5, 0, 99, -1)", 5),
    ("x = Partition(5, -10, 10, 5)", 5),
    ("x = Partition(5, 10, 5, 1)", 5),
    ("x = Partition(5, 0, 0, 1)", 5),
];

/// 脚本用例共用的声明与辅助过程（与 VB6 侧同一份）。
pub const SCRIPT_SUPPORT: &str = r#"
Private Type Pt
    X As Long
    Y As Long
End Type

Function Double2(ByVal n As Long) As Long
    Double2 = n * 2
End Function

Function Triple(ByVal n As Long) As Long
    Triple = n * 3
End Function

Function Fact(ByVal n As Long) As Long
    If n <= 1 Then
        Fact = 1
    Else
        Fact = n * Fact(n - 1)
    End If
End Function

Sub Bump(ByRef n As Long)
    n = n + 1
End Sub

Function Greet(ByVal name As String, Optional ByVal suffix As String = "") As String
    Greet = "Hi " & name & suffix
End Function

Function Counter() As Long
    Static calls As Long
    calls = calls + 1
    Counter = calls
End Function

Function Pick(ByVal n As Long) As String
    If n = 0 Then
        Pick = "zero"
        Exit Function
    End If
    Pick = "other"
End Function

Function SumTo(ByVal n As Long) As Long
    Static depth As Long
    depth = depth + 1
    If n <= 0 Then
        SumTo = depth
    Else
        SumTo = n + SumTo(n - 1)
    End If
End Function

Sub Twice(ByVal value As Long, ByRef doubled As Long)
    doubled = value * 2
End Sub

Private Sub MovePt(ByRef p As Pt, ByVal dx As Long)
    p.X = p.X + dx
End Sub

Function SumAll(ParamArray xs() As Variant) As Long
    Dim i As Long, total As Long
    For i = LBound(xs) To UBound(xs)
        total = total + xs(i)
    Next
    SumAll = total
End Function

Function StaticArr(ByVal i As Long) As Long
    Static buf(0 To 2) As Long
    buf(i) = buf(i) + 1
    StaticArr = buf(i)
End Function
"#;

/// (用例名, 模块级选项（`Option Base 1` 等，空串表示没有）, 语句块, `Debug.Print` 的期望输出（行之间用 `\n` 分隔））。
pub const SCRIPT_CASES: &[(&str, &str, &str, &str)] = &[
    ("for_counter_overflow", "", "Dim i As Integer, n As Long, j As Long
On Error Resume Next
Err.Clear
For i = 32767 To 32767
    n = n + 1
Next i
Debug.Print n, Err.Number
Err.Clear
n = 0
For i = 32766 To 32767 Step 1
    n = n + 1
Next i
Debug.Print n, Err.Number
Err.Clear
n = 0
For i = -32768 To -32768
    n = n + 1
Next i
Debug.Print n, Err.Number
Err.Clear
n = 0
For j = 2147483647 To 2147483647
    n = n + 1
Next j
Debug.Print n, Err.Number", " 1             6\n 2             6\n 1             0\n 1             6"),
    ("for_counter_after", "", "Dim i As Long
For i = 1 To 4
Next i
Debug.Print i
For i = 1 To 10 Step 3
Next i
Debug.Print i
For i = 5 To 1 Step -2
Next i
Debug.Print i
For i = 5 To 1
Next i
Debug.Print i
For i = 1 To 10
    If i = 4 Then Exit For
Next i
Debug.Print i", " 5\n 13\n-1\n 5\n 4"),
    ("for_step", "", "Dim i As Long
For i = 1 To 10 Step 3
    Debug.Print i
Next i", " 1\n 4\n 7\n 10"),
    ("for_negative_step", "", "Dim i As Long
For i = 5 To 1 Step -2
    Debug.Print i
Next i", " 5\n 3\n 1"),
    ("for_zero_iterations", "", "Dim i As Long
For i = 5 To 1
    Debug.Print \"never\"
Next i
Debug.Print \"done\"", "done"),
    ("for_empty_body", "", "Dim i As Long, n As Long
For i = 1 To 4
    n = n + i
Next i
Debug.Print n", " 10"),
    ("nested_for", "", "Dim i As Long, j As Long, s As Long
For i = 1 To 3
    For j = 1 To 2
        s = s + i * j
    Next j
Next i
Debug.Print s", " 18"),
    ("exit_for", "", "Dim i As Long
For i = 1 To 10
    If i = 4 Then Exit For
    Debug.Print i
Next i
Debug.Print \"after \" & i", " 1\n 2\n 3\nafter 4"),
    ("do_while", "", "Dim n As Long
n = 3
Do While n > 0
    Debug.Print n
    n = n - 1
Loop", " 3\n 2\n 1"),
    ("do_until", "", "Dim n As Long
Do
    n = n + 1
Loop Until n >= 3
Debug.Print n", " 3"),
    ("while_wend", "", "Dim n As Long
While n < 3
    n = n + 1
Wend
Debug.Print n", " 3"),
    ("do_exit", "", "Dim n As Long
Do
    n = n + 1
    If n = 3 Then Exit Do
Loop
Debug.Print n", " 3"),
    ("select_int", "", "Dim grade As Long
grade = 85
Select Case grade
    Case Is >= 90
        Debug.Print \"优\"
    Case 60 To 89
        Debug.Print \"及格\"
    Case Else
        Debug.Print \"其它\"
End Select", "及格"),
    ("select_string", "", "Dim s As String
s = \"b\"
Select Case s
    Case \"a\"
        Debug.Print 1
    Case \"b\", \"c\"
        Debug.Print 2
    Case Else
        Debug.Print 3
End Select", " 2"),
    ("select_no_match", "", "Dim n As Long
n = 99
Select Case n
    Case 1
        Debug.Print \"one\"
End Select
Debug.Print \"after\"", "after"),
    ("goto_label", "", "Dim n As Long
Start:
n = n + 1
If n < 3 Then GoTo Start
Debug.Print n", " 3"),
    ("gosub_return", "", "Dim n As Long
GoSub Helper
Debug.Print \"back:\" & n
Exit Sub
Helper:
n = 42
Return", "back:42"),
    ("if_elseif_else", "", "Dim n As Long
n = 2
If n = 1 Then
    Debug.Print \"one\"
ElseIf n = 2 Then
    Debug.Print \"two\"
Else
    Debug.Print \"many\"
End If", "two"),
    ("on_error_resume_next", "", "Dim x As Long
On Error Resume Next
x = 1 / 0
Debug.Print Err.Number
x = 5
Debug.Print x
On Error GoTo 0
Debug.Print \"ok\"", " 11\n 5\nok"),
    ("on_error_goto_handler", "", "Dim x As Long
On Error GoTo Handler
x = 1 / 0
Debug.Print \"not reached\"
Exit Sub
Handler:
Debug.Print \"handled \" & Err.Number", "handled 11"),
    ("array_basics", "", "Dim a(3) As Long
Dim i As Long
For i = 0 To 3
    a(i) = i * i
Next i
Debug.Print a(0), a(3)
Debug.Print LBound(a), UBound(a)", " 0             9\n 0             3"),
    ("array_dynamic_redim", "", "Dim a() As Long
ReDim a(1 To 3)
Dim i As Long
For i = 1 To 3
    a(i) = i
Next i
ReDim Preserve a(1 To 5)
Debug.Print a(3), a(5), UBound(a)", " 3             0             5"),
    ("array_2d", "", "Dim m(1, 2) As Long
m(0, 0) = 1
m(1, 2) = 6
Debug.Print m(0, 0) + m(1, 2)
Debug.Print UBound(m, 1), UBound(m, 2)", " 7\n 1             2"),
    ("array_string_join", "", "Dim a(2) As String
a(0) = \"x\"
a(1) = \"y\"
a(2) = \"z\"
Debug.Print Join(a, \"-\")", "x-y-z"),
    ("udt_basic", "", "Dim p As Pt
p.X = 3
p.Y = 4
Debug.Print p.X * p.Y", " 12"),
    ("with_block", "", "Dim p As Pt
With p
    .X = 5
    .Y = 6
End With
Debug.Print p.X + p.Y", " 11"),
    ("string_concat_loop", "", "Dim i As Long, s As String
For i = 1 To 3
    s = s & i
Next i
Debug.Print s", "123"),
    ("string_function_mix", "", "Dim s As String
s = \"Hello, World\"
Debug.Print UCase(Left(s, 5))
Debug.Print InStr(s, \",\")
Debug.Print Mid(s, 8)
Debug.Print Replace(s, \"l\", \"L\")", "HELLO\n 6\nWorld\nHeLLo, WorLd"),
    ("numeric_accumulate", "", "Dim i As Long, total As Double
For i = 1 To 100
    total = total + i
Next i
Debug.Print total", " 5050"),
    ("nested_function_call", "", "Dim n As Long
n = Double2(Triple(2))
Debug.Print n", " 12"),
    ("recursive", "", "Dim n As Long
n = Fact(5)
Debug.Print n", " 120"),
    ("byref_parameter", "", "Dim n As Long
n = 1
Bump n
Debug.Print n", " 2"),
    ("optional_parameter", "", "Debug.Print Greet(\"A\")
Debug.Print Greet(\"B\", \"!\")", "Hi A\nHi B!"),
    ("static_local", "", "Debug.Print Counter()
Debug.Print Counter()
Debug.Print Counter()", " 1\n 2\n 3"),
    ("boolean_ops", "", "Dim a As Boolean, b As Boolean
a = True
b = False
Debug.Print (a And b), (a Or b), (a Xor b), (Not a)
Debug.Print (a And b) = False", "False         True          True          False\nTrue"),
    ("print_numeric_format", "", "Dim n As Long, d As Double
n = 5
d = 0.5
Debug.Print n
Debug.Print -n
Debug.Print d
Debug.Print -d
Debug.Print 1, 2
Debug.Print True, False", " 5\n-5\n 0.5\n-0.5\n 1             2\nTrue          False"),
    ("print_empty_and_null", "", "Dim v
Debug.Print \"[\" & Empty & \"]\"
Debug.Print \"[\" & Null & \"]\"
Debug.Print v", "[]\n[]"),
    ("null_propagation", "", "Dim v
Debug.Print \"[\" & Null & \"]\"
Debug.Print TypeName(Null & \"x\")
v = Null + 1
Debug.Print TypeName(v)
v = Null + \"x\"
Debug.Print TypeName(v)
v = Null * 2
Debug.Print TypeName(v)
Debug.Print TypeName(Null = Null)", "[]\nString\nNull\nNull\nNull\nNull"),
    ("for_each_array", "", "Dim a(3) As Long
Dim i As Long, total As Long, item
For i = 0 To 3
    a(i) = i + 1
Next i
For Each item In a
    total = total + item
Next item
Debug.Print total", " 10"),
    ("collection_basics", "", "Dim c As Collection
Set c = New Collection
c.Add \"a\"
c.Add \"b\"
Debug.Print c.Count
Debug.Print c.Item(2)
Debug.Print c.Item(1)", " 2\nb\na"),
    ("mid_statement", "", "Dim s As String
s = \"abcdef\"
Mid(s, 2, 3) = \"XY\"
Debug.Print s", "aXYdef"),
    ("string_build_and_parse", "", "Dim i As Long, s As String, parts
For i = 1 To 3
    s = s & i & \",\"
Next i
Debug.Print s
parts = Split(s, \",\")
Debug.Print UBound(parts)
Debug.Print parts(0)", "1,2,3,\n 3\n1"),
    ("exit_function_early", "", "Debug.Print Pick(0)
Debug.Print Pick(5)", "zero\nother"),
    ("udt_array", "", "Dim pts(2) As Pt
Dim i As Long, total As Long
For i = 0 To 2
    pts(i).X = i
    pts(i).Y = i * 2
Next i
For i = 0 To 2
    total = total + pts(i).X + pts(i).Y
Next i
Debug.Print total", " 9"),
    ("nested_with", "", "Dim a As Pt, b As Pt
With a
    .X = 1
    With b
        .Y = 2
    End With
    .Y = 3
End With
Debug.Print a.X + a.Y + b.Y", " 6"),
    ("error_in_loop", "", "Dim i As Long, errors As Long
On Error Resume Next
For i = 1 To 4
    If i Mod 2 = 0 Then
        errors = errors + 1 + (1 / 0)
    End If
Next i
Debug.Print errors
Debug.Print i
On Error GoTo 0", " 0\n 5"),
    ("file_roundtrip", "", "Dim fn As Integer, line1 As String
fn = FreeFile
Open \"golden_io.txt\" For Output As #fn
Print #fn, \"hello\"
Print #fn, 42
Close #fn
fn = FreeFile
Open \"golden_io.txt\" For Input As #fn
Line Input #fn, line1
Debug.Print line1
Input #fn, line1
Debug.Print \"[\" & line1 & \"]\"
Close #fn
Kill \"golden_io.txt\"", "hello\n[42]"),
    ("static_in_recursion", "", "Debug.Print SumTo(4)", " 15"),
    ("select_nested", "", "Dim a As Long, b As Long
a = 1
b = 2
Select Case a
    Case 1
        Select Case b
            Case 2
                Debug.Print \"inner\"
            Case Else
                Debug.Print \"inner-else\"
        End Select
    Case Else
        Debug.Print \"outer-else\"
End Select", "inner"),
    ("string_compare_option", "", "Dim r As Long
r = StrComp(\"abc\", \"ABC\", 1)
Debug.Print r
Debug.Print \"abc\" Like \"a*\"
Debug.Print \"abc\" Like \"b*\"", " 0\nTrue\nFalse"),
    ("numeric_edge", "", "Dim d As Double
d = 1 / 3
Debug.Print d * 3
Debug.Print CInt(d * 3)
Debug.Print -Int(2.5)", " 1\n 1\n-2"),
    ("concat_numbers", "", "Debug.Print 1 & 2
Debug.Print \"5\" + 3
Debug.Print 5 & \"\"
Debug.Print 1 & 2 & 3", "12\n 8\n5\n123"),
    ("arithmetic_negatives", "", "Debug.Print -7 \\ 2
Debug.Print -7 Mod 2
Debug.Print 7 Mod -2
Debug.Print -7 Mod -2
Debug.Print Int(-7 / 2)
Debug.Print Fix(-7 / 2)
Debug.Print 7 \\ -2", "-3\n-1\n 1\n-1\n-4\n-3\n-3"),
    ("format_specs", "", "Debug.Print Format(1234.5, \"0.00\")
Debug.Print Format(1234, \"#,##0\")
Debug.Print Format(5, \"000\")
Debug.Print Format(-1234.5, \"#,##0.00\")
Debug.Print Format(0.25, \"0%\")
Debug.Print Format(0, \"0.0\")", "1234.50\n1,234\n005\n-1,234.50\n25%\n0.0"),
    ("string_number_compare", "", "Debug.Print (\"5\" = 5)
Debug.Print (\"5\" > 4)
Debug.Print (\"a\" < \"b\")
Debug.Print (5 = \"5\")
Debug.Print (\"abc\" = \"ABC\")", "True\nTrue\nTrue\nTrue\nFalse"),
    ("iif_evaluates_both_arguments", "", "On Error Resume Next
Debug.Print IIf(True, 1, 1 / 0)
Debug.Print Err.Number", " 11"),
    ("set_nothing", "", "Dim c As Collection
Set c = New Collection
Debug.Print (c Is Nothing)
Set c = Nothing
Debug.Print (c Is Nothing)", "False\nTrue"),
    ("redim_preserve_2d", "", "Dim a() As Long
ReDim a(0 To 1, 0 To 2)
Dim i As Long, j As Long
For i = 0 To 1
    For j = 0 To 2
        a(i, j) = i * 10 + j
    Next j
Next i
ReDim Preserve a(0 To 1, 0 To 3)
Debug.Print a(0, 0), a(1, 2), a(1, 3)
Debug.Print UBound(a, 2)", " 0             12            0\n 3"),
    ("string_builtin_edges", "", "Debug.Print \"[\" & Left(\"abc\", 0) & \"]\"
Debug.Print \"[\" & Mid(\"abc\", 2, 0) & \"]\"
Debug.Print \"[\" & Mid(\"abc\", 2, 99) & \"]\"
Debug.Print Len(Space(0))
Debug.Print Len(String(0, \"a\"))", "[]\n[]\n[bc]\n 0\n 0"),
    ("nested_byref_out_parameter", "", "Dim n As Long, m As Long
n = 3
Twice n, m
Debug.Print n
Debug.Print m", " 3\n 6"),
    ("option_base_1", "Option Base 1", "Dim a(3) As Long
Dim i As Long, total As Long
Debug.Print LBound(a)
Debug.Print UBound(a)
For i = 1 To 3
    total = total + i
Next i
a(1) = total
Debug.Print a(1)
Debug.Print a(3)", " 1\n 3\n 6\n 0"),
    ("option_compare_text", "Option Compare Text", "Debug.Print (\"abc\" = \"ABC\")
Debug.Print (\"abc\" < \"ABD\")
Debug.Print (\"ABC\" Like \"a*\")
Debug.Print StrComp(\"abc\", \"ABC\")", "True\nTrue\nTrue\n 0"),
    ("option_base_1_2d", "Option Base 1", "Dim m(2, 3) As Long
m(1, 1) = 5
m(2, 3) = 7
Debug.Print m(1, 1) + m(2, 3)
Debug.Print LBound(m, 1), LBound(m, 2)
Debug.Print UBound(m, 1), UBound(m, 2)", " 12\n 1             1\n 2             3"),
    ("write_hash_roundtrip", "", "Dim fn As Integer, s As String, n As Long, d As Double
fn = FreeFile
Open \"golden_w.txt\" For Output As #fn
Write #fn, \"hello\", 42, 1.5
Close #fn
fn = FreeFile
Open \"golden_w.txt\" For Input As #fn
Input #fn, s, n, d
Debug.Print s
Debug.Print n
Debug.Print d
Close #fn
Kill \"golden_w.txt\"", "hello\n 42\n 1.5"),
    ("random_udt_roundtrip", "", "Dim fn As Integer, p As Pt, q As Pt
fn = FreeFile
Open \"golden_r.dat\" For Random As #fn Len = Len(p)
p.X = 11
p.Y = 22
Put #fn, 1, p
p.X = 33
Put #fn, 2, p
Get #fn, 1, q
Debug.Print q.X, q.Y
Get #fn, 2, q
Debug.Print q.X, q.Y
Debug.Print LOF(fn)
Close #fn
Kill \"golden_r.dat\"", " 11            22\n 33            22\n 16"),
    ("binary_seek_roundtrip", "", "Dim fn As Integer, a As Byte, b As Byte
fn = FreeFile
Open \"golden_b.dat\" For Binary As #fn
a = 65
Put #fn, 1, a
a = 66
Put #fn, 2, a
a = 67
Put #fn, 3, a
Seek #fn, 2
Get #fn, , b
Debug.Print b
Debug.Print LOF(fn)
Debug.Print Loc(fn)
Close #fn
Kill \"golden_b.dat\"", " 66\n 3\n 2"),
    ("put_conversion_widths", "", "Dim fn As Integer
Dim b As Byte, i As Integer, l As Long
fn = FreeFile
Open \"golden_pw.dat\" For Binary As #fn
Put #fn, 1, CByte(65)
Put #fn, 2, CInt(1000)
Put #fn, 4, CLng(70000)
Debug.Print LOF(fn)
Seek #fn, 1
Get #fn, , b
Get #fn, , i
Get #fn, , l
Debug.Print b
Debug.Print i
Debug.Print l
Close #fn
Kill \"golden_pw.dat\"", " 7\n 65\n 1000\n 70000"),
    ("err_object_state", "", "Dim x As Long
On Error Resume Next
Err.Clear
x = 1 / 0
Debug.Print Err.Number
Debug.Print Err.Description
Err.Clear
Debug.Print Err.Number", " 11\n除数为零\n 0"),
    ("nested_error_handler", "", "Dim x As Long
On Error Resume Next
x = 1 / 0
Debug.Print \"after \" & x
x = 6
Debug.Print x", "after 0\n 6"),
    ("udt_byref_mutation", "", "Dim p As Pt
p.X = 1
MovePt p, 5
Debug.Print p.X
Debug.Print p.Y", " 6\n 0"),
    ("paramarray_sum", "", "Debug.Print SumAll(1, 2, 3)
Debug.Print SumAll()
Debug.Print SumAll(10)
Debug.Print SumAll(1, 2)", " 6\n 0\n 10\n 3"),
    ("static_array", "", "Debug.Print StaticArr(0)
Debug.Print StaticArr(0)
Debug.Print StaticArr(2)
Debug.Print StaticArr(0)", " 1\n 2\n 1\n 3"),
    ("collection_mutation", "", "Dim c As New Collection
Dim v
c.Add 1
c.Add 2
c.Add 3, \"third\"
Debug.Print c.Count
Debug.Print c.Item(\"third\")
c.Remove 1
Debug.Print c.Count
For Each v In c
    Debug.Print v
Next
c.Add 9, \"fourth\"
Debug.Print c.Count", " 3\n 3\n 2\n 2\n 3\n 3"),
    ("array_assignment_copy", "", "Dim a, b
a = Array(7, 8)
b = a
b(0) = 9
Debug.Print a(0)
Debug.Print b(0)
Debug.Print UBound(b)", " 7\n 9\n 1"),
    ("select_case_forms", "", "Dim n As Long
For n = 0 To 6
    Select Case n
        Case 1, 2
            Debug.Print \"low\"
        Case Is > 4
            Debug.Print \"high\"
        Case Else
            Debug.Print \"mid\"
    End Select
Next", "mid\nlow\nlow\nmid\nmid\nhigh\nhigh"),
    ("exit_do_inner", "", "Dim i As Long, j As Long, hits As Long
For i = 1 To 3
    j = 0
    Do
        j = j + 1
        If j = 2 Then Exit Do
    Loop While j < 10
    hits = hits + j
Next
Debug.Print hits", " 6"),
    ("for_each_collection", "", "Dim c As New Collection
Dim v
Dim total As Long
c.Add 4
c.Add 5
For Each v In c
    total = total + v
Next
Debug.Print total", " 9"),
    ("split_join_roundtrip", "", "Dim parts
parts = Split(\"a,b,c\", \",\")
Debug.Print UBound(parts)
Debug.Print Join(parts, \"|\")
Debug.Print parts(1)", " 2\na|b|c\nb"),
];
