//! 由 `tools/vb6_golden.py` 生成，请勿手工修改。
//!
//! 期望值来自本机 VB6 真编译（`CompilationType=-1`，IDE 调试用的 p-code 形态）的实际输出。

/// 表达式 → `CStr(表达式)` 的结果。
#[rustfmt::skip]
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
    ("VarType(1&)", "3"),
    ("TypeName(1&)", "Long"),
    ("VarType(1.5!)", "4"),
    ("TypeName(1.5!)", "Single"),
    ("VarType(1.5#)", "5"),
    ("VarType(&HFFFF&)", "3"),
    ("TypeName(&HFFFF&)", "Long"),
    ("VarType(&HFFFF)", "2"),
    ("TypeName(&HFFFF)", "Integer"),
    ("Hex(1&)", "1"),
    ("Oct(-1&)", "37777777777"),
    ("Hex(-32768)", "FFFF8000"),
    ("Hex(-200)", "FF38"),
    ("Hex(CInt(-1))", "FFFF"),
    ("VarType(CByte(1) + CByte(1))", "17"),
    ("VarType(1& + CSng(1))", "5"),
    ("VarType(1 + CSng(1))", "4"),
    ("VarType(CSng(1) + CDbl(1))", "5"),
    ("VarType(CCur(1) + CDbl(1))", "6"),
    ("VarType(CDate(1) + 1)", "7"),
    ("VarType(Not CByte(1))", "17"),
    ("VarType(Not 1)", "2"),
    ("VarType(1 < 2)", "11"),
    ("VarType(1 \\ 2)", "2"),
    ("VarType(1 Mod 2)", "2"),
    ("VarType(2 ^ 3)", "5"),
    ("TypeName(1 + 1)", "Integer"),
    ("TypeName(Null = Null)", "Null"),
    ("&HFFFF", "-1"),
    ("&H8000", "-32768"),
    ("&HE000", "-8192"),
    ("&H7FFF", "32767"),
    ("&H10000", "65536"),
    ("&O177777", "-1"),
    ("&H0", "0"),
    ("&HFFFFFFFF", "-1"),
    ("&H80000000", "-2147483648"),
    ("&HFFFFFFFF&", "-1"),
    ("VarType(&HFFFFFFFF)", "3"),
    ("TypeName(&H10000)", "Long"),
    ("Hex(&HFFFF)", "FFFF"),
    ("CLng(&HFFFF)", "-1"),
    ("VarType(65535 And &HFFFF)", "3"),
    ("Round(2.5)", "2"),
    ("Round(3.5)", "4"),
    ("Round(-2.5)", "-2"),
    ("Round(2.675, 2)", "2.68"),
    ("Round(1.005, 2)", "1"),
    ("Int(-1.5)", "-2"),
    ("Fix(-1.5)", "-1"),
    ("Sgn(-0.0)", "0"),
    ("Abs(-32768)", "32768"),
    ("Abs(-1.5)", "1.5"),
    ("Left(\"abcdef\", 0)", ""),
    ("Right(\"abcdef\", 0)", ""),
    ("Mid(\"abcdef\", 3)", "cdef"),
    ("Mid(\"abcdef\", 10)", ""),
    ("Len(\"\")", "0"),
    ("InStr(\"\", \"\")", "0"),
    ("InStr(\"abc\", \"\")", "1"),
    ("InStrRev(\"abcabc\", \"a\")", "4"),
    ("Replace(\"aaa\", \"a\", \"b\", 1, 2)", "bba"),
    ("Replace(\"\", \"\", \"x\")", ""),
    ("StrComp(\"a\", \"A\")", "1"),
    ("StrComp(\"a\", \"A\", 1)", "0"),
    ("String(3, \"xy\")", "xxx"),
    ("String(0, \"x\")", ""),
    ("Space(3) & \"|\"", "   |"),
    ("Val(\"  12  \")", "12"),
    ("Val(\"1.5e3\")", "1500"),
    ("Val(\"&H1F\")", "31"),
    ("Val(\"1,5\")", "1"),
    ("Val(\"abc\")", "0"),
    ("Chr(65) & Asc(\"A\")", "A65"),
    ("ChrW(20013)", "中"),
    ("AscW(\"中\")", "20013"),
    ("Format(1234.5678, \"0.00\")", "1234.57"),
    ("Format(0.5, \"0.0%\")", "50.0%"),
    ("Format(1234.5678, \"#,##0.00\")", "1,234.57"),
    ("Format(-0.004, \"0.00\")", "0.00"),
    ("Format(1234567, \"Scientific\")", "1.23E+06"),
    ("Format(1.5, \"Currency\")", "￥1.50"),
    ("Format(True, \"Yes/No\")", "Yes"),
    ("Format(1, \"On/Off\")", "On"),
    ("IsNumeric(\"\")", "False"),
    ("IsNumeric(\"1.5\")", "True"),
    ("IsDate(\"2024-02-30\")", "False"),
    ("IsArray(Array(1))", "True"),
    ("UBound(Split(\"a,b\", \",\"))", "1"),
    ("Join(Array(\"a\", \"b\"), \"-\")", "a-b"),
    ("Year(#2024-02-29#)", "2024"),
    ("Weekday(#2024-02-29#)", "5"),
    ("DateDiff(\"d\", #2024-01-01#, #2024-03-01#)", "60"),
    ("DateAdd(\"m\", 1, #2024-01-31#)", "2024/2/29"),
    ("Hour(#2024-02-29 13:05:06#)", "13"),
    ("2 ^ 0.5", "1.4142135623731"),
    ("-0.0", "0"),
    ("Int(1E+20)", "1E+20"),
    ("CLng(2.5)", "2"),
    ("CLng(3.5)", "4"),
    ("CInt(2.5)", "2"),
    ("CInt(-2.5)", "-2"),
    ("CByte(1.5)", "2"),
    ("CByte(2.5)", "2"),
    ("IIf(True, 1, 2)", "1"),
    ("Switch(False, 1, True, 2)", "2"),
    ("StrReverse(\"abc\")", "cba"),
    ("InStr(2, \"abcabc\", \"a\")", "4"),
    ("InStrRev(\"abcabc\", \"a\", 4)", "4"),
    ("Mid(\"abcdef\", 2, 3)", "bcd"),
    ("Mid(\"abcdef\", 2, 0)", ""),
    ("Left$(\"abc\", 10)", "abc"),
    ("Right$(\"abc\", 10)", "abc"),
    ("UCase(\"aBc\")", "ABC"),
    ("LCase(\"AbC\")", "abc"),
    ("Trim(\"  x  \")", "x"),
    ("LTrim(\"  x\")", "x"),
    ("RTrim(\"x  \")", "x"),
    ("Format(-1234.5678, \"#,##0.00\")", "-1,234.57"),
    ("Format(0, \"0.00\")", "0.00"),
    ("Format(0.5, \"0\")", "1"),
    ("Format(2.5, \"0\")", "3"),
    ("Format(1.005, \"0.00\")", "1.01"),
    ("Format(1234.5678, \"0\")", "1235"),
    ("DatePart(\"yyyy\", #2024-02-29#)", "2024"),
    ("DatePart(\"q\", #2024-05-15#)", "2"),
    ("DatePart(\"ww\", #2024-01-15#)", "3"),
    ("Weekday(#2024-03-03#)", "1"),
    ("Weekday(#2024-03-03#, 2)", "7"),
    ("DateSerial(2024, 2, 30)", "2024/3/1"),
    ("TimeSerial(25, 0, 0)", "1899/12/31 1:00:00"),
    ("CDate(\"2024-02-29\")", "2024/2/29"),
    ("CDate(45000.5)", "2023/3/15 12:00:00"),
    ("Format(#2024-02-29#, \"yyyy-mm-dd\")", "2024-02-29"),
    ("Year(0)", "1899"),
    ("Month(0)", "12"),
    ("Day(1)", "31"),
    ("Timer > 0", "True"),
    ("LenB(\"abc\")", "6"),
    ("Chr(0) & \"|\"", " |"),
    ("StrComp(\"abc\", \"abd\")", "-1"),
    ("StrComp(\"ABC\", \"abc\", 1)", "0"),
    ("\"abc\" Like \"a*\"", "True"),
    ("\"abc\" Like \"a?c\"", "True"),
    ("\"abc\" Like \"[a-c]*\"", "True"),
    ("\"a1\" Like \"a#\"", "True"),
    ("Hex(CLng(-2147483648))", "80000000"),
    ("Val(\"&HFF\")", "255"),
    ("Val(\"&O17\")", "15"),
    ("Val(\"1e\")", "1"),
    ("CInt(\"5\")", "5"),
    ("CLng(\"5.9\")", "6"),
    ("CCur(\"1.5\")", "1.5"),
    ("CDbl(\"1.5e3\")", "1500"),
    ("CBool(0)", "False"),
    ("CBool(-1)", "True"),
    ("CStr(CDate(45000.5))", "2023/3/15 12:00:00"),
    ("CStr(#2024-02-29#)", "2024/2/29"),
    ("CStr(CDate(#1899-12-31 01:00:00#))", "1899/12/31 1:00:00"),
    ("Format(#2024-02-29#, \"Short Date\")", "2024/2/29"),
    ("Format(#2024-02-29#, \"Medium Date\")", "24-02-29"),
    ("Format(#2024-02-29 13:05:06#, \"General Date\")", "2024/2/29 13:05:06"),
    ("Format(#2024-02-29 13:05:06#, \"Long Time\")", "13:05:06"),
    ("Format(#2024-02-29 13:05:06#, \"Medium Time\")", "01:05 下午"),
    ("Format(#2024-02-29 13:05:06#, \"Short Time\")", "13:05"),
    ("Format(#2024-02-29#, \"Long Date\")", "2024年2月29日"),
    ("Format(1.23456, \"0.00000\")", "1.23456"),
    ("Format(1.00005, \"0.0000\")", "1.0001"),
    ("Format(2.675, \"0.00\")", "2.68"),
    ("DatePart(\"y\", #2024-02-29#)", "60"),
    ("DatePart(\"w\", #2024-02-29#)", "5"),
    ("DatePart(\"w\", #2024-02-29#, 3)", "3"),
    ("DatePart(\"ww\", #2024-01-01#)", "1"),
    ("DatePart(\"ww\", #2024-12-31#)", "53"),
    ("DatePart(\"ww\", #2025-01-01#)", "1"),
    ("DatePart(\"ww\", #2024-01-01#, 1, 2)", "1"),
    ("DatePart(\"ww\", #2024-12-28#, 1, 2)", "52"),
    ("DatePart(\"ww\", #2024-12-29#, 1, 2)", "1"),
    ("DatePart(\"ww\", #2024-12-30#, 1, 2)", "1"),
    ("DatePart(\"ww\", #2024-01-01#, 1, 3)", "53"),
    ("DatePart(\"ww\", #2024-12-30#, 1, 3)", "52"),
    ("DatePart(\"ww\", #2024-01-01#, 2, 1)", "1"),
    ("DatePart(\"ww\", #2024-01-01#, 3, 1)", "1"),
    ("DatePart(\"h\", #2024-02-29 13:05:06#)", "13"),
    ("DatePart(\"n\", #2024-02-29 13:05:06#)", "5"),
    ("DatePart(\"s\", #2024-02-29 13:05:06#)", "6"),
    ("DatePart(\"q\", #2024-12-31#)", "4"),
    ("DatePart(\"d\", #2024-02-29 23:00:00#)", "29"),
    ("CStr(DateSerial(2024, 2, 30))", "2024/3/1"),
    ("Year(DateSerial(2024, 13, 1))", "2025"),
    ("Month(DateSerial(2024, 0, 1))", "12"),
    ("Day(DateSerial(2024, 1, 0))", "31"),
    ("CStr(DateSerial(2024, 3, -1))", "2024/2/28"),
    ("Year(DateSerial(0, 1, 1))", "2000"),
    ("Year(DateSerial(100, 1, 1))", "100"),
    ("LenB(\"中文\")", "4"),
    ("LenB(\"\")", "0"),
    ("Val(\"1e5\")", "100000"),
    ("Val(\"1E+\")", "1"),
    ("Val(\"1.2e3x\")", "1200"),
    ("Val(\".5\")", "0.5"),
    ("Val(\"1.2.3\")", "1.2"),
    ("Mid(\"abc\", 2, 0)", ""),
    ("Mid(\"abc\", 5)", ""),
    ("Left(\"abc\", 0)", ""),
    ("Right(\"abc\", 5)", "abc"),
    ("String(3, \"x\")", "xxx"),
    ("String(3, 65)", "AAA"),
    ("String(0, \"x\")", ""),
    ("Space(3) & \"|\"", "   |"),
    ("UCase(\"中文abc\")", "中文ABC"),
    ("LCase(\"中文ABC\")", "中文abc"),
    ("AscW(\"中\")", "20013"),
    ("ChrW(20013)", "中"),
    ("CInt(2.5)", "2"),
    ("CInt(3.5)", "4"),
    ("CInt(-2.5)", "-2"),
    ("CLng(2.5)", "2"),
    ("CByte(2.5)", "2"),
    ("CCur(2.345)", "2.345"),
    ("CSng(1.0000001)", "1"),
    ("CStr(-0.5)", "-0.5"),
    ("Str(-0.5)", "-.5"),
    ("CStr(1E+20)", "1E+20"),
    ("CStr(0.0000001)", "0.0000001"),
    ("CStr(1 / 3)", "0.333333333333333"),
    ("CStr(1.5E-14)", "0.000000000000015"),
    ("CStr(1.23456789012345E-11)", "1.23456789012345E-11"),
    ("CStr(123456789.123456789)", "123456789.123457"),
    ("CStr(1234567890123.4567)", "1234567890123.46"),
    ("CStr(999900000000000)", "999900000000000"),
    ("Int(-1.5)", "-2"),
    ("Fix(-1.5)", "-1"),
    ("Round(2.5)", "2"),
    ("Round(1.5)", "2"),
    ("Round(-2.5)", "-2"),
    ("Round(2.345, 2)", "2.35"),
    ("Sgn(-0.5)", "-1"),
    ("Sqr(2)", "1.4142135623731"),
    ("2 ^ 0.5", "1.4142135623731"),
    ("-7 \\ 2", "-3"),
    ("-7 Mod 2", "-1"),
    ("7 Mod -2", "1"),
    ("Hex(-1.5)", "FFFFFFFE"),
    ("Hex(0)", "0"),
    ("Oct(-1)", "177777"),
    ("IsNumeric(\"&H10\")", "True"),
    ("IsNumeric(\"1e5\")", "True"),
    ("IsNumeric(\"\")", "False"),
    ("IsDate(\"24-2-29\")", "True"),
    ("IsDate(\"2024-2-30\")", "False"),
    ("\"a\" Like \"[a-z]\"", "True"),
    ("\"5\" Like \"#\"", "True"),
    ("\"abc\" Like \"*b*\"", "True"),
    ("\"abc\" Like \"a?c\"", "True"),
    ("\"ABC\" Like \"a*\"", "False"),
    ("\"a]\" Like \"[]]\"", "False"),
    ("Join(Split(\"a b c\", \" \"), \"-\")", "a-b-c"),
    ("UBound(Split(\"a,b\", \",\", 2))", "1"),
    ("WeekdayName(1)", "星期一"),
    ("WeekdayName(1, True)", "周一"),
    ("WeekdayName(1, False, 1)", "星期日"),
    ("WeekdayName(2, True)", "周二"),
    ("WeekdayName(7)", "星期日"),
    ("MonthName(1)", "一月"),
    ("MonthName(11)", "十一月"),
    ("MonthName(12, True)", "12月"),
    ("MonthName(6, True)", "6月"),
    ("Format(0.5, \"0.0%\")", "50.0%"),
    ("Format(1234.5, \"##0\")", "1235"),
    ("Format(5, \"0000\")", "0005"),
    ("Format(-5, \"0.00;(0.00)\")", "(5.00)"),
    ("Format(1234.5678, \"0.00E-00\")", "1.23E03"),
    ("Format(\"abc\", \">\")", "ABC"),
    ("Val(\"&HFFFFFFFF\")", "-1"),
    ("Val(\"&HFFFF\")", "-1"),
    ("Val(\"&H7FFFFFFF\")", "2147483647"),
    ("Val(\"&H80000000\")", "-2147483648"),
    ("Val(\"&HFFFF&\")", "65535"),
    ("Val(\"1D5\")", "100000"),
    ("Val(\"1.5D3\")", "1500"),
    ("CBool(\"False\")", "False"),
    ("CDate(\"2024/2/29\") = CDate(\"2024-02-29\")", "True"),
    ("CDate(\"24-2-29\")", "2024/2/29"),
    ("DateValue(\"2/29/2024\")", "2024/2/29"),
    ("DateValue(\"3/4/2024\")", "2024/3/4"),
    ("DateValue(\"31/12/2024\")", "2024/12/31"),
    ("DateValue(\"1/2/3\")", "2001/2/3"),
    ("Year(#1899-12-30#)", "1899"),
    ("DateDiff(\"m\", #2024-01-31#, #2024-03-01#)", "2"),
    ("DateDiff(\"ww\", #2024-01-01#, #2024-02-01#)", "4"),
    ("DateAdd(\"q\", 1, #2024-11-30#)", "2025/2/28"),
    ("DateSerial(2024, 2, 29) = #2024-02-29#", "True"),
    ("Abs(-32768)", "32768"),
    ("Int(CCur(1.5))", "1"),
    ("Fix(CCur(-1.5))", "-1"),
    ("Hex(CDbl(1.5))", "2"),
    ("Hex(CSng(-1.5))", "FFFFFFFE"),
    ("Hex(CCur(-1))", "FFFFFFFF"),
    ("Format(CCur(-1.5), \"0.00\")", "-1.50"),
    ("Str(CCur(1.5))", " 1.5"),
    ("CStr(CCur(1.5))", "1.5"),
    ("VarType(Abs(1))", "2"),
    ("VarType(Abs(1&))", "3"),
    ("VarType(Int(1.5))", "5"),
    ("VarType(Sqr(4))", "5"),
    ("VarType(2 ^ 2)", "5"),
    ("Val(\"%\")", "0"),
    ("Val(\"  -  1\")", "-1"),
    ("Format(1.5, \"0.0%\")", "150.0%"),
    ("Format(-1.5, \"0.0%\")", "-150.0%"),
    ("Format(0, \"0.0%\")", "0.0%"),
    ("Choose(1, \"a\", \"b\")", "a"),
    ("Switch(False, 1, True, 2)", "2"),
    ("IIf(True, 1, 2)", "1"),
    ("UBound(Array(1, 2, 3))", "2"),
    ("LBound(Array(1, 2, 3))", "0"),
    ("InStr(1, \"中文abc\", \"ABC\", 1)", "3"),
    ("StrComp(\"a\", \"A\", 0)", "1"),
    ("StrComp(\"a\", \"A\", 1)", "0"),
    ("Replace(\"aaa\", \"a\", \"b\", 1, 0)", "aaa"),
    ("Replace(\"aaa\", \"a\", \"b\", 1, -1, 1)", "bbb"),
    ("Mid(\"abcdef\", 2)", "bcdef"),
    ("Trim(\"  a  \") & \"|\"", "a|"),
    ("Len(Trim(\"  a  \"))", "1"),
    ("Format(1234.5678, \"#,##0.000\")", "1,234.568"),
    ("Format(0.5, \"0%\")", "50%"),
    ("Format(5, \"0.00;;zero\")", "5.00"),
    ("Format(0, \"0.00;neg;zero\")", "zero"),
    ("Hex(255.9)", "100"),
    ("Oct(8.9)", "11"),
    ("CStr(0.1 + 0.2)", "0.3"),
    ("CDbl(\"1e5\")", "100000"),
    ("Format(#2024-02-29#, \"aaaa\")", "星期四"),
    ("Format(#2024-02-29#, \"aaa\")", "周四"),
    ("Format(#2024-02-29#, \"d\")", "29"),
    ("Format(#2024-02-29#, \"dd\")", "29"),
    ("Format(#2024-02-29#, \"m\")", "2"),
    ("Format(#2024-02-29#, \"mm\")", "02"),
    ("Format(#2024-02-29#, \"yy\")", "24"),
    ("Format(#2024-02-29#, \"y\")", "60"),
    ("Format(#2024-02-29#, \"mmm d, yyyy\")", "Feb 29, 2024"),
    ("Format(#2024-02-29#, \"dddd, mmmm dd, yyyy\")", "Thursday, February 29, 2024"),
    ("Format(#2024-02-29#, \"ddd d\")", "Thu 29"),
    ("Format(#2024-02-29 13:05:06#, \"hh:mm\")", "13:05"),
    ("Format(#2024-02-29 13:05:06#, \"nn\")", "05"),
    ("Format(#2024-02-29 13:05:06#, \"h AM/PM\")", "1 PM"),
    ("Format(#2024-02-29 13:05:06#, \"hh am/pm\")", "01 pm"),
    ("Format(#2024-02-29 13:05:06#, \"h A/P\")", "1 P"),
    ("Format(#2024-02-29 13:05:06#, \"h a/p\")", "1 p"),
    ("Format(#2024-02-29 00:05:06#, \"hh:mm AM/PM\")", "12:05 AM"),
    ("Format(#2024-02-29 12:05:06#, \"hh:mm AM/PM\")", "12:05 PM"),
    ("Format(#2024-02-29 13:05:06#, \"mm:ss\")", "02:06"),
    ("Format(#2024-02-29 13:05:06#, \"yyyy-mm-dd hh:mm:ss\")", "2024-02-29 13:05:06"),
    ("Format(#2024-02-29#, \"yyyy\"\"年\"\"m\"\"月\"\"d\"\"日\"\"\")", "2024年2月29日"),
    ("Format(#2024-02-29#, \"yyyy\\-mm\\-dd\")", "2024-02-29"),
    ("Format(#2024-02-29#, \"w\")", "5"),
    ("Format(#2024-02-29#, \"ww\")", "9"),
    ("Format(#2024-01-01#, \"ww\")", "1"),
    ("Format(#2024-12-31#, \"ww\")", "53"),
    ("Format(#2024-01-01#, \"q\")", "1"),
    ("Format(1, \"@@@\")", "  1"),
    ("Format(\"abc\", \"@@@@@\")", "  abc"),
    ("Format(\"ab\", \"&\")", "ab"),
    ("Format(1234.5, \"@@@@@@\")", "1234.5"),
    ("Format(1234567, \"0,\")", "1235"),
    ("Format(1234567, \"0,,\")", "1"),
    ("Format(123, \"abc\")", "ab1900/5/2"),
    ("Format(123, \"c\")", "1900/5/2"),
    ("Format(12345, \"000-00\")", "123-45"),
    ("Format(123, \"00:00\")", "01:23"),
    ("Format(123, \"0 0 0\")", "1 2 3"),
    ("Format(123, \"\\0\")", "0"),
    ("Format(123, \"0.0\"\"x\"\"\")", "123.0x"),
    ("Format(123, \"\\$0\")", "$123"),
    ("Format(123, \"000-000\")", "000-123"),
    ("Format(1234567, \"000,000,000\")", "001,234,567"),
    ("Format(1234567, \"0-000\")", "1234-567"),
    ("Format(12345, \"###-###\")", "12-345"),
    ("Format(12345678, \"000,000\")", "12,345,678"),
    ("Format(1234567, \"0,0\")", "1,234,567"),
    ("Format(1234, \"#,##,##0\")", "1,234"),
    ("Format(12345, \"0,000\")", "12,345"),
    ("Format(1234567, \"0,.0\")", "1234.6"),
    ("Format(1234, \"0,00.00\")", "1,234.00"),
    ("Format(1234, \"0.0,\")", "1234.0"),
    ("Format(1234567, \"0.0,,\")", "1234567.0"),
    ("Format(123, \"!0000\")", "0000"),
    ("Format(123, \"!###\")", "###"),
    ("Format(123, \"!\")", ""),
    ("Format(0.5, \"%0\")", "%1"),
    ("Format(5, \",\")", ","),
    ("Format(0.1234, \"0.##\")", "0.12"),
    ("Format(0, \"#.##\")", "."),
    ("Format(123, \".00\")", "123.00"),
    ("Format(0.5, \".00\")", ".50"),
    ("Format(\"abc\", \"&&&&\")", "abc"),
    ("Format(5, \"0.\")", "5."),
    ("Format(123, \"0.0\\+0\")", "123.0+0"),
    ("Format(0.5, \"0%\")", "50%"),
    ("Format(-1234, \"$#,##0.00\")", "-$1,234.00"),
    ("Format(12345, \"(###) ###-####\")", "() 1-2345"),
    ("Format(0, \"0;(0);zero\")", "zero"),
    ("Format(-5, \"0;(0)\")", "(5)"),
    ("Format(1.5, \"0.0#;0.0#\")", "1.5"),
    ("Format(123.456, \"#,##0.0#\")", "123.46"),
    ("Format(1e20, \"0\")", "100000000000000000000"),
    ("Atn(1)", "0.785398163397448"),
    ("Atn(0)", "0"),
    ("Atn(-1)", "-0.785398163397448"),
    ("Sin(1)", "0.841470984807897"),
    ("Cos(1)", "0.54030230586814"),
    ("Tan(1)", "1.5574077246549"),
    ("Sin(0)", "0"),
    ("Cos(0)", "1"),
    ("Tan(0)", "0"),
    ("Sin(90 * 3.14159265358979 / 180)", "1"),
    ("TypeName(Sin(1))", "Double"),
    ("VarType(Sin(1))", "5"),
    ("IsNull(Abs(Null))", "True"),
    ("IsNull(Int(Null))", "True"),
    ("IsNull(Fix(Null))", "True"),
    ("IsNull(Round(Null))", "True"),
    ("IsNull(Hex(Null))", "True"),
    ("IsNull(Oct(Null))", "True"),
    ("Format(1234, \"#,##0,\")", "1"),
    ("DateAdd(\"y\", 1, #2024-12-31#)", "2025/1/1"),
    ("DateAdd(\"w\", 3, #2024-02-27#)", "2024/3/1"),
    ("DateAdd(\"ww\", 1, #2024-02-27#)", "2024/3/5"),
    ("DateDiff(\"y\", #2024-01-01#, #2024-03-01#)", "60"),
    ("DateDiff(\"w\", #2024-01-01#, #2024-01-15#)", "2"),
    ("DateDiff(\"w\", #2024-01-01#, #2024-01-16#)", "2"),
    ("DateDiff(\"w\", #2024-01-15#, #2024-01-01#)", "-2"),
    ("DateDiff(\"q\", #2024-01-15#, #2024-11-15#)", "3"),
    ("DateDiff(\"q\", #2024-03-31#, #2024-04-01#)", "1"),
    ("DateDiff(\"q\", #2024-04-01#, #2024-03-31#)", "-1"),
    ("DateDiff(\"q\", #2024-01-01#, #2025-01-01#)", "4"),
    ("SLN(10000, 1000, 5)", "1800"),
    ("SLN(1000, 0, 3)", "333.333333333333"),
    ("SLN(0, 0, 3)", "0"),
    ("SLN(10000, 1000, -1)", "-9000"),
    ("SYD(10000, 1000, 5, 1)", "3000"),
    ("SYD(10000, 1000, 5, 2)", "2400"),
    ("SYD(10000, 1000, 5, 3)", "1800"),
    ("SYD(10000, 1000, 5, 4)", "1200"),
    ("SYD(10000, 1000, 5, 5)", "600"),
    ("DDB(10000, 1000, 5, 1)", "4000"),
    ("DDB(10000, 1000, 5, 2)", "2400"),
    ("DDB(10000, 1000, 5, 3)", "1440"),
    ("DDB(10000, 1000, 5, 4)", "864"),
    ("DDB(10000, 1000, 5, 5)", "296"),
    ("DDB(10000, 1000, 5, 2, 1.5)", "2100"),
    ("DDB(10000, 1000, 5, 1, 2.5)", "5000"),
    ("DDB(0, 1000, 5, 1)", "0"),
    ("DDB(10000, 1000, 5, 1.5)", "3098.38667696593"),
    ("DDB(10000, 1000, 5, 4.5)", "669.251522224642"),
    ("DDB(10000, 1000, 5, 4.999)", "296.662199128142"),
    ("DDB(10000, 1000, 3, 3)", "111.111111111111"),
    ("DDB(10000, 0, 3, 3)", "740.740740740741"),
    ("DDB(10000, 2000, 3, 3)", "0"),
    ("DDB(500, 1000, 5, 1)", "-500"),
    ("DDB(10000, 1000, 1, 1)", "9000"),
    ("FV(0.05, 10, -100)", "1257.78925355488"),
    ("FV(0.05, 10, -1000)", "12577.8925355488"),
    ("FV(0.05, 10, -100, 0, 1)", "1320.67871623263"),
    ("FV(0.05, 10, -100, 1000)", "-371.105373222558"),
    ("FV(0, 10, -100)", "1000"),
    ("FV(0, 10, -100, 500)", "500"),
    ("FV(0.05 / 12, 120, -100)", "15528.2279445667"),
    ("FV(0.05, 10, 0, 1000)", "-1628.89462677744"),
    ("FV(0.05, 10, -100, 1000, 1)", "-308.215910544814"),
    ("FV(0.05, -10, -100)", "-772.173492918482"),
    ("PV(0.05, 10, -100)", "772.173492918482"),
    ("PV(0.05, 10, -100, 1000)", "158.260239377723"),
    ("PV(0.05, 10, -100, 0, 1)", "810.782167564406"),
    ("PV(0, 10, -100)", "1000"),
    ("PV(0.05, 0, -100)", "0"),
    ("PV(0.05, 10, 0, 1000)", "-613.913253540759"),
    ("PV(0.05, 10, -100, 1000, 1)", "196.868914023647"),
    ("Pmt(0.05, 10, 10000)", "-1295.04574965457"),
    ("Pmt(0.05, 10, 10000, 0, 1)", "-1233.37690443292"),
    ("Pmt(0, 10, 10000)", "-1000"),
    ("Pmt(0.05 / 12, 120, 10000)", "-106.065515239076"),
    ("Pmt(0.05, 10, 10000, 1000)", "-1374.55032462002"),
    ("Pmt(0.05, 10, 10000, 0, True)", "-1233.37690443292"),
    ("NPer(0.05, -100, 1000)", "14.2066990828905"),
    ("NPer(0.05, -100, 1000, 500)", "18.7802346532834"),
    ("NPer(0.05, -100, 1000, 0, 1)", "13.2532278981381"),
    ("NPer(0.05, -100, 1000, 500, 1)", "17.6306286290802"),
    ("NPer(0, -100, 1000)", "10"),
    ("NPer(0.05, -100, 0)", "0"),
    ("IPmt(0.05, 1, 10, 10000)", "-500"),
    ("IPmt(0.05, 10, 10, 10000)", "-61.668845221646"),
    ("IPmt(0.05, 1, 10, 10000, 0, 1)", "0"),
    ("IPmt(0.05, 2, 10, 10000, 0, 1)", "-438.331154778354"),
    ("IPmt(0.05, 3, 10, 10000, 0, 1)", "-398.578867295626"),
    ("IPmt(0.05, 4, 10, 10000, 0, 1)", "-356.838965438761"),
    ("IPmt(0.05, 10, 10, 10000, 0, 1)", "-58.7322335444249"),
    ("IPmt(0, 1, 10, 10000)", "0"),
    ("PPmt(0.05, 1, 10, 10000)", "-795.045749654566"),
    ("PPmt(0.05, 5, 10, 10000)", "-966.383077741061"),
    ("PPmt(0.05, 10, 10, 10000)", "-1233.37690443292"),
    ("PPmt(0.05, 1, 10, 10000, 0, 1)", "-1233.37690443292"),
    ("PPmt(0.05, 2, 10, 10000, 0, 1)", "-795.045749654566"),
    ("PPmt(0.05, 3, 10, 10000, 0, 1)", "-834.798037137295"),
    ("PPmt(0.05, 10, 10, 10000, 0, 1)", "-1174.6446708885"),
    ("PPmt(0, 1, 10, 10000)", "-1000"),
    ("VarType(FV(0.05, 10, -100))", "5"),
    ("TypeName(SLN(10000, 1000, 5))", "Double"),
    ("FV(\"0.05\", 10, -100)", "1257.78925355488"),
    ("StrConv(\"abc def\", 1)", "ABC DEF"),
    ("StrConv(\"ABC Def\", 2)", "abc def"),
    ("StrConv(\"hello world\", 3)", "Hello World"),
    ("StrConv(\"hello,world/x-y\", 3)", "Hello,world/x-y"),
    ("StrConv(\"a1b2 3c\", 3)", "A1b2 3c"),
    ("StrConv(\"abc\", 4)", "ａｂｃ"),
    ("StrConv(\"abc\", 1 + 4)", "ＡＢＣ"),
    ("StrConv(\"\", 3)", ""),
    ("StrConv(\"你好abc\", 1)", "你好ABC"),
    ("StrConv(\"a\" & Chr(9) & \"b\", 3)", "A	B"),
    ("StrConv(\" abc\", 3)", " Abc"),
    ("StrConv(\"123ABC\", 3)", "123abc"),
    ("StrConv(\"a-b\", 3)", "A-b"),
    ("StrConv(\"你好 abc\", 3)", "你好 Abc"),
    ("IsObject(1)", "False"),
    ("IsObject(Empty)", "False"),
    ("IsObject(Null)", "False"),
    ("IsObject(\"abc\")", "False"),
    ("IsObject(New Collection)", "True"),
    ("IsEmpty(Empty)", "True"),
    ("IsNull(Null)", "True"),
    ("IsEmpty(1)", "False"),
    ("StrPtr(\"abc\") <> 0", "True"),
    ("StrPtr(\"\") = 0", "False"),
    ("TimeValue(\"13:30:00\")", "13:30:00"),
    ("TimeValue(\"1:2:3 PM\")", "13:02:03"),
    ("TimeValue(\"12:00:00 AM\")", "0:00:00"),
    ("TimeValue(\"12:00:00 PM\")", "12:00:00"),
    ("TimeValue(\"1 PM\")", "13:00:00"),
    ("TimeValue(\"9:05am\")", "9:05:00"),
    ("TimeValue(\"1:30\")", "1:30:00"),
    ("TimeValue(\"0:30\")", "0:30:00"),
    ("TimeValue(\"23:59:59\")", "23:59:59"),
    ("TimeValue(\"12:00 am\")", "0:00:00"),
    ("TimeValue(\"12:30AM\")", "0:30:00"),
    ("TimeValue(\"13:00 PM\")", "13:00:00"),
    ("TimeValue(\"00:00:01\")", "0:00:01"),
    ("TimeValue(#6:00:00 PM#)", "18:00:00"),
    ("TimeValue(CDate(0.75))", "18:00:00"),
    ("IsDate(\"1:30 PM\")", "True"),
    ("IsDate(\"abc\")", "False"),
    ("CDate(\"1:2:3 PM\")", "13:02:03"),
    ("CDate(\"13:30\")", "13:30:00"),
    ("VarType(Nothing)", "9"),
    ("TypeName(Nothing)", "Nothing"),
    ("IsObject(Nothing)", "True"),
    ("IsEmpty(Nothing)", "False"),
    ("IsNull(Nothing)", "False"),
    ("Nothing Is Nothing", "True"),
    ("ObjPtr(Nothing) = 0", "True"),
    ("VarType(Empty)", "0"),
    ("TypeName(Empty)", "Empty"),
    ("IsEmpty(Empty)", "True"),
    ("IsNull(Empty)", "False"),
];

