//! 界面设置模型测试：左右两侧栏各最多显示一个面板，打开与拖动会挪到空位或挤掉原面板。

use std::time::Duration;

use crate::settings::{AppearanceSettings, EditorSettings, PanelKind, SideSlot};

fn both_empty() -> AppearanceSettings {
    AppearanceSettings {
        left_panel: None,
        right_panel: None,
        ..AppearanceSettings::default()
    }
}

#[test]
fn default_layout_shows_explorer_on_the_left() {
    let appearance = AppearanceSettings::default();
    assert_eq!(appearance.left_panel, Some(PanelKind::Explorer));
    assert_eq!(appearance.right_panel, None);
    assert!(appearance.show_explorer());
    assert!(!appearance.show_outline());
    assert!(!appearance.show_search());
    assert!(!appearance.show_ai());
}

#[test]
fn opening_uses_the_preferred_side_when_both_are_empty() {
    let mut appearance = both_empty();
    appearance.open_panel(PanelKind::Outline);
    assert_eq!(
        appearance.slot_of(PanelKind::Outline),
        Some(SideSlot::Left)
    );

    let mut appearance = both_empty();
    appearance.open_panel(PanelKind::Ai);
    assert_eq!(appearance.slot_of(PanelKind::Ai), Some(SideSlot::Right));
}

#[test]
fn opening_a_panel_uses_the_free_side() {
    // 资源管理器在右边、左边空着：打开代码大纲放到左边。
    let mut appearance = both_empty();
    appearance.open_panel(PanelKind::Explorer);
    appearance.move_panel(PanelKind::Explorer, SideSlot::Right);

    appearance.open_panel(PanelKind::Outline);
    assert_eq!(
        appearance.slot_of(PanelKind::Explorer),
        Some(SideSlot::Right)
    );
    assert_eq!(
        appearance.slot_of(PanelKind::Outline),
        Some(SideSlot::Left)
    );

    // 代码大纲在右边、左边空着：打开 AI 放到左边。
    let mut appearance = both_empty();
    appearance.open_panel(PanelKind::Outline);
    appearance.move_panel(PanelKind::Outline, SideSlot::Right);
    appearance.open_panel(PanelKind::Ai);
    assert_eq!(
        appearance.slot_of(PanelKind::Outline),
        Some(SideSlot::Right)
    );
    assert_eq!(appearance.slot_of(PanelKind::Ai), Some(SideSlot::Left));
}

#[test]
fn opening_replaces_the_preferred_side_when_both_are_occupied() {
    let mut appearance = both_empty();
    appearance.open_panel(PanelKind::Explorer);
    appearance.open_panel(PanelKind::Ai);

    // 两个位置都占满：代码大纲替换资源管理器（左侧）。
    appearance.open_panel(PanelKind::Outline);
    assert_eq!(appearance.left_panel, Some(PanelKind::Outline));
    assert_eq!(appearance.right_panel, Some(PanelKind::Ai));
    assert!(!appearance.show_explorer());

    // AI 再次打开时替换右侧。
    appearance.close_panel(PanelKind::Ai);
    appearance.open_panel(PanelKind::Ai);
    assert_eq!(appearance.left_panel, Some(PanelKind::Outline));
    assert_eq!(appearance.right_panel, Some(PanelKind::Ai));
}

#[test]
fn moving_a_panel_to_the_other_side_closes_its_occupant() {
    let mut appearance = both_empty();
    appearance.open_panel(PanelKind::Explorer);
    appearance.open_panel(PanelKind::Ai);

    // 把资源管理器拖到右边：原来的 AI 面板关闭。
    appearance.move_panel(PanelKind::Explorer, SideSlot::Right);
    assert_eq!(appearance.left_panel, None);
    assert_eq!(appearance.right_panel, Some(PanelKind::Explorer));
    assert!(!appearance.show_ai());
    assert!(appearance.show_explorer());

    // 再拖回左边也一样。
    appearance.open_panel(PanelKind::Outline);
    assert_eq!(appearance.left_panel, Some(PanelKind::Outline));
    appearance.move_panel(PanelKind::Explorer, SideSlot::Left);
    assert_eq!(appearance.left_panel, Some(PanelKind::Explorer));
    assert_eq!(appearance.right_panel, None);
    assert!(!appearance.show_outline());
}

