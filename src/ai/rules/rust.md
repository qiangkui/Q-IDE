# 当前文件：Rust（Q-IDE 源码）

- 如果当前工作区不是 Q-IDE 源码，忽略下面的项目结构说明，按实际 `Cargo.toml` / `src/main.rs` 判断。

## 项目概况

- Q-IDE 是 Rust + GPUI 的 Windows 桌面编辑器，内置 VB6/VBA 风格脚本解释器；界面文案与注释用中文。
- 二进制 crate（没有 `lib.rs`）：模块在 `src/main.rs` 用 `mod` 声明；测试放在 crate 级 `src/*_tests.rs`，再在 `main.rs` 里用 `#[cfg(test)] mod ...;` 引入。
- 主要目录：`src/interpreter/`（词法/语法/运行时/FFI/调试）、`src/ai/`（DeepSeek 客户端、工具、面板、规则）、`src/app/`（主窗口按 dock/debug/editor/files/session/dialogs/view 拆分）、`src/panels.rs`、`src/explorer.rs`、`src/layout.rs`、`src/settings/`、`src/custom_theme.rs`。
- 改动前先看 `docs/架构.md` 与相关模块的现有写法；不要凭印象调用别的版本 API。

## 改代码

- 先 `read_file` 看清现有实现与调用点，再做最小必要改动；不顺手修无关的旧问题。
- 遵循现有模块边界：持久化模型放 `layout.rs` / `settings` / `custom_theme`，纯逻辑尽量单独成模块，界面渲染留在 `app/` 与 `panels.rs`。
- 保持中文文案与中文注释；新增用户可见文字用简体中文，并与现有措辞风格一致。
- 不编造 `gpui` / `gpui-kit` / `gpui-component` 的 API；不确定时先在同一 crate 或 `vendor/gpui-component` 里找现有用法。
- 不要为了顺手而改 `Cargo.toml` 依赖、`vendor/` 补丁或构建脚本。

## GPUI 约束

- 后台工作（搜索、脚本执行、网络请求）不要阻塞 UI 线程；用 `cx.spawn` / 后台任务，再回到主线程更新实体。
- 不要在实体自己的 `update` 里同步打开或写入会触发布局保存的文件（参考 `AiPanel::spawn_write`），否则 `dump` 面板时会 panic。
- 观察者里设置装饰前先判断值是否真的变化，避免 `notify` 自唤醒死循环（参考 `CodeWindow::bracket_pairs`）。
- `build.rs` 已把 Windows 主线程栈调到 8MB，用来撑住深层 GPUI 渲染；不要移除。

## 持久化

- `layout.txt` / `themes.txt` 是 exe 同目录的逐行 `键=值`（值做百分号转义），未知键直接忽略；新增键要考虑旧版本兼容。
- API Key 只存 Windows 凭据管理器（`secret_store.rs`），绝不写入源码、布局文件、日志或回答。

## 验证

- 依次运行 `cargo build`、`cargo test`、`cargo clippy --all-targets`；这是二进制 crate，`cargo test --lib` 会报 no library targets。
- 持久化、模型与纯逻辑改动要在 `src/xxx_tests.rs` 补测试；纯渲染分支不写测试。
- 界面与交互由人工验证：不要截图，也不要模拟鼠标键盘。
- 源码为 UTF-8：不要用 PowerShell 5.1 的 `Get-Content` / `Set-Content` / `Out-File` 读写；需要命令行处理时用 .NET `UTF8Encoding($false)`。
- 屏幕为 125% DPI（窗口 DPI 120），涉及像素坐标时按物理像素换算。