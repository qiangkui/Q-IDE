//! 资源管理器：项目树侧边栏。
//!
//! 树里可以挂多个根项：用户通过标题栏右侧「+」按钮添加的磁盘文件夹。
//! 每个根项后面的「-」只把它从树里移除，不会改动磁盘上的文件。
//! 标题栏最右侧的关闭按钮收起整个面板（等同取消勾选「视图 → 资源管理器」）。
//! 展开状态与根项列表都由 [`ExplorerModel`] 保存，并随布局写盘，下次启动时恢复。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::dock::DragPanel;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::tree::{self as component_tree, TreeItem, TreeState};
use gpui_kit::component::*;
use gpui_kit::prelude::{
    FluentBuilder as _, InteractiveElement as _, StatefulInteractiveElement as _,
};
use gpui_kit::*;

use crate::panel_drag::PanelDragPreview;
use crate::theme::{self as ui_theme, Palette};

/// 打开文件的回调：参数是树条目 id。
pub type OpenFileAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;
/// 点击标题栏「+」的回调，由窗口弹出系统的文件夹选择框。
pub type AddFolderAction = Rc<dyn Fn(&mut Window, &mut App)>;
/// 点击根项「-」的回调：把该根项从树里移除（不删磁盘文件）。
pub type RemoveRootAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;
/// 点击文件夹的回调：切换展开状态。
pub type ToggleFolderAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;
/// 点击标题栏关闭按钮的回调：收起资源管理器。
pub type ClosePanelAction = Rc<dyn Fn(&mut Window, &mut App)>;
/// 标题栏上的无参命令（新建文件/文件夹、刷新……）。
pub type PanelCommand = Rc<dyn Fn(&mut Window, &mut App)>;
/// 树条目右键回调：条目 id、是否文件夹、是否根项、鼠标位置。
pub type ExplorerMenuAction = Rc<dyn Fn(String, bool, bool, Point<Pixels>, &mut Window, &mut App)>;
/// 把条目拖到某个目录上：源路径、目标目录。
pub type ExplorerMoveAction = Rc<dyn Fn(&str, &str, &mut Window, &mut App)>;

/// 拖动资源管理器条目时携带的数据。
#[derive(Clone)]
pub struct ExplorerDrag {
    pub path: String,
    pub is_dir: bool,
}

/// 拖动时的跟随预览。
pub struct ExplorerDragPreview {
    label: SharedString,
}

impl ExplorerDragPreview {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

impl Render for ExplorerDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_theme(cx);
        div()
            .id("explorer-drag-preview")
            .cursor_grab()
            .py_1()
            .px_2()
            .max_w(px(220.0))
            .overflow_hidden()
            .whitespace_nowrap()
            .border_1()
            .border_color(rgba(palette.border))
            .rounded(px(4.0))
            .text_xs()
            .text_color(rgb(palette.sidebar_text))
            .bg(rgb(palette.sidebar_bg))
            .opacity(0.8)
            .child(self.label.clone())
    }
}

/// 折叠的文件夹也向下预读一层，树才会认为它有子项、才会显示展开箭头。
const MAX_PREVIEW_DEPTH: usize = 32;

/// 空文件夹的占位子条目 id 后缀。
///
/// 树组件用「有没有子项」判断是不是文件夹（`TreeItem::is_folder`），
/// 空目录必须补一个占位子项，否则会显示成文件图标、也点不开。
pub const EMPTY_FOLDER_SUFFIX: char = '\u{1}';

/// 空文件夹占位条目的显示文字。
pub const EMPTY_FOLDER_LABEL: &str = "（空文件夹）";

/// 这个树条目是不是为空文件夹补的占位项。
pub fn is_empty_folder_placeholder(id: &str) -> bool {
    id.ends_with(EMPTY_FOLDER_SUFFIX)
}

/// 能不能把 `source` 移动到 `target_dir` 里。
///
/// 规则：不能拖到自己身上、不能把文件夹拖进它自己的子目录、目标必须是目录。
pub fn can_move_into(source: &str, target_dir: &str, source_is_dir: bool) -> bool {
    if source == target_dir {
        return false;
    }
    let target = Path::new(target_dir);
    if !target.is_dir() {
        return false;
    }
    if source_is_dir {
        // 拖进自己的子目录会把目录搬进自己里面。
        let source_path = Path::new(source);
        if target.starts_with(source_path) {
            return false;
        }
    }
    // 已经在目标目录里的条目不用动。
    Path::new(source).parent() != Some(target)
}

