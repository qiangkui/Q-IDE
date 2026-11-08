//! Dock 面板：停靠初始化、可见性同步、拖动与左右切换。

use super::*;

impl CodeWindow {
    /// 初始化 Dock：优先恢复上次的完整面板布局，没有可用布局时创建默认布局。
    pub(super) fn setup_dock(&mut self, dock_state: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.register_dock_panels(cx);

        if !dock_state.is_empty()
            && let Ok(state) = serde_json::from_str::<DockAreaState>(dock_state)
            && self
                .dock_area
                .update(cx, |area, cx| area.load(state, window, cx))
                .is_ok()
        {
            return;
        }

        let editor = panel_handle(self.editor_panel.clone());
        let explorer = panel_handle(self.explorer_panel.clone());
        let outline = panel_handle(self.outline_panel.clone());
        let search = panel_handle(self.search_panel.clone());
        let ai = panel_handle(self.ai_panel.clone());
        let output = panel_handle(self.output_panel.clone());
        let appearance = self.settings.read(cx).appearance.clone();
        let left_kind = appearance.left_panel;
        let right_kind = appearance.right_panel;
        let left_width = left_kind
            .map(|kind| self.panel_width(kind))
            .unwrap_or(self.explorer_width);
        let right_width = right_kind
            .map(|kind| self.panel_width(kind))
            .unwrap_or(self.ai_width);
        let output_height = self.output_height;
        self.dock_area.update(cx, |area, cx| {
            area.set_center(DockLayout::tabs().panel_view(editor, cx), window, cx);
            if let Some(kind) = left_kind {
                let layout = match kind {
                    PanelKind::Explorer => DockLayout::tabs().panel_view(explorer.clone(), cx),
                    PanelKind::Outline => DockLayout::tabs().panel_view(outline.clone(), cx),
                    PanelKind::Search => DockLayout::tabs().panel_view(search.clone(), cx),
                    PanelKind::Ai => DockLayout::tabs().panel_view(ai.clone(), cx),
                };
                area.set_dock(DockPlacement::Left, layout, window, cx);
                area.set_dock_size(DockPlacement::Left, px(left_width), window, cx);
            }
            if let Some(kind) = right_kind {
                let layout = match kind {
                    PanelKind::Explorer => DockLayout::tabs().panel_view(explorer.clone(), cx),
                    PanelKind::Outline => DockLayout::tabs().panel_view(outline.clone(), cx),
                    PanelKind::Search => DockLayout::tabs().panel_view(search.clone(), cx),
                    PanelKind::Ai => DockLayout::tabs().panel_view(ai.clone(), cx),
                };
                area.set_dock(DockPlacement::Right, layout, window, cx);
                area.set_dock_size(DockPlacement::Right, px(right_width), window, cx);
            }
            area.set_dock(
                DockPlacement::Bottom,
                DockLayout::tabs().panel_view(output, cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Bottom, px(output_height), window, cx);
        });
    }

    /// 面板记住的宽度。
    pub(super) fn panel_width(&self, kind: PanelKind) -> f32 {
        match kind {
            PanelKind::Explorer => self.explorer_width,
            PanelKind::Outline => self.outline_width,
            PanelKind::Search => self.search_width,
            PanelKind::Ai => self.ai_width,
        }
    }

    /// 面板在 Dock 里的 id。
    pub(super) fn panel_id(&self, kind: PanelKind) -> PanelId {
        match kind {
            PanelKind::Explorer => PanelId::from(self.explorer_panel.entity_id()),
            PanelKind::Outline => PanelId::from(self.outline_panel.entity_id()),
            PanelKind::Search => PanelId::from(self.search_panel.entity_id()),
            PanelKind::Ai => PanelId::from(self.ai_panel.entity_id()),
        }
    }

    /// 由 Dock 面板 id 反查面板种类。
    pub(super) fn panel_kind(&self, panel: PanelId) -> Option<PanelKind> {
        PanelKind::ALL
            .into_iter()
            .find(|kind| self.panel_id(*kind) == panel)
    }

    /// 注册面板构造器，供 [`DockArea::load`] 恢复持久化布局时复用现有面板实体。
    pub(super) fn register_dock_panels(&self, cx: &mut App) {
        let editor = self.editor_panel.clone();
        register_panel(cx, crate::panels::EDITOR_PANEL, move |_, _, _| {
            panel_handle(editor.clone())
        });
        let explorer = self.explorer_panel.clone();
        register_panel(cx, crate::panels::EXPLORER_PANEL, move |_, _, _| {
            panel_handle(explorer.clone())
        });
        let outline = self.outline_panel.clone();
        register_panel(cx, crate::panels::OUTLINE_PANEL, move |_, _, _| {
            panel_handle(outline.clone())
        });
        let search = self.search_panel.clone();
        register_panel(cx, crate::panels::SEARCH_PANEL, move |_, _, _| {
            panel_handle(search.clone())
        });
        let ai = self.ai_panel.clone();
        register_panel(cx, crate::panels::AI_PANEL, move |_, _, _| {
            panel_handle(ai.clone())
        });
        let output = self.output_panel.clone();
        register_panel(cx, crate::panels::OUTPUT_PANEL, move |_, _, _| {
            panel_handle(output.clone())
        });
    }

    /// 面板的显示状态由 `Panel::visible` 读取设置；Dock 只在 reconcile 时重新计算可见面板。
    ///
    /// 左右两侧各最多放一个面板：先把不再显示的面板移出 Dock，再把缺失的面板补到
    /// 它所属的一侧，最后统一 reconcile 并收掉空 dock。
    pub(super) fn apply_dock_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let appearance = self.settings.read(cx).appearance.clone();
        let current = (
            appearance.left_panel,
            appearance.right_panel,
            appearance.show_output,
        );
        if self.panel_visibility == Some(current) {
            return;
        }
        self.panel_visibility = Some(current);

        // 记住用户拖动后的实际尺寸；隐藏面板时 dock 会被移除，重新打开时用它恢复。
        self.remember_dock_sizes(cx);

        let explorer = self.explorer_panel.clone();
        let outline = self.outline_panel.clone();
        let search = self.search_panel.clone();
        let ai = self.ai_panel.clone();
        let output = self.output_panel.clone();
        let explorer_width = self.explorer_width;
        let outline_width = self.outline_width;
        let search_width = self.search_width;
        let ai_width = self.ai_width;
        let output_height = self.output_height;
        let show_output = appearance.show_output;

        self.dock_area.update(cx, |area, cx| {
            // 1. 移除不再显示的面板，让出它原来占的那一侧。
            if appearance.slot_of(PanelKind::Explorer).is_none() {
                area.remove_panel(explorer.clone(), window, cx);
            }
            if appearance.slot_of(PanelKind::Outline).is_none() {
                area.remove_panel(outline.clone(), window, cx);
            }
            if appearance.slot_of(PanelKind::Search).is_none() {
                area.remove_panel(search.clone(), window, cx);
            }
            if appearance.slot_of(PanelKind::Ai).is_none() {
                area.remove_panel(ai.clone(), window, cx);
            }

            // 2. 缺失的可见面板补回它所属的一侧。
            for (kind, width) in [
                (PanelKind::Explorer, explorer_width),
                (PanelKind::Outline, outline_width),
                (PanelKind::Search, search_width),
                (PanelKind::Ai, ai_width),
            ] {
                let Some(slot) = appearance.slot_of(kind) else {
                    continue;
                };
                let id = match kind {
                    PanelKind::Explorer => PanelId::from(explorer.entity_id()),
                    PanelKind::Outline => PanelId::from(outline.entity_id()),
                    PanelKind::Search => PanelId::from(search.entity_id()),
                    PanelKind::Ai => PanelId::from(ai.entity_id()),
                };
                if panel_present(area, id) {
                    continue;
                }
                let handle = match kind {
                    PanelKind::Explorer => panel_handle(explorer.clone()),
                    PanelKind::Outline => panel_handle(outline.clone()),
                    PanelKind::Search => panel_handle(search.clone()),
                    PanelKind::Ai => panel_handle(ai.clone()),
                };
                area.add_panel_view(
                    handle,
                    slot_placement(slot),
                    Some(px(width)),
                    window,
                    cx,
                );
            }

            let output_id = PanelId::from(output.entity_id());
            if show_output && !panel_present(area, output_id) {
                area.add_panel_view(
                    panel_handle(output.clone()),
                    DockPlacement::Bottom,
                    Some(px(output_height)),
                    window,
                    cx,
                );
            }

            // 3. 重新计算可见性（没有公开的“只刷新可见性”接口）。
            let locked = area.is_locked();
            area.set_locked(!locked, window, cx);
            area.set_locked(locked, window, cx);

            // 4. 打开需要的 dock；没有可见面板的 dock 直接移除，编辑区占满剩余空间。
            let visible = |placement: DockPlacement| {
                area.layout(placement).is_some_and(|tree| {
                    tree.panels()
                        .any(|id| area.panel(id).is_some_and(|panel| panel.visible(cx)))
                })
            };
            let left_visible = visible(DockPlacement::Left);
            let right_visible = visible(DockPlacement::Right);
            let bottom_visible = visible(DockPlacement::Bottom);

            for (placement, is_visible) in [
                (DockPlacement::Left, left_visible),
                (DockPlacement::Right, right_visible),
            ] {
                if is_visible {
                    if area.has_dock(placement) && !area.is_dock_open(placement) {
                        area.toggle_dock(placement, window, cx);
                    }
                } else if area.has_dock(placement) {
                    area.remove_dock(placement, window, cx);
                }
            }

            if show_output && bottom_visible {
                if area.has_dock(DockPlacement::Bottom)
                    && !area.is_dock_open(DockPlacement::Bottom)
                {
                    area.toggle_dock(DockPlacement::Bottom, window, cx);
                }
            } else if !show_output && area.has_dock(DockPlacement::Bottom) {
                // 底部 dock 关闭后仍会保留 29px 的折叠条，必须直接移除。
                area.remove_dock(DockPlacement::Bottom, window, cx);
            }
        });

        // 面板可能仍停在另一侧：按设置里的槽位搬回来，并收掉空 dock。
        self.normalize_dock_docks(window, cx);
    }

