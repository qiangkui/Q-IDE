# Q-IDE

用 Rust + GPUI（`gpui` / `gpui-kit`）写的 Windows 桌面代码编辑器，内置一个 **VB6/VBA 风格脚本解释器**：用 VB 语法写小脚本、`Declare` 直接调用 Win32 API，并在编辑器里打断点、单步、看变量。

界面文案为中文，当前版本 `0.1.0`，只在 Windows 后端构建验证过。许可证：[MIT](LICENSE)。

## 界面预览

自绘标题栏与菜单、资源管理器、VB 语法高亮、底部输出面板与状态栏：

![主界面](assets/主界面.PNG)

代码大纲面板：按「断点 / 监视 / 过程与函数 / 变量」分组，分区与作用域可折叠，顶部过滤框按名字筛选符号：

![调试窗口](assets/调试窗口.PNG)

## 功能特性

### 编辑器

- 多标签：每个标签独立保留光标、选区、滚动位置、折叠与撤销历史；标签右键菜单支持关闭其它/关闭右侧/关闭全部/复制路径。
- 语法高亮：Visual Basic（`.vb` `.vbs` `.bas` `.cls` `.frm`）、Rust（`.rs`）、JSON，其它文件按纯文本。
- 括号配对高亮（自定义实现），行号栏显示断点圆点，调试暂停行整行淡黄高亮。
- 编辑增强：上移行（`Alt+↑`）/ 下移行（`Alt+↓`）/ 切换注释（`Ctrl+/`）/ `Tab`、`Shift+Tab` 缩进 / 回车自动缩进 / 自动闭合括号。
- 编辑器内嵌查找与替换面板（`Ctrl+F` / `Ctrl+H`），标签下方固定一行显示当前文件的完整路径。
- 编码：自动探测 UTF-8 / UTF-16LE / UTF-16BE / GBK（有 BOM 优先），保存时按原编码写回，状态栏显示当前编码。
- 自动保存：在设置里开启并设置间隔（5–600 秒，默认关闭），按原编码写回，不移动光标、不打断输入。
- 外部修改检测：每秒比对文件修改时间；没有未保存修改时静默重新加载，有修改时弹窗询问。
- 会话恢复：退出时记住打开的标签、激活文件、窗口位置大小、Dock 布局与所有设置。

### 工作区

