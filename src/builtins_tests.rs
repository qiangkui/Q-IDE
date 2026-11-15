//! `Command()`、脚本目录与运行选项的测试。

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::interpreter::RunOptions;

fn run_with_args(source: &str, args: &str) -> Result<Vec<String>, String> {
    let interrupt = AtomicBool::new(false);
    let mut lines = Vec::new();
    let options = RunOptions {
        command_line: args.to_string(),
        ..RunOptions::default()
    };
    crate::interpreter::run_with(
        source,
        |line| lines.push(line.to_string()),
        &interrupt,
        options,
    )
    .map(|()| lines)
    .map_err(|error| error.to_string())
}

/// 建一个本次测试专用的临时目录（并行测试互不影响）。
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gpui-first-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir
}

/// 以 `dir` 作为脚本所在目录运行脚本。
fn run_in_script_dir(dir: &Path, source: &str) -> Result<Vec<String>, String> {
    let interrupt = AtomicBool::new(false);
    let mut lines = Vec::new();
    let options = RunOptions {
        script_path: Some(dir.join("demo.vb")),
        ..RunOptions::default()
    };
    crate::interpreter::run_with(
        source,
        |line| lines.push(line.to_string()),
        &interrupt,
        options,
    )
    .map(|()| lines)
    .map_err(|error| error.to_string())
}

#[test]
fn command_returns_the_run_arguments() {
    let lines = run_with_args(
        "Debug.Print Command()\nDebug.Print Len(Command())\n",
        "input.txt /v",
    )
    .expect("脚本应当跑完");
    assert_eq!(lines, ["input.txt /v", "12"]);
}

#[test]
fn command_is_empty_by_default() {
    let lines = run_with_args("Debug.Print Len(Command())\n", "").expect("脚本应当跑完");
    assert_eq!(lines, ["0"]);
}

#[test]
fn command_is_a_known_builtin() {
    assert!(crate::interpreter::builtins::is_builtin("command"));
}

