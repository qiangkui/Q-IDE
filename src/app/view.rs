//! 主窗口渲染：菜单栏、活动栏、状态栏与整体布局。

use super::*;

impl CodeWindow {
    /// 菜单栏。
    pub(super) fn menu_bar(&self, cx: &mut Context<Self>, palette: Palette) -> AnyElement {
        let settings = self.settings.read(cx).clone();
        let state = MenuState {
            show_explorer: settings.appearance.show_explorer(),
            show_outline: settings.appearance.show_outline(),
            show_search: settings.appearance.show_search(),
            show_ai: settings.appearance.show_ai(),
            show_output: settings.appearance.show_output,
            show_status_bar: settings.appearance.show_status_bar,
            has_files: !self.open_files.is_empty(),
            has_active: self.active_file.is_some(),
            dark_theme: ui_theme::resolve(&settings.appearance.theme, cx)
                .mode
                .is_dark(),
        };
        menu::render(
            cx,
            palette,
            cx.entity().downgrade(),
            state,
            &self.recent_files,
        )
    }

    /// 最左侧活动栏。
    pub(super) fn activity_bar(&self, cx: &mut Context<Self>, appearance: &AppearanceSettings) -> AnyElement {
        // 选中态用偏白的高亮盖掉组件默认的“按下变暗”。
        let selected_bg = rgba(Palette::for_theme(cx).activity_bar_selected);
        v_flex()
            .id("activity-bar")
            .w(px(ui_theme::ACTIVITY_BAR_WIDTH))
            // 与面板一样向内缩 PANEL_GAP/2，上下沿才能和右侧面板对齐。
            .my(px(ui_theme::PANEL_GAP / 2.0))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .py(px(6.0))
            .bg(rgb(Palette::for_theme(cx).activity_bar_bg))
            .rounded(px(ui_theme::PANE_RADIUS))
            .border_1()
            .border_color(rgba(Palette::for_theme(cx).border))
            .overflow_hidden()
            // 把面板拖到活动栏上，就把它放回左侧 dock。
            .drag_over::<DragPanel>(|this, _, _, cx| {
                this.bg(rgba(Palette::for_theme(cx).hover))
            })
            .on_drop(cx.listener(|this, drag: &DragPanel, window, cx| {
                this.panel_drag = None;
                this.move_panel_to_slot(drag.panel(), SideSlot::Left, window, cx);
                cx.notify();
            }))
            .child(
                Button::new("toggle-explorer")
                    .ghost()
                    .icon(IconName::PanelLeft)
                    .selected(appearance.show_explorer())
                    .when(appearance.show_explorer(), |button| {
                        button.bg(selected_bg)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_panel(PanelKind::Explorer, window, cx)
                    })),
            )
            .child(
                Button::new("toggle-outline")
                    .ghost()
                    .icon(IconName::FileText)
                    .selected(appearance.show_outline())
                    .when(appearance.show_outline(), |button| {
                        button.bg(selected_bg)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_panel(PanelKind::Outline, window, cx)
                    })),
            )
            .child(
                Button::new("toggle-ai-panel")
                    .ghost()
                    .icon(IconName::Bot)
                    .tooltip("AI 编程")
                    .selected(appearance.show_ai())
                    .when(appearance.show_ai(), |button| button.bg(selected_bg))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_ai_panel(window, cx)
                    })),
            )
            .child(
                Button::new("toggle-search-panel")
                    .ghost()
                    .icon(IconName::Search)
                    .selected(appearance.show_search())
                    .when(appearance.show_search(), |button| {
                        button.bg(selected_bg)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_search_panel(window, cx)
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .on_click(cx.listener(|_, _, _, cx| crate::settings::open_settings_window(cx))),
            )
            .into_any_element()
    }

    /// 状态栏。
    pub(super) fn status_bar(
        &self,
        cx: &mut Context<Self>,
        palette: Palette,
        settings: &AppSettings,
    ) -> AnyElement {
        let (line_count, char_count) = self
            .active_file
            .and_then(|index| self.contents.get(index))
            .map(|content| (content.lines().count().max(1), content.chars().count()))
            .unwrap_or((0, 0));
        let file_name = self
            .active_file
            .and_then(|index| self.open_files.get(index))
            .map(|file| file.name.to_string());
        let language = workspace::language_label(&self.effective_language(settings));
        let file_encoding = self
            .active_file
            .and_then(|index| self.open_files.get(index))
            .map(|file| file.encoding.label());
        let output_lines = self.output.read(cx).len();
        let cursor = if self.active_file.is_some() {
            let editor = self.editor.read(cx);
            let text = editor.text();
            let offset = editor.cursor().min(text.len());
            let position = text.offset_to_position(offset);
            let line = position.line as usize + 1;
            let line_start = text.position_to_offset(&Position::new(position.line, 0));
            let column = text.slice(line_start..offset).chars().count() + 1;
            Some((line, column))
        } else {
            None
        };

        let on_toggle_output = cx.listener(|_, _, _, cx| {
            SettingsStore::update(cx, |settings| {
                settings.appearance.show_output = !settings.appearance.show_output;
            })
        });
        let debug_state = self.debug.as_ref().map(|session| {
            if session.paused {
                match &session.snapshot {
                    Some(snapshot) => {
                        let stack = call_stack_label(snapshot);
                        if stack.is_empty() {
                            format!("调试：已暂停 · 第 {} 行", snapshot.line)
                        } else {
                            format!("调试：已暂停 · 第 {} 行 · {stack}", snapshot.line)
                        }
                    }
                    None => "调试：已暂停".to_string(),
                }
            } else {
                "调试：运行中".to_string()
            }
        });

        status_bar::render(
            palette,
            StatusInfo {
                file_name: file_name.as_deref(),
                language,
                encoding: file_encoding,
                line_count,
                char_count,
                cursor,
                errors: self.error_count,
                output_lines,
                debug_state: debug_state.as_deref(),
            },
            on_toggle_output,
        )
    }
}

