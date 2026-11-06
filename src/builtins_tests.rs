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
