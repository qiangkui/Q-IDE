//! Dock 面板：编辑器、资源管理器、代码大纲与输出面板。
//!
//! 主窗口只负责活动栏、标题栏、状态栏和 DockArea；各面板的内容与交互都由这里的实体实现。

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use ::gpui::Action;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::*;
use gpui_kit::component::dock::{
    BasePanel, DragPanel, Panel as DockPanel, PanelEvent, PanelId, TabGroup,
};
use gpui_kit::component::input::{
    Copy, Cut, EditorState, Enter, Escape, Indent, Input, InputState, Outdent, Paste, Redo,
    SelectAll, Undo,
};
use gpui_kit::component::tree::TreeState;
use gpui_kit::*;

use crate::app::CodeWindow;
use crate::editor_pane::{
    self, ContextMenuItem, EditorContextMenu, EditorPaneProps, MenuCallback,
    OpenContextMenuAction, TabAction, TabMenuAction,
};
use crate::explorer::{self, ExplorerActions};
use crate::interpreter::debug::DebugSnapshot;
use crate::outline::{self, BreakpointEntry, OutlineModel, OutlineSection, WatchEntry};
use crate::output::{self, OutputLog, OutputPanelProps};
use crate::search::{self, SearchProgress, SearchQuery, SearchResults};
use crate::settings::SettingsStore;
use crate::theme::{self as ui_theme, Palette};
use crate::workspace::OpenFile;

/// 面板名会写进 Dock 的持久化布局，确定后不要修改。
pub const EDITOR_PANEL: &str = "editor";
pub const EXPLORER_PANEL: &str = "explorer";
pub const OUTLINE_PANEL: &str = "outline";
pub const SEARCH_PANEL: &str = "search";
/// AI 编程面板：默认停在右侧，也可以拖到左侧。
pub const AI_PANEL: &str = "ai";
pub const OUTPUT_PANEL: &str = "output";

/// 编辑器面板：文件标签栏 + 代码编辑框。
pub struct EditorPanel {
    app: WeakEntity<CodeWindow>,
    editor: Entity<EditorState>,
    open_files: Vec<OpenFile>,
    dirty: Vec<bool>,
    active: Option<usize>,
    font_size: f32,
    /// 右键菜单的弹出位置（窗口坐标）；`None` 表示没有打开。
    context_menu: Option<Point<Pixels>>,
    /// 标签右键菜单：标签下标与鼠标位置。
    tab_menu: Option<(usize, Point<Pixels>)>,
    on_activate: TabAction,
    on_close: TabAction,
}

