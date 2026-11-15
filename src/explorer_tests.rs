//! 资源管理器数据模型的测试。
//!
//! 这些测试放在独立文件里而不是 `explorer.rs` 的 `mod tests` 中：
//! 写在界面模块内部会触发 rustc 的宏展开递归上限。

use std::fs;
use std::path::PathBuf;

use crate::explorer::{DragOutState, ExplorerModel, ExplorerSelection, Root};

/// 造一个临时文件夹：含 `sub/inner.txt` 与 `a.txt`。
fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("gpui-first-explorer-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("sub")).expect("创建子目录");
    fs::write(dir.join("sub").join("inner.txt"), "x").expect("写子文件");
    fs::write(dir.join("a.txt"), "x").expect("写文件");
    dir
}

#[test]
fn adding_a_folder_expands_it_once() {
    let dir = temp_dir("add");
    let mut model = ExplorerModel::default();

    assert!(model.add_folder(dir.clone()));
    assert!(!model.add_folder(dir.clone()), "同一个文件夹只加一次");
    assert_eq!(model.roots().len(), 1);
    assert_eq!(
        model.expanded_ids(),
        vec![dir.to_string_lossy().into_owned()]
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn removing_a_root_leaves_the_disk_alone() {
    let dir = temp_dir("remove");
    let mut model = ExplorerModel::new(vec![Root::new(dir.clone())], Vec::new());

    assert!(model.remove_root(&dir.to_string_lossy()));
    assert!(model.roots().is_empty());
    assert!(dir.is_dir(), "移除只影响树，不能删磁盘文件");
    assert!(!model.remove_root(&dir.to_string_lossy()));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn folder_items_list_one_level_so_they_can_expand() {
    let dir = temp_dir("items");
    let model = ExplorerModel::new(
        vec![Root::new(dir.clone())],
        vec![dir.to_string_lossy().into_owned()],
    );

    let items = model.items();
    let root = &items[0];
    assert!(root.is_expanded() && root.is_folder());

    let sub = root
        .children
        .iter()
        .find(|item| item.label == "sub")
        .expect("子文件夹");
    assert!(sub.is_folder(), "折叠的子文件夹也要能点开");
    assert!(!sub.is_expanded());
    assert!(root.children.iter().any(|item| item.label == "a.txt"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn toggling_flips_expansion_state() {
    let dir = temp_dir("toggle");
    let id = dir.to_string_lossy().into_owned();
    let mut model = ExplorerModel::new(vec![Root::new(dir.clone())], vec![id.clone()]);

    model.toggle(&id);
    assert!(model.expanded_ids().is_empty());
    model.toggle(&id);
    assert_eq!(model.expanded_ids(), vec![id]);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn moving_is_only_allowed_into_other_existing_folders() {
    let dir = temp_dir("move-target");
    let sub = dir.join("sub");
    let file = dir.join("a.txt").to_string_lossy().into_owned();
    let sub_id = sub.to_string_lossy().into_owned();
    let dir_id = dir.to_string_lossy().into_owned();

    // 文件可以移进子目录（当前不在里面）。
    assert!(crate::explorer::can_move_into(&file, &sub_id, false));
    // 目标就是自己、或拖进自己子目录都不行。
    assert!(!crate::explorer::can_move_into(&dir_id, &dir_id, true));
    assert!(!crate::explorer::can_move_into(&dir_id, &sub_id, true));
    // 同一个目录里不用移动。
    assert!(!crate::explorer::can_move_into(&file, &dir_id, false));
    // 目标不存在不行。
    assert!(!crate::explorer::can_move_into(
        &file,
        &dir.join("缺失").to_string_lossy(),
        false
    ));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn dot_entries_are_hidden() {
    let dir = temp_dir("hidden");
    fs::create_dir_all(dir.join(".git")).expect("创建 .git");

    let model = ExplorerModel::new(vec![Root::new(dir.clone())], Vec::new());
    let labels = model.items()[0]
        .children
        .iter()
        .map(|item| item.label.to_string())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"a.txt".to_string()));
    assert!(!labels.contains(&".git".to_string()));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_folder_gets_a_placeholder_so_it_looks_like_a_folder() {
    let dir = temp_dir("empty-folder");
    fs::create_dir_all(dir.join("empty")).expect("创建空文件夹");
    std::fs::remove_file(dir.join("empty").join("inner.txt")).ok();

    let model = ExplorerModel::new(
        vec![Root::new(dir.clone())],
        vec![dir.to_string_lossy().into_owned()],
    );
    let root = &model.items()[0];
    let empty = root
        .children
        .iter()
        .find(|item| item.label == "empty")
        .expect("空文件夹");

    assert!(empty.is_folder(), "空文件夹也要显示成可展开的文件夹");
    assert_eq!(empty.children.len(), 1);
    let child = &empty.children[0];
    assert!(
        crate::explorer::is_empty_folder_placeholder(&child.id),
        "占位项要能被识别"
    );
    assert_eq!(child.label, crate::explorer::EMPTY_FOLDER_LABEL);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn selection_mode_enters_on_dot_and_exits_when_empty() {
    let mut selection = ExplorerSelection::default();
    assert!(!selection.is_active(), "默认不在选择模式");

    selection.click(r"C:\root\a.txt");
    assert!(selection.is_active(), "点圆点后进入选择模式");
    assert!(selection.contains(r"C:\root\a.txt"));

    selection.toggle(r"C:\root\b.txt");
    assert_eq!(selection.len(), 2, "选择模式下点条目是追加选中");

    selection.toggle(r"C:\root\a.txt");
    selection.toggle(r"C:\root\b.txt");
    assert!(selection.is_empty());
    assert!(!selection.is_active(), "全部取消选中后自动退出选择模式");
}

#[test]
fn selection_rewrites_moved_paths() {
    let mut selection = ExplorerSelection::default();
    selection.click(r"C:\root\dir\a.txt");
    selection.toggle(r"C:\root\dir");
    selection.toggle(r"C:\other.txt");

    selection.rewrite(r"C:\root\dir", r"C:\root\moved");

    assert_eq!(selection.len(), 3);
    assert!(selection.contains(r"C:\root\moved\a.txt"));
    assert!(selection.contains(r"C:\root\moved"));
    assert!(selection.contains(r"C:\other.txt"));
}

#[test]
fn clearing_selection_leaves_selection_mode() {
    let mut selection = ExplorerSelection::default();
    selection.click(r"C:\root\a.txt");
    selection.clear();

    assert!(selection.is_empty());
    assert!(!selection.is_active());
}

#[test]
fn ctrl_click_enters_selection_mode_from_normal_mode() {
    // 普通模式下 `Ctrl+点击` 走 `toggle`：过去只在选择模式里调用过，没把 active 置 true，
    // 所以点下去什么也看不见。现在第一次按下就进入选择模式。
    let mut selection = ExplorerSelection::default();
    assert!(!selection.is_active());

    selection.toggle(r"C:\root\a.txt");
    assert!(selection.is_active(), "Ctrl+点击 要进入选择模式");
    assert!(selection.contains(r"C:\root\a.txt"));

    selection.toggle(r"C:\root\b.txt");
    assert_eq!(selection.len(), 2);

    selection.toggle(r"C:\root\a.txt");
    selection.toggle(r"C:\root\b.txt");
    assert!(selection.is_empty());
    assert!(!selection.is_active(), "全部取消后退出选择模式");
}

#[test]
fn plain_click_records_the_shift_range_anchor() {
    // 普通点击（打开文件 / 展开目录）只记起点：之后 `Shift+点击` 从这里选一段范围。
    let mut selection = ExplorerSelection::default();
    selection.set_anchor(r"C:\root\a.txt");
    assert_eq!(selection.anchor(), Some(r"C:\root\a.txt"));
    assert!(!selection.is_active(), "只记起点不改选择状态");
    assert!(selection.is_empty());
}

#[test]
fn selection_tracks_the_anchor_for_shift_ranges() {
    let mut selection = ExplorerSelection::default();
    selection.click(r"C:\root\a.txt");
    assert_eq!(
        selection.anchor(),
        Some(r"C:\root\a.txt"),
        "点过的条目就是范围起点"
    );

    selection.toggle(r"C:\root\c.txt");
    assert_eq!(selection.anchor(), Some(r"C:\root\c.txt"));

    // `Shift+点击` 由调用方按可见顺序算好路径，这里只负责整体替换并保留起点。
    selection.set_paths(
        [
            r"C:\root\c.txt".to_string(),
            r"C:\root\d.txt".to_string(),
            r"C:\root\e.txt".to_string(),
        ],
        r"C:\root\c.txt",
    );
    assert_eq!(selection.len(), 3);
    assert!(selection.contains(r"C:\root\d.txt"));
    assert_eq!(selection.anchor(), Some(r"C:\root\c.txt"));

    // 范围为空时退出选择模式，起点保留（下一次 Shift+点击还能接着算）。
    selection.set_paths(Vec::new(), r"C:\root\c.txt");
    assert!(!selection.is_active());
    assert!(selection.is_empty());

    selection.clear();
    assert_eq!(selection.anchor(), None, "退出选择后范围起点也清空");
}

#[test]
fn drag_out_copies_only_after_releasing_outside() {
    let mut state = DragOutState::default();
    assert!(!state.is_dragging());

    state.begin(r"C:\root\a.txt");
    assert!(state.is_dragging());
    // 窗口内移动、窗口内松开：正常拖放，不复制。
    assert_eq!(state.moved(true, true), None);
    state.released_inside();
    assert!(!state.is_dragging());
    assert_eq!(state.moved(true, false), None);

    // 离开窗口后松手：复制路径。
    state.begin(r"C:\root\b.txt");
    assert_eq!(state.moved(false, true), None);
    assert_eq!(state.released_outside(), Some(r"C:\root\b.txt".to_string()));
    assert!(!state.is_dragging());
    // 同一次松开只复制一次。
    assert_eq!(state.released_outside(), None);
}

#[test]
fn drag_out_falls_back_when_mouse_up_was_not_delivered() {
    let mut state = DragOutState::default();
    state.begin(r"C:\root\c.txt");
    // 指针离开窗口（左键仍按下）。
    assert_eq!(state.moved(false, true), None);
    // 在别的窗口上松手，`mouse up` 没送进来；回到窗口内时左键已经不按了。
    assert_eq!(state.moved(true, false), Some(r"C:\root\c.txt".to_string()));
    assert!(!state.is_dragging());
}

#[test]
fn drag_out_ignores_moves_without_a_drag() {
    let mut state = DragOutState::default();
    assert_eq!(state.moved(false, true), None);
    assert_eq!(state.moved(true, false), None);
    assert_eq!(state.released_outside(), None);
}

#[test]
fn drag_out_cancel_discards_the_path() {
    let mut state = DragOutState::default();
    state.begin(r"C:\root\d.txt");
    state.moved(false, true);
    state.cancel();
    assert!(!state.is_dragging());
    assert_eq!(state.moved(true, false), None);
    assert_eq!(state.released_outside(), None);

    // 新一次拖动重新记录路径。
    state.begin(r"C:\root\e.txt");
    assert_eq!(state.released_outside(), Some(r"C:\root\e.txt".to_string()));
}
