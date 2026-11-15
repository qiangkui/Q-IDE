"""用本机 VB6 真编译生成解释器的黄金测试数据。

用法（在仓库根目录）：
    python tools/vb6_golden.py

做三件事：
1. 读 `tools/vb6_golden_cases.txt` 里的表达式（每行一个）；
2. 生成一个最小 VB6 工程并 `Vb6.exe /make` 编成 p-code（`CompilationType=-1`，即 IDE 调试形态），
   跑起来把 `CStr(表达式)` 写进文本文件；
3. 把「表达式 → 期望输出」写成 `src/vb6_golden_data.rs`，供 `src/vb6_golden_tests.rs` 断言。

某个用例在 VB6 里报错（那一行会缺失）时脚本会失败并列出缺失的用例——这类用例不要放进清单。
改动清单后重跑即可；`VB6` 常量按本机安装路径调整。
"""

import os
import subprocess
import sys

VB6 = r"C:\Users\89875\Desktop\VB 6.0\Vb6.exe"

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CASES_FILE = os.path.join(HERE, "vb6_golden_cases.txt")
ERRORS_FILE = os.path.join(HERE, "vb6_golden_errors.txt")
SCRIPTS_FILE = os.path.join(HERE, "vb6_golden_scripts.txt")
SCRIPTS_BUILD_DIR = os.path.join(ROOT, "target", "vb6-golden-scripts")

# 脚本用例共用的声明与辅助过程：VB6 与 QIDE 两侧用同一份，避免两边语义不对等。
SCRIPT_SUPPORT = """\
Private Type Pt
    X As Long
    Y As Long
End Type

Function Double2(ByVal n As Long) As Long
    Double2 = n * 2
End Function

Function Triple(ByVal n As Long) As Long
    Triple = n * 3
End Function

Function Fact(ByVal n As Long) As Long
    If n <= 1 Then
        Fact = 1
    Else
        Fact = n * Fact(n - 1)
    End If
End Function

Sub Bump(ByRef n As Long)
    n = n + 1
End Sub

Function Greet(ByVal name As String, Optional ByVal suffix As String = "") As String
    Greet = "Hi " & name & suffix
End Function

Function Counter() As Long
    Static calls As Long
    calls = calls + 1
    Counter = calls
End Function

Function Pick(ByVal n As Long) As String
    If n = 0 Then
        Pick = "zero"
        Exit Function
    End If
    Pick = "other"
End Function

Function SumTo(ByVal n As Long) As Long
    Static depth As Long
    depth = depth + 1
    If n <= 0 Then
        SumTo = depth
    Else
        SumTo = n + SumTo(n - 1)
    End If
End Function

Sub Twice(ByVal value As Long, ByRef doubled As Long)
    doubled = value * 2
End Sub

Private Sub MovePt(ByRef p As Pt, ByVal dx As Long)
    p.X = p.X + dx
End Sub

Function SumAll(ParamArray xs() As Variant) As Long
    Dim i As Long, total As Long
    For i = LBound(xs) To UBound(xs)
        total = total + xs(i)
    Next
    SumAll = total
End Function

Function StaticArr(ByVal i As Long) As Long
    Static buf(0 To 2) As Long
    buf(i) = buf(i) + 1
    StaticArr = buf(i)
End Function
"""

OUTPUT_FILE = os.path.join(ROOT, "src", "vb6_golden_data.rs")
BUILD_DIR = os.path.join(ROOT, "target", "vb6-golden")

VBP = """Type=Exe
Form=golden.frm
IconForm="Form1"
Startup="Form1"
Title="golden"
ExeName32="golden.exe"
Name="golden"
CompatibleMode="0"
MajorVer=1
MinorVer=0
RevisionVer=0
AutoIncrementVer=0
ServerSupportFiles=0
CompilationType=-1
OptimizationType=0
FavorPentiumPro(tm)=0
CodeViewDebugInfo=0
NoAliasing=0
BoundsCheck=0
OverflowCheck=0
FlPointCheck=0
FDIVCheck=0
UnroundedFP=0
StartMode=0
Unattended=0
Retained=0
ThreadPerObject=0
MaxNumberOfThreads=1
"""