impl EditorPanel {
    pub fn new(
        app: WeakEntity<CodeWindow>,
        editor: Entity<EditorState>,
        font_size: f32,
    ) -> Self {
        let on_activate = {
            let app = app.clone();
            Rc::new(move |index: usize, window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.activate_file(index, window, cx));
            }) as TabAction
        };
        let on_close = {
            let app = app.clone();
            Rc::new(move |index: usize, window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.close_file(index, window, cx));
            }) as TabAction
        };

        Self {
            app,
            editor,
            open_files: Vec::new(),
            dirty: Vec::new(),
            active: None,
            font_size,
            context_menu: None,
            tab_menu: None,
            on_activate,
            on_close,
        }
    }

    pub fn sync(
        &mut self,
        editor: Entity<EditorState>,
        open_files: Vec<OpenFile>,
        dirty: Vec<bool>,
        active: Option<usize>,
        font_size: f32,
        cx: &mut Context<Self>,
    ) {
        self.editor = editor;
        self.open_files = open_files;
        self.dirty = dirty;
        self.active = active;
        self.font_size = font_size;
        // 换标签/开关文件时菜单已经没有意义。
        self.context_menu = None;
        self.tab_menu = None;
        cx.notify();
    }

    /// 打开或关闭右键菜单。
    fn set_context_menu(&mut self, position: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        self.context_menu = position;
        if position.is_some() {
            self.tab_menu = None;
        }
        cx.notify();
    }

    /// 打开或关闭标签右键菜单；两个菜单同一时间只会显示一个。
    fn set_tab_menu(&mut self, menu: Option<(usize, Point<Pixels>)>, cx: &mut Context<Self>) {
        self.tab_menu = menu;
        if menu.is_some() {
            self.context_menu = None;
        }
        cx.notify();
    }

    /// 标签右键菜单：关闭相关的几个命令 + 路径与文件操作。
    fn build_tab_menu(
        &self,
        index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> EditorContextMenu {
        let panel = cx.entity().downgrade();
        let dismiss: MenuCallback = Rc::new(move |_window, cx| {
            let _ = panel.update(cx, |panel, cx| panel.set_tab_menu(None, cx));
        });
        let app = self.app.clone();
        let close = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.close_file(index, window, cx));
            }) as MenuCallback
        };
        let close_others = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.close_other_files(index, window, cx));
            }) as MenuCallback
        };
        let close_right = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.close_files_to_right(index, window, cx));
            }) as MenuCallback
        };
        let close_all = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.close_all_files(window, cx));
            }) as MenuCallback
        };
        let copy_path = {
            let app = app.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.copy_file_path(index, cx));
            }) as MenuCallback
        };
        let save_all = {
            let app = app.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.save_all_files(cx));
            }) as MenuCallback
        };

        EditorContextMenu {
            position,
            dismiss,
            items: vec![
                ContextMenuItem {
                    label: "关闭",
                    separator_before: false,
                    action: close,
                },
                ContextMenuItem {
                    label: "关闭其它",
                    separator_before: false,
                    action: close_others,
                },
                ContextMenuItem {
                    label: "关闭右侧",
                    separator_before: false,
                    action: close_right,
                },
                ContextMenuItem {
                    label: "关闭全部",
                    separator_before: false,
                    action: close_all,
                },
                ContextMenuItem {
                    label: "全部保存（Ctrl+Alt+S）",
                    separator_before: true,
                    action: save_all,
                },
                ContextMenuItem {
                    label: "复制路径",
                    separator_before: false,
                    action: copy_path,
                },
            ],
        }
    }

    /// 拼出当前右键菜单的内容；菜单没打开时返回 `None`。
    fn build_context_menu(&self, cx: &mut Context<Self>) -> Option<EditorContextMenu> {
        if let Some((index, position)) = self.tab_menu {
            return Some(self.build_tab_menu(index, position, cx));
        }
        let position = self.context_menu?;
        let panel = cx.entity().downgrade();
        let dismiss: MenuCallback = Rc::new(move |_window, cx| {
            let _ = panel.update(cx, |panel, cx| panel.set_context_menu(None, cx));
        });

        let editor = self.editor.clone();
        let app = self.app.clone();

        // 编辑命令靠动作派发；先把焦点还给编辑框，动作才会落到输入框上。
        let edit = |make: fn() -> Box<dyn Action>| -> MenuCallback {
            let editor = editor.clone();
            Rc::new(move |window, cx| {
                editor.update(cx, |editor, cx| editor.focus(window, cx));
                window.dispatch_action(make(), cx);
            })
        };
        let toggle_breakpoint = {
            let app = app.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.toggle_breakpoint(cx));
            }) as MenuCallback
        };
        let set_condition = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.set_breakpoint_condition_dialog(window, cx));
            }) as MenuCallback
        };
        let run_to_cursor = {
            let app = app.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.run_to_cursor(cx));
            }) as MenuCallback
        };
        let go_to_definition = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.go_to_definition(window, cx));
            }) as MenuCallback
        };
        let find_references = {
            let app = app.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.find_references(cx));
            }) as MenuCallback
        };
        let move_up = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.move_active_lines(true, window, cx));
            }) as MenuCallback
        };
        let move_down = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.move_active_lines(false, window, cx));
            }) as MenuCallback
        };
        let toggle_comment = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.toggle_active_comment(window, cx));
            }) as MenuCallback
        };

        Some(EditorContextMenu {
            position,
            dismiss,
            items: vec![
                ContextMenuItem {
                    label: "撤销",
                    separator_before: false,
                    action: edit(|| Box::new(Undo)),
                },
                ContextMenuItem {
                    label: "重做",
                    separator_before: false,
                    action: edit(|| Box::new(Redo)),
                },
                ContextMenuItem {
                    label: "剪切",
                    separator_before: true,
                    action: edit(|| Box::new(Cut)),
                },
                ContextMenuItem {
                    label: "复制",
                    separator_before: false,
                    action: edit(|| Box::new(Copy)),
                },
                ContextMenuItem {
                    label: "粘贴",
                    separator_before: false,
                    action: edit(|| Box::new(Paste)),
                },
                ContextMenuItem {
                    label: "全选",
                    separator_before: false,
                    action: edit(|| Box::new(SelectAll)),
                },
                ContextMenuItem {
                    label: "上移行（Alt+↑）",
                    separator_before: true,
                    action: move_up,
                },
                ContextMenuItem {
                    label: "下移行（Alt+↓）",
                    separator_before: false,
                    action: move_down,
                },
                ContextMenuItem {
                    label: "增加缩进（Tab）",
                    separator_before: false,
                    action: edit(|| Box::new(Indent)),
                },
                ContextMenuItem {
                    label: "减少缩进（Shift+Tab）",
                    separator_before: false,
                    action: edit(|| Box::new(Outdent)),
                },
                ContextMenuItem {
                    label: "切换注释（Ctrl+/）",
                    separator_before: false,
                    action: toggle_comment,
                },
                ContextMenuItem {
                    label: "切换断点（F9）",
                    separator_before: true,
                    action: toggle_breakpoint,
                },
                ContextMenuItem {
                    label: "设置条件断点…",
                    separator_before: false,
                    action: set_condition,
                },
                ContextMenuItem {
                    label: "运行到光标处（Ctrl+F10）",
                    separator_before: false,
                    action: run_to_cursor,
                },
                ContextMenuItem {
                    label: "转到定义（F12）",
                    separator_before: true,
                    action: go_to_definition,
                },
                ContextMenuItem {
                    label: "查找引用（Shift+F12）",
                    separator_before: false,
                    action: find_references,
                },
            ],
        })
    }
}

impl BasePanel for EditorPanel {
    fn panel_name(&self) -> &'static str {
        EDITOR_PANEL
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl DockPanel for EditorPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some("编辑器".into())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "编辑器"
    }

    fn title_bar(&self, _: &App) -> bool {
        false
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for EditorPanel {}

impl Focusable for EditorPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.read(cx).focus_handle(cx)
    }
}

