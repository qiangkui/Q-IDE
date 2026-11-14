//! 主程序命令行参数解析的测试。

use std::path::{Path, PathBuf};

use crate::startup::{Command, Level, Target, classify, covered_by_roots, parse, version};

fn open(paths: &[&str]) -> Command {
    Command::Open(paths.iter().map(|path| path.to_string()).collect())
}

#[test]
fn no_arguments_means_plain_start() {
    assert_eq!(parse(std::iter::empty::<&str>()), open(&[]));
}

#[test]
fn paths_are_kept_in_order() {
    let args = ["C:\\项目\\a.vb", "D:\\工作", "b.vb"];
    assert_eq!(parse(args), open(&args));
}

#[test]
fn help_spellings() {
    for arg in ["-h", "--help", "/?"] {
        assert_eq!(parse([arg]), Command::Help, "{arg}");
    }
    // 帮助优先：后面的路径不再解析。
    assert_eq!(parse(["a.vb", "--help"]), Command::Help);
}

#[test]
fn version_spellings() {
    for arg in ["-V", "--version"] {
        assert_eq!(parse([arg]), Command::Version, "{arg}");
    }
}

#[test]
fn unknown_option_is_reported() {
    match parse(["a.vb", "--bogus"]) {
        Command::Invalid(message) => assert!(message.contains("--bogus"), "{message}"),
        other => panic!("期望参数错误，实际是 {other:?}"),
    }
    // 单个 `-` 当作路径：它经常被用来表示标准输入。
    assert_eq!(parse(["-"]), open(&["-"]));
}

#[test]
fn double_dash_stops_option_parsing() {
    assert_eq!(parse(["--", "--help", "-V"]), open(&["--help", "-V"]));
    assert_eq!(parse(["a.vb", "--", "-x"]), open(&["a.vb", "-x"]));
}

#[test]
fn classify_uses_the_file_system() {
    let dir = std::env::temp_dir().join(format!("qide-startup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    let file = dir.join("示例.vb");
    std::fs::write(&file, "Sub Main\nEnd Sub\n").expect("写临时文件");

    assert_eq!(classify(&dir), Target::Dir);
    assert_eq!(classify(&file), Target::File);
    assert_eq!(classify(&dir.join("不存在.vb")), Target::Missing);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn roots_cover_subfolders() {
    let roots = vec![PathBuf::from("D:\\工作")];
    assert!(covered_by_roots(&roots, Path::new("D:\\工作")));
    assert!(covered_by_roots(&roots, Path::new("D:\\工作\\脚本")));
    assert!(!covered_by_roots(&roots, Path::new("E:\\其它")));
    assert!(!covered_by_roots(&[], Path::new("D:\\工作")));
}

#[test]
fn version_names_the_app() {
    assert!(version().starts_with("Q-IDE "), "{}", version());
}

#[test]
fn usage_documents_every_option() {
    for flag in ["-h", "--help", "-V", "--version", "--"] {
        assert!(crate::startup::USAGE.contains(flag), "用法里缺少 {flag}");
    }
    // Level 只用于选择图标，这里确认两个取值都存在，避免误删。
    assert_ne!(Level::Info, Level::Warning);
}
