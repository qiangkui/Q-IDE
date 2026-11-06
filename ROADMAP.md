# 开发进度与后续计划

> 更新时间：2026-11-06。下次接手先看这里，再照「验证约定」跑一遍命令。

## 当前状态

- `cargo build` 通过、`cargo test` **360 passed**（5 ignored）、`cargo clippy --all-targets` **0 警告**。
- 界面/交互效果由人工验证（不要用截图、也不要模拟鼠标键盘去点界面）。

## 已完成的功能（按模块）

### 界面

- 状态栏：输出开关、文件编码、调试状态（暂停时带 `模块 → 过程 → 过程` 调用栈）、光标「行 N，列 M」、错误数、输出行数。
- 自绘标题栏：左侧菜单栏，中间显示程序名 **Q-IDE**（`title_bar::APP_TITLE`，同一常量也用作系统窗口标题 / 任务栏名称，字号 `.text_xs()`），右侧是最小化/最大化/关闭。
- 活动栏：资源管理器 / 代码大纲 / 搜索 / 设置（最底）。资源管理器、代码大纲与搜索面板互斥，同一时间只显示一个。
- Dock：主区域由 gpui-kit 的 `DockArea` + `DockSkin` 管理，编辑器固定在中间，资源管理器/代码大纲可在左侧或右侧 dock，输出固定在底部；面板可以拖动，完整布局写入布局文件的 `dock_state`。拖到中间的侧栏会自动回到左右 dock，避免和输出区域混在一起。拖动侧栏时由主窗口绘制贴合最终 dock 位置的蓝色指示条，不使用 Dock 自带的半区占位框。
- 面板标题栏由 Dock 提供：资源管理器有「新建文件 / 新建文件夹 / 刷新 / + 添加根目录 / 关闭」，代码大纲有「关闭」，输出有「复制 / 清空 / 关闭」；编辑器面板隐藏 Dock 标题栏，保留自己的文件标签栏。
- 路径栏：标签栏下方固定一行显示**当前标签对应文件的完整路径**，底色与代码编辑框相同（都是 `palette.window_bg`），目录部分用灰色、文件名用正文色并保证不被截断（目录太长时只截断目录）。`.when_some` 只在有激活文件时插入，没打开文件时不占位。
- 资源管理器：头部按钮依次是新建文件、新建文件夹、刷新、添加根目录、关闭。新建落在最近点过的文件夹（没点过用第一个根目录，没有任何根目录时先弹文件夹选择框）；刷新重新读盘，外部新增/删除的文件立刻可见。空文件夹会补一个「（空文件夹）」占位行（树组件按「有没有子项」判断是不是文件夹，没有占位项就会显示成文件、也点不开）。**右键任意条目**弹出与编辑器同款的自绘菜单：文件是「打开 / 重命名… / 删除… / 复制路径 / 刷新」，文件夹另有「新建文件 / 新建文件夹」，根项目录还有「从资源管理器移除」。
- 资源管理器的拖动：条目可以拖动，**拖到文件夹上**即移动进去（不能拖进自己或自己的子目录、目标已有同名项会拒绝、打开中的文件先写回再搬），**拖到编辑区标签栏**即打开该文件。暂不提供多选（树行高亮与修饰键判断在这个后端上不稳定，见「已知遗留」）。
- 代码大纲：变量按作用域（模块/过程/类）二次分组；分区与作用域都可折叠，**默认全部收起**，人工点开才展开；调试暂停时变量下方多一行「值 · 类型」（灰色单行省略）；点行跳转到对应代码行。「断点」一节按行号列出当前文件的断点（带该行源码，条件断点用黄色并另起一行显示条件，点击跳转）；「监视」一节列出用户添加的表达式（添加后自动展开），暂停时每条下方显示求值结果。顶部有**过滤框**：按名字忽略大小写筛选符号，过滤时符号分区与作用域分组强制展开、没有匹配的分区隐藏，清空后恢复原状。
- 菜单：
  - 文件：最近打开（最多 5 项，持久化、「清除最近打开」）、保存 / 另存为 / **全部保存（Ctrl+Alt+S）**、重命名当前文件…、删除当前文件…；
  - 编辑：撤销/重做/剪切/复制/粘贴 + **上移行（Alt+↑）/ 下移行（Alt+↓）/ 切换注释（Ctrl+/）** + **查找（Ctrl+F）/ 替换（Ctrl+H）**（组件内置面板）；
  - 查看：面板开关（资源管理器、代码大纲、搜索、输出、状态栏）、主题、全屏；字号用 `Ctrl+=` / `Ctrl+-` / `Ctrl+0`；
  - 转到：转到定义（F12）、查找引用（Shift+F12）、转到行 / 符号（Ctrl+G）、转到文件…（Ctrl+P，按文件名或路径模糊匹配，候选来自已打开的文件、最近打开的文件与资源管理器各根目录下的全部文件）；
  - 运行：运行、检查语法、停止脚本、运行参数…，分隔线后是调试组（调试运行 Ctrl+F5、暂停、继续 F11、单步 F10、切换断点 F9、设置条件断点…、运行到光标处 Ctrl+F10、清除所有断点 Ctrl+Shift+F9、添加监视…、清除所有监视）。
