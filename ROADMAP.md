# 开发进度与后续计划

> 更新时间：2026-11-15。下次接手先看这里，再照「验证约定」跑一遍命令。

## 当前状态

- `cargo build` 通过、`cargo test` **813 passed**（625 + 2 + 186，12 ignored）、`cargo clippy --all-targets` 无警告。
- 界面/交互效果由人工验证（不要用截图、也不要模拟鼠标键盘去点界面）。
- **解释器语义以 VB6 官方行为为准**：`cargo test` 会跑 `src/vb6_golden_tests.rs`，
  期望值来自本机 VB6 真编译（629 条表达式 + 63 条错误号 + 90 段整脚本）。
  改动解释器语义后先跑这一组；换机器/装 VB6 后用 `python tools/vb6_golden.py` 重新生成。
- 解释器性能（release，`qide-run`，2M 次循环，取 8 轮最小值）：数值循环约 0.25~0.36s、数组元素自增约 1.01s、
  1500 元素冒泡约 0.83s（机器负载会差 ±10%，跨版本比较请用交错 A/B）；VB6 p-code 同脚本约 0.36s / 0.06s。
  热路径约定见 `docs/架构.md`「线程与性能约束」。
  对照脚本现在放在 `tools/bench/`（`numeric_loop.vb` / `array_increment.vb`（需 `--max-steps 60000000`）/ `bubble_sort.vb`）：
  改热路径时用「同一台机器、两个二进制交错跑、取每边最小值」的方式比，不要拿单次绝对值跨版本对比。

## 已完成的功能（按模块）

### 界面

- 状态栏：输出开关、文件编码、调试状态（暂停时带 `模块 → 过程 → 过程` 调用栈）、光标「行 N，列 M」、错误数、输出行数。
- 自绘标题栏：左侧菜单栏，中间显示程序名 **Q-IDE**（`title_bar::APP_TITLE`，同一常量也用作系统窗口标题 / 任务栏名称，字号 `.text_xs()`），右侧是最小化/最大化/关闭。
- 活动栏：资源管理器 / 代码大纲 / 搜索 / 设置（最底）。资源管理器、代码大纲与搜索面板互斥，同一时间只显示一个。
- Dock：主区域由 gpui-kit 的 `DockArea` + `DockSkin` 管理，编辑器固定在中间，资源管理器/代码大纲可在左侧或右侧 dock，输出固定在底部；面板可以拖动，完整布局写入布局文件的 `dock_state`。拖到中间的侧栏会自动回到左右 dock，避免和输出区域混在一起。拖动侧栏时由主窗口绘制贴合最终 dock 位置的蓝色指示条，不使用 Dock 自带的半区占位框。
- 面板标题栏由 Dock 提供：资源管理器有「新建文件 / 新建文件夹 / 刷新 / + 添加根目录 / 关闭」，代码大纲有「关闭」，输出有「复制 / 清空 / 关闭」；编辑器面板隐藏 Dock 标题栏，保留自己的文件标签栏。
- 路径栏：标签栏下方固定一行显示**当前标签对应文件的完整路径**，底色与代码编辑框相同（都是 `palette.window_bg`），目录部分用灰色、文件名用正文色并保证不被截断（目录太长时只截断目录）。`.when_some` 只在有激活文件时插入，没打开文件时不占位。
- 资源管理器：头部按钮依次是新建文件、新建文件夹、刷新、添加根目录、关闭。新建落在最近点过的文件夹（没点过用第一个根目录，没有任何根目录时先弹文件夹选择框）；刷新重新读盘，外部新增/删除的文件立刻可见。空文件夹会补一个「（空文件夹）」占位行（树组件按「有没有子项」判断是不是文件夹，没有占位项就会显示成文件、也点不开）。**右键任意条目**弹出与编辑器同款的自绘菜单：文件是「打开 / 重命名… / 删除… / 复制路径 / 刷新」，文件夹另有「新建文件 / 新建文件夹」，根项目录还有「从资源管理器移除」。
- 资源管理器的拖动：条目可以拖动，**拖到文件夹上**即移动进去（不能拖进自己或自己的子目录、目标已有同名项会拒绝、打开中的文件先写回再搬），**拖到编辑区标签栏**即打开该文件，**拖出窗口外松开**即把该路径复制到剪贴板（Windows 后端没有原生 `IDropSource`，用鼠标事件自行判断，见「坑位备忘」）。暂不提供多选（树行高亮与修饰键判断在这个后端上不稳定，见「已知遗留」）。
- 代码大纲：变量按作用域（模块/过程/类）二次分组；分区与作用域都可折叠，**默认全部收起**，人工点开才展开；过程行标注过程形式（`Function` / `Sub` / `Property Get` / `Property Let` / `Property Set`）；调试暂停时变量下方多一行「值 · 类型」（灰色单行省略）；点行跳转到对应代码行。「断点」一节按行号列出当前文件的断点（带该行源码，条件断点用黄色并另起一行显示条件，点击跳转）；「监视」一节列出用户添加的表达式（添加后自动展开），暂停时每条下方显示求值结果。顶部有**过滤框**：按名字忽略大小写筛选符号，过滤时符号分区与作用域分组强制展开、没有匹配的分区隐藏，清空后恢复原状。
- 菜单：
  - 文件：最近打开（最多 5 项，持久化、「清除最近打开」）、保存 / 另存为 / **全部保存（Ctrl+Alt+S）**、重命名当前文件…、删除当前文件…；
  - 编辑：撤销/重做/剪切/复制/粘贴 + **上移行（Alt+↑）/ 下移行（Alt+↓）/ 切换注释（Ctrl+/）** + **查找（Ctrl+F）/ 替换（Ctrl+H）**（组件内置面板）；
  - 查看：面板开关（资源管理器、代码大纲、搜索、输出、状态栏）、主题、全屏；字号用 `Ctrl+=` / `Ctrl+-` / `Ctrl+0`；
  - 转到：转到定义（F12）、查找引用（Shift+F12）、转到行 / 符号（Ctrl+G）、转到文件…（Ctrl+P，按文件名或路径模糊匹配，候选来自已打开的文件、最近打开的文件与资源管理器各根目录下的全部文件）；
  - 运行：运行、检查语法、停止脚本、运行参数…，分隔线后是调试组（调试运行 Ctrl+F5、暂停、继续 F11、单步 F10、切换断点 F9、设置条件断点…、运行到光标处 Ctrl+F10、清除所有断点 Ctrl+Shift+F9、添加监视…、清除所有监视）。
