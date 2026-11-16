//! 调试控制器（断点、暂停/继续、单步、停止）的测试。
//!
//! 与控制台无关的纯逻辑：解释器线程停在断点上、界面轮询快照、再继续执行。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use crate::interpreter::RunOptions;
use crate::interpreter::debug::{DebugControl, DebugValue, Mode, STOPPED_MESSAGE, step_reached};

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

/// 等某个 id 的求值/赋值结果回来。
fn wait_for_eval(control: &DebugControl, id: u64) -> Result<DebugValue, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(outcome) = control.take_eval_result()
            && outcome.id == id
        {
            return outcome.result;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("请求 {id} 没有在预期时间内返回");
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
    runner
        .join()
        .expect("解释器线程不应 panic")
        .0
        .expect("脚本应当跑完");
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
    control.set_breakpoint_conditions(HashMap::from([(2, "missing = 1".to_string())]));
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
    assert_eq!(control.request_eval("1 + 1"), None, "没暂停时不能求值");
    assert!(control.take_eval_result().is_none());
}

#[test]
fn parse_expression_rejects_trailing_tokens() {
    use crate::interpreter::parser::parse_expression;

    assert!(parse_expression("1 + 2 * 3").is_ok());
    assert!(parse_expression("total").is_ok());
    assert!(parse_expression("p.Name").is_ok());
    assert!(parse_expression("1 + 2)").is_err(), "多余的右括号应当报错");
    assert!(
        parse_expression("total 5").is_err(),
        "表达式后面不能再有内容"
    );
    assert!(parse_expression("").is_err(), "空表达式应当报错");
}

/// 单步的停/不停判定是纯函数，直接按深度验证（不要靠线程时序）。
#[test]
fn step_reached_matches_the_requested_depth() {
    // 逐语句：任何深度都停。
    assert!(step_reached(Mode::Pausing, 0));
    assert!(step_reached(Mode::Pausing, 3));
    // 逐过程：只在发起深度或更浅处停，进入被调过程（更深）不停。
    assert!(step_reached(Mode::StepOver { depth: 1 }, 1));
    assert!(step_reached(Mode::StepOver { depth: 1 }, 0));
    assert!(!step_reached(Mode::StepOver { depth: 1 }, 2));
    // 跳出：只在比发起深度更浅的语句处停。
    assert!(step_reached(Mode::StepOut { depth: 1 }, 0));
    assert!(!step_reached(Mode::StepOut { depth: 1 }, 1));
    assert!(!step_reached(Mode::StepOut { depth: 1 }, 2));
    // 没在单步时不停。
    assert!(!step_reached(Mode::Running, 0));
    assert!(!step_reached(Mode::Paused, 0));
}

#[test]
fn step_over_does_not_enter_the_called_procedure() {
    let control = Arc::new(DebugControl::new(HashSet::new()));
    control.pause();
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = crate::interpreter::run_debug(
                "Sub Inner()\nDebug.Print \"inner\"\nEnd Sub\nDebug.Print \"start\"\nInner\nDebug.Print \"end\"\n",
                |line| lines.push(line.to_string()),
                &interrupt,
                RunOptions::default(),
                control,
            );
            (result, lines)
        })
    };

    // 模块级第一条语句是第 4 行（前 3 行是 `Sub Inner`）。
    wait_until(|| control.is_paused());
    assert_eq!(control.snapshot().expect("快照").line, 4);

    // 逐过程：停在第 5 行的 `Inner` 调用上，不进入过程体。
    control.step_over();
    wait_until(|| control.is_paused() && control.snapshot().is_some_and(|s| s.line == 5));
    assert_eq!(
        control.snapshot().expect("快照").procedure,
        "模块",
        "逐过程不该停在被调过程里"
    );

    // 再逐过程一次：把 Inner 整段跑完，停在第 6 行。
    control.step_over();
    wait_until(|| control.is_paused() && control.snapshot().is_some_and(|s| s.line == 6));

    control.resume();
    let (result, lines) = runner.join().expect("解释器线程不应 panic");
    result.expect("脚本应当跑完");
    assert_eq!(lines, ["start", "inner", "end"]);
}

