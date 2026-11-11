# 当前文件：VB6 / VBA 风格脚本（Q-IDE 解释器）

- 没有 `Module` / `End Module`、`Console`、`List(Of T)`、`Try/Catch`、`String.Join` 等 .NET 专有语法。
- 输出用 `Debug.Print` 或 `MsgBox`；数组写成 `Dim a() As Integer` 或 `Dim a As Integer() = {1, 2, 3}`。
- `Option Explicit` 下必须先声明变量；模块级变量可以直接写 `Private x As Long`。
- 公共结构体 / 常量 / `Declare` 可以拆到单独文件，用 `#Include "文件.vb"` 引入（这是 Q-IDE 扩展，不是标准 VB）。
- 调用 Win32 API 时，x64 的句柄 / 指针用 `LongPtr`（`Long` 只有 4 字节）；字符串参数按 UTF-16 传递，优先用 W 版 API。
- `AddressOf` 回调最多 4 个参数；写进 UDT 字段（如 `WNDCLASSW.lpfnWndProc`）的回调由解释器保管到脚本结束。
- 文件读写用 `Open` / `Print #` / `Line Input #` 等 VB 语句，不要用 `File.ReadAllText`。
