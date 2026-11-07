//! 自定义主题的解析、保存与调色板映射测试。

use crate::custom_theme::{
    COLORS, CustomTheme, NEW_THEME_KEY, ThemeDraft, ThemeStore, format_color, parse_color,
};
use crate::theme::Palette;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gpui-first-theme-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn colors_parse_with_and_without_hash_and_alpha() {
    assert_eq!(parse_color("#1e2d3c"), Some(0x1e2d3c));
    assert_eq!(parse_color("1E2D3C"), Some(0x1e2d3c));
    assert_eq!(parse_color("#8a8a8a66"), Some(0x8a8a8a66));
    assert_eq!(parse_color(" #1e2d3c "), Some(0x1e2d3c));
    assert_eq!(parse_color("#12345"), None);
    assert_eq!(parse_color("#gggggg"), None);
    assert_eq!(parse_color(""), None);
}

#[test]
fn colors_format_round_trip() {
    for value in [0x000000, 0x1e2d3c, 0xffffff, 0x8a8a8a66, 0xffffff12] {
        assert_eq!(parse_color(&format_color(value)), Some(value));
    }
    assert_eq!(format_color(0x1e2d3c), "#1E2D3C");
    assert_eq!(format_color(0x8a8a8a66), "#8A8A8A66");
}

#[test]
fn theme_file_round_trip_keeps_every_theme() {
    let mut store = ThemeStore::default();
    store.upsert(CustomTheme {
        name: "深蓝".to_string(),
        dark: true,
        colors: COLORS.iter().map(|_| 0x112233).collect(),
    });
    let mut light = CustomTheme {
        name: "米色".to_string(),
        dark: false,
        colors: COLORS.iter().map(|_| 0xaabbcc).collect(),
    };
    // 选区与分隔线带透明度。
    light.colors[7] = 0x33669980;
    light.colors[8] = 0x44556622;
    store.upsert(light);

    let parsed = ThemeStore::parse(&store.to_text());
    assert_eq!(parsed.themes(), store.themes());
}

#[test]
fn missing_colors_fall_back_to_the_base_palette() {
    let store = ThemeStore::parse("theme=暗夜\nbase=dark\nwindow_bg=#000010\n");
    let theme = store.find("暗夜").expect("应解析出主题");
    assert!(theme.dark);
    assert_eq!(theme.colors.len(), COLORS.len());
    assert_eq!(theme.colors[0], 0x000010);
    // 没写的颜色取深色内置主题的值。
    assert_eq!(theme.colors[1], Palette::dark().activity_bar_bg);
}

#[test]
fn light_base_and_unknown_keys_are_handled() {
    let store = ThemeStore::parse(
        "theme=晨光\nbase=light\nnonsense=1\npanel_bg=oops\nwindow_bg=#FFFFFF\n",
    );
    let theme = store.find("晨光").expect("应解析出主题");
    assert!(!theme.dark);
    assert_eq!(theme.colors[0], 0xffffff);
    assert_eq!(theme.colors[3], Palette::light().panel_bg);
}

#[test]
fn reserved_and_empty_names_are_skipped() {
    let store = ThemeStore::parse("theme=dark\nwindow_bg=#000000\n\ntheme=\nbase=dark\n");
    assert!(store.themes().is_empty());
}

#[test]
fn reserved_names_cannot_be_saved_from_a_draft() {
    let mut draft = ThemeDraft::new_from(&Palette::dark(), true);
    draft.set_name("Light");
    assert!(draft.to_theme().is_err());
    draft.set_name("   ");
    assert!(draft.to_theme().is_err());
    draft.set_name("深蓝");
    assert!(draft.to_theme().is_ok());
}

#[test]
fn draft_rejects_invalid_colors_and_points_at_the_field() {
    let mut draft = ThemeDraft::new_from(&Palette::dark(), true);
    draft.set_name("深蓝");
    draft.set_color(0, "#12");
    let error = draft.to_theme().expect_err("颜色非法应报错");
    assert!(error.contains(COLORS[0].label), "错误信息应指出哪一项：{error}");
}

#[test]
fn draft_round_trips_through_a_custom_theme() {
    let mut draft = ThemeDraft::new_from(&Palette::light(), false);
    draft.set_name("深蓝");
    let theme = draft.to_theme().expect("应能保存");
    assert_eq!(ThemeDraft::saved(&theme).colors, draft.colors);
    assert_eq!(theme.colors[0], Palette::light().window_bg);
}

