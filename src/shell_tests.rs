//! 需要真实系统调用的内置函数（目前是 `Shell`）的测试。

#![cfg(windows)]

use std::sync::atomic::AtomicBool;

fn run(source: &str) -> Result<Vec<String>, String> {
    let interrupt = AtomicBool::new(false);
    let mut lines = Vec::new();
    let result = crate::interpreter::run(source, |line| lines.push(line.to_string()), &interrupt);
    result.map(|()| lines).map_err(|error| error.to_string())
}

#[test]
fn shell_launches_a_program_and_returns_a_task_id() {
    // `cmd.exe /c exit` 立刻结束；窗口样式 0 = 隐藏，测试时不会闪窗口。
    let lines = run("Dim id As Long\nid = Shell(\"cmd.exe /c exit\", 0)\nDebug.Print id > 0\n")
        .expect("Shell 应当成功");
    assert_eq!(lines, ["True"], "应当拿到正数的进程 id");
}

#[test]
fn shell_reports_a_missing_program() {
    let error = run("Shell \"no-such-program-xyz.exe\"\n").expect_err("启动不存在的程序应当报错");
    assert!(error.contains("无法启动"), "{error}");
}

#[test]
fn shell_is_a_known_builtin() {
    assert!(crate::interpreter::builtins::is_builtin("shell"));
}