#[test]
fn step_out_returns_to_the_caller() {
    let control = Arc::new(DebugControl::new(HashSet::new()));
    control.pause();
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = crate::interpreter::run_debug(
                "Sub Inner()\nDebug.Print \"inner\"\nDebug.Print \"inner2\"\nEnd Sub\nInner\nDebug.Print \"end\"\n",
                |line| lines.push(line.to_string()),
                &interrupt,
                RunOptions::default(),
                control,
            );
            (result, lines)
        })
    };

    wait_until(|| control.is_paused());
    assert_eq!(control.snapshot().expect("快照").line, 5);

    // 逐语句进入 Inner（第 2 行），停在过程内部。
    control.step();
    wait_until(|| control.is_paused() && control.snapshot().is_some_and(|s| s.line == 2));
    // 过程名在解释器内部一律小写（VB 名字不区分大小写），快照按内部名字报告。
    assert_eq!(control.snapshot().expect("快照").procedure, "inner");

    // 跳出过程：Inner 里剩下的语句照常执行，停在调用者的下一条语句（第 6 行）。
    control.step_out();
    wait_until(|| control.is_paused() && control.snapshot().is_some_and(|s| s.line == 6));
    assert_eq!(control.snapshot().expect("快照").procedure, "模块");

    control.resume();
    let (result, lines) = runner.join().expect("解释器线程不应 panic");
    result.expect("脚本应当跑完");
    assert_eq!(lines, ["inner", "inner2", "end"]);
}

/// 「出错时中断」开着时：脚本遇到没处理的错误先停在出错行，快照里带错误信息；
/// 点继续后照常抛错、脚本结束。
#[test]
fn break_on_error_pauses_at_the_failing_line() {
    let control = Arc::new(DebugControl::new(HashSet::new()));
    control.set_break_on_error(true);
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                "Dim x As Long\nx = 1 / 0\nDebug.Print x\n",
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    wait_until(|| control.is_paused());
    let snapshot = control.snapshot().expect("出错时应当有快照");
    assert_eq!(snapshot.line, 2, "应当停在出错的那一行");
    let error = snapshot.error.expect("快照里应当带上错误信息");
    assert!(error.contains("除数"), "错误信息不对：{error}");
    assert!(
        snapshot
            .variables
            .iter()
            .any(|variable| variable.name == "x")
    );

    control.resume();
    let result = runner.join().expect("解释器线程不应 panic");
    let error = result.expect_err("继续后脚本应当照常报错结束");
    // 显示文本形如「第 2 行运行时错误: 除数为零」。
    assert!(
        error.to_string().contains("第 2 行") && error.to_string().contains("除数"),
        "继续后应当照常报除零：{error}"
    );
}

/// 没开「出错时中断」时行为不变：不停、直接把错误抛出来。
#[test]
fn errors_do_not_pause_without_break_on_error() {
    let control = Arc::new(DebugControl::new(HashSet::new()));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                "Dim x As Long\nx = 1 / 0\n",
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    let error = runner
        .join()
        .expect("解释器线程不应 panic")
        .expect_err("脚本应当报错");
    assert!(
        error.to_string().contains("第 2 行") && error.to_string().contains("除数"),
        "应当照常报除零：{error}"
    );
    assert!(!control.is_paused(), "没开开关时不该暂停");
    assert!(control.snapshot().is_none());
}

/// 两层调用的脚本：外层 `Outer` 有自己的局部变量，内层 `Inner` 在断点处停下。
const NESTED_SOURCE: &str = "\
Sub Outer()
    Dim outerValue As Long
    outerValue = 7
    Inner
    Debug.Print outerValue