/// 会失败的语句 → `Err.Number`。
#[rustfmt::skip]
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
    ("x = 32767 + 1", 6),
    ("x = 32767 * 2", 6),
    ("x = 2147483647 + 1", 6),
    ("x = 2147483647 * 2", 6),
    ("x = -2147483647 - 2", 6),
    ("x = Sqr(-1)", 5),
    ("x = Log(0)", 5),
    ("x = Exp(1000)", 6),
    ("x = CSng(1E+40)", 6),
    ("x = CDbl(\"1e400\")", 6),
    ("x = CInt(\"32768\")", 6),
    ("x = Asc(Null)", 94),
    ("x = Asc(\"\")", 5),
    ("x = Chr(Null)", 94),
    ("x = Val(Null)", 94),
    ("x = Split(Null, \",\")", 94),
    ("x = Join(Null, \",\")", 13),
    ("x = Replace(Null, \"a\", \"b\")", 94),
    ("x = Replace(\"abc\", Null, \"b\")", 94),
    ("x = StrReverse(Null)", 94),
    ("x = Len(Array(1))", 13),
    ("x = CStr(Null)", 94),
    ("x = CStr(Array(1))", 13),
    ("x = CInt(Null)", 94),
    ("x = CLng(Null)", 94),
    ("x = Space(Null)", 94),
    ("x = Space(-1)", 5),
    ("x = String(-1, \"x\")", 5),
    ("x = Mid(\"abcdef\", 0, 2)", 5),
    ("x = Mid(\"abc\", Null)", 94),
    ("x = Left(\"abc\", Null)", 94),
    ("x = Left(\"abc\", -1)", 5),
    ("x = InStr(0, \"abc\", \"b\")", 5),
    ("x = InStrRev(\"abc\", \"b\", 0)", 5),
    ("x = InStr(\"abc\", \"b\", Null)", 13),
    ("x = Sgn(Null)", 94),
    ("x = Sin(Null)", 94),
    ("x = Atn(Null)", 94),
    ("x = Val(Null)", 94),
    ("x = TimeValue(\"25:00\")", 13),
    ("x = TimeValue(\"12:34:56.5\")", 13),
    ("x = TimeValue(\"0:60\")", 13),
    ("x = TimeValue(\"\")", 13),
    ("x = TimeValue(\"1:2:3 XM\")", 13),
    ("x = TimeValue(\"abc\")", 13),
    ("x = CDate(\"abc\")", 13),
    ("x = TimeValue(0.75)", 13),
    ("x = TimeValue(1)", 13),
];

