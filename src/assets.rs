//! 应用自己的资源：把 `assets/` 里的图片一起编进 exe。
//!
//! gpui-kit 的图标资源仍由 [`IconAssets`] 提供，这里只在它前面补上应用自己的文件，
//! 标题栏图标就不会因为换掉默认资源包而丢失。

use std::borrow::Cow;

use gpui_kit::AssetSource;
use gpui_kit::assets::Assets as IconAssets;

/// `assets/` 目录里的图片；未命中的路径交给 gpui-kit 的资源包。
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        match path {
            // 标题栏左上角的程序图标。
            "app-icon.png" => Ok(Some(Cow::Borrowed(include_bytes!("../assets/icon.png")))),
            _ => IconAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<gpui_kit::SharedString>> {
        IconAssets.list(path)
    }
}
