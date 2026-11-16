"""用本机 VB6 真编译生成解释器的黄金测试数据。

用法（在仓库根目录）：

    python tools/vb6_golden.py                 # 全套重生成（表达式 + 错误号 + 近似用例 + 整脚本）
    python tools/vb6_golden.py --check         # 只校验 src/vb6_golden_data.rs 与清单是否同步（不编译）
    python tools/vb6_golden.py --expr-only     # 只重生成表达式 / 错误号 / 近似用例，脚本沿用现有数据
    python tools/vb6_golden.py --scripts-only --only 名字前缀   # 只重生成匹配的脚本用例
    python tools/vb6_golden.py --bisect        # 编译失败时二分定位到具体用例（每条用例多编一次）

做三件事：
1. 读四份清单：`vb6_golden_cases.txt`（表达式）、`vb6_golden_errors.txt`（失败语句）、
   `vb6_golden_approx.txt`（浮点末位不稳的表达式，带容差）、`vb6_golden_scripts.txt`（整段脚本）；
2. 生成最小 VB6 工程并 `Vb6.exe /make` 编成 p-code（`CompilationType=-1`，即 IDE 调试形态），
   跑起来把 `CStr(表达式)` / `Err.Number` / `Debug.Print` 的输出写进文本文件；
3. 把「用例 → 期望输出」写成 `src/vb6_golden_data.rs`，供 `src/vb6_golden_tests.rs` 断言。

某个用例在 VB6 里报错（那一行会缺失）时脚本会失败并列出缺失的用例——这类用例不要放进清单。
编译失败时会顺手把 VB6 报的行号翻回具体的用例名（VB6 自己的行号经常指向过程头，只能尽力而为）。
VB6 路径用环境变量 `QIDE_VB6` 覆盖（不设时用 `VB6` 常量里的默认路径）。
"""

import os
import re
import subprocess
import sys

VB6 = os.environ.get("QIDE_VB6") or r"E:\vb\VB 6.0\Vb6.exe"

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CASES_FILE = os.path.join(HERE, "vb6_golden_cases.txt")
ERRORS_FILE = os.path.join(HERE, "vb6_golden_errors.txt")
APPROX_FILE = os.path.join(HERE, "vb6_golden_approx.txt")
SCRIPTS_FILE = os.path.join(HERE, "vb6_golden_scripts.txt")
SCRIPTS_BUILD_DIR = os.path.join(ROOT, "target", "vb6-golden-scripts")

DEFAULT_TOLERANCE = 1e-9

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

Function QideMissing(Optional ByVal a As Variant, Optional ByVal b As Variant) As String
    QideMissing = CStr(IsMissing(a)) & "/" & CStr(IsMissing(b))
End Function

Function QideMiddle(ByVal a As Long, Optional ByVal b As Variant, Optional ByVal c As Variant) As String
    QideMiddle = CStr(a) & "/" & CStr(IsMissing(b)) & "/" & CStr(IsMissing(c))
End Function
"""

OUTPUT_FILE = os.path.join(ROOT, "src", "vb6_golden_data.rs")
BUILD_DIR = os.path.join(ROOT, "target", "vb6-golden")

VBP = """Type=Exe
Reference=*\\G{420B2830-E718-11CF-893D-00A0C9054228}#1.0#0#C:\\Windows\\SysWOW64\\scrrun.dll#Microsoft Scripting Runtime
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

' 用例里用到的变量放在模块级：表达式用例会被切成多个 `PartN` 过程（VB6 单个过程不能超过 64KB，
' 全都塞进 `Form_Load` 会报「过程太大」），各过程共用同一份变量。
Private x, i As Integer, b As Byte, l As Long, d As Date
Private arr(2) As Long
Private empty_arr() As Long
Private c As Collection

Private Sub Form_Load()
    Dim f As Integer
    f = FreeFile
    Open "golden.txt" For Output As #f
"""

FRM_TAIL = """    Close #f
    End