#[test]
fn moving_a_panel_to_its_own_side_keeps_it() {
    let mut appearance = both_empty();
    appearance.open_panel(PanelKind::Explorer);
    appearance.move_panel(PanelKind::Explorer, SideSlot::Left);
    assert_eq!(appearance.left_panel, Some(PanelKind::Explorer));
    assert_eq!(appearance.right_panel, None);
}

#[test]
fn closing_and_toggling_clear_the_right_slot() {
    let mut appearance = both_empty();
    appearance.open_panel(PanelKind::Ai);
    assert_eq!(appearance.right_panel, Some(PanelKind::Ai));

    appearance.close_panel(PanelKind::Ai);
    assert_eq!(appearance.right_panel, None);
    assert!(!appearance.show_ai());

    appearance.toggle_panel(PanelKind::Ai);
    assert!(appearance.show_ai());
    appearance.toggle_panel(PanelKind::Ai);
    assert!(!appearance.show_ai());
    assert_eq!(appearance.right_panel, None);
}

#[test]
fn set_panel_matches_open_and_close() {
    let mut appearance = both_empty();
    appearance.set_panel(PanelKind::Search, true);
    assert_eq!(appearance.left_panel, Some(PanelKind::Search));
    appearance.set_panel(PanelKind::Search, false);
    assert_eq!(appearance.left_panel, None);
}

