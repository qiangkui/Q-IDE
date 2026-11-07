//! 应用自带资源的测试：标题栏图标确实被编进了 exe，组件图标仍然可用。

use crate::assets::Assets;
use gpui_kit::AssetSource;

#[test]
fn bundled_app_icon_is_available() {
    let data = Assets
        .load("app-icon.png")
        .expect("读取资源")
        .expect("标题栏图标应存在");
    assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n", "应当是 PNG 文件");
    assert!(data.len() > 100, "图标数据不像完整的图片");
}

#[test]
fn unknown_paths_fall_through_to_the_icon_bundle() {
    let data = Assets
        .load("icons/search.svg")
        .expect("读取组件图标")
        .expect("组件图标应仍然可用");
    assert!(!data.is_empty());
}