#[test]
fn relative_paths_resolve_against_the_script_directory() {
    let dir = temp_dir("script-path");
    std::fs::write(dir.join("data.txt"), "hello").expect("写测试文件");

    let source = "\
Dim content As String
Open \"data.txt\" For Input As #1
Line Input #1, content
Close #1
Debug.Print content
Open \"out.txt\" For Output As #2
Print #2, \"written\"
Close #2
Debug.Print FileLen(\"data.txt\")
Debug.Print App.Path
Debug.Print Dir(\"*.txt\")
";
    let lines = run_in_script_dir(&dir, source).expect("脚本应当跑完");

    assert_eq!(lines[0], "hello", "Open 应当按脚本目录找到 data.txt");
    assert_eq!(lines[1], "5", "FileLen 也按脚本目录解析");
    assert_eq!(
        PathBuf::from(&lines[2]),
        dir,
        "App.Path 应当返回脚本所在目录"
    );
    assert_eq!(lines[3], "data.txt", "Dir 应当列出脚本目录里的文件");
    assert_eq!(
        std::fs::read_to_string(dir.join("out.txt")).expect("out.txt 应写在脚本目录"),
        "written\n"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn chdir_changes_only_the_scripts_current_directory() {
    let dir = temp_dir("script-chdir");
    let process_dir = std::env::current_dir().expect("读取进程目录");

    let source = "\
MkDir \"sub\"
ChDir \"sub\"
Debug.Print CurDir()
Open \"inner.txt\" For Output As #1
Print #1, \"x\"
Close #1
";
    let lines = run_in_script_dir(&dir, source).expect("脚本应当跑完");

    assert_eq!(
        PathBuf::from(&lines[0]),
        dir.join("sub"),
        "CurDir 应当跟着 ChDir 走"
    );
    assert!(
        dir.join("sub").join("inner.txt").is_file(),
        "ChDir 之后的相对路径应当写在子目录里"
    );
    assert_eq!(
        std::env::current_dir().expect("读取进程目录"),
        process_dir,
        "脚本的 ChDir 不能改动编辑器进程的当前目录"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn demo_script_with_relative_paths_runs_end_to_end() {
    let dir = temp_dir("demo-script");
    std::fs::write(dir.join("data.txt"), "hello").expect("写测试文件");
    let source = "\
Option Explicit

Dim content As String
Dim name As String

Open \"data.txt\" For Input
Line Input #1, content
Close
Debug.Print \"读到: \" & content

Debug.Print \"App.Path = \" & App.Path
Debug.Print \"CurDir   = \" & CurDir()

Open \"output.txt\" For Output
Print #1, \"hello from script\"
Close
Debug.Print \"FileLen(output.txt) = \" & FileLen(\"output.txt\")

name = Dir(\"*.txt\")
Do While name <> \"\"
    Debug.Print \"文件: \" & name
    name = Dir()
Loop

On Error Resume Next
MkDir \"sub\"
On Error GoTo 0
ChDir \"sub\"
Debug.Print \"ChDir 后 CurDir = \" & CurDir()
Open \"inner.txt\" For Output
Print #1, \"x\"
Close
";
    let lines = run_in_script_dir(&dir, source).expect("示例脚本应当跑完");

    assert_eq!(lines[0], "读到: hello");
    assert_eq!(lines[1], format!("App.Path = {}", dir.display()));
    assert_eq!(lines[2], format!("CurDir   = {}", dir.display()));
    assert_eq!(lines[3], "FileLen(output.txt) = 18");
    assert_eq!(lines[4], "文件: data.txt");
    assert_eq!(lines[5], "文件: output.txt");
    assert_eq!(
        lines[6],
        format!("ChDir 后 CurDir = {}", dir.join("sub").display())
    );
    assert!(dir.join("sub").join("inner.txt").is_file());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_without_a_file_number_uses_the_next_free_number() {
    let dir = temp_dir("open-implicit");
    std::fs::write(dir.join("data.txt"), "hello\n").expect("写测试文件");
    std::fs::write(dir.join("other.txt"), "ab\n").expect("写测试文件");

    let source = "\
Dim content As String
Open \"data.txt\" For Input
Open \"other.txt\" For Input
Debug.Print LOF(1)
Debug.Print LOF(2)
Line Input #1, content
Close
Debug.Print content
";
    let lines = run_in_script_dir(&dir, source).expect("省略 As #n 时应当自动分配文件号");

    assert_eq!(lines[0], "6", "#1 是第一个文件");
    assert_eq!(lines[1], "3", "#2 是第二个文件");
    assert_eq!(lines[2], "hello");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 本机 VB6 实测（`Dim` 后立刻打印）：`VarType` / `TypeName` 反映变量的**声明类型**——
/// Integer 2/"Integer"、Byte 17/"Byte"、Single 4/"Single"、Long 3、Double 5、Currency 6、Date 7、
/// Boolean 11、String 8、`Integer` 数组 8194/"Integer()"、未赋值的 Variant 0/"Empty"。
#[test]
fn vartype_and_typename_follow_the_declared_type() {
    let source = r#"
Dim i As Integer, b As Byte, s As Single, l As Long, d As Double
Dim c As Currency, dt As Date, flag As Boolean, text As String
Dim arr(2) As Integer, v As Variant
Debug.Print VarType(i) & " " & TypeName(i)
Debug.Print VarType(b) & " " & TypeName(b)
Debug.Print VarType(s) & " " & TypeName(s)
Debug.Print VarType(l) & " " & TypeName(l)
Debug.Print VarType(d) & " " & TypeName(d)
Debug.Print VarType(c) & " " & TypeName(c)
Debug.Print VarType(dt) & " " & TypeName(dt)
Debug.Print VarType(flag) & " " & TypeName(flag)
Debug.Print VarType(text) & " " & TypeName(text)
Debug.Print VarType(arr) & " " & TypeName(arr)
Debug.Print VarType(v) & " " & TypeName(v)
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "2 Integer",
            "17 Byte",
            "4 Single",
            "3 Long",
            "5 Double",
            "6 Currency",
            "7 Date",
            "11 Boolean",
            "8 String",
            "8194 Integer()",
            "0 Empty"
        ]
    );
}

/// 期望值来自本机 VB6 真编译（`CompilationType=-1`）运行后写出的文件。
#[test]
fn conversions_and_choices_match_vb6_golden() {
    let source = r#"
Dim a(3) As Integer
Debug.Print CByte(1.5)
Debug.Print CByte(2.5)
Debug.Print CByte(255)
Debug.Print CByte(True)
Debug.Print CByte(False)
Debug.Print VarType(1.5)
Debug.Print VarType("s")
Debug.Print VarType(Empty)
Debug.Print VarType(Null)
Debug.Print VarType(True)
Debug.Print VarType(a)
Debug.Print CVar(3)
Debug.Print Choose(2, "a", "b")
Debug.Print IsNull(Choose(0, "a", "b"))
Debug.Print IsNull(Choose(3, "a", "b"))
Debug.Print Switch(False, 1, True, 2)
Debug.Print IsNull(Switch(False, 1, False, 2))
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "2", "2", "255", "255", "0", "5", "8", "0", "1", "11", "8194", "3", "b", "True",
            "True", "2", "True"
        ]
    );
}

#[test]
fn strconv_ismissing_and_numeric_suffixes_match_vb6_golden() {
    let source = r#"
Debug.Print StrConv("aBc", 1)
Debug.Print StrConv("aBc", 2)
Debug.Print StrConv("hello world", 3)
Debug.Print StrConv("abc", 4)
Debug.Print Opt()
Debug.Print Opt(1)
Debug.Print TypeName(123&)
Debug.Print TypeName(1.5#)
Debug.Print TypeName(3.25@)

Function Opt(Optional x)
    If IsMissing(x) Then
        Opt = "missing"
    Else
        Opt = "given:" & x
    End If
End Function

Function IsNull(value)
    IsNull = TypeName(value) = "Null"
End Function
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "ABC",
            "abc",
            "Hello World",
            "ａｂｃ",
            "missing",
            "given:1",
            "Long",
            "Double",
            "Currency"
        ]
    );
}

#[test]
fn strconv_rejects_unknown_conversion() {
    let source = "On Error Resume Next\nDebug.Print StrConv(\"abc\", 99)\nDebug.Print Err.Number";
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        ["5"],
        "无效的转换方式应当报错误号 5"
    );
}

/// 文案来自本机 VB6 p-code 实测的 `Error$(n)`（运行时表，不是 IDE 编译对话框的 RT_STRING）。
#[test]
fn error_text_matches_vb6_official_messages() {
    let expected = [
        (3, "不执行 GoSub 而返回"),
        (5, "无效的过程调用或参数"),
        (6, "溢出"),
        (7, "超出内存"),
        (9, "下标越界"),
        (10, "该数组被固定或临时锁定"),
        (11, "除数为零"),
        (13, "类型不匹配"),
        (14, "超出字符串空间"),
        (16, "表达式太复杂"),
        (17, "不能执行请求的操作"),
        (18, "发生用户中断"),
        (20, "无错误恢复"),
        (28, "堆栈空间溢出"),
        (35, "Sub 或 Function 未定义"),
        (47, "太多 DLL 应用程序客户"),
        (48, "加载 DLL 错误"),
        (49, "DLL 调用约定错误"),
        (51, "内部错误"),
        (52, "错误文件名或数"),
        (53, "文件未找到"),
        (54, "错误文件模式"),
        (55, "文件已经打开"),
        (57, "设备 I/O 错误"),
        (58, "文件已经存在"),
        (59, "错误记录长度"),
        (61, "磁盘满"),
        (62, "输入超出文件尾"),
        (63, "错误记录号"),
        (67, "太多文件"),
        (68, "设备不可用"),
        (70, "拒绝的权限"),
        (71, "磁盘未准备好"),
        (74, "不能用不同驱动器重命名"),
        (75, "路径/文件访问错误"),
        (76, "未找到路径"),
        (91, "未设置对象变量或 With block 变量"),
        (92, "For 循环未初始化"),
        (93, "无效格式字符串"),
        (94, "使用 Null 无效"),
        (95, "定义的应用程序或对象错误"),
        (
            96,
            "无法吸收对象的事件，因为引发的事件已经达到它的对象接收器所支持的最大数目。",
        ),
        (97, "在不是定义类实例的对象中不能调用友元函数"),
        (
            98,
            "属性或方法调用不能包括对私有对象的引用，无论是作为参数或者作为返回值。",
        ),
        (424, "需要对象"),
        (429, "ActiveX 部件不能创建对象"),
        (438, "对象不支持该属性或方法"),
        (449, "参数不是可选的"),
        (450, "错误的参数数或无效属性分配"),
        (452, "无效的序号"),
        (453, "未找到指定的 DLL 函数"),
        (458, "变量使用了 Visual Basic 不支持的 Automation 类型"),
        (0, ""),
        (9999, "定义的应用程序或对象错误"),
    ];
    for (number, text) in expected {
        assert_eq!(
            crate::interpreter::builtins::error_text(number),
            text,
            "错误号 {number} 的文案与 VB6 实测不一致"
        );
    }
}

/// `Format` 的日期命名格式：输出固定、不读系统区域设置（取本机 VB6 中文区域的写法）。
#[test]
fn format_named_date_formats_are_fixed() {
    let source = r#"
Dim d As Date
d = #2024-02-29 13:05:06#
Debug.Print Format(d, "General Date")
Debug.Print Format(#2024-02-29#, "General Date")
Debug.Print Format(#13:05:06#, "General Date")
Debug.Print Format(CDate(0), "General Date")
Debug.Print Format(d, "Long Date")
Debug.Print Format(d, "Medium Date")
Debug.Print Format(d, "Short Date")
Debug.Print Format(d, "Long Time")
Debug.Print Format(d, "Medium Time")
Debug.Print Format(d, "short time")
Debug.Print Format("2024-02-29", "long date")
Debug.Print Format(45000, "Short Date")
Debug.Print Format(#2024-02-29 00:30:00#, "Medium Time")
Debug.Print Format(#2024-02-29 12:00:00#, "Medium Time")
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "2024/2/29 13:05:06",
            "2024/2/29",
            "13:05:06",
            "0:00:00",
            "2024年2月29日",
            "24-02-29",
            "2024/2/29",
            "13:05:06",
            "01:05 下午",
            "13:05",
            "2024年2月29日",
            "2023/3/15",
            "12:30 上午",
            "12:00 下午",
        ]
    );
}

/// 字符串查找/替换与 `Format` 的比较方式参数、命名格式、进位规则（期望值全部来自本机 VB6）。
#[test]
fn string_search_and_format_options_match_vb6() {
    let source = r#"
Debug.Print InStr(1, "abc", "B")
Debug.Print InStr(1, "abc", "B", 1)
Debug.Print InStr(3, "aBcaBc", "b", 1)
Debug.Print InStrRev("aBcaBc", "b", -1, 1)
Debug.Print InStrRev("abcabc", "b", 3)
Debug.Print Replace("aXbX", "x", "-", 1, -1, 1)
Debug.Print Replace("aXbX", "X", "-", 3)
Debug.Print Replace("aXbX", "X", "-", 1, 1)
Debug.Print Replace("aaa", "a", "b", 2)
Debug.Print Replace("abc", "", "-")
Debug.Print UBound(Split(""))
Debug.Print Format(1234.5678, "Standard")
Debug.Print Format(1234.5678, "Fixed")
Debug.Print Format(0.5, "Percent")
Debug.Print Format(1234, "Scientific")
Debug.Print Format(True, "Yes/No")
Debug.Print Format(False, "Yes/No")
Debug.Print Format(True, "On/Off")
Debug.Print Format(0.5, "0")
Debug.Print Format(2.5, "0")
Debug.Print Format(1234.5678, "0.00E+00")
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "0", "2", "5", "5", "2", "a-b-", "b-", "a-bX", "bb", "abc", "-1", "1,234.57",
            "1234.57", "50.00%", "1.23E+03", "Yes", "No", "On", "1", "3", "1.23E+03"
        ]
    );
}

/// `Format` 的取舍在最短十进制表示上做、`Val` 的指数要真有数字、`LenB` 是字节数
/// （期望值全部来自本机 VB6 实测）。
#[test]
fn format_rounding_val_and_lenb_match_vb6() {
    let source = r#"
Debug.Print Format(1.005, "0.00")
Debug.Print Format(2.675, "0.00")
Debug.Print Format(1.00005, "0.0000")
Debug.Print Format(1.23456, "0.00000")
Debug.Print Format(1.0049999, "0.00")
Debug.Print Val("1e")
Debug.Print Val("1E+")
Debug.Print Val("1.2e3x")
Debug.Print Val("1.2.3")
Debug.Print LenB("abc")
Debug.Print LenB("中文")
Debug.Print LenB("")
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "1.01", "2.68", "1.0001", "1.23456", "1.00", "1", "1", "1200", "1.2", "6", "4", "0"
        ]
    );
}

/// `DatePart` 的各个间隔与 `firstdayofweek`/`firstweekofyear` 参数（期望值来自本机 VB6）。
#[test]
fn date_part_matches_vb6() {
    let source = r#"
Dim d As Date
d = #2024-02-29 13:05:06#
Debug.Print DatePart("yyyy", d)
Debug.Print DatePart("q", d)
Debug.Print DatePart("m", d)
Debug.Print DatePart("y", d)
Debug.Print DatePart("d", d)
Debug.Print DatePart("w", d)
Debug.Print DatePart("w", d, 3)
Debug.Print DatePart("ww", d)
Debug.Print DatePart("ww", #2024-12-31#, 1, 1)
Debug.Print DatePart("ww", #2024-01-01#, 1, 2)
Debug.Print DatePart("ww", #2024-12-29#, 1, 2)
Debug.Print DatePart("ww", #2024-01-01#, 1, 3)
Debug.Print DatePart("ww", #2024-12-30#, 1, 3)
Debug.Print DatePart("h", d)
Debug.Print DatePart("n", d)
Debug.Print DatePart("s", d)
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "2024", "1", "2", "60", "29", "5", "3", "9", "53", "1", "1", "53", "52", "13", "5", "6"
        ]
    );
    let error = run_with_args("Debug.Print DatePart(\"x\", #2024-01-01#)\n", "").unwrap_err();
    assert!(error.contains("间隔"), "{error}");
}

/// `Date` 值转文本按 VB6 的 `General Date`：`2024/2/29`、时间 `13:05:06`、序列 0 只写时间。
#[test]
fn date_display_matches_vb6_general_date() {
    let source = r#"
Debug.Print #2024-02-29#
Debug.Print #2024-02-29 13:05:06#
Debug.Print CDate(0)
Debug.Print CStr(CDate(45000.5))
Debug.Print DateValue("2024-01-05")
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "2024/2/29",
            "2024/2/29 13:05:06",
            "0:00:00",
            "2023/3/15 12:00:00",
            "2024/1/5"
        ]
    );
}

/// `String` 的字符代码、`Val` 的进制/`D` 指数、`IsNumeric`/`IsDate` 的写法、
/// 中文星期/月名（期望值全部来自本机 VB6 实测）。
#[test]
fn string_val_and_name_builtins_match_vb6() {
    let source = r#"
Debug.Print String(3, 65)
Debug.Print String(3, "x")
Debug.Print Val("&HFFFF")
Debug.Print Val("&HFFFF&")
Debug.Print Val("&H80000000")
Debug.Print Val("1D5")
Debug.Print Val("1.5D3")
Debug.Print IsNumeric("&H10")
Debug.Print IsNumeric("&O17")
Debug.Print IsNumeric("12a")
Debug.Print IsDate("24-2-29")
Debug.Print IsDate("2024-2-30")
Debug.Print CDate("24-2-29")
Debug.Print DateValue("3/4/2024")
Debug.Print DateValue("31/12/2024")
Debug.Print WeekdayName(1)
Debug.Print WeekdayName(1, True)
Debug.Print WeekdayName(1, False, 1)
Debug.Print MonthName(12)
Debug.Print MonthName(12, True)
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "AAA",
            "xxx",
            "-1",
            "65535",
            "-2147483648",
            "100000",
            "1500",
            "True",
            "True",
            "False",
            "True",
            "False",
            "2024/2/29",
            "2024/3/4",
            "2024/12/31",
            "星期一",
            "周一",
            "星期日",
            "十二月",
            "12月",
        ]
    );
}