impl Render for EditorPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity().downgrade();
        let on_context_menu: OpenContextMenuAction = Rc::new(move |position, _window, cx| {
            let _ = panel.update(cx, |panel, cx| panel.set_context_menu(Some(position), cx));
        });
        let panel = cx.entity().downgrade();
        let on_tab_menu: TabMenuAction = Rc::new(move |index, position, _window, cx| {
            let _ =
                panel.update(cx, |panel, cx| panel.set_tab_menu(Some((index, position)), cx));
        });
        let on_drop_file = {
            let app = self.app.clone();
            Rc::new(move |path: &str, window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.open_file(path, window, cx));
            })
        };
        let context_menu = self.build_context_menu(cx);
        div()
            .size_full()
            .p(px(ui_theme::PANEL_GAP / 2.0))
            .child(editor_pane::render(
                cx,
                EditorPaneProps {
                    editor: &self.editor,
                    open_files: &self.open_files,
                    dirty: &self.dirty,
                    active: self.active,
                    palette: Palette::for_theme(cx),
                    font_size: self.font_size,
                    context_menu,
                    on_context_menu,
                    on_tab_menu,
                    on_drop_file,
                },
                self.on_activate.clone(),
                self.on_close.clone(),
            ))
    }
}

/// 资源管理器面板。
/// 资源管理器条目右键菜单的项：`(标签, 是否在前面画分隔线)`。
///
/// 纯数据，方便单测「第一项不带分隔线、也没有连续分隔线」。
pub fn explorer_menu_entries(is_folder: bool, is_root: bool) -> Vec<(&'static str, bool)> {
    let mut entries: Vec<(&'static str, bool)> = if is_folder {
        vec![("新建文件", false), ("新建文件夹", false)]
    } else {
        vec![("打开", false)]
    };
    entries.push(("重命名…", true));
    entries.push(("删除…", false));
    entries.push(("复制路径", true));
    entries.push(("刷新", false));
    if is_root {
        entries.push(("从资源管理器移除", true));
    }
    entries
}

pub struct ExplorerPanel {
    app: WeakEntity<CodeWindow>,
    tree: Entity<TreeState>,
    is_empty: bool,
    /// 树条目右键菜单（复用编辑器面板的自绘菜单外观）。
    menu: Option<EditorContextMenu>,
    group: Option<WeakEntity<TabGroup>>,
    focus: FocusHandle,
}

impl ExplorerPanel {
    pub fn new(
        app: WeakEntity<CodeWindow>,
        tree: Entity<TreeState>,
        is_empty: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            app,
            tree,
            is_empty,
            menu: None,
            group: None,
            focus: cx.focus_handle(),
        }
    }

    fn set_menu(&mut self, menu: Option<EditorContextMenu>, cx: &mut Context<Self>) {
        self.menu = menu;
        cx.notify();
    }

    /// 树条目右键菜单：文件和文件夹各一组命令。
    fn build_menu(
        &self,
        path: String,
        is_folder: bool,
        is_root: bool,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> EditorContextMenu {
        let panel = cx.entity().downgrade();
        let dismiss: MenuCallback = Rc::new(move |_window, cx| {
            let _ = panel.update(cx, |panel, cx| panel.set_menu(None, cx));
        });
        let app = self.app.clone();

        // 文件才有「打开」，文件夹把新建命令放在最上面。
        let open: Option<MenuCallback> = (!is_folder).then(|| {
            let app = app.clone();
            let path = path.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let path = path.clone();
                let _ = app.update(cx, |this, cx| this.open_file(&path, window, cx));
            }) as MenuCallback
        });
        let dir = PathBuf::from(&path);
        let new_file: Option<MenuCallback> = is_folder.then(|| {
            let app = app.clone();
            let dir = dir.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let dir = dir.clone();
                let _ = app.update(cx, |this, cx| this.create_file_in(dir, window, cx));
            }) as MenuCallback
        });
        let new_folder: Option<MenuCallback> = is_folder.then(|| {
            let app = app.clone();
            let dir = dir.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let dir = dir.clone();
                let _ = app.update(cx, |this, cx| this.create_folder_in(dir, cx));
            }) as MenuCallback
        });

        let rename = {
            let app = app.clone();
            let path = path.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let path = path.clone();
                let _ = app.update(cx, |this, cx| this.rename_path_dialog(path, window, cx));
            }) as MenuCallback
        };
        let delete = {
            let app = app.clone();
            let path = path.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let path = path.clone();
                let _ = app.update(cx, |this, cx| this.delete_path_dialog(path, is_folder, window, cx));
            }) as MenuCallback
        };
        let copy_path = {
            let app = app.clone();
            let path = path.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let path = path.clone();
                let _ = app.update(cx, |this, cx| this.copy_path_text(&path, cx));
            }) as MenuCallback
        };
        let refresh = {
            let app = app.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.refresh_explorer(cx));
            }) as MenuCallback
        };
        let remove_root = {
            let app = app.clone();
            let path = path.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let path = path.clone();
                let _ = app.update(cx, |this, cx| this.remove_root(&path, cx));
            }) as MenuCallback
        };

        // 菜单项与分隔线来自纯函数 `explorer_menu_entries`，这里只负责挂动作。
        let items: Vec<ContextMenuItem> = explorer_menu_entries(is_folder, is_root)
            .into_iter()
            .map(|(label, separator_before)| {
                let action: MenuCallback = match label {
                    "打开" => open.clone().expect("文件菜单要有「打开」"),
                    "新建文件" => new_file.clone().expect("文件夹菜单要有「新建文件」"),
                    "新建文件夹" => new_folder.clone().expect("文件夹菜单要有「新建文件夹」"),
                    "重命名…" => rename.clone(),
                    "删除…" => delete.clone(),
                    "复制路径" => copy_path.clone(),
                    "刷新" => refresh.clone(),
                    "从资源管理器移除" => remove_root.clone(),
                    _ => unreachable!("未定义的菜单项：{label}"),
                };
                ContextMenuItem {
                    label,
                    separator_before,
                    action,
                }
            })
            .collect();

        EditorContextMenu {
            position,
            items,
            dismiss,
        }
    }

    pub fn set_empty(&mut self, is_empty: bool, cx: &mut Context<Self>) {
        if self.is_empty != is_empty {
            self.is_empty = is_empty;
            cx.notify();
        }
    }
}