FRM_HEAD = """VERSION 5.00
Begin VB.Form Form1 
   Caption         =   "Form1"
   ClientHeight    =   3015
   ClientLeft      =   120
   ClientTop       =   465
   ClientWidth     =   4560
   LinkTopic       =   "Form1"
   ScaleHeight     =   3015
   ScaleWidth      =   4560
   StartUpPosition =   3
End
Attribute VB_Name = "Form1"
Attribute VB_GlobalNameSpace = False
Attribute VB_Creatable = False
Attribute VB_PredeclaredId = True
Attribute VB_Exposed = False
Option Explicit

Private Sub Form_Load()
    Dim f As Integer
    Dim x, i As Integer, b As Byte, l As Long, d As Date
    Dim arr(2) As Long
    Dim empty_arr() As Long
    Dim c As Collection
    f = FreeFile
    Open "golden.txt" For Output As #f
    On Error Resume Next
"""

FRM_TAIL = """    Close #f
    End
End Sub
"""


def load_cases(path):
    with open(path, encoding="utf-8") as handle:
        lines = [line.strip() for line in handle]
    return [line for line in lines if line and not line.startswith("#")]


def load_scripts(path):
    """按 `=== 名字` 分段；`@` 开头的行是该用例的模块级选项（`Option Base 1` 等）。

    返回 [(名字, 选项元组, 语句块正文), ...]。
    """
    cases = []
    name = None
    options = []
    body = []
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            stripped = line.rstrip("\r\n")
            if stripped.startswith("#"):
                continue
            if stripped.startswith("==="):
                if name is not None:
                    cases.append((name, tuple(options), "\n".join(body).strip("\n")))
                name = stripped[3:].strip()
                options = []
                body = []
            elif name is not None:
                if stripped.startswith("@"):
                    options.append(stripped[1:].strip())
                else:
                    body.append(stripped)
    if name is not None:
        cases.append((name, tuple(options), "\n".join(body).strip("\n")))
    return cases


def compile_project(vbp):
    """用 `Vb6.exe /make` 编译，返回 VB6 的构建日志文本（编译失败时要靠它定位）。"""
    directory = os.path.dirname(vbp)
    log = os.path.join(directory, "build.log")
    if os.path.exists(log):
        os.remove(log)
    result = subprocess.run(
        [VB6, "/make", vbp, "/out", log],
        cwd=directory,
        capture_output=True,
        timeout=300,
    )
    text = (result.stdout + result.stderr).decode("mbcs", "replace")
    if os.path.exists(log):
        with open(log, encoding="mbcs", errors="replace") as handle:
            text += handle.read()
    return text


def build_script_group(index, options, cases):
    """把一组「模块级选项相同」的用例编成一个 VB6 工程，输出写文件。"""
    directory = os.path.join(SCRIPTS_BUILD_DIR, f"g{index}")
    os.makedirs(directory, exist_ok=True)
    project = f"scripts{index}"
    exe = os.path.join(directory, project + ".exe")
    if os.path.exists(exe):
        os.remove(exe)
    calls = "".join(
        f'    Print #f, "#{name}"\n    Case{position} f\n'
        for position, (name, _) in enumerate(cases)
    )
    subs = []
    for position, (_, body) in enumerate(cases):
        # `Debug.Print` → 写文件；其余语句原样保留。
        converted = body.replace("Debug.Print", "Print #f,")
        subs.append(f"\nPrivate Sub Case{position}(f As Integer)\n{converted}\nEnd Sub\n")
    head, _, _ = FRM_HEAD.partition("Private Sub Form_Load()")
    # 选项必须写在模块最前面；`Type` 等声明也必须放在过程之前，否则 VB6 拒绝编译。
    head = head.replace("Option Explicit\n", "Option Explicit\n" + "".join(f"{o}\n" for o in options))
    form = (
        head
        + SCRIPT_SUPPORT
        + "\nPrivate Sub Form_Load()\n"
        + "    Dim f As Integer\n"
        + "    f = FreeFile\n"
        + f'    Open "{project}.txt" For Output As #f\n'
        + calls
        + "    Close #f\n    End\nEnd Sub\n"
        + "".join(subs)
    )
    with open(os.path.join(directory, project + ".vbp"), "w", encoding="mbcs") as handle:
        handle.write(VBP.replace("golden", project))
    with open(os.path.join(directory, project + ".frm"), "w", encoding="mbcs") as handle:
        handle.write(form)
    result = compile_project(os.path.join(directory, project + ".vbp"))
    if not os.path.exists(exe):
        print(result)
        sys.exit(f"VB6 编译脚本用例失败（{project}）")
    return directory, project, exe


