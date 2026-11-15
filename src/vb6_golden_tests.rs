//! VB6 官方行为黄金测试。
//!
//! 期望值由 `tools/vb6_golden.py` 用本机 VB6 真编译生成（见 [`crate::vb6_golden_data`]），
//! 覆盖运算符、类型转换、字符串与数值函数。改解释器语义后跑这一组，能立刻发现偏离。

use std::sync::atomic::AtomicBool;

use crate::interpreter::{RunOptions, run};

/// QIDE 有意与 VB6 不同、或受实现限制的用例（原因见 `docs/脚本语言.md`）。
///
/// 以 `*` 结尾的条目按前缀匹配（前缀部分用例太多时用）。
const KNOWN_DIFFERENCES: &[&str] = &[
    // 整数字面量在 VB6 里是 Integer（`VarType`/`TypeName` 给 2/"Integer"），QIDE 统一按 Long 承载。
    "VarType(1)",
    "TypeName(1)",
    // QIDE 没有独立的 Single 值类型：`CSng` 的值已窄化到 32 位，但按 Double 的 15 位有效数字打印
    // （VB6 按 Single 的 7 位）。
    "CSng(*",
    // 日期显示用固定的 ISO 格式（`2024-02-29`），VB6 用系统区域短日期格式（中文系统是 `2024/2/29`）。
    // 取舍见 `docs/脚本语言.md`：ISO 不随机器区域变化，测试才可复现。
    "CStr(CDate(*",
    "DateAdd(*",
    // `Hex`/`Oct` 的位数跟着参数类型走：VB6 里 `-1` 是 Integer，给 16 位的 `FFFF`；
    // QIDE 没有独立的 Integer 值（整数字面量一律按 Long 承载），给 32 位的 `FFFFFFFF`。
    "Hex(-1)",
    "Oct(-1)",
];

fn evaluate(expression: &str) -> Result<String, String> {
    let source = format!("Debug.Print {expression}");
    let mut lines = Vec::new();
    let interrupt = AtomicBool::new(false);
    run(&source, |line| lines.push(line.to_string()), &interrupt)
        .map(|()| lines.join(" | "))
        .map_err(|error| error.to_string())
}

#[test]
fn expressions_match_vb6_golden() {
    let mut failures = Vec::new();
    for (expression, expected) in crate::vb6_golden_data::CASES {
        if is_known_difference(expression) {
            continue;
        }
        match evaluate(expression) {
            Ok(actual) if actual == *expected => {}
            Ok(actual) => failures.push(format!(
                "{expression}\n  期望 {expected:?}\n  实际 {actual:?}"
            )),
            Err(error) => failures.push(format!(
                "{expression}\n  期望 {expected:?}\n  实际报错 {error}"
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "与 VB6 不一致（{} 条）：\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn is_known_difference(expression: &str) -> bool {
    KNOWN_DIFFERENCES
        .iter()
        .any(|pattern| match pattern.strip_suffix('*') {
            Some(prefix) => expression.starts_with(prefix),
            None => expression == *pattern,
        })
}

/// 比较脚本用例输出前先折叠空白：VB6 的 `Print` 给数值留符号位并补尾随空格，
/// QIDE 直接输出数值文本（见 `docs/脚本语言.md`），两者的**换行与内容**仍逐行比对。
fn normalize_print(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 脚本用例里已知与 VB6 不同的名字（原因见 `docs/脚本语言.md`）。
const SCRIPT_KNOWN_DIFFERENCES: &[&str] = &[];

/// 整段脚本（控制流、数组、错误处理、过程调用）的输出必须与 VB6 一致。
///
/// 期望值同样是本机 VB6 真编译跑出来的；`Debug.Print` 在 VB6 侧被换成写文件，
/// 因此这条测试同时校验「语句语义」和「`Debug.Print` 的分行/分区」。
#[test]
fn scripts_match_vb6_golden() {
    let mut failures = Vec::new();
    for (name, options, body, expected) in crate::vb6_golden_data::SCRIPT_CASES {
        if SCRIPT_KNOWN_DIFFERENCES.contains(name) {
            continue;
        }
        // 与 VB6 侧同构：模块级选项 + 共用声明 + 一段过程体（`Exit Sub` 等语句要有过程可退）。
        let source = format!(
            "{options}\n{}\nSub Main()\n{body}\nEnd Sub\n",
            crate::vb6_golden_data::SCRIPT_SUPPORT
        );
        let mut lines = Vec::new();
        let interrupt = AtomicBool::new(false);
        let options = crate::interpreter::RunOptions {
            allow_ui: false,
            ..RunOptions::default()
        };
        match crate::interpreter::run_with(
            &source,
            |line| lines.push(line.to_string()),
            &interrupt,
            options,
        ) {
            Ok(()) => {
                let actual = lines.join("\n");
                if normalize_print(&actual) != normalize_print(expected) {
                    failures.push(format!("[{name}]\n  期望:\n{expected}\n  实际:\n{actual}"));
                }
            }
            Err(error) => failures.push(format!("[{name}]\n  期望:\n{expected}\n  报错: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "脚本用例与 VB6 不一致（{} 段）：\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// 错误号也必须与 VB6 一致（同一段脚本在两边跑出不同的 `Err.Number`，用户就没法靠它分支）。
#[test]
fn error_numbers_match_vb6_golden() {
    use crate::interpreter::RunOptions;

    let mut failures = Vec::new();
    for (statement, expected) in crate::vb6_golden_data::ERROR_CASES {
        // 前置声明与表达式用例保持一致，单独的语句才能顺利跑起来。
        let source = format!(
            "Option Explicit\n\
             Dim x, i As Integer, b As Byte, l As Long, d As Date\n\
             Dim arr(2) As Long\n\
             Dim empty_arr() As Long\n\
             Dim c As Collection\n\
             On Error Resume Next\n\
             {statement}\n\
             Debug.Print Err.Number\n"
        );
        let mut lines = Vec::new();
        let interrupt = AtomicBool::new(false);
        let options = RunOptions {
            allow_ui: false,
            ..RunOptions::default()
        };
        let outcome = crate::interpreter::run_with(
            &source,
            |line| lines.push(line.to_string()),
            &interrupt,
            options,
        );
        match outcome {
            Ok(()) => {
                let actual = lines.last().map(String::as_str).unwrap_or("");
                // 不报错的语句在 VB6 那边也会被生成脚本拒绝，这里作为硬错误。
                if actual == "0" {
                    failures.push(format!("{statement}\n  VB6 报 {expected}，QIDE 没有报错"));
                } else if actual != expected.to_string() {
                    failures.push(format!(
                        "{statement}\n  期望错误号 {expected}\n  实际 {actual}"
                    ));
                }
            }
            Err(error) => failures.push(format!(
                "{statement}\n  期望错误号 {expected}\n  实际报错 {error}"
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "错误号与 VB6 不一致（{} 条）：\n{}",
        failures.len(),
        failures.join("\n")
    );
}