End Sub

Sub Inner()
    Dim innerValue As Long
    innerValue = 9
    Debug.Print innerValue
End Sub

Outer
";

/// 解释器线程跑完后的结果：脚本结果与打印出来的行。
type RunnerOutput = (Result<(), crate::interpreter::Error>, Vec<String>);

/// 在第 11 行（内层的 `Debug.Print innerValue`）停下两层的调用，返回控制器与线程句柄。
fn pause_in_nested_call() -> (Arc<DebugControl>, std::thread::JoinHandle<RunnerOutput>) {
    let control = Arc::new(DebugControl::new(HashSet::from([11])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = crate::interpreter::run_debug(
                NESTED_SOURCE,
                |line| lines.push(line.to_string()),
                &interrupt,
                RunOptions::default(),
                control,
            );
            (result, lines)
        })
    };
    wait_until(|| control.is_paused());
    (control, runner)
}

/// 切调用层：变量列表、位置与类型都换成那一层的。
#[test]
fn selecting_an_outer_frame_shows_its_variables() {
    let (control, runner) = pause_in_nested_call();

    let inner = control.snapshot().expect("暂停时应当有快照");
    assert_eq!(inner.frame, 2, "刚暂停时看的是最内层");
    assert_eq!(inner.line, 11);
    assert_eq!(inner.procedure, "inner");
    assert!(
        inner
            .variables
            .iter()
            .any(|variable| variable.name == "innervalue" && variable.value == "9"),
        "最内层应当有 innervalue = 9：{:?}",
        inner.variables
    );

    let epoch = control.pause_epoch();
    assert!(control.select_frame(1), "暂停时应当能切到外层");
    wait_until(|| control.pause_epoch() > epoch);

    let outer = control.snapshot().expect("切帧后应当有新快照");
    assert_eq!(outer.frame, 1);
    assert_eq!(outer.procedure, "outer");
    assert_eq!(outer.line, 4, "外层的行号应当是它的调用处");
    assert!(
        outer
            .variables
            .iter()
            .any(|variable| variable.name == "outervalue"
                && variable.scope == "outer"
                && variable.value == "7"),
        "外层应当看到自己的 outervalue = 7：{:?}",
        outer.variables
    );
    assert!(
        !outer
            .variables
            .iter()
            .any(|variable| variable.name == "innervalue"),
        "看外层时不该混进内层的局部变量：{:?}",
        outer.variables
    );

    control.resume();
    let (result, lines) = runner.join().expect("解释器线程不应 panic");
    result.expect("脚本应当跑完");
    assert_eq!(lines, ["9", "7"], "切帧不影响脚本本身的执行");
}

/// 在外层帧上赋值：改的是外层的局部变量，内层的变量不受影响。
#[test]
fn assigning_a_variable_of_an_outer_frame() {
    let (control, runner) = pause_in_nested_call();

    assert!(control.select_frame(1), "先切到外层");
    let id = control
        .request_assign("outervalue", "11")
        .expect("暂停时应当能发出赋值请求");
    let value = wait_for_eval(&control, id).expect("赋值应当成功");
    assert_eq!(value.value, "11");

    // 快照刷新的是当前看的那一层。
    let snapshot = control.snapshot().expect("赋值后应当有快照");
    assert_eq!(snapshot.frame, 1);
    assert!(
        snapshot
            .variables
            .iter()
            .any(|variable| variable.scope == "outer" && variable.value == "11"),
        "外层快照里应当是改过的值：{:?}",
        snapshot.variables
    );

    // 内层的变量没被碰到。
    let epoch = control.pause_epoch();
    assert!(control.select_frame(2), "切回最内层");
    wait_until(|| control.pause_epoch() > epoch);
    let inner = control.snapshot().expect("切帧后应当有新快照");
    assert!(
        inner
            .variables
            .iter()
            .any(|variable| variable.name == "innervalue" && variable.value == "9"),
        "内层的 innervalue 不该被外层赋值影响：{:?}",
        inner.variables
    );

    control.resume();
    let (result, lines) = runner.join().expect("解释器线程不应 panic");
    result.expect("脚本应当跑完");
    assert_eq!(lines, ["9", "11"], "外层之后读到的应当是改过的值");
}

