//! 调试支持：断点（含条件断点）、暂停/继续、单步与暂停期间的表达式求值。
//!
//! 解释器线程在每条语句执行前调用 [`DebugControl::before_line`]：命中断点、
//! 或者界面请求了暂停/单步时，这里会保存当时的行号、调用栈与变量快照，然后阻塞解释器线程，
//! 直到界面请求继续、单步或者停止。界面用 [`DebugControl::snapshot`] 轮询快照。
//!
//! 阻塞期间界面还可以用 [`DebugControl::request_eval`] 求值监视表达式：
//! 解释器线程在等待循环里取出请求，借一次可变的运行时把结果写回，界面再轮询
//! [`DebugControl::take_eval_result`]。

use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex};

/// 脚本被「停止」时使用的错误信息；界面据此区分普通运行错误。
pub const STOPPED_MESSAGE: &str = "脚本已停止";

/// 暂停时某个变量的当前值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugVariable {
    /// 变量所属作用域：`模块` 或过程名。
    pub scope: String,
    pub name: String,
    /// 声明类型（`Long`、`String`、`Variant` …）。
    pub ty: String,
    /// 显示值（数组/记录只给规模，过长的字符串会截断）。
    pub value: String,
}

/// 暂停时交给界面的快照。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugSnapshot {
    /// 即将执行的行号（1 基）。
    pub line: usize,
    /// 当前所在的过程；模块级代码为 `模块`。
    pub procedure: String,
    /// 可见变量及其当前值，按作用域、名字排序。
    pub variables: Vec<DebugVariable>,
    /// 调用栈：从最外层（模块）到当前过程。
    pub call_stack: Vec<CallFrame>,
}

/// 调用栈里的一层。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFrame {
    /// 过程名；模块级代码为 `模块`。
    pub procedure: String,
    /// 这一层当前执行到的行号（1 基）。
    pub line: usize,
}

/// 暂停期间界面交给解释器线程的查询。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugQuery<'a> {
    /// 收集变量与调用栈快照。
    Snapshot,
    /// 求值一个监视表达式。
    Evaluate(&'a str),
    /// 求值条件断点的条件，返回真假。
    Condition(&'a str),
}

/// 解释器线程对 [`DebugQuery`] 的回应。
pub enum DebugReply {
    Snapshot(DebugSnapshot),
    Value(Result<DebugValue, String>),
    Condition(Result<bool, String>),
}

/// 监视表达式的求值结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugValue {
    /// 显示用文本（数组/记录只给规模，过长会截断）。
    pub value: String,
    /// 运行时类型名（`Long`、`String` …）。
    pub ty: String,
}

/// 一次求值请求的结果；`id` 用来和界面的请求配对。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalOutcome {
    pub id: u64,
    pub result: Result<DebugValue, String>,
}

#[derive(Clone, Debug)]
struct EvalRequest {
    id: u64,
    expression: String,
}

/// 解释器线程对调试控制器的回应。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugSignal {
    /// 继续执行。
    Continue,
    /// 用户要求停止脚本。
    Stop,
}

/// 调试器的运行状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    /// 直接往下跑，只在命中断点时停。
    #[default]
    Running,
    /// 已经请求暂停，下一条语句处停下。
    Pausing,
    /// 已经暂停，等界面给出下一步动作。
    Paused,
}

#[derive(Default)]
struct Inner {
    breakpoints: HashSet<usize>,
    /// 条件断点：行号 → 条件表达式。没有条目的断点是无条件断点。
    conditions: HashMap<usize, String>,
    mode: Mode,
    stop: bool,
    finished: bool,
    snapshot: Option<DebugSnapshot>,
    /// 每次进入暂停加一，界面据此只在暂停变化时取快照。
    pause_epoch: u64,
    /// 界面刚发出、还没被解释器线程取走的求值请求。
    eval_request: Option<EvalRequest>,
    /// 解释器线程算好、还没被界面取走的求值结果。
    eval_result: Option<EvalOutcome>,
    next_eval_id: u64,
}