/// `Format` 的日期/文本占位符与 `DateAdd`/`DateDiff` 的间隔（期望值全部来自本机 VB6 实测）。
#[test]
fn format_placeholders_and_date_intervals_match_vb6() {
    let source = r##"
Dim d As Date
d = #2024-02-29 13:05:06#
Debug.Print Format(d, "ddd d")
Debug.Print Format(d, "dddd, mmmm dd, yyyy")
Debug.Print Format(d, "aaa")
Debug.Print Format(d, "aaaa")
Debug.Print Format(d, "y")
Debug.Print Format(d, "w")
Debug.Print Format(d, "ww")
Debug.Print Format(d, "q")
Debug.Print Format(d, "h:m:s")
Debug.Print Format(d, "mm:ss")
Debug.Print Format(d, "hh:mm")
Debug.Print Format(d, "hh am/pm")
Debug.Print Format(d, "h A/P")
Debug.Print Format(#2024-02-29 00:05:06#, "hh:mm AM/PM")
Debug.Print Format(d, "yyyy""年""m""月""d""日""")
Debug.Print Format(1, "@@@")
Debug.Print Format("abc", "@@@@@")
Debug.Print Format("ab", "&")
Debug.Print Format(1234567, "0,")
Debug.Print Format(1234, "#,##0,")
Debug.Print DateAdd("w", 3, #2024-02-27#)
Debug.Print DateDiff("w", #2024-01-01#, #2024-01-15#)
Debug.Print DateDiff("q", #2024-03-31#, #2024-04-01#)
Debug.Print DateDiff("y", #2024-01-01#, #2024-03-01#)
"##;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "Thu 29",
            "Thursday, February 29, 2024",
            "周四",
            "星期四",
            "60",
            "5",
            "9",
            "1",
            "13:5:6",
            "02:06",
            "13:05",
            "01 pm",
            "1 P",
            "12:05 AM",
            "2024年2月29日",
            "  1",
            "  abc",
            "ab",
            "1235",
            "1",
            "2024/3/1",
            "2",
            "1",
            "60",
        ]
    );
}

/// `Variant` 数组 / `Collection` 的元素内容类型和 `Byte` 运算溢出（期望值全部来自本机 VB6 实测）。
#[test]
fn narrow_container_types_and_byte_overflow_match_vb6() {
    let source = r#"
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
Dim b As Byte
b = 200
On Error Resume Next
Dim total
total = b + b
Debug.Print Err.Number
Dim dic As New Dictionary
dic("k") = CByte(1)
Debug.Print VarType(dic("k"))
dic.Add "k2", CSng(1.5)
Debug.Print VarType(dic("k2"))
Dim saved
saved = dic("k2")
Debug.Print VarType(saved)
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        ["17", "2", "17", "17", "6", "17", "4", "4"]
    );
}

/// `GetAllSettings`：0 基二维 `String` 数组、写入顺序、空节返回 `Empty`（期望值来自本机 VB6 实测）。
#[test]
fn getallsettings_returns_a_two_dimensional_string_array_like_vb6() {
    let source = r#"
DeleteSetting "qide-tests-all"
SaveSetting "qide-tests-all", "sec", "Key1", "v1"
SaveSetting "qide-tests-all", "sec", "Key2", "v2"
Dim all As Variant
all = GetAllSettings("qide-tests-all", "sec")
Debug.Print IsArray(all)
Debug.Print UBound(all, 1)
Debug.Print UBound(all, 2)
Debug.Print LBound(all, 1)
Debug.Print all(0, 0) & "|" & all(0, 1)
Debug.Print all(1, 0) & "|" & all(1, 1)
Debug.Print VarType(all)
Debug.Print TypeName(all)
DeleteSetting "qide-tests-one"
SaveSetting "qide-tests-one", "sec", "Only", "solo"
Dim one1 As Variant
one1 = GetAllSettings("qide-tests-one", "sec")
Debug.Print UBound(one1, 1) & "/" & one1(0, 0) & "|" & one1(0, 1)
DeleteSetting "qide-tests-one"
Dim none1 As Variant
none1 = GetAllSettings("qide-tests-all", "NoSuchSection")
Debug.Print IsArray(none1)
Debug.Print IsEmpty(none1)
Debug.Print VarType(none1)
On Error Resume Next
Dim result As Variant
result = Empty
result = UBound(none1, 1)
Debug.Print Err.Number
Err.Clear
result = IsArray(GetAllSettings("", ""))
Debug.Print Err.Number
Err.Clear
result = IsArray(GetAllSettings("NoSuchApp", "NoSuchSection"))
Debug.Print Err.Number
Debug.Print result
DeleteSetting "qide-tests-all"
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "True",
            "1",
            "1",
            "0",
            "Key1|v1",
            "Key2|v2",
            "8200",
            "String()",
            "0/Only|solo",
            "False",
            "True",
            "0",
            "13",
            "5",
            "0",
            "False",
        ]
    );
}

