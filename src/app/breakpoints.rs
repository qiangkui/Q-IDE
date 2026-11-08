//! 断点、断点条件与编辑器里的断点标记。

use super::*;
use super::dialogs::*;

impl CodeWindow {
    /// 在当前行切换断点（`F9`）。断点按文件分别记录。
    pub(crate) fn toggle_breakpoint(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.active_file else {
            self.output().warning(cx, "没有打开的文件，无法设置断点");
            return;
        };
        let id = self.open_files[index].id.to_string();
        let line = self.cursor_line(cx);
        let set = self.breakpoints.entry(id.clone()).or_default();
        let added = !set.remove(&line) && set.insert(line);
        if !added && let Some(conditions) = self.breakpoint_conditions.get_mut(&id) {
            // 取消断点时条件一并清掉。
            conditions.remove(&line);
        }

        self.push_breakpoints_to_session();
        self.refresh_breakpoint_marks(cx);
        self.sync_outline_panel(cx);
        // 断点会写进布局文件，重启后还在。
        self.save_layout(cx);

        let output = self.output();
        if added {
            output.command(cx, format!("> 断点：第 {line} 行"));
        } else {
            output.command(cx, format!("> 取消断点：第 {line} 行"));
        }
        cx.notify();
    }

    /// 当前文件的断点集合。
    pub(super) fn active_breakpoints(&self) -> HashSet<usize> {
        self.active_file
            .and_then(|index| self.open_files.get(index))
            .and_then(|file| self.breakpoints.get(file.id.as_ref()))
            .cloned()
            .unwrap_or_default()
    }

    /// 当前文件的条件断点：行号 → 条件表达式。
    pub(super) fn active_breakpoint_conditions(&self) -> HashMap<usize, String> {
        self.active_file
            .and_then(|index| self.open_files.get(index))
            .and_then(|file| self.breakpoint_conditions.get(file.id.as_ref()))
            .cloned()
            .unwrap_or_default()
    }

    /// 把当前文件的断点与条件交给正在运行的调试控制器。
    pub(super) fn push_breakpoints_to_session(&self) {
        let Some(session) = &self.debug else {
            return;
        };
        session.control.set_breakpoints(self.active_breakpoints());
        session
            .control
            .set_breakpoint_conditions(self.active_breakpoint_conditions());
    }

