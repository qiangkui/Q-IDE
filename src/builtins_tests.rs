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

/// 文案来自 VB6 IDE 的 RT_STRING（Err.Number 对应的官方中文说明，共 44 条）。
#[test]
fn error_text_matches_vb6_official_messages() {
    let expected = [
        (3, "无 GoSub 返回"),
        (5, "无效的过程调用或参数"),
        (6, "溢出"),
        (7, "内存溢出"),
        (9, "下标越界"),
        (10, "该数组被固定或暂时锁定"),
        (11, "除数为零"),
        (13, "类型不匹配"),
        (14, "字符串空间溢出"),
        (16, "表达式太复杂"),
        (17, "不能执行所需的操作"),
        (18, "出现用户中断"),
        (20, "无错误恢复"),
        (28, "堆栈空间溢出"),
        (35, "子程序或函数未定义"),
        (47, "DLL 应用程序客户太多"),
        (48, "加载 DLL 错误"),
        (49, "DLL 调用约定错误"),
        (51, "内部错误"),
        (52, "错误的文件名或号码"),
        (53, "文件未找到"),
        (54, "文件模式错误"),
        (55, "文件已打开"),
        (57, "设备 I/O 错误"),
        (58, "文件已存在"),
        (59, "记录长度错误"),
        (61, "磁盘已满"),
        (62, "输入超出文件尾"),
        (63, "记录号错误"),
        (67, "文件太多"),
        (68, "设备不可用"),
        (70, "拒绝的权限"),
        (71, "磁盘未准备好"),
        (74, "不能以不同的驱动器改名"),
        (75, "路径/文件访问错误"),
        (76, "路径未找到"),
        (91, "对象变量或 With 块变量未设置"),
        (92, "For 循环未初始化"),
        (93, "无效的模式串"),
        (94, "无效使用 Null"),
        (95, "应用程序定义或对象定义错误"),
        (
            96,
            "由于对象已经激活了事件接受器支持的最多个事件，因此不能激活对象的事件",
        ),
        (97, "不能调用对象的友元函数，该对象不是所定义类的一个实例。"),
        (
            98,
            "一个属性或方法调用不能包含对一个私有对象的引用，无论是作为参数还是返回值",
        ),
    ];
    for (number, text) in expected {
        assert_eq!(
            crate::interpreter::builtins::error_text(number),
            text,
            "错误号 {number} 的文案与 VB6 官方不一致"
        );
    }
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
