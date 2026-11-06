//! 调试控制器（断点、暂停/继续、单步、停止）的测试。
//!
//! 与控制台无关的纯逻辑：解释器线程停在断点上、界面轮询快照、再继续执行。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use crate::interpreter::RunOptions;
use crate::interpreter::debug::{DebugControl, DebugValue, STOPPED_MESSAGE};

/// 等调试器进入目标状态；超时直接失败，避免测试挂死。
fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("调试器没有在预期时间内进入目标状态");
}

/// 暂停期间求值一条表达式，等解释器线程把结果写回来。
fn evaluate(control: &DebugControl, expression: &str) -> Result<DebugValue, String> {
    let id = control
        .request_eval(expression)
        .expect("暂停时应当能发出求值请求");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(outcome) = control.take_eval_result()
            && outcome.id == id
        {
            return outcome.result;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("求值 `{expression}` 没有在预期时间内返回");
}

#[test]
fn breakpoint_pauses_and_resume_runs_to_the_end() {
    let control = Arc::new(DebugControl::new(HashSet::from([3])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = crate::interpreter::run_debug(
                "Debug.Print 1\nDebug.Print 2\nDebug.Print 3\n",
                |line| lines.push(line.to_string()),
                &interrupt,
                RunOptions::default(),
                control,
            );
            (result, lines)
        })
    };

    wait_until(|| control.is_paused());
    let snapshot = control.snapshot().expect("暂停时应当有位置快照");
    assert_eq!(snapshot.line, 3, "应当停在第 3 行的断点上");
    assert_eq!(snapshot.procedure, "模块");

    control.resume();
    let (result, lines) = runner.join().expect("解释器线程不应 panic");
    result.expect("继续后脚本应当跑完");
    assert_eq!(lines, ["1", "2", "3"]);
}

#[test]
fn snapshot_lists_current_variables() {
    let control = Arc::new(DebugControl::new(HashSet::from([3])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = crate::interpreter::run_debug(
                "Dim total As Long\ntotal = 42\nDebug.Print total\n",
                |line| lines.push(line.to_string()),
                &interrupt,
                RunOptions::default(),
                control,
            );
            (result, lines)
        })
    };

    wait_until(|| control.is_paused());
    let snapshot = control.snapshot().expect("暂停时应当有位置快照");
    assert!(
        snapshot.variables.iter().any(|variable| {
            variable.scope == "模块"
                && variable.name == "total"
                && variable.ty == "Long"
                && variable.value == "42"
        }),
        "快照里应当有模块变量 total As Long = 42：{:?}",
        snapshot.variables
    );

    control.resume();
    runner.join().expect("解释器线程不应 panic").0.expect("脚本应当跑完");
}

#[test]
fn step_stops_at_every_line() {
    let control = Arc::new(DebugControl::new(HashSet::new()));
    control.pause();

    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = crate::interpreter::run_debug(
                "Debug.Print 1\nDebug.Print 2\n",
                |line| lines.push(line.to_string()),
                &interrupt,
                RunOptions::default(),
                control,
            );
            (result, lines)
        })
    };

    wait_until(|| control.is_paused());
    assert_eq!(control.snapshot().expect("快照").line, 1);

    control.step();
    wait_until(|| control.is_paused() && control.snapshot().is_some_and(|s| s.line == 2));

    control.step();
    let (result, lines) = runner.join().expect("解释器线程不应 panic");
    result.expect("单步走完脚本应当成功");
    assert_eq!(lines, ["1", "2"]);
}