/// 没暂停、或者层号越界时切帧请求直接忽略。
#[test]
fn selecting_a_frame_requires_a_pause_and_a_valid_index() {
    let control = DebugControl::new(HashSet::from([3]));
    assert!(!control.select_frame(0), "没暂停时不能切帧");

    let (control, runner) = pause_in_nested_call();
    assert!(!control.select_frame(2), "已经是最内层时不算切换");
    assert!(!control.select_frame(9), "越界的层号应当被拒绝");
    control.resume();
    runner
        .join()
        .expect("解释器线程不应 panic")
        .0
        .expect("脚本应当跑完");
}

/// 暂停期间可以改变量值：右侧按表达式求值、写回后再回读一次（类型转换生效）。
#[test]
fn assigning_a_variable_while_paused() {
    let control = Arc::new(DebugControl::new(HashSet::from([3])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = crate::interpreter::run_debug(
                "Dim total As Long\ntotal = 1\nDebug.Print total\n",
                |line| lines.push(line.to_string()),
                &interrupt,
                RunOptions::default(),
                control,
            );
            (result, lines)
        })
    };

    wait_until(|| control.is_paused());
    let id = control
        .request_assign("total", "40 + 2")
        .expect("暂停时应当能发出赋值请求");
    let value = wait_for_eval(&control, id).expect("赋值应当成功");
    assert_eq!(value.value, "42");
    assert_eq!(value.ty, "Long");

    // 快照立刻刷新：变量面板能看到新值（`pause_epoch` 也跟着加一）。
    let snapshot = control.snapshot().expect("赋值后应当有快照");
    assert!(
        snapshot.variables.iter().any(|variable| {
            variable.name == "total" && variable.value == "42" && variable.ty == "Long"
        }),
        "快照里应当是赋值后的值：{:?}",
        snapshot.variables
    );

    control.resume();
    let (result, lines) = runner.join().expect("解释器线程不应 panic");
    result.expect("脚本应当跑完");
    assert_eq!(lines, ["42"], "脚本读到的应当是改过的值");
}

/// 赋值也会走变量声明类型的转换：`Long` 变量收到 1.5 变成 2。
#[test]
fn assignment_coerces_to_the_declared_type() {
    let control = Arc::new(DebugControl::new(HashSet::from([3])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                "Dim n As Long\nn = 0\nDebug.Print n\n",
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    wait_until(|| control.is_paused());
    let id = control.request_assign("n", "1.5").expect("发请求");
    let value = wait_for_eval(&control, id).expect("赋值应当成功");
    assert_eq!(value.value, "2");
    control.resume();
    runner.join().expect("解释器线程不应 panic").expect("跑完");
}

/// 变量名不存在时给出可读的错误，而不是静默忽略。
#[test]
fn assigning_an_unknown_variable_reports_an_error() {
    let control = Arc::new(DebugControl::new(HashSet::from([2])));
    let runner = {
        let control = control.clone();
        let interrupt = AtomicBool::new(false);
        std::thread::spawn(move || {
            crate::interpreter::run_debug(
                "Dim total As Long\nDebug.Print total\n",
                |_| {},
                &interrupt,
                RunOptions::default(),
                control,
            )
        })
    };

    wait_until(|| control.is_paused());
    let id = control.request_assign("missing", "1").expect("发请求");
    let error = wait_for_eval(&control, id).expect_err("不存在的变量应当报错");
    assert!(error.contains("missing"), "{error}");
    control.resume();
    runner.join().expect("解释器线程不应 panic").expect("跑完");
}
