# 项目说明

Rust + GPUI 桌面代码编辑器，内置 VB6/VBA 风格脚本解释器。

- `src/interpreter/`：词法、语法、运行时
- `src/app.rs`：主窗口装配（模型接线、菜单、对话框、布局保存）
- `src/explorer.rs`、`src/layout.rs`：资源管理器与布局持久化
- 界面文案、注释使用中文

# 验证分工（重要）

- **界面/交互效果由人工验证。** 只需写代码，并保证 `cargo build` 与 `cargo test` 通过后交付，然后说明需要人工确认的点即可。
- **不要**用截图、模拟鼠标键盘（SetCursorPos / SendInput）等方式去验证 UI：既慢又容易点到用户正在使用的其他窗口。
- 遇到界面问题，等用户反馈后再改。

# 常用命令

```powershell
cargo build
cargo test          # 二进制 crate，`cargo test --lib` 会报 no library targets
cargo clippy --all-targets
```

# 代码约定

- 只改与需求直接相关的代码；发现无关的既有缺陷时单独指出，不要顺手修改。
- 新增测试优先放在 crate 级独立文件（如 `src/explorer_tests.rs`），并在 `src/main.rs` 用 `#[cfg(test)] mod ...;` 引入。放在模块内部写 `#[cfg(test)] mod tests` 会触发 rustc `recursion limit while expanding #[test]`。
- 持久化/模型类逻辑要有单元测试；纯渲染分支不需要测试。

# 环境注意

- 源码为 UTF-8。**禁止**用 PowerShell 的 `Get-Content` / `Set-Content` / `Out-File` 读写源码：Windows PowerShell 5.1 按 GBK 解码会破坏中文，`-Encoding utf8` 又会写入 BOM。改用编辑工具或 .NET `UTF8Encoding($false)`。
- 屏幕为 125% DPI 缩放（窗口 DPI 120），涉及像素坐标时需按物理像素换算。