- 输出面板：任何以「第 N 行…」开头的行都能**单击跳转**（运行错误、语法检查结果、查找引用列表）；脚本结束时显示用时（`脚本执行结束（用时 1.23s）`，调试会话包含暂停时间）；连续出现完全相同的错误/警告合并为一行并标注「（重复 N 次）」，普通信息不合并。标题栏右侧有**过滤框**（关键字，忽略大小写）与「只看错误」开关，两者可叠加；被过滤掉的行不会参与渲染、选区与「复制」（复制只拿可见内容），全部被过滤时显示「没有匹配当前过滤条件的输出」。
- 命令面板：`Ctrl+Shift+P`（转到菜单里也有「命令面板…」）输入命令名，按名字模糊匹配后回车执行，共 39 条命令，覆盖文件、编辑、跳转、面板开关、主题与运行/调试；执行结果写进输出（`> 保存（Ctrl+S）`），找不到命令会在输出里提示。
- 快捷键一览：帮助菜单「快捷键一览…」或 `F1` 弹出只读对话框，按文件 / 编辑 / 标签 / 跳转 / 视图与字号 / 运行与调试 / 帮助 / 对话框 / 鼠标操作分区列出全部按键，内容比长时在对话框内部滚动；数据在 `src/shortcuts.rs`，`shortcuts_tests` 会检查命令面板里每条带快捷键的命令都出现在表里、且命令之间没有快捷键冲突。
- 外部修改检测：每秒的自动保存循环顺带比对已打开文件的修改时间。没有未保存修改时**静默重新加载**并在输出里记一行；有未保存修改时弹「重新加载 / 取消（保留我的）」，同一处改动只问一次。
- 重新打开关闭的标签：`Ctrl+Shift+T` 或文件菜单，按关闭顺序后进先出（最多 20 个），磁盘上已不存在的会被跳过。
- 编辑器右键菜单：撤销/重做/剪切/复制/粘贴/全选，分隔线后是上移行 / 下移行 / 切换注释 / 增加缩进 / 减少缩进，再后面是切换断点、设置条件断点…、运行到光标处，最后是转到定义、查找引用。菜单由应用自绘（不是系统原生菜单），圆角、边框颜色与底色跟顶部弹出菜单一致。
- 编辑增强：`Tab` / `Shift+Tab` 缩进与反缩进、回车自动缩进由编辑器组件内置（`IndentInline` / `OutdentInline` / smart indent）；**VB 的块级缩进规则由本应用提供**（`src/indent.rs` 的正则经 `gpui-base` 的 `LanguageConfig` 注册给 `vb`/`vb6`/`vba`：`Sub` / `Function` / `Property` / `Type` / `Class` / `Do` / `For` / `While` / `With` / `Select Case` / `Case` / `Else` 及以 `Then` 结尾的行在回车后多缩进一级，只在代码上下文生效）；**块结束行的行尾回车**由 `CodeWindow::dedent_after_block_end` 补一次反缩进（组件的「减少缩进」只匹配光标之后的文本，行尾时光标后面是空的），VB 文件的一级缩进固定为 4 个空格（与 VB6 和仓库里的脚本一致）。编辑菜单与右键菜单里也加了对应项；**括号配对高亮**由本应用实现（`brackets::matching_pair` 找配对，两个 `RangeDecoration` 画淡蓝色底），光标贴着 `(`/`[`/`{` 或它们的闭括号时高亮成对的两个括号，字符串与 VB 的 `'` 注释里的括号不算。
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

- 语义检查 `sema.rs`：`Option Explicit` 下的未声明变量、`ReDim` 未声明数组、未定义的函数或过程（`With` 块内跳过，调用名是变量时跳过）；另外做 `Implements` 的成员一致性与签名一致性检查（漏实现接口公开成员、写了接口没有的 `接口名_xxx`、过程形式 / 参数个数 / 参数类型 / `ByRef`-`ByVal` / 数组形式 / `Optional` 与默认值 / 返回类型与接口不一致），这一项与 `Option Explicit` 无关，与 VB6 一样属于编译期错误。
- 声明顺序：`As 类型名` 与 `Implements 接口名` 支持**前向引用**——`Type` / `Enum` / `Class` 可以写在使用点之后（`#Include` 进来的定义同样算），解析前先扫一遍 token 收集声明名；解析结束时检查 `Type` 循环定义（直接或间接包含自身）并报语法错误。VB6 里类模块/标准模块各是独立文件、互相引用没有先后要求，所以这一条不产生新的语义基准，只是让单文件写法对齐。
- 补全 `completion.rs`：文档里的符号优先于关键字/内置函数；**成员补全**（`p.` 后列 UDT 字段、类字段与方法、`Me.`、`New Collection` / `New Dictionary`）。
- 编码 `encoding.rs`：BOM / UTF-8 / UTF-16LE/BE / GBK 探测，保存按原编码写回；状态栏显示编码，**点击后在上方弹出编码列表**（当前项打勾，选中即按该编码重新解码，有未保存修改时先确认）；另存为时可指定编码（UTF-8 / UTF-8 BOM / UTF-16 LE / UTF-16 BE / GBK）与换行符（保持原样 / CRLF / LF，选 CRLF 或 LF 时同时改写编辑器内容）。
- 对话框 `dialog.rs`：`MsgBox` / `InputBox` 都自己建窗口 + 自己跑消息循环（理由见「坑位备忘」），支持按钮组语义，**开窗期间点「停止脚本」会立刻关闭**。
- 内置函数：新增 `Shell(path[, windowstyle])`、`Command()`（配合「运行参数…」）。
- 调试：断点（按文件记录、写进布局文件，重启后还在）、暂停/继续/单步、运行到光标处、清除所有断点、暂停快照（变量值 + 调用栈）、暂停行淡黄整行高亮。
- **条件断点**：菜单「运行 → 设置条件断点…」给光标所在行加条件（该行还没有断点会自动补上，留空表示清除条件）；条件在每次执行到该行时求值，为真才停，求值出错时照样停下方便排查；条件断点用黄色圆点区分普通断点；条件按 `breakpoint_condition=` 写进布局文件。断点与条件都列在代码大纲的「断点」一节里，便于查看。
- **变量监视**：菜单「运行 → 添加监视…」添加表达式（光标处的名字会预填），列表显示在「代码大纲 → 监视」一节；每次暂停都重新求值并显示值与类型，表达式出错时显示错误文本，恢复运行时清空结果；监视列表写进布局文件（`watch=` 行），重启后还在。求值不阻塞界面，「停止脚本」能中断求值表达式里的死循环。
- **脚本所在目录**：主窗口把当前文件路径放进 `RunOptions::script_path`。`Open`、`Dir`、`FileLen`、`FileDateTime`、`Kill`、`FileCopy`、`MkDir`、`RmDir`、`Name` 里的相对路径一律按脚本目录解析；`CurDir`/`ChDir` 只改脚本视角的目录，**不动编辑器进程的 CWD**；新增 `App.Path`（返回脚本所在目录，`Option Explicit` 下不误报）。`Open ... For Input` 省略 `As #n` 时会自动取最小空闲文件号（与 `FreeFile` 一致）。
- COM Automation（`com.rs`）：`CreateObject` / `GetObject`（路径与 moniker）/ ROT、`IDispatch::Invoke`（连续 VARIANT 实参数组、命名与 `ByRef` 实参、先属性后方法、默认成员 `DISPID_VALUE`）、任意维数与强类型 `SAFEARRAY`、`TypeName`（`IProvideClassInfo` + 接口名兜底）、`For Each` 走 `_NewEnum`、**`WithEvents` 事件接收**（模块级变量与类字段，`IConnectionPoint` + 自带 sink，事件在语句边界分派）。缺 UDT 记录封送，见「待办」第 6 条。
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
- 资源管理器拖出窗口：`gpui-pre` 的 Windows 后端**没有实现** `start_external_drag`（`can_start_external_drag()` 为 false），`external_drag_payload` 不会生效，所以「拖出复制路径」自己实现：拖动真正开始时（`on_drag` 的预览构造函数）通过 `ExplorerActions::on_item_drag` 记下路径（`explorer::DragOutState`），主窗口根节点监听 `on_mouse_move` / `on_mouse_up` / `on_mouse_up_out`，在窗口外松开时写剪贴板。窗口外的 `mouse up` 不保证送达，因此「离开过窗口、随后左键已松开地回到窗口内」也按外部松开处理；下次按下鼠标会清掉残留状态。各分支的单测在 `explorer_tests`。
- COM 事件接收（`WithEvents`，`com.rs`）三个必知点：① 连接点 `Advise` 时服务器按**事件接口 IID**（dispinterface）调 sink 的 `QueryInterface`，所以 `sink_query_interface` 必须接受 `SinkState.iid`，否则一律 `CONNECT_E_CANNOTCONNECT`（`0x80040202`）；② **ADODB 不支持 `IProvideClassInfo`**（QI 返回 `E_NOINTERFACE`），事件 dispid 只能走「按接口 IID 查注册表」（`HKCR\Interface\{IID}\TypeLib` → `HKCR\TypeLib\{LIBID}\<ver>\0\win32` → `LoadTypeLib`），`TypeName` 的 "Recordset"/"Connection" 也是靠接口名兜底得来的；③ 同一个事件常常挂在**多个连接点**上（ADO），要按事件名去重（`claimed`），否则处理过程被调用两次以上。sink 回调里**只排队**（`PendingEvent` + `EventArg`），绝不调用 `&mut Runtime`——分派发生在解释器的语句边界。
- COM 类型库的 vtable 只用到了少量槽位，**槽位下标必须逐个核对**：`ITypeInfo::GetContainingTypeLib` 在第 18 槽（0 基）、`GetDocumentation` 第 12、`GetIDsOfNames` 第 10；`ITypeLib` 只声明前 7 槽（`GetTypeInfoOfGuid` 在第 6）。`Guid` 是 `Copy`，传 IID 时不要借来借去（clippy `needless_borrow`）。