impl BasePanel for ExplorerPanel {
    fn panel_name(&self) -> &'static str {
        EXPLORER_PANEL
    }

    fn visible(&self, cx: &App) -> bool {
        SettingsStore::snapshot(cx).appearance.show_explorer()
    }

    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.group = Some(group);
    }

    fn on_removed(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.group = None;
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl DockPanel for ExplorerPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some("资源管理器".into())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "资源管理器"
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        let app = self.app.clone();
        let add = {
            Button::new("explorer-add-folder")
                .ghost()
                .xsmall()
                .icon(IconName::Plus)
                .on_click(move |_, window, cx| {
                    let _ = app.update(cx, |this, cx| this.add_folder_dialog(window, cx));
                })
        };
        let close = Button::new("explorer-close")
            .ghost()
            .xsmall()
            .icon(IconName::PanelLeftClose)
            .on_click(move |_, _, cx| {
                SettingsStore::update(cx, |settings| settings.appearance.set_explorer(false));
            });

        Some(vec![add, close])
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for ExplorerPanel {}

impl Focusable for ExplorerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ExplorerPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.clone();
        let add_folder = {
            let app = app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.add_folder_dialog(window, cx));
            }) as explorer::AddFolderAction
        };
        let remove_root = {
            let app = app.clone();
            Rc::new(move |id: &str, _window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.remove_root(id, cx));
            }) as explorer::RemoveRootAction
        };
        let close_panel = Rc::new(move |_: &mut Window, cx: &mut App| {
            SettingsStore::update(cx, |settings| settings.appearance.set_explorer(false));
        }) as explorer::ClosePanelAction;
        let new_file = {
            let app = self.app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.new_file(window, cx));
            }) as explorer::PanelCommand
        };
        let new_folder = {
            let app = self.app.clone();
            Rc::new(move |_window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.new_folder(cx));
            }) as explorer::PanelCommand
        };
        let refresh = {
            let app = self.app.clone();
            Rc::new(move |_window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.refresh_explorer(cx));
            }) as explorer::PanelCommand
        };
        let panel = cx.entity().downgrade();
        let on_context_menu: explorer::ExplorerMenuAction =
            Rc::new(move |path, is_folder, is_root, position, _window, cx| {
                let _ = panel.update(cx, |panel, cx| {
                    let menu = panel.build_menu(path, is_folder, is_root, position, cx);
                    panel.set_menu(Some(menu), cx);
                });
            });
        let open_file: explorer::OpenFileAction = {
            let app = self.app.clone();
            Rc::new(move |path: &str, window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.open_file(path, window, cx));
            })
        };
        let toggle_folder: explorer::ToggleFolderAction = {
            let app = self.app.clone();
            Rc::new(move |path: &str, _window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.toggle_folder(path, cx));
            })
        };
        let on_move: explorer::ExplorerMoveAction = {
            let app = self.app.clone();
            Rc::new(move |source, target, _window, cx| {
                let _ = app.update(cx, |this, cx| this.move_path(source, target, cx));
            })
        };
        let menu = self.menu.clone();

        let panel_id = PanelId::from(cx.entity().entity_id());
        let drag = self.group.as_ref().and_then(|group| group.upgrade()).map(|group| {
            DragPanel::new(panel_id, group.read(cx).node())
        });
        let drag_start = {
            let app = self.app.clone();
            Some(Rc::new(move |cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.begin_panel_drag(panel_id, cx));
            }) as explorer::DragStartAction)
        };

        let panel_element = div()
            .size_full()
            .p(px(ui_theme::PANEL_GAP / 2.0))
            .child(explorer::render(
                &self.tree,
                None,
                Palette::for_theme(cx),
                ExplorerActions {
                    open_file,
                    add_folder,
                    remove_root,
                    toggle_folder,
                    close_panel,
                    new_file,
                    new_folder,
                    refresh,
                    on_context_menu,
                    on_move,
                    is_empty: self.is_empty,
                },
                true,
                drag,
                drag_start,
            ));

        // 菜单放在带 overflow_hidden 的面板外面，靠近边缘时不会被裁掉。
        match menu {
            Some(menu) => div()
                .size_full()
                .child(panel_element)
                .child(editor_pane::context_menu_element(
                    menu,
                    Palette::for_theme(cx),
                ))
                .into_any_element(),
            None => panel_element.into_any_element(),
        }
    }
}