End Sub
"""


def ansi_encoding():
    """VB6 只认 ANSI 代码页：固定用 936，缺了才退回当前代码页。"""
    for name in ("cp936", "mbcs"):
        try:
            "中文".encode(name)
            return name
        except (LookupError, UnicodeEncodeError):
            continue
    return "latin-1"


ENC = ansi_encoding()

_ESCAPES = {"n": "\n", "r": "\r", "t": "\t", "\\": "\\", '"': '"'}
_RUST_STR = r'"((?:[^"\\]|\\.)*)"'


def unescape(text):
    return re.sub(r"\\(.)", lambda match: _ESCAPES.get(match.group(1), match.group(1)), text)


def escape(text):
    return text.replace("\\", "\\\\").replace('"', '\\"')


def escape_lines(text):
    """Rust 字符串字面量里的多行文本：先转义引号/反斜杠，再把换行写成 `\\n`。"""
    return escape(text).replace("\n", "\\n")


def load_cases(path):
    with open(path, encoding="utf-8") as handle:
        lines = [line.strip() for line in handle]
    return [line for line in lines if line and not line.startswith("#")]


def load_approx(path):
    """`表达式 | 容差` 每行一条；容差省略时用 1e-9。"""
    if not os.path.exists(path):
        return []
    entries = []
    for line in load_cases(path):
        expression, _, tolerance = line.partition("|")
        expression = expression.strip()
        tolerance = tolerance.strip()
        entries.append((expression, float(tolerance) if tolerance else DEFAULT_TOLERANCE))
    return entries


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


def convert_debug_print(body):
    """把代码里的 `Debug.Print` 换成 `Print #f,`（跳过字符串字面量与注释）。"""
    out = []
    index = 0
    length = len(body)
    while index < length:
        ch = body[index]
        if ch == '"':
            end = index + 1
            while end < length:
                if body[end] == '"':
                    if end + 1 < length and body[end + 1] == '"':
                        end += 2
                        continue
                    end += 1
                    break
                end += 1
            out.append(body[index:end])
            index = end
        elif ch == "'":
            end = body.find("\n", index)
            end = length if end < 0 else end
            out.append(body[index:end])
            index = end
        elif body.startswith("Debug.Print", index) and (
            index == 0 or not (body[index - 1].isalnum() or body[index - 1] == "_")
        ):
            out.append("Print #f,")
            index += len("Debug.Print")
        else:
            out.append(ch)
            index += 1
    return "".join(out)


def span_lookup(spans, line):
    """把 VB6 报的行号翻成用例标签；落在用例之间时给最近的用例。"""
    for start, end, label in spans:
        if start <= line <= end:
            return label
    for start, _, label in spans:
        if start >= line:
            return label
    return spans[-1][2] if spans else ""


def annotate_compile_error(text, spans):
    """在编译日志后面补一行「这个行号对应哪个用例」。"""
    match = re.search(r"行\s*(\d+)", text)
    if not match or not spans:
        return text
    line = int(match.group(1))
    if line < spans[0][0]:
        note = "（VB6 报的行号落在模块头部，映射不到具体用例；可加 `--bisect` 自动定位）"
    else:
        note = f"（VB6 报的第 {line} 行对应用例：{span_lookup(spans, line)}）"
    return text + "\n" + note + "\n"


def compile_project(vbp, exe):
    """用 `Vb6.exe /make` 编译，返回 (日志, 是否成功)。

    VB6 的 `/make` **第一次调用经常直接返回**（既不编译也不写日志），
    所以按「exe 是否生成」判定并自动再试一次；日志里已明确报编译错误时不再重试。
    """
    directory = os.path.dirname(vbp)
    log = os.path.join(directory, "build.log")
    text = ""
    for _ in range(2):
        for path in (log, exe):
            if os.path.exists(path):
                os.remove(path)
        result = subprocess.run(
            [VB6, "/make", vbp, "/out", log],
            cwd=directory,
            capture_output=True,
            timeout=300,
        )
        text = (result.stdout + result.stderr).decode(ENC, "replace")
        if os.path.exists(log):
            with open(log, encoding=ENC, errors="replace") as handle:
                text += handle.read()
        if os.path.exists(exe):
            return text, True
        if "编译错误" in text or "compilation error" in text.lower():
            break
    return text, False


def build_script_group(index, options, cases):
    """把一组「模块级选项相同」的用例编成一个 VB6 工程，输出写文件。"""
    directory = os.path.join(SCRIPTS_BUILD_DIR, f"g{index}")
    os.makedirs(directory, exist_ok=True)
    project = f"scripts{index}"
    exe = os.path.join(directory, project + ".exe")
    calls = "".join(
        f'    Print #f, "#{name}"\n    Case{position} f\n'
        for position, (name, _) in enumerate(cases)
    )
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
    )
    spans = []
    line = form.count("\n")
    for position, (name, body) in enumerate(cases):
        # `Debug.Print` → 写文件；其余语句原样保留。
        chunk = (
            f"\nPrivate Sub Case{position}(f As Integer)\n"
            f"{convert_debug_print(body)}\nEnd Sub\n"
        )
        spans.append((line + 1, line + chunk.count("\n"), name))
        form += chunk
        line += chunk.count("\n")
    with open(os.path.join(directory, project + ".vbp"), "w", encoding=ENC) as handle:
        handle.write(VBP.replace("golden", project))
    with open(os.path.join(directory, project + ".frm"), "w", encoding=ENC) as handle:
        handle.write(form)
    text, ok = compile_project(os.path.join(directory, project + ".vbp"), exe)
    if not ok:
        print(annotate_compile_error(text, spans))
        sys.exit(f"VB6 编译脚本用例失败（{project}）")
    return directory, project, exe


def collect_script_group(directory, project, exe, names):
    output = os.path.join(directory, project + ".txt")
    if os.path.exists(output):
        os.remove(output)
    subprocess.run([exe], cwd=directory, timeout=300, check=False)
    sections = {}
    current = None
    with open(output, encoding=ENC) as handle:
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


# VB6 单个过程上限 64KB（p-code 下大约几千行）。用例总量已经过千，全部塞进 `Form_Load`
# 会报「过程太大」，所以按行数切成若干 `PartN` 过程，`Form_Load` 只负责打开输出文件并逐个调用。
CHUNK_LINES = 300


def expression_body(cases, errors, approx=()):
    """拼出表达式 / 错误号 / 近似用例的正文，同时返回「行号 → 用例」映射。

    返回 `(调用行, 各 Part 的正文, spans)`：调用行插在 `Form_Load` 里，各 Part 追加在文件末尾。
    """
    items = []
    for index, expression in enumerate(cases):
        items.append(([f'Print #f, "{index}" & vbTab & CStr({expression})'], 1, expression))
    # 错误号用例：每条语句前后清 `Err`，失败后把 `Err.Number` 写出来。
    for index, statement in enumerate(errors):
        items.append(
            (
                [
                    "Err.Clear",
                    statement,
                    f'Print #f, "{len(cases) + index}" & vbTab & CStr(Err.Number)',
                ],
                2,
                statement,
            )
        )
    # 近似用例：同样只取 `CStr` 文本，容差比对在 Rust 侧做。
    offset = len(cases) + len(errors)
    for index, (expression, _) in enumerate(approx):
        items.append(([f'Print #f, "{offset + index}" & vbTab & CStr({expression})'], 1, expression))

    chunks = []
    current, current_lines = [], 0
    for item in items:
        if current and current_lines + len(item[0]) > CHUNK_LINES:
            chunks.append(current)
            current, current_lines = [], 0
        current.append(item)
        current_lines += len(item[0])
    if current:
        chunks.append(current)

    calls = "".join(f"    Part{index} f\n" for index in range(len(chunks)))
    parts = ""
    spans = []
    line = FRM_HEAD.count("\n") + len(chunks) + FRM_TAIL.count("\n")
    for index, chunk in enumerate(chunks):
        # `On Error Resume Next` 是**按过程**生效的，每个 Part 都要自己写一句。
        parts += f"\nPrivate Sub Part{index}(f As Integer)\n    On Error Resume Next\n"
        line += 3
        for lines, span, expression in chunk:
            parts += "".join(f"    {text}\n" for text in lines)
            spans.append((line + 1, line + span, expression))
            line += len(lines)
        parts += "End Sub\n"
        line += 1
    return calls, parts, spans


def compile_form(directory, project, body, spans, parts=""):
    """把一段正文写成工程并编译，失败时打印「行号→用例」提示；返回 (是否成功, 日志)。"""
    os.makedirs(directory, exist_ok=True)
    exe = os.path.join(directory, project + ".exe")
    # `.frm` 按 Windows ANSI 代码页（中文系统是 936）保存，VB6 才能正确读入字符串里的中文。
    with open(os.path.join(directory, project + ".vbp"), "w", encoding=ENC) as handle:
        handle.write(VBP.replace("golden", project))
    with open(os.path.join(directory, project + ".frm"), "w", encoding=ENC) as handle:
        handle.write(FRM_HEAD + body + FRM_TAIL + parts)
    text, ok = compile_project(os.path.join(directory, project + ".vbp"), exe)
    if not ok:
        print(annotate_compile_error(text, spans))
    return ok, text


def build(cases, errors, approx=()):
    """编译表达式 / 错误号 / 近似用例，返回 exe。

    近似用例是可选的（`vb6_probe.py` 只探表达式时不需要），空元组就少写那几行。
    """
    body, parts, spans = expression_body(cases, errors, approx)
    ok, _ = compile_form(BUILD_DIR, "golden", body, spans, parts)
    if not ok:
        sys.exit("VB6 编译失败")
    return os.path.join(BUILD_DIR, "golden.exe")


def bisect_failure(cases, errors, approx):
    """二分定位「哪一条用例让 VB6 编译失败」（VB6 自己报的行号经常落在模块头部）。

    返回出问题的用例文本；定位不到返回 `None`。每次尝试都要真编译一次，只在需要时用。
    """
    directory = os.path.join(BUILD_DIR, "bisect")

    def fails(count):
        subset_cases = cases[: min(count, len(cases))]
        remaining = count - len(subset_cases)
        subset_errors = errors[: max(0, min(remaining, len(errors)))]
        remaining -= len(subset_errors)
        subset_approx = approx[: max(0, remaining)]
        body, parts, spans = expression_body(subset_cases, subset_errors, subset_approx)
        ok, _ = compile_form(directory, "bisect", body, spans, parts)
        return not ok

    labels = list(cases) + list(errors) + [entry[0] for entry in approx]
    total = len(labels)
    if total == 0 or not fails(total):
        return None
    low, high = 0, total  # `low` 个用例能过、`high` 个用例失败
    while high - low > 1:
        middle = (low + high) // 2
        if fails(middle):
            high = middle
        else:
            low = middle
    return labels[high - 1]


def collect(exe):
    output = os.path.join(BUILD_DIR, "golden.txt")
    if os.path.exists(output):
        os.remove(output)
    subprocess.run([exe], cwd=BUILD_DIR, timeout=300, check=False)
    values = {}
    with open(output, encoding=ENC) as handle:
        for line in handle:
            index, _, value = line.rstrip("\r\n").partition("\t")
            values[int(index)] = value
    return values


def collect_values(cases, errors, approx, values):
    """把 VB6 的原始输出整理成三组数据，顺便校验「该报错的用例确实报错了」。"""
    missing = [expression for i, expression in enumerate(cases) if i not in values]
    if missing:
        sys.exit("这些表达式在 VB6 里求值失败（不要放进清单）：\n  " + "\n  ".join(missing))
    unexpected = [errors[i] for i in range(len(errors)) if values.get(len(cases) + i) == "0"]
    if unexpected:
        sys.exit("这些语句在 VB6 里没有报错（不要放进清单）：\n  " + "\n  ".join(unexpected))
    offset = len(cases) + len(errors)
    missing_approx = [approx[i][0] for i in range(len(approx)) if offset + i not in values]
    if missing_approx:
        sys.exit(
            "这些近似用例在 VB6 里求值失败（不要放进清单）：\n  " + "\n  ".join(missing_approx)
        )
    return (
        [(expression, values[i]) for i, expression in enumerate(cases)],
        [(statement, values[len(cases) + i]) for i, statement in enumerate(errors)],
        [
            (expression, values[offset + i], tolerance)
            for i, (expression, tolerance) in enumerate(approx)
        ],
    )


def render(data):
    """把四组数据渲染成 `src/vb6_golden_data.rs` 的内容。"""
    lines = [
        "//! 由 `tools/vb6_golden.py` 生成，请勿手工修改。",
        "//!",
        "//! 期望值来自本机 VB6 真编译（`CompilationType=-1`，IDE 调试用的 p-code 形态）的实际输出。",
        "",
        "/// 表达式 → `CStr(表达式)` 的结果。",
        "#[rustfmt::skip]",
        "pub const CASES: &[(&str, &str)] = &[",
    ]
    for expression, expected in data["cases"]:
        lines.append(f'    ("{escape(expression)}", "{escape(expected)}"),')
    lines.append("];")
    lines.append("")
    lines.append("/// 会失败的语句 → `Err.Number`。")
    lines.append("#[rustfmt::skip]")
    lines.append("pub const ERROR_CASES: &[(&str, i64)] = &[")
    for statement, number in data["errors"]:
        lines.append(f'    ("{escape(statement)}", {number}),')
    lines.append("];")
    lines.append("")
    lines.append("/// 浮点末位不稳的表达式 → (VB6 的 `CStr` 结果, 允许的相对误差)。")
    lines.append("/// 比对规则：`|QIDE - VB6| <= 容差 * max(|VB6|, 1)`。")
    lines.append("#[rustfmt::skip]")
    lines.append("pub const APPROX_CASES: &[(&str, &str, f64)] = &[")
    for expression, expected, tolerance in data["approx"]:
        lines.append(f'    ("{escape(expression)}", "{escape(expected)}", {tolerance!r}),')
    lines.append("];")
    lines.append("")
    lines.append("/// 脚本用例共用的声明与辅助过程（与 VB6 侧同一份）。")
    lines.append("#[rustfmt::skip]")
    lines.append('pub const SCRIPT_SUPPORT: &str = r#"')
    lines.append(SCRIPT_SUPPORT.rstrip("\n"))
    lines.append('"#;')
    lines.append("")
    lines.append(
        "/// (用例名, 模块级选项（`Option Base 1` 等，空串表示没有）, 语句块,"
        " `Debug.Print` 的期望输出（行之间用 `\\n` 分隔））。"
    )
    lines.append("#[rustfmt::skip]")
    lines.append("pub const SCRIPT_CASES: &[(&str, &str, &str, &str)] = &[")
    for name, options, body, expected in data["scripts"]:
        option_text = escape_lines("\n".join(options))
        lines.append(
            f'    ("{escape(name)}", "{option_text}", "{escape(body)}",'
            f' "{escape_lines(expected)}"),'
        )
    lines.append("];")
    lines.append("")
    return lines


def parse_data(path):
    """把已生成的 `src/vb6_golden_data.rs` 解析回四组数据（`--check` 与增量重生成用）。"""
    with open(path, encoding="utf-8") as handle:
        text = handle.read()

    def body(name):
        start = text.find(f"pub const {name}")
        if start < 0:
            return ""
        start = text.find("= &[", start)
        if start < 0:
            return ""
        end = text.find("\n];", start)
        return text[start:end]

    def pairs(source):
        # 末尾的 `,?` 是给 rustfmt 拆行的情形留的：跨行时最后一个元素后面会有逗号。
        pattern = r"\(\s*" + _RUST_STR + r"\s*,\s*" + _RUST_STR + r"\s*,?\s*\)"
        return [(unescape(a), unescape(b)) for a, b in re.findall(pattern, source)]

    support = ""
    support_start = text.find('pub const SCRIPT_SUPPORT: &str = r#"')
    if support_start >= 0:
        start = text.find('r#"', support_start) + 3
        support = text[start:text.find('"#;', start)]

    return {
        "cases": pairs(body("CASES")),
        "errors": [
            (unescape(statement), int(number))
            for statement, number in re.findall(
                r"\(\s*" + _RUST_STR + r"\s*,\s*(-?\d+)\s*,?\s*\)", body("ERROR_CASES")
            )
        ],
        "approx": [
            (unescape(expression), unescape(expected), float(tolerance))
            for expression, expected, tolerance in re.findall(
                r"\(\s*"
                + _RUST_STR
                + r"\s*,\s*"
                + _RUST_STR
                + r"\s*,\s*([0-9eE.+-]+)\s*,?\s*\)",
                body("APPROX_CASES"),
            )
        ],
        "support": support,
        "scripts": [
            (
                unescape(name),
                tuple(unescape(options).split("\n")) if unescape(options) else (),
                unescape(source),
                unescape(expected),
            )
            for name, options, source, expected in re.findall(
                r"\(\s*"
                + _RUST_STR
                + r"\s*,\s*"
                + _RUST_STR
                + r"\s*,\s*"
                + _RUST_STR
                + r"\s*,\s*"
                + _RUST_STR
                + r"\s*,?\s*\)",
                body("SCRIPT_CASES"),
            )
        ],
    }


def check(cases, errors, approx, scripts):
    """校验数据文件与清单是否同步（不跑 VB6）。"""
    if not os.path.exists(OUTPUT_FILE):
        sys.exit(f"缺少 {OUTPUT_FILE}，先跑一次 `python tools/vb6_golden.py`")
    data = parse_data(OUTPUT_FILE)
    problems = []

    if [expression for expression, _ in data["cases"]] != cases:
        problems.append(_diff("表达式", cases, [e for e, _ in data["cases"]]))
    if [statement for statement, _ in data["errors"]] != errors:
        problems.append(_diff("错误号语句", errors, [s for s, _ in data["errors"]]))
    stored_approx = [(expression, tolerance) for expression, _, tolerance in data["approx"]]
    if stored_approx != approx:
        problems.append(
            _diff(
                "近似用例",
                [f"{e} | {t}" for e, t in approx],
                [f"{e} | {t}" for e, t in stored_approx],
            )
        )
    stored_scripts = [
        (name, "\n".join(options), body) for name, options, body, _ in data["scripts"]
    ]
    wanted_scripts = [(name, "\n".join(options), body) for name, options, body in scripts]
    if stored_scripts != wanted_scripts:
        problems.append(
            _diff(
                "脚本用例（名字/选项/正文）",
                [f"{name} [{options}]" for name, options, _ in wanted_scripts],
                [f"{name} [{options}]" for name, options, _ in stored_scripts],
            )
        )
    if data["support"].strip("\n") != SCRIPT_SUPPORT.strip("\n"):
        problems.append("脚本共用声明（`SCRIPT_SUPPORT`）与 `tools/vb6_golden.py` 不一致")

    if problems:
        sys.exit(
            "数据文件与清单不同步：\n"
            + "\n".join(problems)
            + "\n跑一次 `python tools/vb6_golden.py` 重新生成。"
        )
    print(
        f"同步：{len(cases)} 条表达式 + {len(errors)} 条错误号 + {len(approx)} 条近似"
        f" + {len(scripts)} 段脚本。"
    )
    return 0


def _diff(label, wanted, stored):
    missing = [item for item in wanted if item not in stored]
    extra = [item for item in stored if item not in wanted]
    detail = [f"{label}不同步（清单 {len(wanted)} 条 / 数据 {len(stored)} 条）"]
    if missing:
        detail.append("  清单里有、数据里没有：" + "、".join(str(m) for m in missing[:5]))
    if extra:
        detail.append("  数据里有、清单里没有：" + "、".join(str(e) for e in extra[:5]))
    if not missing and not extra:
        detail.append("  条目相同但顺序不同")
    return "\n".join(detail)


# 这些内置函数依赖界面、进程环境、COM 服务器、时钟或随机源，写不进「一份 VB6 工程 + 一份输出」
# 的黄金用例（同样的脚本每次跑结果不同）。它们的行为由 `builtins_tests` / `interpreter_tests`
# 覆盖，覆盖率报告里单独列出、不再当成缺口。
ENVIRONMENT_DEPENDENT = {
    "msgbox": "弹窗",
    "inputbox": "弹窗",
    "shell": "启动外部进程",
    "command": "进程命令行",
    "getallsettings": "读注册表",
    "loadpicture": "读图片文件",
    "createobject": "要本机装了对应的 COM 服务器",
    "getobject": "要本机装了对应的 COM 服务器",
    "doevents": "消息循环",
    "beep": "蜂鸣",
    "sleep": "耗时",
    "tickcount": "时钟",
    "now": "当前时间",
    "rnd": "随机源：VB6 同一 `Randomize 42` 两次跑出的序列都不一样，没法用固定种子复现",
    "randomize": "随机源（同上）",
}


# 有些内置函数在脚本里不是「函数调用」的写法，覆盖率检查光看名字会误报成没覆盖。
COVERED_ALIASES = {
    # `Name 旧 As 新` 语句最终调用的就是 `rename`。
    "rename": r"\bname\b[^\n]*\bas\b",
}


def builtin_names():
    """从解释器源码里读出全部内置函数名（用于覆盖率提示；去重后返回）。"""
    names = []
    for relative in ("src/interpreter/builtins.rs", "src/interpreter/finance.rs"):
        path = os.path.join(ROOT, relative)
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as handle:
            text = handle.read()
        for block in re.findall(r"const BUILTINS: &\[&str\] = &\[(.*?)\];", text, re.S):
            names.extend(re.findall(r'"([a-z0-9_]+)"', block))
    return list(dict.fromkeys(names))


def missing_coverage(cases, errors, scripts):
    """找出「黄金清单里没调用过」的内置函数，并区分单元测试里有没有覆盖。

    返回 (只在单元测试里覆盖的, 两边都没覆盖的, 环境相关因而不做黄金覆盖的)。
    """
    golden = "\n".join(cases + list(errors) + [body for _, _, body in scripts]).lower()
    tests = ""
    for relative in ("src/builtins_tests.rs", "src/interpreter_tests.rs"):
        path = os.path.join(ROOT, relative)
        if os.path.exists(path):
            with open(path, encoding="utf-8") as handle:
                tests += handle.read().lower() + "\n"
    only_unit_tests = []
    untested = []
    environment = []
    for name in builtin_names():
        pattern = rf"\b{re.escape(name)}\b"
        if re.search(pattern, golden):
            continue
        alias = COVERED_ALIASES.get(name)
        if alias and re.search(alias, golden):
            continue
        if name in ENVIRONMENT_DEPENDENT:
            environment.append(name)
        elif re.search(pattern, tests):
            only_unit_tests.append(name)
        else:
            untested.append(name)
    return only_unit_tests, untested, environment


def parse_argv(argv):
    """解析命令行：`--check` / `--expr-only` / `--scripts-only` / `--only 前缀`。"""
    options = {
        "help": False,
        "check": False,
        "expr_only": False,
        "scripts_only": False,
        "bisect": False,
        "only": None,
    }
    index = 0
    while index < len(argv):
        argument = argv[index]
        if argument in ("-h", "--help"):
            options["help"] = True
        elif argument == "--check":
            options["check"] = True
        elif argument == "--expr-only":
            options["expr_only"] = True
        elif argument == "--scripts-only":
            options["scripts_only"] = True
        elif argument == "--bisect":
            options["bisect"] = True
        elif argument == "--only":
            index += 1
            if index >= len(argv):
                sys.exit("--only 后面要跟用例名前缀")
            options["only"] = argv[index]
        else:
            sys.exit(f"未知参数：{argument}\n\n{__doc__}")
        index += 1
    if options["only"] and not options["scripts_only"]:
        sys.exit("--only 只对脚本用例生效，请和 --scripts-only 一起用")
    return options


def main(argv):
    options = parse_argv(argv)
    if options["help"]:
        print(__doc__)
        return 0

    cases = load_cases(CASES_FILE)
    errors = load_cases(ERRORS_FILE)
    approx = load_approx(APPROX_FILE)
    scripts = load_scripts(SCRIPTS_FILE)
    if not cases:
        sys.exit("用例清单是空的")

    if options["check"]:
        return check(cases, errors, approx, scripts)

    existing = parse_data(OUTPUT_FILE) if os.path.exists(OUTPUT_FILE) else None
    partial = options["expr_only"] or options["scripts_only"]
    if existing is None and partial:
        sys.exit("还没有生成过数据文件，先跑一次完整生成")

    data = {
        "cases": existing["cases"] if existing else [],
        "errors": existing["errors"] if existing else [],
        "approx": existing["approx"] if existing else [],
        "scripts": existing["scripts"] if existing else [],
    }

    if not options["scripts_only"]:
        body, parts, spans = expression_body(cases, errors, approx)
        ok, _ = compile_form(BUILD_DIR, "golden", body, spans, parts)
        if not ok:
            if options["bisect"]:
                culprit = bisect_failure(cases, errors, approx)
                if culprit:
                    sys.exit(f"VB6 编译失败，二分定位到这条用例：{culprit}")
            sys.exit("VB6 编译失败（可加 `--bisect` 自动定位到具体用例）")
        values = collect(os.path.join(BUILD_DIR, "golden.exe"))
        data["cases"], data["errors"], data["approx"] = collect_values(
            cases, errors, approx, values
        )

    if scripts and not options["expr_only"]:
        previous = {name: output for name, _, _, output in data["scripts"]}
        if options["only"]:
            selected = [case for case in scripts if case[0].startswith(options["only"])]
            if not selected:
                sys.exit(f"没有名字以 {options['only']} 开头的脚本用例")
            previous.update(build_scripts(selected))
        else:
            previous = build_scripts(scripts)
        data["scripts"] = [
            (name, options_, body, previous.get(name, "")) for name, options_, body in scripts
        ]

    with open(OUTPUT_FILE, "w", encoding="utf-8", newline="\n") as handle:
        handle.write("\n".join(render(data)))
    print(
        f"已生成 {OUTPUT_FILE}（{len(data['cases'])} 条表达式 + {len(data['errors'])} 条错误号"
        f" + {len(data['approx'])} 条近似 + {len(data['scripts'])} 段脚本）。"
    )
    only_unit_tests, untested, environment = missing_coverage(cases, errors, scripts)
    if only_unit_tests:
        print("这些内置函数只在 `builtins_tests`/`interpreter_tests` 里覆盖：" + "、".join(only_unit_tests))
    if untested:
        print("这些内置函数在黄金清单和单元测试里都没出现（可能是漏覆盖）：" + "、".join(untested))
    if environment:
        pairs = "、".join(f"{name}（{ENVIRONMENT_DEPENDENT[name]}）" for name in environment)
        print("这些内置函数依赖环境/时钟/随机源，不做黄金覆盖：" + pairs)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
