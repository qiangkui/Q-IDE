//! 外部 agent 接入：Q-IDE 在本机回环端口上接受请求，`qide-mcp` 负责 MCP stdio 与端口的桥接。
//!
//! 这里只负责传输：请求会被投递到主窗口线程执行，真正的工具逻辑在 `app::agent` 里。
//! 端点信息（端口 + 随机 token）写到 `%APPDATA%\gpui-first\agent-endpoint.json`，
//! 只监听 `127.0.0.1`。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Q-IDE 与 `qide-mcp` 共用的工具清单。
pub const TOOLS_JSON: &str = include_str!("../assets/agent-tools.json");

pub const ENDPOINT_FILE: &str = "agent-endpoint.json";
const LEGACY_DIR: &str = "gpui-first";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// 允许外部 agent 执行的命令：保存、编辑动作以及面板/主题切换。
/// 不包含运行脚本、调试、删除、以及会弹对话框的命令。
pub const SAFE_COMMANDS: &[&str] = &[
    "file.save",
    "file.save-all",
    "edit.comment",
    "edit.move-up",
    "edit.move-down",
    "edit.indent",
    "edit.outdent",
    "edit.select-all",
    "view.explorer",
    "view.outline",
    "view.search",
    "view.ai",
    "view.output",
    "view.status-bar",
    "view.theme",
    "view.fullscreen",
];

/// Q-IDE 暴露给 `qide-mcp` 的端点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
}

impl Endpoint {
    pub fn to_json(&self) -> Value {
        json!({
            "port": self.port,
            "token": self.token,
            "pid": std::process::id(),
        })
    }

    pub fn write(&self) -> std::io::Result<()> {
        let payload = serde_json::to_vec_pretty(&self.to_json())?;
        let mut last_error = None;
        for path in endpoint_candidates() {
            if let Some(parent) = path.parent()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                last_error = Some(error);
                continue;
            }
            match std::fs::write(&path, &payload) {
                Ok(()) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| std::io::Error::other("没有可写的 agent 端点文件位置")))
    }
}

fn endpoint_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(base) = std::env::var_os("APPDATA").or_else(|| std::env::var_os("LOCALAPPDATA")) {
        paths.push(PathBuf::from(base).join(LEGACY_DIR).join(ENDPOINT_FILE));
    }
    paths.push(std::env::temp_dir().join(format!("{LEGACY_DIR}-{ENDPOINT_FILE}")));
    paths
}

/// 一次外部工具调用。
pub struct AgentRequest {
    pub tool: String,
    pub arguments: Value,
    pub reply: SyncSender<AgentReply>,
}

/// 工具执行结果。
#[derive(Clone, Debug)]
pub struct AgentReply {
    pub ok: bool,
    pub content: String,
}

impl AgentReply {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            ok: true,
            content: content.into(),
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            ok: false,
            content: content.into(),
        }
    }
}

/// 启动本地回环服务，返回端点信息供 `qide-mcp` 发现。
pub fn start_server(request_tx: Sender<AgentRequest>) -> std::io::Result<Endpoint> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let token = new_token();
    let endpoint = Endpoint {
        port,
        token: token.clone(),
    };

    std::thread::Builder::new()
        .name("qide-agent".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let tx = request_tx.clone();
                let token = token.clone();
                std::thread::Builder::new()
                    .name("qide-agent-conn".to_string())
                    .spawn(move || handle_connection(stream, tx, &token))
                    .ok();
            }
        })?;

    Ok(endpoint)
}

fn handle_connection(stream: TcpStream, request_tx: Sender<AgentRequest>, token: &str) {
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let response = match serde_json::from_str::<Value>(line.trim()) {
        Ok(message) => handle_message(message, &request_tx, token),
        Err(error) => error_response(Value::Null, format!("请求不是合法 JSON：{error}")),
    };
    let mut writer = stream;
    let _ = writeln!(writer, "{response}");
    let _ = writer.flush();
}

fn handle_message(message: Value, request_tx: &Sender<AgentRequest>, token: &str) -> Value {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    if message.get("token").and_then(Value::as_str) != Some(token) {
        return error_response(id, "token 不匹配".to_string());
    }
    let Some(tool) = message.get("tool").and_then(Value::as_str) else {
        return error_response(id, "缺少 tool".to_string());
    };
    let arguments = message
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    let request = AgentRequest {
        tool: tool.to_string(),
        arguments,
        reply: reply_tx,
    };
    if request_tx.send(request).is_err() {
        return error_response(id, "Q-IDE 主窗口已关闭".to_string());
    }
    match reply_rx.recv_timeout(REQUEST_TIMEOUT) {
        Ok(reply) => json!({
            "id": id,
            "ok": reply.ok,
            "content": reply.content,
        }),
        Err(_) => error_response(id, "Q-IDE 响应超时".to_string()),
    }
}

fn error_response(id: Value, content: String) -> Value {
    json!({
        "id": id,
        "ok": false,
        "content": content,
    })
}

/// 删除端点文件（关闭外部 agent 接口时用）。
pub fn remove_endpoint() {
    for path in endpoint_candidates() {
        let _ = std::fs::remove_file(path);
    }
}

/// 外部工具名是否在共享清单里。
pub fn tool_defined(name: &str) -> bool {
    serde_json::from_str::<Vec<Value>>(TOOLS_JSON)
        .unwrap_or_default()
        .iter()
        .any(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
}

/// 不允许删除/移动任何项目根目录。
pub fn ensure_not_root(path: &Path, roots: &[PathBuf]) -> Result<(), String> {
    if roots.iter().any(|root| path == root) {
        return Err("不能操作项目根目录".to_string());
    }
    Ok(())
}

/// 移动前的路径校验：源存在、目标是目录、目标不在源里面、目标里没有同名条目。
pub fn validate_move(source: &Path, target_dir: &Path) -> Result<(), String> {
    if !source.exists() {
        return Err(format!("源路径不存在：{}", source.display()));
    }
    if !target_dir.is_dir() {
        return Err(format!("目标不是目录：{}", target_dir.display()));
    }
    if source == target_dir || source.parent() == Some(target_dir) {
        return Err("源已经在这个目录里".to_string());
    }
    if source.is_dir() && target_dir.starts_with(source) {
        return Err("不能把文件夹移动到它自己的子目录".to_string());
    }
    let Some(name) = source.file_name() else {
        return Err("源路径没有文件名".to_string());
    };
    let target = target_dir.join(name);
    if target.exists() {
        return Err(format!("目标里已经有 {} 了", name.to_string_lossy()));
    }
    Ok(())
}

fn new_token() -> String {
    let mut hasher = DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    let first = hasher.finish();
    first.rotate_left(21).hash(&mut hasher);
    format!("{first:016x}{:016x}", hasher.finish())
}