/// 代码大纲面板显示调试信息所需的快照。
pub struct OutlineDebugState {
    pub paused: bool,
    pub snapshot: Option<DebugSnapshot>,
}

/// 代码大纲面板。
pub struct OutlinePanel {
    app: WeakEntity<CodeWindow>,
    model: OutlineModel,
    collapsed_sections: HashSet<OutlineSection>,
    expanded_scopes: HashSet<String>,
    vb_supported: bool,
    debug: Option<OutlineDebugState>,
    breakpoints: Vec<BreakpointEntry>,
    watches: Vec<WatchEntry>,
    /// 顶部过滤框：按名字筛选符号。
    filter: Entity<InputState>,
    _filter_subscription: Subscription,
    group: Option<WeakEntity<TabGroup>>,
    focus: FocusHandle,
}

impl OutlinePanel {
    pub fn new(app: WeakEntity<CodeWindow>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("过滤符号"));
        let filter_subscription = cx.observe(&filter, |_, _, cx| cx.notify());
        Self {
            app,
            model: OutlineModel::default(),
            collapsed_sections: OutlineSection::ALL.into_iter().collect(),
            expanded_scopes: HashSet::new(),
            vb_supported: true,
            debug: None,
            breakpoints: Vec::new(),
            watches: Vec::new(),
            filter,
            _filter_subscription: filter_subscription,
            group: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn sync(
        &mut self,
        model: OutlineModel,
        vb_supported: bool,
        debug: Option<OutlineDebugState>,
        breakpoints: Vec<BreakpointEntry>,
        watches: Vec<WatchEntry>,
        cx: &mut Context<Self>,
    ) {
        // 从「没有断点」变成「有断点」时自动展开这一节，否则用户看不到刚加的行。
        if self.breakpoints.is_empty() && !breakpoints.is_empty() {
            self.collapsed_sections.remove(&OutlineSection::Breakpoints);
        }
        // 从「没有监视」变成「有监视」时自动展开这一节，否则用户看不到刚加的行。
        if self.watches.is_empty() && !watches.is_empty() {
            self.collapsed_sections.remove(&OutlineSection::Watch);
        }
        self.model = model;
        self.vb_supported = vb_supported;
        self.debug = debug;
        self.breakpoints = breakpoints;
        self.watches = watches;
        cx.notify();
    }
}

impl BasePanel for OutlinePanel {
    fn panel_name(&self) -> &'static str {
        OUTLINE_PANEL
    }

    fn visible(&self, cx: &App) -> bool {
        SettingsStore::snapshot(cx).appearance.show_outline()
    }

    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.group = Some(group);
    }

    fn on_removed(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.group = None;
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl DockPanel for OutlinePanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some("代码大纲".into())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "代码大纲"
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![
            Button::new("outline-close")
                .ghost()
                .xsmall()
                .icon(IconName::PanelLeftClose)
                .on_click(|_, _, cx| {
                    SettingsStore::update(cx, |settings| settings.appearance.set_outline(false));
                }),
        ])
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for OutlinePanel {}

impl Focusable for OutlinePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for OutlinePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.clone();
        let panel = cx.entity();
        let toggle_section: outline::ToggleSectionAction = {
            let panel = panel.clone();
            Rc::new(move |section, _window, cx| {
                panel.update(cx, |panel, cx| {
                    if !panel.collapsed_sections.remove(&section) {
                        panel.collapsed_sections.insert(section);
                    }
                    cx.notify();
                });
            })
        };
        let toggle_scope: outline::ToggleScopeAction = {
            Rc::new(move |scope: String, _window, cx| {
                panel.update(cx, |panel, cx| {
                    if !panel.expanded_scopes.remove(&scope) {
                        panel.expanded_scopes.insert(scope);
                    }
                    cx.notify();
                });
            })
        };
        let close_panel = Rc::new(move |_: &mut Window, cx: &mut App| {
            SettingsStore::update(cx, |settings| settings.appearance.set_outline(false));
        }) as outline::ClosePanelAction;
        let go_to_line = {
            Rc::new(move |line: usize, window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.go_to_line(line, window, cx));
            }) as outline::GoToLineAction
        };

        let panel_id = PanelId::from(cx.entity().entity_id());
        let drag = self.group.as_ref().and_then(|group| group.upgrade()).map(|group| {
            DragPanel::new(panel_id, group.read(cx).node())
        });
        let drag_start = {
            let app = self.app.clone();
            Some(Rc::new(move |cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.begin_panel_drag(panel_id, cx));
            }) as outline::DragStartAction)
        };
        let filter = self.filter.read(cx).value().to_string();
        let palette = Palette::for_theme(cx);
        let filter_focused = self.filter.read(cx).has_selection_focus(window, cx);
        let filter_input = self.vb_supported.then(|| {
            let mut input = Input::new(&self.filter)
                .xsmall()
                .cleanable(true)
                .bg(rgb(palette.side_panel_bg))
                // 不要组件自带的聚焦外圈：焦点只用更亮的边框表示。
                .focus_ring(false);
            if filter_focused {
                input = input.border_color(rgb(palette.sidebar_text));
            }
            div()
                .px(px(8.0))
                .pt(px(6.0))
                .pb(px(2.0))
                .bg(rgb(palette.side_panel_bg))
                .child(input)
                .into_any_element()
        });

        div()
            .size_full()
            .p(px(ui_theme::PANEL_GAP / 2.0))
            .child(outline::render(
                outline::OutlineRender {
                    width: None,
                    palette,
                    model: &self.model,
                    collapsed_sections: &self.collapsed_sections,
                    expanded_scopes: &self.expanded_scopes,
                    vb_supported: self.vb_supported,
                    debug: self.debug.as_ref().map(|debug| outline::DebugView {
                        paused: debug.paused,
                        snapshot: debug.snapshot.as_ref(),
                    }),
                    watches: &self.watches,
                    breakpoints: &self.breakpoints,
                    filter: &filter,
                    toggle_section,
                    toggle_scope,
                    close_panel,
                    go_to_line,
                },
                true,
                drag,
                drag_start,
                filter_input,
            ))
    }
}

