//! 资源管理器数据模型的测试。
//!
//! 这些测试放在独立文件里而不是 `explorer.rs` 的 `mod tests` 中：
//! 写在界面模块内部会触发 rustc 的宏展开递归上限。

use std::fs;
use std::path::PathBuf;

use crate::explorer::{ExplorerModel, Root};

/// 造一个临时文件夹：含 `sub/inner.txt` 与 `a.txt`。
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gpui-first-explorer-{}-{name}", std::process::id()));
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
    assert_eq!(model.expanded_ids(), vec![dir.to_string_lossy().into_owned()]);

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
    assert!(
        root.children
            .iter()
            .any(|item| item.label == "a.txt")
    );

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