/// 颜色、`Format*` 系列、`Spc`/`Tab`、设置读写与 `vb*` 常量（期望值全部来自本机 VB6 实测）。
#[test]
fn color_format_and_settings_builtins_match_vb6() {
    let source = r#"
Debug.Print RGB(1, 2, 3)
Debug.Print RGB(256, 0, 0)
Debug.Print QBColor(1)
Debug.Print QBColor(15)
Debug.Print FormatNumber(1234.5678)
Debug.Print FormatNumber(1234.5, 2, vbTrue, vbTrue)
Debug.Print FormatNumber(0.5, 2, vbFalse)
Debug.Print FormatCurrency(1234.5)
Debug.Print FormatCurrency(-1234.5, 2, vbTrue, vbTrue)
Debug.Print FormatPercent(0.5)
Debug.Print FormatPercent(0.5, 1)
Debug.Print FormatDateTime(#2024-02-29 13:05:06#)
Debug.Print FormatDateTime(#2024-02-29 13:05:06#, 1)
Debug.Print FormatDateTime(#2024-02-29 13:05:06#, 4)
Debug.Print "a"; Tab(10); "b"
Debug.Print "abc"; Tab(2); "z"
Debug.Print VarType(vbTrue)
Debug.Print Len(vbCrLf)
SaveSetting "qide-tests", "section", "key", "value"
Debug.Print GetSetting("qide-tests", "section", "key")
Debug.Print GetSetting("qide-tests", "section", "missing", "def")
DeleteSetting "qide-tests", "section", "key"
Debug.Print "[" & GetSetting("qide-tests", "section", "key") & "]"
On Error Resume Next
Dim total
total = RGB(-1, 2, 3)
Debug.Print Err.Number
total = QBColor(16)
Debug.Print Err.Number
total = FormatDateTime(#2024-01-01#, 5)
Debug.Print Err.Number
"#;
    assert_eq!(
        run_with_args(source, "").expect("脚本应当执行成功"),
        [
            "197121",
            "255",
            "8388608",
            "16777215",
            "1,234.57",
            "1,234.50",
            ".50",
            "￥1,234.50",
            "(￥1,234.50)",
            "50.00%",
            "50.0%",
            "2024/2/29 13:05:06",
            "2024年2月29日",
            "13:05",
            "a        b",
            "abc\n z",
            "3",
            "2",
            "value",
            "def",
            "[]",
            "5",
            "5",
            "5",
        ]
    );
}
/// `VarPtr` / `ObjPtr` / `LoadPicture`：本机 VB6 实测的语义（`VarPtr`/`ObjPtr` 都是 `Long`，`ObjPtr(Nothing)` 是 0，
/// `LoadPicture` 找不到文件报 53、内容不是图片报 481、空参数返回空的 `Picture` 对象）。
#[test]
fn pointer_and_load_picture_builtins_match_vb6() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let stamp = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(format!("qide-picture-{stamp}"));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let bmp = dir.join("px.bmp");
    // 1x1 24 位 BMP（VB6 的 LoadPicture 认这种文件）。
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&58u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40i32.to_le_bytes());
    bytes.extend_from_slice(&1i32.to_le_bytes());
    bytes.extend_from_slice(&1i32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    bytes.extend_from_slice(&[0, 0, 0xFF, 0]);
    std::fs::write(&bmp, &bytes).expect("写 BMP");
    let text = dir.join("note.txt");
    std::fs::write(&text, "not an image").expect("写文本");
    let missing = dir.join("nope.bmp");

    let source = format!(
        r#"
Dim picture As Object
On Error Resume Next
Set picture = LoadPicture("{bmp}")
Debug.Print Err.Number
Debug.Print TypeName(picture)
Debug.Print VarType(picture)
Debug.Print picture Is Nothing
Debug.Print IsObject(picture)
Debug.Print ObjPtr(picture) <> 0
Set picture = Nothing
Debug.Print picture Is Nothing
Set picture = LoadPicture("")
Debug.Print TypeName(picture)
Debug.Print picture Is Nothing
Set picture = LoadPicture("{missing}")
Debug.Print Err.Number
Err.Clear
Set picture = LoadPicture("{text}")
Debug.Print Err.Number
Dim counter As Integer
Dim other As Long
Debug.Print VarPtr(counter) = VarPtr(counter)
Debug.Print VarPtr(counter) = VarPtr(other)
Debug.Print VarType(VarPtr(counter))
Dim items As New Collection
Dim items2 As New Collection
Debug.Print ObjPtr(items) = ObjPtr(items2)
Debug.Print ObjPtr(items) = ObjPtr(items)
Dim nothing1 As Collection
Debug.Print ObjPtr(nothing1)
Debug.Print VarType(ObjPtr(nothing1))
"#,
        bmp = bmp.display(),
        missing = missing.display(),
        text = text.display(),
    );
    let result = run_with_args(&source, "");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        result.expect("脚本应当执行成功"),
        [
            "0", "Picture", "9", "False", "True", "True", "True", "Picture", "False", "53", "481",
            "True", "False", "3", "False", "True", "0", "3",
        ]
    );
}