- 输出面板：任何以「第 N 行…」开头的行都能**单击跳转**（运行错误、语法检查结果、查找引用列表）；脚本结束时显示用时（`脚本执行结束（用时 1.23s）`，调试会话包含暂停时间）；连续出现完全相同的错误/警告合并为一行并标注「（重复 N 次）」，普通信息不合并。标题栏右侧有**过滤框**（关键字，忽略大小写）与「只看错误」开关，两者可叠加；被过滤掉的行不会参与渲染、选区与「复制」（复制只拿可见内容），全部被过滤时显示「没有匹配当前过滤条件的输出」。
- 命令面板：`Ctrl+Shift+P`（转到菜单里也有「命令面板…」）输入命令名，按名字模糊匹配后回车执行，共 37 条命令，覆盖文件、编辑、跳转、面板开关、主题与运行/调试；执行结果写进输出（`> 保存（Ctrl+S）`），找不到命令会在输出里提示。
- 外部修改检测：每秒的自动保存循环顺带比对已打开文件的修改时间。没有未保存修改时**静默重新加载**并在输出里记一行；有未保存修改时弹「重新加载 / 取消（保留我的）」，同一处改动只问一次。
- 重新打开关闭的标签：`Ctrl+Shift+T` 或文件菜单，按关闭顺序后进先出（最多 20 个），磁盘上已不存在的会被跳过。
- 编辑器右键菜单：撤销/重做/剪切/复制/粘贴/全选，分隔线后是上移行 / 下移行 / 切换注释 / 增加缩进 / 减少缩进，再后面是切换断点、设置条件断点…、运行到光标处，最后是转到定义、查找引用。菜单由应用自绘（不是系统原生菜单），圆角、边框颜色与底色跟顶部弹出菜单一致。
- 编辑增强：`Tab` / `Shift+Tab` 缩进与反缩进、回车自动缩进由编辑器组件内置（`IndentInline` / `OutdentInline` / smart indent），编辑菜单与右键菜单里也加了对应项；**括号配对高亮**由本应用实现（`brackets::matching_pair` 找配对，两个 `RangeDecoration` 画淡蓝色底），光标贴着 `(`/`[`/`{` 或它们的闭括号时高亮成对的两个括号，字符串与 VB 的 `'` 注释里的括号不算。
- 标签右键菜单（自绘，复用同一套菜单外观）：关闭、关闭其它、关闭右侧、关闭全部、全部保存、复制路径。**重命名/删除搬到了资源管理器条目的右键菜单**（那样能直接作用于没打开的文件与文件夹），文件菜单里另有「重命名当前文件…」「删除当前文件…」。重命名会先把未保存内容写回旧文件再改名（文件、文件夹都支持，目录下已打开的文件、断点、最近打开按前缀一起改写）；删除前弹确认框，删除后关掉受影响标签并清掉断点记录。
- 窗口位置与大小：每秒读一次窗口矩形，变化后写进 `layout.txt` 的 `window_bounds=`，下次启动按它恢复（尺寸过小或坐标离谱的坏数据自动忽略）。
- 会话恢复：退出时把还开着的标签（顺序）与当前激活的那个写进 `layout.txt` 的 `open=` / `active=`，下次启动逐个重新打开（磁盘上已不存在的文件跳过，最多 40 个），并回到当时的激活标签。
- 标签切换：`Ctrl+Tab` / `Ctrl+Shift+Tab` 在打开的文件之间循环，编辑菜单里也有对应项。
- 自动保存：设置 → 编辑器 →「保存」里开关（默认关闭）并设置间隔（5–600 秒）；每秒检查一次，到点后把所有有改动的文件按原编码写回磁盘，输出「已自动保存 N 个文件」，不移动光标、不打断输入。
- 设置持久化：编辑器（字号、换行、行号、折叠、自动闭合、自动保存开关与间隔、语法分析开关与语言）与输出（字号、时间戳、最多保留行数）设置随布局写进 `layout.txt`，重启后继续生效；读取时对超出范围的数值做夹取，缺键沿用默认值（旧布局文件可直接升级）。
- 对话框：运行参数、转到行 / 符号、转到文件、添加监视、条件断点、重命名文件六个输入框对话框 —— 打开即聚焦输入框，回车等同「确定」；删除文件用确认框（取消 / 删除）。
- 搜索面板（活动栏放大镜 / `Ctrl+Shift+F` / 查看 → 搜索）：项目内文本搜索，候选文件与「转到文件」相同（已打开 + 最近打开 + 资源管理器各根目录递归），可以把范围限定到某个文件夹（`选择…`，`×` 恢复全部位置）。关键字输入框回车或点放大镜按当前选项重搜，`Esc` 直接收起面板；`Aa` 区分大小写、`全词` 只匹配完整单词（前后不能是字母/数字/`_`）；结果按文件分组，每行显示行号与命中附近的原文，**命中片段用醒目颜色高亮**，点击打开文件并跳到那一行；扫描在**后台线程**执行，状态行实时显示「搜索中… 已处理/总数 个文件」，结束显示「N 处结果 · M 个文件」，命中达到上限、读取预算用尽、跳过二进制文件或被新搜索取代时另有提示。改动关键字会立即作废旧结果并取消在途搜索，避免列表与输入不符。
  - **跨文件替换**：关键字下面一行是「替换为」输入框 +「全部替换」（回车同样触发）。替换只作用于**本次结果里出现的文件**，匹配条件沿用这次搜索的选项（不是输入框里正在改的文字），行尾原样保留；写完后输出「已替换 N 个文件、共 M 处」，没有未保存修改的已打开标签立刻刷新，有修改的标签不覆盖（留给外部修改检测去问），最后自动重搜一遍。

