//! AI 对话的纯逻辑测试：地址解析、请求体拼装、应答解析与 SSE 增量解析。

use crate::ai::client::{
    ChatMessage, Endpoint, Role, ToolCall, endpoint_from, fit_history, parse_reply, request_body,
    REQUEST_CHAR_BUDGET,
};
use crate::ai::stream::{SseDecoder, SseEvent};
use serde_json::json;

/// 拼一条 DeepSeek 风格的增量事件（带空行）。
fn sse_delta(content: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{content}\"}}}}]}}\n\n"
    )
}

fn endpoint(secure: bool, host: &str, port: u16, path: &str) -> Endpoint {
    Endpoint {
        secure,
        host: host.to_string(),
        port,
        path: path.to_string(),
    }
}

#[test]
fn endpoint_uses_default_ports_and_appends_the_completion_path() {
    assert_eq!(
        endpoint_from("https://api.deepseek.com"),
        Ok(endpoint(true, "api.deepseek.com", 443, "/chat/completions"))
    );
    // 结尾多写的斜杠与首尾空格不影响结果。
    assert_eq!(
        endpoint_from("  https://api.deepseek.com/  "),
        Ok(endpoint(true, "api.deepseek.com", 443, "/chat/completions"))
    );
    assert_eq!(
        endpoint_from("http://example.com"),
        Ok(endpoint(false, "example.com", 80, "/chat/completions"))
    );
}

#[test]
fn endpoint_keeps_custom_path_and_port() {
    assert_eq!(
        endpoint_from("https://api.deepseek.com/v1"),
        Ok(endpoint(
            true,
            "api.deepseek.com",
            443,
            "/v1/chat/completions"
        ))
    );
    assert_eq!(
        endpoint_from("http://127.0.0.1:8080/v1"),
        Ok(endpoint(false, "127.0.0.1", 8080, "/v1/chat/completions"))
    );
    assert_eq!(
        endpoint_from("https://api.deepseek.com:8443"),
        Ok(endpoint(
            true,
            "api.deepseek.com",
            8443,
            "/chat/completions"
        ))
    );
}

#[test]
fn endpoint_rejects_addresses_without_scheme_or_host() {
    assert!(endpoint_from("").is_err());
    assert!(endpoint_from("api.deepseek.com").is_err());
    assert!(endpoint_from("https://").is_err());
    assert!(endpoint_from("https://api.deepseek.com:99999").is_err());
}

#[test]
fn request_body_starts_with_the_system_prompt_and_keeps_the_roles() {
    let messages = vec![
        ChatMessage::user("写一个冒泡排序"),
        ChatMessage::assistant("好的"),
        ChatMessage::user("改成 \"降序\""),
    ];
    let body = request_body("deepseek-chat", &messages, true, None, &json!([]));
    let value: serde_json::Value = serde_json::from_str(&body).expect("请求体是合法 JSON");

    assert_eq!(value["model"], "deepseek-chat");
    assert_eq!(value["stream"], true);
    let json_messages = value["messages"].as_array().expect("messages 是数组");
    assert_eq!(json_messages.len(), 4);
    assert_eq!(json_messages[0]["role"], "system");
    assert_eq!(json_messages[1]["role"], "user");
    assert_eq!(json_messages[1]["content"], "写一个冒泡排序");
    assert_eq!(json_messages[2]["role"], "assistant");
    assert_eq!(json_messages[3]["content"], "改成 \"降序\"");
    // 密钥绝不会进请求体，只在请求头里。
    assert!(!body.contains("Authorization"));
}

#[test]
fn reply_content_is_extracted_from_the_first_choice() {
    let body = r#"{
        "id": "chat-1",
        "choices": [
            {"index": 0, "message": {"role": "assistant", "content": "  print 1  \n"}, "finish_reason": "stop"}
        ]
    }"#;
    assert_eq!(parse_reply(200, body), Ok("print 1".to_string()));
}