/// 浮点末位不稳的表达式 → (VB6 的 `CStr` 结果, 允许的相对误差)。
/// 比对规则：`|QIDE - VB6| <= 容差 * max(|VB6|, 1)`。
#[rustfmt::skip]
pub const APPROX_CASES: &[(&str, &str, f64)] = &[
    ("Tan(1.5707963267949)", "-286411217403671", 1e-05),
    ("Sin(3.14159265358979)", "3.23108510433268E-15", 1e-05),
    ("IPmt(0.05, 5, 10, 10000)", "-328.662671913506", 1e-09),
    ("Rate(11, -100, 1000)", "1.62313281745908E-02", 1e-09),
    ("Rate(10, -100, 1000, 100)", "-0.020569696650118", 1e-09),
    ("Rate(360, -1000, 200000)", "3.65592795236269E-03", 1e-09),
    ("Rate(20, 100, -1000)", "7.75468953001051E-02", 1e-09),
];

/// 脚本用例共用的声明与辅助过程（与 VB6 侧同一份）。
#[rustfmt::skip]
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

Function QideMissing(Optional ByVal a As Variant, Optional ByVal b As Variant) As String
    QideMissing = CStr(IsMissing(a)) & "/" & CStr(IsMissing(b))
End Function

Function QideMiddle(ByVal a As Long, Optional ByVal b As Variant, Optional ByVal c As Variant) As String
    QideMiddle = CStr(a) & "/" & CStr(IsMissing(b)) & "/" & CStr(IsMissing(c))
