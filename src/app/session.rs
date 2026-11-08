//! 会话恢复、标签关闭、外部修改检测与布局保存。

use super::*;
use super::dialogs::*;

impl CodeWindow {
    /// 恢复上次退出时开着的标签，并把焦点放回当时激活的那一个。
    pub(super) fn restore_session(&mut self, layout: &Layout, window: &mut Window, cx: &mut Context<Self>) {
        if layout.open.is_empty() {
            return;
        }
        let mut restored = 0usize;
        let mut active_index = None;
        for path in layout.open.iter().take(SESSION_LIMIT) {
            // 只恢复还在磁盘上的文件；被删掉的标签不值得弹错误。
            if !Path::new(path).is_file() {
                continue;
            }
            self.open_file(path, window, cx);
            if let Some(index) = self.open_files.iter().position(|file| file.id == path.as_str()) {
                if path == &layout.active {
                    active_index = Some(index);
                }
                restored += 1;
            }
        }
        if let Some(index) = active_index {
            self.activate_file(index, window, cx);
        }
        if restored > 1 {
            self.output()
                .info(cx, format!("已恢复上次打开的 {restored} 个文件"));
        }
    }

    /// 在标签之间循环切换（`Ctrl+Tab` / `Ctrl+Shift+Tab`）。
    pub(crate) fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active_file else {
            return;
        };
        let len = self.open_files.len();
        if len < 2 {
            return;
        }
        let next = if forward {
            (active + 1) % len
        } else {
            (active + len - 1) % len
        };
        self.activate_file(next, window, cx);
    }

    /// 把当前布局写盘，下次启动时恢复。
    pub(super) fn save_layout(&self, cx: &App) {
        let settings = self.settings.read(cx);
        let dock = self.dock_area.read(cx);
        let left_width = dock.dock_size(DockPlacement::Left).map(f32::from);
        let right_width = dock.dock_size(DockPlacement::Right).map(f32::from);
        let left_panels: Vec<PanelId> = dock
            .layout(DockPlacement::Left)
            .map(|tree| tree.panels().collect())
            .unwrap_or_default();
        let right_panels: Vec<PanelId> = dock
            .layout(DockPlacement::Right)
            .map(|tree| tree.panels().collect())
            .unwrap_or_default();
        // 只把面板实际所在那一侧的宽度记到它名下：任何面板都可能被拖到任一侧，
        // 不能像以前那样把某一栏的宽度固定记到某个面板上。
        let width_of = |panel: PanelId, fallback: f32| {
            if left_panels.contains(&panel) {
                left_width.unwrap_or(fallback)
            } else if right_panels.contains(&panel) {
                right_width.unwrap_or(fallback)
            } else {
                fallback
            }
        };
        let output_height = dock
            .dock_size(DockPlacement::Bottom)
            .map(f32::from)
            .unwrap_or(self.output_height);
        let layout = Layout {
            layout_version: LAYOUT_VERSION,
            explorer_width: width_of(
                PanelId::from(self.explorer_panel.entity_id()),
                self.explorer_width,
            ),
            outline_width: width_of(
                PanelId::from(self.outline_panel.entity_id()),
                self.outline_width,
            ),
            search_width: width_of(
                PanelId::from(self.search_panel.entity_id()),
                self.search_width,
            ),
            ai_width: width_of(PanelId::from(self.ai_panel.entity_id()), self.ai_width),
            output_height,
            left_panel: settings.appearance.left_panel,
            right_panel: settings.appearance.right_panel,
            show_output: settings.appearance.show_output,
            show_status_bar: settings.appearance.show_status_bar,
            theme: settings.appearance.theme.as_str().to_string(),
            roots: self
                .explorer
                .roots()
                .iter()
                .map(|root| root.path().to_string_lossy().into_owned())
                .collect(),
            expanded: self.explorer.expanded_ids(),
            breakpoints: {
                let mut breakpoints: Vec<(String, Vec<usize>)> = self
                    .breakpoints
                    .iter()
                    .map(|(path, lines)| {
                        let mut lines: Vec<usize> = lines.iter().copied().collect();
                        lines.sort_unstable();
                        (path.clone(), lines)
                    })
                    .collect();
                breakpoints.sort_by(|left, right| left.0.cmp(&right.0));
                breakpoints
            },
            breakpoint_conditions: {
                let mut conditions: Vec<(String, Vec<(usize, String)>)> = self
                    .breakpoint_conditions
                    .iter()
                    .map(|(path, items)| {
                        let mut items: Vec<(usize, String)> = items
                            .iter()
                            .map(|(line, condition)| (*line, condition.clone()))
                            .collect();
                        items.sort_by_key(|(line, _)| *line);
                        (path.clone(), items)
                    })
                    .filter(|(_, items)| !items.is_empty())
                    .collect();
                conditions.sort_by(|left, right| left.0.cmp(&right.0));
                conditions
            },
            recent: self.recent_files.clone(),
            open: self
                .open_files
                .iter()
                .map(|file| file.id.to_string())
                .take(SESSION_LIMIT)
                .collect(),
            active: self
                .active_file
                .and_then(|index| self.open_files.get(index))
                .map(|file| file.id.to_string())
                .unwrap_or_default(),
            script_args: self.script_args.clone(),
            watches: self.watches.clone(),
            dock_state: serde_json::to_string(&dock.dump(cx)).unwrap_or_default(),
            editor: settings.editor.clone(),
            output: settings.output.clone(),
            // 注意：只存接口地址和模型，API Key 由凭据管理器单独保存。
            ai: settings.ai.clone(),
            window_bounds: self.window_bounds,
            // 这次打开过设置窗口就用新的尺寸，否则沿用文件里记住的值。
            settings_size: crate::settings::remembered_window_size(cx).or(self.settings_size),
        };
        // 写盘失败不影响本次使用，下次启动会退回默认布局。
        let _ = layout.save();
    }

    /// 文件是否有未保存的修改。
    pub(super) fn file_is_dirty(&self, index: usize) -> bool {
        match (self.contents.get(index), self.saved.get(index)) {
            (Some(current), Some(saved)) => current != saved,
            _ => false,
        }
    }

    pub(super) fn any_dirty(&self) -> bool {
        (0..self.contents.len()).any(|index| self.file_is_dirty(index))
    }

    pub(super) fn dirty_count(&self) -> usize {
        (0..self.contents.len())
            .filter(|&index| self.file_is_dirty(index))
            .count()
    }

    /// 记下窗口当前的位置与大小；变化时标记为待写盘。
    pub(super) fn remember_window_bounds(&mut self, window: &Window, _cx: &mut Context<Self>) {
        let bounds = window.bounds();
        let next = (
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        );
        if self.window_bounds != Some(next) {
            self.window_bounds = Some(next);
            self.window_bounds_dirty = true;
        }
    }

    /// 保存所有有改动的文件。
    pub(super) fn save_all_dirty(&mut self, cx: &mut Context<Self>) {
        let mut saved_any = false;
        for index in 0..self.open_files.len() {
            if !self.file_is_dirty(index) {
                continue;
            }
            let id = self.open_files[index].id.to_string();
            match self.write_content(index, Path::new(&id)) {
                Ok(()) => saved_any = true,
                Err(error) => {
                    let output = self.output();
                    output.error(cx, format!("保存 {} 失败：{error}", self.open_files[index].name));
                }
            }
        }
        if saved_any {
            let output = self.output();
            output.success(cx, "已保存全部文件");
        }
        self.sync_editor_panel(cx);
    }

    /// 关闭标签（有未保存修改时先确认）。
    pub(crate) fn close_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.open_files.len() {
            return;
        }
        self.save_active_content(cx);
        if !self.file_is_dirty(index) {
            self.close_file_now(index, window, cx);
            return;
        }

        let name = self.open_files[index].name.to_string();
        let handle = cx.weak_entity();
        let save_handle = handle.clone();
        let on_save: DialogCallback = Rc::new(move |window, cx| {
            let _ = save_handle.update(cx, |this, cx| {
                this.save_file(cx);
                if !this.file_is_dirty(index) {
                    this.close_file_now(index, window, cx);
                }
            });
        });
        let discard_handle = handle;
        let on_discard: DialogCallback = Rc::new(move |window, cx| {
            let _ = discard_handle.update(cx, |this, cx| this.close_file_now(index, window, cx));
        });
        show_unsaved_dialog(
            window,
            cx,
            format!("\"{name}\" 有未保存的修改。"),
            on_save,
            on_discard,
        );
    }

    /// 直接关闭标签（不再检查未保存修改）；关掉当前标签后聚焦相邻文件。
    pub(super) fn close_file_now(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.open_files.len() {
            return;
        }

        self.save_active_content(cx);
        self.remember_closed(self.open_files[index].id.to_string());
        self.open_files.remove(index);
        self.contents.remove(index);
        self.saved.remove(index);
        self.disk_stamps.remove(index);
        self.editors.remove(index);
        drop(self.editor_subscriptions.remove(index));
        self.breakpoint_marks.remove(index);
        self.paused_line_marks.remove(index);
        self.bracket_marks.remove(index);
        self.bracket_pairs.remove(index);

        if let Some(active) = self.active_file {
            if index < active {
                self.active_file = Some(active - 1);
            } else if index == active {
                if self.open_files.is_empty() {
                    self.active_file = None;
                } else {
                    let next = index.min(self.open_files.len() - 1);
                    self.active_file = Some(next);
                    self.editor = self.editors[next].clone();
                    self.editor.update(cx, |editor, cx| editor.focus(window, cx));
                }
            }
        }

        if self.active_file.is_none() {
            // 没有标签时回到空编辑器，避免继续显示已关闭文件的内容。
            self.editor = self.empty_editor.clone();
            self.editor.update(cx, |editor, cx| {
                editor.set_value("", window, cx);
                editor.focus(window, cx);
            });
        }

        // 这些缓存属于「当前标签」，切换后要按新标签重新计算。
        self.reset_editor_caches();
        if self.active_file.is_some() {
            self.refresh_outline(cx);
            self.refresh_diagnostics(cx);
        }
        self.refresh_editor_language(cx);
        self.refresh_breakpoint_marks(cx);
        self.sync_editor_panel(cx);
        cx.notify();
    }

    /// 关闭所有标签（有未保存修改时先确认）。
    pub(crate) fn close_all_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        if !self.any_dirty() {
            self.close_all_files_now(window, cx);
            return;
        }

        let count = self.dirty_count();
        let handle = cx.weak_entity();
        let save_handle = handle.clone();
        let on_save: DialogCallback = Rc::new(move |window, cx| {
            let _ = save_handle.update(cx, |this, cx| {
                this.save_all_dirty(cx);
                if !this.any_dirty() {
                    this.close_all_files_now(window, cx);
                }
            });
        });
        let discard_handle = handle;
        let on_discard: DialogCallback = Rc::new(move |window, cx| {
            let _ =
                discard_handle.update(cx, |this, cx| this.close_all_files_now(window, cx));
        });
        show_unsaved_dialog(
            window,
            cx,
            format!("有 {count} 个文件有未保存的修改。"),
            on_save,
            on_discard,
        );
    }

    /// 直接关闭所有标签并清空编辑框。
    pub(super) fn close_all_files_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let closed: Vec<String> = self
            .open_files
            .iter()
            .map(|file| file.id.to_string())
            .collect();
        for path in closed {
            self.remember_closed(path);
        }
        self.open_files.clear();
        self.contents.clear();
        self.saved.clear();
        self.disk_stamps.clear();
        self.editors.clear();
        self.editor_subscriptions.clear();
        self.breakpoint_marks.clear();
        self.paused_line_marks.clear();
        self.bracket_marks.clear();
        self.bracket_pairs.clear();
        self.active_file = None;
        self.editor = self.empty_editor.clone();
        self.editor.update(cx, |editor, cx| {
            editor.set_value("", window, cx);
            editor.focus(window, cx);
        });
        self.reset_editor_caches();
        self.refresh_outline(cx);
        self.refresh_editor_language(cx);
        self.sync_editor_panel(cx);
        cx.notify();
    }

    /// 记住刚关闭的标签，供「重新打开关闭的标签」使用（后进先出，最多 20 个）。
    pub(super) fn remember_closed(&mut self, path: String) {
        self.closed_files.retain(|closed| closed != &path);
        self.closed_files.push(path);
        let limit = 20;
        if self.closed_files.len() > limit {
            let excess = self.closed_files.len() - limit;
            self.closed_files.drain(..excess);
        }
    }

    /// 重新打开最近关闭的标签（`Ctrl+Shift+T`）。
    pub(crate) fn reopen_closed_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(path) = self.closed_files.pop() {
            if Path::new(&path).is_file() {
                self.open_file(&path, window, cx);
                return;
            }
            // 磁盘上已经没有了：跳过这一条，接着看更早关闭的标签。
        }
        self.output().warning(cx, "没有可重新打开的标签");
    }

    /// 检查已打开文件在磁盘上是否被外部改过。
    ///
    /// 没有未保存修改时直接重新加载；有修改时弹确认框，由用户决定重新加载还是保留。
    pub(super) fn check_external_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 已经有对话框在前台时先不打扰，下一轮再检查。
        if window.has_active_dialog(cx) {
            return;
        }
        self.save_active_content(cx);
        for index in 0..self.open_files.len() {
            let path = PathBuf::from(self.open_files[index].id.to_string());
            let current = workspace::disk_stamp(&path);
            let known = self.disk_stamps.get(index).copied().flatten();
            if !workspace::stamp_differs(known, current) {
                continue;
            }
            // 先记下新时间戳：同一处改动只处理一次，也避免反复弹框。
            if let Some(slot) = self.disk_stamps.get_mut(index) {
                *slot = current;
            }
            if self.file_is_dirty(index) {
                self.prompt_external_reload(index, window, cx);
                return;
            }
            let name = self.open_files[index].name.to_string();
            self.reload_from_disk(index, window, cx);
            self.output()
                .command(cx, format!("> {name} 在磁盘上被修改，已重新加载"));
        }
    }

    /// 把磁盘上的新内容读回标签（丢掉未保存内容时也走这里）。
    pub(super) fn reload_from_disk(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.open_files.len() {
            return;
        }
        let path = PathBuf::from(self.open_files[index].id.to_string());
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let decoded = encoding::decode(&bytes);
        let content: SharedString = decoded.text.into();
        if let Some(file) = self.open_files.get_mut(index) {
            file.encoding = decoded.encoding;
        }
        if let Some(slot) = self.disk_stamps.get_mut(index) {
            *slot = workspace::disk_stamp(&path);
        }
        if let Some(slot) = self.contents.get_mut(index) {
            *slot = content.clone();
        }
        if let Some(slot) = self.saved.get_mut(index) {
            *slot = content.clone();
        }
        if let Some(editor) = self.editors.get(index).cloned() {
            editor.update(cx, |editor, cx| editor.set_value(content.to_string(), window, cx));
        }
        if self.active_file == Some(index) {
            self.reset_editor_caches();
            self.refresh_outline(cx);
            self.refresh_diagnostics(cx);
            self.refresh_breakpoint_marks(cx);
            self.refresh_editor_language(cx);
        }
        self.sync_editor_panel(cx);
        cx.notify();
    }

    /// 有未保存修改时问用户：保留我的内容，还是用磁盘上的版本覆盖。
    pub(super) fn prompt_external_reload(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.open_files[index].name.to_string();
        let handle = cx.entity().downgrade();
        let on_reload: DialogCallback = Rc::new(move |window, cx| {
            let _ = handle.update(cx, |this, cx| this.reload_from_disk(index, window, cx));
        });
        show_confirm_dialog(
            window,
            cx,
            "文件已被外部修改",
            format!("\"{name}\" 在磁盘上被修改过，同时还有未保存的修改。重新加载会丢掉这些修改。"),
            "重新加载",
            on_reload,
        );
    }

    /// 项目内「全部替换」之后：刷新受影响的打开标签，并把结果写进输出。
    ///
    /// 有未保存修改的标签不动（磁盘上的新内容会和用户的编辑冲突，留给外部修改检测去问）。
    pub(crate) fn after_replace(
        &mut self,
        outcome: &search::ReplaceOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for index in 0..self.open_files.len() {
            let id = self.open_files[index].id.to_string();
            let changed = outcome.changed.iter().any(|(path, _)| path == &id);
            if changed && !self.file_is_dirty(index) {
                self.reload_from_disk(index, window, cx);
            }
        }
        let files = outcome.changed.len();
        let total = outcome.replacements;
        if files == 0 {
            self.output().warning(cx, "没有替换任何内容");
        } else {
            self.output()
                .success(cx, format!("已替换 {files} 个文件、共 {total} 处"));
        }
        if outcome.skipped > 0 {
            self.output().warning(
                cx,
                format!("{} 个文件无法读取，已跳过", outcome.skipped),
            );
        }
        for failure in &outcome.failures {
            let message = format!("替换失败：{failure}");
            self.output().error(cx, message);
        }
        cx.notify();
    }

    /// 退出程序（有未保存修改时先确认）。
    pub(crate) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_window_close(window, cx) {
            cx.quit();
        }
    }

    /// 窗口关闭前的确认：有未保存修改时弹窗，返回 `false` 阻止本次关闭。
    pub(super) fn request_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // 暂停中的调试线程会一直等指令，关窗口时先放它走。
        if let Some(session) = &self.debug {
            session.control.stop();
        }
        self.save_active_content(cx);
        self.save_layout(cx);
        if !self.any_dirty() {
            return true;
        }

        let count = self.dirty_count();
        let handle = cx.weak_entity();
        let on_save: DialogCallback = Rc::new(move |_window, cx| {
            let all_saved = handle
                .update(cx, |this, cx| {
                    this.save_all_dirty(cx);
                    !this.any_dirty()
                })
                .unwrap_or(false);
            if all_saved {
                cx.quit();
            }
        });
        let on_discard: DialogCallback = Rc::new(move |_window, cx| cx.quit());
        show_unsaved_dialog(
            window,
            cx,
            format!("有 {count} 个文件有未保存的修改。关闭前要保存吗？"),
            on_save,
            on_discard,
        );
        false
    }
}