/// 项目内搜索面板：顶部关键字输入框 + 选项 + 按文件分组的结果列表。
pub struct SearchPanel {
    app: WeakEntity<CodeWindow>,
    query: Entity<InputState>,
    _query_subscription: Subscription,
    /// 替换用的新文字。
    replace: Entity<InputState>,
    case_sensitive: bool,
    whole_word: bool,
    results: SearchResults,
    /// 上次搜索用的条件：输入框文字一变，旧结果就作废。
    searched_query: SearchQuery,
    /// 是否已经搜过：区分「还没搜」与「搜了没有结果」。
    searched: bool,
    /// 限定搜索的文件夹；`None` 表示全部候选位置。
    scope: Option<String>,
    /// 后台搜索的进度与取消标志。
    progress: Option<Arc<SearchProgress>>,
    /// 正在后台搜索。
    searching: bool,
    /// 每次搜索的序号：旧搜索的结果回来时用它丢弃。
    search_id: u64,
    /// 是否已经在轮询进度。
    progress_polling: bool,
    group: Option<WeakEntity<TabGroup>>,
    focus: FocusHandle,
}

impl SearchPanel {
    pub fn new(app: WeakEntity<CodeWindow>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("在文件中搜索"));
        let query_subscription = cx.observe(&query, |panel, _, cx| panel.on_query_changed(cx));
        let replace = cx.new(|cx| InputState::new(window, cx).placeholder("替换为"));
        Self {
            app,
            query,
            _query_subscription: query_subscription,
            replace,
            case_sensitive: false,
            whole_word: false,
            results: SearchResults::default(),
            searched_query: SearchQuery::default(),
            searched: false,
            scope: None,
            progress: None,
            searching: false,
            search_id: 0,
            progress_polling: false,
            group: None,
            focus: cx.focus_handle(),
        }
    }

    /// 把焦点交给关键字输入框（活动栏点开搜索时用）。
    pub fn focus_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |input, cx| input.focus(window, cx));
        // 面板刚加进 dock 时输入框还没上树，下一帧再确认一次焦点。
        let query = self.query.clone();
        window.on_next_frame(move |window, cx| {
            query.update(cx, |input, cx| input.focus(window, cx));
        });
    }

    fn current_query(&self, cx: &App) -> SearchQuery {
        let mut query = SearchQuery::new(self.query.read(cx).value().to_string());
        query.case_sensitive = self.case_sensitive;
        query.whole_word = self.whole_word;
        query
    }

    /// 关键字被改动后旧结果就不再对应当前输入，直接清掉，避免列表骗人。
    fn on_query_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.query.read(cx).value().to_string();
        if self.searched && text != self.searched_query.text {
            self.searched = false;
            self.results = SearchResults::default();
            // 在途的搜索也别再往界面上写结果了。
            self.invalidate_search();
        }
        cx.notify();
    }

    /// 取消在途搜索并让它的结果失效。
    fn invalidate_search(&mut self) {
        if let Some(progress) = self.progress.take() {
            progress.cancel();
        }
        self.searching = false;
        self.search_id += 1;
    }

    /// 设定（或清除）搜索范围；已经搜过就按新范围重搜。
    pub fn set_scope(&mut self, scope: Option<String>, cx: &mut Context<Self>) {
        self.scope = scope;
        if self.searched {
            self.run(cx);
        } else {
            cx.notify();
        }
    }

    /// 读输入框里的关键字，在后台线程里跑一遍搜索。
    fn run(&mut self, cx: &mut Context<Self>) {
        let query = self.current_query(cx);
        self.searched = !query.is_empty();
        self.searched_query = query.clone();
        self.results = SearchResults::default();
        // 上一轮搜索还在跑就取消它；`search_id` 自增让旧结果失效。
        self.invalidate_search();
        if query.is_empty() {
            cx.notify();
            return;
        }

        let Some(app) = self.app.upgrade() else {
            return;
        };
        let paths = app.update(cx, |this, _| this.search_candidates(self.scope.as_deref()));
        let progress = Arc::new(SearchProgress::new(paths.len()));
        self.progress = Some(progress.clone());
        self.searching = true;
        self.search_id += 1;
        let search_id = self.search_id;
        cx.notify();

        // 真正的扫描放在后台线程，界面不会因为大目录卡住。
        let task = cx.background_executor().spawn(async move {
            let progress = progress;
            search::search_files_with_progress(&paths, &query, &progress)
        });
        let panel = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let results = task.await;
            let _ = panel.update(cx, |panel, cx| {
                // 只有这次搜索仍是最新的一次时才回填结果。
                if panel.search_id == search_id {
                    panel.searching = false;
                    panel.progress = None;
                    panel.results = results;
                }
                cx.notify();
            });
        })
        .detach();
        self.spawn_progress_poll(cx);
    }

    /// 搜索期间每 120ms 刷一次界面，让进度数字动起来。
    fn spawn_progress_poll(&mut self, cx: &mut Context<Self>) {
        if self.progress_polling {
            return;
        }
        self.progress_polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(120))
                    .await;
                let keep = this
                    .update(cx, |panel, cx| {
                        if !panel.searching {
                            panel.progress_polling = false;
                            return false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    fn toggle_option(&mut self, option: search::SearchOption, cx: &mut Context<Self>) {
        match option {
            search::SearchOption::CaseSensitive => self.case_sensitive = !self.case_sensitive,
            search::SearchOption::WholeWord => self.whole_word = !self.whole_word,
        }
        // 已经搜过就按新选项重搜一次，否则用户还得再按回车。
        if self.searched {
            self.run(cx);
        } else {
            cx.notify();
        }
    }

    /// 把结果列表里每个文件中的命中全部替换掉。
    ///
    /// 只动这一次搜索命中的文件，匹配条件沿用上次搜索（而不是输入框里正在改的文字），
    /// 写完刷新受影响的打开标签并重搜一遍。
    fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.searched_query.clone();
        if query.is_empty() || self.results.is_empty() {
            return;
        }
        let replacement = self.replace.read(cx).value().to_string();
        let mut paths: Vec<String> = Vec::new();
        for hit in &self.results.hits {
            if !paths.contains(&hit.path) {
                paths.push(hit.path.clone());
            }
        }
        let outcome = search::replace_in_files(&paths, &query, &replacement);
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |this, cx| this.after_replace(&outcome, window, cx));
        }
        self.run(cx);
    }
}