/// 资源管理器的一个根项：用户添加的磁盘文件夹。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root(PathBuf);

impl Root {
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// 根项的树条目 id：磁盘文件夹的绝对路径。
    pub fn id(&self) -> String {
        self.path().to_string_lossy().into_owned()
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// 资源管理器的数据模型：根项列表 + 各文件夹的展开状态。
pub struct ExplorerModel {
    roots: Vec<Root>,
    /// 显式设置过的展开状态；没有记录的条目按默认值决定。
    expanded: HashMap<String, bool>,
}

impl Default for ExplorerModel {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

impl ExplorerModel {
    pub fn new(roots: Vec<Root>, expanded: Vec<String>) -> Self {
        let mut model = Self {
            roots,
            expanded: HashMap::new(),
        };
        for id in expanded {
            model.expanded.insert(id, true);
        }
        model
    }

    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// 所有处于展开状态的文件夹 id，排序后写入布局文件。
    pub fn expanded_ids(&self) -> Vec<String> {
        let mut ids = self
            .expanded
            .iter()
            .filter(|(_, open)| **open)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    /// 添加一个磁盘文件夹；已经在树里时返回 `false`。
    pub fn add_folder(&mut self, path: PathBuf) -> bool {
        if self.roots.iter().any(|root| same_folder(root, &path)) {
            return false;
        }
        self.expanded
            .insert(path.to_string_lossy().into_owned(), true);
        self.roots.push(Root::new(path));
        true
    }

    /// 移除一个根项（只影响树，不动磁盘）；找不到时返回 `false`。
    pub fn remove_root(&mut self, id: &str) -> bool {
        let before = self.roots.len();
        self.roots.retain(|root| root.id() != id);
        before != self.roots.len()
    }

    /// 切换文件夹的展开状态。
    pub fn toggle(&mut self, id: &str) {
        let current = self.is_expanded(id);
        self.expanded.insert(id.to_string(), !current);
    }

    /// 记下某个文件夹已展开（新建文件后用它把新文件显示出来）。
    pub fn expand(&mut self, path: &Path) {
        self.expanded
            .insert(path.to_string_lossy().into_owned(), true);
    }

    fn is_expanded(&self, id: &str) -> bool {
        self.expanded.get(id).copied().unwrap_or(false)
    }

    /// 生成当前模型对应的树条目。
    pub fn items(&self) -> Vec<TreeItem> {
        self.roots.iter().map(|root| self.root_item(root)).collect()
    }

    fn root_item(&self, root: &Root) -> TreeItem {
        self.dir_item(root.path(), 1)
    }

    /// 磁盘目录对应的树条目：展开时继续向下建树，折叠时只预读一层子项。
    fn dir_item(&self, path: &Path, depth: usize) -> TreeItem {
        let id = path.to_string_lossy().into_owned();
        let expanded = self.is_expanded(&id);
        let mut item = TreeItem::new(id.clone(), label_of(path)).expanded(expanded);

        let mut dirs = Vec::new();
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let child = entry.path();
                if is_hidden(&child) {
                    continue;
                }
                if child.is_dir() {
                    dirs.push(child);
                } else {
                    files.push(child);
                }
            }
        }
        dirs.sort_by_key(|child| sort_key(child));
        files.sort_by_key(|child| sort_key(child));

        // 空目录补一个占位子项：树组件靠「有没有子项」判断文件夹。
        if dirs.is_empty() && files.is_empty() {
            item = item.child(
                TreeItem::new(format!("{id}{EMPTY_FOLDER_SUFFIX}"), EMPTY_FOLDER_LABEL).disabled(true),
            );
        }

        let recurse = expanded && depth < MAX_PREVIEW_DEPTH;
        for child in dirs.into_iter().chain(files) {
            item = item.child(if recurse {
                self.entry_item(&child, depth + 1)
            } else {
                TreeItem::new(child.to_string_lossy().into_owned(), label_of(&child))
            });
        }
        item
    }

    fn entry_item(&self, path: &Path, depth: usize) -> TreeItem {
        if path.is_dir() {
            self.dir_item(path, depth)
        } else {
            TreeItem::new(path.to_string_lossy().into_owned(), label_of(path))
        }
    }
}

/// 一次渲染需要的全部回调与状态。
pub struct ExplorerActions {
    pub open_file: OpenFileAction,
    pub add_folder: AddFolderAction,
    pub remove_root: RemoveRootAction,
    pub toggle_folder: ToggleFolderAction,
    pub close_panel: ClosePanelAction,
    /// 在目标目录里新建文件。
    pub new_file: PanelCommand,
    /// 在目标目录里新建文件夹。
    pub new_folder: PanelCommand,
    /// 重新读盘刷新树。
    pub refresh: PanelCommand,
    /// 在树条目上右键：打开自绘菜单。
    pub on_context_menu: ExplorerMenuAction,
    /// 把条目拖到目录上（移动）。
    pub on_move: ExplorerMoveAction,
    /// 树里没有任何根项时显示提示文字。
    pub is_empty: bool,
}

/// 开始拖动侧栏标题时通知主窗口。
pub type DragStartAction = Rc<dyn Fn(&mut App)>;

/// 面板标题栏上的 18×18 图标按钮。
fn header_icon_button(
    id: &'static str,
    icon: IconName,
    palette: Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_shrink_0()
        .size(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(rgba(palette.hover)))
        .child(
            Icon::new(icon)
                .xsmall()
                .text_color(rgb(palette.sidebar_text)),
        )
        .on_click(move |_, window, cx| on_click(window, cx))
}

/// 渲染资源管理器。
pub fn render(
    tree: &Entity<TreeState>,
    width: Option<f32>,
    palette: Palette,
    actions: ExplorerActions,
    show_header: bool,
    drag: Option<DragPanel>,
    on_drag_start: Option<DragStartAction>,
) -> AnyElement {
    let ExplorerActions {
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
        is_empty,
    } = actions;

    let add_button = header_icon_button(
        "explorer-add-folder",
        IconName::Plus,
        palette,
        move |window, cx| add_folder(window, cx),
    );
    let new_file_button = header_icon_button(
        "explorer-new-file",
        IconName::File,
        palette,
        move |window, cx| new_file(window, cx),
    );
    let new_folder_button = header_icon_button(
        "explorer-new-folder",
        IconName::Folder,
        palette,
        move |window, cx| new_folder(window, cx),
    );
    let refresh_button = header_icon_button(
        "explorer-refresh",
        IconName::RefreshCw,
        palette,
        move |window, cx| refresh(window, cx),
    );
    let close_button = header_icon_button(
        "explorer-close-panel",
        IconName::PanelLeftClose,
        palette,
        move |window, cx| close_panel(window, cx),
    );

    let body = if is_empty {
        v_flex()
            .p(px(12.0))
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.sidebar_text))
                    .child("还没有添加文件夹"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.sidebar_text))
                    .child("点击右上角的 + 选择电脑上的文件夹"),
            )
            .into_any_element()
    } else {
        component_tree::tree(
            tree,
            move |_, entry, selected, _, _| {
                let is_folder = entry.is_folder();
                let is_root = entry.is_root();
                let id = entry.item().id.to_string();
                // 空文件夹占位项：只画一行灰色说明，不能点、也不能开菜单。
                if is_empty_folder_placeholder(&id) {
                    let depth = entry.depth() as f32;
                    return ListItem::new(entry.item().id.clone()).disabled(true).child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_1()
                            .pl(px(8.0 + depth * 12.0 + 12.0))
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .text_xs()
                                    .text_color(rgb(palette.muted_text))
                                    .child(EMPTY_FOLDER_LABEL),
                            ),
                    );
                }

                let indicator = if is_folder {
                    Icon::new(if entry.is_expanded() {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .xsmall()
                    .text_color(rgb(palette.sidebar_text))
                    .into_any_element()
                } else {
                    div().w(px(12.0)).into_any_element()
                };

                let mut row = h_flex()
                    .w_full()
                    .items_center()
                    .gap_1()
                    .rounded(px(3.0))
                    .pl(px(8.0 + entry.depth() as f32 * 12.0))
                    .child(indicator)
                    .child(
                        Icon::new(if is_folder {
                            IconName::FolderClosed
                        } else {
                            IconName::File
                        })
                        .xsmall()
                        .text_color(rgb(if is_folder {
                            palette.folder
                        } else {
                            palette.file
                        })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            // 侧栏变窄时用省略号截断，不要让文件名折成多行。
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.sidebar_text))
                            .child(entry.item().label.clone()),
                    );

                // 每个根项后面都带一个「-」，只从树里移除，不删磁盘文件。
                if is_root {
                    let remove_root = remove_root.clone();
                    let remove_id = id.clone();
                    row = row.child(
                        div()
                            .id(SharedString::from(format!("explorer-remove-{id}")))
                            .flex_shrink_0()
                            .size(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .text_sm()
                            .text_color(rgb(palette.sidebar_text))
                            .hover(move |this| this.bg(rgba(palette.hover)))
                            .child("-")
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                remove_root(&remove_id, window, cx);
                            }),
                    );
                }

                let item = ListItem::new(entry.item().id.clone())
                    .selected(selected)
                    .child(row);

                // 拖动条目：数据里带上路径，拖到目录或标签栏上时用。
                let drag_path = id.clone();
                let drag_label = entry.item().label.clone();
                let item = item.on_drag(
                    ExplorerDrag {
                        path: drag_path,
                        is_dir: is_folder,
                    },
                    move |drag: &ExplorerDrag, _, _, cx| {
                        let label = if drag.is_dir {
                            format!("{}/", drag_label)
                        } else {
                            drag_label.to_string()
                        };
                        cx.new(|_| ExplorerDragPreview::new(label))
                    },
                );

                // 文件夹可以作为放置目标：把拖来的条目移动进去。
                let item = if is_folder {
                    let target = id.clone();
                    let move_action = on_move.clone();
                    let dragged = id.clone();
                    item.drag_over::<ExplorerDrag>(move |style, drag, _, _| {
                        if can_move_into(&drag.path, &dragged, drag.is_dir) {
                            style.bg(rgba(palette.selection))
                        } else {
                            style
                        }
                    })
                    .on_drop(move |drag: &ExplorerDrag, window, cx| {
                        cx.stop_propagation();
                        if can_move_into(&drag.path, &target, drag.is_dir) {
                            let source = drag.path.clone();
                            move_action(&source, &target, window, cx);
                        }
                    })
                } else {
                    item
                };

                // 右键在条目上打开自绘菜单（新建/重命名/删除……）。
                let menu = on_context_menu.clone();
                let menu_id = id.clone();
                let item = item.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                    cx.stop_propagation();
                    let path = menu_id.clone();
                    menu(path, is_folder, is_root, event.position, window, cx);
                });

                // 左键：文件夹展开/收起，文件打开。
                if is_folder {
                    let toggle_folder = toggle_folder.clone();
                    let folder_id = id.clone();
                    item.on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        toggle_folder(&folder_id, window, cx);
                    })
                } else {
                    let open_file = open_file.clone();
                    let file_id = id.clone();
                    item.on_click(move |_, window, cx| open_file(&file_id, window, cx))
                }
            },
        )
        .into_any_element()
    };

    let mut panel = v_flex()
        .h_full()
        .bg(rgb(palette.sidebar_bg))
        .rounded(px(ui_theme::PANE_RADIUS))
        .border_1()
        .border_color(rgba(palette.border))
        .overflow_hidden();
    panel = match width {
        Some(width) => panel.w(px(width)).flex_shrink_0(),
        None => panel.w_full(),
    };

    let title = div()
        .id("explorer-title")
        .flex_1()
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .text_xs()
        .text_color(rgb(palette.sidebar_text))
        .child("资源管理器");
    let title = if let Some(drag) = drag {
        title.on_drag(drag, move |drag, offset, _, cx| {
            cx.stop_propagation();
            if let Some(on_drag_start) = on_drag_start.clone() {
                on_drag_start(cx);
            }
            drag.set_drag_offset(offset);
            drag.set_preview_size(size(px(96.0), px(30.0)));
            cx.new(|_| PanelDragPreview::new("资源管理器"))
        })
    } else {
        title
    };

    panel
        .when(show_header, |this| {
            this.child(
                h_flex()
                    .h(px(ui_theme::EXPLORER_HEADER_HEIGHT))
                    .px(px(16.0))
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(title)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(new_file_button)
                            .child(new_folder_button)
                            .child(refresh_button)
                            .child(add_button)
                            .child(close_button),
                    ),
            )
        })
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .bg(rgb(palette.side_panel_bg))
                .child(body),
        )
        .into_any_element()
}

fn label_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn sort_key(path: &Path) -> String {
    label_of(path).to_lowercase()
}

/// 以 `.` 开头的条目（`.git` 之类的工具目录）默认不显示，减少干扰。
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
}

/// 判断某个根项是否就是这个文件夹（Windows 上路径大小写不敏感）。
fn same_folder(root: &Root, path: &Path) -> bool {
    root.path()
        .to_string_lossy()
        .to_lowercase()
        .trim_end_matches(['\\', '/'])
        == path.to_string_lossy().to_lowercase().trim_end_matches(['\\', '/'])
}
