//! 构建脚本：把 Windows 图标资源编进可执行文件。
//!
//! gpui 的 Windows 后端按资源 id 1 读窗口图标（`app.rc` 里的 `1 ICON`），
//! 所以不做这一步时窗口与任务栏会显示系统默认图标。

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    // 非 Windows 平台上这个函数什么都不做。
    let _ = embed_resource::compile("app.rc", embed_resource::NONE);

    // GPUI 按元素树递归绘制界面；主窗口层级较深，AI 面板里的 Markdown 文本
    // 还会在最深处再占一层很大的栈帧，默认 1MB 的主线程栈会在发送消息时溢出。
    // 把保留大小提高到 8MB（提交大小仍按需增长）。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-arg=/STACK:8388608");
        } else {
            println!("cargo:rustc-link-arg=-Wl,--stack,8388608");
        }
    }
}