#[test]
fn preferred_side_of_side_panels_is_left_and_ai_is_right() {
    assert_eq!(PanelKind::Explorer.preferred_slot(), SideSlot::Left);
    assert_eq!(PanelKind::Outline.preferred_slot(), SideSlot::Left);
    assert_eq!(PanelKind::Search.preferred_slot(), SideSlot::Left);
    assert_eq!(PanelKind::Ai.preferred_slot(), SideSlot::Right);
    assert_eq!(SideSlot::Left.other(), SideSlot::Right);
    assert_eq!(SideSlot::Right.other(), SideSlot::Left);
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

#[test]
fn ai_settings_fall_back_to_deepseek_defaults() {
    use crate::settings::{
        AiApproval, AiRoundLimit, AiSettings, DEFAULT_AI_BASE_URL, DEFAULT_AI_FONT_SIZE,
        DEFAULT_AI_MODEL,
    };

    let ai = AiSettings::default();
    assert_eq!(ai.base_url, DEFAULT_AI_BASE_URL);
    assert_eq!(ai.model, DEFAULT_AI_MODEL);
    assert_eq!(ai.effective_base_url(), DEFAULT_AI_BASE_URL);
    assert_eq!(ai.effective_model(), DEFAULT_AI_MODEL);
    // 默认每次写入前确认，工具最多连续跑 6 轮，对话字号与原 text_xs 一致。
    assert_eq!(ai.approval, AiApproval::Ask);
    assert_eq!(ai.round_limit, AiRoundLimit::Six);
    assert_eq!(ai.font_size, DEFAULT_AI_FONT_SIZE);

    // 设置窗口里清空输入不应该让请求打到空地址或空模型。
    let blank = AiSettings {
        base_url: "   ".to_string(),
        model: String::new(),
        ..AiSettings::default()
    };
    assert_eq!(blank.effective_base_url(), DEFAULT_AI_BASE_URL);
    assert_eq!(blank.effective_model(), DEFAULT_AI_MODEL);

    let custom = AiSettings {
        base_url: " https://example.com/v1 ".to_string(),
        model: " deepseek-reasoner ".to_string(),
        ..AiSettings::default()
    };
    assert_eq!(custom.effective_base_url(), "https://example.com/v1");
    assert_eq!(custom.effective_model(), "deepseek-reasoner");
}

#[test]
fn ai_approval_round_trips_through_text() {
    use crate::settings::AiApproval;

    assert_eq!(AiApproval::from_str("auto"), AiApproval::Auto);
    assert_eq!(AiApproval::from_str("ask"), AiApproval::Ask);
    // 旧布局或写坏的值一律按「每次确认」处理。
    assert_eq!(AiApproval::from_str(""), AiApproval::Ask);
    assert_eq!(AiApproval::from_str("whatever"), AiApproval::Ask);
    assert_eq!(AiApproval::Auto.as_str(), "auto");
    assert_eq!(AiApproval::Ask.as_str(), "ask");
    // 下拉框的每个值都能原样解析回来。
    for (value, _) in AiApproval::OPTIONS {
        assert_eq!(AiApproval::from_str(value).as_str(), *value);
    }
    // 面板底栏用的是短标签，必须有字且各不相同。
    assert_eq!(AiApproval::Ask.short_label(), "执行前确认");
    assert_eq!(AiApproval::Auto.short_label(), "全部允许");
}

#[test]
fn ai_round_limit_options_and_reached_boundary() {
    use crate::settings::AiRoundLimit;

    assert_eq!(AiRoundLimit::default(), AiRoundLimit::Six);
    assert_eq!(AiRoundLimit::Six.limit(), Some(6));
    assert_eq!(AiRoundLimit::Twelve.limit(), Some(12));
    assert_eq!(AiRoundLimit::Unlimited.limit(), None);

    // 到上限那一轮执行完就停，不能多跑一轮。
    assert!(!AiRoundLimit::Six.reached(5));
    assert!(AiRoundLimit::Six.reached(6));
    assert!(!AiRoundLimit::Twelve.reached(11));
    assert!(AiRoundLimit::Twelve.reached(12));
    assert!(!AiRoundLimit::Unlimited.reached(10_000));

    assert_eq!(AiRoundLimit::from_str("12"), AiRoundLimit::Twelve);
    assert_eq!(AiRoundLimit::from_str("unlimited"), AiRoundLimit::Unlimited);
    // 旧布局或写坏的值回退到默认的 6 轮。
    assert_eq!(AiRoundLimit::from_str(""), AiRoundLimit::Six);
    assert_eq!(AiRoundLimit::from_str("whatever"), AiRoundLimit::Six);
    for (value, _) in AiRoundLimit::OPTIONS {
        assert_eq!(AiRoundLimit::from_str(value).as_str(), *value);
    }

    // 底栏短标签必须能区分三种设置。
    let labels: std::collections::HashSet<_> = AiRoundLimit::OPTIONS
        .iter()
        .map(|(value, _)| AiRoundLimit::from_str(value).short_label())
        .collect();
    assert_eq!(labels.len(), AiRoundLimit::OPTIONS.len());
}

#[test]
fn ai_model_options_cover_the_default_model() {
    use crate::settings::{AI_MODEL_OPTIONS, AiSettings, DEFAULT_AI_MODEL};

    assert!(!AI_MODEL_OPTIONS.is_empty());
    for (value, label) in AI_MODEL_OPTIONS {
        assert!(!value.trim().is_empty());
        assert!(!label.trim().is_empty());
    }
    // 面板「模型」下拉要能把默认模型标成已选中，所以它必须在列表里。
    assert!(
        AI_MODEL_OPTIONS
            .iter()
            .any(|(value, _)| *value == DEFAULT_AI_MODEL)
    );
    // 未改动过设置时，当前模型名与下拉里的值一致。
    let ai = AiSettings::default();
    assert_eq!(ai.effective_model(), DEFAULT_AI_MODEL);
}