#[test]
fn reply_error_message_is_surfaced_with_the_status() {
    let body = r#"{"error": {"message": "Insufficient Balance", "type": "insufficient_quota"}}"#;
    assert_eq!(
        parse_reply(402, body),
        Err("Insufficient Balance（HTTP 402）".to_string())
    );
}

#[test]
fn reply_without_content_or_json_reports_a_readable_error() {
    assert!(parse_reply(500, "<html>bad gateway</html>").is_err());
    assert!(parse_reply(200, r#"{"choices": []}"#).is_err());
    assert!(parse_reply(200, r#"{"choices": [{"message": {"content": "   "}}]}"#).is_err());
    assert_eq!(
        parse_reply(404, r#"{"ok": false}"#),
        Err("请求失败（HTTP 404）".to_string())
    );
}

#[test]
fn sse_decoder_reassembles_events_split_across_chunks() {
    let mut decoder = SseDecoder::new();
    let payload = format!("{}{}", sse_delta("你"), sse_delta("好"));
    let bytes = payload.as_bytes();
    // 故意切在汉字中间：网络读到的数据可能在任意字节处断开。
    let chinese = payload.find('你').expect("测试数据里有汉字") + 1;
    assert!(chinese < bytes.len());

    let mut events = decoder.push(&bytes[..chinese]);
    events.extend(decoder.push(&bytes[chinese..]));

    assert_eq!(
        events,
        vec![
            SseEvent::Delta("你".to_string()),
            SseEvent::Delta("好".to_string())
        ]
    );
    assert!(!decoder.is_done());
    assert!(decoder.finish().is_empty(), "没有残留的半行");
}

#[test]
fn sse_decoder_stops_at_done_and_ignores_later_bytes() {
    let mut decoder = SseDecoder::new();
    assert_eq!(
        decoder.push(b"data: [DONE]\r\n\r\n"),
        vec![SseEvent::Done]
    );
    assert!(decoder.is_done());
    assert!(decoder.push(b"data: {\"choices\":[]}\n").is_empty());
    assert!(decoder.finish().is_empty());
}

#[test]
fn sse_decoder_reports_errors_and_skips_noise() {
    let mut decoder = SseDecoder::new();
    let input = concat!(
        ": keep-alive\n",
        "event: message\n",
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n",
        "data: {\"choices\":[{\"delta\":{}}]}\n",
        "data: 不是 JSON\n",
        "data: {\"error\":{\"message\":\"rate limited\"}}\n",
    );
    assert_eq!(
        decoder.push(input.as_bytes()),
        vec![SseEvent::Error("rate limited".to_string())]
    );
}

#[test]
fn sse_decoder_flushes_a_last_line_without_newline() {
    let mut decoder = SseDecoder::new();
    let payload = sse_delta("嗨");
    let payload = payload.trim_end_matches('\n');
    assert!(decoder.push(payload.as_bytes()).is_empty());
    assert_eq!(decoder.finish(), vec![SseEvent::Delta("嗨".to_string())]);
}

#[test]
fn markdown_split_separates_prose_and_code() {
    use crate::ai::markdown::{self, Segment};

    let content = "先说明一下：\n\n```vb\nFor i = 1 To 10\nNext\n```\n\n最后一句。\n";
    let segments = markdown::split(content);
    assert_eq!(segments.len(), 3);
    match &segments[0] {
        Segment::Text(text) => assert!(text.contains("先说明一下")),
        other => panic!("第一段应该是说明文字：{other:?}"),
    }
    match &segments[1] {
        Segment::Code {
            language,
            code,
            closed,
        } => {
            assert_eq!(language, "vb");
            assert_eq!(code, "For i = 1 To 10\nNext");
            assert!(closed);
        }
        other => panic!("第二段应该是代码块：{other:?}"),
    }
    match &segments[2] {
        Segment::Text(text) => assert!(text.contains("最后一句")),
        other => panic!("第三段应该是说明文字：{other:?}"),
    }
}

#[test]
fn markdown_split_keeps_an_unclosed_fence_open() {
    use crate::ai::markdown::{self, Segment};

    // 流式输出到一半：围栏还没收尾，剩下的都算代码。
    let segments = markdown::split("看代码：\n```rust\nfn main() {\n");
    assert_eq!(segments.len(), 2);
    match &segments[1] {
        Segment::Code {
            language,
            code,
            closed,
        } => {
            assert_eq!(language, "rust");
            assert_eq!(code, "fn main() {");
            assert!(!closed);
        }
        other => panic!("第二段应该是还没收尾的代码块：{other:?}"),
    }
}

#[test]
fn markdown_split_without_fences_is_a_single_text_segment() {
    use crate::ai::markdown::{self, Segment};

    let segments = markdown::split("只有一段说明文字。");
    assert_eq!(segments.len(), 1);
    assert!(matches!(segments[0], Segment::Text(_)));
    assert!(markdown::split("   ").is_empty());
}

#[test]
fn request_body_puts_the_file_context_into_the_system_prompt() {
    use crate::ai::client::file_context;

    let context = file_context("src/main.vb", "Sub Main()\n    MsgBox \"hi\"\nEnd Sub");
    let body = request_body("deepseek-chat", &[], true, Some(&context), &json!([]));
    let value: serde_json::Value = serde_json::from_str(&body).expect("请求体是合法 JSON");
    let messages = value["messages"].as_array().expect("messages 是数组");
    assert_eq!(messages.len(), 1, "上下文不额外占一条消息");
    let system = messages[0]["content"].as_str().expect("system 内容是字符串");
    assert_eq!(messages[0]["role"], "system");
    assert!(system.contains("src/main.vb"));
    assert!(system.contains("Sub Main()"));
    // 密钥绝不会进请求体，只在请求头里。
    assert!(!body.contains("Authorization"));
}

#[test]
fn file_context_truncates_very_long_files() {
    use crate::ai::client::file_context;

    // 每行 10 个字符：截断要停在整行边界上，并写清总长度与后续怎么读。
    let long = "0123456789\n".repeat(5_000);
    let context = file_context("big.vb", &long);
    assert!(context.contains("文件过长"));
    assert!(context.contains("全文共 55000 个字符"));
    assert!(context.contains("read_file"));
    assert!(context.chars().count() < 41_000);
    let head = context
        .split("…（文件过长")
        .next()
        .expect("截断标记之前是文件内容");
    assert!(
        head.ends_with('\n'),
        "截断点要落在整行末尾，不能把一行劈开"
    );
}

// ---------- 工具调用 ----------

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ai::tools::{self, PlanCtx, ToolPlan};

/// 造一个工具调用的参数 JSON。
fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "call_0".to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
    }
}

/// 在内存里跑一次工具调用：`files` 充当项目目录。
fn run_tool(root: &Path, files: &HashMap<PathBuf, String>, call: &ToolCall) -> ToolPlan {
    let read = |path: &Path| files.get(path).cloned();
    let output = || String::from("脚本执行结束（用时 0.01s）");
    let ctx = PlanCtx {
        root,
        read: &read,
        output: &output,
    };
    tools::plan(&ctx, call)
}

fn project() -> (PathBuf, HashMap<PathBuf, String>) {
    let root = PathBuf::from("C:/proj");
    let mut files = HashMap::new();
    files.insert(
        root.join("a.vb"),
        "Dim x As Integer\nx = 1\nMsgBox \"hi\"\n".to_string(),
    );
    (root, files)
}

#[test]
fn tool_read_file_numbers_lines() {
    let (root, files) = project();
    match run_tool(&root, &files, &call("read_file", json!({"path": "a.vb"}))) {
        ToolPlan::Read { content, summary } => {
            assert!(content.contains("1| Dim x As Integer"));
            assert!(content.contains("3| MsgBox \"hi\""));
            assert!(summary.contains("a.vb"));
        }
        _ => panic!("read_file 应当只读"),
    }
}

#[test]
fn tool_replace_in_file_requires_a_unique_snippet() {
    let (root, files) = project();
    // 唯一命中：给出待写入的新内容。
    match run_tool(
        &root,
        &files,
        &call(
            "replace_in_file",
            json!({"path": "a.vb", "old": "MsgBox \"hi\"", "new": "MsgBox \"hello\""}),
        ),
    ) {
        ToolPlan::Write {
            path,
            content,
            summary,
            done,
        } => {
            assert_eq!(path, root.join("a.vb"));
            assert_eq!(content, "Dim x As Integer\nx = 1\nMsgBox \"hello\"\n");
            assert!(summary.contains("替换 1 处"));
            assert!(done.contains("已修改"));
        }
        _ => panic!("replace_in_file 应当给出写入计划"),
    }

    // 找不到原文 / 原文不唯一：都不写盘，把原因回给模型。
    for old in ["不存在的内容", "x"] {
        match run_tool(
            &root,
            &files,
            &call("replace_in_file", json!({"path": "a.vb", "old": old, "new": ""})),
        ) {
            ToolPlan::Read { content, .. } => assert!(content.starts_with("错误：")),
            _ => panic!("有问题的替换不该写盘：{old}"),
        }
    }
}

#[test]
fn tool_paths_cannot_escape_the_project() {
    let (root, files) = project();
    match run_tool(&root, &files, &call("read_file", json!({"path": "../secret.vb"}))) {
        ToolPlan::Read { content, .. } => {
            assert!(content.contains("路径不在项目目录内"), "{content}");
        }
        _ => panic!("越界路径应当被拒绝"),
    }
}

#[test]
fn tool_write_file_plans_a_new_file() {
    let (root, files) = project();
    match run_tool(
        &root,
        &files,
        &call("write_file", json!({"path": "new.vb", "content": "Dim y\n"})),
    ) {
        ToolPlan::Write { path, content, .. } => {
            assert_eq!(path, root.join("new.vb"));
            assert_eq!(content, "Dim y\n");
        }
        _ => panic!("write_file 应当给出写入计划"),
    }
}

#[test]
fn tool_check_syntax_reports_the_broken_line() {
    let (root, mut files) = project();
    files.insert(root.join("bad.vb"), "Dim a = {1, 2\n".to_string());
    match run_tool(&root, &files, &call("check_syntax", json!({"path": "bad.vb"}))) {
        ToolPlan::Read { content, summary } => {
            assert!(content.contains("数组字面量"), "{content}");
            assert!(summary.contains("1 个问题"), "{summary}");
        }
        _ => panic!("check_syntax 应当只读"),
    }
}

#[test]
fn tool_read_output_returns_the_panel_tail() {
    let (root, files) = project();
    match run_tool(&root, &files, &call("read_output", json!({}))) {
        ToolPlan::Read { content, summary } => {
            assert!(content.contains("脚本执行结束"));
            assert!(summary.contains("输出面板"));
        }
        _ => panic!("read_output 应当只读"),
    }
}

#[test]
fn streamed_tool_calls_are_parsed_from_sse() {
    let first = json!({"choices": [{"delta": {"tool_calls": [{
        "index": 0, "id": "call_0", "type": "function",
        "function": {"name": "read_file", "arguments": ""}
    }]}}]});
    let second = json!({"choices": [{"delta": {"tool_calls": [{
        "index": 0, "function": {"arguments": "{\"path\":\"a.vb\"}"}
    }]}}]});

    let mut decoder = SseDecoder::new();
    let events = decoder.push(format!("data: {first}\n\ndata: {second}\n\n").as_bytes());
    assert_eq!(events.len(), 2);

    let SseEvent::ToolCalls(calls) = &events[0] else {
        panic!("第一条应当是工具调用：{:?}", events[0]);
    };
    assert_eq!(calls[0].id.as_deref(), Some("call_0"));
    assert_eq!(calls[0].name.as_deref(), Some("read_file"));

    let SseEvent::ToolCalls(calls) = &events[1] else {
        panic!("第二条应当是参数增量：{:?}", events[1]);
    };
    assert_eq!(calls[0].arguments.as_deref(), Some("{\"path\":\"a.vb\"}"));
}

#[test]
fn request_body_encodes_tools_and_tool_results() {
    let mut assistant = ChatMessage::assistant("");
    assistant.tool_calls = vec![ToolCall {
        id: "call_0".to_string(),
        name: "read_file".to_string(),
        arguments: "{\"path\":\"a.vb\"}".to_string(),
    }];
    let messages = vec![
        ChatMessage::user("看看 a.vb"),
        assistant,
        ChatMessage::tool("call_0", "1| Dim x"),
    ];
    let body = request_body(
        "deepseek-chat",
        &messages,
        true,
        None,
        &tools::tools_json(),
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("请求体是合法 JSON");

    assert_eq!(value["tool_choice"], "auto");
    assert!(value["tools"].as_array().is_some_and(|list| list.len() >= 5));
    let json_messages = value["messages"].as_array().expect("messages 是数组");
    assert_eq!(
        json_messages[2]["tool_calls"][0]["function"]["name"],
        "read_file"
    );
    assert_eq!(json_messages[3]["role"], "tool");
    assert_eq!(json_messages[3]["tool_call_id"], "call_0");
    assert!(!body.contains("Authorization"));
}

#[test]
fn a_short_conversation_is_sent_as_is() {
    let messages = vec![ChatMessage::user("你好"), ChatMessage::assistant("你好！")];
    let (kept, dropped, clipped) = fit_history(&messages, REQUEST_CHAR_BUDGET);
    assert_eq!((dropped, clipped), (0, 0));
    assert_eq!(kept, messages);
}

#[test]
fn a_long_conversation_is_trimmed_and_marked() {
    let mut messages = Vec::new();
    for index in 0..40 {
        messages.push(ChatMessage::user(format!(
            "第 {index} 个问题：{}",
            "很长很长的提问".repeat(200)
        )));
        messages.push(ChatMessage::assistant("很长的回答".repeat(200)));
    }
    messages.push(ChatMessage::user("最后一个问题"));
    messages.push(ChatMessage::assistant(""));

    let (kept, dropped, _clipped) = fit_history(&messages, 20_000);
    assert!(dropped > 0, "超出预算时要丢掉更早的对话");
    assert!(kept.len() < messages.len());
    // 只在用户消息处下刀，工具调用链不会被拆开。
    assert!(matches!(kept[0].role, Role::User));
    assert_eq!(kept.last().expect("至少留一条").content, "");

    // 请求体里要注明省略了多少条，让模型知道自己看到的不完整。
    let body = request_body("deepseek-chat", &messages, true, None, &tools::tools_json());
    let value: serde_json::Value = serde_json::from_str(&body).expect("请求体是合法 JSON");
    let system = value["messages"][0]["content"]
        .as_str()
        .expect("第一条是系统提示");
    assert!(system.contains("已省略"), "系统提示里要有省略标记：{system}");
    // 最新的提问一定还在。
    let sent = value["messages"].as_array().expect("messages 是数组");
    assert!(sent.iter().any(|item| item["content"] == "最后一个问题"));
}

#[test]
fn an_over_long_tool_output_is_clipped_with_a_marker() {
    let messages = vec![
        ChatMessage::user("看看这个文件"),
        ChatMessage::tool("call_0", "x".repeat(60_000)),
    ];
    let (kept, dropped, clipped) = fit_history(&messages, 30_000);
    // 只有一条用户消息，不能再整条丢，只能截断过长的工具输出。
    assert_eq!(dropped, 0);
    assert!(clipped >= 1);
    assert!(kept[1].content.contains("已截断"));
    assert!(kept[1].content.chars().count() < 40_000);

    let body = request_body(
        "deepseek-chat",
        &messages,
        true,
        Some(&"文件内容".repeat(15_000)),
        &tools::tools_json(),
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("请求体是合法 JSON");
    let system = value["messages"][0]["content"].as_str().expect("系统提示");
    assert!(system.contains("已截断"), "系统提示里要有截断标记：{system}");
}

fn temp_rules_dir(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("qide_ai_rules_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建规则临时目录");
    dir
}

#[test]
fn ai_rules_select_common_plus_language_file() {
    let dir = temp_rules_dir("select");
    std::fs::write(dir.join("common.md"), "COMMON").unwrap();
    std::fs::write(dir.join("vb.md"), "VB").unwrap();
    std::fs::write(dir.join("default.md"), "DEFAULT").unwrap();

    let vb = crate::ai::rules::rules_for_language_in(Some(&dir), Some("vb"));
    assert!(vb.contains("COMMON") && vb.contains("VB"), "{vb}");
    assert!(!vb.contains("DEFAULT"), "{vb}");

    let rust = crate::ai::rules::rules_for_language_in(Some(&dir), Some("rust"));
    assert!(rust.contains("COMMON") && rust.contains("DEFAULT"), "{rust}");
    assert!(!rust.contains("VB"), "{rust}");

    let none = crate::ai::rules::rules_for_language_in(Some(&dir), None);
    assert!(none.contains("COMMON") && none.contains("DEFAULT"), "{none}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ai_rules_fall_back_to_the_embedded_vb_rules() {
    let dir = temp_rules_dir("vb-fallback");
    std::fs::write(dir.join("common.md"), "COMMON").unwrap();
    std::fs::write(dir.join("default.md"), "DEFAULT").unwrap();
    let vb = crate::ai::rules::rules_for_language_in(Some(&dir), Some("vb"));
    assert!(vb.contains("COMMON"), "{vb}");
    assert!(vb.contains("VB6 / VBA"), "{vb}");
    assert!(!vb.contains("DEFAULT"), "{vb}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ai_rules_defaults_do_not_overwrite_user_edits() {
    let dir = temp_rules_dir("ensure");
    crate::ai::rules::ensure_defaults_in(&dir).expect("写出默认规则");
    assert!(dir.join("README.md").exists());
    assert!(dir.join("common.md").exists());
    assert!(dir.join("vb.md").exists());
    assert!(dir.join("default.md").exists());

    std::fs::write(dir.join("vb.md"), "MY VB RULES").unwrap();
    crate::ai::rules::ensure_defaults_in(&dir).expect("再次补齐默认规则");
    assert_eq!(
        std::fs::read_to_string(dir.join("vb.md")).unwrap(),
        "MY VB RULES"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn request_body_for_uses_the_language_rules() {
    let body = crate::ai::client::request_body_for(
        Some("vb"),
        "deepseek-chat",
        &[],
        true,
        None,
        &json!([]),
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("请求体是合法 JSON");
    let system = value["messages"][0]["content"].as_str().expect("系统提示");
    assert!(system.contains("VB6 / VBA"), "{system}");
    assert!(system.contains("read_file"), "{system}");

    let body = crate::ai::client::request_body_for(
        Some("rust"),
        "deepseek-chat",
        &[],
        true,
        None,
        &json!([]),
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("请求体是合法 JSON");
    let system = value["messages"][0]["content"].as_str().expect("系统提示");
    assert!(system.contains("非 VB 脚本"), "{system}");
    assert!(system.contains("Rust"), "{system}");
}

#[test]
fn the_reasoner_model_gets_no_tools() {
    let messages = vec![ChatMessage::user("你好")];

    // 对话模型带工具，能改文件。
    assert!(tools::model_supports_tools("deepseek-chat"));
    assert_eq!(tools::tools_for_model("deepseek-chat"), tools::tools_json());
    let body = request_body(
        "deepseek-chat",
        &messages,
        true,
        None,
        &tools::tools_for_model("deepseek-chat"),
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("请求体是合法 JSON");
    assert!(value["tools"].as_array().is_some_and(|list| !list.is_empty()));

    // 推理模型不支持函数调用：不带 tools，也就不会去改文件。
    assert!(!tools::model_supports_tools("deepseek-reasoner"));
    let body = request_body(
        "deepseek-reasoner",
        &messages,
        true,
        None,
        &tools::tools_for_model("deepseek-reasoner"),
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("请求体是合法 JSON");
    assert!(value.get("tools").is_none());
    assert!(value.get("tool_choice").is_none());
}