/// 跨线程的调试控制器：界面线程发指令，解释器线程在 `before_line` 里等指令。
pub struct DebugControl {
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl DebugControl {
    pub fn new(breakpoints: HashSet<usize>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                breakpoints,
                ..Inner::default()
            }),
            wake: Condvar::new(),
        }
    }

    /// 更新断点集合（下一次 `before_line` 生效）。
    pub fn set_breakpoints(&self, breakpoints: HashSet<usize>) {
        self.lock().breakpoints = breakpoints;
    }

    /// 更新条件断点的条件（下一次 `before_line` 生效）；没有条件的断点直接删掉条目。
    pub fn set_breakpoint_conditions(&self, conditions: HashMap<usize, String>) {
        self.lock().conditions = conditions;
    }

    /// 请求在下一行暂停；已经在暂停时不做任何事。
    pub fn pause(&self) {
        let mut inner = self.lock();
        if inner.mode == Mode::Running {
            inner.mode = Mode::Pausing;
        }
    }

    /// 继续运行到下一个断点（或脚本结束）。
    pub fn resume(&self) {
        let paused = {
            let mut inner = self.lock();
            let paused = inner.mode == Mode::Paused;
            if paused {
                inner.mode = Mode::Running;
            }
            paused
        };
        if paused {
            self.wake.notify_all();
        }
    }

    /// 单步：执行完当前语句后再停一次。
    pub fn step(&self) {
        let paused = {
            let mut inner = self.lock();
            let paused = inner.mode == Mode::Paused;
            if paused {
                inner.mode = Mode::Pausing;
            }
            paused
        };
        if paused {
            self.wake.notify_all();
        }
    }

    /// 停止脚本：暂停中也会立刻醒来。
    pub fn stop(&self) {
        self.lock().stop = true;
        self.wake.notify_all();
    }

    /// 最近一次暂停时的位置快照。
    ///
    /// 这个值可能是几 MB 的变量快照，界面只在 [`Self::pause_epoch`] 变化时取一次。
    pub fn snapshot(&self) -> Option<DebugSnapshot> {
        self.lock().snapshot.clone()
    }

    /// 暂停计数：每进入一次暂停就加一。
    pub fn pause_epoch(&self) -> u64 {
        self.lock().pause_epoch
    }

    pub fn is_paused(&self) -> bool {
        self.lock().mode == Mode::Paused
    }

    /// 暂停期间请求求值一个表达式，返回请求 id；没有暂停时返回 `None`。
    ///
    /// 结果不会立刻拿到：解释器线程在下一次等待循环里取出请求，
    /// 算完后由界面用 [`Self::take_eval_result`] 轮询。
    pub fn request_eval(&self, expression: &str) -> Option<u64> {
        let mut inner = self.lock();
        if inner.mode != Mode::Paused || inner.stop {
            return None;
        }
        inner.next_eval_id += 1;
        let id = inner.next_eval_id;
        inner.eval_request = Some(EvalRequest {
            id,
            expression: expression.to_string(),
        });
        drop(inner);
        // 解释器线程正睡在条件变量上，必须叫醒它。
        self.wake.notify_all();
        Some(id)
    }

    /// 取走解释器算好的求值结果；没有新结果时返回 `None`。
    pub fn take_eval_result(&self) -> Option<EvalOutcome> {
        self.lock().eval_result.take()
    }

    /// 脚本已经结束（停止或跑完），界面据此收尾。
    pub fn is_finished(&self) -> bool {
        self.lock().finished
    }

    /// 标记脚本结束；正在等待的 `before_line` 会返回 `Stop`。
    pub fn finish(&self) {
        self.lock().finished = true;
    }

    /// 解释器在每条语句执行前调用；需要暂停时在这里阻塞。
    ///
    /// `query` 只在真的要暂停时才调用：正常运行时不必为每条语句收集变量。
    /// 暂停期间界面发来求值请求时，这里会用 [`DebugQuery::Evaluate`] 借一次运行时。
    pub fn before_line(
        &self,
        line: usize,
        mut query: impl FnMut(DebugQuery<'_>) -> DebugReply,
    ) -> DebugSignal {
        let mut inner = self.lock();
        if inner.stop {
            inner.finished = true;
            return DebugSignal::Stop;
        }

        // 单步/暂停请求优先；否则看这一行有没有断点，有条件就先求值。
        if inner.mode != Mode::Pausing {
            if !inner.breakpoints.contains(&line) {
                return DebugSignal::Continue;
            }
            let condition = inner.conditions.get(&line).cloned();
            if let Some(condition) = condition {
                // 条件可能调用脚本过程，必须在锁外求值。
                drop(inner);
                let should_stop = match query(DebugQuery::Condition(&condition)) {
                    DebugReply::Condition(Ok(value)) => value,
                    // 条件本身出错时照样停下，方便用监视查看原因。
                    _ => true,
                };
                inner = self.lock();
                if inner.stop {
                    inner.finished = true;
                    return DebugSignal::Stop;
                }
                if !should_stop && inner.mode != Mode::Pausing {
                    return DebugSignal::Continue;
                }
            }
        }

        let snapshot = match query(DebugQuery::Snapshot) {
            DebugReply::Snapshot(snapshot) => snapshot,
            DebugReply::Value(_) | DebugReply::Condition(_) => DebugSnapshot::default(),
        };
        inner.snapshot = Some(snapshot);
        inner.pause_epoch += 1;
        inner.eval_request = None;
        inner.eval_result = None;
        inner.mode = Mode::Paused;

        loop {
            // 求值会执行用户表达式（可能调用脚本过程），不能持着调试锁。
            if let Some(request) = inner.eval_request.take() {
                drop(inner);
                let result = match query(DebugQuery::Evaluate(&request.expression)) {
                    DebugReply::Value(result) => result,
                    DebugReply::Snapshot(_) | DebugReply::Condition(_) => {
                        Err("求值请求没有得到结果".to_string())
                    }
                };
                inner = self.lock();
                if inner.mode == Mode::Paused && !inner.stop {
                    inner.eval_result = Some(EvalOutcome {
                        id: request.id,
                        result,
                    });
                }
                continue;
            }

            if inner.mode != Mode::Paused || inner.stop {
                break;
            }
            inner = self.wake.wait(inner).expect("调试锁被投毒");
        }

        // 暂停结束：丢弃没处理完的请求与结果，避免界面拿到过期数据。
        inner.eval_request = None;
        inner.eval_result = None;

        if inner.stop {
            inner.finished = true;
            return DebugSignal::Stop;
        }
        DebugSignal::Continue
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }
}
