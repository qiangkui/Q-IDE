# 开发进度与后续计划

> 更新时间：2026-11-15。下次接手先看这里，再照「验证约定」跑一遍命令。

## 当前状态

- `cargo build` 通过、`cargo test` **862 passed**（672 + 2 + 188，16 ignored）、`cargo clippy --all-targets` 无警告。
- CI：`.github/workflows/ci.yml`（Windows：黄金数据同步检查 + `cargo build --all-targets` + `cargo test` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --all -- --check`）。
- 界面/交互效果由人工验证（不要用截图、也不要模拟鼠标键盘去点界面）。
- **崩溃日志**：`panic_log::install` 在 `main` 一开始装钩子，界面或启动阶段 panic 时把「时间/版本/位置/消息」
  追加到 `%APPDATA%\gpui-first\crash.log`（>256 KB 自动重开），然后照常交给默认钩子。用户回报闪退时先要这个文件。
- **解释器语义以 VB6 官方行为为准**：`cargo test` 会跑 `src/vb6_golden_tests.rs`，
  期望值来自本机 VB6 真编译（814 条表达式 + 76 条错误号 + 7 条近似用例 + 94 段整脚本）。
  改动解释器语义后先跑这一组；换机器/装 VB6 后用 `python tools/vb6_golden.py` 重新生成，
  用 `python tools/vb6_golden.py --check` 只校验数据文件与清单是否同步（不编译）。
- 解释器性能（release，`qide-run`，2M 次循环，**同一台机器交错 A/B、每边取 8 轮最小值**；单跑绝对值会漂 ±20%，
  不要拿不同时间的两次单跑对比）：数值循环约 0.29~0.35s、数组元素自增约 0.46~0.58s（热路径修掉「每次数组赋值白分配一条
  错误字符串」后，交错 A/B 是 0.456s vs 0.589s，约 −23%）、1500 元素冒泡约 0.77~0.83s、
  30 万次过程调用（`procedure_call.vb`）约 0.31s（变量名改 `Cow` 之后；改之前同轮对比是 0.37~0.47s）。
  热路径约定见 `docs/架构.md`「线程与性能约束」。
  对照脚本放在 `tools/bench/`（`numeric_loop.vb` / `array_increment.vb`（需 `--max-steps 60000000`）/
  `array_read.vb` / `bubble_sort.vb` / `procedure_call.vb`），用 `python tools/bench.py --old 旧.exe --new 新.exe` 交错 A/B
  （每项各取最小值）——手写循环很容易被 CPU 频率漂移带偏，脚本化的这套流程才可复现。
  旧记录里「VB6 p-code 同脚本 0.36s / 0.06s」的第二个数字与 QIDE 的实测不成比例（同样 4 条语句/轮不可能只花 30ns），
  疑为当时脚本/计时方式不同，**不要再拿它当基线**。

## 已完成的功能（按模块）

### 界面

- 状态栏：**只看图标**的「输出 / 终端」面板开关与 AI 面板开关（都在最右、面板打开时带选中底色；输出行数不再占文字，移到那枚图标的悬停提示里）、文件编码、调试状态（暂停时带 `模块 → 过程 → 过程` 调用栈）、光标「行 N，列 M」、错误数。
- 自绘标题栏：左侧菜单栏，中间显示程序名 **Q-IDE**（`title_bar::APP_TITLE`，同一常量也用作系统窗口标题 / 任务栏名称，字号 `.text_xs()`），右侧是最小化/最大化/关闭。
- 活动栏：资源管理器 / 代码大纲 / 搜索 / 设置（最底）。资源管理器、代码大纲与搜索面板互斥，同一时间只显示一个。**AI 面板的开关不在活动栏**，在状态栏最右边（见上一条），打开时那枚按钮带选中底色。
- Dock：主区域由 gpui-kit 的 `DockArea` + `DockSkin` 管理，编辑器固定在中间，资源管理器/代码大纲可在左侧或右侧 dock，输出固定在底部；面板可以拖动，完整布局写入布局文件的 `dock_state`。拖到中间的侧栏会自动回到左右 dock，避免和输出区域混在一起。拖动侧栏时由主窗口绘制贴合最终 dock 位置的蓝色指示条，不使用 Dock 自带的半区占位框。
- 面板标题栏由 Dock 提供：资源管理器有「新建文件 / 新建文件夹 / 刷新 / + 添加根目录 / 关闭」，代码大纲有「关闭」，输出有「只看错误 / 复制 / 清空 / 关闭」（**这几个只画图标不写字**，含义靠悬停提示：只看错误与警告、复制选区（没选区时复制全部输出）、清空输出；终端页的「清空」同样只有图标）；编辑器面板隐藏 Dock 标题栏，保留自己的文件标签栏。
- 路径栏：标签栏下方固定一行显示**当前标签对应文件的完整路径**，底色与代码编辑框相同（都是 `palette.window_bg`），目录部分用灰色、文件名用正文色并保证不被截断（目录太长时只截断目录）。`.when_some` 只在有激活文件时插入，没打开文件时不占位。
- 资源管理器：头部按钮依次是新建文件、新建文件夹、刷新、添加根目录、关闭。新建落在最近点过的文件夹（没点过用第一个根目录，没有任何根目录时先弹文件夹选择框）；刷新重新读盘，外部新增/删除的文件立刻可见。空文件夹会补一个「（空文件夹）」占位行（树组件按「有没有子项」判断是不是文件夹，没有占位项就会显示成文件、也点不开）。**右键任意条目**弹出与编辑器同款的自绘菜单：文件是「打开 / 重命名… / 删除… / 复制路径 / 刷新」，文件夹另有「新建文件 / 新建文件夹」，根项目录还有「从资源管理器移除」。
- 资源管理器的拖动：条目可以拖动，**拖到文件夹上**即移动进去（不能拖进自己或自己的子目录、目标已有同名项会拒绝、打开中的文件先写回再搬），**拖到编辑区标签栏**即打开该文件，**拖出窗口外松开**即把该路径复制到剪贴板（Windows 后端没有原生 `IDropSource`，用鼠标事件自行判断，见「坑位备忘」）。选择模式下拖其中一个选中项会**带着整批选中项一起搬**（父目录也在选中列表里时子项不重复搬，见 `explorer::move_targets`）。
- 资源管理器的多选与框选：条目左侧圆点进入选择模式（标题栏变成「已选择 N 项」，只保留删除与退出），`Ctrl+点击` 切换、`Shift+点击` 选范围；**在树的空白处按住左键拖动可以框选**，扫过的行连成一段一起选中，没框到任何行时清空选择（与 Windows 资源管理器一致）。选择模式下焦点停在面板里时还有键盘快捷键：`Ctrl+A` 全选、`Delete` 删除选中、`Esc` 退出选择模式（`Explorer` 键上下文，不与编辑器的同名快捷键冲突）。
- 代码大纲：变量按作用域（模块/过程/类）二次分组；分区与作用域都可折叠，**默认全部收起**，人工点开才展开；过程行标注过程形式（`Function` / `Sub` / `Property Get` / `Property Let` / `Property Set`）；调试暂停时变量下方多一行「值 · 类型」（正文色、悬停高亮）；点行跳转到对应代码行，**双击这行「值」直接改这个变量**（弹窗只输新值、预填当前值，回车按表达式求值后写回，变量列表立刻刷新）；没暂停时这一行不存在，变量保持只读。**「调用栈」一节只在暂停时出现**（刚进入暂停会自动展开）：从模块到最内层一行一层（带各自的行号），**单击某一行就切到那一层**——变量列表、编辑器高亮、监视表达式的求值作用域都跟着换。**「断点」一节按行号列出当前文件的断点**（带该行源码，条件断点用黄色并另起一行显示条件，点击跳转）；「监视」一节列出用户添加的表达式（添加后自动展开），暂停时每条下方显示求值结果。顶部有**过滤框**：按名字忽略大小写筛选符号，过滤时符号分区与作用域分组强制展开、没有匹配的分区隐藏，清空后恢复原状。
- 菜单：
  - 文件：最近打开（最多 5 项，持久化、「清除最近打开」）、保存 / 另存为 / **全部保存（Ctrl+Alt+S）**、重命名当前文件…、删除当前文件…；
  - 编辑：撤销/重做/剪切/复制/粘贴 + **上移行（Alt+↑）/ 下移行（Alt+↓）/ 切换注释（Ctrl+/）** + **查找（Ctrl+F）/ 替换（Ctrl+H）**（组件内置面板）；
  - 查看：面板开关（资源管理器、代码大纲、搜索、AI 编程、输出、状态栏）、主题、全屏；字号用 `Ctrl+=` / `Ctrl+-` / `Ctrl+0`；
  - 转到：转到定义（F12）、查找引用（Shift+F12）、转到行 / 符号（Ctrl+G）、转到文件…（Ctrl+P，按文件名或路径模糊匹配，候选来自已打开的文件、最近打开的文件与资源管理器各根目录下的全部文件）；
  - 运行：运行、检查语法、停止脚本、运行参数…，分隔线后是调试组（调试运行 Ctrl+F5、暂停、继续 F11、单步 F10、切换断点 F9、设置条件断点…、运行到光标处 Ctrl+F10、清除所有断点 Ctrl+Shift+F9、添加监视…、清除所有监视）。
- 输出面板：任何以「第 N 行…」开头的行都能**单击跳转**（运行错误、语法检查结果、查找引用列表）；脚本结束时显示用时（`脚本执行结束（用时 1.23s）`，调试会话包含暂停时间）；连续出现完全相同的错误/警告合并为一行并标注「（重复 N 次）」，普通信息不合并。标题栏右侧有**过滤框**（关键字，忽略大小写）与「只看错误」开关，两者可叠加；被过滤掉的行不会参与渲染、选区与「复制」（复制只拿可见内容），全部被过滤时显示「没有匹配当前过滤条件的输出」。
- 命令面板：`Ctrl+Shift+P`（转到菜单里也有「命令面板…」）输入命令名，按名字模糊匹配后回车执行，共 43 条命令，覆盖文件、编辑、跳转、面板开关、主题与运行/调试；执行结果写进输出（`> 保存（Ctrl+S）`），找不到命令会在输出里提示。
- 快捷键一览：帮助菜单「快捷键一览…」或 `F1` 弹出只读对话框，按文件 / 编辑 / 标签 / 跳转 / 视图与字号 / 运行与调试 / 帮助 / 对话框 / 鼠标操作分区列出全部按键，内容比长时在对话框内部滚动；数据在 `src/shortcuts.rs`，`shortcuts_tests` 会检查命令面板里每条带快捷键的命令都出现在表里、且命令之间没有快捷键冲突。
- 外部修改检测：每秒的自动保存循环顺带比对已打开文件的修改时间。没有未保存修改时**静默重新加载**并在输出里记一行；有未保存修改时弹「重新加载 / 取消（保留我的）」，同一处改动只问一次。
- 重新打开关闭的标签：`Ctrl+Shift+T` 或文件菜单，按关闭顺序后进先出（最多 20 个），磁盘上已不存在的会被跳过。
- 编辑器右键菜单：撤销/重做/剪切/复制/粘贴/全选，分隔线后是上移行 / 下移行 / 切换注释 / 增加缩进 / 减少缩进，再后面是切换断点、设置条件断点…、运行到光标处，最后是转到定义、查找引用。菜单由应用自绘（不是系统原生菜单），圆角、边框颜色与底色跟顶部弹出菜单一致。
- 编辑增强：`Tab` / `Shift+Tab` 缩进与反缩进、回车自动缩进由编辑器组件内置（`IndentInline` / `OutdentInline` / smart indent）；**VB 的块级缩进规则由本应用提供**（`src/indent.rs` 的正则经 `gpui-base` 的 `LanguageConfig` 注册给 `vb`/`vb6`/`vba`：`Sub` / `Function` / `Property` / `Type` / `Class` / `Do` / `For` / `While` / `With` / `Select Case` / `Case` / `Else` 及以 `Then` 结尾的行在回车后多缩进一级，只在代码上下文生效）；**块结束行的行尾回车**与**行尾带注释的开块行**由 `CodeWindow::dedent_after_block_end` 补一次（组件的缩进规则只看光标之后的文本/上下文：块结束行尾时光标后面是空的，行尾注释又让光标算作「在注释里」），VB 文件的一级缩进固定为 4 个空格（与 VB6 和仓库里的脚本一致）。编辑菜单与右键菜单里也加了对应项；**括号配对高亮**由本应用实现（`brackets::matching_pair` 找配对，两个 `RangeDecoration` 画淡蓝色底），光标贴着 `(`/`[`/`{` 或它们的闭括号时高亮成对的两个括号，字符串与 VB 的 `'` 注释里的括号不算。
- 标签右键菜单（自绘，复用同一套菜单外观）：关闭、关闭其它、关闭右侧、关闭全部、全部保存、复制路径。**重命名/删除搬到了资源管理器条目的右键菜单**（那样能直接作用于没打开的文件与文件夹），文件菜单里另有「重命名当前文件…」「删除当前文件…」。重命名会先把未保存内容写回旧文件再改名（文件、文件夹都支持，目录下已打开的文件、断点、最近打开按前缀一起改写）；删除前弹确认框，删除后关掉受影响标签并清掉断点记录。
- 窗口位置与大小：每秒读一次窗口矩形，变化后写进 `layout.txt` 的 `window_bounds=`，下次启动按它恢复（尺寸过小或坐标离谱的坏数据自动忽略）。
- 会话恢复：退出时把还开着的标签（顺序）与当前激活的那个写进 `layout.txt` 的 `open=` / `active=`，下次启动逐个重新打开（磁盘上已不存在的文件跳过，最多 40 个），并回到当时的激活标签。
- 标签切换：`Ctrl+Tab` / `Ctrl+Shift+Tab` 在打开的文件之间循环，编辑菜单里也有对应项。
- 自动保存：设置 → 编辑器 →「保存」里开关（默认关闭）并设置间隔（5–600 秒）；每秒检查一次，到点后把所有有改动的文件按原编码写回磁盘，输出「已自动保存 N 个文件」，不移动光标、不打断输入。
- 设置持久化：编辑器（字号、换行、行号、折叠、自动闭合、自动保存开关与间隔、语法分析开关与语言）与输出（字号、时间戳、最多保留行数）设置随布局写进 `layout.txt`，重启后继续生效；读取时对超出范围的数值做夹取，缺键沿用默认值（旧布局文件可直接升级）。
- 对话框：运行参数、转到行 / 符号、转到文件、添加监视、条件断点、重命名文件六个输入框对话框 —— 打开即聚焦输入框，回车等同「确定」；删除文件用确认框（取消 / 删除）。
- 搜索面板（活动栏放大镜 / `Ctrl+Shift+F` / 查看 → 搜索）：项目内文本搜索，候选文件与「转到文件」相同（已打开 + 最近打开 + 资源管理器各根目录递归），可以把范围限定到某个文件夹（`选择…`，`×` 恢复全部位置）。关键字输入框回车或点放大镜按当前选项重搜，`Esc` 直接收起面板；`Aa` 区分大小写、`全词` 只匹配完整单词（前后不能是字母/数字/`_`）、`.*` 把关键字当**正则表达式**（`regex` crate 语法，`全词` 在正则模式下变成给整个模式套一层 `\b(?:…)\b`，替换串里可用 `$1` 引用分组；正则写错时状态行显示「正则无效」、不产生命中，也不会改动文件）；结果按文件分组，每行显示行号与命中附近的原文，**命中片段用醒目颜色高亮**，点击打开文件并跳到那一行；扫描在**后台线程**执行，状态行实时显示「搜索中… 已处理/总数 个文件」，结束显示「N 处结果 · M 个文件」，命中达到上限、读取预算用尽、跳过二进制文件或被新搜索取代时另有提示。改动关键字会立即作废旧结果并取消在途搜索，避免列表与输入不符。
  - **跨文件替换**：关键字下面是「替换为」输入框（**单独占一行**，窄面板下不会和按钮挤在一起），再下一行从左往右排「全部替换」「替换选中」「撤销替换」三个按钮（在替换框里回车触发「全部替换」）。替换只作用于**本次结果里出现的文件**，匹配条件沿用这次搜索的选项（不是输入框里正在改的文字），行尾原样保留，**写回按每个文件原来的编码**（GBK / UTF-16 / 带 BOM 的 UTF-8 都不会被转成无 BOM 的 UTF-8）；写完后输出「已替换 N 个文件、共 M 处」，没有未保存修改的已打开标签立刻刷新，有修改的标签不覆盖（留给外部修改检测去问），最后自动重搜一遍。
  - **逐处勾选 + 撤销**：每条命中左边有勾选框（新结果默认全勾，选项行右边是「勾选 N/M」，点一下全选、再点一下全不选）；**双击文件分组标题整行**可以把这一组一次全选/全不选——这一组全勾着就全不勾，否则全勾，别的文件不受影响（`search::toggle_file_selection`，纯逻辑有单测；标题行只认双击，单击不做事，也不画勾选框）；「替换选中」只动勾上的行——列表一行只有一条命中，所以勾上是把**这一行里的匹配全部换掉**，实际次数按 `replace_line` 的真实命中累加、输出里照实报。替换前会把要改的文件的原始字节读进内存（`search::snapshot_files`），「撤销替换」把它们逐字节写回（`search::undo_replace`，保留原编码与行尾；正则替换不可逆，所以不做「反向再替换一次」），只保留最近一次，撤销后按钮置灰。

### 解释器

- 语义检查 `sema.rs`：`Option Explicit` 下的未声明变量、`ReDim` 未声明数组、未定义的函数或过程（`With` 块内跳过，调用名是变量时跳过）；另外做 `Implements` 的成员一致性与签名一致性检查（漏实现接口公开成员、写了接口没有的 `接口名_xxx`、过程形式 / 参数个数 / 参数类型 / `ByRef`-`ByVal` / 数组形式 / `Optional` 与默认值 / 返回类型与接口不一致），这一项与 `Option Explicit` 无关，与 VB6 一样属于编译期错误。
- 声明顺序：`As 类型名` 与 `Implements 接口名` 支持**前向引用**——`Type` / `Enum` / `Class` 可以写在使用点之后（`#Include` 进来的定义同样算），解析前先扫一遍 token 收集声明名；解析结束时检查 `Type` 循环定义（直接或间接包含自身）并报语法错误。VB6 里类模块/标准模块各是独立文件、互相引用没有先后要求，所以这一条不产生新的语义基准，只是让单文件写法对齐。
- 补全 `completion.rs`：文档里的符号优先于关键字/内置函数；**成员补全**（`p.` 后列 UDT 字段、类字段与方法、`Me.`、`New Collection` / `New Dictionary`）。
- 编码 `encoding.rs`：BOM / UTF-8 / UTF-16LE/BE / GBK 探测，保存按原编码写回；状态栏显示编码，**点击后在上方弹出编码列表**（当前项打勾，选中即按该编码重新解码，有未保存修改时先确认）；另存为时可指定编码（UTF-8 / UTF-8 BOM / UTF-16 LE / UTF-16 BE / GBK）与换行符（保持原样 / CRLF / LF，选 CRLF 或 LF 时同时改写编辑器内容）。
- 对话框 `dialog.rs`：`MsgBox` / `InputBox` 都自己建窗口 + 自己跑消息循环（理由见「坑位备忘」），支持按钮组语义，**开窗期间点「停止脚本」会立刻关闭**。
- 内置函数：新增 `Shell(path[, windowstyle])`、`Command()`（配合「运行参数…」）。
- 调试：断点（按文件记录、写进布局文件，重启后还在）、暂停/继续、**单步三种**（逐语句 `F10` 进入被调过程、逐过程 `Shift+F10` 跳过被调过程、跳出过程 `Ctrl+Shift+F10` 回到调用者）、**出错时中断**（运行菜单里勾选，脚本遇到没处理的错误时停在出错行、输出面板写明「调试中断：第 N 行 …」，继续后照常抛错结束）、**暂停时改变量值**（运行菜单「设置变量值…」输入 `total = 42`，右侧按表达式求值、按声明类型转换后写回，变量面板立刻刷新）、**按调用栈切帧**（代码大纲「调用栈」里点哪一层就看哪一层的变量，求值/赋值作用域与编辑器高亮一并跟过去）、运行到光标处、清除所有断点、暂停快照（变量值 + 调用栈）、暂停行淡黄整行高亮。
- **条件断点**：菜单「运行 → 设置条件断点…」给光标所在行加条件（该行还没有断点会自动补上，留空表示清除条件）；条件在每次执行到该行时求值，为真才停，求值出错时照样停下方便排查；条件断点用黄色圆点区分普通断点；条件按 `breakpoint_condition=` 写进布局文件。断点与条件都列在代码大纲的「断点」一节里，便于查看。
- **变量监视**：菜单「运行 → 添加监视…」添加表达式（光标处的名字会预填），列表显示在「代码大纲 → 监视」一节；每次暂停都重新求值并显示值与类型，表达式出错时显示错误文本，恢复运行时清空结果；监视列表写进布局文件（`watch=` 行），重启后还在。求值不阻塞界面，「停止脚本」能中断求值表达式里的死循环。
- **脚本所在目录**：主窗口把当前文件路径放进 `RunOptions::script_path`。`Open`、`Dir`、`FileLen`、`FileDateTime`、`Kill`、`FileCopy`、`MkDir`、`RmDir`、`Name` 里的相对路径一律按脚本目录解析；`CurDir`/`ChDir` 只改脚本视角的目录，**不动编辑器进程的 CWD**；新增 `App.Path`（返回脚本所在目录，`Option Explicit` 下不误报）。`Open ... For Input` 省略 `As #n` 时会自动取最小空闲文件号（与 `FreeFile` 一致）。
- 成员赋值：`obj.A.B = v`、`obj.Method(实参).Prop = v`、`Set obj.Method().Prop = 对象` 与 `With` 块里的
  `.A.B = v` / `.Prop(1) = v` / `Set .X = 对象` 都按「中间层不带括号按属性读、带括号按调用」执行；
  普通赋值走 `Property Let`/字段、`Set` 走 `Property Set`（细节见下方「已完成」表的「链式成员赋值」「`With` 里的赋值」两行）。
- COM Automation（`com.rs`）：`CreateObject` / `GetObject`（路径与 moniker）/ ROT、`IDispatch::Invoke`（连续 VARIANT 实参数组、命名与 `ByRef` 实参、先属性后方法、默认成员 `DISPID_VALUE`）、任意维数与强类型 `SAFEARRAY`、`TypeName`（`IProvideClassInfo` + 接口名兜底）、**`For Each` 走 `_NewEnum` 的活枚举器**（每轮 `Next` 取一个，循环体里增删集合元素与 VB6 一样由服务器的枚举器决定）、**`WithEvents` 事件接收**（模块级变量与类字段，`IConnectionPoint` + 自带 sink，在语句边界与 `DoEvents` 里分派）。缺 UDT 记录封送，见「待办」第 6 条。
- 其他：转到定义 `outline::definition`、查找引用 `outline::references`、转到行/符号解析 `outline::goto_target`。

## 坑位备忘（改代码前先看）

- gpui 颜色：`hsla(h, s, l, a)` 的四个参数都被夹到 `0..1`，**色相不是角度**（黄色是 `1.0/6.0`）。
- 行号栏 `GutterDecoration` 用空范围会被画成「行边界」（两行之间），必须传整行范围。
- `RangeDecoration` 的 `Fill` 只覆盖文本范围（到行尾 + 一个空格），做不了整行宽度。
- `.truncate()` = 溢出隐藏 + 不换行 + 省略号；flex 子元素还要 `min_w_0()` 才能真正收缩。
- 脚本跑在后台线程，**不是前台线程**：`MessageBoxW` 会被创建在编辑器窗口后面（用户以为卡死），所以 `MsgBox`/`InputBox` 一律自建对话框 + `SetWindowPos(HWND_TOPMOST)`。
- 自建对话框（`CreateDialogIndirectParamW` + 自己的消息循环，`MsgWaitForMultipleObjects` 50ms 超时）才能在「停止脚本」时检查中断标志；`close_dialogs()` 还会给本进程残留的 `#32770` 窗口发 `WM_CLOSE`。
- 「出错时中断」只在**真正没人处理**的错误上触发：判定看 `error_handler()` 是 `None` `Some(Disabled)` 且 `is_root`，
  `On Error Resume Next` / `GoTo` 一律不打断；暂停后「继续」要把**原始错误**原样抛回去（脚本照常结束），
  只有「停止」才换成 `STOPPED_MESSAGE`。`DebugSnapshot.error` 是这个功能专用的字段，普通断点/单步永远是 `None`。
- 调试单步按**调用深度**决定停不停：`before_line(line, depth, …)` 里的 `depth` 是 `frames.len()`（模块级 0），
  逐过程只在 `depth <= 暂停时深度` 停、跳出只在 `depth < 暂停时深度` 停；暂停时把深度记在 `Inner::paused_depth`，
  所以界面侧不需要自己算深度。判定逻辑在纯函数 `debug::step_reached` 里，改动别把它埋回 `before_line` 的时序里。
- 调试轮询只在 `(is_paused, pause_epoch)` 变化时才取快照：快照可能几 MB，每 40ms 克隆会让界面发卡。监视求值结果走另一个轻量轮询（`take_eval_result`），一次只发一条请求（`watch_queue` + `watch_request`）。
- 监视求值发生在解释器线程仍停在 `before_line` 里的时候：求值前必须**放开调试锁**，并通过 `Runtime::evaluating` 让表达式内部的语句跳过暂停检查，否则调用脚本过程会再次进入 `before_line` 把自己锁死。条件断点的条件求值走同一条路径（`DebugQuery::Condition`），只是判断真假后接着往下跑而不是停住。
- 条件断点只在「命中断点且不是单步/暂停请求」时求值；条件本身出错时按「真」处理。编辑代码后断点行号由行号栏装饰反推，条件按行号顺序重新挂（数量不变时一一对应，数量变了只保留行号仍存在的条件），这是有意的启发式，别把它当成精确的行映射。
- 输入框单行模式下回车只 `cx.propagate()`，对话框里要自己挂 `.on_action(|_: &Enter, ..|)` 才能回车确认。
- 测试放在 crate 级文件（如 `src/outline_tests.rs`）并在 `src/main.rs` 用 `#[cfg(test)] mod xxx_tests;` 注册；写在模块内的 `#[cfg(test)] mod tests` 会触发 rustc 的宏展开递归上限。模块自己的测试要贴在模块旁边时，用 `#[cfg(test)] #[path = "run_tests.rs"] mod tests;`（`cli.rs`、`interpreter/mod.rs` 都这样）：模块路径不变、`use super::*` 照旧，文件本身也不会把主模块撑大。
- `src/vb6_golden_data.rs` 是**生成文件**：`tools/vb6_golden.py` 的四组常量都带 `#[rustfmt::skip]`，`cargo fmt` 不会把一行一条的用例拆成多行；`parse_data` 的正则也容忍拆行与尾逗号（`\s*,?\s*\)`），这样即便有人手工格式化过，`--check` 也不会误报「清单里有、数据里没有」。
- 输出面板的重复分组只合并**相邻**的同类错误/警告（`OutputLine::repeat`），复制全部输出时带「（重复 N 次）」，拖选复制仍是原始文本。
- 脚本路径：`RunOptions::script_path` → 运行时 `script_dir`（`App.Path`）/ `cwd`（相对路径基准）。`cwd` 只在解释器内部随 `ChDir` 变，绝不要改进程 CWD（编辑器、资源管理器、后台验证都会受影响）。没有脚本路径时两者都退回进程目录，保持旧测试/`run` 入口的行为。
- `Open` 的 `As #n` 是可选的（解析成 `Option<Expr>`）：省略时运行时用 `free_file_number`（与 `FreeFile` 同一逻辑）分配最小空闲号，之后用 `#1`、`#2`… 读写即可。
- 自动保存由主窗口里每秒一次的异步循环检查（`EditorSettings::auto_save_due` 判断开关/间隔/脏文件数），保存复用 `write_content`（按原编码写回并更新 `saved` 快照），不弹对话框、不抢焦点。
- 输入框聚焦：组件的 `focus_ring_style` 会在边框外再画一圈；本应用统一用 `.focus_ring(false)` 关掉，只在聚焦时把边框换成 `palette.sidebar_text`（大纲过滤框与所有输入框对话框都这样）。
- 文件拆分：`builtins.rs`（3705 行）已按职责拆成 `builtins/{mod.rs,constants,dates,files,format,settings,strings}.rs`，`call()` 与它直接用的小工具留在 `mod.rs`（1680 行）；`interpreter/mod.rs` 的 147 条内嵌测试搬到 `interpreter/run_tests.rs`。拆分时注意：**文档注释和 `#[cfg(...)]` 属性要从所属项那一侧带走**，否则会掉在上一个文件末尾（clippy 会报 `empty line after doc comment`）。`runtime.rs`（6873 行）也已按现有 `// ----------` 分区拆成 `runtime/{mod.rs,variables,control,statements,files,classes,records,expr,calls,callbacks,helpers}.rs`：`mod.rs` 只剩 744 行（类型定义 + `run_program*` + 调试挂点），最大的一块是 `statements.rs` 1829 行。做法要点：整段搬走后给每个文件包一层 `impl<'p, 'o> Runtime<'p, 'o> { … }`（原来所有方法都在同一个 impl 里），方法统一提 `pub(super)`；**inherent 方法不需要跨文件 `use`**，所以子模块之间互调不用互相引用。下一步只剩把 `builtins/mod.rs` 里 `call()` 的 `match` 按类别再拆一层。
- **拖动卡顿的根因是 debug 构建**：`QIDE_RESIZE_PROBE=1` 让程序自己连续改左栏宽度（走和拖分割条同一条 `set_dock_size` + 重绘路径），配合 `QIDE_UI_PROFILE=1` 量出真实帧间隔——全未优化的 debug 构建里**普通重绘约 30ms、改尺寸的那一帧 55–100ms**（我们自己的 render 只占 1–2ms，其余全在 gpui 的排版/绘制里）；同一二进制换成 `--release` 则一次慢帧都没有。所以 `Cargo.toml` 里加了 `[profile.dev] opt-level = 1` + `[profile.dev.package."*"] opt-level = 3`：依赖（gpui / gpui-component / 解释器依赖）全优化，本仓库代码保留调试信息——改完后同样的探针只剩 10 条慢帧、各段耗时从 1.4ms 掉到 0.1ms。**以后判断界面性能要用 `--release` 或这套 dev profile，别用默认的全未优化 debug。**
- 卡顿排查：gpui 是**即时模式**，每次重绘都会重跑所有可见面板的 `render`，所以「拖动分割条时的卡顿」= 每帧开销被连续重绘放大。`src/ui_profile.rs` 提供采样：`$env:QIDE_UI_PROFILE=1; .\first.exe` 后拖一次分割条，超过 8 ms 的段会写进 `%APPDATA%\gpui-first\ui-profile.log`（形如 `间隔 16ms | output=12.0ms search=1.5ms`，日志上限 512 KB 自动重开）。已经挂上计时的段：`frame`（根 render）、`status-bar`、`editor-pane`、`explorer`、`outline`、`search`、`output`、`ai-panel`。顺手修掉一处确定的白干活：状态栏的「N 行 · M 字符」原来每帧都扫全文，现在按「文件 + 字节数」缓存（`CodeWindow::status_counts`）。**输出面板已按视口裁剪**：每行都是单行文本、行距固定，所以 `output::visible_window` 直接用「滚动偏移 / 行距 / 视口高度」算出这一帧要渲染的行区间（上下各留 8 行余量），区间之外用两块等高的留白撑住总高度，滚动条长度与滚动位置都不变；行距由 `on_children_prepainted` 实测（首帧或内容变化那帧先整篇渲染一次来量）。`OutputGeometry` 现在带 `start` / `count` / `pitch`，`position_at` 按 `start` 偏移换算，窗口外的点击夹到首尾行。搜索结果列表还没裁剪（命中上千条时同样每帧上千行），等有 profile 数据再决定要不要照做。
- 悬浮提示的样式只能动本地补丁：`Button::tooltip` 只收文本，气泡的 `text_sm()` + `popover_foreground` 写死在 `vendor/gpui-component/src/tooltip.rs` 里（没有公开的自定义内容入口），要改字号/颜色就改那里——现在是 `text_xs()` + `muted_foreground`，同时 `theme::apply` 把 `colors.muted_foreground` 对齐成 `palette.muted_text`，提示的灰才和界面上其它次要文字一致。
- 字号：只把**弹窗**调小（标题 `.text_sm()`、说明 `.text_xs()`、输入框 `.xsmall()`、按钮 `.small()`），面板、菜单与设置窗口保持组件默认字号（`theme.font_size` 不要动，否则整块界面都会变小）。
- 资源管理器拖动：拖动数据是 `explorer::ExplorerDrag`（`path` 是被拖的那一行，`items` 是随行的一批选中项），文件夹行用 `drag_over::<ExplorerDrag>` + `on_drop` 接收，落地前先过 `explorer::move_targets(items, 目标)`：`can_move_into` 判断不能拖进自己/子目录、目标必须存在且不是当前父目录，另外跳过「父目录也在这一批里」的子项（搬父目录时子项已经跟着走了）。标签栏用 `editor_pane::DropFileAction` 接收（只打开主条目）。行点击回到「文件夹展开/收起、文件打开」两个回调（`OpenFileAction` / `ToggleFolderAction`）。多选本身靠圆点 + `Ctrl`/`Shift` 点击维护 `ExplorerSelection`，行高亮由 `ListItem::selected` 单行绘制、其余靠圆点。
框选（rubber band）：在树的**空白处**按住左键拖动，半透明矩形由 `canvas` + `window.paint_quad` 画在树上方；扫过哪些行由每一行的 `on_mouse_move` 报告（虚拟列表的行高/滚动偏移都交给组件，不自己算），松手时取扫过下标的最小..最大之间的可见行（跳过根项与占位行）并进入选择模式；没框到任何行时清空选择。行在 `on_mouse_down(Left)` 里打 `Cell<bool>` 标记，容器据此区分「按在条目上」与「按在空白处」；树右侧 16px 是滚动条，按在那里不开始框选（树 bounds 由容器的 `on_children_prepainted` 缓存）。历史坑：曾用 `window.modifiers()` 在 `on_mouse_down(Left)` 里判断 Ctrl/Shift 并给行加 `multi_select_bg` 底色，但 `ListItem::selected` 的选中态始终只有一个、行内自绘底色也不会保留，实际表现是「Ctrl 点第二条，第一条高亮就消失」，已整体移除（相关代码：`ExplorerPanel::selected`、`selection_for`、`CodeWindow::delete_paths_dialog`、`Palette::multi_select_bg`）。
- 转到文件：候选文件由 `CodeWindow::file_candidates` 拼装（已打开 → 最近打开 → 资源管理器各根目录递归，`workspace::collect_files` 跳过 `.` 开头的目录与 `target`/`node_modules`/`vendor`/`.git`，不跟随符号链接，深度上限 12、总数上限 `MAX_GOTO_FILES`），匹配用纯函数 `workspace::fuzzy_score` / `best_match`（完全相等 > 前缀 > 包含 > 子序列，同分取路径更短）。查询串含 `/` 或 `\` 时按完整路径匹配（`\` 先归一成 `/`），否则只比文件名。扫描在按键回调里同步执行，因此候选数量必须设上限。`Ctrl+P` 额外注册了 `Dialog` / `Dialog > Input` 上下文下的 `Cancel`，对话框里再按一次等于关闭；`goto_file_dialog` 开头还有 `has_active_dialog` 守卫，避免叠出第二个对话框。
- 资源管理器树：gpui-kit 的 `TreeItem::is_folder()` 就是 `!children.is_empty()`，所以**空目录必须补一个占位子项**（`explorer::EMPTY_FOLDER_SUFFIX`，渲染成一行灰色的「（空文件夹）」），否则会显示成文件、点不开。右键菜单挂在 `ListItem::on_mouse_down(MouseButton::Right, ...)` 上（面板自己的 `context_menu` 之外不弹系统菜单），菜单本身复用 `editor_pane::context_menu_element`。菜单项的**顺序与分隔线由纯函数 `panels::explorer_menu_entries` 决定**（`panels_tests` 保证第一项不带分隔线、也不会出现连续分隔线，之前「新建文件」上方多一条空线就是这么来的）；`context_menu_element` 另外加了一道保险：第一项永远不画分隔线。路径改名/删除后统一用 `workspace::reprefix_path` 改写打开的文件、断点、条件断点与最近打开：只有「完全相同」或「后面紧跟路径分隔符」才算目录下的子项，避免 `old` 误匹配 `older`。
- **菜单项状态别去读 CodeWindow 实体**：`menu::render` 是在 `CodeWindow::render` 里拼的，那一刻实体正被更新，
  `handle.upgrade().read(cx)` 会直接 panic（`cannot read first::app::CodeWindow while it is already being updated`，
  表现为**启动一闪就退出**，崩溃日志里能查到这条）。需要什么状态就加进 `MenuState`（在 `app/view.rs` 里从 `self` 取），
  或者用 `app_action` 的回调在点击时再拿。
- 括号高亮：`RangeDecorationCollection::set` **每次调用都会 `notify`**（即使装饰完全相同），所以在编辑器观察者里无条件 `set` 会把自己再次唤醒，形成死循环——表现为**程序启动即 100% CPU 卡死**。现在用 `CodeWindow::bracket_pairs`（`BracketHighlight::{Unset,None,Pair}`）缓存上一次画过的配对，只有变化时才 `set`。以后凡是「在实体观察者里设置装饰」都要先做同样的变化判断。
- 搜索面板：`crate::search` 里只有纯逻辑（`find_in_text` / `search_files_with_progress`），面板只做渲染与回调，所以匹配规则可以直接单测。扫描跑在后台线程（`cx.background_executor().spawn`），靠三道限制兜底：`MAX_SEARCH_BYTES`（单文件 2 MiB）、`MAX_SEARCH_TOTAL_BYTES`（单次总计 32 MiB，`search_files_with_budget` 供测试注入小预算）、`MAX_SEARCH_HITS`（500 处）。取消用 `SearchProgress::cancel`（后台循环每个文件检查一次），结果回填再看 `SearchPanel::search_id`：编号不一致就丢弃，避免旧搜索结果覆盖新关键字的结果。进度靠 `spawn_progress_poll` 每 120ms `cx.notify()` 一次，扫描结束即停止。每行只记第一处命中，行号/列号都是 1 基；含 NUL 字节（且没有 BOM）的二进制、读不了的文件计入 `skipped`；**GBK / UTF-16 源码改用 `encoding::decode` 解码后照常搜索**。大小写不敏感用逐字符 `to_lowercase()` 比较，不能用整行 `to_lowercase()`（`İ` 之类小写后字符数会变）。
- 跨文件替换：纯逻辑在 `search::replace_in_text`（逐行处理，`\r\n` / `\n` 原样保留；匹配复用 `match_at` + `is_word_boundary`，与搜索规则完全一致）与 `search::replace_in_files`（先用 `read_text` 过一遍与搜索同一套「过大/二进制/非 UTF-8」门槛，所以写回的仍是原编码内容）。界面只做编排：`SearchPanel::replace_all` 从**结果**里取去重后的文件列表 → 替换 → `CodeWindow::after_replace` 刷新受影响且**没有未保存修改**的标签（有修改的留给外部修改检测弹框，避免覆盖用户编辑）→ 再 `run()` 重搜一遍。
  逐处替换走 `search::replace_selected`（按（路径, 行号）分组，只重写勾中的行，编码/行尾规则与 `replace_in_files` 共用 `split_line_ending`），撤销走 `search::snapshot_files` + `search::undo_replace`（替换前把原始**字节**读进内存、撤销时逐字节写回，面板只留最近一次；`ReplaceUndo::paths()` 给 `after_replace` 刷新标签用）。**别改成「再替换一次」来回退**：正则替换不可逆。
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
- COM dispid 缓存挂在 `ComObject` 上（`Rc<RefCell<HashMap>>`，`Clone` 共享）：键是成员名小写，**只缓存成功结果**；带命名参数的调用会顺手缓存成员名。改 `dispid` / `dispids` 时注意别在 `borrow()` 还没释放时 `borrow_mut()`。
- 资源管理器选择模式的快捷键用独立的 `Explorer` 键上下文（`panels.rs` 里 `track_focus` + `key_context`），进入选择模式/开始框选时由 `focus_panel` 把焦点交给面板；不要把 `Ctrl+A` / `Delete` / `Esc` 注册成全局绑定，否则会抢编辑器的键。

## 后续计划（按建议顺序）

界面侧的框选（rubber band）与多选拖动都已完成；解释器侧待办见下一节（COM UDT 封送已确认 VB6 自己也做不到，
只剩诊断措辞；`For Each` 活枚举器、`DoEvents` 同步分派事件、COM 错误号、`GetObject` 单参数语义、
链式成员赋值、`With` 赋值与 `Format` 自定义数值格式
已做完，`Slot` 扁平化已做完第一刀，值内联已否决；还剩 Win32 混合浮点参数 >4 与 `AddressOf` 数组/UDT 形参，
两条都给出了「暂不做」的理由）。

**当前进度一句话版**：解释器语义（含 `Null` 规则、参数校验、全部内置函数与 `Format` 自定义格式、COM Automation
主要能力、`WithEvents` 事件、`For Each` 活枚举器、`DoEvents` 同步分派事件、COM 错误号、`GetObject` 语义与
链式/`With` 赋值）已按本机 VB6 oracle 对齐；界面多选（圆点/Ctrl/Shift/拖动/框选/`Ctrl+A`·`Delete`·`Esc`）也已完成；
COM 成员访问有 dispid 缓存，Windows CI 会跑 build + test + clippy。
剩下未做的两条（Win32 混合浮点参数 >4、`AddressOf` 数组/UDT 形参）都有明确理由与替代写法，见待办第 11、12 条。

## 解释器：语义完善进度与待办

**验证方式（改语义前必须先做）**：语义一律以本机 VB6 真编译为 oracle。用例写在
`tools/vb6_golden_cases.txt`（表达式）/ `vb6_golden_errors.txt`（失败语句 → `Err.Number`）/
`vb6_golden_approx.txt`（浮点末位不稳的表达式，`表达式 | 容差`，按相对误差比对）/
`vb6_golden_scripts.txt`（整段脚本，`=== 名字` 分段，`@` 开头行写模块级选项），
跑 `python tools/vb6_golden.py` 用 `Vb6.exe /make`（`CompilationType=-1` 即 p-code）生成
`src/vb6_golden_data.rs`，再由 `cargo test`（`vb6_golden_tests.rs` 的 4 条数据驱动用例 + `builtins_tests.rs` 里若干条实测值用例）逐条比对。
与 VB6 有意不同之处写进 `KNOWN_DIFFERENCES` / `SCRIPT_KNOWN_DIFFERENCES` 并在
`docs/脚本语言.md` 写明理由。当前覆盖 **814 条表达式 + 76 条错误号 + 7 条近似用例 + 94 段整脚本**。
只想看某几个表达式的取值时用 `python tools/vb6_probe.py "表达式"` 或 `--file 清单.txt`，
整段脚本用 `python tools/vb6_probe.py --script 清单.txt`（同 `vb6_golden_scripts.txt` 的格式；
需要 WSH/ADODB 之类 COM 服务器的探测就走这条）——两种模式都只编临时工程、不写黄金数据。
只改一段脚本时用 `python tools/vb6_golden.py --scripts-only --only 名字前缀`（其余用例沿用现有数据），
只改表达式时用 `--expr-only`；`--check` 只比对数据文件与清单、不启动 VB6；
编译失败时加 `--bisect` 会让它二分编译、报出具体是哪一条用例。
生成结束后会打印内置函数覆盖率，并按三类分开报：「只在单元测试里覆盖」「两边都没覆盖」（真缺口）
「依赖环境/时钟/随机源、不做黄金覆盖」（`ENVIRONMENT_DEPENDENT` 白名单，`MsgBox`/`Shell`/`Command`/
`Now`/`Rnd`/`CreateObject` 这类同一段脚本两次跑结果不同的，硬塞进黄金清单只会来回抖）。
表达式用例会被切成若干个 `PartN` 过程（`CHUNK_LINES = 300`），`Form_Load` 只负责打开输出文件再逐个调用——
VB6 单个过程上限 64KB，用例过千条全塞进 `Form_Load` 会直接报「过程太大」；用例里用到的变量因此放在模块级，
每个 `Part` 自己写一句 `On Error Resume Next`（VB6 的错误处理按过程生效）。
VB6 路径用环境变量 `QIDE_VB6` 覆盖（默认 `E:\vb\VB 6.0\Vb6.exe`）。
注意：PowerShell 5.1 向原生程序传含双引号的参数会**吃掉引号**，带引号的用例请写成清单文件再用 `--file` / `--script`。

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
| COM `For Each` 改活枚举器 | 原先进循环前把 `IEnumVARIANT::Next` 一次取成快照，循环体里增删集合元素的效果被吃掉。现在 `For Each` 的循环里持有枚举器、每轮 `Next(1)` 取一个（`Exit For` / 异常跳出时随栈释放），效果由服务器自己的枚举器决定，与 VB6 一致：往 `Scripting.Dictionary` 里边遍历边 `Add`，新键会被枚举到（`for_each_over_a_com_collection_uses_a_live_enumerator`，需 WSH，标 `#[ignore]`） |
| `DoEvents` 同步分派事件 | 事件分派抽成 `Runtime::pump_com_events()`，**语句边界**与 **`DoEvents`** 两处调用：`DoEvents` 在 VB6 里是消息循环，事件在其中同步执行完，现在 QIDE 的 `DoEvents` 返回时排队事件也已处理（`DoEvents` 不再走纯函数的内置函数表，运行时先分派再让出时间片）。`pumping_events` 标志防止处理过程里再触发的事件递归分派，新事件留到下一次边界 |
| COM 错误号（`EXCEPINFO.scode`） | 旧实现一律取 `scode` 低 16 位，于是 VB6 报 -2147024894 的地方 QIDE 报 2。本机 VB6 实测的规则：`0x800Axxxx` 取低 16 位（FSO 的 53/76/68）、`E_INVALIDARG` 报 5、**其余系统包装码原样当 `Long` 报**（-2147024894 / -2147024891 / -2147024843），VB6 并没有整张 Win32→VB 表。`exception_error_number` 按此实现，`com_number_tests` + （`--ignored`）`com_error_numbers_match_vb6` 钉住 |
| `GetObject` 单参数语义 | VB6 实测：`GetObject(x)` **一律**按路径/moniker 处理（`GetObject("ProgID")` 不是「取活动对象」的简写）。现在路径缺失/非 OLE 文档报 432、空串报 5、裸名字交给 moniker 解析（语法错误报 -2147221020）、`new:` 找不到类报 429、`clsid:` 之类按原 HRESULT 报；两参数形式先解析类名（缺类 429）再加载文件（失败 432）。`GetObject(, "ProgID")` 仍取活动对象、`GetObject("", "ProgID")` 仍新建实例 |
| 省略实参与语句级表达式 | 解析层新增 `Expr::OmittedArg` / `Stmt::Eval`：`F a, , c` / `F(a, , c)` 的空位对应可选形参（`Variant` 且无默认值时 `IsMissing` 为真，有默认值按默认值绑定，必填形参报 449），`obj.Method(实参).属性` 单独成句会先调用再读属性（原先报「`.成员` 调用只能出现在 `With` 块内」）。黄金脚本新增 `omitted_arguments`（VB6 实测 `QideMissing(, "x")` = `True/False`、`QideMiddle(1, , 3)` = `1/True/False`）。**语句级表达式比 VB6 宽**：本机 VB6 `/make` 实测 `p.X`、`h.Self2()`、`h.Self2().Value`、`With` 里的 `.Kiddie(2).Value` 单独成句都编译不过，QIDE 都接受（求值后丢掉结果），见「已知遗留」 |
| 链式成员赋值 | 赋值语句的中间层**不带括号按属性读、带括号按调用**（类 / COM / 记录三种容器统一走 `read_member_chain` / `read_member_step` + `assign_through_members` / `assign_value_member`）：`h.Self2().Value = 7`、`h.Method1(3).Value = 13`、`h.PropChain(2).Value = 25`、`h.Self3().Value = 27`、`h.Inner.Value = 21`、`h.Self2().Other.Value = 23`、`Set h.Self2().Other = e`、`Set h.Inner.Other = e` 全部按「先调用/读属性，再在返回的对象上赋值」执行；`Set` 语句也接受空括号后接成员路径。原先 `()` 被当成下标，报 438（类）或 5（记录）。本机 VB6 类模块工程（`Class=Holder; Holder.cls` + `Form_Load`）逐条实测，回归测试 `chained_assignment_*` |
| `With` 里的赋值 | `.X = 10` 原先一律当 `Set`（`set_op=true`）走 `Property Set`，只有 `Property Let` 的类会报 438；现在普通赋值走 `Property Let` / 字段，新增解析 `Set .X = 对象` 走 `Property Set`。同时支持链式：`.Inner.Value = 31`、`.Kiddie(2).Value = 33`、`.Fields("id").Value = 1`（COM，ADODB 实测）与 `.A.B = v`（COM 原先直接报「暂不支持链式成员赋值」）。本机 VB6 类模块工程实测 5 条（`.Value = 10`→10、`Set .Child = e`→True、`.Child = e` 报 438、两条链式），回归测试 `with_assigns_through_property_let_and_chained_members` + （`--ignored`）`with_block_writes_through_com_chained_members` |
| `Format` 自定义数值格式 | 旧实现只认「前缀字面量 + 纯占位符 + 后缀」，数字占位符里夹字面量（`000-00`、`00:00`）、`\x`/`"…"` 转义、千位分组与缩放的组合、`!`、`c` 都错或直接报「不支持的格式字符」。现按 VB6 的模板语义重写（`parse_format_chars` + `push_positional_section` + `push_fraction_literals` + `render_section_scientific`）：字面量按位置保留、数字右对齐填占位符（多出的位数接最左占位符前）、占位符之间的逗号千位分组、整数占位符后的逗号缩放、小数点后的逗号忽略、`%` 只在最后一个占位符后乘 100、段首 `!` 整段按字面量、无占位符整串按字面量、`&` 不补空格、单个 `c` 是 `General Date`。39 条新黄金用例（`python tools/vb6_golden.py` 重生成，689 条表达式全过） |
| `GetAttr` / `SetAttr` | 原先两个函数与 `vbNormal`…`vbAlias` 常量都没有。现在 `GetAttr` 返回 Win32 属性位（`FILE_ATTRIBUTE_NORMAL` 0x80 清成 `vbNormal` 0，其余位原样保留，例如新建文件 8224），`SetAttr` 只接受 vbReadOnly/vbHidden/vbSystem/vbArchive 的组合（其它位报 5、属性非数值报 13、缺文件报 53），`vbNormal` 映射成 `FILE_ATTRIBUTE_NORMAL` 调 `SetFileAttributesW`；目录的 `vbDirectory` 位由系统保留（设 `vbHidden` 后 `GetAttr` 是 18、改回 `vbNormal` 仍是 16）。本机 VB6 脚本实测，回归测试 `builtins_tests::get_attr_and_set_attr_match_vb6` |
| 三角函数与数学函数的 `Null` | 原先没有 `Sin`/`Cos`/`Tan`/`Atn`（VB6 的标准数学函数），且 `Abs`/`Int`/`Fix`/`Round`/`Hex`/`Oct` 收到 `Null` 一律报 94。现在补上四个三角函数（参数与结果都是 `Double`，`Null` 报 94），并让 `Abs`/`Int`/`Fix`/`Round`/`Hex`/`Oct` 对 `Null` 原样返回（VB6 实测 `IsNull(Abs(Null))` 是 `True`；`Sgn`/`Sqr`/`Exp`/`Log`/三角函数/`Val` 仍报 94）。二十条新黄金用例；`Tan(π/2 附近)` 与 `Sin(π 附近)` 的最后几位因 x87 `FSIN`/`FPTAN` 与 libm 的差异放进了**近似用例**清单 `tools/vb6_golden_approx.txt`（按相对误差比对，见「黄金测试工具」一行） |
| 财务函数 | 新增 `src/interpreter/finance.rs`，13 个 VB6 财务函数全部实现：`SLN`/`SYD`/`DDB`/`FV`/`PV`/`Pmt`/`NPer`/`IPmt`/`PPmt`（闭式公式，求值顺序按 VB6 实测校准）与 `NPV`/`IRR`/`MIRR`/`Rate`（数组 / 迭代）。语义都来自本机 VB6：`type` 形参非 0 即期初付款（`True` 也算）、期初付款时第 1 期 `IPmt` 是 0、`DDB` 末期不超过「期初净值 − 残值」（支持小数期，`DDB(10000,1000,5,4.5)` = 669.251522224642，VB6 是几何式而不是逐期循环；首期钳制成「成本 − 残值」、后续各期钳制后不小于 0，`DDB(500,1000,5,1)` = -500、`DDB(10000,2000,3,3)` = 0）、`NPV` 的 `values(0)` 是第 1 期、`MIRR` 用 `-NPV(再投资率, 正) × (1+再投资率)^n / (NPV(融资率, 负) × (1+融资率))`、零利率恰好是解时 `Rate` 报 5（`Rate(10,-100,1000)` 就报错）。`IRR`/`Rate` 用割线法，与 VB6 的差 < 1e-11（VB6 自己收敛到 1e-7 就停）。零利率恰好是解时 QIDE 报 5（与 VB6 默认 `guess = 0.1` 一致；VB6 换个 `guess` 偶尔会收敛到 1e-9 量级的值，这种迭代偶然性没有复刻）。回归测试 `financial_functions_match_vb6` / `financial_cash_flow_functions_match_vb6` / `financial_iterative_results_stay_close_to_vb6` / `financial_functions_raise_vb6_error_numbers`。**这批用例没有进黄金数据**：VB6 用 x87 长双精度，`IPmt(0.05,5,10,10000)`、`MIRR(c,0.1,0.1)` 这类两段式结果末位差 1 ulp，逐位比对必然失败，所以改用「字符串全等 + 1e-9 容差」两条测试 |
| 黄金测试工具 | `tools/vb6_golden.py` 从「一键全量重生成」扩成可用的小工具：`--check` 只比对 `src/vb6_golden_data.rs` 与四份清单（不启动 VB6，改完清单能立刻发现不同步）；`--expr-only` / `--scripts-only --only 名字前缀` 支持增量重生成（其余用例沿用现有数据，实测只重生成一段脚本后数据文件哈希不变）；新增**近似用例**通道 `tools/vb6_golden_approx.txt`（`表达式 | 容差`，Rust 侧按 `|QIDE − VB6| <= 容差 × max(|VB6|, 1)` 比对），三角函数极点与 `IRR`/`Rate` 这类末位不稳的值终于能进 oracle；编译失败时把 VB6 报的行号翻回用例名（VB6 常指向过程头，只能尽力而为）；`/make` 首次静默返回时按 exe 是否生成自动重试；`--bisect` 用二分编译把「哪条用例导致编译失败」定位到具体一行（VB6 报的行号常落在模块头部，只能尽力映射）；`.frm` 固定按 cp936 写（不再随本机 ANSI 代码页漂移）；`Debug.Print` 换写文件时跳过字符串与注释（原来会把字面量 `"Debug.Print"` 也改掉）；结尾还会提示「只在单元测试里覆盖」「黄金与单元测试都没覆盖」的内置函数 |
| 崩溃日志 | 界面线程 panic 时进程直接退出、什么都不留。新增 `src/panic_log.rs`：`main` 开头装 `panic` 钩子，把「UTC 时间 / 版本 / 位置 / 消息」追加到 `%APPDATA%\gpui-first\crash.log`（超过 256 KB 删掉重开），写完再调用默认钩子（`RUST_BACKTRACE=1` 的堆栈输出不受影响）；时间格式化自己算（`civil_from_days`），不引日期库。`panic_log_tests` 覆盖闰日/跨年/无位置三种情况 |
| 搜索逐行分配 & `Slot::narrow` 结论 | 搜索/替换原来每处理一行都 `line.chars().collect()` + `query.text.chars().collect()` 各分配一次 `Vec`，大文件等于几十万次小分配；改成关键字字符表每次搜索只建一次（正则同理），`match_span` / `replace_line` 收 `&[char]`。另外把「`Slot::narrow` 改成按需分配」试了一遍并**否决**：`Slot` 会克隆，原始槽标注为 `None` 时克隆各自独立，`set_narrow` 写在克隆上就丢——`VarType`/`Get`/`Put` 的窄类型共 5 条测试当场失败，结论已写进 `docs/架构.md` 热路径清单，避免重复踩 |
| 搜索正则模式 | 选项行新增 `.*`：关键字按 `regex` crate 语法匹配（复用已有依赖，不新增包），`Aa` 变成 `case_insensitive(false)`、`全词` 变成 `\b(?:模式)\b`；正则只在每次搜索前编译一次（`SearchQuery::compile`），逐行匹配用编译结果，替换走 `Regex::replace_all`（替换串支持 `$1`）；命中行高亮改用 `SearchHit.length`（正则命中长度不固定，原来按关键字字符数取会切错）。正则写错时 `SearchResults.invalid_pattern` 置位、状态行显示「正则无效」，搜索不扫描、替换原样返回。新增 6 条单测（命中列号/长度、大小写、全词包裹、分组替换、无效正则、字面量模式不受影响） |
| 过程名改借用 | `Frame.procedure` 与 `statics` 的键从 `String` 改成 `&'p str`（过程名来自 AST、整次运行不变），每次调用少一次 `String` 分配 + 拷贝；`Frame<'p>` 多一个生命周期参数，`Frame::default()` 与调试快照那两处拼字符串的地方改成 `map(str::to_string)`。交错 A/B（`tools/bench.py --old … --new … --runs 6`）：`procedure_call` 0.358s → 0.343s（**−4.2%**），`numeric_loop` / `array_read` / `array_increment` 都在 ±0.5% 噪声内，`bubble_sort` +2.2%（噪声）。下一步才是形参名（`locals` 的键改 `&'p str`），见下面「变量名改 `Cow`」 |
| 变量名改 `Cow` | `Frame::locals` / `globals` / `scope_mut()` 的键从 `String` 改成 `Cow<'p, str>`：过程名与形参名直接 `Cow::Borrowed`（原始字符串活在 AST 里，整次运行不变），每次调用少一次形参名的 `String` 分配 + 拷贝。名字来自语句本身（`Dim` / `Const` / 隐式声明 / 实例字段）时借用活不过 `'p`，改走 `Cow::Owned(name.to_string())`——所以 `set_var` / `declare` / `declare_slot` 的形参仍是 `&str`，只在插入分支复制；写已有变量走 `lookup_slot(&str)`，靠 `Cow: Borrow<str>` 零分配。踩坑记两条：①`get` / `contains_key` 必须传 `&str`（`Cow<'_, str>` 没实现 `Borrow<String>`，传 `&String` 直接编译不过）；②表达式位置写不出 `Cow<'p, str>::Borrowed`（`<` 被当成小于号解析），只能写 `Cow::Borrowed` 靠推断。交错 A/B 两次独立测量（`--runs 4` / `--runs 6`，每边取最小值）：`procedure_call` 0.366 → 0.311 s（**−15.0%**）与 0.472 → 0.441 s（**−6.6%**），`array_increment` −3.4%，`array_read` / `bubble_sort` / `numeric_loop` 在 ±1.3% 噪声内；`cargo test` 仍 848 passed，`clippy` 干净 |
| 内置函数覆盖率补齐（`StrConv` / `TimeValue` / `IsObject`） | 按覆盖率报告补 oracle 时用本机 VB6 探针发现三处语义偏差，都改成 VB6 实测行为并进了黄金清单：①`StrConv(…, vbProperCase)` 只有**空白**才是词边界、且只有词首字符本身是字母时才大写（`hello,world/x-y` → `Hello,world/x-y`，原来把逗号/斜杠也当边界；`123abc` / `a-b` 同理）；②`TimeValue` 支持 12 小时制（`AM`/`PM` 紧贴或空一格、`12:30AM` → 0:30:00、`13:00 PM` 的后缀被忽略），分秒按整数且校验范围（`12:34:56.5`、`0:60`、`25:00` 报 **13**，原来收小数秒、失败报 5），并且只接受字符串与 `Date` 值（`TimeValue(0.75)` / `TimeValue(1)` 报 13，`TimeValue(CDate(0.75))` 才是 18:00:00）；`CDate` / `IsDate` 走同一个时间解析器，`IsDate("1:30 PM")` 随之变成 `True`。③新增 43 条黄金用例（表达式 803 条、错误号 76 条），`python tools/vb6_golden.py --expr-only` 重生成。`vb6_golden.py` 的内置函数覆盖率报告改成三类：只在单元测试里覆盖 / 两边都没覆盖（真缺口）/ `ENVIRONMENT_DEPENDENT` 白名单（`MsgBox`、`Shell`、`Command`、`Now`、`Rnd`、`CreateObject` 这类依赖界面、进程环境、时钟或随机源的，同一段脚本两次跑结果不同，本来就没法做 oracle）。`Rnd`/`Randomize` **明确不复刻** VB6 序列：本机实测同一段 `Randomize 42` + 三次 `Rnd` 两次跑出的值都不一样（0.3391077… 与 0.4311945…），种子不只由实参决定 |
| `Nothing` 独立成值 | 原来 `Nothing` 被解析成 `Empty`，于是 `VarType(Nothing)` 是 0（VB6 是 9）、`TypeName` 是 `Empty`、`IsObject(Nothing)` 是 `False`、`IsEmpty(Nothing)` 是 `True`、`Dim o As Object` 的初值也被当成空值。新增 `Value::Nothing` 变体，`Nothing` 字面量、`Set x = Nothing`、对象变量的默认初值（`default_value(Object)`）都用它：`VarType` 9 / `TypeName` `Nothing` / `IsObject` 真 / `IsEmpty`·`IsNull` 假 / `Nothing Is Nothing` 真 / `ObjPtr` 0；`Dim v`（`Empty`）写 `v Is Nothing` 按 VB6 报 **424**（原来返回 `True`）、未赋值对象取成员与 `If Nothing Then` 报 **91**、`CallByName` 落在 `Nothing` 上报 5（空 `Variant` 仍 424）、COM 侧 `Nothing` ↔ `VT_DISPATCH` 空指针。VB6 把 `CStr(Nothing)` / `Len(Nothing)` / `Nothing & ""` 判成编译期错误，QIDE 无运行时行为可对齐，统一按空串处理（已写明）。新增 11 条表达式 + 1 段脚本（`nothing_semantics`，含 424/91 两条错误号）。顺带修掉黄金工具的过程体积上限：表达式用例切成 `PartN` 过程（`CHUNK_LINES = 300`）、变量提到模块级、每个 Part 自带 `On Error Resume Next`，否则用例过千条时 VB6 报「过程太大」 |
| `_NewEnum` 缺失对象的 `For Each` | 原来 QIDE 直接报 438、循环体一次也不跑。本机 VB6 实测（`WScript.Shell`，`On Error Resume Next` 下）：循环体照样跑**一次**（此时循环变量是 `Empty`、`IsObject` 假、`IsEmpty` 真），`Err.Number` 已经是 **438**，然后 `Next` 报 **92**（「For 循环未初始化」）；循环体里写 `Exit For` 也照样报 92。现在按此复刻（先把 438 记进 `last_error` 再跑一次循环体，最后抛 92），只在这一种错误处理模式下复刻——`On Error GoTo` 之类仍把 438 交给处理过程。回归测试 `for_each_over_object_without_new_enum_matches_vb6`（`#[ignore]`，需要 WSH，`cargo test -- --ignored` 跑） |
| 搜索替换：逐处勾选 + 撤销 | 原来只有「全部替换」，一次误替换没法回退。现在每条命中左侧有勾选框（新结果默认全勾，选项行右侧「勾选 N/M」点一下全选、再点一下全不选），新增「替换选中」——列表一行只有一条命中，所以勾上是把**这一行里的匹配全部换掉**，次数按 `replace_line` 的真实命中累加。新增「撤销替换」：替换前用 `search::snapshot_files`把要改的文件的**原始字节**读进内存（含原编码与行尾），撤销时 `search::undo_replace` 逐字节写回（正则替换不可逆，不做「反向再替换」），只保留最近一次、撤销后按钮置灰。模型层新增 `replace_selected` / `snapshot_files` / `undo_replace` 与共用 `split_line_ending`；替换输入框单独占一行、按钮在下一行从左往右；**双击文件分组标题整行**切换该文件的勾选（`search::toggle_file_selection`，标题行不画勾选框）。6 条新单测（只改勾中的行、一行多匹配全换、GBK 原编码写回、撤销逐字节还原、空输入/坏正则不动文件、按文件切换勾选只动那一组） |
| 调试：单步跳过 / 跳出过程 | 原来只有一个「单步」（`Mode::Pausing`，任何深度都停，实际是逐语句）。现在 `before_line` 多收一个**调用深度**（`Runtime::debug_check` 传 `frames.len()`，模块级 0），`DebugControl` 新增 `step_over()`（`Mode::StepOver{depth}`，只在**不浅于**暂停深度处停）与 `step_out()`（`Mode::StepOut{depth}`，等深度小于暂停深度），判定抽成纯函数 `step_reached(mode, depth)`。界面：运行菜单新增「单步跳过（Shift+F10）」「跳出过程（Ctrl+Shift+F10）」，命令面板 `run.step-over` / `run.step-out`，`docs/快捷键.md` 同步。测试：`step_reached_matches_the_requested_depth`（纯函数）+ `step_over_does_not_enter_the_called_procedure` / `step_out_returns_to_the_caller`（真跑脚本、按行号与过程名断言） |
| 调试：出错时中断 | 脚本遇到没有 `On Error` 处理的错误时，原来直接结束、用户看不到当时的变量。现在 `DebugControl` 多一个`break_on_error` 开关（`before_error`）：开着时先把错误当成一次暂停——快照带上 `error` 字段并在出错行停下，监控表达式照常可用；点「继续」后调用方把原始错误抛出去（脚本照常结束），点「停止」换成 `STOPPED_MESSAGE`。解释器侧只在**真正没人处理**时触发（`error_handler()` 是 `None` 或 `Some(Disabled)`，且在最外层块），`On Error Resume Next` / `GoTo` 不受影响。界面：运行菜单「出错时中断」带勾（读 `CodeWindow::break_on_error_enabled`），`toggle_break_on_error` 会同步到正在跑的会话；命令面板 `run.break-on-error`；暂停时输出面板写「调试中断：第 N 行 …」。测试：`break_on_error_pauses_at_the_failing_line`（停在出错行、快照带错误信息、变量可查、继续后仍报错）与`errors_do_not_pause_without_break_on_error`（开关关着时行为不变） |
| 调试：暂停时改变量值 | 原来暂停只能看变量、求值监视表达式，改不了值。`DebugControl` 的请求体从「一条表达式」扩成`RequestKind::{Evaluate, Assign}`，新增 `request_assign(变量名, 新值表达式)`；`DebugQuery` 多一个 `Assign`，解释器侧 `debug_assign` 先求值右侧、再用 `set_var` 写回（**声明类型转换生效**：`Long` 变量收到 `1.5` 变成 2），然后回读一次给出新值与类型。变量名不存在、类型不匹配都返回可读错误。赋值成功后 `pause_and_wait` 会**再取一次快照**并让 `pause_epoch` 加一，界面那套「暂停变化才刷新」的逻辑自动把变量面板、监视结果更新成新值。界面：运行菜单「设置变量值…」（暂停时才可用，光标停在某个名字上会预填 `名字 = `），命令面板 `run.set-variable`，输入框按**第一个** `=` 切分（值里可以再写 `=`）。测试：`assigning_a_variable_while_paused`（改完脚本读到新值 + 快照已刷新）、`assignment_coerces_to_the_declared_type`、`assigning_an_unknown_variable_reports_an_error` |
| 大纲里边看边改变量 | 「设置变量值」原本只有菜单里那个输入框（要自己敲 `名字 = 值`）。现在**双击代码大纲里变量的「值：…」行**就能改：变量名与当前值都已知，弹窗只输新值（预填当前值），回车后走同一条 `request_assign` 通道；该行改成正文色 + 悬停高亮 + 手型光标，明确「可写」；没暂停时这一行不存在，变量保持只读。实现走 `outline::EditVariableAction`（`SectionContext` → `value_row` 的双击事件）→ `OutlinePanel` 转给 `CodeWindow::edit_variable_dialog` / `assign_variable_value`（与菜单入口共用一份赋值逻辑），快捷键一览的「鼠标操作」也补了这条 |
| 调试：按调用栈切帧 | 原来调用栈只出现在状态栏那一行文字里，变量列表永远是最内层。现在 `DebugSnapshot` 多一个 `frame`（`call_stack` 下标，0 是模块），`DebugControl::select_frame` 让界面切层：解释器线程醒来后按这一层重取快照、`pause_epoch` 加一，变量列表 / 编辑器高亮 / 「运行到光标处」就都跟着走。求值与赋值也按层：`DebugQuery` 的 `Snapshot` / `Evaluate` / `Assign` 都带 `frame`，运行时用 `with_debug_frame`（临时 `split_off` 更深的帧、事后再接回去）让那一层成为栈顶，于是「看外层看到了什么、改外层改的是哪一份」都对得上，`Static` 与模块变量不受影响。界面：代码大纲新增「调用栈」一节（**只在暂停时出现**），从模块到最内层一行一层带行号，选中的那层用选中底色，点一下即切层；agent 状态文本里若看的是外层会注明「实际停住的是第 N 层」。3 条新测试（切层后的变量/行号/过程名、外层赋值不影响内层、没暂停或层号越界时拒绝） |
| `Print #` 行尾改 CRLF + 文件语句进黄金 | 补文件类内置函数的 oracle 时发现 **`Print #` 写的是 `\n`，VB6 写 `\r\n`**（本机实测 `Print #1, "hello from script"` 之后 `FileLen` 是 19 而不是 18，`Width` 强制折行同理），会让读回去的字节数、`FileLen`、外部工具看到的换行全都不对——已改成 CRLF，4 条相关单测的期望值同步（`7`/`19`/`\r\n`）。同时新增整脚本黄金用例 `file_statements`（`Print #`/`FileLen`/`GetAttr`/`EOF`/`Line Input #`/`FileCopy`/`Name … As …`/`Dir`/`FileDateTime`/`Kill` 各走一遍，只说相对路径、只打印与目录无关的值），黄金脚本测试改用**临时目录**跑（`RunOptions::script_path`），不再有把测试文件写进仓库根目录的风险。覆盖率报告补 `COVERED_ALIASES`（`Name x As y` 语句即 `rename`），现在**真缺口为零**：剩下的都是单元测试覆盖或`ENVIRONMENT_DEPENDENT` 白名单里的 |
| `FileAttr` / `Input(n, #f)` | 两个文件函数原来没有。现按本机 VB6 实测实现：`FileAttr(n, 1)` 返回打开模式码（Input 1 / Output 2 / Random 4 / Append 8 / Binary 32——`FileWriter` 因此多记一个 `mode`），**先查文件号**（没打开报 52，`returntype` 不是 1 才报 5）；`Input(n, #f)` 按 UTF-8 字符逐个读（`read_char` 按首字节定长度），剩下的字符不够 `n` 个报 **62**，`For Binary` 上不报错、返回空串，`For Output`/`Append` 报 **54**。解析器补了表达式位置的 `Input(...)`（含 `#文件号` 的写法），语句形式的 `Input #` / `Line Input #` 不变。新增黄金脚本 `file_attr_input`（13 行输出含 52/52/54 三个错误号），覆盖率报告再无真缺口 |
| 搜索支持 GBK/UTF-16 源码 | 搜索与跨文件替换原来只认 UTF-8（`String::from_utf8` 失败就计入 skipped），中文 VB6 工程大量是 GBK，等于整片搜不到。`read_text` 改用与打开文件同一套探测（`encoding::decode`）：UTF-8 直接搜，GBK / 带 BOM 的 UTF-16 解码后照常搜；「二进制」判断改成「没有 BOM 且前 8 KiB 有 0 字节」，否则 UTF-16 正文里的 0 字节会被误判。**替换写回按每个文件原来的编码**（`read_text_with_encoding` 同时给回编码，`encoding::encode` 写回），不然把 GBK 源码写成 UTF-8 会直接毁文件。新增 4 条测试（GBK 可搜、UTF-16 可搜、GBK 文件替换后仍是 GBK、UTF-8 BOM 文件替换后 BOM 仍在） |
| 类型探测改 `Option` | 原来的 `Value::as_dictionary()` / `as_collection()` / `as_record()` 返回 `Result<_, String>`，失败分支会现场构造错误字符串；13 处分派点（数组元素赋值、字典下标、`With` 成员、`For Each`、`MemberIndex` 等）把它当类型判断用，每次走到失败分支都白分配一次。新增不分配的 `collection_rc` / `dictionary_rc` / `record_rc`（返回 `Option`），13 处全部换掉，`as_collection` 因为没人用直接删掉；只有真要报错的地方继续用 `as_*()`。新增 `tools/bench/procedure_call.vb` 记录调用开销：空 `Sub` 约 630 ns/次、带一个 `ByVal Long` 形参 1.15 µs/次（每加一个形参约 +500 ns），下一步要压这里得把形参名换成 `Rc<str>` |
| 数组元素赋值热路径 | `Stmt::AssignIndex` 原来先调 `current.as_dictionary()` 判类型——它在「不是字典」时会构造一条错误字符串，而数组赋值每次都会走到这条失败分支，于是**每次给数组元素赋值都白分配一次 `String`**。改成先匹配 `Value::Array`（数组赋值走快路径，字典/COM 分支保持不变）。交错 A/B（2M 次 `a(i) = ...`，每边 8 轮取最小值）0.456s vs 0.589s，约 −23%；新增 `tools/bench/array_read.vb` 用于把读/写成本分开看。顺带纠正了性能基线：数组自增的真实耗时与「每轮 4 条语句 × 单语句开销」吻合，旧记录里 VB6 的 0.06s 不可复现、已标注不要当基线 |

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
   - `Sin` / `Cos` / `Tan` / `Atn` 与 `GetAttr` / `SetAttr` 已完成（见上表）。
   - 财务函数（`SLN` `SYD` `DDB` `FV` `PV` `Pmt` `NPer` `IPmt` `PPmt` `NPV` `IRR` `MIRR` `Rate`）已完成（见上表）；
     只剩 `IRR`/`Rate` 的末几位因为 VB6 的迭代细节（x87 长双精度 + 1e-7 收敛判据）无法逐位复刻。
   - 仍未实现（都是「有条件再做」）：`CVErr` / `IsError` 需要给 `Value` 加 `Error` 变体（`VarType` 10），
     QIDE 目前没有「错误值」这个概念，收益小；`FileAttr` 要回查 `Open` 的模式；`Input(n, #f)` 只有 `Input #`
     语句没有函数形式；`ScriptEngine*` 是 VBScript
     运行时信息、`IMEStatus` 是输入法状态，QIDE 都没有对应概念。
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
6. **COM UDT 记录封送**（**已确认做不到，见结论**）：VB6 实测——把标准模块里的 `Type` 传给晚期绑定
   COM 调用（`d.Add "k", udt`）**编译就过不去**：`只允许定义在公共对象模块中的用户定义类型和变量
   相互转换或传递给晚期绑定功能`。也就是说 VB6 侧只有「公共对象模块里的 UDT」才有类型库、才能按
   `VT_RECORD` 封送。QIDE 的 UDT 是脚本里现定义的、没有类型库，`GetRecordInfoFromGuids` /
   `GetRecordInfoFromTypeInfo` 都拿不到 `IRecordInfo`；要真做就得自己实现整个 `IRecordInfo`
   （vtable 十几个槽），收益远小于风险。**结论：不做**，只在封送处给出明确原因
   （`不能把用户定义类型 `X` 作为 COM 参数…`，原先是一句笼统的「不支持」）。
7. **~~`For Each` 改成活枚举器~~（已完成）**：见上表「COM `For Each` 改活枚举器」。仍未对齐的只剩
   `_NewEnum` 都拿不到的对象（`WScript.Shell`）：QIDE 报 438、循环体一次不跑，VB6 是跑一次再在 `Next` 上报 92，
   见「已知遗留」。
8. **~~`HRESULT_FROM_WIN32` 错误号映射~~（已实测：VB6 没有这张表，不需要表）**：
   本机 VB6 逐条对比后确认：`EXCEPINFO.scode` 为 `0x8007xxxx` 时 VB6 **原样**把它当 `Long` 报
   （`Run "不存在的程序"` 是 -2147024894、`Exec "C:\Windows"` 是 -2147024891、UNC 不通是 -2147024843），
   唯一的特例是 `E_INVALIDARG`（`0x80070057`）报 5；服务器自己的 `0x800Axxxx` 取低 16 位。
   已按这个规则改掉「一律取低 16 位」的旧实现（见上表），并用
   `python tools/vb6_probe.py --script` 对照了 30 余个失败调用（WSH / `Scripting.FileSystemObject`），
   `cargo test -- --ignored` 里的 `com_error_numbers_match_vb6` 钉住这些值。
9. **~~COM 事件在 `DoEvents` 内同步分派~~（已完成）**：见上表「`DoEvents` 同步分派事件」。语句边界那一处保留，
   因为 COM 回调里不能重入解释器；纯忙等循环（不写 `DoEvents`）仍能收到事件，这一点与 VB6 有差异，
   见「已知遗留」。
10. **~~`Format` 的自定义数值格式~~（已完成）**：见上表「`Format` 自定义数值格式」，39 条 VB6 黄金用例。
11. **Win32 混合整型/浮点参数超过 4 个（暂不做）**：`ffi.rs` 的 `mixed_dispatch!` 是按「参数个数 + 浮点位置」
    展开的静态签名表（1..4 个参数），5 个以上直接报「暂不支持」。要通用支持得按 Windows x64 ABI 动态摆
    寄存器/栈（手写汇编或运行期生成机器码，还要处理返回值在 XMM0/RAX 的差异），而实际 API 里混合超过
    4 个参数的情况很少，收益远小于风险；脚本侧可以把浮点实参挪到前四个位置或拆成多次调用。
12. **`AddressOf` 回调的数组/UDT 形参（暂不做）**：`callback_arg` 只把标量/`String` 形参还原成脚本值，
    数组与 UDT 直接报「暂不支持」。VB6 写回调的惯例也是用 `ByVal Long` 接指针再 `CopyMemory`，
    脚本里同样可以这么做（`VarPtr`/`CopyMemory` 与 `Declare` 的 UDT 支持都已有），因此不计划直接支持。
13. **脚本源码兼容性缺口（本轮用 `qide-run` 逐条实测，按「会不会让真实 VB6 源码直接跑不起来」排序）**：
   这些都是**语法层完全没有**的东西，不是语义偏差；所以粘贴真实 VB6 代码时最先撞上：
   - `Attribute VB_Name = "..."` 一行（`.bas` / `.cls` / `.frm` 的第一行必有）现在被当成过程调用，
     第 1 行就报**错误号 5**。建议把 `Attribute` 行当声明忽略（`VB_UserMemId` 另见下面「默认成员」）。
   - `Event 名字(ByVal 参数 As 类型)` 声明语句**不存在**（`event` 不在关键字表）：`RaiseEvent` 只能发
     类里 ad-hoc 的事件名，参数靠同类里 `<类名>_<事件名>` 处理过程的同名形参接收。
   - 模块级（脚本级）`Dim WithEvents x As 脚本类` **静默无效**：`connect_events_for_name` 只认
     `Value::ComObject`，脚本类对象走不到 `register_event_sink`，事件一条也不派发、也不报错；
     而 VB6 里模块级 `WithEvents` 是最常见的订阅写法。
   - `Static Sub` / `Static Function`（过程内全部局部变量变 Static）解析报「需要变量名」；
     只有 `Sub` 内的 `Static n As Long` 支持。
   - `DefInt A-Z` / `DefStr S` 等 `DefType` 声明不存在。
   - `LSet` / `RSet` 语句不存在（错误号 5）。
   - 独立定长字符串 `Dim s As String * 8` 解析报错；UDT 成员里的 `String * 4` 是支持的（`Len`/补空格都对）。
   - 命名实参 `x.M y:=1` 解析报错（VBA 与 COM 调用里很常见）。
   - `#If ... Then` / `#End If` 条件编译不存在（`#` 直接语法错）。
   - `Debug.Assert` 不存在（`Debug` 只认 `.Print`）。
   - `Reset` 语句（关闭全部文件）不存在。
   - 脚本类的**默认成员**（`Attribute Item.VB_UserMemId = 0` 之后的 `c(3)`）不支持，COM 对象支持。
   - 小偏差：未初始化动态数组的 `LBound(a)` 报 5，VB6 报 9。
   - `CVErr` / `IsError` / `Erl` 见第 2 条；`Error$(n)` 正常（错误文案已对齐），`Err.Line` 本就不是 VB6 的东西。

**已决定不做**：模块级 `Public` / `Private` 隔离。`#Include` 是文本展开，整个脚本就是一个模块，
与 VB6 单模块内的语义一致（同一模块里的过程互相可见）；需要真正隔离的只有类成员，那部分已实现
（字段默认 `Private`、方法默认 `Public`）。将来做多模块工程时再补。

## 常用命令

```powershell
cargo build
cargo test          # 二进制 crate，不要用 cargo test --lib
cargo clippy --all-targets
cargo test -- --ignored   # 需要本机 COM（ADODB/WSH）与真实对话框

cargo build --release
python tools/bench.py                              # 跑 tools/bench/*.vb，打印各自最小值
python tools/bench.py --old 旧.exe --new 新.exe     # 两个二进制交错 A/B，打印变化百分比
```

性能改动一律用 `tools/bench.py` 的 A/B 模式（同轮交错、各取最小值）验收，不要拿不同时间的单跑比。

CI（`.github/workflows/ci.yml`）在 `windows-latest` 上跑 `cargo build --all-targets`、`cargo test`、
`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`；黄金数据已提交，CI 不需要装 VB6。

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
  再走 `OutdentInline`，与 `Shift+Tab` 同一条路径、撤销记录仍是一条）。同一个回调还补了另一半：
  上一行**行尾带注释**时组件把光标算作「在注释里」、整条缩进规则都不生效，`If x Then ' 说明` 之后回车原来
  不会缩进；现在用 `indent::opens_block_ignoring_comment`（先剥掉注释，剥的时候跳过字符串里的 `'`、
  认 `""` 转义）判断开块，新行没有比上一行更深就补一次 `Indent`。
  仍有一处是按语义做的取舍：一行里先写 `End If` 再写别的语句（`End If: x = 1`）时行首仍是 `End` 开头，
  回车后照常反缩进——该行执行完确实已经出了 `If` 块，与 VB6 一致。
- 资源管理器多选：勾选靠条目左侧圆点进入「选择模式」（`ExplorerSelection`），现在是**单选高亮 + 圆点模式 +
  `Ctrl+点击` 切换 + `Shift+点击` 选范围**并存。`Shift` 的范围从**上一次点过的条目**（普通点击、点圆点、`Ctrl+点击` 都算）
  到这一行，按树当前的可见顺序算（收起的分支不参与），根项与「空文件夹」占位行跳过。
  **拖动多选已做**：选择模式下拖任一选中项会带上整批（`ExplorerDrag::items` + `explorer::move_targets`）。
  **框选已做**（`explorer::BandDrag`，见「坑位备忘」）：树的空白处按住左键拖动即可，扫过的行按最小..最大范围选中；
  限制是没有空白处（树撑满面板）时无法起框，和 Windows 资源管理器一样。
- **语句级表达式比 VB6 宽**：本机 VB6 `/make` 实测，`p.X`（纯字段读取）、`h.Self2()`（带括号的方法调用）、
  `h.Self2().Value`、`With` 块里的 `.Kiddie(2).Value` 单独成句都**编译不过**（「缺少过程/语法错误」），
  QIDE 一律接受并求值后丢掉结果（`Stmt::Eval`）。这属于有意的宽容，不改（改成报错会让现有脚本失效）；
  `With` 里带括号的链式写法（`.A(1).B`）QIDE 也明确报错，与 VB6 一致。
- 仓库根目录只保留 `Cargo.toml` / `Cargo.lock` / `.gitignore` / `ROADMAP.md` / `app.rc` / `run.rc` / `build.rs` / `assets/`（图标与资源脚本；历史调试日志已于 2026-11-06 清理）。
- COM：`_NewEnum` 不可用的对象（如 `WScript.Shell`）的 `For Each` 已按本机 VB6 对齐（循环体跑一次、
  `Err.Number` 在体内是 438、`Next` 报 92；`Exit For` 也一样），见「已完成」表。
  注意这条只在 `On Error Resume Next` 下复刻：其它错误处理模式下 QIDE 直接把 438 抛给处理过程。
- **省略实参**：`F a, , c` 已支持（含 `IsMissing` 语义，见「已完成」表）。QIDE 比 VB6 宽：末尾省略
  `F(1, )` 与无括号形式 `S 1,` 也接受，VB6 这两个位置是语法错误。
- **按调用栈切层的边界**：切到外层时变量列表给的是「模块变量 + 那一层的局部变量」，**`Static` 变量任何一层都不显示**
  （`Runtime::statics` 没进快照，所有层都一样，不是切帧引入的）；求值与赋值也以外层为准（更深层的变量看不见是预期行为，
  与 VB6 的「按帧求值」一致）。`select_frame` 只在暂停期间、且层号不超过当前深度时生效。
- COM 事件的**时机**：QIDE 在「下一条语句」开头分派排队的事件，**并且** `DoEvents` 会当场把排队事件执行完
  （与 VB6 的消息循环一致，见待办第 9 条）。仍有一处差异：不写 `DoEvents` 的纯忙等循环也能收到事件
  （VB6 收不到，因为事件只能从消息循环里进来）——这是解释器没有自己的消息泵导致的，
  保留语句边界分派是为了让 `rs.Open` 这类调用返回后立刻能看到事件结果（ADODB 的 `WillConnect` 就是这种）。
  事件处理过程里再触发的 COM 事件要等下一次语句边界（`pumping_events` 防递归）。
- COM 事件**没有 VB6 oracle**：本机 VBP 的 `Reference=*\G{...}` 只有 scrrun 生效，ADODB（`msado15.dll` /
  `msado25.tlb`）被静默忽略（`Private WithEvents rs As ADODB.Recordset` 报「变量未定义」），MSXML6 报
  「不能加载 DLL」。所以 `WithEvents` 的语义是按 MS 文档 + ADODB 实测实现的（`connection_WillConnect` 等），
  没有逐条 VB6 对照数据；以后换机器（能挂上 ADODB 引用）可以补。
- COM 事件的**已知小差异**：同一个事件在多个连接点上都存在时只挂第一个（ADO 如此，否则处理过程被重复调用）；
  VB6 的 COM 连接点复用细节未逐一核对。