#[test]
fn builtin_drafts_are_read_only() {
    let dark = ThemeDraft::builtin(true);
    assert!(dark.is_builtin());
    assert!(!dark.is_editable());
    assert!(!dark.can_delete());
    assert!(dark.to_theme().is_err(), "内置主题不能保存");
    assert_eq!(dark.name, "深色");
    assert_eq!(dark.pick_key(), "dark");
    assert_eq!(dark.colors[0], format_color(Palette::dark().window_bg));

    let light = ThemeDraft::builtin(false);
    assert_eq!(light.name, "浅色");
    assert_eq!(light.pick_key(), "light");
}

#[test]
fn new_drafts_start_empty_and_follow_the_current_palette() {
    let draft = ThemeDraft::new_from(&Palette::dark(), true);
    assert!(draft.is_new());
    assert!(draft.is_editable());
    assert!(!draft.can_delete(), "新建时不能删除");
    assert!(draft.name.is_empty());
    assert_eq!(draft.pick_key(), NEW_THEME_KEY);
    assert_eq!(draft.colors[0], format_color(Palette::dark().window_bg));
}

#[test]
fn saved_drafts_replace_the_original_entry() {
    let theme = CustomTheme {
        name: "深蓝".to_string(),
        dark: true,
        colors: COLORS.iter().map(|_| 0x102030).collect(),
    };
    let mut draft = ThemeDraft::saved(&theme);
    assert!(draft.can_delete());
    assert_eq!(draft.pick_key(), "深蓝");
    assert_eq!(draft.saved_original(), Some("深蓝"));

    // 改名后仍然是对同一条目的替换。
    draft.set_name("靛蓝");
    let replaced = draft.replace_saved().expect("名称合法时应能即时替换");
    assert_eq!(replaced.name, "靛蓝");

    // 名称暂时清空（输入到一半）时不写回。
    draft.set_name("");
    assert!(draft.replace_saved().is_none());
}

#[test]
fn palette_applies_overrides_on_top_of_the_base() {
    let mut theme = CustomTheme {
        name: "深蓝".to_string(),
        dark: true,
        colors: COLORS.iter().map(|_| 0x101820).collect(),
    };
    theme.colors[6] = 0x8899aa; // 次要文字
    let palette = theme.palette();

    assert_eq!(palette.window_bg, 0x101820);
    assert_eq!(palette.sidebar_bg, 0x101820);
    assert_eq!(palette.popover_bg, 0x101820, "弹出层跟随侧栏底色");
    assert_eq!(palette.title_text, 0x101820);
    assert_eq!(palette.sidebar_text, 0x101820, "正文文字统一设置");
    assert_eq!(palette.muted_text, 0x8899aa);
    assert_eq!(palette.menu_item_active, 0x101820);
    // 只写 RGB 的透明色会补上基础配色的透明度，而不是变成全透明。
    assert_eq!(palette.border & 0xFF, Palette::dark().border & 0xFF);
    assert!(palette.border > 0x00ff_ffff);
}

#[test]
fn theme_store_saves_and_loads_from_disk() {
    let dir = temp_dir("disk");
    let file = dir.join("themes.txt");
    let mut store = ThemeStore::default();
    store.upsert(CustomTheme {
        name: "深蓝".to_string(),
        dark: true,
        colors: COLORS.iter().map(|_| 0x0b1a2b).collect(),
    });
    store.save_to(&file).expect("保存主题文件");
    assert_eq!(ThemeStore::load_from(&file).themes(), store.themes());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_store_writes_a_commented_header() {
    let text = ThemeStore::default().to_text();
    assert!(text.starts_with("# Q-IDE 自定义主题文件"));
    assert!(text.contains("# window_bg=#0B1A2B"), "应给出手工编辑的示例");
    assert!(ThemeStore::parse(&text).themes().is_empty());
}

#[test]
fn upsert_replaces_the_same_name_and_remove_deletes_it() {
    let mut store = ThemeStore::default();
    let mut theme = CustomTheme {
        name: "深蓝".to_string(),
        dark: true,
        colors: COLORS.iter().map(|_| 0x111111).collect(),
    };
    store.upsert(theme.clone());
    theme.colors[0] = 0x222222;
    store.upsert(theme.clone());
    assert_eq!(store.themes().len(), 1);
    assert_eq!(store.find("深蓝").unwrap().colors[0], 0x222222);

    assert!(store.remove("深蓝"));
    assert!(!store.remove("深蓝"));
    assert!(store.themes().is_empty());
}
