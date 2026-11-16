//! 语句计数、栈深度保护与「停止脚本」的检查。

use super::*;

impl<'p, 'o> Runtime<'p, 'o> {
    // ---------- 步数 / 中断 ----------

    pub(super) fn tick(&mut self, line: usize) -> Result<(), RuntimeError> {
        self.steps += 1;
        if self.steps.is_multiple_of(1024) {
            if self.interrupt.load(AtomicOrdering::Relaxed) {
                return Err(RuntimeError::new("脚本已被中断", line));
            }
            if self.steps > self.max_steps {
                return Err(RuntimeError::new(
                    "脚本执行步数超过上限，可能是死循环",
                    line,
                ));
            }
        }
        Ok(())
    }

    /// 调试模式下，在每条语句执行前把位置与变量交给界面，必要时在这里阻塞。
    pub(super) fn debug_check(&mut self, line: usize) -> Result<(), RuntimeError> {
        // 监视表达式执行期间不再停下来：解释器线程本来就卡在 `before_line` 里，
        // 再次进入会等一个永远不会到来的「继续」。
        if self.evaluating || self.debug.is_none() {
            return Ok(());
        }
        let Some(control) = self.debug.clone() else {
            return Ok(());
        };
        let depth = self.frames.len();
        let mut query = |request: DebugQuery<'_>| match request {
            DebugQuery::Snapshot(frame) => DebugReply::Snapshot(self.debug_snapshot(line, frame)),
            DebugQuery::Evaluate { frame, expression } => {
                DebugReply::Value(self.debug_evaluate(frame, expression, line))
            }
            DebugQuery::Condition(expression) => {
                DebugReply::Condition(self.debug_condition(expression, line))
            }
            DebugQuery::Assign {
                frame,
                target,
                value,
            } => DebugReply::Value(self.debug_assign(frame, target, value, line)),
        };
        if control.before_line(line, depth, &mut query) == DebugSignal::Stop {
            return Err(RuntimeError::new(debug::STOPPED_MESSAGE, line));
        }
        Ok(())
    }

    /// 「出错时中断」：没有 `On Error` 处理时，先把这条错误交给调试器停一下。
    ///
    /// 用户在暂停中点了「停止」就换成 [`debug::STOPPED_MESSAGE`] 收场；点「继续」则照常抛错。
    pub(super) fn debug_break_on_error(
        &mut self,
        error: &RuntimeError,
    ) -> Result<(), RuntimeError> {
        if self.evaluating || self.debug.is_none() {
            return Ok(());
        }
        let Some(control) = self.debug.clone() else {
            return Ok(());
        };
        if !control.break_on_error() {
            return Ok(());
        }
        let line = error.line;
        let depth = self.frames.len();
        let message = error.message.to_string();
        let mut query = |request: DebugQuery<'_>| match request {
            DebugQuery::Snapshot(frame) => DebugReply::Snapshot(self.debug_snapshot(line, frame)),
            DebugQuery::Evaluate { frame, expression } => {
                DebugReply::Value(self.debug_evaluate(frame, expression, line))
            }
            DebugQuery::Condition(expression) => {
                DebugReply::Condition(self.debug_condition(expression, line))
            }
            DebugQuery::Assign {
                frame,
                target,
                value,
            } => DebugReply::Value(self.debug_assign(frame, target, value, line)),
        };
        if control.before_error(line, &message, depth, &mut query) == DebugSignal::Stop {
            return Err(RuntimeError::new(debug::STOPPED_MESSAGE, line));
        }
        Ok(())
    }

    /// 暂停期间给变量赋值（界面的「设置变量值…」）：先求值右侧，再写回同名变量。
    ///
    /// `frame` 是要赋值的那一层调用栈（0 = 模块/最外层）：外层过程里的局部变量也能改。
    /// 只支持变量名（模块级、过程内的局部变量与 `Static`），不支持字段/数组元素——
    /// 那两种写法在暂停时用监视表达式读就够了，写回去要额外处理字段槽与下标越界。
    pub(super) fn debug_assign(
        &mut self,
        frame: usize,
        target: &str,
        value: &str,
        line: usize,
    ) -> Result<DebugValue, String> {
        let target = target.trim();
        if target.is_empty() {
            return Err("变量名不能为空".to_string());
        }
        let target = target.to_string();
        let value = value.to_string();
        self.with_debug_frame(frame, move |runtime| {
            runtime.debug_assign_here(&target, &value, line)
        })
    }

    /// 已站到目标调用层上：求值右侧并写回，再回读一次（类型转换已经生效）。
    pub(super) fn debug_assign_here(
        &mut self,
        target: &str,
        value: &str,
        line: usize,
    ) -> Result<DebugValue, String> {
        if self.find_slot(target).is_none() {
            return Err(format!("没有名为 `{target}` 的变量"));
        }
        let value = self.debug_eval_value(value, line)?;
        self.set_var(target, value, line, None)
            .map_err(|error| error.message.to_string())?;
        // 回读一次：`set_var` 会按变量声明类型做转换（`Long` 收到 1.5 会变成 2）。
        match self.lookup_slot(target).map(|slot| slot.value()) {
            Some(value) => Ok(DebugValue {
                value: value.debug_preview(DEBUG_VALUE_CHARS),
                ty: value.type_name(),
            }),
            None => Err(format!("没有名为 `{target}` 的变量")),
        }
    }

    /// 在暂停处求值一条监视表达式，给出显示文本与类型名。
    ///
    /// `frame` 是要站到哪一层调用栈上去求值（0 = 模块/最外层）。
    pub(super) fn debug_evaluate(
        &mut self,
        frame: usize,
        expression: &str,
        line: usize,
    ) -> Result<DebugValue, String> {
        let expression = expression.to_string();
        self.with_debug_frame(frame, move |runtime| {
            let value = runtime.debug_eval_value(&expression, line)?;
            Ok(DebugValue {
                value: value.debug_preview(DEBUG_VALUE_CHARS),
                ty: value.type_name(),
            })
        })
    }

    /// 求值条件断点的条件（总在最内层求值：它就是正在执行的那一层）。
    pub(super) fn debug_condition(
        &mut self,
        expression: &str,
        line: usize,
    ) -> Result<bool, String> {
        let value = self.debug_eval_value(expression, line)?;
        value.to_bool()
    }

    /// 调试器借一次运行时求值表达式；期间不再触发暂停检查。
    pub(super) fn debug_eval_value(
        &mut self,
        expression: &str,
        line: usize,
    ) -> Result<Value, String> {
        let expr = crate::interpreter::parser::parse_expression(expression)
            .map_err(|error| error.message)?;
        self.evaluating = true;
        let outcome = self.eval(&expr, line);
        self.evaluating = false;
        outcome.map_err(|error| error.message.to_string())
    }

    /// 站在指定调用层上做一件事：先临时把更深的帧摘下来，事后再原样放回去。
    ///
    /// 变量查找（`find_slot`/`lookup_slot`/`scope_mut`）认的都是栈顶帧，
    /// 「看外层过程的变量」最快的就是让那一层暂时成为栈顶。
    pub(super) fn with_debug_frame<R>(
        &mut self,
        frame: usize,
        action: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let frame = frame.min(self.frames.len());
        let deeper = self.frames.split_off(frame);
        let outcome = action(self);
        // 求值出错时可能留下半层帧，先截回目标层再放回外层，别把栈弄乱。
        if self.frames.len() > frame {
            self.frames.truncate(frame);
        }
        self.frames.extend(deeper);
        outcome
    }

    /// 取指定调用层（`call_stack` 下标，0 = 模块/最外层）的变量与位置快照。
    pub(super) fn debug_snapshot(&self, line: usize, frame: usize) -> DebugSnapshot {
        let depth = self.frames.len();
        let frame = frame.min(depth);
        // 选中的那一层；`None` 表示模块级（`frames` 里没有对应项）。
        let selected = if frame > 0 {
            self.frames.get(frame - 1)
        } else {
            None
        };
        let procedure = match selected {
            Some(inner) => inner
                .procedure
                .map(str::to_string)
                .unwrap_or_else(|| "过程".to_string()),
            None => "模块".to_string(),
        };
        let mut variables = Vec::new();
        for (name, slot) in &self.globals {
            variables.push(DebugVariable {
                scope: "模块".to_string(),
                name: name.to_string(),
                ty: slot.ty().name().to_string(),
                value: slot.value().debug_preview(DEBUG_VALUE_CHARS),
            });
        }
        if let Some(selected) = selected {
            for (name, slot) in &selected.locals {
                variables.push(DebugVariable {
                    scope: procedure.clone(),
                    name: name.to_string(),
                    ty: slot.ty().name().to_string(),
                    value: slot.value().debug_preview(DEBUG_VALUE_CHARS),
                });
            }
        }
        variables.sort_by(|left, right| {
            left.scope
                .cmp(&right.scope)
                .then_with(|| left.name.cmp(&right.name))
        });

        // 调用栈：从内往外走，每一层的行号由它的调用者给出。
        let mut call_stack = Vec::new();
        let mut caller_line = line;
        for inner in self.frames.iter().rev() {
            call_stack.push(debug::CallFrame {
                procedure: inner
                    .procedure
                    .map(str::to_string)
                    .unwrap_or_else(|| "过程".to_string()),
                line: caller_line,
            });
            caller_line = inner.call_line;
        }
        call_stack.push(debug::CallFrame {
            procedure: "模块".to_string(),
            line: caller_line,
        });
        call_stack.reverse();

        // 选中的那一层不是最内层时，快照里的行号换成它的调用处：
        // 编辑器的高亮与「运行到光标处」都跟着这一行走。
        let frame_line = call_stack.get(frame).map_or(line, |selected| selected.line);

        DebugSnapshot {
            line: frame_line,
            procedure,
            variables,
            call_stack,
            frame,
            error: None,
        }
    }
}