- 资源管理器：多根目录、新建文件/文件夹、重命名、删除、复制路径、刷新；条目可拖到文件夹里移动，也可拖到标签栏直接打开。
- 项目内搜索（`Ctrl+Shift+F`）：大小写敏感、全词匹配、限定目录，后台线程扫描并实时显示进度；结果按文件分组、命中片段高亮，点击跳转。
- 跨文件替换：只作用于本次搜索结果里的文件，沿用本次搜索的匹配选项。
- 代码大纲：符号按模块/过程/类分组、可折叠、带过滤框；另有「断点」「监视」两节，调试暂停时显示变量值与类型。
- 跳转：转到定义（`F12`）、查找引用（`Shift+F12`）、转到行/符号（`Ctrl+G`）、转到文件（`Ctrl+P`，按文件名或路径模糊匹配）。
- 命令面板（`Ctrl+Shift+P`）：38 条命令，模糊匹配、回车执行，结果写进输出面板。
- Dock 布局：编辑器居中、输出固定底部；左右两侧各最多放一个面板（资源管理器/代码大纲/搜索/AI 编程）。打开面板时，它偏好的那一侧空着就放那里，否则借用另一侧的空位，两侧都占满时替换偏好侧的面板；拖到已占用的一侧会关闭原面板。完整布局写入布局文件。
- 主题：设置窗口「主题」页选择内置深色 / 浅色（只读）、已保存的自定义主题或「新建主题」；自定义主题可改名、改颜色（取色器或手填十六进制），改动直接替换同名主题并写入 `themes.txt`。设置窗口按分类分页（布局 / 主题 / 排版 / 保存 / 语法分析器 / 输出 / AI），点左侧导航直接切换。
- AI 设置：设置窗口「AI」页里先是 **DeepSeek** 分组（以后接入其它模型时按模型分组），组内填 API 密钥、接口地址（默认 `https://api.deepseek.com`）与模型（默认 `deepseek-chat`）。密钥一行是「标签在前、输入框在后」，内容始终遮罩，只存进 Windows 凭据管理器，不写进 `layout.txt`；清空输入或点「清除」即删除。
- AI 对话：右侧 AI 编程面板里输入需求，回车或点「发送」发给 DeepSeek（OpenAI 兼容的 `/chat/completions`，`stream: true`）。发送时会**自动带上当前打开文件的路径与内容**（文件超过 4 万字符时在整行末尾截断并注明总长度与后续读取方式）。**请求长度有上限**：对话越积越多时先丢最早的那几轮（带「已省略」标记），仍然过长就把最长的工具输出截断（带「已截断」标记），面板里保存的对话始终是完整原文。回答**边生成边显示**：用户消息靠右、AI 消息靠左，各是一个圆角气泡；回复按 Markdown 渲染，代码块单独成块（更深底色 + 圆角边框，右上角浮着「复制」「写入」「展开/收起」，默认只露前 5 行）；消息文字可拖选，`Ctrl+C` 复制。标题栏右侧的「＋」开始新对话（清空内容与输入框、滚动回顶部、焦点回输入框并取消进行中的请求），「清空」只清掉对话内容、输入框里的草稿保留。
- AI 固定规则：程序目录下的 `ai-rules/`（启动时自动补齐默认文件，不覆盖用户修改）。`common.md` 总是加载；VB 脚本加载 `vb.md`，Rust 源码加载内置的 `rust.md`，其它语言加载 `<语言>.md`（也可自行添加 `json.md`），没有就 `default.md`。每次请求重新读取，改完下一条消息生效；`QIDE_AI_RULES_DIR` 可改用其它目录。
- AI 工具调用：模型可以自己调工具把活干完——`read_file` / `list_files` 看代码，`replace_in_file`（精确替换唯一片段）或 `write_file` 改文件，`check_syntax` 自查语法，`read_output` 看运行输出；用户明确要求运行或调试时，还可调用 `run_script` / `start_debug` / `debug_continue` / `debug_step` 执行脚本，使用 `set_breakpoint` 设置普通或条件断点，使用 `get_debug_state` 查看暂停行、调用栈和变量值，并用 `add_watch` 添加监视表达式。**交互方式、工具轮数与模型都在 AI 面板底栏（聊天输入框下方）切换**：左边按钮选「执行前确认」（默认，写文件或启动/推进脚本前弹确认卡片）或「全部允许」；旁边按钮选「6 轮」（默认）/「12 轮」/「一直执行」；右边按钮选当前用的 AI 模型（`deepseek-chat` / `deepseek-reasoner`，也可显示设置里的自定义模型）。生成过程中「发送」旁边会出现「停止」，可中断网络请求、工具循环或待确认的操作；停止后未完成的工具调用会补一条「已停止」结果，保证下一轮对话仍符合接口协议。推理模型 `deepseek-reasoner` 不支持函数调用，选中它时只聊天、不调用工具。路径限制在项目根目录内。
- AI 写入文件：代码块右上角的「写入」会弹出路径输入框（默认是当前打开的文件，相对路径按项目根目录解析），确认后**用这段代码覆盖目标文件的全部内容**：文件已打开就直接替换编辑器内容并按原编码保存，没打开就写到磁盘再打开，同时刷新资源管理器与输出日志。

### 运行与调试

- `F5` 运行当前 VB 脚本，「检查语法」只解析不执行，「运行参数…」的内容由 `Command()` 读取。
- 断点（`F9`）、条件断点、清除所有断点（`Ctrl+Shift+F9`）；调试运行（`Ctrl+F5`）、继续（`F11`）、单步（`F10`）、运行到光标处（`Ctrl+F10`）、停止脚本（`Shift+F5`）。
- 暂停时可查看变量快照（值 + 类型）、调用栈、监视表达式求值结果；「添加监视…」的表达式会持久化。
- 输出面板：单击「第 N 行…」跳转到对应代码、相邻重复错误合并计数、关键字过滤与「只看错误」、时间戳、最大保留行数。

