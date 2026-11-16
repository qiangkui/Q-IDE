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
    /// 变量取自哪一层调用栈（`call_stack` 的下标，0 是模块/最外层）。
    ///
    /// 刚暂停时是最内层；界面点「调用栈」里的某一行后变成那一层。
    pub frame: usize,
    /// 因为「出错时中断」停在这里时的错误信息；普通断点/单步暂停是 `None`。
    pub error: Option<String>,
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
///
/// 带 `frame` 的查询都是「对着 `call_stack` 的某一层」做：0 是最外层（模块），
/// 暂停时的最内层则是 `call_stack.len() - 1`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugQuery<'a> {
    /// 收集指定调用层的变量与位置快照。
    Snapshot(usize),
    /// 在指定调用层求值一个监视表达式。
    Evaluate { frame: usize, expression: &'a str },
    /// 求值条件断点的条件，返回真假（总在最内层求值：它就是正在执行的那一层）。
    Condition(&'a str),
    /// 在指定调用层给变量赋值：`(变量名, 新值的表达式文本)`。
    Assign {
        frame: usize,
        target: &'a str,
        value: &'a str,
    },
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
enum RequestKind {
    /// 求值一条表达式（监视、条件返回）；`frame` 是要站到哪一层去求值。
    Evaluate { frame: usize, expression: String },
    /// 给变量赋值：`(变量名, 新值的表达式文本)`。
    Assign {
        frame: usize,
        target: String,
        value: String,
    },
}

#[derive(Clone, Debug)]
struct EvalRequest {
    id: u64,
    kind: RequestKind,
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
pub(crate) enum Mode {
    /// 直接往下跑，只在命中断点时停。
    #[default]
    Running,
    /// 已经请求暂停（或「单步逐语句」），下一条语句处停下——进入被调过程也会停。
    Pausing,
    /// 单步跳过（逐过程）：只在**不浅于** `depth` 的语句处停，进入更深的调用不拦。
    StepOver { depth: usize },
    /// 单步跳出：等调用深度**小于** `depth` 的第一条语句。
    StepOut { depth: usize },
    /// 已经暂停，等界面给出下一步动作。
    Paused,
}

/// 这一条语句要不要因为「单步」而停下（`depth` 是当前调用深度，模块级为 0）。
pub(crate) fn step_reached(mode: Mode, depth: usize) -> bool {
    match mode {
        Mode::Pausing => true,
        Mode::StepOver { depth: start } => depth <= start,
        Mode::StepOut { depth: start } => depth < start,
        Mode::Running | Mode::Paused => false,
    }
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
    /// 暂停那一刻的调用深度（模块级 0）：单步跳过/跳出按它决定停不停。
    paused_depth: usize,
    /// 界面选中的调用层（`call_stack` 下标）：变量、求值与赋值都按它来。
    ///
    /// 每次进入暂停都重置成最内层；只有暂停期间才能改。
    frame: usize,
    /// 界面请求切换调用层、还没被解释器线程处理。
    frame_request: Option<usize>,
    /// 「出错时中断」：脚本遇到没有处理的错误时先停下，而不是直接结束。
    break_on_error: bool,
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

    /// 开关「出错时中断」（下一次遇到未处理的错误时生效）。
    pub fn set_break_on_error(&self, enabled: bool) {
        self.lock().break_on_error = enabled;
    }

    pub fn break_on_error(&self) -> bool {
        self.lock().break_on_error
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

    /// 单步跳过（逐过程）：执行完当前语句后再停，但进入被调过程不停。
    pub fn step_over(&self) {
        self.arm_step(|depth| Mode::StepOver { depth });
    }

    /// 单步跳出：一路跑到当前过程的调用者里，再停下。
    pub fn step_out(&self) {
        self.arm_step(|depth| Mode::StepOut { depth });
    }

    /// 暂停中发起的单步：把当前深度记进 `Mode`，然后叫醒解释器线程。
    fn arm_step(&self, mode: impl FnOnce(usize) -> Mode) {
        let paused = {
            let mut inner = self.lock();
            let paused = inner.mode == Mode::Paused;
            if paused {
                let depth = inner.paused_depth;
                inner.mode = mode(depth);
            }
            paused
        };
        if paused {
            self.wake.notify_all();
        }
    }

    /// 单步（逐语句）：执行完当前语句后再停一次，进入被调过程也会停。
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

    /// 界面点了「调用栈」里的某一层：切过去看那一层的变量。
    ///
    /// 返回是否真的换了层。解释器线程醒来后重新取快照，`pause_epoch` 加一，
    /// 界面那套「暂停变化才刷新」的逻辑会把新的变量列表显示出来。
    pub fn select_frame(&self, frame: usize) -> bool {
        let changed = {
            let mut inner = self.lock();
            if inner.mode != Mode::Paused || inner.stop {
                return false;
            }
            if frame > inner.paused_depth || frame == inner.frame {
                return false;
            }
            inner.frame = frame;
            inner.frame_request = Some(frame);
            true
        };
        if changed {
            self.wake.notify_all();
        }
        changed
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
            kind: RequestKind::Evaluate {
                frame: inner.frame,
                expression: expression.to_string(),
            },
        });
        drop(inner);
        // 解释器线程正睡在条件变量上，必须叫醒它。
        self.wake.notify_all();
        Some(id)
    }

    /// 暂停期间把一个变量改成新值（`value` 是新值的 VB 表达式文本）。
    ///
    /// 结果和 [`Self::request_eval`] 走同一个队列：界面用 [`Self::take_eval_result`] 按 `id` 取回。
    pub fn request_assign(&self, target: &str, value: &str) -> Option<u64> {
        let mut inner = self.lock();
        if inner.mode != Mode::Paused || inner.stop {
            return None;
        }
        inner.next_eval_id += 1;
        let id = inner.next_eval_id;
        inner.eval_request = Some(EvalRequest {
            id,
            kind: RequestKind::Assign {
                frame: inner.frame,
                target: target.to_string(),
                value: value.to_string(),
            },
        });
        drop(inner);
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
        depth: usize,
        mut query: impl FnMut(DebugQuery<'_>) -> DebugReply,
    ) -> DebugSignal {
        let mut inner = self.lock();
        if inner.stop {
            inner.finished = true;
            return DebugSignal::Stop;
        }

        // 单步/暂停请求优先；否则看这一行有没有断点，有条件就先求值。
        let step_stop = step_reached(inner.mode, depth);
        if !step_stop {
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
                if !should_stop && !step_reached(inner.mode, depth) {
                    return DebugSignal::Continue;
                }
            }
        }

        let snapshot = match query(DebugQuery::Snapshot(depth)) {
            DebugReply::Snapshot(snapshot) => snapshot,
            _ => DebugSnapshot::default(),
        };
        self.pause_and_wait(inner, snapshot, depth, query)
    }

    /// 出错（没有 `On Error` 处理）时由解释器调用：开着「出错时中断」就在这里停下。
    ///
    /// 暂停期间照常可以看变量、求值监视表达式；点「继续」后调用方把原来的错误抛出去，
    /// 点「停止」则由调用方换成 [`STOPPED_MESSAGE`]。
    pub fn before_error(
        &self,
        line: usize,
        message: &str,
        depth: usize,
        mut query: impl FnMut(DebugQuery<'_>) -> DebugReply,
    ) -> DebugSignal {
        let inner = self.lock();
        if inner.stop {
            return DebugSignal::Stop;
        }
        if !inner.break_on_error {
            return DebugSignal::Continue;
        }
        drop(inner);
        let mut snapshot = match query(DebugQuery::Snapshot(depth)) {
            DebugReply::Snapshot(snapshot) => snapshot,
            _ => DebugSnapshot::default(),
        };
        snapshot.error = Some(message.to_string());
        snapshot.line = line;
        let inner = self.lock();
        if inner.stop {
            return DebugSignal::Stop;
        }
        self.pause_and_wait(inner, snapshot, depth, query)
    }

    /// 把快照交给界面并阻塞到用户继续/停止（`before_line` 与 `before_error` 共用）。
    fn pause_and_wait<'a>(
        &'a self,
        mut inner: std::sync::MutexGuard<'a, Inner>,
        snapshot: DebugSnapshot,
        depth: usize,
        mut query: impl FnMut(DebugQuery<'_>) -> DebugReply,
    ) -> DebugSignal {
        inner.snapshot = Some(snapshot);
        inner.pause_epoch += 1;
        inner.eval_request = None;
        inner.eval_result = None;
        inner.paused_depth = depth;
        // 每次暂停都从最内层（正在执行的那一层）开始看变量。
        inner.frame = depth;
        inner.frame_request = None;
        inner.mode = Mode::Paused;

        loop {
            // 界面切了调用层：重新取那一层的快照交给它。
            if let Some(frame) = inner.frame_request.take() {
                drop(inner);
                let changed = match query(DebugQuery::Snapshot(frame)) {
                    DebugReply::Snapshot(snapshot) => Some(snapshot),
                    _ => None,
                };
                inner = self.lock();
                if inner.mode == Mode::Paused
                    && !inner.stop
                    && let Some(snapshot) = changed
                {
                    inner.snapshot = Some(snapshot);
                    inner.pause_epoch += 1;
                }
                continue;
            }

            // 求值会执行用户表达式（可能调用脚本过程），不能持着调试锁。
            if let Some(request) = inner.eval_request.take() {
                drop(inner);
                let refresh = matches!(request.kind, RequestKind::Assign { .. });
                let result = match &request.kind {
                    RequestKind::Evaluate { frame, expression } => {
                        match query(DebugQuery::Evaluate {
                            frame: *frame,
                            expression,
                        }) {
                            DebugReply::Value(result) => result,
                            _ => Err("求值请求没有得到结果".to_string()),
                        }
                    }
                    RequestKind::Assign {
                        frame,
                        target,
                        value,
                    } => match query(DebugQuery::Assign {
                        frame: *frame,
                        target,
                        value,
                    }) {
                        DebugReply::Value(result) => result,
                        _ => Err("赋值请求没有得到结果".to_string()),
                    },
                };
                // 赋值成功后再取一次快照：变量面板与监视列表立刻显示新值
                // （`pause_epoch` 加一，界面那一套「暂停变化才刷新」的逻辑会跟着走）。
                // 快照取的是赋值那层：改的是外层变量的值时，显示的也该是外层。
                let refreshed = if refresh
                    && result.is_ok()
                    && let RequestKind::Assign { frame, .. } = &request.kind
                {
                    match query(DebugQuery::Snapshot(*frame)) {
                        DebugReply::Snapshot(snapshot) => Some(snapshot),
                        _ => None,
                    }
                } else {
                    None
                };
                inner = self.lock();
                if inner.mode == Mode::Paused && !inner.stop {
                    inner.eval_result = Some(EvalOutcome {
                        id: request.id,
                        result,
                    });
                    if let Some(snapshot) = refreshed {
                        inner.snapshot = Some(snapshot);
                        inner.pause_epoch += 1;
                    }
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
