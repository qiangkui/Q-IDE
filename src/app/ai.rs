//! AI 面板发来的「写入文件」请求：确认路径、写盘，并同步编辑器与资源管理器。

use super::dialogs::*;
use super::*;

impl CodeWindow {
    /// 执行 AI 调试工具请求；运行前先验证活动文件，避免把无效请求报告成成功。
    pub(crate) fn ai_execute_action(
        &mut self,
        action: crate::ai::tools::AppAction,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        use crate::ai::tools::AppAction;

        match action {
            AppAction::RunScript => {
                self.ai_validate_runnable(cx)?;
                self.run_script(cx);
                Ok("已启动当前 VB 脚本；可用 read_output 查看输出。".to_string())
            }
            AppAction::StartDebug => {
                self.ai_validate_runnable(cx)?;
                if self
                    .debug
                    .as_ref()
                    .is_some_and(|session| !session.control.is_finished())
                {
                    return Err("调试会话已经在运行，请先检查状态或停止当前会话".to_string());
                }
                self.start_debug(cx);
                Ok("已启动调试会话；可用 get_debug_state 查看断点和变量。".to_string())
            }
            AppAction::ContinueDebug => {
                if self.debug.is_none() {
                    self.ai_validate_runnable(cx)?;
                } else if self
                    .debug
                    .as_ref()
                    .is_some_and(|session| session.control.is_finished())
                {
                    return Err("调试会话已经结束，请重新启动调试".to_string());
                }
                self.debug_resume(cx);
                Ok("已请求继续调试；可用 get_debug_state 查看暂停位置和变量。".to_string())
            }
            AppAction::StepDebug => {
                if self.debug.is_none() {
                    self.ai_validate_runnable(cx)?;
                } else if !self.debug.as_ref().is_some_and(|session| session.paused) {
                    return Err("单步操作需要脚本处于调试暂停状态".to_string());
                }
                self.debug_step(cx);
                Ok("已请求单步执行；可用 get_debug_state 查看下一次暂停状态。".to_string())
            }
            AppAction::PauseDebug => {
                if self.debug.is_none() {
                    return Err("当前没有调试会话，无法暂停".to_string());
                }
                self.debug_pause(cx);
                Ok("已请求在下一条语句处暂停。".to_string())
            }
            AppAction::StopExecution => {
                if self.debug.is_none() && self.script_interrupt.is_none() {
                    return Err("当前没有正在运行的脚本".to_string());
                }
                self.stop_script(cx);
                Ok("已请求停止脚本。".to_string())
            }
            AppAction::SetBreakpoint {
                line,
                enabled,
                condition,
            } => self.ai_set_breakpoint(line, enabled, condition.as_deref(), cx),
            AppAction::AddWatch { expression } => self.ai_add_watch(&expression, cx),
        }
    }