### 解释器

- 语义检查 `sema.rs`：`Option Explicit` 下的未声明变量、`ReDim` 未声明数组、未定义的函数或过程（`With` 块内跳过，调用名是变量时跳过）。
- 补全 `completion.rs`：文档里的符号优先于关键字/内置函数；**成员补全**（`p.` 后列 UDT 字段、类字段与方法、`Me.`、`New Collection` / `New Dictionary`）。
- 编码 `encoding.rs`：BOM / UTF-8 / UTF-16LE/BE / GBK 探测，保存按原编码写回，状态栏显示。
- 对话框 `dialog.rs`：`MsgBox` / `InputBox` 都自己建窗口 + 自己跑消息循环（理由见「坑位备忘」），支持按钮组语义，**开窗期间点「停止脚本」会立刻关闭**。
- 内置函数：新增 `Shell(path[, windowstyle])`、`Command()`（配合「运行参数…」）。
- 调试：断点（按文件记录、写进布局文件，重启后还在）、暂停/继续/单步、运行到光标处、清除所有断点、暂停快照（变量值 + 调用栈）、暂停行淡黄整行高亮。
- **条件断点**：菜单「运行 → 设置条件断点…」给光标所在行加条件（该行还没有断点会自动补上，留空表示清除条件）；条件在每次执行到该行时求值，为真才停，求值出错时照样停下方便排查；条件断点用黄色圆点区分普通断点；条件按 `breakpoint_condition=` 写进布局文件。断点与条件都列在代码大纲的「断点」一节里，便于查看。
- **变量监视**：菜单「运行 → 添加监视…」添加表达式（光标处的名字会预填），列表显示在「代码大纲 → 监视」一节；每次暂停都重新求值并显示值与类型，表达式出错时显示错误文本，恢复运行时清空结果；监视列表写进布局文件（`watch=` 行），重启后还在。求值不阻塞界面，「停止脚本」能中断求值表达式里的死循环。
- **脚本所在目录**：主窗口把当前文件路径放进 `RunOptions::script_path`。`Open`、`Dir`、`FileLen`、`FileDateTime`、`Kill`、`FileCopy`、`MkDir`、`RmDir`、`Name` 里的相对路径一律按脚本目录解析；`CurDir`/`ChDir` 只改脚本视角的目录，**不动编辑器进程的 CWD**；新增 `App.Path`（返回脚本所在目录，`Option Explicit` 下不误报）。`Open ... For Input` 省略 `As #n` 时会自动取最小空闲文件号（与 `FreeFile` 一致）。
- 其他：转到定义 `outline::definition`、查找引用 `outline::references`、转到行/符号解析 `outline::goto_target`。

