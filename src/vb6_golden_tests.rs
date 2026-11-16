//! VB6 官方行为黄金测试。
//!
//! 期望值由 `tools/vb6_golden.py` 用本机 VB6 真编译生成（见 [`crate::vb6_golden_data`]），
//! 覆盖运算符、类型转换、字符串与数值函数。改解释器语义后跑这一组，能立刻发现偏离。

use std::sync::atomic::AtomicBool;

use crate::interpreter::{RunOptions, run};

/// QIDE 有意与 VB6 不同、或受实现限制的用例（原因见 `docs/脚本语言.md`）。
///
/// 以 `*` 结尾的条目按前缀匹配（前缀部分用例太多时用）。日期显示、`DateSerial` 归一化、
/// `DatePart` 都已按 VB6 实测对齐（`DatePart` 的年末两天偏差见 `docs/脚本语言.md` 第 13 条，
/// 那种日期不进清单）。
///
/// 目前是**空的**：三角函数极点/零点附近那几个末位不稳的用例已经挪到
/// `tools/vb6_golden_approx.txt`（`APPROX_CASES`，按相对误差比对），这里只留机制。
const KNOWN_DIFFERENCES: &[&str] = &[];

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

/// 浮点末位不稳的用例（`tools/vb6_golden_approx.txt`）：按相对误差比对。
///
/// VB6 是 x87 长双精度（`IRR`/`Rate` 还是迭代解），这些值逐位比对永远过不了；
/// 判据是 `|QIDE - VB6| <= 容差 * max(|VB6|, 1)`。
#[test]
fn approximate_expressions_match_vb6_golden() {
    let mut failures = Vec::new();
    for (expression, expected, tolerance) in crate::vb6_golden_data::APPROX_CASES {
        let actual = match evaluate(expression) {
            Ok(actual) => actual,
            Err(error) => {
                failures.push(format!(
                    "{expression}\n  期望 {expected:?}\n  实际报错 {error}"
                ));
                continue;
            }
        };
        let (Ok(wanted), Ok(got)) = (expected.parse::<f64>(), actual.parse::<f64>()) else {
            failures.push(format!(
                "{expression}\n  期望 {expected:?}\n  实际 {actual:?}（不是数值，无法按容差比对）"
            ));
            continue;
        };
        let limit = tolerance * wanted.abs().max(1.0);
        let difference = (got - wanted).abs();
        if difference.is_nan() || difference > limit {
            failures.push(format!(
                "{expression}\n  期望 {wanted}（容差 {tolerance:e}）\n  实际 {got}（差 {difference:.3e}）"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "近似用例与 VB6 差得太多（{} 条）：\n{}",
        failures.len(),
        failures.join("\n")
    );
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
    let script_dir = std::env::temp_dir().join(format!(
        "qide-golden-scripts-{}-{}",
        std::process::id(),
        crate::vb6_golden_data::SCRIPT_CASES.len()
    ));
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
        // 每段脚本都在自己的临时目录里跑：文件类用例（`Open` / `FileCopy` / `Name`）只碰相对路径，
        // 免得把测试生成的文件写进仓库根目录。
        let _ = std::fs::remove_dir_all(&script_dir);
        std::fs::create_dir_all(&script_dir).expect("创建脚本用例的临时目录");
        let options = crate::interpreter::RunOptions {
            allow_ui: false,
            script_path: Some(script_dir.join("golden-script.vb")),
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
    let _ = std::fs::remove_dir_all(&script_dir);
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
