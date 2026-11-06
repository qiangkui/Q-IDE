//! 界面设置模型测试：资源管理器与代码大纲共用左侧栏，必须互斥。

use std::time::Duration;

use crate::app::choose_side_dock;
use crate::settings::{AppearanceSettings, EditorSettings, SidePanel};
use gpui_kit::component::dock::DockPlacement;

fn only_outline() -> AppearanceSettings {
    AppearanceSettings {
        show_explorer: false,
        show_outline: true,
        ..AppearanceSettings::default()
    }
}

fn only_explorer() -> AppearanceSettings {
    AppearanceSettings {
        show_explorer: true,
        show_outline: false,
        ..AppearanceSettings::default()
    }
}

#[test]
fn opening_explorer_closes_outline() {
    let mut appearance = only_outline();
    appearance.toggle_explorer();
    assert!(appearance.show_explorer);
    assert!(!appearance.show_outline);
}

#[test]
fn opening_outline_closes_explorer() {
    let mut appearance = only_explorer();
    appearance.toggle_outline();
    assert!(appearance.show_outline);
    assert!(!appearance.show_explorer);
}

#[test]
fn closing_a_panel_does_not_open_the_other() {
    let mut appearance = only_explorer();
    appearance.toggle_explorer();
    assert!(!appearance.show_explorer);
    assert!(!appearance.show_outline);

    let mut appearance = only_outline();
    appearance.toggle_outline();
    assert!(!appearance.show_outline);
    assert!(!appearance.show_explorer);
}

#[test]
fn set_explorer_and_set_outline_enforce_exclusive_panels() {
    let mut appearance = AppearanceSettings::default();
    appearance.set_outline(true);
    assert!(appearance.show_outline);
    assert!(!appearance.show_explorer);

    appearance.set_explorer(true);
    assert!(appearance.show_explorer);
    assert!(!appearance.show_outline);
}

#[test]
fn legacy_layout_with_both_panels_keeps_explorer() {
    let mut appearance = AppearanceSettings::default();
    assert!(appearance.show_explorer && appearance.show_outline);
    appearance.normalize_side_panels();
    assert!(appearance.show_explorer);
    assert!(!appearance.show_outline);
    assert!(!appearance.show_search);
}

#[test]
fn opening_search_closes_the_other_side_panels() {
    let mut appearance = AppearanceSettings::default();
    appearance.toggle_search();
    assert_eq!(appearance.side_panel(), Some(SidePanel::Search));
    assert!(!appearance.show_explorer);
    assert!(!appearance.show_outline);

    appearance.toggle_outline();
    assert_eq!(appearance.side_panel(), Some(SidePanel::Outline));
    assert!(!appearance.show_search);

    appearance.toggle_search();
    assert_eq!(appearance.side_panel(), Some(SidePanel::Search));
    assert!(!appearance.show_outline);
}

#[test]
fn normalize_keeps_only_the_highest_priority_panel() {
    let mut appearance = AppearanceSettings {
        show_search: true,
        ..AppearanceSettings::default()
    };
    appearance.normalize_side_panels();
    assert_eq!(appearance.side_panel(), Some(SidePanel::Explorer));
}

#[test]
fn side_panel_stays_on_the_visible_side() {
    assert_eq!(
        choose_side_dock(true, false, true, true, DockPlacement::Left),
        DockPlacement::Right
    );
    assert_eq!(
        choose_side_dock(true, true, true, false, DockPlacement::Right),
        DockPlacement::Left
    );
}

#[test]
fn side_panel_remembers_the_last_side_when_nothing_is_present() {
    assert_eq!(
        choose_side_dock(false, false, false, false, DockPlacement::Right),
        DockPlacement::Right
    );
    assert_eq!(
        choose_side_dock(false, false, true, false, DockPlacement::Left),
        DockPlacement::Right
    );
}

#[test]
fn auto_save_waits_for_the_interval_and_dirty_files() {
    let settings = EditorSettings {
        auto_save: true,
        auto_save_interval_secs: 30,
        ..EditorSettings::default()
    };

    assert!(!settings.auto_save_due(Duration::from_secs(29), 1), "还没到间隔");
    assert!(settings.auto_save_due(Duration::from_secs(30), 1));
    assert!(!settings.auto_save_due(Duration::from_secs(60), 0), "没有改动不保存");
}

#[test]
fn auto_save_is_off_by_default() {
    let settings = EditorSettings::default();
    assert!(!settings.auto_save);
    assert!(!settings.auto_save_due(Duration::from_secs(600), 3));
}