## 坑位备忘（改代码前先看）

- gpui 颜色：`hsla(h, s, l, a)` 的四个参数都被夹到 `0..1`，**色相不是角度**（黄色是 `1.0/6.0`）。
- 行号栏 `GutterDecoration` 用空范围会被画成「行边界」（两行之间），必须传整行范围。
- `RangeDecoration` 的 `Fill` 只覆盖文本范围（到行尾 + 一个空格），做不了整行宽度。
- `.truncate()` = 溢出隐藏 + 不换行 + 省略号；flex 子元素还要 `min_w_0()` 才能真正收缩。
- 脚本跑在后台线程，**不是前台线程**：`MessageBoxW` 会被创建在编辑器窗口后面（用户以为卡死），所以 `MsgBox`/`InputBox` 一律自建对话框 + `SetWindowPos(HWND_TOPMOST)`。
- 自建对话框（`CreateDialogIndirectParamW` + 自己的消息循环，`MsgWaitForMultipleObjects` 50ms 超时）才能在「停止脚本」时检查中断标志；`close_dialogs()` 还会给本进程残留的 `#32770` 窗口发 `WM_CLOSE`。
- 调试轮询只在 `(is_paused, pause_epoch)` 变化时才取快照：快照可能几 MB，每 40ms 克隆会让界面发卡。监视求值结果走另一个轻量轮询（`take_eval_result`），一次只发一条请求（`watch_queue` + `watch_request`）。
- 监视求值发生在解释器线程仍停在 `before_line` 里的时候：求值前必须**放开调试锁**，并通过 `Runtime::evaluating` 让表达式内部的语句跳过暂停检查，否则调用脚本过程会再次进入 `before_line` 把自己锁死。条件断点的条件求值走同一条路径（`DebugQuery::Condition`），只是判断真假后接着往下跑而不是停住。
- 条件断点只在「命中断点且不是单步/暂停请求」时求值；条件本身出错时按「真」处理。编辑代码后断点行号由行号栏装饰反推，条件按行号顺序重新挂（数量不变时一一对应，数量变了只保留行号仍存在的条件），这是有意的启发式，别把它当成精确的行映射。
- 输入框单行模式下回车只 `cx.propagate()`，对话框里要自己挂 `.on_action(|_: &Enter, ..|)` 才能回车确认。
- 测试放在 crate 级文件（如 `src/outline_tests.rs`）并在 `src/main.rs` 用 `#[cfg(test)] mod xxx_tests;` 注册；写在模块内的 `#[cfg(test)] mod tests` 会触发 rustc 的宏展开递归上限。
- 输出面板的重复分组只合并**相邻**的同类错误/警告（`OutputLine::repeat`），复制全部输出时带「（重复 N 次）」，拖选复制仍是原始文本。
- 脚本路径：`RunOptions::script_path` → 运行时 `script_dir`（`App.Path`）/ `cwd`（相对路径基准）。`cwd` 只在解释器内部随 `ChDir` 变，绝不要改进程 CWD（编辑器、资源管理器、后台验证都会受影响）。没有脚本路径时两者都退回进程目录，保持旧测试/`run` 入口的行为。
- `Open` 的 `As #n` 是可选的（解析成 `Option<Expr>`）：省略时运行时用 `free_file_number`（与 `FreeFile` 同一逻辑）分配最小空闲号，之后用 `#1`、`#2`… 读写即可。
- 自动保存由主窗口里每秒一次的异步循环检查（`EditorSettings::auto_save_due` 判断开关/间隔/脏文件数），保存复用 `write_content`（按原编码写回并更新 `saved` 快照），不弹对话框、不抢焦点。
- 输入框聚焦：组件的 `focus_ring_style` 会在边框外再画一圈；本应用统一用 `.focus_ring(false)` 关掉，只在聚焦时把边框换成 `palette.sidebar_text`（大纲过滤框与所有输入框对话框都这样）。
- 字号：只把**弹窗**调小（标题 `.text_sm()`、说明 `.text_xs()`、输入框 `.xsmall()`、按钮 `.small()`），面板、菜单与设置窗口保持组件默认字号（`theme.font_size` 不要动，否则整块界面都会变小）。
- 资源管理器拖动：拖动数据是 `explorer::ExplorerDrag`，文件夹行用 `drag_over::<ExplorerDrag>` + `on_drop` 接收（`can_move_into` 判断不能拖进自己/子目录、目标必须存在且不是当前父目录），标签栏用 `editor_pane::DropFileAction` 接收。行点击回到「文件夹展开/收起、文件打开」两个回调（`OpenFileAction` / `ToggleFolderAction`）。**暂不提供多选**：曾用 `window.modifiers()` 在 `on_mouse_down(Left)` 里判断 Ctrl/Shift 并给行加 `multi_select_bg` 底色，但 `ListItem::selected` 的选中态始终只有一个、行内自绘底色也不会保留，实际表现是「Ctrl 点第二条，第一条高亮就消失」，已整体移除（相关代码：`ExplorerPanel::selected`、`selection_for`、`CodeWindow::delete_paths_dialog`、`Palette::multi_select_bg`）。以后若要做多选，必须先确认这个后端下 `ListItem` 的行选中态能多个并存。
- 转到文件：候选文件由 `CodeWindow::file_candidates` 拼装（已打开 → 最近打开 → 资源管理器各根目录递归，`workspace::collect_files` 跳过 `.` 开头的目录与 `target`/`node_modules`/`vendor`/`.git`，不跟随符号链接，深度上限 12、总数上限 `MAX_GOTO_FILES`），匹配用纯函数 `workspace::fuzzy_score` / `best_match`（完全相等 > 前缀 > 包含 > 子序列，同分取路径更短）。查询串含 `/` 或 `\` 时按完整路径匹配（`\` 先归一成 `/`），否则只比文件名。扫描在按键回调里同步执行，因此候选数量必须设上限。`Ctrl+P` 额外注册了 `Dialog` / `Dialog > Input` 上下文下的 `Cancel`，对话框里再按一次等于关闭；`goto_file_dialog` 开头还有 `has_active_dialog` 守卫，避免叠出第二个对话框。
- 资源管理器树：gpui-kit 的 `TreeItem::is_folder()` 就是 `!children.is_empty()`，所以**空目录必须补一个占位子项**（`explorer::EMPTY_FOLDER_SUFFIX`，渲染成一行灰色的「（空文件夹）」），否则会显示成文件、点不开。右键菜单挂在 `ListItem::on_mouse_down(MouseButton::Right, ...)` 上（面板自己的 `context_menu` 之外不弹系统菜单），菜单本身复用 `editor_pane::context_menu_element`。菜单项的**顺序与分隔线由纯函数 `panels::explorer_menu_entries` 决定**（`panels_tests` 保证第一项不带分隔线、也不会出现连续分隔线，之前「新建文件」上方多一条空线就是这么来的）；`context_menu_element` 另外加了一道保险：第一项永远不画分隔线。路径改名/删除后统一用 `workspace::reprefix_path` 改写打开的文件、断点、条件断点与最近打开：只有「完全相同」或「后面紧跟路径分隔符」才算目录下的子项，避免 `old` 误匹配 `older`。
- 括号高亮：`RangeDecorationCollection::set` **每次调用都会 `notify`**（即使装饰完全相同），所以在编辑器观察者里无条件 `set` 会把自己再次唤醒，形成死循环——表现为**程序启动即 100% CPU 卡死**。现在用 `CodeWindow::bracket_pairs`（`BracketHighlight::{Unset,None,Pair}`）缓存上一次画过的配对，只有变化时才 `set`。以后凡是「在实体观察者里设置装饰」都要先做同样的变化判断。
- 搜索面板：`crate::search` 里只有纯逻辑（`find_in_text` / `search_files_with_progress`），面板只做渲染与回调，所以匹配规则可以直接单测。扫描跑在后台线程（`cx.background_executor().spawn`），靠三道限制兜底：`MAX_SEARCH_BYTES`（单文件 2 MiB）、`MAX_SEARCH_TOTAL_BYTES`（单次总计 32 MiB，`search_files_with_budget` 供测试注入小预算）、`MAX_SEARCH_HITS`（500 处）。取消用 `SearchProgress::cancel`（后台循环每个文件检查一次），结果回填再看 `SearchPanel::search_id`：编号不一致就丢弃，避免旧搜索结果覆盖新关键字的结果。进度靠 `spawn_progress_poll` 每 120ms `cx.notify()` 一次，扫描结束即停止。每行只记第一处命中，行号/列号都是 1 基；含 NUL 字节、非 UTF-8、读不了的文件计入 `skipped`。大小写不敏感用逐字符 `to_lowercase()` 比较，不能用整行 `to_lowercase()`（`İ` 之类小写后字符数会变）。
- 跨文件替换：纯逻辑在 `search::replace_in_text`（逐行处理，`\r\n` / `\n` 原样保留；匹配复用 `match_at` + `is_word_boundary`，与搜索规则完全一致）与 `search::replace_in_files`（先用 `read_text` 过一遍与搜索同一套「过大/二进制/非 UTF-8」门槛，所以写回的仍是原编码内容）。界面只做编排：`SearchPanel::replace_all` 从**结果**里取去重后的文件列表 → 替换 → `CodeWindow::after_replace` 刷新受影响且**没有未保存修改**的标签（有修改的留给外部修改检测弹框，避免覆盖用户编辑）→ 再 `run()` 重搜一遍。
- 编辑器文本命令（`editing.rs`）是纯函数：`move_lines` / `toggle_comment` 接收全文 + 字节选区，返回新全文与新选区，覆盖了「最后一行没有换行符」「选区终点正好落在行首」等边界，全部有单测。执行时用 `replace_all` 一次性替换全文，**必须自己把选区与 `scroll_offset` 放回去**：`replace_all` 会把滚动位置重置到顶部。`comment_token` 按文件语言取（VB 用 `'`，Rust/JSON 用 `//`，纯文本不支持）。
- 路径栏与查找框：查找/替换面板是 `gpui-component` 的编辑器内嵌 overlay，`Input::render` 用 `v_flex().children(search_panel).child(文本区)` 把它画在**编辑控件自己的最上方**。所以查找框要落在路径栏下面，不需要额外定位，只要把路径栏作为编辑区 `v_flex` 里的兄弟节点插在标签栏和 `body` 之间。`editor_pane::split_path_for_display` 只做「目录 + 文件名」的拆分（`editor_pane_tests` 覆盖 `C:\`、`/`、纯文件名、结尾带分隔符几种情况），渲染时目录可截断、文件名 `flex_shrink_0` 保持完整。
- 编辑器右键菜单用自绘浮层（`editor_pane::context_menu_element`）：原生系统菜单无法做圆角/自定义边框，所以 `Editor::context_menu(|menu, ..| menu)` 只保留一个空的原生菜单（编辑器仍会把光标移到右键位置，但不会弹系统菜单），真正显示的菜单由外层 `on_mouse_up(Right)` 记录位置、`deferred(anchored(..))` 画在面板外。样式与 `menu.rs` 的 `menu_panel` 一致（`palette.border` 边框 + `sidebar_bg` 底色 + 4px 圆角）。编辑命令用 `window.dispatch_action` 派发（先聚焦编辑框），其它命令直接调 `CodeWindow` 方法。
- 源码是 UTF-8：**不要**用 PowerShell 的 `Get-Content`/`Set-Content`/`Out-File` 读写源码（GBK 解码会毁中文，`-Encoding utf8` 会写 BOM）。
- 持久化：界面布局在 `%APPDATA%\gpui-first\layout.txt`（尺寸、开关、根目录、展开项、`breakpoint=`、`breakpoint_condition=`、`recent=`、`open=`、`active=`、`script_args=`、`watch=`、`dock_state=`、`side_dock=`、`show_search=`、`search_width=`、`window_bounds=`，以及 `editor_*` / `syntax_*` / `output_*` 这些设置键）；其它设置走 `SettingsStore`（`SettingsStore::update`，主窗口观察后 `sync_settings` 顺手 `save_layout` 落盘）。`layout_version` 用于迁移：旧版本保存的 `dock_state` 会被丢弃并按当前默认布局重建，其余字段保留。`side_dock` 记录资源管理器/代码大纲最近使用的左/右侧栏，互斥面板切换时沿用该侧。窗口矩形由每秒一次的自动保存循环读取（`observe_window_bounds` 也会更新），只在变化后写盘。恢复会话只在文件仍存在时打开，且上限 `SESSION_LIMIT`（40）。
- Dock 面板要通过 `panel_handle(...)` 包装后再交给 `DockLayout::panel_view` / `register_panel`；直接传裸 `Entity` 时 `DockSkin` 无法取回 `title_bar`、`inner_padding` 等呈现设置。
- 命令面板：清单在 `commands.rs`（纯数据 + `workspace::best_match`，「名称完全相等 > 前缀 > 包含 > 子序列」），执行由 `CodeWindow::run_command` 按 `id` 分发。**对话框必须先 `window.close_dialog(cx)` 再执行命令**：`open_dialog` 是压栈、`close_dialog` 弹栈，顺序反过来会把命令自己弹出的对话框（打开文件、转到行……）一起关掉。
- 外部修改检测：`workspace::disk_stamp` / `stamp_differs`（任何一边没有时间戳都算「没变」，新建与刚删除的文件不弹提示）。`CodeWindow::disk_stamps` 与 `open_files` 平行，**自己写盘后必须刷新时间戳**（`write_content` 里做），否则下一次 tick 会把自己的保存当成外部修改；准备弹框前先记下新时间戳，保证同一处改动只问一次。
- 输出过滤：`OutputLog::visible` 存「过滤后可见行号」，界面里所有行下标（渲染、`position_at`、选区、双击选行、点行跳转）一律按**可见下标**解释，取内容走 `visible_line`。新增输出时增量维护 `visible`，`trim` 裁行时整体前移；过滤条件变化会清空选区（旧行号已经失效）。`selection_text` 只剩单测在用，界面的复制走 `visible_selection_text` / `visible_text`。

## 后续计划（按建议顺序）

1. 资源管理器支持把条目拖到窗口外（拖出为复制路径）；多选等 `ListItem` 的行选中态能稳定多个并存后再做。
2. 文件编码：状态栏点击切换编码并重新解码，另存时可指定编码与换行符（CRLF/LF）。
3. 帮助菜单加「快捷键一览」对话框。
4. 给 `gpui-base` 打本地补丁以支持语言级缩进规则（见「已知遗留」）。

## 常用命令

```powershell
cargo build
cargo test          # 二进制 crate，不要用 cargo test --lib
cargo clippy --all-targets
```

## 图标与打包

- **窗口/任务栏图标**：gpui 的 Windows 后端在启动时执行 `LoadImageW(模块句柄, MAKEINTRESOURCE(1), IMAGE_ICON, ...)`，也就是**只认可执行文件里资源 id 1 的图标组**（`WindowOptions::icon` 那个字段是 X11 专用，Windows 上会被忽略）。所以图标必须编进 exe：
  - `assets/app.ico`：从 `assets/icon.png` 生成的 16/24/32/48/64/128（BMP）+ 256（PNG）多尺寸 ICO；
  - `app.rc`：只有一行 `1 ICON "assets/app.ico"`；
  - `build.rs`：`embed_resource::compile("app.rc", embed_resource::NONE)`（build-dependency `embed-resource`），它会找到 Windows SDK 里的 `rc.exe` 编出 `.res` 并 `cargo:rustc-link-arg-bins`。换图标只要替换 `assets/app.ico` 后重新构建。
- **发布**：`cargo build --release` 后把 `target\release\first.exe` 复制出去即可；exe 已包含图标与全部内置资源（图标字体是 `gpui-kit-assets` 编译进二进制的 SVGs）。
  - 运行期唯一的外部依赖是 MSVC 运行库 `vcruntime140.dll`（导入表里还有 `icuuc.dll`，那是 Windows 10 1903+ 自带的系统组件，不用带）。目标机器没装 VC++ 运行时的话，把 VS 的 `VC\Redist\MSVC\<ver>\x64\Microsoft.VC143.CRT\vcruntime140.dll` 放到 exe 同目录。
  - 布局/设置文件在 `%APPDATA%\gpui-first\layout.txt`，随程序自动创建，不需要预置配置文件。

## 已知遗留

- VB 的自动缩进目前只跟上一行缩进走（组件默认规则按括号判断），**块关键字规则装不上**：`gpui-base` 的 `LanguageConfig` 是 crate 私有类型，`set_language_config` 虽然被 re-export，外部无法构造参数。想支持 `Sub`/`If ... Then`/`For`/`Do`/`With` 的自动缩进，需要给 `gpui-base` 打本地补丁（仓库目前只 patch 了 `gpui-component`）。
- 资源管理器暂不提供多选（`ListItem` 的行选中态在这个后端下同时只有一个生效，`Ctrl+点击` 表现成「换了选中项」），目前只支持单选右键菜单与拖动。
- 仓库根目录只保留 `Cargo.toml` / `Cargo.lock` / `.gitignore` / `ROADMAP.md` / `app.rc` / `build.rs` / `assets/`（图标与资源脚本；历史调试日志已于 2026-11-06 清理）。