    /// 从当前 dock 读取并记录侧栏宽度、输出高度。
    pub(super) fn remember_dock_sizes(&mut self, cx: &App) {
        let explorer_id = PanelId::from(self.explorer_panel.entity_id());
        let outline_id = PanelId::from(self.outline_panel.entity_id());
        let search_id = PanelId::from(self.search_panel.entity_id());
        let ai_id = PanelId::from(self.ai_panel.entity_id());
        let area = self.dock_area.read(cx);
        for placement in [DockPlacement::Left, DockPlacement::Right] {
            let Some(width) = area.dock_size(placement).map(f32::from) else {
                continue;
            };
            let Some(tree) = area.layout(placement) else {
                continue;
            };
            let panels: Vec<PanelId> = tree.panels().collect();
            if panels.contains(&explorer_id) {
                self.explorer_width = width;
            }
            if panels.contains(&outline_id) {
                self.outline_width = width;
            }
            if panels.contains(&search_id) {
                self.search_width = width;
            }
            if panels.contains(&ai_id) {
                self.ai_width = width;
            }
        }
        if let Some(height) = area.dock_size(DockPlacement::Bottom).map(f32::from) {
            self.output_height = height;
        }
    }

    /// 把被拖到错误区域的面板送回它应该在的 dock。
    pub(super) fn enforce_dock_placements(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor_id = PanelId::from(self.editor_panel.entity_id());
        let output_id = PanelId::from(self.output_panel.entity_id());
        let slots = self.settings.read(cx).appearance.clone();

        let (editor, output, side_placements) = {
            let area = self.dock_area.read(cx);
            (
                panel_placement(area, editor_id),
                panel_placement(area, output_id),
                PanelKind::ALL.map(|kind| panel_placement(area, self.panel_id(kind))),
            )
        };

        let mut moves: Vec<(PanelId, DockPlacement)> = Vec::new();
        if editor != Some(DockPlacement::Center) {
            moves.push((editor_id, DockPlacement::Center));
        }
        // 侧栏面板只能待在设置指定的那一侧；被拖到别处时搬回去。
        for (kind, placement) in PanelKind::ALL.into_iter().zip(side_placements) {
            if let Some(slot) = slots.slot_of(kind) {
                let target = slot_placement(slot);
                if placement.is_some_and(|placement| placement != target) {
                    moves.push((self.panel_id(kind), target));
                }
            }
        }
        if output.is_some() && output != Some(DockPlacement::Bottom) {
            moves.push((output_id, DockPlacement::Bottom));
        }

        for (panel, placement) in moves {
            self.dock_panel(panel, placement, window, cx);
        }
    }