    /// 打开「条件断点」对话框：给当前行的断点加条件，留空表示清除条件。
    pub(crate) fn set_breakpoint_condition_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            // 已有对话框在前台时不要再叠一个。
            return;
        }
        let Some(index) = self.active_file else {
            self.output().warning(cx, "没有打开的文件，无法设置条件断点");
            return;
        };
        let line = self.cursor_line(cx);
        let current = self
            .breakpoint_conditions
            .get(self.open_files[index].id.as_ref())
            .and_then(|conditions| conditions.get(&line))
            .cloned()
            .unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("例如 i > 3；留空表示清除条件")
                .default_value(current)
        });
        let handle = cx.entity().downgrade();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let text = field.read(cx).value().to_string();
                let _ = handle
                    .update(cx, |this, cx| this.set_breakpoint_condition(line, &text, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "breakpoint-condition",
                title: "条件断点",
                description: Some("只有表达式为真时才会在这一行停下；留空表示清除条件。"),
                confirm: "确定",
            },
            input,
            apply,
        );
    }

    /// 设置或清除某一行的断点条件；设置时如果这一行还没有断点，会一并加上。
    pub(crate) fn set_breakpoint_condition(
        &mut self,
        line: usize,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.active_file else {
            self.output().warning(cx, "没有打开的文件，无法设置条件断点");
            return;
        };
        let id = self.open_files[index].id.to_string();
        let condition = text.trim();
        if condition.is_empty() {
            let removed = self
                .breakpoint_conditions
                .get_mut(&id)
                .is_some_and(|conditions| conditions.remove(&line).is_some());
            if removed {
                self.push_breakpoints_to_session();
                self.refresh_breakpoint_marks(cx);
                self.sync_outline_panel(cx);
                self.save_layout(cx);
                self.output()
                    .command(cx, format!("> 已清除第 {line} 行的断点条件"));
                cx.notify();
            } else {
                self.output()
                    .warning(cx, format!("第 {line} 行没有断点条件"));
            }
            return;
        }

        self.breakpoints.entry(id.clone()).or_default().insert(line);
        self.breakpoint_conditions
            .entry(id)
            .or_default()
            .insert(line, condition.to_string());
        self.push_breakpoints_to_session();
        self.refresh_breakpoint_marks(cx);
        self.sync_outline_panel(cx);
        self.save_layout(cx);
        self.output().command(
            cx,
            format!("> 条件断点：第 {line} 行，条件 `{condition}`"),
        );
        cx.notify();
    }

    /// 编辑之后装饰会跟着文字挪动，这里把行号同步回断点集合。
    pub(super) fn sync_breakpoints_from_marks(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.active_file else {
            return;
        };
        let Some(collection) = self
            .breakpoint_marks
            .get(index)
            .and_then(|marks| marks.as_ref())
        else {
            return;
        };
        let file_id = self.open_files[index].id.to_string();
        let lines: HashSet<usize> = {
            let editor = self.editor.read(cx);
            let text = editor.text();
            collection
                .get_ranges(cx)
                .into_iter()
                .map(|range| text.offset_to_position(range.start).line as usize + 1)
                .collect()
        };
        let entry = self.breakpoints.entry(file_id.clone()).or_default();
        if *entry != lines {
            *entry = lines;
            // 装饰范围会跟着编辑移动，条件也要按新的行号重新挂。
            if let Some(conditions) = self.breakpoint_conditions.get(&file_id) {
                let remapped = crate::layout::remap_breakpoint_conditions(conditions, entry);
                self.breakpoint_conditions.insert(file_id, remapped);
            }
            self.push_breakpoints_to_session();
            self.sync_outline_panel(cx);
        }
    }

    /// 光标所在行（1 基）。
    pub(super) fn cursor_line(&self, cx: &App) -> usize {
        let editor = self.editor.read(cx);
        let text = editor.text();
        let offset = editor.cursor().min(text.len());
        text.offset_to_position(offset).line as usize + 1
    }

    /// 某一行的字节范围（含行尾的换行符）；行号越界时返回 `None`。
    ///
    /// 空范围会被编辑器当成「行边界」画在两行之间，所以行号栏的断点标记
    /// 也必须用整行范围。
    pub(super) fn line_offsets(state: &EditorState, row: usize) -> Option<Range<usize>> {
        let text = state.text();
        if row >= text.lines_len() {
            return None;
        }
        let start = text.position_to_offset(&Position::new(row as u32, 0));
        let end = if row + 1 < text.lines_len() {
            text.position_to_offset(&Position::new(row as u32 + 1, 0))
        } else {
            text.len()
        };
        (start < end).then_some(start..end)
    }

    /// 把断点画到当前标签的行号栏里。
    pub(super) fn refresh_breakpoint_marks(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.active_file else {
            return;
        };
        let Some(editor) = self.editors.get(index).cloned() else {
            return;
        };
        let palette = Palette::for_theme(cx);
        let breakpoints = self.active_breakpoints();
        let conditions = self.active_breakpoint_conditions();
        let marks: Vec<GutterDecoration> = {
            let state = editor.read(cx);
            breakpoints
                .iter()
                .filter_map(|line| {
                    let row = line.checked_sub(1)?;
                    let range = Self::line_offsets(state, row)?;
                    // 条件断点用不同颜色，和普通断点一眼能分开。
                    let color = if conditions.contains_key(line) {
                        palette.warning
                    } else {
                        palette.error
                    };
                    Some(GutterDecoration::new(range).with_color(rgb(color).into()))
                })
                .collect()
        };
        if self.breakpoint_marks[index].is_none() {
            let collection = editor.update(cx, |editor, cx| {
                editor.create_gutter_decorations_collection(Vec::new(), cx)
            });
            self.breakpoint_marks[index] = Some(collection);
        }
        if let Some(collection) = &self.breakpoint_marks[index] {
            collection.set(marks, cx);
        }
    }

    /// 高亮光标旁边括号的配对括号；光标不挨着括号时清掉高亮。
    pub(super) fn refresh_bracket_match(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.active_file else {
            return;
        };
        let Some(editor) = self.editors.get(index).cloned() else {
            return;
        };
        let pair = {
            let state = editor.read(cx);
            let text = state.value().to_string();
            brackets::matching_pair(&text, state.cursor())
        };
        let next = match pair {
            Some((open, close)) => BracketHighlight::Pair(open, close),
            None => BracketHighlight::None,
        };
        // 只有配对变化时才重画：`set` 每次都会 notify，否则会把自己再叫醒。
        if self.bracket_pairs[index] == next {
            return;
        }
        self.bracket_pairs[index] = next;
        if self.bracket_marks[index].is_none() {
            let collection = editor.update(cx, |editor, cx| {
                editor.create_range_decorations_collection(Vec::new(), cx)
            });
            self.bracket_marks[index] = Some(collection);
        }
        if let Some(collection) = &self.bracket_marks[index] {
            let marks: Vec<RangeDecoration> = match pair {
                Some((open, close)) => {
                    let text = editor.read(cx).value().to_string();
                    let open_len = text[open..].chars().next().map_or(1, char::len_utf8);
                    let close_len = text[close..].chars().next().map_or(1, char::len_utf8);
                    [open..open + open_len, close..close + close_len]
                        .into_iter()
                        .map(|range| {
                            RangeDecoration::new(range)
                                .with_style(RangeDecorationStyle::Fill)
                                .with_color(ui_theme::bracket_match_highlight())
                        })
                        .collect()
                }
                None => Vec::new(),
            };
            collection.set(marks, cx);
        }
    }

    /// 清除所有文件的断点（`Ctrl+Shift+F9`）。
    pub(crate) fn clear_all_breakpoints(&mut self, cx: &mut Context<Self>) {
        let total: usize = self.breakpoints.values().map(HashSet::len).sum();
        if total == 0 {
            self.output().warning(cx, "当前没有断点");
            return;
        }
        self.breakpoints.clear();
        self.breakpoint_conditions.clear();
        self.run_to = None;
        self.push_breakpoints_to_session();
        self.refresh_breakpoint_marks(cx);
        self.sync_outline_panel(cx);
        self.save_layout(cx);
        self.output().command(cx, format!("> 已清除 {total} 个断点"));
        cx.notify();
    }

}