## 后续计划（按建议顺序）

界面侧只剩资源管理器多选的两块（拖动多选、框选），其余按下一节「解释器：语义完善进度与待办」的顺序做
（COM 事件配套项 → COM UDT 记录封送 → `Integer`/`Byte`/`Single` 独立值类型 → 编译错误文案 → 剩余内置函数；
`Slot` 扁平化已做完第一刀，值内联已否决）。

**当前进度一句话版**：解释器语义（含 `Null` 规则、参数校验、全部内置函数、COM Automation 主要能力、
`WithEvents` 事件）已按本机 VB6 oracle 对齐；剩余待办集中在 **COM 的 UDT 记录封送、活枚举器、
`HRESULT_FROM_WIN32` 错误号映射、事件同步时机** 与 **界面多选** 这几处，清单见下面两节。

## 解释器：语义完善进度与待办

**验证方式（改语义前必须先做）**：语义一律以本机 VB6 真编译为 oracle。用例写在
`tools/vb6_golden_cases.txt`（表达式）/ `vb6_golden_errors.txt`（失败语句 → `Err.Number`）/
`vb6_golden_scripts.txt`（整段脚本，`=== 名字` 分段，`@` 开头行写模块级选项），
跑 `python tools/vb6_golden.py` 用 `Vb6.exe /make`（`CompilationType=-1` 即 p-code）生成
`src/vb6_golden_data.rs`，再由 `cargo test`（`vb6_golden_tests.rs` 的 3 条数据驱动用例 + `builtins_tests.rs` 里若干条实测值用例）逐条比对。
与 VB6 有意不同之处写进 `KNOWN_DIFFERENCES` / `SCRIPT_KNOWN_DIFFERENCES` 并在
`docs/脚本语言.md` 写明理由。当前覆盖 **629 条表达式 + 63 条错误号 + 90 段整脚本**。
只想看某几个表达式的取值时用 `python tools/vb6_probe.py "表达式"` 或 `--file 清单.txt`
（只编临时工程、不写黄金数据）。

手工探针的坑：`Vb6.exe /make ...` **第一次调用常常直接返回、既不编译也不写日志**，连调两次（或先检查 exe / build.log 是否生成）再判断成败；工程文件里的 `Module=` / `Class=` 必须写成 `名字; 文件名`（只写名字会报「工程文件中断，不能加载」），EXE 工程的类模块要 `Attribute VB_Exposed = False`；探针既可以放 `Form_Load`（照 `tools/vb6_golden.py`），也可以用 `Module=` + `Startup="Sub Main"`。另外接口里的属性别取名 `Item`（实测读回 `.Item` 报「未找到方法或数据成员」，改名后正常，原因未深究）。

### 已按此流程修掉的语义缺陷（累计）

