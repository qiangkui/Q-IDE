//! 脚本运行、调试会话与监视表达式。

use super::*;
use super::dialogs::*;

impl CodeWindow {
    /// 当前脚本在磁盘上的路径：相对路径按它所在目录解析，`App.Path` 也用这个目录。
    pub(super) fn active_script_path(&self) -> Option<PathBuf> {
        self.active_file
            .and_then(|index| self.open_files.get(index))
            .map(|file| PathBuf::from(file.id.to_string()))
    }

    /// 运行当前编辑框里的 VB 脚本。
    ///
    /// 只有 VB 脚本才能运行；其它语言的文件直接在输出面板提示无法运行。
    /// 脚本在独立线程里执行（`MsgBox` 之类的阻塞调用不会冻结窗口），
    /// 输出的每一行通过通道回到界面线程，追加到输出面板。
    pub(crate) fn run_script(&mut self, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        let source = self.editor.read(cx).value().to_string();
        let output = self.output();
        output.update(cx, |log, _| log.set_channel("vb 脚本"));

        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            output.error(cx, "无法运行：当前文件不是 VB 脚本");
            return;
        }

        if source.trim().is_empty() {
            output.warning(cx, "当前文件没有内容，无法运行");
            return;
        }

        // 上一次脚本（或暂停中的调试会话）还没结束就先结束它。
        if let Some(session) = &self.debug {
            session.control.stop();
        }
        self.debug = None;
        self.show_debug_line(None, cx);
        if let Some(interrupt) = self.script_interrupt.take() {
            interrupt.store(true, Ordering::Relaxed);
        }
        // 上一次脚本可能卡在对话框上，先把它关掉，否则那个线程会一直挂着。
        crate::interpreter::ffi::close_dialogs();

        output.command(cx, "> 运行 VB 脚本");

        let interrupt = Arc::new(AtomicBool::new(false));
        self.script_interrupt = Some(interrupt.clone());
        let options = RunOptions {
            command_line: self.script_args.clone(),
            script_path: self.active_script_path(),
            ..RunOptions::default()
        };

        let (sender, receiver) = mpsc::channel::<ScriptEvent>();
        let event_interrupt = interrupt.clone();
        let spawned = crate::interpreter::spawn_script_thread(move || {
            let line_sender = sender.clone();
            let result = crate::interpreter::run_with(
                &source,
                move |line| {
                    let _ = line_sender.send(ScriptEvent::Line(line.to_string()));
                },
                &interrupt,
                options,
            );
            let failure = result.map_err(|error| ScriptFailure::from_error(&error));
            let _ = sender.send(ScriptEvent::Finished(failure));
        });
        if let Err(error) = spawned {
            self.script_interrupt = None;
            output.error(cx, format!("无法启动脚本线程：{error}"));
            return;
        }

