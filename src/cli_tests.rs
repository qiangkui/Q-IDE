//! `qide-run` 的命令行解析测试：参数、时长与退出码。
//!
//! 这里只测纯逻辑；真正跑脚本的路径在 `interpreter_tests` 与 `include_tests` 里覆盖。

use std::time::Duration;

use crate::cli::{
    Args, Command, EXIT_OK, EXIT_RUNTIME, EXIT_SYNTAX, Invocation, exit_code, parse_args,
    parse_duration,
};
use crate::interpreter::Error;
use crate::interpreter::error::RuntimeError;

fn parse(args: &[&str]) -> Args {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    match parse_args(&args) {
        Ok(Invocation::Execute(args)) => args,
        other => panic!("应当解析成执行命令：{other:?}"),
    }
}

#[test]
fn run_defaults_to_headless_and_plain_text() {
    let args = parse(&["run", "a.vb"]);
    assert_eq!(args.command, Command::Run);
    assert_eq!(args.script, std::path::PathBuf::from("a.vb"));
    assert!(!args.json);
    assert!(!args.quiet);
    // headless：默认不弹窗，agent 调用时不会卡在 MsgBox 上。
    assert_eq!(args.ui, None);
    assert_eq!(args.pause, None);
    assert_eq!(args.timeout, None);
    assert_eq!(args.max_steps, None);
    assert_eq!(args.command_line, "");
}

#[test]
fn options_parse_in_any_order() {
    let args = parse(&[
        "--json",
        "check",
        "scripts/demo.vb",
        "--max-steps",
        "5000",
        "--max-depth",
        "50",
        "--timeout",
        "1.5s",
        "--include-dir",
        "assets",
        "--arg",
        "hello",
        "--quiet",
        "--ui",
    ]);
    assert_eq!(args.command, Command::Check);
    assert_eq!(args.script, std::path::PathBuf::from("scripts/demo.vb"));
    assert!(args.json && args.quiet);
    assert_eq!(args.ui, Some(true));
    assert_eq!(args.pause, None);
    assert_eq!(args.max_steps, Some(5000));
    assert_eq!(args.max_depth, Some(50));
    assert_eq!(args.timeout, Some(Duration::from_millis(1500)));
    assert_eq!(args.include_dir, Some(std::path::PathBuf::from("assets")));
    assert_eq!(args.command_line, "hello");
}

#[test]
fn a_bare_script_path_means_run() {
    // 文件关联只传一个路径：`qide-run "脚本.vb"` 当 run 处理。
    let args = parse(&["C:\\demo\\脚本.vb"]);
    assert_eq!(args.command, Command::Run);
    assert_eq!(args.script, std::path::PathBuf::from("C:\\demo\\脚本.vb"));

    let args = parse(&["--json", "demo.vbs"]);
    assert_eq!(args.command, Command::Run);
    assert!(args.json);
}

#[test]
fn pause_and_ui_flags_are_three_state() {
    assert_eq!(parse(&["run", "a.vb", "--pause"]).pause, Some(true));
    assert_eq!(parse(&["run", "a.vb", "--no-pause"]).pause, Some(false));
    assert_eq!(parse(&["run", "a.vb", "--no-ui"]).ui, Some(false));
    assert_eq!(parse(&["run", "a.vb"]).ui, None);
}

#[test]
fn help_and_version_short_circuit() {
    for text in ["-h", "--help"] {
        assert_eq!(
            parse_args(&[text.to_string()]),
            Ok(Invocation::Help),
            "{text}"
        );
    }
    for text in ["-V", "--version"] {
        assert_eq!(
            parse_args(&[text.to_string()]),
            Ok(Invocation::Version),
            "{text}"
        );
    }
}

#[test]
fn bad_arguments_are_rejected() {
    let cases: &[&[&str]] = &[
        &[],
        &["run"],
        &["unknown", "a.vb"],
        &["run", "a.vb", "extra.vb"],
        &["run", "a.vb", "--nope"],
        &["run", "a.vb", "--max-steps"],
        &["run", "a.vb", "--max-steps", "0"],
        &["run", "a.vb", "--timeout", "-3s"],
        &["run", "a.vb", "--timeout", "abc"],
    ];
    for case in cases {
        let args: Vec<String> = case.iter().map(|arg| arg.to_string()).collect();
        assert!(parse_args(&args).is_err(), "应当报错：{case:?}");
    }
}

#[test]
fn durations_accept_seconds_millis_and_minutes() {
    assert_eq!(parse_duration("30"), Ok(Duration::from_secs(30)));
    assert_eq!(parse_duration("30s"), Ok(Duration::from_secs(30)));
    assert_eq!(parse_duration("500ms"), Ok(Duration::from_millis(500)));
    assert_eq!(parse_duration("2m"), Ok(Duration::from_secs(120)));
    assert!(parse_duration("0").is_err());
}

#[test]
fn exit_codes_follow_the_documented_mapping() {
    assert_eq!(exit_code(&Ok(())), EXIT_OK);
    assert_eq!(
        exit_code(&Err(Error::Parse(crate::interpreter::ParseError::new(
            "x", 1
        )))),
        EXIT_SYNTAX
    );
    assert_eq!(
        exit_code(&Err(Error::Runtime(RuntimeError::new("x", 1)))),
        EXIT_RUNTIME
    );
}
