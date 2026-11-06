//! 构建脚本：把 Windows 图标资源编进可执行文件。
//!
//! gpui 的 Windows 后端按资源 id 1 读窗口图标（`app.rc` 里的 `1 ICON`），
//! 所以不做这一步时窗口与任务栏会显示系统默认图标。

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    // 非 Windows 平台上这个函数什么都不做。
    let _ = embed_resource::compile("app.rc", embed_resource::NONE);
}