### 脚本解释器

完整清单见 [docs/脚本语言.md](docs/脚本语言.md)，要点：

- 声明与作用域：`Dim` / `Static` / `Const` / `Public` / `Private`，`Option Explicit` / `Option Base` / `Option Compare`；`#Include "文件.vb"` 引入公共结构体 / 常量 / API 声明（Q-IDE 扩展；先查脚本目录，再查 exe 同级 `assets/`）。
- 过程：`Sub` / `Function` / `Optional` 默认值 / `ParamArray` / 模块级与类内 `Property Get|Let|Set`。
- 控制流：`If` / `For` / `For Each` / `Do ... Loop` / `While ... Wend` / `Select Case` / `Exit` / `GoTo` 与标签 / `Stop`。
- 数据结构：多维数组与 `{1, 2, 3}` 数组字面量、`ReDim Preserve`、`Erase`、`Type ... End Type` 记录、`Enum`、`With`。
- 对象：`Collection`、`Dictionary`、自定义 `Class`（构造函数 / 析构、字段、方法、属性、`WithEvents` + `RaiseEvent` 事件）。
- 错误处理：`On Error Resume Next` / `On Error GoTo` / `Resume`、`Err.Number|Description|Source`、`Err.Raise`。
- 文件 I/O：`Open ... For Input|Output|Append|Binary|Random`、`Print #`、`Write #`、`Line Input #`、`Input #`、`Get #`/`Put #`、`Seek #`、`Lock`/`Unlock`、`EOF`/`LOF`/`Loc`/`FreeFile`。
- Win32：`Declare ... Lib "user32"`（`ByVal`/`ByRef` 结构体、整数与浮点参数），`AddressOf` 把 VB 过程当回调传给 API（同步回调）。
- 90 多个内置函数：字符串、数值、类型转换、日期、文件系统、`Shell` / `Command` / `MsgBox` / `InputBox` 等。

### 主程序命令行（`first.exe`）

界面版也能接住命令行里的路径，所以 `.vb` 的“打开方式”既可以是 `qide-run.exe`（只跑不编辑），也可以是 `first.exe`（打开编辑器）：

```powershell
first.exe                      # 按上次保存的布局启动
first.exe D:\脚本\demo.vb       # 启动并打开这个文件（所在文件夹自动加进资源管理器）
first.exe D:\脚本 e.vb         # 多个路径按顺序处理：文件开成标签页，文件夹加进资源管理器
first.exe --help               # 用法说明（有控制台就输出到控制台，否则弹消息框）
first.exe --version            # 版本号，例如 Q-IDE 0.1.0
```

- `--` 之后的参数一律当作路径；未知选项会提示用法并以退出码 `2` 结束。
- 路径不存在时在“输出”面板报错，其余路径照常打开。

### 命令行运行（`qide-run`）

不开界面也能跑脚本，供批处理、文件关联和外部 agent 调用（`cargo build` 会同时产出 `target\debug\qide-run.exe`）：

```powershell
qide-run demo.vb                   # 只给路径就等于 run（文件关联用）
qide-run run  demo.vb              # 跑脚本：脚本输出走 stdout，诊断走 stderr
qide-run run  demo.vb --json       # 每行一个 JSON 事件，交给程序 / agent 解析
qide-run check demo.vb --json      # 只做语法与语义检查，不执行
```