    /// 某个 dock 里是否还有可见面板。
    pub(super) fn dock_has_visible_panel(&self, placement: DockPlacement, cx: &App) -> bool {
        let area = self.dock_area.read(cx);
        area.layout(placement).is_some_and(|tree| {
            tree.panels()
                .any(|id| area.panel(id).is_some_and(|panel| panel.visible(cx)))
        })
    }

    /// 拖动/合并后，被搬空的 dock 仍可能保留宽度或 29px 折叠条。
    /// 没有可见面板时移除左/右/底部 dock，让编辑区填满。
    pub(super) fn normalize_dock_docks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 编辑器、资源和输出有固定职责：编辑器必须在中间，资源和代码大纲只能
        // 在左右侧栏，输出只能在底部。拖到中间时 base 会生成一条分裂区域，
        // 那样面板会跑到输出上方，且没有 dock 的宽度调节手柄。
        self.enforce_dock_placements(window, cx);

        // 移除 dock 前记住它最后的大小，下次打开面板时沿用。
        self.remember_dock_sizes(cx);
        let left_visible = self.dock_has_visible_panel(DockPlacement::Left, cx);
        let right_visible = self.dock_has_visible_panel(DockPlacement::Right, cx);
        let bottom_visible = self.dock_has_visible_panel(DockPlacement::Bottom, cx);
        let (has_left, has_right, has_bottom) = {
            let area = self.dock_area.read(cx);
            (
                area.has_dock(DockPlacement::Left),
                area.has_dock(DockPlacement::Right),
                area.has_dock(DockPlacement::Bottom),
            )
        };

