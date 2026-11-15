//! 外部 agent 传输层与工具清单的测试。

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use crate::agent::{
    AgentReply, Endpoint, SAFE_COMMANDS, TOOLS_JSON, ensure_not_root, start_server, tool_defined,
    validate_move,
};

#[test]
fn shared_tool_list_is_valid_and_unique() {
    let tools: Vec<Value> = serde_json::from_str(TOOLS_JSON).expect("工具清单应当是合法 JSON");
    let mut names = Vec::new();
    for tool in &tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .expect("每个工具都要有 name");
        let parameters = tool.get("parameters").expect("每个工具都要有 parameters");
        assert_eq!(
            parameters.get("type").and_then(Value::as_str),
            Some("object")
        );
        assert!(!names.contains(&name), "工具名重复：{name}");
        names.push(name);
    }
    for expected in [
        "qide_get_app_state",
        "qide_list_commands",
        "qide_list_files",
        "qide_read_file",
        "qide_read_output",
        "qide_get_debug_state",
        "qide_open_file",
        "qide_run_command",
        "qide_write_file",
        "qide_replace_in_file",
        "qide_delete_path",
        "qide_move_path",
        "qide_check_syntax",
        "qide_run_script",
        "qide_start_debug",
        "qide_debug_continue",
        "qide_debug_step",
        "qide_debug_pause",
        "qide_stop_execution",
        "qide_set_breakpoint",
        "qide_add_watch",
    ] {
        assert!(names.contains(&expected), "缺少工具 {expected}");
        assert!(tool_defined(expected));
    }
    assert!(!tool_defined("qide_delete_everything"));
}

#[test]
fn safe_commands_exclude_dangerous_operations() {
    for allowed in ["file.save", "edit.comment", "view.output"] {
        assert!(SAFE_COMMANDS.contains(&allowed), "{allowed} 应当允许");
    }
    for denied in [
        "run.start",
        "run.debug",
        "run.stop",
        "file.save-as",
        "goto.line",
    ] {
        assert!(!SAFE_COMMANDS.contains(&denied), "{denied} 不应允许");
    }
}

#[test]
fn endpoint_json_carries_port_and_token() {
    let endpoint = Endpoint {
        port: 45678,
        token: "secret-token".to_string(),
    };
    let value = endpoint.to_json();
    assert_eq!(value.get("port").and_then(Value::as_u64), Some(45678));
    assert_eq!(
        value.get("token").and_then(Value::as_str),
        Some("secret-token")
    );
}

#[test]
fn loopback_server_round_trips_a_tool_call() {
    let (request_tx, request_rx) = std::sync::mpsc::channel();
    let endpoint = start_server(request_tx).expect("启动回环服务");
    let worker = std::thread::spawn(move || {
        let request = request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("应当收到请求");
        assert_eq!(request.tool, "qide_get_app_state");
        assert_eq!(request.arguments, json!({"x": 1}));
        let _ = request.reply.send(AgentReply::ok(r#"{"roots":[]}"#));
    });

    let response = send_request(
        &endpoint,
        &endpoint.token,
        "qide_get_app_state",
        json!({"x": 1}),
    );
    assert_eq!(response.get("ok").and_then(Value::as_bool), Some(true));
    assert_eq!(
        response.get("content").and_then(Value::as_str),
        Some(r#"{"roots":[]}"#)
    );
    worker.join().expect("假执行线程");
}

#[test]
fn loopback_server_rejects_a_wrong_token() {
    let (request_tx, _request_rx) = std::sync::mpsc::channel();
    let endpoint = start_server(request_tx).expect("启动回环服务");
    let response = send_request(&endpoint, "wrong-token", "qide_get_app_state", json!({}));
    assert_eq!(response.get("ok").and_then(Value::as_bool), Some(false));
    assert!(
        response
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| content.contains("token"))
    );
}

#[test]
fn mutation_guards_reject_roots_and_bad_move_targets() {
    let root = std::env::temp_dir().join(format!("qide-agent-mutation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).expect("创建子目录");
    std::fs::write(root.join("a.txt"), "x").expect("写文件");

    let roots: Vec<PathBuf> = vec![root.clone()];
    assert!(
        ensure_not_root(&root, &roots).is_err(),
        "不能操作项目根目录"
    );
    assert!(ensure_not_root(&root.join("a.txt"), &roots).is_ok());

    assert!(
        validate_move(&root.join("a.txt"), &root).is_err(),
        "已经在目标目录里"
    );
    assert!(validate_move(&root.join("a.txt"), &root.join("sub")).is_ok());
    assert!(
        validate_move(&root.join("sub"), &root.join("sub")).is_err(),
        "不能移动到自身"
    );

    let inner = root.join("sub").join("inner");
    std::fs::create_dir_all(&inner).expect("创建孙目录");
    assert!(
        validate_move(&root.join("sub"), &inner).is_err(),
        "不能移动到自己的子目录"
    );

    let _ = std::fs::remove_dir_all(&root);
}

fn send_request(endpoint: &Endpoint, token: &str, tool: &str, arguments: Value) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", endpoint.port)).expect("连接回环端口");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("设置读超时");
    let request = json!({
        "token": token,
        "id": 1,
        "tool": tool,
        "arguments": arguments,
    });
    writeln!(stream, "{request}").expect("发送请求");
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).expect("读取响应");
    serde_json::from_str(line.trim()).expect("响应应当是 JSON")
}