        let jump_source = self.editor.read(cx).value().to_string();
        self.spawn_script_events(receiver, jump_source, None, event_interrupt, cx);
    }

    /// 把脚本线程的事件搬到界面上；调试会话还会轮询暂停状态并高亮当前行。
    pub(super) fn spawn_script_events(
        &self,
        receiver: mpsc::Receiver<ScriptEvent>,
        source: String,
        debug: Option<Arc<DebugControl>>,
        interrupt: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) {
        let log = self.output.clone();
        let editor = self.editor.clone();
        // 脚本从启动到结束的墙钟时间（调试时的暂停也算在内）。
        let started = Instant::now();
        cx.spawn(async move |this, cx| {
            let mut last_debug_state: Option<(bool, u64)> = Some((false, 0));
            loop {
                loop {
                    match receiver.try_recv() {
                        Ok(ScriptEvent::Line(text)) => {
                            log.update(cx, |log, cx| log.push(OutputLevel::Info, text, cx));
                        }
                        Ok(ScriptEvent::Finished(result)) => {
                            let stopped = result.as_ref().err().is_some_and(|failure| {
                                failure.message.contains(vb_debug::STOPPED_MESSAGE)
                            });
                            match result {
                                Ok(()) => {
                                    let elapsed = started.elapsed().as_secs_f64();
                                    log.update(cx, |log, cx| {
                                        log.push(
                                            OutputLevel::Success,
                                            format!("脚本执行结束（用时 {elapsed:.2}s）"),
                                            cx,
                                        )
                                    });
                                    editor.update(cx, crate::lint::clear);
                                }
                                Err(_) if stopped => {
                                    let elapsed = started.elapsed().as_secs_f64();
                                    log.update(cx, |log, cx| {
                                        log.push(
                                            OutputLevel::Warning,
                                            format!("脚本已停止（用时 {elapsed:.2}s）"),
                                            cx,
                                        )
                                    });
                                }
                                Err(failure) => {
                                    log.update(cx, |log, cx| {
                                        log.push(OutputLevel::Error, failure.message.clone(), cx)
                                    });
                                    editor.update(cx, |editor, cx| {
                                        reveal_error_line(
                                            editor,
                                            &source,
                                            failure.line,
                                            &failure.message,
                                            cx,
                                        );
                                    });
                                }
                            }
                            let _ = this.update(cx, |this, cx| {
                                // 旧会话的结束事件不能把刚启动的新会话清掉。
                                let is_current = match (&debug, &this.debug) {
                                    (Some(finished), Some(current)) => {
                                        Arc::ptr_eq(finished, &current.control)
                                    }
                                    (None, None) => this
                                        .script_interrupt
                                        .as_ref()
                                        .is_some_and(|current| Arc::ptr_eq(current, &interrupt)),
                                    _ => false,
                                };
                                if is_current {
                                    this.debug = None;
                                    this.script_interrupt = None;
                                    this.watch_states =
                                        vec![WatchState::Inactive; this.watches.len()];
                                    this.show_debug_line(None, cx);
                                    this.sync_outline_panel(cx);
                                    cx.notify();
                                }
                            });
                            return;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }

                if let Some(control) = &debug {
                    // 只在「暂停状态 / 暂停次数」变化时取快照：变量快照可能有几 MB，
                    // 每 40ms 克隆一次会让菜单、面板开关明显发卡。
                    // 求值结果很轻，每轮轮询一次，有结果才碰界面。
                    let state = (control.is_paused(), control.pause_epoch());
                    let changed = last_debug_state != Some(state);
                    let outcome = if state.0 {
                        control.take_eval_result()
                    } else {
                        None
                    };
                    if changed || outcome.is_some() {
                        if changed {
                            last_debug_state = Some(state);
                        }
                        let snapshot = if changed && state.0 {
                            control.snapshot()
                        } else {
                            None
                        };
                        let _ = this.update(cx, |this, cx| {
                            let mut dirty = false;
                            let mut run_to_hit = false;
                            if changed {
                                let mut current_line = None;
                                let mut matched = false;
                                if let Some(session) = &mut this.debug
                                    && Arc::ptr_eq(control, &session.control)
                                {
                                    session.paused = state.0;
                                    session.snapshot = snapshot;
                                    session.watch_request = None;
                                    if state.0 {
                                        // 每次暂停都重新求值：变量可能已经变了。
                                        session.watch_queue =
                                            (0..this.watches.len()).collect();
                                        this.watch_states =
                                            vec![WatchState::Pending; this.watches.len()];
                                    } else {
                                        session.watch_queue.clear();
                                        this.watch_states =
                                            vec![WatchState::Inactive; this.watches.len()];
                                    }
                                    let line = session
                                        .snapshot
                                        .as_ref()
                                        .filter(|_| session.paused)
                                        .map(|snapshot| snapshot.line);
                                    run_to_hit = line.is_some() && this.run_to == line;
                                    current_line = Some(line);
                                    matched = true;
                                    dirty = true;
                                }
                                if matched {
                                    this.show_debug_line(current_line.flatten(), cx);
                                }
                            }
                            if let Some(outcome) = outcome {
                                let mut applied = false;
                                if let Some(session) = &mut this.debug
                                    && Arc::ptr_eq(control, &session.control)
                                    && session
                                        .watch_request
                                        .is_some_and(|(id, _)| id == outcome.id)
                                {
                                    if let Some((_, index)) = session.watch_request.take()
                                        && let Some(slot) = this.watch_states.get_mut(index)
                                    {
                                        *slot = match outcome.result {
                                            Ok(value) => WatchState::Ready {
                                                value: value.value,
                                                ty: value.ty,
                                            },
                                            Err(message) => WatchState::Failed(message),
                                        };
                                    }
                                    applied = true;
                                }
                                dirty |= applied;
                            }
                            // 一条求值完成后接着发下一条。
                            dirty |= this.pump_watch_requests();
                            // 「运行到光标处」的临时断点命中一次就撤掉。
                            if run_to_hit {
                                this.run_to = None;
                                let breakpoints = this.active_breakpoints();
                                if let Some(session) = &this.debug {
                                    session.control.set_breakpoints(breakpoints);
                                }
                            }
                            if dirty {
                                this.sync_outline_panel(cx);
                                cx.notify();
                            }
                        });
                    }
                }

                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
            }
        })
        .detach();
    }

    /// 带调试器运行当前脚本（`Ctrl+F5`）；已经停顿的会话用 `F11` 继续。
    pub(crate) fn start_debug(&mut self, cx: &mut Context<Self>) {
        self.begin_debug(false, cx);
    }

    /// 启动调试会话；`pause_first` 为真时在第一条语句处就停下（单步启动）。
    pub(super) fn begin_debug(&mut self, pause_first: bool, cx: &mut Context<Self>) {
        self.begin_debug_with(pause_first, None, cx);
    }

    /// 启动调试会话，并额外在一个临时断点处停下（「运行到光标处」）。
    pub(super) fn begin_debug_with(
        &mut self,
        pause_first: bool,
        run_to: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.save_active_content(cx);
        let output = self.output();
        output.update(cx, |log, _| log.set_channel("vb 调试"));

        if let Some(session) = &self.debug {
            if !session.control.is_finished() {
                output.warning(cx, "已经在调试中：F11 继续、F10 单步、Shift+F5 停止");
                return;
            }
            // 上一次会话已经结束（界面还没来得及收尾），直接换成新的。
            self.debug = None;
            self.show_debug_line(None, cx);
        }
        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            output.error(cx, "无法调试：当前文件不是 VB 脚本");
            return;
        }

        let source = self.editor.read(cx).value().to_string();
        if source.trim().is_empty() {
            output.warning(cx, "当前文件没有内容，无法调试");
            return;
        }

        // 上一次脚本还没结束就先中断它。
        if let Some(interrupt) = self.script_interrupt.take() {
            interrupt.store(true, Ordering::Relaxed);
        }

        output.command(cx, "> 调试 VB 脚本");

        let interrupt = Arc::new(AtomicBool::new(false));
        self.script_interrupt = Some(interrupt.clone());
        let mut breakpoints = self.active_breakpoints();
        if let Some(line) = run_to {
            breakpoints.insert(line);
        }
        self.run_to = run_to;
        let control = Arc::new(DebugControl::new(breakpoints));
        control.set_breakpoint_conditions(self.active_breakpoint_conditions());
        if pause_first {
            control.pause();
        }
        // 上一次会话留下的监视结果已经过期。
        self.watch_states = vec![WatchState::Inactive; self.watches.len()];
        self.debug = Some(DebugSession {
            control: control.clone(),
            paused: false,
            snapshot: None,
            watch_queue: VecDeque::new(),
            watch_request: None,
        });

        let (sender, receiver) = mpsc::channel::<ScriptEvent>();
        let thread_interrupt = interrupt;
        let event_interrupt = thread_interrupt.clone();
        let thread_control = control.clone();
        let options = RunOptions {
            command_line: self.script_args.clone(),
            script_path: self.active_script_path(),
            ..RunOptions::default()
        };
        let spawned = crate::interpreter::spawn_script_thread(move || {
            let line_sender = sender.clone();
            let result = crate::interpreter::run_debug(
                &source,
                move |line| {
                    let _ = line_sender.send(ScriptEvent::Line(line.to_string()));
                },
                &thread_interrupt,
                options,
                thread_control,
            );
            let failure = result.map_err(|error| ScriptFailure::from_error(&error));
            let _ = sender.send(ScriptEvent::Finished(failure));
        });
        if let Err(error) = spawned {
            self.script_interrupt = None;
            self.debug = None;
            output.error(cx, format!("无法启动调试线程：{error}"));
            return;
        }

        self.spawn_script_events(
            receiver,
            self.editor.read(cx).value().to_string(),
            Some(control),
            event_interrupt,
            cx,
        );
    }

    /// 在下一行暂停（菜单「运行 → 暂停」）。
    pub(crate) fn debug_pause(&mut self, cx: &mut Context<Self>) {
        match &self.debug {
            Some(session) => {
                session.control.pause();
                self.output().command(cx, "> 暂停：下一条语句处停下");
            }
            None => self.output().warning(cx, "当前没有调试会话"),
        }
    }

    /// 继续运行到下一个断点；没有会话时直接启动调试运行。
    pub(crate) fn debug_resume(&mut self, cx: &mut Context<Self>) {
        match &self.debug {
            Some(session) => {
                session.control.resume();
                self.output().command(cx, "> 继续");
            }
            None => self.begin_debug(false, cx),
        }
    }

    /// 单步：没有会话时启动调试并停在第一条语句。
    pub(crate) fn debug_step(&mut self, cx: &mut Context<Self>) {
        match &self.debug {
            Some(session) => {
                session.control.step();
                self.output().command(cx, "> 单步");
            }
            None => self.begin_debug(true, cx),
        }
    }

    /// 打开「添加监视」对话框：暂停时输入的表达式会在每次暂停时求值。
    pub(crate) fn add_watch_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个。
            return;
        }
        // 光标停在某个名字上时先填进去，省得手输。
        let current = {
            let editor = self.editor.read(cx);
            let text = editor.text();
            let offset = editor.cursor().min(text.len());
            Self::word_at(text, offset).unwrap_or_default()
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("表达式，例如 total + 1 或 p.Name")
                .default_value(current)
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle.update(cx, |this, cx| this.add_watch(&text, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "watch",
                title: "添加监视",
                description: Some("脚本暂停时在「代码大纲 → 监视」里显示结果。"),
                confirm: "添加",
            },
            input,
            apply,
        );
    }

    /// 记住一条监视表达式；正在暂停时立刻排队求值。
    pub(crate) fn add_watch(&mut self, expression: &str, cx: &mut Context<Self>) {
        let expression = expression.trim().to_string();
        if expression.is_empty() {
            return;
        }
        if self
            .watches
            .iter()
            .any(|watch| watch.eq_ignore_ascii_case(&expression))
        {
            self.output()
                .warning(cx, format!("已经在监视 `{expression}`"));
            return;
        }

        self.watches.push(expression.clone());
        self.watch_states.push(WatchState::Inactive);
        let index = self.watches.len() - 1;
        if let Some(session) = &mut self.debug
            && session.paused
        {
            session.watch_queue.push_back(index);
            self.watch_states[index] = WatchState::Pending;
        }
        self.pump_watch_requests();
        self.save_layout(cx);
        self.sync_outline_panel(cx);
        self.output()
            .command(cx, format!("> 添加监视：{expression}（显示在「代码大纲」面板）"));
        cx.notify();
    }

    /// 清空监视列表。
    pub(crate) fn clear_watches(&mut self, cx: &mut Context<Self>) {
        if self.watches.is_empty() {
            self.output().warning(cx, "监视列表已经是空的");
            return;
        }
        self.watches.clear();
        self.watch_states.clear();
        if let Some(session) = &mut self.debug {
            session.watch_queue.clear();
            session.watch_request = None;
        }
        self.save_layout(cx);
        self.sync_outline_panel(cx);
        self.output().command(cx, "> 已清除所有监视");
        cx.notify();
    }

    /// 暂停中还有没求值的监视就发出下一条；返回是否发了新请求。
    pub(super) fn pump_watch_requests(&mut self) -> bool {
        let Some(session) = &mut self.debug else {
            return false;
        };
        if !session.paused || session.watch_request.is_some() {
            return false;
        }
        while let Some(index) = session.watch_queue.pop_front() {
            let Some(expression) = self.watches.get(index) else {
                continue;
            };
            let Some(id) = session.control.request_eval(expression) else {
                // 已经不在暂停状态，本轮不再发。
                return false;
            };
            session.watch_request = Some((id, index));
            if let Some(slot) = self.watch_states.get_mut(index) {
                *slot = WatchState::Pending;
            }
            return true;
        }
        false
    }

    /// 高亮（或取消高亮）当前执行行。
    pub(super) fn show_debug_line(&mut self, line: Option<usize>, cx: &mut Context<Self>) {
        let Some(line) = line else {
            // 调试行高亮按标签保存；结束时把所有标签上的高亮都清掉。
            for collection in self.paused_line_marks.iter().flatten() {
                collection.clear(cx);
            }
            return;
        };
        let Some(index) = self.active_file else {
            return;
        };
        let editor = self.editor.clone();
        let range = {
            let state = editor.read(cx);
            let row = line
                .saturating_sub(1)
                .min(state.text().lines_len().saturating_sub(1));
            Self::line_offsets(state, row)
        };
        let Some(range) = range else {
            return;
        };
        if self.paused_line_marks[index].is_none() {
            let collection = editor.update(cx, |editor, cx| {
                editor.create_range_decorations_collection(Vec::new(), cx)
            });
            self.paused_line_marks[index] = Some(collection);
        }
        if let Some(collection) = &self.paused_line_marks[index] {
            collection.set(
                vec![
                    RangeDecoration::new(range)
                        .with_style(RangeDecorationStyle::Fill)
                        .with_color(ui_theme::debug_line_highlight()),
                ],
                cx,
            );
        }
    }

    /// 运行到光标处（`Ctrl+F10`）：临时在当前行停下，命中后自动撤掉这个断点。
    pub(crate) fn run_to_cursor(&mut self, cx: &mut Context<Self>) {
        if self.active_file.is_none() {
            self.output().warning(cx, "没有打开的文件，无法运行");
            return;
        }
        let line = self.cursor_line(cx);
        let running = self
            .debug
            .as_ref()
            .map(|session| session.control.clone())
            .filter(|control| !control.is_finished());

        match running {
            Some(control) => {
                let mut breakpoints = self.active_breakpoints();
                breakpoints.insert(line);
                control.set_breakpoints(breakpoints);
                self.run_to = Some(line);
                control.resume();
                self.output().command(cx, format!("> 运行到第 {line} 行"));
            }
            None => self.begin_debug_with(false, Some(line), cx),
        }
    }

    /// 请求中断正在运行的脚本；调试会话暂停中也会立刻结束。
    pub(crate) fn stop_script(&mut self, cx: &mut Context<Self>) {
        let output = self.output();
        let debugging = self.debug.as_ref().is_some();
        if let Some(session) = &self.debug {
            session.control.stop();
        }
        // 脚本可能正等在一个对话框上，光设标志它看不到。
        crate::interpreter::ffi::close_dialogs();
        match self.script_interrupt.take() {
            Some(interrupt) => {
                interrupt.store(true, Ordering::Relaxed);
                output.warning(cx, "已请求中断脚本");
            }
            None if !debugging => output.warning(cx, "当前没有正在运行的脚本"),
            None => {}
        }
    }

    /// 语法 + 语义检查（菜单「运行 → 检查语法」）：不执行脚本。
    pub(crate) fn check_syntax(&mut self, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        let output = self.output();
        output.update(cx, |log, _| log.set_channel("vb 语法"));

        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            output.warning(cx, "当前文件不是 VB 脚本，已跳过语法检查");
            return;
        }

        let source = self.editor.read(cx).value().to_string();
        output.command(cx, "> 语法检查");

        let editor = self.editor.clone();
        let analysis = crate::lint::analyze_with_path(&source, self.active_script_path().as_deref());
        if analysis.issues.is_empty() {
            editor.update(cx, crate::lint::clear);
            self.checked_source = Some(source);
            self.error_count = 0;
            let stats = analysis.program.as_ref().map_or_else(String::new, |program| {
                format!(
                    "检查通过：{} 个过程 / {} 个 API 声明 / {} 条主语句",
                    program.procedures.len(),
                    program.declares.len(),
                    program.main.len()
                )
            });
            output.success(cx, stats);
            return;
        }

        // 语法错误和语义问题都写到输出面板，编辑器里定位到第一处。
        for issue in &analysis.issues {
            output.error(cx, format!("第 {} 行：{}", issue.line, issue.message));
        }
        let first = &analysis.issues[0];
        let (line, message) = (first.line, first.message.clone());
        editor.update(cx, |editor, cx| {
            reveal_error_line(editor, &source, line, &message, cx);
        });
        self.checked_source = Some(source);
        self.error_count = analysis.issues.len();
        cx.notify();
    }
}