| 批次 | 缺陷 |
| --- | --- |
| 首次接入 | `^` 左结合写错（`2 ^ 3 ^ 2` 应为 64）、`CSng` 未窄化到 32 位 |
| 脚本用例 | `Null` 传播（算术/比较/`&`）、除零错误文案、`For` 结束后计数器应停在第一个不满足条件的值 |
| 文件 I/O | `Loc()` 语义（应为最后一个读写字节位置）、`Put #` 按转换函数定宽（1/2/4/8 字节） |
| 二进制字符串 | `Put #` / `Get #` 的变长 `String`：VB6 实测**没有长度前缀**（`"AB"` 让文件长 2 字节、空串 0 字节），`Get` 按变量**当前长度（字节）**读、当前是空串就一个字节也不读；原先直接报「不支持 `String`」。`Variant` 仍不支持（VB6 自己的 `Get` 一个 `Variant` 会报错误号 458） |
| 窄类型声明 | `VarType` / `TypeName` 优先看变量**声明类型**（VB6 实测 Integer 2 / Byte 17 / Single 4 / 数组 8194、`"Integer()"`）；原先一律按值判断，`Dim n As Integer` 的 `VarType(n)` 会错报 3 |
| 表达式类型 | `VarType` / `TypeName` / `Hex` / `Oct` 改为按语法推断**静态类型**：字面量宽度（`VarType(32767)`=2、`VarType(32768)`=3）、数字后缀（`123&` Long、`1.5!` Single，词法分析原先直接丢掉 `%`/`&`）、**进制字面量按最小整数宽度取补码**（`&HFFFF`=-1、`&H8000`=-32768、`&HFFFFFFFF`=-1 且 `VarType` 3、`&HFFFF&`=65535，`&O177777` 同样按 16 位）、运算提升（`1+1` Integer、`1&+1` Long、`Byte+Byte` Byte、`1+CSng(1)` Single、`1&+CSng(1)` Double、`CCur+CDbl` Currency、`/` 与 `^` Double、比较 Boolean、`Null` 不推断）；因此 `Hex(-1)` 变 16 位 `FFFF`、`Hex(-32768)` 是 `FFFF8000`。规则与 43 条新黄金表达式全部由本机 VB6 实测 |
| `Variant` 内容类型 | 赋值时把右侧表达式的静态类型记进槽（`Slot::narrow_type`），`VarType`/`TypeName` 据此区分 Integer/Long、Byte、Single/Double：`v = i`（Integer）→ 2、`v = CByte(1)` → 17、`v = True` → 11、`v = CCur(1)` → 6、`v = 40000` → 3、`v = 1.5` → 5、`v = "x"` → 8，隐式 `Dim x` 同样；原先一律按值报（`v = i` 报 3）。标注随 `Slot` 克隆共享，每次写入先清空，不会留下过期类型 |
| 整数运算溢出 | `+` / `-` / `*` / `\` 与一元 `-` 的结果超出 **Integer/Long** 范围时报错误号 6（VB6 实测：`32767 + 1`、`32767 * 2`、`2147483647 + 1`、`-i`（Integer -32768）都报；`-32768 - 1`（Long 字面量）、`c + c`（Long 30000）、`200 + 200` 不报）。检查放在 `apply_binary` 的整数路径里、只在「操作数装得进窄类型、结果装不进去」时给候选，调用方再核一次静态类型；交错 A/B 三个基准（`tools/bench/`）与不带检查的版本相比在噪声范围内（0%）。`Byte` 的运算溢出（VB6 也报 6）暂不检查 |
| `For Each` 元素类型 | 循环变量取数组的**元素声明类型**（VB6 实测：Integer 数组 → 2/`Integer`、Long → 3、Double → 5、String → 8、Byte → 17），原先一律按值报 3；循环变量的槽只查一次，`Variant` 变量每轮写一次标注。`Variant()` 数组与 `Collection` 仍取不到内容类型 |
| `Single` 打印 | `Single` 按 **7 位有效数字**打印（VB6 实测 21 组：`CSng(1.23456789)`→`1.234568`、`CSng(12345678)`→`1.234568E+07`、`CSng(0.00001)`→`0.00001`、`CSng(0.000123456789)`→`1.234568E-04`、`s = 1/3`→`0.3333333`、`CStr(CSng(1.23456789))`→`1.234568`、`Str(CSng(0.5))`→`" .5"`）；`Debug.Print` / `Print #` 与 `&` 拼接按表达式的静态类型判断（`&` 只在对浮点操作数时才查类型），`CStr` / `Str` 由运行时在分派前先渲染成字符串 |
| 错误文案 | `Error$(n)` / `Err.Raise n` / 新增的 `Error n` 语句改用 VB6 **运行时**错误表（本机实测 51 条文案，例如 3 号「不执行 GoSub 而返回」、7 号「超出内存」、91 号「未设置对象变量或 With block 变量」），兜底文案改成 VB6 的「定义的应用程序或对象错误」；原先用的是 IDE 编译对话框那套 RT_STRING 文案 |
| 类型与边界 | `For` 计数器声明成 Integer/Long/Byte 时自增溢出报**错误号 6**；`Partition` 全部规则（宽度 = 位数(终点+1)） |
| 静态检查 | `Implements` 漏实现接口成员、写了接口没有的 `接口名_xxx`、`Property Let/Set` 与接口形式不一致 |
| 接口签名 | 接口实现的过程形式（`Sub` / `Function` / `Property Get|Let|Set`）、参数个数、每个参数的类型 / `ByRef`-`ByVal` / 数组形式 / `Optional` 与默认值、返回类型都必须与接口一致（参数名可不同）；规则逐条来自本机 VB6 `/make` 实测 |
| 字符串与格式化 | `InStr` 忽略比较方式；`InStrRev` 忽略起点与比较方式；`Replace` 忽略起点/次数/比较方式、空查找串、返回值应从 `start` 开始；`Split("")` 应返回空数组；`Format` 命名格式全错（把字母当占位符）、`Format` 应逢五进一、`Format` 不支持 `0.00E+00` |
| 错误处理与遍历 | 任何 `On Error` 语句都清空 `Err`（`Resume Next` / `GoTo 0` / `GoTo -1` 实测都清，单纯读不清）；支持 `On Error GoTo -1`；`For Each` 遍历 `Dictionary` 改为取「键」并按插入顺序（原实现取「值」） |
| 日期格式 | `Format` 的 7 种日期命名格式（`General Date` / `Long Date` / `Medium Date` / `Short Date` / `Long Time` / `Medium Time` / `Short Time`）：原先按字母当占位符输出乱码；现按固定写法输出（`General Date` = `2024/2/29 13:05:06`、中文 `Long Date`、12 小时制 `Medium Time`，与本机 VB6 中文区域逐条一致），参数按 `CDate` 规则转换（数字当序列、文本会解析） |
| 日期显示 | `Date` 值转文本改用 VB6 的 `General Date`（`2024/2/29`、`2024/2/29 13:05:06`、序列 0 只给 `0:00:00`）；原先用固定 ISO（`2024-02-29`）并把 `CStr(CDate(*` / `DateAdd(*` 列进 `KNOWN_DIFFERENCES`，现在清单已清空 |
| `DateSerial` | 月、日允许溢出（`DateSerial(2024, 2, 30)` = 2024-03-01、`DateSerial(2024, 3, -1)` = 2024-02-28、`DateSerial(2024, 13, 1)` = 2025-01-01），两位年份按 0–29 → 2000–2029 / 30–99 → 1930–1999 展开；原先直接报「月份/日期超出范围」 |
| 新增内置函数 | `DatePart`（`yyyy`/`q`/`m`/`y`/`d`/`w`/`ww`/`h`/`n`/`s` + `firstdayofweek`/`firstweekofyear` 三种首周规则，117 组年末/年初日期与本机 VB6 逐条比对）、`LenB`（`Len` 的两倍，UTF-16 字节数） |
| 补齐内置函数 | `RGB` / `QBColor`（分量超 255 按 255、负数与越界报 5）、`Beep`（`MessageBeep`）、`FormatNumber` / `FormatCurrency` / `FormatPercent` / `FormatDateTime`（与 VB6 中文区域逐条一致，含负数括号、前导零、千位分隔、`vbUseDefault`）、`GetSetting` / `SaveSetting` / `DeleteSetting`（QIDE 存 `%APPDATA%\gpui-first\vb-settings.txt`，不分大小写）、`Spc`/`Tab` 的打印定位（`Tab` 越过目标列时先换行）；同时补了 VB 标准常量表（`vbTrue`/`vbFalse`/`vbCrLf`/`vbTextCompare`/`vbSunday`…/`vbYesNo`/`vbObjectError`/`vbArray` 等，`VarType(vbTrue)` 与 VB6 一样是 3） |
| `Format` 取舍与 `Val` | `Format` 的逢五进位改在**最短十进制表示**上做（`Format(1.005, "0.00")`=1.01、`Format(2.675, "0.00")`=2.68，原先乘 100 取整给 1.00）；`Val("1e")` / `Val("1E+")` 应为 1（指数后没有数字就不算指数）、`Val("1.2.3")`=1.2（原先整串解析失败给 0） |
| 第 4 批扫描 | `Double` 转文本的定点/科学切换改成「小数位或整数位超过 15 位才用科学计数法」（`CStr(1.5E-14)` 是 `0.000000000000015`、`CStr(0.0001)` 是 `0.0001`，原先 <`0.001` 一律 `1E-04`）；`String(3, 65)` 应得 `"AAA"`（第二个参数是数值时按字符代码，原先取数字文本的首字符 `"666"`）；`Val` 的进制文本改用与字面量相同的补码宽度规则（`Val("&HFFFF")`=-1、`Val("&H80000000")`=-2147483648、`Val("&HFFFF&")`=65535）并支持 `D` 指数（`Val("1D5")`=100000）；`IsNumeric("&H10")`=True；`IsDate("24-2-29")`=True（日期文本按「年-月-日 → 月-日-年 → 日-月-年」试，两位年份按 1930/2000 展开）；`WeekdayName` / `MonthName` 改输出中文名（`星期一` / `周一` / `十二月` / `12月`，原先英文），`WeekdayName` 省略 `firstdayofweek` 时按系统（星期一起算） |
| 第 5 批扫描 | 数学函数的定义域/溢出：`Sqr(-1)`、`Log(0)` 报错误号 5，`Exp(1000)`、`CDbl("1e400")` 报错误号 6（原先静默给 NaN / -inf / inf）；`Val("  -  1")` 应是 -1（符号后面允许空格） |
| 第 6 批扫描 | `Format` 的自定义日期占位符补齐：`ddd`/`dddd`（英文星期 `Thu`/`Thursday`）、`mmm`/`mmmm`（英文月份）、`w`（星期号）/`ww`（年内第几周）/`q`（季度）、`y`（年内第几天，原先当成两位年）、`aaa`/`aaaa`（中文星期）、`AM/PM`/`am/pm`/`A/P`/`a/p` 与 12 小时制 `h`（`hh:mm AM/PM` 原先给 `13:02:06 A2/P2`）、`m`/`mm` 紧跟 `h`/`hh` 时为分钟（`h:m:s` 原先把 `m` 当月份）；数值侧补 `@`/`&` 文本占位符与末尾逗号的千位缩放（`Format(1234567, "0,")`=1235，原先报「暂不支持」）；`DateAdd`/`DateDiff` 补 `y`/`w` 间隔，`DateDiff` 补 `q` 并按绝对季度序号相减（`DateDiff("q", #2024-03-31#, #2024-04-01#)`=1）、`w` 按周算（原先按天） |
| 容器窄类型 | `Variant` 数组 / `Collection` / `Dictionary` 各记一份元素内容类型：`arr(0) = CByte(9)` 后 `VarType(arr(0))` 是 17、`Array(CByte(1), 2)` 的 `For Each` 拿到 17/2、`c.Add CByte(7)` 后 17、`d("k") = CByte(1)` 后 17（原先一律按值报 3）；字面量下标/键才做静态查询，`ReDim Preserve` 会保留旧元素的内容类型 |
| `Byte` 运算溢出 | `CByte(200) + CByte(200)` 报错误号 6（原先静默给 400）。`overflow_candidate` 加了「结果落在 `0..=255` 直接放行」的快路径，交错 A/B 三个基准与改动前相比在噪声内（-1.6% / -0.5% / -0.8%） |
| 变量访问提速（`Slot` 扁平化第一刀） | 变量作用域表改用 FNV-1a（`hasher::FastMap`）、读变量与写变量都改走借用（`Runtime::lookup_slot`）：数值循环 −20.5%/−6.3%/−6.3%、数组元素自增 −12.4%/−3.8%/−1.8%、冒泡 −15.8%/−1.7%/−2.9%（三段交错 A/B 的最小值，`numeric_loop` 0.45→0.25~0.36 s）。「值内联」经实测上限只有个位数百分比且要改 31 处写路径，已否决（理由见 `docs/架构.md`「线程与性能约束」） |
| `CallByName` | 新增按名字后期绑定：类实例的 `VbGet`/`VbLet`/`VbSet`/`VbMethod`、COM 对象的同名分派，实参按原表达式传入所以 `ByRef` 形参照样写回（`CallByName(o, "RefArg", VbMethod, x)` 后 `x` 会变）；错误号逐条对齐本机 VB6（438 成员不存在/私有/非法调用方式、449 少传必填、450 参数过多或只读分配、446 少了值、91 对象字段用 `VbLet`、451 普通字段用 `VbSet`、5 对 `Nothing`、424 非对象）。同时补 `vbGet`(2)/`vbLet`(4)/`vbSet`(8)/`vbMethod`(1) 常量 |
| `GetAllSettings` | 返回 0 基二维 `String` 数组（`VarType` 8200、`TypeName` `String()`、列 0 键名/列 1 值、按写入顺序），没有键时返回 `Empty`，应用名/节名为空报 5（VB6 实测）。设置文件读写加了进程内互斥，避免并发脚本（与并行测试）互相丢数据 |
| 值侧窄类型 | `ByVal v As Variant` 参数与 UDT 里 `As Variant` 的字段现在也记内容类型（VB6 实测 `Narrow(i)`=2、`Narrow(CSng(1.5))`=4、`Narrow(CByte(1))`=17、`u.f = i` 后 `VarType(u.f)`=2），`Expr::Member` 的静态类型能查到字段标注；`RecordField` 带一份共享标注，UDT 字段 `ByRef` 传出时也不丢 |
| `Get #` / `Put #` 的 `Variant` | 按 VB6 的**带标记布局**读写：2 字节 VARTYPE + 内容（`v = 5` 写 4 字节、`v = "AB"` 写 6 字节、`Currency`/`Double`/`Date` 各 10 字节、`Boolean` 4 字节、`Single` 6 字节、`Empty` 2 字节），`Get` 读回后变量类型跟着标记走；标记是对象/数组报 458。逐字节比对过 VB6 写出的文件 |
| 值侧窄类型（剩余路径） | `VarType` 在**调用/成员访问**路径上也能报对：返回 `Variant` 的函数（`VarType(FV())`=2）、`IIf`/`Choose`/`Switch`（按选中的实参算，`IIf(True, i, s)`=2、`IIf(False, i, s)`=4）、`Property Get` 的返回声明（`VarType(c.PG)`=2/`c.PS`=4/`c.PV`=2）、类 `Variant` 字段（`c.P = i` 之后是 2）、`With` 里的字段、`ParamArray` 元素（2/4）、`Collection.Item`/`c(1)`（2）；原先一律按值报 3/5 |
| `VarPtr` / `ObjPtr` / `LoadPicture` | 补齐的三个内置函数：`VarPtr`/`ObjPtr` 返回 `Long` 形式的**进程内唯一且稳定**标识（同一变量/对象相等、`Nothing` 是 0，与 VB6 的可见行为一致）；`LoadPicture` 按幻数校验图片（缺文件 53、内容无效 481、空参数返回空 `Picture` 对象，`TypeName` 是 `Picture`、`VarType` 9）。`Set p = LoadPicture(...)` 现在可用 |
| COM 多实参封送 | `DISPPARAMS.rgvarg` 必须是**一段连续的 VARIANT 数组**，旧实现指向内部结构体字段，导致第 2 个及以后的实参全部错位（`d.Add "k", "v"` 的键读成空串、`d.Item("k")` 取不到值）。改成 `Vec<VariantGuard>` + `Box` 的 BYREF 目标后，多实参方法与 `ByRef` 写回都正确 |
| COM 参数化属性 | 成员带实参时**先按属性取**（`DISPATCH_PROPERTYGET`）失败再按方法调（`DISPATCH_METHOD`）：`Scripting.Dictionary` 的 `Item` 只认 `PROPERTYGET`、`Add` 只认 `METHOD`，原先取 `METHOD\|PROPERTYGET` 会静默拿到空值 |
| COM 默认成员 | 对象后直接接实参走 `DISPID_VALUE`：`d("k")` 取、`d("k") = "v"` 写（VB6 实测两种写法都可用，`CallByName(d, "k", VbGet)` 则仍是 438） |
| COM 错误号 | 失败号按 VB6 对齐：`DISP_E_EXCEPTION` 取 `EXCEPINFO.scode` 低 16 位（重复 `Add` 457）、成员不存在 438、参数个数不符 450、`DISP_E_TYPEMISMATCH` 13、其余 440（`ComFailure`）；原先一律报 438。实测 6 组（少传/多传实参 450、数组键 5、缺文件 53、`d.Item` 无参 450、重复键 457）与本机 VB6 逐条一致 |
| COM `For Each` | 走 `_NewEnum`（`IEnumVARIANT::Next`）取元素快照，`For Each k In d` 取到键（VB6 实测 `a`、`b`，`f.Drives` 取到 4 个 `Drive` 对象）；原先报错误号 5 |
| COM 转字符串 | 没有默认属性的对象不能转成字符串：`CStr(对象)`、`"x" & 对象`、`s = 对象`、`Debug.Print 对象` 都报 450（VB6 实测），原先会输出 `COM Object` |
| COM `TypeName` | 先走 `IProvideClassInfo::GetClassInfo` 取组件类名（`Dictionary` / `FileSystemObject` / `Connection` / `DOMDocument` / `SpVoice`），不支持时退回默认接口名（`WScript.Shell`→`IWshShell3`、`Shell.Application`→`IShellDispatch6`），ADO 的 `_Connection` 去掉前导下划线；7 个 ProgID 与本机 VB6 完全一致，原先一律 `Object` |
| COM 多维/强类型数组 | `SAFEARRAY` 支持任意维数与强类型元素：维序与元素下标都按 VB 的维序（`dims[0]` 在最前，与 `SafeArrayGetLBound(a, 1)` 一致，本机 ADODB `GetRows` 实测过），往返后各维下界/长度不变；元素类型按数组声明类型映射（`Long()`→`VT_ARRAY\|VT_I4` 即 `VarType` 8195、`String()` 8200、`Byte()` 8209、`Integer()` 8194、`Single()` 8196、`Double()` 8197、`Boolean()` 8203、`Date()` 8199、`Currency()` 8198、`Variant()` 8204）。原先只支持一维 `VT_ARRAY\|VT_VARIANT`，多维直接报错、强类型数组在服务器侧会类型不匹配 |
| COM 链式成员 | 语句级的成员路径不再限一层：`rs.Fields.Append "x", 3` 先读出 `rs.Fields` 再调 `Append`；赋值同样支持 `obj.Sub.Prop = v`、`d.Item("k") = v`、`rs.Fields("id").Value = v`（中间层按属性读出来再设最后一层）。另外 `obj.` 后面的成员名允许是关键字（`Append`/`Type`/`Open`/`Close` 这类），VB6 本来就允许。原先「成员名撞关键字」直接语法错误、链式赋值报「暂不支持」，ADODB 的建表→写值→`GetRows` 全流程跑不通 |
| `Null` 形参与参数校验 | 按 VB6 的形参类型规则处理 `Null`：非 `Variant` 形参（`Asc`/`Chr`/`Val`/`Split`/`Join`/`Replace`/`StrReverse`/`InStrRev`/`Space`/`String` 的个数参数、`CInt`/`CLng`/`CDbl`/`CDate`）报 94，`Variant` 形参（`Trim`/`LTrim`/`RTrim`/`UCase`/`LCase`、`Left`/`Right`/`Mid` 的第一个实参、`Len`/`LenB`、`String(3, Null)`、`InStr`）原样返回 `Null`；`Format(Null, "0")` 是空串、`CStr(Null)` 报 94、`CStr(数组)`/`Len(数组)` 报 13。参数校验也对齐：`Mid`/`InStr` 起点、`InStrRev` 起点 0、`Left`/`Right`/`Space`/`String` 负数长度报 5，`Asc("")` 报 5。原先这些位置一律返回 `"Null"` 文本、0 或静默给出结果 |
| COM `WithEvents` 事件 | `Dim WithEvents x As Object` 的变量 `Set` 到 COM 对象时，枚举 `IConnectionPointContainer` 的连接点并 `Advise` 自己的 `IDispatch` 接收器；事件名按源接口类型信息解析 dispid（服务器没有 `IProvideClassInfo` 就按接口 IID 查注册表，ADODB 走这条路），回调只排队，解释器在**下一条语句**开头分派给 `x_事件` 处理过程（不在 COM 回调里重入解释器）。同一个事件在多个连接点上都存在时只挂第一个，避免处理过程重复调用；`Drop` 时 `Unadvise`。声明位置支持模块级变量与类字段（类字段用私有过程 `字段_事件`，事件里用实例的局部变量访问数据）。原先 COM 事件完全不支持 |