End Function
"#;

/// (用例名, 模块级选项（`Option Base 1` 等，空串表示没有）, 语句块, `Debug.Print` 的期望输出（行之间用 `\n` 分隔））。
#[rustfmt::skip]
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
    ("on_error_goto_minus_one", "", "On Error Resume Next
Dim x As Long
x = 1 / 0
Debug.Print Err.Number
On Error GoTo -1
Debug.Print Err.Number
On Error Resume Next
x = 1 / 0
Debug.Print Err.Number", " 11\n 0\n 11"),
    ("any_on_error_statement_clears_err", "", "On Error Resume Next
Dim x As Long
x = 1 / 0
Debug.Print Err.Number
Debug.Print Err.Number
On Error Resume Next
Debug.Print Err.Number
x = 1 / 0
Debug.Print Err.Number
On Error GoTo 0
Debug.Print Err.Number", " 11\n 11\n 0\n 11\n 0"),
    ("dictionary_for_each_keys", "", "Dim d As New Dictionary
d.Add \"a\", 10
d.Add \"b\", 20
Dim k As Variant
Dim names As String
For Each k In d
    names = names & k
Next
Debug.Print names
Dim total As Long
For Each k In d
    total = total + d(k)
Next
Debug.Print total", "ab\n 30"),
    ("binary_string_put_get", "", "Dim g As Integer
Dim s As String, t As String
Dim path As String
path = Environ(\"TEMP\") & \"\\qide_golden_bin.dat\"
s = \"AB\"
g = FreeFile
Open path For Binary As #g
Put #g, , s
Put #g, , \"CD\"
Put #g, , \"\"
Debug.Print LOF(g)
Close #g
Open path For Binary As #g
t = \"ABCD\"
Get #g, 1, t
Debug.Print t
t = \"\"
Get #g, , t
Debug.Print Len(t)
Close #g
Kill path", " 4\nABCD\n 0"),
    ("variant_content_types", "", "Dim i As Integer, v As Variant
Dim x
i = 1
v = i
Debug.Print VarType(v) & \" \" & TypeName(v)
v = CByte(1)
Debug.Print VarType(v)
v = True
Debug.Print VarType(v)
v = CCur(1)
Debug.Print VarType(v)
v = CDate(1)
Debug.Print VarType(v)
v = i
Debug.Print VarType(v + 1) & \" \" & VarType(v * 2)
v = \"x\"
Debug.Print VarType(v) & \" \" & TypeName(v)
v = Null
Debug.Print VarType(v)
v = Empty
Debug.Print VarType(v)
x = 5
Debug.Print VarType(x) & \" \" & TypeName(x)
x = 40000
Debug.Print VarType(x)
x = 1.5
Debug.Print VarType(x)", "2 Integer\n 17\n 11\n 6\n 7\n2 2\n8 String\n 1\n 0\n2 Integer\n 3\n 5"),
    ("for_each_variant_element_types", "", "Dim ai(1 To 2) As Integer
Dim al(1 To 2) As Long
Dim ad(1 To 2) As Double
Dim at(1 To 2) As String
Dim ab(1 To 2) As Byte
Dim v As Variant
ai(1) = 1
al(1) = 40000
ad(1) = 1.5
at(1) = \"x\"
ab(1) = 9
For Each v In ai
    Debug.Print VarType(v) & \" \" & TypeName(v)
Next
For Each v In al
    Debug.Print VarType(v)
Next
For Each v In ad
    Debug.Print VarType(v)
Next
For Each v In at
    Debug.Print VarType(v) & \" \" & TypeName(v)
Next
For Each v In ab
    Debug.Print VarType(v)
Next", "2 Integer\n2 Integer\n 3\n 3\n 5\n 5\n8 String\n8 String\n 17\n 17"),
    ("single_printing", "", "Dim s As Single
Debug.Print \"1=[\" & CSng(1.23456789) & \"]\"
Debug.Print \"2=[\" & CSng(0.1) & \"]\"
Debug.Print \"3=[\" & CSng(12345678) & \"]\"
Debug.Print \"4=[\" & CSng(0.00001) & \"]\"
Debug.Print \"5=[\" & CSng(1234567) & \"]\"
Debug.Print \"6=[\" & CSng(1.5) & \"]\"
Debug.Print \"7=[\" & CSng(0.000123456789) & \"]\"
Debug.Print \"8=[\" & CSng(123456.789) & \"]\"
s = 1 / 3
Debug.Print \"9=[\" & s & \"]\"
Debug.Print \"10=[\" & CSng(-1.23456789) & \"]\"
Debug.Print \"11=[\" & CSng(100000) & \"]\"
Debug.Print \"12=[\" & CSng(1E+20) & \"]\"
Debug.Print \"13=[\" & CStr(CSng(1.23456789)) & \"]\"
Debug.Print \"14=[\" & CStr(CSng(0.5)) & \"]\"
Debug.Print \"15=[\" & Str(CSng(0.5)) & \"]\"
Debug.Print \"16=[\" & Str(CSng(1.23456789)) & \"]\"
Debug.Print \"17=[\" & Str(CSng(-1.23456789)) & \"]\"
s = 0.1
Debug.Print \"18=[\" & CStr(s) & \"]\"
Debug.Print \"19=[\" & s & \"]\"
Debug.Print \"20=[\" & Str(s) & \"]\"
Debug.Print \"21=[\" & Format(CSng(1.23456789), \"0.000\") & \"]\"", "1=[1.234568]\n2=[0.1]\n3=[1.234568E+07]\n4=[0.00001]\n5=[1234567]\n6=[1.5]\n7=[1.234568E-04]\n8=[123456.8]\n9=[0.3333333]\n10=[-1.234568]\n11=[100000]\n12=[1E+20]\n13=[1.234568]\n14=[0.5]\n15=[ .5]\n16=[ 1.234568]\n17=[-1.234568]\n18=[0.1]\n19=[0.1]\n20=[ .1]\n21=[1.235]"),
    ("vartype_of_declared_variables", "", "Dim i As Integer, b As Byte, s As Single, l As Long, c As Currency
Dim dt As Date, flag As Boolean, text As String, v As Variant
Dim arr(2) As Integer
Debug.Print VarType(i) & \" \" & TypeName(i)
Debug.Print VarType(b) & \" \" & TypeName(b)
Debug.Print VarType(s) & \" \" & TypeName(s)
Debug.Print VarType(l) & \" \" & TypeName(l)
Debug.Print VarType(c) & \" \" & TypeName(c)
Debug.Print VarType(dt) & \" \" & TypeName(dt)
Debug.Print VarType(flag) & \" \" & TypeName(flag)
Debug.Print VarType(text) & \" \" & TypeName(text)
Debug.Print VarType(arr) & \" \" & TypeName(arr)
Debug.Print VarType(v) & \" \" & TypeName(v)", "2 Integer\n17 Byte\n4 Single\n3 Long\n6 Currency\n7 Date\n11 Boolean\n8 String\n8194 Integer()\n0 Empty"),
    ("error_statement_and_error_text", "", "On Error Resume Next
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
Debug.Print Len(Error(0))", " 5\n无效的过程调用或参数\n 40080\n定义的应用程序或对象错误\n无效的过程调用或参数\n无效的过程调用或参数\n定义的应用程序或对象错误\n 0"),
    ("narrow_value_types", "", "Dim s As Single
s = CSng(1.23456789)
Debug.Print s
Dim v As Variant
v = CSng(1.23456789)
Debug.Print v
Debug.Print VarType(v)
Debug.Print TypeName(v)
Dim b As Byte
b = 200
On Error Resume Next
Dim total
total = b + b
Debug.Print Err.Number
On Error GoTo 0
Dim items
items = Array(CByte(1), 2)
Dim item
For Each item In items
    Debug.Print VarType(item)
Next
Dim picked(1)
picked(0) = CByte(9)
Debug.Print VarType(picked(0))
Dim c As Collection
Set c = New Collection
c.Add CByte(7)
For Each item In c
    Debug.Print VarType(item)
Next
Dim dic As New Dictionary
dic(\"k\") = CByte(1)
Debug.Print VarType(dic(\"k\"))
dic.Add \"k2\", CSng(1.5)
Debug.Print VarType(dic(\"k2\"))
Dim v2
v2 = dic(\"k2\")
Debug.Print VarType(v2)", " 1.234568\n 1.234568\n 4\nSingle\n 6\n 17\n 2\n 17\n 17\n 17\n 4\n 4"),
    ("new_builtins", "", "On Error Resume Next
Err.Clear
Dim x As Long
x = RGB(-1, 2, 3)
Debug.Print Err.Number
x = QBColor(16)
Debug.Print Err.Number
On Error GoTo 0
Debug.Print RGB(1, 2, 3)
Debug.Print RGB(255, 255, 255)
Debug.Print RGB(256, 0, 0)
Debug.Print RGB(300, 258, 3)
Debug.Print QBColor(0)
Debug.Print QBColor(1)
Debug.Print QBColor(7)
Debug.Print QBColor(15)
Debug.Print FormatNumber(1234.5678)
Debug.Print FormatNumber(1234.5678, 1)
Debug.Print FormatNumber(-1234.5)
Debug.Print FormatNumber(1234.5, 0, vbTrue, vbTrue, vbFalse)
Debug.Print FormatNumber(1234.5, 2, vbTrue, vbTrue)
Debug.Print FormatNumber(0.5, 2, vbFalse)
Debug.Print FormatCurrency(1234.5)
Debug.Print FormatCurrency(1234.5, 0)
Debug.Print FormatCurrency(-1234.5, 2, vbTrue, vbTrue)
Debug.Print FormatPercent(0.5)
Debug.Print FormatPercent(0.5, 1)
Debug.Print FormatPercent(-0.5, 0)
Debug.Print FormatDateTime(#2024-02-29 13:05:06#)
Debug.Print FormatDateTime(#2024-02-29 13:05:06#, 1)
Debug.Print FormatDateTime(#2024-02-29 13:05:06#, 2)
Debug.Print FormatDateTime(#2024-02-29 13:05:06#, 3)
Debug.Print FormatDateTime(#2024-02-29 13:05:06#, 4)
Debug.Print \"a\"; Spc(3); \"b\"
Debug.Print \"a\"; Tab(10); \"b\"
Debug.Print Tab(3); \"x\"
Debug.Print Tab(1); \"y\"
Debug.Print \"abc\"; Tab(2); \"z\"
SaveSetting \"qide-golden\", \"section\", \"key\", \"value\"
Debug.Print GetSetting(\"qide-golden\", \"section\", \"key\")
Debug.Print GetSetting(\"qide-golden\", \"section\", \"missing\", \"def\")
Debug.Print \"[\" & GetSetting(\"qide-golden\", \"nosuch\", \"key\") & \"]\"
DeleteSetting \"qide-golden\", \"section\", \"key\"
Debug.Print \"[\" & GetSetting(\"qide-golden\", \"section\", \"key\") & \"]\"
Debug.Print VarType(vbTrue)
Debug.Print TypeName(vbTrue)
Debug.Print Len(vbCrLf)
Debug.Print vbCrLf = Chr(13) & Chr(10)
Debug.Print vbYesNo
Debug.Print vbOKOnly
Debug.Print vbObjectError", " 5\n 5\n 197121\n 16777215\n 255\n 262143\n 0\n 8388608\n 12632256\n 16777215\n1,234.57\n1,234.6\n-1,234.50\n1235\n1,234.50\n.50\n￥1,234.50\n￥1,235\n(￥1,234.50)\n50.00%\n50.0%\n-50%\n2024/2/29 13:05:06\n2024年2月29日\n2024/2/29\n13:05:06\n13:05\na   b\na        b\n  x\ny\nabc\n z\nvalue\ndef\n[]\n[]\n 3\nLong\n 2\nTrue\n 4\n 0\n-2147221504"),
    ("null_argument_rules", "", "' `Null` 走 `Variant` 形参的函数要原样返回 `Null`，非 `Variant` 形参的函数报 94（本机 VB6 实测）
Dim v As Variant
v = Trim(Null)
Debug.Print IsNull(v)
v = LTrim(Null)
Debug.Print IsNull(v)
v = RTrim(Null)
Debug.Print IsNull(v)
v = UCase(Null)
Debug.Print IsNull(v)
v = LCase(Null)
Debug.Print IsNull(v)
v = Left(Null, 2)
Debug.Print IsNull(v)
v = Right(Null, 2)
Debug.Print IsNull(v)
v = Mid(Null, 1, 2)
Debug.Print IsNull(v)
v = Len(Null)
Debug.Print IsNull(v)
v = LenB(Null)
Debug.Print IsNull(v)
v = String(3, Null)
Debug.Print IsNull(v)
v = InStr(Null, \"a\")
Debug.Print IsNull(v)
v = InStr(\"a\", Null)
Debug.Print IsNull(v)
' 不传播、返回空串或非 Null 的
v = Format(Null, \"0\")
Debug.Print \"[\" & v & \"]\", IsNull(v)
v = IsNull(Null)
Debug.Print v
v = IsNumeric(Null)
Debug.Print v", "True\nTrue\nTrue\nTrue\nTrue\nTrue\nTrue\nTrue\nTrue\nTrue\nTrue\nTrue\nTrue\n[]            False\nTrue\nFalse"),
    ("omitted_arguments", "", "Dim m1 As String, m2 As String, m3 As String
m1 = QideMissing(1, \"y\")
m2 = QideMissing(, \"x\")
m3 = QideMiddle(1, , 3)
Debug.Print m1
Debug.Print m2
Debug.Print m3", "False/False\nTrue/False\n1/True/False"),
    ("nothing_semantics", "", "Dim o As Object, v As Variant, e As Variant
Debug.Print VarType(o), TypeName(o), IsObject(o), IsEmpty(o), IsNull(o)
Debug.Print o Is Nothing
Set v = Nothing
Debug.Print VarType(v), TypeName(v), IsObject(v), IsEmpty(v), IsNull(v)
Debug.Print v Is Nothing
Debug.Print ObjPtr(o)
On Error Resume Next
Err.Clear
Debug.Print o.Foo
Debug.Print Err.Number
Err.Clear
If Nothing Then
End If
Debug.Print Err.Number
Err.Clear
Debug.Print e Is Nothing
Debug.Print Err.Number", " 9            Nothing       True          False         False\nTrue\n 9            Nothing       True          False         False\nTrue\n 0\n 91\n 91\n 424"),
    ("file_statements", "", "Dim h As Integer, s As String
On Error Resume Next
Kill \"golden_a.txt\"
Kill \"golden_b.txt\"
Kill \"golden_c.txt\"
Err.Clear
h = FreeFile
Open \"golden_a.txt\" For Output As #h
Print #h, \"one\"
Print #h, \"two\"
Close #h
Debug.Print FileLen(\"golden_a.txt\")
Debug.Print GetAttr(\"golden_a.txt\") And (vbReadOnly Or vbHidden Or vbSystem Or vbArchive)
h = FreeFile
Open \"golden_a.txt\" For Input As #h
Debug.Print EOF(h)
Line Input #h, s
Debug.Print s
Debug.Print EOF(h)
Line Input #h, s
Debug.Print s
Debug.Print EOF(h)
Close #h
FileCopy \"golden_a.txt\", \"golden_b.txt\"
Debug.Print FileLen(\"golden_b.txt\")
Debug.Print Dir(\"golden_b.txt\")
Name \"golden_b.txt\" As \"golden_c.txt\"
Debug.Print Dir(\"golden_c.txt\")
Debug.Print IsDate(FileDateTime(\"golden_a.txt\"))
Debug.Print Abs(DateDiff(\"s\", FileDateTime(\"golden_a.txt\"), Now)) < 120
Kill \"golden_a.txt\"
Kill \"golden_c.txt\"
Debug.Print Len(Dir(\"golden_a.txt\"))
Debug.Print Err.Number", " 10\n 32\nFalse\none\nFalse\ntwo\nTrue\n 10\ngolden_b.txt\ngolden_c.txt\nTrue\nTrue\n 0\n 0"),
    ("file_attr_input", "", "Dim h As Integer, s As String
On Error Resume Next
Kill \"ga.txt\"
Kill \"ga.dat\"
Err.Clear
h = FreeFile
Open \"ga.txt\" For Output As #h
Print #h, \"abcdef\"
Close #h
h = FreeFile
Open \"ga.txt\" For Append As #h
Debug.Print FileAttr(h, 1)
Close #h
h = FreeFile
Open \"ga.txt\" For Input As #h
Debug.Print FileAttr(h, 1)
Debug.Print \"[\" & Input(3, #h) & \"]\"
Debug.Print \"[\" & Input(1, #h) & \"]\"
Err.Clear
s = Input(100, #h)
Debug.Print Len(s)
Debug.Print Err.Number
Close #h
h = FreeFile
Open \"ga.dat\" For Binary As #h
Debug.Print FileAttr(h, 1)
Err.Clear
s = Input(1, #h)
Debug.Print Len(s)
Debug.Print Err.Number
Close #h
h = FreeFile
Open \"ga.dat\" For Random As #h
Debug.Print FileAttr(h, 1)
Close #h
Err.Clear
Debug.Print FileAttr(99, 1)
Debug.Print Err.Number
Err.Clear
Debug.Print FileAttr(h, 2)
Debug.Print Err.Number
Err.Clear
h = FreeFile
Open \"ga.dat\" For Output As #h
s = Input(1, #h)
Debug.Print Err.Number
Close #h
Kill \"ga.txt\"
Kill \"ga.dat\"", " 8\n 1\n[abc]\n[d]\n 0\n 62\n 32\n 0\n 0\n 4\n 52\n 52\n 54"),
];