- 常用选项：`--json`、`-q/--quiet`、`--max-steps N`、`--max-depth N`、`--timeout 30s`（也接受 `500ms`、`2m`）、`--include-dir DIR`、`--arg 文本`、`--ui/--no-ui`、`--pause/--no-pause`。
- **双击 / 文件关联**：把 `.vb` 的打开方式指向 `qide-run.exe`，命令行写 `"...\qide-run.exe" "%1"` 即可。检测到「独占控制台且 stdout 没被重定向」时，会自动允许 `MsgBox`/`InputBox` 弹窗，并在结束时停住等回车（窗口不会一闪就没）；从终端或 agent 调用（输出被管道接走）时不会暂停。
- 退出码：`0` 成功、`2` 语法/语义问题、`3` 运行时错误、`4` 超时或 Ctrl+C、`5` 读不到脚本或内部错误、`64` 参数用法错误。
- `--json` 的结束事件带结构化错误：`{"type":"finished","ok":false,"kind":"runtime","number":6,"line":12,"message":"溢出"}`。
- 相对路径按脚本所在目录解析（与编辑器里运行一致），控制台代码页会切到 UTF-8。
- `qide-run.exe` 只链接解释器，不链接界面，体积约 1.7 MB（GUI 版约 32 MB）；图标是浅绿色（`assets/run.ico`），与深蓝色的 `first.exe` 一眼可分。

## 界面布局

```text
┌ 自绘标题栏：文件 编辑 选择 查看 转到 运行 帮助 ────────── 最小化 最大化 关闭 ┐
│ 活动栏 │ 左侧面板（最多一个） │ 编辑区（标签栏/路径栏/代码） │ 右侧面板（最多一个） │
│ 图标   │                      │                              │                      │
│ 竖排   │                      │                              │                      │
│        │                      │                              │                      │
├────────┴──────────────────────┴──────────────────────────────┴──────────────────────┤
│                                                                                     │
│ 输出面板（可拖动、可隐藏）                                                          │
│                                                                                     │
├─────────────────────────────────────────────────────────────────────────────────────┤
│ 状态栏：输出开关 · 编码 · 调试状态 · 行/列 · 错误数 · 输出行数                      │
└─────────────────────────────────────────────────────────────────────────────────────┘
```

## 环境要求

| 项目 | 要求 |
| --- | --- |
| 操作系统 | Windows 10 / 11（只在 Windows 后端验证过） |
| Rust | 1.85+（`edition = "2024"`；开发机使用 1.99.1） |
| 工具链 | MSVC（`stable-x86_64-pc-windows-msvc`）+ Windows SDK，`build.rs` 通过 `embed-resource` 把 `app.rc` 里的图标编进 exe |
| 网络 | `.cargo/config.toml` 默认把 crates.io 指到 rsproxy.cn 镜像，可换回官方源 |

## 构建与运行

```powershell
git clone git@github.com:qiangkui/Q-IDE.git
cd Q-IDE
cargo build      # 产物：target\debug\first.exe（界面）与 target\debug\qide-run.exe（命令行运行器）
cargo run
```

`Cargo.toml` 里的包名仍是早期的 `first`，所以可执行文件叫 `first.exe`；窗口标题与任务栏名称取自 `title_bar::APP_TITLE`，即 **Q-IDE**。

`[patch.crates-io]` 把 `gpui-component` 指向 `vendor/gpui-component`（本地补丁：菜单字号更小、取消斜体；设置项说明文字比标题小一档），因此必须连同 `vendor/` 一起克隆，不能只复制 `src/`。

## 上手

1. 打开程序，「资源管理器 → `+` 添加根目录」选一个文件夹。
2. 「新建文件」建一个 `.vbs`（或 `.bas`）脚本。
3. `F5` 运行，结果显示在输出面板；`Ctrl+F5` 用调试方式运行。

```vb
Option Explicit

Dim i As Long, total As Long
For i = 1 To 5
    total = total + i
Next i
Debug.Print "1..5 合计 " & total

Dim names As New Collection
names.Add "甲"
names.Add "乙"
Debug.Print "名字个数：" & names.Count
```

涉及文件的脚本里，相对路径按**脚本所在目录**解析（不是编辑器进程的当前目录）；`App.Path` 返回脚本目录，`Command()` 返回「运行参数…」里填的字符串。

## 常用快捷键