def collect_script_group(directory, project, exe, names):
    output = os.path.join(directory, project + ".txt")
    if os.path.exists(output):
        os.remove(output)
    subprocess.run([exe], cwd=directory, timeout=300, check=False)
    sections = {}
    current = None
    with open(output, encoding="mbcs") as handle:
        for line in handle:
            text = line.rstrip("\r\n")
            if text.startswith("#"):
                current = text[1:]
                sections[current] = []
            elif current is not None:
                sections[current].append(text.rstrip())
    missing = [name for name in names if name not in sections]
    if missing:
        sys.exit("这些脚本用例在 VB6 里没有输出（不要放进清单）：\n  " + "\n  ".join(missing))
    # `Print #` 会在每段末尾留下空行，统一去掉。
    return {name: "\n".join(lines).rstrip("\n") for name, lines in sections.items()}


def build_scripts(cases):
    """按模块级选项分组编译，返回 {用例名: 期望输出}。"""
    os.makedirs(SCRIPTS_BUILD_DIR, exist_ok=True)
    groups = {}
    for name, options, body in cases:
        groups.setdefault(options, []).append((name, body))
    outputs = {}
    for index, (options, members) in enumerate(sorted(groups.items())):
        directory, project, exe = build_script_group(index, options, members)
        outputs.update(collect_script_group(directory, project, exe, [n for n, _ in members]))
    return outputs


def build(cases, errors):
    os.makedirs(BUILD_DIR, exist_ok=True)
    exe = os.path.join(BUILD_DIR, "golden.exe")
    # 上一次编译留下的 exe 会让「编译失败」看起来像「用例求值失败」，先删掉。
    if os.path.exists(exe):
        os.remove(exe)
    body = "".join(
        f'    Print #f, "{index}" & vbTab & CStr({expression})\n'
        for index, expression in enumerate(cases)
    )
    # 错误号用例：每条语句前后清 `Err`，失败后把 `Err.Number` 写出来。
    error_body = "".join(
        f"    Err.Clear\n    {statement}\n"
        f'    Print #f, "{len(cases) + index}" & vbTab & CStr(Err.Number)\n'
        for index, statement in enumerate(errors)
    )
    # `.frm` 按 Windows ANSI 代码页（中文系统是 936）保存，VB6 才能正确读入字符串里的中文。
    with open(os.path.join(BUILD_DIR, "golden.vbp"), "w", encoding="mbcs") as handle:
        handle.write(VBP)
    with open(os.path.join(BUILD_DIR, "golden.frm"), "w", encoding="mbcs") as handle:
        handle.write(FRM_HEAD + body + error_body + FRM_TAIL)
    result = compile_project(os.path.join(BUILD_DIR, "golden.vbp"))
    if not os.path.exists(exe):
        print(result)
        sys.exit("VB6 编译失败")
    return exe


def collect(exe):
    output = os.path.join(BUILD_DIR, "golden.txt")
    if os.path.exists(output):
        os.remove(output)
    subprocess.run([exe], cwd=BUILD_DIR, timeout=300, check=False)
    values = {}
    with open(output, encoding="mbcs") as handle:
        for line in handle:
            index, _, value = line.rstrip("\r\n").partition("\t")
            values[int(index)] = value
    return values