        self.dock_area.update(cx, |area, cx| {
            // 关闭的左右 dock 仍保留自己的宽度，base 计算另一侧 dock 的最大
            // 宽度时也会把它算进去，导致分割条拖到一半就拖不动。直接移除，
            // 下次打开时再用保存的宽度重建。
            if has_left && !left_visible {
                area.remove_dock(DockPlacement::Left, window, cx);
            }
            if has_right && !right_visible {
                area.remove_dock(DockPlacement::Right, window, cx);
            }
            if has_bottom && !bottom_visible {
                area.remove_dock(DockPlacement::Bottom, window, cx);
            }
        });
    }

    /// 把被拖动的面板移动到指定 dock（主要供活动栏的“拖回左侧”使用）。
    pub(super) fn dock_panel(
        &mut self,
        panel: PanelId,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remember_dock_sizes(cx);
        let explorer_id = PanelId::from(self.explorer_panel.entity_id());
        let outline_id = PanelId::from(self.outline_panel.entity_id());
        let search_id = PanelId::from(self.search_panel.entity_id());
        let ai_id = PanelId::from(self.ai_panel.entity_id());
        let output_id = PanelId::from(self.output_panel.entity_id());
        let size = match self.panel_kind(panel) {
            Some(kind) => px(self.panel_width(kind)),
            None if panel == output_id => px(self.output_height),
            None => px(self.explorer_width),
        };

        let target = {
            let area = self.dock_area.read(cx);
            area.layout(placement)
                .and_then(|tree| first_tabs_node(tree.root()))
        };
        if let Some(node) = target {
            self.dock_area.update(cx, |area, cx| {
                area.move_panel(
                    panel,
                    InsertTarget::Tabs {
                        node,
                        ix: None,
                        activate: true,
                    },
                    window,
                    cx,
                );
                if !area.is_dock_open(placement) {
                    area.toggle_dock(placement, window, cx);
                }
                area.set_dock_size(placement, size, window, cx);
            });
            return;
        }

        // 目标 dock 还没有标签组：先从原位置移除，再以现有视图加入目标 dock。
        let view = self.dock_area.read(cx).panel(panel).cloned();
        let Some(view) = view else {
            return;
        };
        let explorer = self.explorer_panel.clone();
        let outline = self.outline_panel.clone();
        let search = self.search_panel.clone();
        let ai = self.ai_panel.clone();
        let output = self.output_panel.clone();
        self.dock_area.update(cx, |area, cx| {
            if panel == explorer_id {
                area.remove_panel(explorer, window, cx);
            } else if panel == outline_id {
                area.remove_panel(outline, window, cx);
            } else if panel == search_id {
                area.remove_panel(search, window, cx);
            } else if panel == ai_id {
                area.remove_panel(ai, window, cx);
            } else if panel == output_id {
                area.remove_panel(output, window, cx);
            } else {
                return;
            }
            area.add_panel_view(view, placement, Some(size), window, cx);
            if !area.is_dock_open(placement) {
                area.toggle_dock(placement, window, cx);
            }
            area.set_dock_size(placement, size, window, cx);
        });
    }

    /// 侧栏标题开始拖动时记录状态，并在 Dock 上方显示自定义停靠区域。
    pub(crate) fn begin_panel_drag(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        self.remember_dock_sizes(cx);
        let output_id = PanelId::from(self.output_panel.entity_id());
        let placement = {
            let area = self.dock_area.read(cx);
            panel_placement(area, panel)
        };
        let target = if panel == output_id {
            DockPlacement::Bottom
        } else if placement == Some(DockPlacement::Right) {
            DockPlacement::Right
        } else {
            DockPlacement::Left
        };
        let extent = if panel == output_id {
            self.output_height
        } else {
            self.panel_kind(panel)
                .map(|kind| self.panel_width(kind))
                .unwrap_or(self.explorer_width)
        };
        self.panel_drag = Some(PanelDragState {
            panel,
            target,
            extent,
        });
        cx.notify();
        self.watch_panel_drag(cx);
    }

    /// 指针在 Dock 区域移动时更新目标侧栏，指示条始终和最终停靠位置一致。
    pub(super) fn update_panel_drag_target(
        &mut self,
        event: &DragMoveEvent<DragPanel>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.panel_drag else {
            return;
        };
        let output_id = PanelId::from(self.output_panel.entity_id());
        let (target, extent) = if state.panel == output_id {
            let height = self
                .output_height
                .min(f32::from(event.bounds.size.height) * 0.5)
                .max(80.0);
            (DockPlacement::Bottom, height)
        } else {
            let middle = event.bounds.left() + event.bounds.size.width * 0.5;
            let target = if event.event.position.x < middle {
                DockPlacement::Left
            } else {
                DockPlacement::Right
            };
            let width = self
                .panel_kind(state.panel)
                .map(|kind| self.panel_width(kind))
                .unwrap_or(self.explorer_width)
                .min(f32::from(event.bounds.size.width))
                .max(80.0);
            (target, width)
        };
        let next = PanelDragState {
            panel: state.panel,
            target,
            extent,
        };
        if next != state {
            self.panel_drag = Some(next);
            cx.notify();
        }
    }

    /// 放下被拖动的面板；目标来自自定义指示条。
    pub(super) fn finish_panel_drag(
        &mut self,
        drag: &DragPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .panel_drag
            .map(|state| state.target)
            .unwrap_or(DockPlacement::Left);
        self.panel_drag = None;
        match target {
            DockPlacement::Left => {
                self.move_panel_to_slot(drag.panel(), SideSlot::Left, window, cx)
            }
            DockPlacement::Right => {
                self.move_panel_to_slot(drag.panel(), SideSlot::Right, window, cx)
            }
            other => self.dock_panel(drag.panel(), other, window, cx),
        }
        cx.notify();
    }

    /// 把面板拖到某一侧：目标侧原来的面板关闭，被拖的面板占住这一侧。
    pub(super) fn move_panel_to_slot(
        &mut self,
        panel: PanelId,
        slot: SideSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.panel_kind(panel) else {
            return;
        };
        SettingsStore::update(cx, |settings| {
            settings.appearance.move_panel(kind, slot);
        });
        self.dock_panel(panel, slot_placement(slot), window, cx);
        // 移除被挤掉的面板，并让两侧布局与设置一致。
        self.apply_dock_visibility(window, cx);
    }

    /// 拖动结束后关闭自定义指示层；Dock 没有公开“拖动取消”事件，用轻量轮询收尾。
    pub(super) fn watch_panel_drag(&mut self, cx: &mut Context<Self>) {
        if self.panel_drag_watch {
            return;
        }
        self.panel_drag_watch = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        if this.panel_drag.is_none() {
                            this.panel_drag_watch = false;
                            return false;
                        }
                        if cx.has_active_drag() {
                            return true;
                        }
                        this.panel_drag = None;
                        this.panel_drag_watch = false;
                        cx.notify();
                        false
                    })
                    .unwrap_or_default();
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    /// Dock 上方的拖动目标层：指示条贴着最终停靠的侧栏/底栏，而不是编辑区分裂区。
    pub(super) fn panel_drag_overlay(
        &self,
        state: PanelDragState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = cx.theme().tokens.drop_target;
        let indicator = match state.target {
            DockPlacement::Left => div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(state.extent))
                .bg(color),
            DockPlacement::Right => div()
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .w(px(state.extent))
                .bg(color),
            DockPlacement::Bottom => div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(state.extent))
                .bg(color),
            DockPlacement::Center => div(),
        };

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .on_drag_move(cx.listener(
                |this, event: &DragMoveEvent<DragPanel>, _, cx| {
                    this.update_panel_drag_target(event, cx);
                },
            ))
            .on_drop(cx.listener(|this, drag: &DragPanel, window, cx| {
                cx.stop_propagation();
                this.finish_panel_drag(drag, window, cx);
            }))
            .child(indicator)
            .into_any_element()
    }

    /// 把标签栏数据同步给 Dock 里的编辑器面板。
    pub(super) fn sync_editor_panel(&mut self, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        let open_files = self.open_files.clone();
        let dirty = (0..self.open_files.len())
            .map(|index| self.file_is_dirty(index))
            .collect();
        let active = self.active_file;
        let font_size = self.settings.read(cx).editor.font_size;
        self.editor_panel.update(cx, |panel, cx| {
            panel.sync(editor, open_files, dirty, active, font_size, cx);
        });
    }

    /// 根目录数量变化后刷新资源管理器面板的空状态。
    pub(super) fn sync_explorer_panel(&mut self, cx: &mut Context<Self>) {
        let is_empty = self.explorer.roots().is_empty();
        self.explorer_panel
            .update(cx, |panel, cx| panel.set_empty(is_empty, cx));
    }

    /// 把大纲模型与调试快照同步给 Dock 里的大纲面板。
    pub(super) fn sync_outline_panel(&mut self, cx: &mut Context<Self>) {
        let model = self.outline_model.clone();
        let vb_supported = self.active_file_language().eq_ignore_ascii_case("vb");
        let debug = self.debug.as_ref().map(|session| OutlineDebugState {
            paused: session.paused,
            snapshot: session.snapshot.clone(),
        });
        let watches: Vec<WatchEntry> = self
            .watches
            .iter()
            .enumerate()
            .map(|(index, expression)| WatchEntry {
                expression: expression.clone(),
                state: self.watch_states.get(index).cloned().unwrap_or_default(),
            })
            .collect();
        let breakpoints = {
            let source = self.outline_source.as_deref().unwrap_or_default();
            let conditions = self.active_breakpoint_conditions();
            outline::breakpoint_entries(self.active_breakpoints(), &conditions, source)
        };
        self.outline_panel.update(cx, |panel, cx| {
            panel.sync(model, vb_supported, debug, breakpoints, watches, cx);
        });
    }

    /// 输出面板字号来自设置。
    pub(super) fn sync_output_panel(&mut self, cx: &mut Context<Self>) {
        let font_size = self.settings.read(cx).output.font_size as f32;
        self.output_panel
            .update(cx, |panel, cx| panel.set_font_size(font_size, cx));
    }

    pub(crate) fn close_output(&mut self, cx: &mut Context<Self>) {
        SettingsStore::update(cx, |settings| settings.appearance.show_output = false);
    }

    /// 打开或收起两侧面板：有空的一侧就放到那一侧，否则替换它偏好的那一侧。
    pub(crate) fn toggle_panel(
        &mut self,
        kind: PanelKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let show = !self.settings.read(cx).appearance.show(kind);
        SettingsStore::update(cx, |settings| settings.appearance.set_panel(kind, show));
        // 立即按新槽位整理 Dock，不必等设置观察者下一帧再刷新。
        self.apply_dock_visibility(window, cx);
        if !show {
            return;
        }
        let id = self.panel_id(kind);
        self.dock_area
            .update(cx, |area, cx| area.select_panel(id, window, cx));
        match kind {
            // 打开搜索/AI 面板就把焦点放进输入框，直接可以打字。
            PanelKind::Search => self
                .search_panel
                .update(cx, |panel, cx| panel.focus_query(window, cx)),
            PanelKind::Ai => self
                .ai_panel
                .update(cx, |panel, cx| panel.focus_input(window, cx)),
            PanelKind::Explorer | PanelKind::Outline => {}
        }
    }

    /// 打开或收起项目内搜索面板（`Ctrl+Shift+F` 与活动栏的搜索图标）。
    pub(crate) fn toggle_search_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(PanelKind::Search, window, cx);
    }

    /// 打开或收起 AI 编程面板（活动栏的机器人图标）。
    pub(crate) fn toggle_ai_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(PanelKind::Ai, window, cx);
    }

    /// 切换查找 / 替换面板（`Ctrl+F` / `Ctrl+H`）：已经是同一种模式就收起。
    pub(crate) fn toggle_search(
        &mut self,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            let (open, replace_mode) = {
                let session = editor.search_session();
                (session.open, session.replace_mode)
            };
            if open && replace_mode == replace {
                editor.close_search(cx);
            } else {
                editor.focus(window, cx);
                editor.open_search(replace, cx);
            }
        });
    }
}