    fn ai_validate_runnable(&self, cx: &App) -> Result<(), String> {
        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            return Err("当前文件不是 VB 脚本，无法运行或调试".to_string());
        }
        if self.editor.read(cx).value().trim().is_empty() {
            return Err("当前文件没有内容，无法运行或调试".to_string());
        }
        Ok(())
    }

    /// 当前脚本的运行状态、断点、暂停快照与监视结果，供 AI 调试工具读取。
    pub(crate) fn ai_debug_state(&self) -> String {
        let mut lines = Vec::new();
        let state = match &self.debug {
            Some(session) if session.control.is_finished() => "调试会话已结束",
            Some(session) if session.paused => "调试会话已暂停",
            Some(_) => "调试会话运行中",
            None if self.script_interrupt.is_some() => "脚本运行中（非调试）",
            None => "脚本未运行",
        };
        lines.push(format!("状态：{state}"));
        if let Some(path) = self.active_script_path() {
            lines.push(format!("当前文件：{}", path.display()));
        }

        let breakpoints = self.active_breakpoints();
        if breakpoints.is_empty() {
            lines.push("断点：无".to_string());
        } else {
            let conditions = self.active_breakpoint_conditions();
            let mut breakpoints: Vec<_> = breakpoints.into_iter().collect();
            breakpoints.sort_unstable();
            lines.push(format!(
                "断点：{}",
                breakpoints
                    .into_iter()
                    .map(|line| match conditions.get(&line) {
                        Some(condition) => format!("{line}（条件：{condition}）"),
                        None => line.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }

        if let Some(session) = &self.debug
            && let Some(snapshot) = &session.snapshot
        {
            lines.push(format!(
                "暂停位置：第 {} 行，过程 {}",
                snapshot.line, snapshot.procedure
            ));
            if !snapshot.call_stack.is_empty() {
                lines.push(format!(
                    "调用栈：{}",
                    snapshot
                        .call_stack
                        .iter()
                        .map(|frame| format!("{}（第 {} 行）", frame.procedure, frame.line))
                        .collect::<Vec<_>>()
                        .join(" → ")
                ));
            }
            if snapshot.variables.is_empty() {
                lines.push("可见变量：无".to_string());
            } else {
                lines.push("可见变量：".to_string());
                for variable in snapshot.variables.iter().take(200) {
                    lines.push(format!(
                        "{}.{} As {} = {}",
                        variable.scope, variable.name, variable.ty, variable.value
                    ));
                }
                if snapshot.variables.len() > 200 {
                    lines.push(format!(
                        "（另有 {} 个变量未显示）",
                        snapshot.variables.len() - 200
                    ));
                }
            }
        }

        if self.watches.is_empty() {
            lines.push("监视：无".to_string());
        } else {
            lines.push("监视：".to_string());
            for (index, expression) in self.watches.iter().enumerate() {
                let result = match self.watch_states.get(index) {
                    Some(WatchState::Pending) => "（求值中）".to_string(),
                    Some(WatchState::Ready { value, ty }) => format!(" = {value}（{ty}）"),
                    Some(WatchState::Failed(message)) => format!("（求值失败：{message}）"),
                    _ => "（尚无结果）".to_string(),
                };
                lines.push(format!("{expression}{result}"));
            }
        }
        lines.join("\n")
    }

    /// 由 AI 按 1 基行号设置或移除当前 VB 文件的断点。
    pub(crate) fn ai_set_breakpoint(
        &mut self,
        line: usize,
        enabled: bool,
        condition: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let Some(index) = self.active_file else {
            return Err("没有打开的文件，无法设置断点".to_string());
        };
        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            return Err("当前文件不是 VB 脚本，无法设置断点".to_string());
        }
        let line_count = self.editor.read(cx).text().lines_len();
        if line == 0 || line > line_count {
            return Err(format!(
                "行号 {line} 超出当前文件范围（共 {line_count} 行）"
            ));
        }
        let id = self.open_files[index].id.to_string();
        if enabled {
            self.breakpoints.entry(id.clone()).or_default().insert(line);
            if let Some(condition) = condition.map(str::trim).filter(|text| !text.is_empty()) {
                self.breakpoint_conditions
                    .entry(id)
                    .or_default()
                    .insert(line, condition.to_string());
            } else if let Some(conditions) = self.breakpoint_conditions.get_mut(&id) {
                conditions.remove(&line);
            }
        } else {
            if let Some(breakpoints) = self.breakpoints.get_mut(&id) {
                breakpoints.remove(&line);
            }
            if let Some(conditions) = self.breakpoint_conditions.get_mut(&id) {
                conditions.remove(&line);
            }
        }
        self.push_breakpoints_to_session();
        self.refresh_breakpoint_marks(cx);
        self.sync_outline_panel(cx);
        self.save_layout(cx);
        cx.notify();

        Ok(if enabled {
            match condition.map(str::trim).filter(|text| !text.is_empty()) {
                Some(condition) => format!("已设置第 {line} 行条件断点：{condition}"),
                None => format!("已设置第 {line} 行断点"),
            }
        } else {
            format!("已移除第 {line} 行断点")
        })
    }

    /// 添加 AI 请求的监视表达式，并返回明确的工具执行结果。
    pub(crate) fn ai_add_watch(
        &mut self,
        expression: &str,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err("监视表达式不能为空".to_string());
        }
        if self
            .watches
            .iter()
            .any(|watch| watch.eq_ignore_ascii_case(expression))
        {
            return Ok(format!("已经在监视 `{expression}`"));
        }
        self.add_watch(expression, cx);
        Ok(format!(
            "已添加监视 `{expression}`；暂停时可通过 get_debug_state 查看结果"
        ))
    }

    /// 当前打开文件的路径与实时内容，作为 AI 请求的上下文。
    pub(crate) fn ai_file_context(&self) -> Option<(String, String)> {
        let index = self.active_file?;
        let file = self.open_files.get(index)?;
        let content = self.contents.get(index)?;
        Some((file.id.to_string(), content.to_string()))
    }

    /// 当前编辑器使用的语言（没有打开文件时按新建的 VB 缓冲区处理）。
    pub(crate) fn ai_language(&self) -> String {
        self.active_file_language().to_string()
    }

    /// 「写入文件」：先让用户确认目标路径，再写盘。
    pub(crate) fn apply_ai_code(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        let hint = self.ai_target_hint();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("文件路径，例如 src/main.vb")
                .default_value(hint)
        });
        let handle = cx.entity().downgrade();
        let code = code.to_string();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let target = field.read(cx).value().to_string();
                let _ = handle
                    .update(cx, |this, cx| this.write_ai_code(&target, &code, window, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "ai-write",
                title: "写入文件",
                description: Some(
                    "会用这段代码覆盖目标文件的全部内容；相对路径按项目根目录解析，\
                     文件不存在会自动新建。",
                ),
                confirm: "写入",
            },
            input,
            apply,
        );
    }

    /// 写入对话框里的默认路径：当前文件（相对项目根目录），没有就新建一个。
    fn ai_target_hint(&self) -> String {
        if let Some(index) = self.active_file
            && let Some(file) = self.open_files.get(index)
        {
            let id = file.id.to_string();
            let path = Path::new(&id);
            if let Some(root) = self
                .explorer
                .roots()
                .iter()
                .find(|root| path.starts_with(root.path()))
                && let Ok(relative) = path.strip_prefix(root.path())
            {
                return relative.to_string_lossy().into_owned();
            }
            return id;
        }
        "ai-output.vb".to_string()
    }

    /// 把对话框里填的路径变成绝对路径：相对项目根目录，没根目录时用当前文件所在目录。
    fn resolve_ai_path(&self, target: &str) -> PathBuf {
        let path = PathBuf::from(target);
        if path.is_absolute() {
            return path;
        }
        if let Some(root) = self.explorer.roots().first() {
            return root.path().join(path);
        }
        if let Some(index) = self.active_file
            && let Some(file) = self.open_files.get(index)
        {
            let id = file.id.to_string();
            if let Some(dir) = Path::new(&id).parent() {
                return dir.join(path);
            }
        }
        path
    }

    /// 写盘：文件已打开就换掉编辑器内容并按原编码保存，否则写完再打开它。
    fn write_ai_code(
        &mut self,
        target: &str,
        code: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = target.trim();
        if target.is_empty() {
            self.output().warning(cx, "没有填写文件路径，已取消写入");
            return;
        }
        let path = self.resolve_ai_path(target);
        if let Err(error) = self.write_ai_file(&path, code, window, cx) {
            self.output()
                .error(cx, format!("写入 {} 失败：{error}", path.display()));
        }
    }

    /// 把内容写进文件（AI 的「写入」按钮与工具调用共用）。
    ///
    /// 已打开的标签会直接换掉编辑器内容并按原编码保存；没打开的写到磁盘再打开。
    pub(crate) fn write_ai_file(
        &mut self,
        path: &Path,
        content: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let id = path.to_string_lossy().into_owned();

        if let Some(index) = self
            .open_files
            .iter()
            .position(|file| file.id.as_ref() == id.as_str())
        {
            if let Some(editor) = self.editors.get(index).cloned() {
                let (selection, scroll) = {
                    let state = editor.read(cx);
                    (state.selected_range(), state.scroll_offset())
                };
                editor.update(cx, |editor, cx| {
                    editor.set_value(content.to_string(), window, cx);
                    let selection = selection.start.min(content.len())
                        ..selection.end.min(content.len());
                    let start = char_boundary_at_or_before(content, selection.start);
                    let end = char_boundary_at_or_before(content, selection.end).max(start);
                    editor.set_selected_range(start..end, cx);
                    editor.set_scroll_offset(scroll, cx);
                });
            }
            if let Some(slot) = self.contents.get_mut(index) {
                *slot = content.to_string().into();
            }

            self.write_content(index, path)
                .map_err(|error| error.to_string())?;
            if self.active_file == Some(index) {
                self.reset_editor_caches();
                self.refresh_outline(cx);
                self.refresh_diagnostics(cx);
                self.refresh_breakpoint_marks(cx);
                self.refresh_editor_language(cx);
            }
            self.sync_editor_panel(cx);
            self.output().success(cx, format!("已写入 {id}"));
            cx.notify();
            return Ok(());
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建目录 {}：{error}", parent.display()))?;
        }
        std::fs::write(path, encoding::encode(content, Default::default()))
            .map_err(|error| error.to_string())?;
        self.output().success(cx, format!("已写入 {id}"));
        self.rebuild_tree(cx);
        self.open_file(&id, window, cx);
        Ok(())
    }

    /// 项目根目录（资源管理器里的第一个根文件夹）。
    pub(crate) fn ai_project_root(&self) -> Option<PathBuf> {
        self.explorer
            .roots()
            .first()
            .map(|root| root.path().to_path_buf())
    }

    /// 文件当前的内容：已打开的标签用编辑器里的版本（可能还没保存），否则读磁盘。
    pub(crate) fn file_content(&self, path: &Path) -> Option<String> {
        let id = path.to_string_lossy();
        if let Some(index) = self
            .open_files
            .iter()
            .position(|file| file.id.as_ref() == id.as_ref())
        {
            return self.contents.get(index).map(|content| content.to_string());
        }
        let bytes = std::fs::read(path).ok()?;
        Some(encoding::decode(&bytes).text)
    }

    /// 输出面板最近的内容（脚本输出与报错都在里面）。
    pub(crate) fn ai_output_tail(&self, cx: &App) -> String {
        self.output.read(cx).text()
    }
}

fn char_boundary_at_or_before(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}
