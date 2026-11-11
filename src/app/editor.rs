//! 编辑区相关：导航、命令面板、补全/诊断刷新与文本编辑动作。

use super::*;
use super::dialogs::*;

impl CodeWindow {
    /// 转到定义（`F12`）：光标处的名字在当前脚本里的声明位置。
    pub(crate) fn go_to_definition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            self.output().warning(cx, "转到定义目前只支持 VB 脚本");
            return;
        }
        let (word, line) = {
            let editor = self.editor.read(cx);
            let text = editor.text();
            let offset = editor.cursor().min(text.len());
            let line = text.offset_to_position(offset).line as usize + 1;
            (Self::word_at(text, offset), line)
        };
        let Some(word) = word else {
            self.output().warning(cx, "光标处没有可跳转的名字");
            return;
        };

        let target = outline::definition(&self.outline_model, &word, line)
            .map(|symbol| (symbol.name.clone(), symbol.line));
        let Some((name, target_line)) = target else {
            self.output().warning(cx, format!("找不到 `{word}` 的定义"));
            return;
        };

        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(
                Position::new(target_line.saturating_sub(1) as u32, 0),
                window,
                cx,
            );
        });
        self.output()
            .command(cx, format!("> 转到定义：{name}（第 {target_line} 行）"));
    }

    /// 把光标移到指定行（输出面板里点错误行时用）。
    pub(crate) fn go_to_line(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            let lines = editor.text().lines_len();
            if lines == 0 {
                return;
            }
            let row = line.saturating_sub(1).min(lines - 1) as u32;
            editor.set_cursor_position(Position::new(row, 0), window, cx);
        });
    }

    /// 查找引用（`Shift+F12`）：光标处的名字在脚本里出现的位置，列到输出面板，
    /// 点某一条就跳到那一行。
    pub(crate) fn find_references(&mut self, cx: &mut Context<Self>) {
        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            self.output().warning(cx, "查找引用目前只支持 VB 脚本");
            return;
        }
        let (word, source) = {
            let editor = self.editor.read(cx);
            let text = editor.text();
            let offset = editor.cursor().min(text.len());
            (Self::word_at(text, offset), text.to_string())
        };
        let Some(word) = word else {
            self.output().warning(cx, "光标处没有可查找的名字");
            return;
        };

        let found = outline::references(&source, &word);
        let output = self.output();
        output.update(cx, |log, _| log.set_channel("查找引用"));
        if found.is_empty() {
            output.warning(cx, format!("找不到 `{word}` 的引用"));
            return;
        }
        output.command(cx, format!("> 查找引用：{word}（{} 处）", found.len()));
        for (line, text) in found {
            output.info(cx, format!("第 {line} 行：{text}"));
        }
    }

    /// 调整编辑器字号（`Ctrl+=` / `Ctrl+-` / `Ctrl+0`）；`None` 表示恢复默认。
    pub(crate) fn zoom_editor(&mut self, delta: Option<f32>, cx: &mut Context<Self>) {
        const MIN_SIZE: f32 = 8.0;
        const MAX_SIZE: f32 = 32.0;
        let default = crate::settings::EditorSettings::default().font_size;

        SettingsStore::update(cx, |settings| {
            settings.editor.font_size = match delta {
                Some(delta) => (settings.editor.font_size + delta).clamp(MIN_SIZE, MAX_SIZE),
                None => default,
            };
        });
        let size = self.settings.read(cx).editor.font_size;
        self.output().command(cx, format!("> 编辑器字号：{size}"));
        cx.notify();
    }

    /// 清空「最近打开」列表。
    pub(crate) fn clear_recent_files(&mut self, cx: &mut Context<Self>) {
        if self.recent_files.is_empty() {
            self.output().warning(cx, "最近打开列表已经是空的");
            return;
        }
        self.recent_files.clear();
        self.save_layout(cx);
        self.output().command(cx, "> 已清除最近打开列表");
        cx.notify();
    }

    /// 打开「运行参数」对话框：设置脚本里 `Command()` 读到的字符串。
    pub(crate) fn edit_run_arguments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个。
            return;
        }
        let current = self.script_args.clone();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("例如：input.txt /v")
                .default_value(current)
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle.update(cx, |this, cx| this.set_run_arguments(text, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "args",
                title: "运行参数",
                description: Some("脚本里用 Command() 读取这段文字。"),
                confirm: "确定",
            },
            input,
            apply,
        );
    }

    /// 记住新的运行参数（持久化到布局文件）。
    pub(crate) fn set_run_arguments(&mut self, args: String, cx: &mut Context<Self>) {
        self.script_args = args;
        self.save_layout(cx);
        let shown = if self.script_args.is_empty() {
            "（空）".to_string()
        } else {
            self.script_args.clone()
        };
        self.output().command(cx, format!("> 运行参数：{shown}"));
        cx.notify();
    }

    /// 打开「转到行 / 符号」对话框（`Ctrl+G`；对话框里再按一次会关闭）。
    pub(crate) fn goto_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个（Ctrl+G 在对话框里由「取消」绑定处理）。
            return;
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("行号或过程 / 变量名，例如 42 或 AddOne")
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle.update(cx, |this, cx| this.apply_goto(&text, window, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "goto",
                title: "转到行 / 符号",
                description: None,
                confirm: "转到",
            },
            input,
            apply,
        );
    }

    /// 执行「转到行 / 符号」。
    pub(crate) fn apply_goto(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match outline::goto_target(text) {
            Some(outline::GotoTarget::Line(line)) => self.go_to_line(line, window, cx),
            Some(outline::GotoTarget::Symbol(name)) => {
                let current = self.cursor_line(cx);
                let target = outline::definition(&self.outline_model, &name, current)
                    .map(|symbol| symbol.line);
                match target {
                    Some(line) => self.go_to_line(line, window, cx),
                    None => self.output().warning(cx, format!("找不到 `{name}`")),
                }
            }
            None => self.output().warning(cx, "请输入行号或名字"),
        }
    }

    /// 打开「转到文件」对话框（`Ctrl+P`）：按文件名（或路径）模糊匹配。
    pub(crate) fn goto_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个。
            return;
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("文件名或路径，例如 main 或 src/main.rs")
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle.update(cx, |this, cx| this.open_matching_file(&text, window, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "goto-file",
                title: "转到文件",
                description: Some("按文件名模糊匹配；输入 / 或 \\ 时按完整路径匹配。"),
                confirm: "打开",
            },
            input,
            apply,
        );
    }

    /// 命令面板（`Ctrl+Shift+P`）：输入命令名，回车执行。
    pub(crate) fn command_palette_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("命令名，例如 保存 / 运行 / 主题")
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                // 先关掉命令面板再执行：有些命令自己会弹对话框（打开文件、转到行），
                // 后关会把新对话框一起关掉。
                window.close_dialog(cx);
                let _ = handle.update(cx, |this, cx| this.run_command_text(&text, window, cx));
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "command-palette",
                title: "命令面板",
                description: Some("按命令名模糊匹配，回车执行。"),
                confirm: "执行",
            },
            input,
            apply,
        );
    }

    /// 按用户输入的名字找命令并执行。
    pub(crate) fn run_command_text(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = commands::best_command(text) else {
            let query = text.trim();
            self.output()
                .warning(cx, format!("找不到命令 `{query}`，按 Ctrl+Shift+P 重新输入"));
            return;
        };
        let shortcut = if command.shortcut.is_empty() {
            String::new()
        } else {
            format!("（{}）", command.shortcut)
        };
        self.output()
            .command(cx, format!("> {}{}", command.label, shortcut));
        self.run_command(command.id, window, cx);
    }

    /// 执行命令面板里的命令（按 `commands::Command::id` 分发）。
    pub(crate) fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "file.new" => self.new_file(window, cx),
            "file.open" => self.open_files_dialog(window, cx),
            "file.save" => self.save_file(cx),
            "file.save-as" => self.save_file_as(window, cx),
            "file.save-all" => self.save_all_files(cx),
            "file.close" => {
                if let Some(index) = self.active_file {
                    self.close_file(index, window, cx);
                }
            }
            "file.close-all" => self.close_all_files(window, cx),
            "file.reopen" => self.reopen_closed_file(window, cx),
            "edit.find" => self.open_search(false, window, cx),
            "edit.replace" => self.open_search(true, window, cx),
            "edit.comment" => self.toggle_active_comment(window, cx),
            "edit.move-up" => self.move_active_lines(true, window, cx),
            "edit.move-down" => self.move_active_lines(false, window, cx),
            "edit.indent" => self.run_editor_action(Box::new(Indent), window, cx),
            "edit.outdent" => self.run_editor_action(Box::new(Outdent), window, cx),
            "edit.select-all" => self.run_editor_action(Box::new(SelectAll), window, cx),
            "goto.file" => self.goto_file_dialog(window, cx),
            "goto.line" => self.goto_dialog(window, cx),
            "goto.definition" => self.go_to_definition(window, cx),
            "goto.references" => self.find_references(cx),
            "view.explorer" => self.toggle_panel(PanelKind::Explorer, window, cx),
            "view.outline" => self.toggle_panel(PanelKind::Outline, window, cx),
            "view.search" => self.toggle_panel(PanelKind::Search, window, cx),
            "view.ai" => self.toggle_panel(PanelKind::Ai, window, cx),
            "view.output" => {
                SettingsStore::update(cx, |settings| {
                    settings.appearance.show_output = !settings.appearance.show_output;
                });
            }
            "view.status-bar" => {
                SettingsStore::update(cx, |settings| {
                    settings.appearance.show_status_bar = !settings.appearance.show_status_bar;
                });
            }
            "view.theme" => {
                let dark = ui_theme::active(cx).mode.is_dark();
                SettingsStore::update(cx, |settings| {
                    settings.appearance.theme = if dark {
                        ThemeChoice::Light
                    } else {
                        ThemeChoice::Dark
                    };
                });
            }
            "view.fullscreen" => window.toggle_fullscreen(),
            "run.start" => self.run_script(cx),
            "run.check" => self.check_syntax(cx),
            "run.stop" => self.stop_script(cx),
            "run.debug" => self.start_debug(cx),
            "run.pause" => self.debug_pause(cx),
            "run.resume" => self.debug_resume(cx),
            "run.step" => self.debug_step(cx),
            "run.to-cursor" => self.run_to_cursor(cx),
            "debug.toggle-breakpoint" => self.toggle_breakpoint(cx),
            "debug.clear-breakpoints" => self.clear_all_breakpoints(cx),
            _ => {}
        }
    }

    /// 执行「转到文件」：在已打开、最近打开与资源管理器根目录下的文件里找最匹配的一个。
    pub(crate) fn open_matching_file(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = text.trim();
        if query.is_empty() {
            self.output().warning(cx, "先输入文件名再转到");
            return;
        }
        let candidates = self.file_candidates();
        if candidates.is_empty() {
            self.output()
                .warning(cx, "没有可打开的文件：先在资源管理器里添加文件夹或打开文件");
            return;
        }
        let Some(path) = workspace::best_match(query, &candidates).cloned() else {
            self.output()
                .warning(cx, format!("找不到匹配 `{query}` 的文件"));
            return;
        };
        let name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        self.open_file(&path, window, cx);
        self.output()
            .command(cx, format!("> 转到文件：{name}（{path}）"));
    }

    /// 搜索候选：给了 `scope` 就只在该文件夹下递归收集，否则与「转到文件」一致。
    pub(crate) fn search_candidates(&self, scope: Option<&str>) -> Vec<String> {
        match scope {
            Some(scope) => {
                let mut files = Vec::new();
                let mut seen = HashSet::new();
                workspace::collect_files(Path::new(scope), &mut files, &mut seen);
                files
            }
            None => self.file_candidates(),
        }
    }

    /// 弹出文件夹选择框，把选中的文件夹设为搜索范围。
    pub(crate) fn choose_search_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择搜索范围".into()),
        });
        let panel = self.search_panel.clone();
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(dir) = paths.into_iter().next()
            {
                let scope = dir.to_string_lossy().into_owned();
                panel.update(cx, |panel, cx| panel.set_scope(Some(scope), cx));
            }
        })
        .detach();
    }

    /// 打开搜索命中的位置：先把文件打开（必要时切标签），再跳到那一行。
    pub(crate) fn open_search_hit(
        &mut self,
        path: &str,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self
            .active_file
            .is_some_and(|index| self.open_files[index].id == path);
        if !active {
            self.open_file(path, window, cx);
        }
        self.go_to_line(line, window, cx);
    }

    /// 「转到文件」的候选：已打开的文件、最近打开的文件与资源管理器各根目录下的文件。
    pub(super) fn file_candidates(&self) -> Vec<String> {
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for file in &self.open_files {
            let id = file.id.to_string();
            if seen.insert(id.clone()) {
                files.push(id);
            }
        }
        for path in &self.recent_files {
            if Path::new(path).is_file() && seen.insert(path.clone()) {
                files.push(path.clone());
            }
        }
        for root in self.explorer.roots() {
            workspace::collect_files(root.path(), &mut files, &mut seen);
        }
        files
    }

    /// 光标处的标识符（`_` 与字母数字），用于转到定义。
    pub(super) fn word_at(text: &Rope, offset: usize) -> Option<String> {
        let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
        let mut start = offset;
        for ch in text.chars_at(offset).reversed() {
            if is_word(ch) {
                start -= ch.len_utf8();
            } else {
                break;
            }
        }
        let mut end = offset;
        for ch in text.chars_at(offset) {
            if is_word(ch) {
                end += ch.len_utf8();
            } else {
                break;
            }
        }
        (start < end).then(|| text.slice(start..end).to_string())
    }

    /// 按当前文件与语法设置刷新编辑器的语法分析器与补全提供者。
    ///
    /// 补全只对 VB 脚本开放：打开 Rust / JSON / 纯文本文件时不提示 VB 关键字。
    pub(super) fn refresh_editor_language(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.read(cx).clone();
        let language = self.effective_language(&settings);
        let completion = self
            .active_file_language()
            .eq_ignore_ascii_case(crate::syntax::VB_LANGUAGE);
        if self.applied_language.as_ref() == Some(&language)
            && self.completion_active == completion
        {
            return;
        }

        let editor = self.editor.clone();
        let language_changed = self.applied_language.as_ref() != Some(&language);
        let completion_changed = self.completion_active != completion;
        editor.update(cx, |editor, cx| {
            if language_changed {
                editor.set_highlighter(language.clone(), cx);
            }
            if completion_changed {
                editor.lsp_mut().completion_provider =
                    completion.then(crate::completion::provider);
            }
        });
        self.applied_language = Some(language);
        self.completion_active = completion;
    }

    pub(super) fn refresh_outline(&mut self, cx: &mut Context<Self>) {
        let source = self.editor.read(cx).value().to_string();
        if self.outline_source.as_deref() == Some(source.as_str()) {
            return;
        }
        self.outline_source = Some(source.clone());
        let path = self.active_script_path();
        self.outline_model = if self.active_file_language().eq_ignore_ascii_case("vb") {
            OutlineModel::from_source_with_path(&source, path.as_deref())
        } else {
            OutlineModel::default()
        };
        self.sync_outline_panel(cx);
        cx.notify();
    }

    /// 当前生效的语言：设置里开启语法高亮时按设置解析，否则为纯文本。
    pub(super) fn effective_language(&self, settings: &AppSettings) -> SharedString {
        settings.editor.syntax.resolve(&self.active_file_language())
    }

    pub(super) fn active_file_language(&self) -> SharedString {
        self.active_file
            .and_then(|index| self.open_files.get(index))
            .map(|file| file.language.clone())
            // 没有打开文件时编辑框是新建的 VB 缓冲区。
            .unwrap_or_else(|| SharedString::from("vb"))
    }

    /// 这些缓存都属于「当前标签」：切换或关闭标签后要按新标签重新计算。
    pub(super) fn reset_editor_caches(&mut self) {
        self.outline_source = None;
        self.checked_source = None;
        self.applied_language = None;
        self.completion_active = false;
    }

    /// 把编辑器里的最新内容写回当前标签。
    pub(super) fn save_active_content(&mut self, cx: &App) {
        if let Some(active) = self.active_file
            && let Some(content) = self.contents.get_mut(active)
        {
            *content = self.editor.read(cx).value();
        }
    }

    /// 某个编辑器是否是当前标签的编辑器。
    pub(super) fn is_active_editor(&self, editor: &Entity<EditorState>) -> bool {
        self.active_file
            .and_then(|index| self.editors.get(index))
            .is_some_and(|active| active.entity_id() == editor.entity_id())
    }

    /// 按当前设置创建一个独立的编辑器状态。
    pub(super) fn create_editor(
        &self,
        content: SharedString,
        language: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorState> {
        let settings = self.settings.read(cx).editor.clone();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .soft_wrap(settings.soft_wrap)
                .line_number(settings.line_numbers)
                .folding(settings.folding)
                .auto_close(settings.auto_close)
                .default_value(content)
        });
        editor.update(cx, |editor, cx| {
            editor.set_highlighter_factory(crate::syntax::highlighter_factory(), cx);
        });
        editor
    }

    /// 为新标签创建编辑器并挂上变化监听，同时补齐与标签一一对应的状态容器。
    pub(super) fn add_editor(
        &mut self,
        content: SharedString,
        language: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorState> {
        let editor = self.create_editor(content, language, window, cx);
        let subscription = cx.observe_in(&editor, window, |this, editor, _, cx| {
            if !this.is_active_editor(&editor) {
                return;
            }
            this.sync_active_content(cx);
            this.refresh_outline(cx);
            this.refresh_diagnostics(cx);
            this.sync_breakpoints_from_marks(cx);
        });
        self.editors.push(editor.clone());
        self.editor_subscriptions.push(subscription);
        self.breakpoint_marks.push(None);
        self.paused_line_marks.push(None);
        self.bracket_marks.push(None);
        self.bracket_pairs.push(BracketHighlight::Unset);
        editor
    }

    /// 编辑框内容变化时把它同步到当前标签；内容真的变了才重绘（用于未保存圆点）。
    pub(super) fn sync_active_content(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active_file else {
            return;
        };
        let value = self.editor.read(cx).value();
        let mut changed = false;
        if let Some(content) = self.contents.get_mut(active)
            && *content != value
        {
            *content = value;
            changed = true;
        }
        if changed {
            self.sync_editor_panel(cx);
            cx.notify();
        }
    }

    /// 编辑内容变化后重新做一次 VB 语法检查。
    ///
    /// 只有 VB 脚本才检查；文本没变（只是移动光标、滚动）时直接返回，
    /// 避免重复解析同一个文本。
    pub(super) fn refresh_diagnostics(&mut self, cx: &mut Context<Self>) {
        crate::completion::set_script_path(self.active_script_path());
        let editor = self.editor.clone();

        if !self.active_file_language().eq_ignore_ascii_case("vb") {
            if self.checked_source.take().is_some() {
                editor.update(cx, crate::lint::clear);
            }
            if self.error_count != 0 {
                self.error_count = 0;
                cx.notify();
            }
            return;
        }

        let source = self.editor.read(cx).value().to_string();
        if self.checked_source.as_deref() == Some(source.as_str()) {
            return;
        }
        self.checked_source = Some(source);

        let path = self.active_script_path();
        let issues = editor.update(cx, |editor, cx| {
            crate::lint::refresh(editor, path.as_deref(), cx)
        });
        let errors = issues.len();
        if self.error_count != errors {
            self.error_count = errors;
            cx.notify();
        }
    }

    /// 把当前行（或选区覆盖的整行）上移 / 下移一行。
    pub(crate) fn move_active_lines(
        &mut self,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_file.is_none() {
            return;
        }
        let editor = self.editor.clone();
        let (text, selection, scroll) = {
            let state = editor.read(cx);
            (
                state.value().to_string(),
                state.selected_range(),
                state.scroll_offset(),
            )
        };
        let Some(edit) = editing::move_lines(&text, selection, up) else {
            let message = if up {
                "已经在第一行，无法上移"
            } else {
                "已经在最后一行，无法下移"
            };
            self.output().info(cx, message);
            return;
        };
        self.apply_text_edit(&editor, edit, scroll, window, cx);
    }

    /// 切换当前行（或选区覆盖的整行）的行注释（`Ctrl+/`）。
    pub(crate) fn toggle_active_comment(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_file.is_none() {
            return;
        }
        let language = self.active_file_language();
        let Some(token) = editing::comment_token(&language) else {
            self.output().info(cx, "当前文件类型没有行注释");
            return;
        };
        let editor = self.editor.clone();
        let (text, selection, scroll) = {
            let state = editor.read(cx);
            (
                state.value().to_string(),
                state.selected_range(),
                state.scroll_offset(),
            )
        };
        let Some(edit) = editing::toggle_comment(&text, selection, token) else {
            return;
        };
        self.apply_text_edit(&editor, edit, scroll, window, cx);
    }

    /// 用一次纯文本编辑替换整篇内容，并把选区、滚动位置放回原处。
    pub(super) fn apply_text_edit(
        &mut self,
        editor: &Entity<EditorState>,
        edit: editing::TextEdit,
        scroll: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        editor.update(cx, |state, cx| {
            state.replace_all(edit.text, window, cx);
            state.set_selected_range(edit.selection, cx);
            state.set_scroll_offset(scroll, cx);
            state.focus(window, cx);
        });
        self.sync_active_content(cx);
        cx.notify();
    }

    /// 把 gpui 动作交给编辑器执行（菜单里的撤销、复制等）。
    pub(crate) fn run_editor_action(
        &mut self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        window.dispatch_action(action, cx);
    }

    /// 打开编辑器自带的查找 / 替换面板（菜单「编辑 → 查找/替换」）。
    pub(crate) fn open_search(
        &mut self,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.focus(window, cx);
            editor.open_search(replace, cx);
        });
    }
}
