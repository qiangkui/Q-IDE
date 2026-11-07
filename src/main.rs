//! GPUI Kit 代码编辑器示例。
//!
//! 代码按职责拆分：
//! - [`app`]：主窗口布局与标签页状态；
//! - [`completion`] / [`lint`]：VB 脚本的补全与实时语法检查；
//! - [`encoding`]：文本文件编码探测（UTF-8/UTF-16/GBK）与按原编码写回；
//! - [`editor_pane`] / [`explorer`] / [`output`] / [`status_bar`] / [`title_bar`]：各界面区域；
//! - [`panels`]：Dock 面板（编辑器、资源管理器、代码大纲、搜索、输出）；
//! - [`search`]：项目内文本搜索的匹配模型与搜索面板渲染；
//! - [`menu`]：顶部菜单；
//! - [`settings`]：设置模型与设置窗口；
//! - [`theme`]：配色与尺寸常量；
//! - [`workspace`]：标签页模型与语言推断；
//! - [`window_frame`]：Windows 窗口外框处理。

// 桌面程序不需要控制台窗口：不加这个属性时，双击 exe 会额外弹出一个黑色控制台。
// 这是 Windows 专属属性，其它平台会忽略。
#![windows_subsystem = "windows"]

// 资源管理器的模型测试放在独立文件里：写在 explorer.rs 的 tests 子模块中会
// 触发 rustc 的宏展开递归上限。
mod app;
mod assets;
mod brackets;
mod commands;
mod completion;
mod custom_theme;
mod editor_pane;
mod editing;
mod encoding;
mod explorer;
mod interpreter;
mod layout;
mod lint;
mod menu;
mod outline;
mod output;
mod panels;
mod panel_drag;
mod search;
mod settings;
mod status_bar;
mod syntax;
mod theme;
mod title_bar;
mod window_frame;
mod workspace;

#[cfg(test)]
mod editor_pane_tests;

#[cfg(test)]
mod assets_tests;

#[cfg(test)]
mod custom_theme_tests;

#[cfg(test)]
mod explorer_tests;

#[cfg(test)]
mod settings_tests;

#[cfg(test)]
mod commands_tests;

#[cfg(test)]
mod completion_tests;

#[cfg(test)]
mod debug_tests;

#[cfg(test)]
mod dialog_tests;

#[cfg(test)]
mod shell_tests;

#[cfg(test)]
mod builtins_tests;

#[cfg(test)]
mod encoding_tests;

#[cfg(test)]
mod sema_tests;

#[cfg(test)]
mod lint_tests;

#[cfg(test)]
mod outline_tests;

#[cfg(test)]
mod output_tests;

#[cfg(test)]
mod workspace_tests;

#[cfg(test)]
mod search_tests;

#[cfg(test)]
mod editing_tests;

#[cfg(test)]
mod panels_tests;

#[cfg(test)]
mod brackets_tests;

#[cfg(test)]
mod shortcut_tests;

use gpui_kit::*;

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            // Must be called before using any GPUI Kit / Component features.
            gpui_kit::init(cx);
            app::init(cx);

            // 先建立全局设置与自定义主题库，并用布局文件里记住的主题开窗，
            // 这样窗口外框的第一帧就是上次的主题底色（自定义主题也包括在内）。
            settings::SettingsStore::init(cx);
            custom_theme::ThemeStore::init(cx);
            let layout = layout::Layout::load();
            settings::SettingsStore::update(cx, |settings| {
                settings.appearance.theme = settings::ThemeChoice::from_str(&layout.theme);
            });
            theme::apply_from_settings(cx);

            // 上次关闭时的窗口位置与大小；没有记录或数据不合理时用默认值。
            let window_bounds = layout
                .window_bounds
                .map(|(x, y, width, height)| {
                    WindowBounds::Windowed(Bounds::new(
                        point(px(x), px(y)),
                        size(px(width), px(height)),
                    ))
                })
                .unwrap_or_else(|| {
                    WindowBounds::Windowed(Bounds::new(
                        point(px(120.0), px(90.0)),
                        size(px(960.0), px(600.0)),
                    ))
                });

            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some(title_bar::APP_TITLE.into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(9.0), px(9.0))),
                }),
                window_bounds: Some(window_bounds),
                window_min_size: Some(size(px(480.0), px(320.0))),
                // 交给系统自带的边框处理缩放，外沿只有 1px 边框。
                is_resizable: true,
                window_background: WindowBackgroundAppearance::Opaque,
                ..Default::default()
            };

            gpui_kit::open_window(options, cx, |window, cx| {
                let background = theme::Palette::for_theme(cx).window_bg;
                window_frame::setup(window, background);
                cx.new(|cx| app::CodeWindow::new(window, cx))
            })
            .expect("Failed to open window");
        });
}