/// 侧栏槽位对应的 Dock 区域。
fn slot_placement(slot: SideSlot) -> DockPlacement {
    match slot {
        SideSlot::Left => DockPlacement::Left,
        SideSlot::Right => DockPlacement::Right,
    }
}

/// 面板是否还在某个区域里。
fn panel_present(area: &DockArea, panel: PanelId) -> bool {
    [
        DockPlacement::Center,
        DockPlacement::Left,
        DockPlacement::Right,
        DockPlacement::Bottom,
    ]
    .into_iter()
    .any(|placement| {
        area.layout(placement)
            .is_some_and(|tree| tree.panels().any(|id| id == panel))
    })
}

/// 面板当前位于哪个区域。
fn panel_placement(area: &DockArea, panel: PanelId) -> Option<DockPlacement> {
    [
        DockPlacement::Center,
        DockPlacement::Left,
        DockPlacement::Right,
        DockPlacement::Bottom,
    ]
    .into_iter()
    .find(|placement| {
        area.layout(*placement)
            .is_some_and(|tree| tree.panels().any(|id| id == panel))
    })
}

/// 找到 Dock 布局树里的第一个标签组。
fn first_tabs_node(node: &PaneNode) -> Option<NodeId> {
    match node.kind() {
        PaneRef::Tabs { .. } => Some(node.id()),
        PaneRef::Split { children, .. } => children.iter().find_map(first_tabs_node),
    }
}
