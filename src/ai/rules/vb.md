# 当前文件：VB6 / VBA 风格脚本（Q-IDE 解释器）

## 语言范围

- 目标是 **VB6 语法子集 + Win64 / VBA 扩展**，不是完整 VB6/VBA，也没有 .NET 语法：不要写 `Module` / `End Module`、`Console`、`List(Of T)`、`Try/Catch`、`String.Join` 等。
- 大小写不敏感；`'` 到行尾是注释；`:` 分隔语句；行尾 `_` 续行。
- 新脚本先写 `Option Explicit`，变量与过程先声明；模块级可直接写 `Private x As Long`。
- 输出用 `Debug.Print` 或 `MsgBox`；文件读写用 `Open` / `Print #` / `Line Input #` / `Get #` / `Put #`，不要用 `File.ReadAllText` 等 .NET API。
- 数组写成 `Dim a() As Integer` 或 `Dim a As Integer() = {1, 2, 3}`；动态数组用 `ReDim` / `ReDim Preserve`（只能改最后一维）。

## 组织与复用

- 公共结构体、常量、`Declare` 拆到单独文件，用 `#Include "文件.vb"` 引入（Q-IDE 扩展，不是标准 VB）。
- 优先复用 exe 同级 `assets\Win32常用定义.vb`：已含常用窗口/GDI 常量、`RECT` / `MSG` / `WNDCLASSW` 等结构体与 user32/gdi32 声明；缺少的再按标准写法补。
- `#Include` 相对路径先按脚本目录解析，再查 include_dir / exe 同级 `assets/`；同一文件只展开一次，循环引用会报错。
- 过程参数默认 `ByRef`，需要按值传时写 `ByVal`；支持 `Optional` 默认值与 `ParamArray`；函数名赋值即返回值。
- 错误处理用 `On Error Resume Next` 或 `On Error GoTo 标签`，配合 `Err.Number` / `Err.Description`；处理完用 `Err.Clear`，结束时 `On Error GoTo 0`。

## Win32 / 回调

- x64 下句柄与指针必须用 `LongPtr`（`Long` 只有 4 字节，放句柄/函数指针会被截断）。
- 字符串参数按 UTF-16 传递，统一调用 W 版 API；需要给 `LongPtr` 参数传字符串时用 `StrPtr(s)`（暂不支持 `VarPtr`）。
- `Type` 必须定义在引用它的 `Declare` 之前；结构体参数先 `Dim` 一个变量再传，不能像 C 那样传临时值。
- 结构体字段支持整型/浮点、`String * n`、定长数组与嵌套 UDT；不支持变长 `String` 字段。
- `AddressOf` 回调最多 64 个受支持的形参，必须在脚本线程触发；直接写在 `Declare` 实参里的跳板只在本次调用有效，存进 UDT 字段（如 `WNDCLASSW.lpfnWndProc`）则由解释器保管到脚本结束。
- `SendMessageW` 这类既有数值又有字符串参数的 API，要按参数类型分别写两个 `Declare`（其中一个用 `Alias`），不要用 `As Any` 顶替。
- 不支持：数组作为回调形参、COM 事件接收、多维/强类型 `SAFEARRAY`、UDT 封送、`Variant` 与变长 `String` 的直接 `Get`/`Put`。

## 常见坑

- `True` 参与数值运算是 `-1`；`If` / `While` / `Do Until` 的条件只接受 `True` / `False` 或数字文本，不要拿任意非空字符串当条件。
- `CInt` / `CLng` 用银行家舍入并检查范围，越界报错误号 6；`Str(非负数)` 会带一个前导空格。
- `\` 是整除，`&` 拼字符串；`/` 是浮点除法。
- `Dim` 在过程内只声明一次：写在循环里不会每轮重置变量，初值表达式才会每轮重算。
- 过程递归默认上限 200 层；`#Include` 读取磁盘文件，改完被包含文件要先保存再运行。

## 验证与调试

- 改完脚本先用 `check_syntax`；只有用户明确要求时才 `run_script` / `start_debug`。
- 调试可配合 `set_breakpoint`（支持条件）、`debug_step` / `debug_continue`、`get_debug_state`、`add_watch`；结束后用 `read_output` 看输出与错误行号。
- 脚本可能产生文件、弹窗等副作用，运行前先说明会做什么。