def emit(cases, errors, values):
    missing = [cases[i] for i in range(len(cases)) if i not in values]
    if missing:
        sys.exit("这些表达式在 VB6 里求值失败（不要放进清单）：\n  " + "\n  ".join(missing))
    unexpected = [
        errors[i]
        for i in range(len(errors))
        if values.get(len(cases) + i) == "0"
    ]
    if unexpected:
        sys.exit("这些语句在 VB6 里没有报错（不要放进清单）：\n  " + "\n  ".join(unexpected))
    lines = [
        "//! 由 `tools/vb6_golden.py` 生成，请勿手工修改。",
        "//!",
        "//! 期望值来自本机 VB6 真编译（`CompilationType=-1`，IDE 调试用的 p-code 形态）的实际输出。",
        "",
        "/// 表达式 → `CStr(表达式)` 的结果。",
        "pub const CASES: &[(&str, &str)] = &[",
    ]
    for index, expression in enumerate(cases):
        expected = values[index].replace("\\", "\\\\").replace('"', '\\"')
        expression = expression.replace("\\", "\\\\").replace('"', '\\"')
        lines.append(f'    ("{expression}", "{expected}"),')
    lines.append("];")
    lines.append("")
    lines.append("/// 会失败的语句 → `Err.Number`。")
    lines.append("pub const ERROR_CASES: &[(&str, i64)] = &[")
    for index, statement in enumerate(errors):
        number = values[len(cases) + index]
        escaped = statement.replace("\\", "\\\\").replace('"', '\\"')
        lines.append(f'    ("{escaped}", {number}),')
    lines.append("];")
    lines.append("")
    return lines


def emit_scripts(lines, scripts, outputs):
    """`scripts` 是 [(名字, 选项元组, 语句块), ...]。"""
    lines.append("/// 脚本用例共用的声明与辅助过程（与 VB6 侧同一份）。")
    lines.append("pub const SCRIPT_SUPPORT: &str = r#\"")
    lines.append(SCRIPT_SUPPORT.rstrip("\n"))
    lines.append("\"#;")
    lines.append("")
    lines.append(
        "/// (用例名, 模块级选项（`Option Base 1` 等，空串表示没有）, 语句块,"
        " `Debug.Print` 的期望输出（行之间用 `\\n` 分隔））。"
    )
    lines.append("pub const SCRIPT_CASES: &[(&str, &str, &str, &str)] = &[")
    for name, options, body in scripts:
        escaped_name = name.replace("\\", "\\\\").replace('"', '\\"')
        # 选项之间用换行分隔，测试侧直接接在源文件开头。
        escaped_options = (
            "\n".join(options).replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
        )
        escaped_source = body.replace("\\", "\\\\").replace('"', '\\"')
        escaped_output = outputs[name].replace("\\", "\\\\").replace('"', '\\"')
        escaped_output = escaped_output.replace("\n", "\\n")
        lines.append(
            f'    ("{escaped_name}", "{escaped_options}", "{escaped_source}", "{escaped_output}"),'
        )
    lines.append("];")
    lines.append("")


def main():
    cases = load_cases(CASES_FILE)
    errors = load_cases(ERRORS_FILE)
    if not cases:
        sys.exit("用例清单是空的")
    exe = build(cases, errors)
    values = collect(exe)
    lines = emit(cases, errors, values)

    scripts = load_scripts(SCRIPTS_FILE)
    if scripts:
        outputs = build_scripts(scripts)
        emit_scripts(lines, scripts, outputs)

    with open(OUTPUT_FILE, "w", encoding="utf-8", newline="\n") as handle:
        handle.write("\n".join(lines))
    print(
        f"已生成 {OUTPUT_FILE}（{len(cases)} 条表达式 + {len(errors)} 条错误号"
        f" + {len(scripts)} 段脚本）。"
    )


if __name__ == "__main__":
    main()