| 快捷键 | 功能 |
| --- | --- |
| `Ctrl+N` / `Ctrl+O` / `Ctrl+S` / `Ctrl+Shift+S` | 新建文件 / 打开文件 / 保存 / 另存为 |
| `Ctrl+Alt+S` / `Ctrl+Shift+T` | 保存全部 / 重新打开关闭的标签 |
| `Ctrl+F` / `Ctrl+H` | 编辑器内查找 / 替换 |
| `Ctrl+/`、`Alt+↑`、`Alt+↓` | 切换注释 / 上移行 / 下移行 |
| `Ctrl+P` / `Ctrl+G` / `Ctrl+Shift+P` | 转到文件 / 转到行或符号 / 命令面板 |
| `Ctrl+Shift+F` | 项目内搜索面板 |
| `F12` / `Shift+F12` | 转到定义 / 查找引用 |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | 下一个 / 上一个标签 |
| `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | 放大 / 缩小 / 重置界面字号 |
| `F5` / `Shift+F5` / `Ctrl+F5` | 运行 / 停止 / 调试运行 |
| `F9` / `Ctrl+Shift+F9` | 切换断点 / 清除所有断点 |
| `F11` / `F10` / `Ctrl+F10` | 继续 / 单步 / 运行到光标处 |

完整快捷键与命令面板清单见 [docs/快捷键.md](docs/快捷键.md)。

## 项目结构

| 路径 | 说明 |
| --- | --- |
| `src/main.rs` | 入口：解析命令行（`src/startup.rs`）、初始化 GPUI、设置与主题，按上次记录的窗口矩形开窗 |
| `src/startup.rs` | 主程序命令行：文件/文件夹路径、`--help`、`--version`，以及没有控制台时的消息框兜底 |
| `src/app.rs` | `CodeWindow` 主体：窗口构造、共享状态、设置同步与通用辅助函数 |
| `src/app/*.rs` | 子模块：`ai`（AI 写入文件）、`dock`（面板停靠）、`debug` / `breakpoints`（运行调试与断点）、`editor`（导航与编辑动作）、`files` / `session`（文件与会话）、`dialogs`（通用对话框）、`view`（界面渲染） |
| `src/interpreter/` | VB 解释器：`lexer` / `parser` / `ast`、`runtime`、`ffi`、`builtins`、`debug`、`sema`、`dialog`、`filelock`、`trampoline` |
| `src/bin/qide-run.rs` / `src/cli.rs` | 命令行运行器：只链接解释器与编码模块（不链接界面），提供 `run` / `check` 子命令、JSONL 事件与退出码 |
| `src/panels.rs` | Dock 面板注册、面板标题栏按钮、面板右键菜单项 |
| `src/ai/client.rs` | DeepSeek 网络层：地址解析、请求体（含 tools）/应答解析、WinHTTP 流式调用；不依赖界面，可整体移植 |
| `src/ai/stream.rs` | SSE 增量解析（纯逻辑，可单测）：字节流 → 文本增量 / 工具调用增量 |
| `src/ai/markdown.rs` | 极简 Markdown 分段：把回复拆成说明文字与围栏代码块（纯逻辑） |
| `src/ai/tools.rs` | 工具层：工具清单（JSON Schema）+ 把一次调用算成只读结果或待写入文件（路径限制在项目内） |
| `src/ai/panel.rs` | AI 编程面板（GPUI）：对话气泡、工具活动、写入确认、代码块折叠/展开/复制、流式追加、输入框、关闭与拖动 |
| `src/editor_pane.rs` | 编辑区面板：标签栏、路径栏、编辑器右键菜单 |
| `src/explorer.rs` | 资源管理器模型：根目录、展开状态、拖拽、重命名/删除 |
| `src/outline.rs` | 代码大纲、符号解析、转到定义 / 查找引用 |
| `src/search.rs` | 项目内搜索与跨文件替换的纯逻辑 |
| `src/layout.rs` | `layout.txt` 的读写与版本迁移 |
| `src/custom_theme.rs` | 自定义主题模型与 `themes.txt` 读写 |
| `src/settings/` | 设置模型（`SettingsStore`）与设置窗口（含「AI」页） |
| `src/secret_store.rs` | API Key 存取：Windows 凭据管理器（不进任何文本文件） |
| `src/menu.rs` / `src/title_bar.rs` / `src/window_frame.rs` | 顶部菜单、自绘标题栏、窗口外框 |
| `src/theme.rs` | 内置深/浅配色、自定义主题解析结果与尺寸常量 |
| `src/*_tests.rs` | crate 级测试文件（在 `src/main.rs` 里用 `#[cfg(test)] mod ...;` 引入） |
| `assets/`、`app.rc`、`run.rc`、`build.rs` | 图标：`first.exe` 用深蓝的 `app.ico`，`qide-run.exe` 用浅绿的 `run.ico` |
| `vendor/gpui-component/` | gpui-component 本地补丁 |
| `docs/` | 项目文档 |

## 配置与数据

- 布局、会话与设置：可执行文件同目录的 `layout.txt`（`dock_state`、`window_bounds`、`open`/`active`、`recent`、`breakpoint`、`breakpoint_condition`、`watch`、`script_args` 等键）。安装目录不可写时（如放在 `Program Files`）退回 `%APPDATA%\gpui-first\layout.txt`。
- 旧版本保存在 `%APPDATA%\gpui-first\layout.txt` 的配置，会在首次启动时自动搬到 exe 旁边并删除旧文件。
- 自定义主题：同样放在可执行文件同目录的 `themes.txt`，每个主题以 `theme=名称` 开头，`base=dark|light` 指定基础配色，其余为 `#RRGGBB` / `#RRGGBBAA` 颜色键；删掉某一段即删除该主题。设置窗口「主题」页里：选内置主题时名称与颜色只读；选已保存的主题时改名称/颜色会直接替换该主题（改名即改名，不新增）；「新建主题」以当前主题的配色为底、名字留空，保存后成为新条目。颜色可用输入框右侧的取色器，也可直接输入十六进制。
- 设置窗口的大小也记在 `layout.txt` 的 `settings_size` 键里，下次打开恢复上次的尺寸。
- AI 设置里的接口地址与模型记在 `layout.txt` 的 `ai_base_url` / `ai_model` 键里；**API Key 不写文件**，只存进 Windows 凭据管理器（目标名 `Q-IDE/DeepSeek API Key`），由 `src/secret_store.rs` 读写。
- 布局文件是纯文本，可以直接编辑或删除；删除后恢复默认布局。文件带 `layout_version`：旧版本的 `dock_state` 会被丢弃并按默认布局重建，其余字段保留，旧文件可直接升级。
- 布局与设置都放在设置窗口里改，改完立即写盘；主题颜色也可以直接在 `themes.txt` 里手工编辑。

## 文档

| 文档 | 内容 |
| --- | --- |
| [docs/脚本语言.md](docs/脚本语言.md) | VB6/VBA 子集参考：词法、类型、语句、内置函数、Win32 调用、限制 |
| [docs/快捷键.md](docs/快捷键.md) | 全部快捷键、菜单结构、命令面板 37 条命令 |
| [docs/架构.md](docs/架构.md) | 模块职责、运行/调试数据流、持久化格式、测试与开发约定 |
| [ROADMAP.md](ROADMAP.md) | 已完成功能清单、坑位备忘、后续计划 |
| [.github/copilot-instructions.md](.github/copilot-instructions.md) | 协作约定（验证分工、编码约束、环境注意） |

## 开发与验证

```powershell
cargo build              # 通过：first.exe（界面）+ qide-run.exe（命令行）
cargo test               # 通过：界面 crate 520 条 + qide-run crate 170 条（含解释器自带测试）
cargo clippy --all-targets
```

- 这是二进制 crate，没有 lib target，别用 `cargo test --lib`。
- `qide-run` 用 `#[path = "../interpreter/mod.rs"]` 直接包含解释器源码，所以解释器自带的测试会在两个 crate 里各跑一遍（互不影响）。
- 新增测试放 crate 级文件（如 `src/xxx_tests.rs`），在 `src/main.rs` 里用 `#[cfg(test)] mod xxx_tests;` 注册；写在模块内部的 `#[cfg(test)] mod tests` 会触发 rustc 的宏展开递归上限。
- 界面与交互需要人工验证，自动化测试只覆盖模型与纯逻辑。
- 源码是 UTF-8：不要用 Windows PowerShell 5.1 的 `Get-Content` / `Set-Content` / `Out-File` 读写源码（前者按 GBK 解码，后者会写 BOM）。

## 已知限制

- 只支持 Windows 后端；界面与窗口处理有 Windows 专属代码（`#![windows_subsystem = "windows"]`、`app.rc`/`run.rc` 图标、自绘窗口外框）。
- 解释器支持基础 COM Automation（`CreateObject`、活动对象/新对象/moniker `GetObject`、文件和 ProgID `GetObject`、位置/命名参数方法与属性读写）；命名参数要求 COM 组件通过 `IDispatch` 暴露形参名，可写标量/字符串变量按 `ByRef VARIANT` 传递，一维 Variant 数组按值转换为 `SAFEARRAY`；多维/强类型数组、UDT 记录封送、COM 事件及 `Implements` 尚不支持。文件中的 `GetObject` 依赖已注册且支持 `IPersistFile` 的类。`Currency` 使用四位小数定点数；`Date` 是独立类型，内部以 OLE Automation `Double` 序列存储。
- Win32 调用最多支持 64 个标量参数；结构体里的变长 `String` 仍不支持。
- `AddressOf` 回调最多支持 64 个参数，需在脚本仍运行的同一线程执行；其他线程的回调会返回默认值。可在 API 调用返回后由脚本持有回调指针，但脚本结束后不会再进入解释器，数组形参因原生指针不包含长度/边界信息而不支持。
- 资源管理器暂不支持多选（受当前树组件的行选中态限制）。

## 声明

- **版权与许可**：Q-IDE 的源码与文档版权归作者（qiangkui）所有，© 2026。本项目以 **MIT 许可证**发布，全文见 [LICENSE](LICENSE)：可以自由使用、修改、分发与商用（包括闭源分发），只需保留版权声明与许可声明。
- **按现状提供**：本项目为个人学习与示例性质的项目，按「现状」提供，不附带任何明示或暗示的担保（包括适销性、特定用途适用性与不侵权担保）。因使用本项目而产生的任何直接或间接损失，作者不承担责任（详见 MIT 许可证原文）。
- **脚本执行风险**：内置解释器可以调用 Win32 API（`Declare`）、读写本地文件、用 `Shell` 启动外部程序。请**只运行你信任的脚本**，不要执行来源不明的 `.vb` / `.vbs` / `.bas` 文件。
- **隐私**：除 AI 功能外，程序不发起网络请求；打开的文件、布局与设置只保存在本机（可执行文件同目录的 `layout.txt`，目录不可写时为 `%APPDATA%\gpui-first\layout.txt`）。DeepSeek 的 API Key 单独存进 Windows 凭据管理器，不进 `layout.txt`、不进日志与输出面板；界面上始终遮罩显示。**AI 对话会把当前打开文件的路径与内容（最多 4 万字符）作为上下文一并发给 DeepSeek**；模型调用 `read_file` / `list_files` / `read_output` 时读到的内容同样会发给它，只想闲聊时可以先切到一个空标签再提问。写入默认要你在对话里点「同意写入」；在 AI 面板底栏把交互方式切成「全部允许」后，AI 会直接改文件（路径仍限制在项目根目录内）。
- **第三方组件**：`gpui`（`gpui-pre`）、`gpui-kit`、`gpui-component`、`tree-sitter`、`tree-sitter-rust`、`encoding_rs`、`lsp-types`、`serde_json`、`embed-resource` 等依赖的版权归各自作者所有，使用与分发请遵守各自的许可证；`vendor/gpui-component` 是对其源码的本地补丁，同样受原项目许可证约束。
