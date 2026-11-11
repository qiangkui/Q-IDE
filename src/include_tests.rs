//! `#Include "文件.vb"` 的展开、行号映射与 IDE 集成测试。

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::interpreter::{RunOptions, run_with};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("qide_include_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("创建临时目录");
        Self(path)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("写临时脚本");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn output(source: &str, path: &Path) -> Result<Vec<String>, crate::interpreter::Error> {
    let mut lines = Vec::new();
    let flag = AtomicBool::new(false);
    run_with(
        source,
        |line| lines.push(line.to_string()),
        &flag,
        RunOptions {
            script_path: Some(path.to_path_buf()),
            ..RunOptions::default()
        },
    )?;
    Ok(lines)
}

#[test]
fn include_expands_types_constants_and_keeps_main_line_numbers() {
    let dir = TempDir::new("expand");
    dir.write(
        "common.vb",
        "' 常用定义\nPrivate Const ANSWER As Long = 42\nPrivate Type Pt\n    X As Long\n    Y As Long\nEnd Type\n",
    );
    let main = "#Include \"common.vb\"\nOption Explicit\nDebug.Print ANSWER\nDim p As Pt\np.X = 7\nDebug.Print p.X\nSub Main()\nEnd Sub\n";
    let path = dir.write("main.vb", main);

    let program = crate::interpreter::parse_with_path(main, Some(&path)).expect("应当解析成功");
    // 主文件的行号不受 include 影响（断点、诊断都靠它）。
    assert_eq!(program.procedures[0].line, 7);
    assert!(program.types.iter().any(|ty| ty.display == "Pt"));

    assert_eq!(
        output(main, &path).expect("应当运行成功"),
        ["42", "7"]
    );
    assert!(
        crate::lint::analyze_with_path(main, Some(&path))
            .issues
            .is_empty()
    );
}

#[test]
fn include_falls_back_to_the_program_assets_directory() {
    let dir = TempDir::new("assets");
    let assets = TempDir::new("assets-dir");
    assets.write("common.vb", "Private Const FROM_ASSETS As Long = 7\n");
    let main = "#Include \"common.vb\"\nDebug.Print FROM_ASSETS\n";
    let path = dir.write("main.vb", main);

    let expanded =
        crate::interpreter::include::expand_with_assets(main, Some(&path), Some(assets.0.as_path()))
            .expect("应当从 assets 找到 include");
    assert!(expanded.text.contains("FROM_ASSETS"), "{}", expanded.text);

    let program =
        crate::interpreter::parser::parse_mapped(&expanded.text, &expanded.logical_lines).unwrap();
    let mut lines = Vec::new();
    let flag = AtomicBool::new(false);
    crate::interpreter::runtime::run_program(
        &program,
        &mut |line| lines.push(line.to_string()),
        &flag,
        &RunOptions {
            script_path: Some(path.clone()),
            ..RunOptions::default()
        },
        None,
    )
    .expect("应当运行成功");
    assert_eq!(lines, ["7"]);
}

#[test]
fn script_directory_takes_precedence_over_assets() {
    let dir = TempDir::new("precedence");
    let assets = TempDir::new("precedence-assets");
    dir.write("common.vb", "Private Const FROM_SCRIPT As Long = 1\n");
    assets.write("common.vb", "Private Const FROM_ASSETS As Long = 2\n");
    let main = "#Include \"common.vb\"\nDebug.Print FROM_SCRIPT\n";
    let path = dir.write("main.vb", main);
    let expanded =
        crate::interpreter::include::expand_with_assets(main, Some(&path), Some(assets.0.as_path()))
            .expect("脚本目录里的同名文件优先");
    assert!(expanded.text.contains("FROM_SCRIPT"));
    assert!(!expanded.text.contains("FROM_ASSETS"));
}

#[test]
fn missing_include_reports_the_assets_directory_too() {
    let dir = TempDir::new("assets-missing");
    let assets = TempDir::new("assets-missing-dir");
    let main = "#Include \"nope.vb\"\n";
    let path = dir.write("main.vb", main);
    let error = crate::interpreter::include::expand_with_assets(
        main,
        Some(&path),
        Some(assets.0.as_path()),
    )
    .unwrap_err();
    assert_eq!(error.line, 1);
    assert!(error.message.contains("assets"), "{error:?}");
}

#[test]
fn include_only_happens_once() {
    let dir = TempDir::new("once");
    dir.write("common.vb", "Private Const VALUE As Long = 9\n");
    let main = "#Include \"common.vb\"\n#Include \"common.vb\"\nDebug.Print VALUE\n";
    let path = dir.write("main.vb", main);
    assert_eq!(output(main, &path).expect("重复 include 不应重复定义"), ["9"]);
}

#[test]
fn include_cycle_is_reported_at_the_directive() {
    let dir = TempDir::new("cycle");
    dir.write("a.vb", "#Include \"b.vb\"\n");
    dir.write("b.vb", "#Include \"a.vb\"\n");
    let main = "#Include \"a.vb\"\nDebug.Print 1\n";
    let path = dir.write("main.vb", main);
    let error = crate::interpreter::parse_with_path(main, Some(&path)).unwrap_err();
    assert_eq!(error.line(), 1);
    assert!(error.message().contains("循环引用"), "{error}");
}

#[test]
fn missing_include_reports_file_and_line() {
    let dir = TempDir::new("missing");
    let main = "Option Explicit\n#Include \"missing.vb\"\n";
    let path = dir.write("main.vb", main);
    let error = crate::interpreter::parse_with_path(main, Some(&path)).unwrap_err();
    assert_eq!(error.line(), 2);
    assert!(error.message().contains("找不到文件"), "{error}");
}

#[test]
fn nested_include_error_maps_to_the_top_level_directive() {
    let dir = TempDir::new("nested-error");
    dir.write(
        "b.vb",
        "Private Const B1 As Long = 1\nPrivate Const B2 As Long = 2\nPrivate Const B3 As Long =\n",
    );
    dir.write(
        "a.vb",
        "Private Const A1 As Long = 1\n#Include \"b.vb\"\nPrivate Const A2 As Long = 2\n",
    );
    let main = "#Include \"a.vb\"\nDebug.Print 1\n";
    let path = dir.write("main.vb", main);
    let error = crate::interpreter::parse_with_path(main, Some(&path)).unwrap_err();
    assert_eq!(error.line(), 1);
    assert!(error.message().contains("b.vb 第 3 行"), "{error}");
}

#[test]
fn nested_include_definitions_are_in_scope() {
    let dir = TempDir::new("nested-ok");
    dir.write("b.vb", "Private Const B1 As Long = 2\n");
    dir.write("a.vb", "Private Const A1 As Long = 1\n#Include \"b.vb\"\nPrivate Const A2 As Long = 3\n");
    let main = "#Include \"a.vb\"\nDebug.Print A1 + B1 + A2\n";
    let path = dir.write("main.vb", main);
    assert_eq!(output(main, &path).expect("嵌套 include 应当可用"), ["6"]);
}

#[test]
fn parse_error_in_included_file_points_to_the_directive() {
    let dir = TempDir::new("parse-error");
    dir.write("bad.vb", "Private Const X As Long =\n");
    let main = "#Include \"bad.vb\"\nDebug.Print 1\n";
    let path = dir.write("main.vb", main);
    let error = crate::interpreter::parse_with_path(main, Some(&path)).unwrap_err();
    assert_eq!(error.line(), 1);
    assert!(error.message().contains("bad.vb 第 1 行"), "{error}");
}

#[test]
fn runtime_error_in_included_file_points_to_the_directive() {
    let dir = TempDir::new("runtime-error");
    dir.write("bad.vb", "Private Const X As Long = 1 / 0\n");
    let main = "#Include \"bad.vb\"\nDebug.Print 1\n";
    let path = dir.write("main.vb", main);
    let error = output(main, &path).unwrap_err();
    assert_eq!(error.line(), 1);
    assert!(error.message().contains("bad.vb 第 1 行"), "{error}");
}

#[test]
fn completion_model_includes_symbols_from_included_files() {
    let dir = TempDir::new("completion");
    dir.write(
        "common.vb",
        "Private Const VALUE As Long = 1\nPrivate Type Shared\n    X As Long\nEnd Type\nPrivate Sub InCommon()\nEnd Sub\n",
    );
    let main = "#Include \"common.vb\"\nSub Main()\nEnd Sub\n";
    let path = dir.write("main.vb", main);
    let model = crate::outline::OutlineModel::from_source_for_completion(main, Some(&path));
    assert!(model.procedures.iter().any(|symbol| symbol.name == "main"));
    assert!(
        model
            .procedures
            .iter()
            .any(|symbol| symbol.name == "incommon")
    );
    assert!(model.types.iter().any(|symbol| symbol.name == "Shared"));
}

#[test]
fn outline_only_lists_symbols_from_the_main_file() {
    let dir = TempDir::new("outline");
    dir.write(
        "common.vb",
        "Private Const VALUE As Long = 1\nPrivate Type Shared\n    X As Long\nEnd Type\nPrivate Sub InCommon()\nEnd Sub\n",
    );
    let main = "#Include \"common.vb\"\nSub Main()\nEnd Sub\n";
    let path = dir.write("main.vb", main);
    let model = crate::outline::OutlineModel::from_source_with_path(main, Some(&path));
    assert!(model.procedures.iter().any(|symbol| symbol.name == "main"));
    assert!(
        !model
            .procedures
            .iter()
            .any(|symbol| symbol.name == "incommon"),
        "{:?}",
        model.procedures
    );
    assert!(model.types.is_empty());
}