impl BasePanel for SearchPanel {
    fn panel_name(&self) -> &'static str {
        SEARCH_PANEL
    }

    fn visible(&self, cx: &App) -> bool {
        SettingsStore::snapshot(cx).appearance.show_search()
    }

    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.group = Some(group);
    }

    fn on_removed(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.group = None;
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl DockPanel for SearchPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some("搜索".into())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "搜索"
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![
            Button::new("search-close-panel-button")
                .ghost()
                .xsmall()
                .icon(IconName::PanelLeftClose)
                .on_click(|_, _, cx| {
                    SettingsStore::update(cx, |settings| settings.appearance.set_search(false));
                }),
        ])
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for SearchPanel {}

impl Focusable for SearchPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SearchPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.clone();
        let panel = cx.entity();
        let toggle_option: search::ToggleOptionAction = {
            let panel = panel.clone();
            Rc::new(move |option, _window, cx| {
                panel.update(cx, |panel, cx| panel.toggle_option(option, cx));
            })
        };
        let run_search: search::RunSearchAction = {
            let panel = panel.clone();
            Rc::new(move |_window, cx| {
                panel.update(cx, |panel, cx| panel.run(cx));
            })
        };
        let run_replace: search::RunSearchAction = {
            let panel = panel.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                panel.update(cx, |panel, cx| panel.replace_all(window, cx));
            })
        };
        let close_panel = Rc::new(move |_: &mut Window, cx: &mut App| {
            SettingsStore::update(cx, |settings| settings.appearance.set_search(false));
        }) as outline::ClosePanelAction;
        let choose_scope: search::RunSearchAction = {
            let app = self.app.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.choose_search_scope(window, cx));
            })
        };
        let clear_scope: search::RunSearchAction = {
            let panel = panel.clone();
            Rc::new(move |_window, cx| {
                panel.update(cx, |panel, cx| panel.set_scope(None, cx));
            })
        };
        let close_for_escape = close_panel.clone();
        let open_hit: search::OpenHitAction = Rc::new(
            move |path: &str, line: usize, window: &mut Window, cx: &mut App| {
                let _ = app.update(cx, |this, cx| {
                    this.open_search_hit(path, line, window, cx)
                });
            },
        );

        let query = self.current_query(cx);
        let palette = Palette::for_theme(cx);
        let focused = self.query.read(cx).has_selection_focus(window, cx);
        let enter = run_search.clone();
        let escape = close_for_escape.clone();
        let mut input = Input::new(&self.query)
            .xsmall()
            .cleanable(true)
            .bg(rgb(palette.panel_bg))
            // 不要组件自带的聚焦外圈：焦点只用更亮的边框表示。
            .focus_ring(false);
        if focused {
            input = input.border_color(rgb(palette.sidebar_text));
        }
        let query_input = div()
            .px(px(8.0))
            .pt(px(6.0))
            .pb(px(4.0))
            .bg(rgb(palette.panel_bg))
            .on_action(move |_: &Enter, window, cx| enter(window, cx))
            .on_action(move |_: &Escape, window, cx| escape(window, cx))
            .child(input)
            .into_any_element();

        let replace_focused = self.replace.read(cx).has_selection_focus(window, cx);
        let mut replace_field = Input::new(&self.replace)
            .xsmall()
            .cleanable(true)
            .bg(rgb(palette.panel_bg))
            .focus_ring(false);
        if replace_focused {
            replace_field = replace_field.border_color(rgb(palette.sidebar_text));
        }
        let replace_enter = run_replace.clone();
        let replace_input = div()
            .on_action(move |_: &Enter, window, cx| replace_enter(window, cx))
            .child(replace_field)
            .into_any_element();

        let panel_id = PanelId::from(cx.entity().entity_id());
        let drag = self.group.as_ref().and_then(|group| group.upgrade()).map(|group| {
            DragPanel::new(panel_id, group.read(cx).node())
        });
        let drag_start = {
            let app = self.app.clone();
            Some(Rc::new(move |cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.begin_panel_drag(panel_id, cx));
            }) as outline::DragStartAction)
        };
        let progress = self
            .progress
            .as_ref()
            .filter(|_| self.searching)
            .map(|progress| progress.snapshot());

        div()
            .size_full()
            .p(px(ui_theme::PANEL_GAP / 2.0))
            .on_action({
                let close = close_panel.clone();
                move |_: &Escape, window, cx| close(window, cx)
            })
            .child(search::render(
                search::SearchRender {
                    palette,
                    query: &query,
                    searched: self.searched,
                    results: &self.results,
                    progress,
                    scope: self.scope.as_deref(),
                    toggle_option,
                    run_search,
                    run_replace,
                    choose_scope,
                    clear_scope,
                    close_panel,
                    open_hit,
                },
                drag,
                drag_start,
                query_input,
                replace_input,
            ))
    }
}