#[test]
fn pause_epoch_advances_on_every_stop() {
    let control = Arc::new(DebugControl::new(HashSet::from([2, 3])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                "Debug.Print 1\nDebug.Print 2\nDebug.Print 3\n",
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    wait_until(|| control.is_paused());
    assert_eq!(control.pause_epoch(), 1, "第一次暂停");
    control.resume();

    wait_until(|| control.is_paused() && control.pause_epoch() == 2);
    control.resume();

    runner
        .join()
        .expect("解释器线程不应 panic")
        .expect("脚本应当跑完");
}

#[test]
fn debug_preview_truncates_long_values() {
    let value = crate::interpreter::value::Value::Str("x".repeat(500));
    let preview = value.debug_preview(120);
    assert_eq!(preview.chars().count(), 120);
    assert!(preview.ends_with('…'), "超长值应当被截断：{preview}");
}

#[test]
fn stop_wakes_a_paused_script() {
    let control = Arc::new(DebugControl::new(HashSet::from([2])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = crate::interpreter::run_debug(
                "Debug.Print 1\nDebug.Print 2\n",
                |line| lines.push(line.to_string()),
                &interrupt,
                RunOptions::default(),
                control,
            );
            (result, lines)
        })
    };

    wait_until(|| control.is_paused());
    control.stop();

    let (result, lines) = runner.join().expect("解释器线程不应 panic");
    let error = result.expect_err("停止后应当返回错误");
    assert!(
        error.to_string().contains(STOPPED_MESSAGE),
        "错误信息应当说明脚本已停止：{error}"
    );
    assert_eq!(lines, ["1"], "断点之前的那一行应当已经执行");
}

#[test]
fn snapshot_records_the_call_stack() {
    let source = "\
Sub Outer()
    Debug.Print \"outer\"
    Inner
End Sub

Sub Inner()
    Debug.Print \"inner\"
End Sub

Outer
";
    // 第 7 行是 `Debug.Print "inner"`：断点应当在最内层过程里命中。
    let control = Arc::new(DebugControl::new(HashSet::from([7])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                source,
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    wait_until(|| control.is_paused());
    let snapshot = control.snapshot().expect("暂停时应当有位置快照");
    let names: Vec<&str> = snapshot
        .call_stack
        .iter()
        .map(|frame| frame.procedure.as_str())
        .collect();
    assert_eq!(names, ["模块", "outer", "inner"], "调用栈应当从外到内");

    let lines: Vec<usize> = snapshot.call_stack.iter().map(|frame| frame.line).collect();
    assert_eq!(lines, [10, 3, 7], "外层指向调用处，当前层指向断点行");

    control.resume();
    runner
        .join()
        .expect("解释器线程不应 panic")
        .expect("脚本应当跑完");
}

#[test]
fn watch_evaluates_expressions_while_paused() {
    let control = Arc::new(DebugControl::new(HashSet::from([3])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                "Dim total As Long\ntotal = 40\nDebug.Print total\n",
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    wait_until(|| control.is_paused());
    let number = evaluate(&control, "total + 2").expect("求值应当成功");
    assert_eq!(number.value, "42");
    assert_eq!(number.ty, "Long");

    let text = evaluate(&control, "\"a\" & total").expect("求值应当成功");
    assert_eq!(text.value, "a40");
    assert_eq!(text.ty, "String");

    control.resume();
    runner
        .join()
        .expect("解释器线程不应 panic")
        .expect("脚本应当跑完");
}

#[test]
fn watch_reports_evaluation_errors() {
    let control = Arc::new(DebugControl::new(HashSet::from([2])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                "Dim total As Long\ntotal = 1\n",
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    wait_until(|| control.is_paused());
    let error = evaluate(&control, "missing + 1").expect_err("未定义的变量应当报错");
    assert!(
        error.contains("未定义的变量") && error.contains("missing"),
        "错误信息应当说明是哪个变量：{error}"
    );

    control.resume();
    runner
        .join()
        .expect("解释器线程不应 panic")
        .expect("脚本应当跑完");
}

#[test]
fn watch_can_call_script_functions_without_deadlock() {
    let source = "\
Function Twice(ByVal x As Long) As Long
    Twice = x * 2
End Function

Dim value As Long
value = 21
Debug.Print value
";
    // 第 7 行是 `Debug.Print value`，断点在模块级代码里命中。
    let control = Arc::new(DebugControl::new(HashSet::from([7])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                source,
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    wait_until(|| control.is_paused());
    let value = evaluate(&control, "Twice(value)").expect("监视里可以调用脚本过程");
    assert_eq!(value.value, "42");
    assert_eq!(value.ty, "Long");

    control.resume();
    runner
        .join()
        .expect("解释器线程不应 panic")
        .expect("脚本应当跑完");
}

#[test]
fn conditional_breakpoint_stops_only_when_the_condition_holds() {
    let source = "\
Dim i As Long
For i = 1 To 3
    Debug.Print i
Next
";
    let control = Arc::new(DebugControl::new(HashSet::from([3])));
    control.set_breakpoint_conditions(HashMap::from([(3, "i = 2".to_string())]));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = crate::interpreter::run_debug(
                source,
                |line| lines.push(line.to_string()),
                &interrupt,
                RunOptions::default(),
                control,
            );
            (result, lines)
        })
    };

    wait_until(|| control.is_paused());
    let snapshot = control.snapshot().expect("条件为真时应当停下");
    assert_eq!(snapshot.line, 3);
    assert!(
        snapshot
            .variables
            .iter()
            .any(|variable| variable.name == "i" && variable.value == "2"),
        "应当停在 i = 2 的那一次：{:?}",
        snapshot.variables
    );

    control.resume();
    let (result, lines) = runner.join().expect("解释器线程不应 panic");
    result.expect("脚本应当跑完");
    assert_eq!(lines, ["1", "2", "3"]);
    assert_eq!(control.pause_epoch(), 1, "条件为假时不应停下");
}

#[test]
fn conditional_breakpoint_with_a_broken_condition_still_stops() {
    let control = Arc::new(DebugControl::new(HashSet::from([2])));
    control.set_breakpoint_conditions(HashMap::from([(
        2,
        "missing = 1".to_string(),
    )]));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                "Dim total As Long\ntotal = 1\n",
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    wait_until(|| control.is_paused());
    assert_eq!(control.snapshot().expect("快照").line, 2);

    control.resume();
    runner
        .join()
        .expect("解释器线程不应 panic")
        .expect("脚本应当跑完");
}

#[test]
fn conditional_breakpoint_with_a_false_condition_never_stops() {
    let control = Arc::new(DebugControl::new(HashSet::from([2])));
    control.set_breakpoint_conditions(HashMap::from([(2, "False".to_string())]));
    let interrupt = AtomicBool::new(false);
    let result = crate::interpreter::run_debug(
        "Dim total As Long\ntotal = 1\n",
        |_| {},
        &interrupt,
        RunOptions::default(),
        control.clone(),
    );

    result.expect("脚本应当直接跑完");
    assert_eq!(control.pause_epoch(), 0, "条件为假时不应暂停");
}

#[test]
fn evaluate_requires_a_pause() {
    let control = DebugControl::new(HashSet::new());
    assert_eq!(
        control.request_eval("1 + 1"),
        None,
        "没暂停时不能求值"
    );
    assert!(control.take_eval_result().is_none());
}

#[test]
fn parse_expression_rejects_trailing_tokens() {
    use crate::interpreter::parser::parse_expression;

    assert!(parse_expression("1 + 2 * 3").is_ok());
    assert!(parse_expression("total").is_ok());
    assert!(parse_expression("p.Name").is_ok());
    assert!(parse_expression("1 + 2)").is_err(), "多余的右括号应当报错");
    assert!(parse_expression("total 5").is_err(), "表达式后面不能再有内容");
    assert!(parse_expression("").is_err(), "空表达式应当报错");
}