### 待办（按建议顺序）

1. **`Value` 加窄类型（值侧只剩下拿不到类型的两种情形）**：类型报告（`VarType`/`TypeName`/`Hex`/`Oct`，含 `Variant` 内容类型）、
   Integer/Long/**Byte** 运算的溢出检查、`Variant` 数组 / `Collection` / `Dictionary` 的元素内容类型、
   `ByVal v As Variant` 参数、UDT 与类里 `As Variant` 的字段、`ParamArray` 元素、返回 `Variant` 的函数、
   `IIf`/`Choose`/`Switch`、`Property Get` 与 `With` 成员都已对齐，见上表；只剩 `Declare` 的 `As Any` 实参
   （按声明类型封送）与 `Single` 没有独立值类型（靠静态推断）——两者都不影响脚本可见行为，除非将来要按值做
   `Single` 运算的精度检查。真正要动 `Value` 才能再进一步，而代价是热路径上多一次判别。
2. **剩余未实现的内置函数（有意留空，逐条给出结论）**：
   - `DblQuote`：**VB6 里没有这个函数**（是 Access/VBA 的），实测编译不过，因此不提供。
   - `LoadPicture` / `ObjPtr` / `VarPtr` 已完成（见上表）。`LoadPicture` 只做「文件 → 空图片对象」，
     不解码图像（`Width`/`Height`/`Handle` 等 `IPicture` 成员不支持，QIDE 也没有图片控件）。
   - `CallByName` / `GetAllSettings` 已完成（见上表）。`CallByName` 对 COM 的**参数化属性**
     （`d.Item("k")` 这种）走 `call_with_argument_modes`，和直接写 `d.Item("k")` 的表现一致（COM 层的
     参数化属性缺口已在上表修掉）；但 `CallByName(d, "k", VbGet)` 这种**默认成员**写法在 VB6 里本来就是 438，
     QIDE 也一样。
3. **语法错误文案（已确认按 QIDE 自己的说法保留）**：`E:\dsh工作区\反编译\data\errors\vb6-compile-errors-zh.tsv`
   那 296 条是 VB6 **IDE 编译对话框**的文案（RT_STRING），本机实测 `Error$(40080)` 之类的运行时函数**不认**这些错误号
   （一律给兜底「定义的应用程序或对象错误」），所以这张表只能用来改 QIDE 自己的诊断措辞。
   QIDE 现在的提示更具体（如「`If` 块缺少 `End If`」、带反引号的标识符），逐条替换成 VB6 的说法收益不大，因此**不做整体替换**；
   运行时那套文案已经在「错误文案」一行对齐。若以后要仿 VB6 IDE 的报错体验，再按这张表加「错误号 → 文案」用于诊断面板。
4. **~~`Slot` 扁平化~~（已完成第一刀，值内联部分已否决）**：换 FNV-1a 哈希 + 读写变量都借用槽，三个基准
   −12%~−20%（哈希）/ −1.8%~−6.3%（借用，见上表）。剩下的「值内联、只在 `ByRef` 时造共享单元」实测上限只有个位数百分比，
   而写路径（`find_slot` 克隆后写入、`ByRef`、数组元素、类字段共享 `Rc<RefCell<Value>>`）要改成 `&mut`
   才能吃到，风险远大于收益，**不做**。要再提速请走「编译成字节码 / 缓存已解析的槽位」这条线。
5. `Property Set` 的接口分派（**已实测：与 VB6 一致，不需要改代码**）：接口变量上 `Set h.Payload = t`、
   `h.Title = "x"` 都走实现类的私有 `接口名_属性` 访问器，本机 VB6 与 QIDE 输出相同，已加回归测试
   `interface_property_set_dispatches_to_the_implementing_class`。
   `For Each` 遍历 `Dictionary`、`On Error GoTo -1`、`Static` 局部 UDT、`Format` 日期命名格式、`Format` 的自定义
   日期/文本占位符、`DateAdd`/`DateDiff` 的 `y`/`w`/`q` 间隔、`Get #`/`Put #` 的变长 `String`、
   `RGB`/`QBColor`/`Format*`/`SaveSetting` 系列/`Spc`/`Tab`
   已按本机 VB6 实测完成或确认原本就一致（见上表）。
6. **COM UDT 记录封送**（Automation 里最后一块大缺口）：现在 `Type ... End Type` 既不能作为
   `SAFEARRAY` 元素，也不能作为 Automation 实参/返回值。做法是 `oleaut32` 的
   `GetRecordInfoFromGuids`（新式）或旧式 `GetRecordInfoFromTypeInfo` 取 `IRecordInfo`，
   把脚本 UDT 按 `VT_RECORD`（`VARIANT.pRecInfo` + `pvRecord`）封送，数组走
   `SafeArrayCreate`+`VT_RECORD`。注意 `IRecordInfo` 的 vtable 槽位多（`RecordCreate`/
   `GetField`/`PutField`/`GetFieldNoCopy`/`PutFieldNoCopy`/`GetFieldNames`…），至少要能
   `RecordCreate` + 逐字段 `GetField`/`PutField`；字段顺序与名字必须与 VB6 的类型库一致
   （字节对齐/`Alignment` 由 `IRecordInfo` 负责，不要自己算偏移）。
   **验证**：本机 VB6 的 VBP `Reference=` 加不了 ADODB，所以没法直接对照；只能用
   `Scripting.Dictionary` 之类能存 `Variant` 的对象做往返（存进去再取出来），或写一个
   自带的 `IUnknown` 测试服务器。**这条大概率要拉长周期，可以先只做「不崩 + 报 450」。**
7. **`For Each` 改成活枚举器**：现在是先把 `_NewEnum` 的 `IEnumVARIANT::Next` 一次性取成快照
   再迭代，所以循环体里增删集合元素的行为与 VB6 不同。改成在 `For Each` 的帧里持有
   `IEnumVARIANT`（AddRef 后存 `NonNull`），每次迭代 `Next(1)` 取一个；`Exit For` / 异常
   退出时释放。顺带处理上面「已知遗留」里 `WScript.Shell` 在 VB6 报 92 的怪异行为。
8. **`HRESULT_FROM_WIN32` 错误号映射**：`EXCEPINFO.scode` 是 `0x8007xxxx` 时，现在报 Win32 码本身
   （`0x80070057` → 87）。VB6 有一张 Win32 → VB 错误号表（同一处报 5）。要补齐必须先拿 VB6 逐条
   实测建立映射（用 `tools/vb6_probe.py` 批量跑），不要凭经验猜；表建好后在 `ComFailure` 里加一层转换。
9. **COM 事件在 `DoEvents` 内同步分派**：现在事件在**下一条语句**开头才执行，VB6 是
   `DoEvents` 里同步执行（所以 `DoEvents` 之后立刻能拿到事件结果，且不写 `DoEvents` 的纯忙等循环
   收不到事件）。改法：把「取事件队列 → 逐个调处理过程」抽成 `Runtime::pump_com_events()`，
   在 `DoEvents` 内置函数与语句边界两处调用；注意 `pump_com_events` 会重入解释器执行用户过程，
   必须像 `evaluating` 那样防止递归触发自己（加 `pumping_events` 标志）。

**已决定不做**：模块级 `Public` / `Private` 隔离。`#Include` 是文本展开，整个脚本就是一个模块，
与 VB6 单模块内的语义一致（同一模块里的过程互相可见）；需要真正隔离的只有类成员，那部分已实现
（字段默认 `Private`、方法默认 `Public`）。将来做多模块工程时再补。

## 常用命令

```powershell
cargo build
cargo test          # 二进制 crate，不要用 cargo test --lib
cargo clippy --all-targets
```

## 图标与打包

- **窗口/任务栏图标**：gpui 的 Windows 后端在启动时执行 `LoadImageW(模块句柄, MAKEINTRESOURCE(1), IMAGE_ICON, ...)`，也就是**只认可执行文件里资源 id 1 的图标组**（`WindowOptions::icon` 那个字段是 X11 专用，Windows 上会被忽略）。所以图标必须编进 exe：
  - `assets/app.ico`：从 `assets/icon.png` 生成的 16/24/32/48/64/128（BMP）+ 256（PNG）多尺寸 ICO；
  - `assets/run.ico`：`qide-run.exe` 用同一张图的浅绿版本（浅绿底 `#AED581` + 深绿字形 `#1B5E20`，`assets/run.png` 是 256 的预览图），与深蓝的 `first.exe` 一眼可分；
  - `app.rc` / `run.rc`：各一行 `1 ICON "assets/xxx.ico"`；
  - `build.rs`：`embed_resource::compile_for("app.rc", ["first"], ...)` 与 `compile_for("run.rc", ["qide-run"], ...)`（build-dependency `embed-resource`），它找到 Windows SDK 里的 `rc.exe` 编出 `.res` 并只链进对应 bin（两个都链成 `--bins` 的话后编译的会覆盖前一个）。换图标只要替换 ico 后重新构建。
- **发布**：`cargo build --release` 后把 `target\release\first.exe` 复制出去即可；exe 已包含图标与全部内置资源（图标字体是 `gpui-kit-assets` 编译进二进制的 SVGs）。
  - 运行期唯一的外部依赖是 MSVC 运行库 `vcruntime140.dll`（导入表里还有 `icuuc.dll`，那是 Windows 10 1903+ 自带的系统组件，不用带）。目标机器没装 VC++ 运行时的话，把 VS 的 `VC\Redist\MSVC\<ver>\x64\Microsoft.VC143.CRT\vcruntime140.dll` 放到 exe 同目录。
  - 布局/设置文件在 `%APPDATA%\gpui-first\layout.txt`，随程序自动创建，不需要预置配置文件。

## 已知遗留

- VB 的自动缩进：块开始关键字见上（`src/indent.rs` + `LanguageConfig`）。块结束关键字在**行尾回车**时由
  `CodeWindow::dedent_after_block_end` 补一次反缩进（`PressEnter` 后看新行是否只有空白、上一行是否是块结束行，
  再走 `OutdentInline`，与 `Shift+Tab` 同一条路径、撤销记录仍是一条）。仍有两处限制：
  ① 行尾注释不触发（光标落在注释上下文里，组件不认为在代码里）；
  ② 一行里先写 `End If` 再写别的语句（`End If: x = 1`）时，行首仍是 `End` 开头，会按块结束处理。
- 资源管理器多选：勾选靠条目左侧圆点进入「选择模式」（`ExplorerSelection`），现在是**单选高亮 + 圆点模式 +
  `Ctrl+点击` 切换 + `Shift+点击` 选范围**并存。`Shift` 的范围从**上一次点过的条目**（普通点击、点圆点、`Ctrl+点击` 都算）
  到这一行，按树当前的可见顺序算（收起的分支不参与），根项与「空文件夹」占位行跳过。
  **未做**：拖动多选（一次只拖一条）、框选（rubber band）。
- 仓库根目录只保留 `Cargo.toml` / `Cargo.lock` / `.gitignore` / `ROADMAP.md` / `app.rc` / `run.rc` / `build.rs` / `assets/`（图标与资源脚本；历史调试日志已于 2026-11-06 清理）。
- COM：`_NewEnum` 不可用的对象（如 `WScript.Shell`）在 QIDE 里 `For Each` 报 438、循环体一次也不执行；
  本机 VB6 的表现是循环体跑一次并在 `Next` 上报 92（`On Error Resume Next` 下的怪异行为），未对齐。
- COM 错误号的系统包装：服务器若用 `HRESULT_FROM_WIN32` 包装（`EXCEPINFO.scode` 是 `0x8007xxxx`），
  QIDE 直接报 Win32 码本身（`0x80070057` → 87），VB6 有一张 Win32 → VB 错误号的映射表（同一处报 5）。
  常见路径（缺文件 53、盘符无效 68、类型不符 13）两边一致，只有这类系统包装码没逐条对齐；要补齐得先拿 VB6 逐条实测映射表。
- COM 事件的**时机**：QIDE 在「下一条语句」开头分派排队的事件，VB6 是在 `DoEvents` 里**同步**执行。
  因此 `DoEvents` 之后不会立刻拿到事件结果，纯忙等循环（不写 `DoEvents`）也能收到事件——这两点与 VB6 不同，
  见待办第 9 条。事件处理过程里再触发的 COM 事件同样要等下一次语句边界。
- COM 事件**没有 VB6 oracle**：本机 VBP 的 `Reference=*\G{...}` 只有 scrrun 生效，ADODB（`msado15.dll` /
  `msado25.tlb`）被静默忽略（`Private WithEvents rs As ADODB.Recordset` 报「变量未定义」），MSXML6 报
  「不能加载 DLL」。所以 `WithEvents` 的语义是按 MS 文档 + ADODB 实测实现的（`connection_WillConnect` 等），
  没有逐条 VB6 对照数据；以后换机器（能挂上 ADODB 引用）可以补。
- COM 事件的**已知小差异**：同一个事件在多个连接点上都存在时只挂第一个（ADO 如此，否则处理过程被重复调用）；
  VB6 的 COM 连接点复用细节未逐一核对。