/// 输出面板。
pub struct OutputPanel {
    app: WeakEntity<CodeWindow>,
    log: Entity<OutputLog>,
    font_size: f32,
    /// 日志过滤输入框（按关键字过滤，空串表示不过滤）。
    filter: Entity<InputState>,
    /// 只看错误/警告。
    only_errors: bool,
    focus: FocusHandle,
    _log_subscription: Subscription,
    _filter_subscription: Subscription,
}

impl OutputPanel {
    pub fn new(
        app: WeakEntity<CodeWindow>,
        log: Entity<OutputLog>,
        font_size: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&log, |_, _, cx| cx.notify());
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("过滤输出"));
        let filter_subscription = cx.observe(&filter, |panel: &mut Self, _, cx| {
            panel.apply_filter(cx);
            cx.notify();
        });
        Self {
            app,
            log,
            font_size,
            filter,
            only_errors: false,
            focus: cx.focus_handle(),
            _log_subscription: subscription,
            _filter_subscription: filter_subscription,
        }
    }

    /// 把输入框与开关的状态推给日志模型。
    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let text = self.filter.read(cx).value().to_string();
        let filter = output::OutputFilter {
            text,
            only_errors: self.only_errors,
        };
        self.log.update(cx, |log, cx| log.set_filter(filter, cx));
    }

    pub fn set_font_size(&mut self, font_size: f32, cx: &mut Context<Self>) {
        if self.font_size != font_size {
            self.font_size = font_size;
            cx.notify();
        }
    }
}

impl BasePanel for OutputPanel {
    fn panel_name(&self) -> &'static str {
        OUTPUT_PANEL
    }

    fn visible(&self, cx: &App) -> bool {
        SettingsStore::snapshot(cx).appearance.show_output
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl DockPanel for OutputPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some("输出".into())
    }

    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        format!("输出 · {}", self.log.read(cx).channel())
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        let copy_log = self.log.clone();
        let clear_log = self.log.clone();
        let empty = self.log.read(cx).is_empty();
        Some(vec![
            Button::new("output-copy")
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .disabled(empty)
                .on_click(move |_, _, cx| {
                    let text = copy_log.read(cx).copy_text();
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }),
            Button::new("output-clear")
                .ghost()
                .xsmall()
                .icon(IconName::Delete)
                .on_click(move |_, _, cx| {
                    clear_log.update(cx, |log, cx| log.clear(cx));
                }),
            Button::new("output-close")
                .ghost()
                .xsmall()
                .icon(IconName::ChevronDown)
                .on_click(|_, _, cx| {
                    SettingsStore::update(cx, |settings| {
                        settings.appearance.show_output = false;
                    });
                }),
        ])
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for OutputPanel {}

impl Focusable for OutputPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for OutputPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let log = self.log.read(cx);
        let panel = cx.entity().downgrade();
        let on_toggle_only_errors: output::FilterToggle = Rc::new(move |_, _, cx| {
                let _ = panel.update(cx, |panel, cx| {
                    panel.only_errors = !panel.only_errors;
                    panel.apply_filter(cx);
                    cx.notify();
                });
            });
        let props = OutputPanelProps {
            log,
            entity: self.log.clone(),
            palette: Palette::for_theme(cx),
            height: None,
            font_size: self.font_size,
            filter_input: Some(self.filter.clone()),
            only_errors: self.only_errors,
            on_toggle_only_errors: Some(on_toggle_only_errors),
        };

        let copy_log = self.log.clone();
        let on_copy = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            let text = copy_log.read(cx).copy_text();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        };
        let clear_log = self.log.clone();
        let on_clear = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            clear_log.update(cx, |log, cx| log.clear(cx));
        };
        let app = self.app.clone();
        let on_close = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            let _ = app.update(cx, |this, cx| this.close_output(cx));
        };

        div()
            .size_full()
            .p(px(ui_theme::PANEL_GAP / 2.0))
            .child(output::render(props, on_copy, on_clear, on_close, true))
    }
}