impl Render for CodeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_theme(cx);
        let settings = self.settings.read(cx).clone();
        let appearance = settings.appearance.clone();

        div()
            .relative()
            .v_flex()
            .size_full()
            .bg(rgb(palette.window_bg))
            .on_action(cx.listener(|this, _: &SaveFile, _window, cx| this.save_file(cx)))
            .on_action(cx.listener(|this, _: &SaveFileAs, window, cx| {
                this.save_file_as(window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewFile, window, cx| this.new_file(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFiles, window, cx| {
                this.open_files_dialog(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RunScript, _window, cx| this.run_script(cx)))
            .on_action(cx.listener(|this, _: &StopScript, _window, cx| this.stop_script(cx)))
            .on_action(cx.listener(|this, _: &DebugRun, _window, cx| this.start_debug(cx)))
            .on_action(cx.listener(|this, _: &DebugPause, _window, cx| this.debug_pause(cx)))
            .on_action(cx.listener(|this, _: &DebugResume, _window, cx| this.debug_resume(cx)))
            .on_action(cx.listener(|this, _: &DebugStep, _window, cx| this.debug_step(cx)))
            .on_action(cx.listener(|this, _: &GoToDefinition, window, cx| {
                this.go_to_definition(window, cx)
            }))
            .on_action(cx.listener(|this, _: &FindReferences, _, cx| {
                this.find_references(cx)
            }))
            .on_action(cx.listener(|this, _: &GoToLine, window, cx| {
                this.goto_dialog(window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToFile, window, cx| {
                this.goto_file_dialog(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSearch, window, cx| {
                this.toggle_search(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleReplace, window, cx| {
                this.toggle_search(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSearchPanel, window, cx| {
                this.toggle_search_panel(window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveLineUp, window, cx| {
                this.move_active_lines(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveLineDown, window, cx| {
                this.move_active_lines(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleComment, window, cx| {
                this.toggle_active_comment(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SaveAllFiles, _window, cx| {
                this.save_all_files(cx)
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                this.cycle_tab(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| {
                this.cycle_tab(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowCommands, window, cx| {
                this.command_palette_dialog(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ReopenClosedFile, window, cx| {
                this.reopen_closed_file(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RunToCursor, _, cx| {
                this.run_to_cursor(cx)
            }))
            .on_action(cx.listener(|this, _: &ClearBreakpoints, _, cx| {
                this.clear_all_breakpoints(cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.zoom_editor(Some(1.0), cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.zoom_editor(Some(-1.0), cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| {
                this.zoom_editor(None, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleBreakpoint, window, cx| {
                // 断点按编辑框光标所在行切换，先让编辑框拿到焦点。
                let editor = this.editor.clone();
                editor.update(cx, |editor, cx| editor.focus(window, cx));
                this.toggle_breakpoint(cx);
            }))
            .child(title_bar::render(
                palette,
                window.is_maximized(),
                self.menu_bar(cx, palette),
            ))
            .child(
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .p(px(ui_theme::PANE_GAP))
                    .child(self.activity_bar(cx, &appearance))
                    // 左面板自身还会向内缩 PANEL_GAP/2，这里的间隔只补一半，
                    // 活动栏与面板之间的可见间距才和其他面板之间一致。
                    .child(div().w(px(ui_theme::PANEL_GAP / 2.0)).flex_shrink_0())
                    .child({
                        let drag_overlay = self
                            .panel_drag
                            .map(|state| self.panel_drag_overlay(state, cx));
                        v_flex()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(self.dock_area.clone())
                            .when_some(drag_overlay, |this, overlay| this.child(overlay))
                    }),
            )
            .when(appearance.show_status_bar, |this| {
                this.child(self.status_bar(cx, palette, &settings))
            })
    }
}
