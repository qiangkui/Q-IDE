//! qide-mcp：把 MCP stdio（stdin/stdout 的 JSON-RPC）桥接到 Q-IDE。
//!
//! 用法：在外部 agent 的 MCP 配置里把 `command` 指向本程序即可。
//! Q-IDE 启动后会把端口与 token 写进 `%APPDATA%\gpui-first\agent-endpoint.json`；
//! Q-IDE 没在运行时，第一次 `tools/call` 会自动在后台拉起同目录下的 Q-IDE。

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const TOOLS_JSON: &str = include_str!("../../assets/agent-tools.json");
const ENDPOINT_FILE: &str = "agent-endpoint.json";
const LEGACY_DIR: &str = "gpui-first";
const DEFAULT_PROTOCOL: &str = "2024-11-05";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// 自动拉起 Q-IDE 后最多等多久出现端点。
const WAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// 等待期间的重试间隔。
const WAKE_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// 两次自动拉起的间隔，避免 Q-IDE 还在启动时被重复拉起。
const WAKE_DEBOUNCE: Duration = Duration::from_secs(10);
/// 主程序可能的文件名（与本程序同目录）。
const APP_NAMES: &[&str] = &["first.exe", "qide.exe", "Q-IDE.exe"];

/// 最近一次自动拉起的时间；命中防抖窗口就不再重复启动。
static LAST_WAKE: Mutex<Option<Instant>> = Mutex::new(None);

#[derive(Clone, Debug)]
struct Endpoint {
    port: u16,
    token: String,
}

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let message = line.trim();
        if message.is_empty() {
            continue;
        }
        if let Some(response) = respond(message) {
            if writeln!(stdout, "{response}").is_err() {
                break;
            }
            let _ = stdout.flush();
        }
    }
}

fn respond(message: &str) -> Option<String> {
    let value: Value = match serde_json::from_str(message) {
        Ok(value) => value,
        Err(error) => {
            return Some(jsonrpc_error(
                Value::Null,
                -32700,
                format!("JSON 解析失败：{error}"),
            ));
        }
    };
    let method = value.get("method").and_then(Value::as_str)?;
    let id = value.get("id").cloned();
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|version| !version.is_empty())
                .unwrap_or(DEFAULT_PROTOCOL);
            Some(jsonrpc_result(
                id?,
                json!({
                    "protocolVersion": requested,
                    "capabilities": {"tools": {}},
                    "serverInfo": {
                        "name": "qide-mcp",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
            ))
        }
        "notifications/initialized" => None,
        "ping" => id.map(|id| jsonrpc_result(id, json!({}))),
        "tools/list" => id.map(|id| jsonrpc_result(id, json!({"tools": tools_for_mcp()}))),
        "tools/call" => {
            let id = id?;
            Some(tools_call(id, &params))
        }
        "shutdown" => id.map(|id| jsonrpc_result(id, json!({}))),
        "exit" => None,
        other => id.map(|id| jsonrpc_error(id, -32601, format!("不支持的方法：{other}"))),
    }
}

fn tools_for_mcp() -> Vec<Value> {
    serde_json::from_str::<Vec<Value>>(TOOLS_JSON)
        .unwrap_or_default()
        .into_iter()
        .map(|spec| {
            json!({
                "name": spec.get("name").cloned().unwrap_or(Value::Null),
                "description": spec.get("description").cloned().unwrap_or(Value::Null),
                "inputSchema": spec
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object"})),
            })
        })
        .collect()
}

fn tools_call(id: Value, params: &Value) -> String {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return jsonrpc_error(id, -32602, "缺少工具名".to_string());
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match call_qide(name, arguments) {
        Ok(text) => jsonrpc_result(
            id,
            json!({
                "content": [{"type": "text", "text": text}],
                "isError": false,
            }),
        ),
        Err(error) => jsonrpc_result(
            id,
            json!({
                "content": [{"type": "text", "text": error}],
                "isError": true,
            }),
        ),
    }
}

fn call_qide(tool: &str, arguments: Value) -> Result<String, String> {
    match try_call(tool, &arguments) {
        Ok(content) => Ok(content),
        // Q-IDE 明确返回的工具错误直接交给 agent，不再尝试其它端点或拉起新实例。
        Err(BridgeError::Tool(error)) => Err(error),
        // 连不上（没启动、已退出或接口关闭）：后台拉起 Q-IDE 后再重试。
        Err(BridgeError::Connect(error)) => {
            wake_qide()?;
            wait_for_qide(tool, &arguments, &error)
        }
    }
}

fn try_call(tool: &str, arguments: &Value) -> Result<String, BridgeError> {
    let endpoints = read_endpoints();
    if endpoints.is_empty() {
        return Err(BridgeError::Connect(format!(
            "找不到 Q-IDE 的 agent 端点。已查找：{}",
            endpoint_candidates()
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("、")
        )));
    }
    let mut errors = Vec::new();
    for endpoint in endpoints {
        match call_endpoint(&endpoint, tool, arguments.clone()) {
            Ok(content) => return Ok(content),
            Err(BridgeError::Tool(error)) => return Err(BridgeError::Tool(error)),
            Err(BridgeError::Connect(error)) => {
                errors.push(format!("127.0.0.1:{}：{error}", endpoint.port));
            }
        }
    }
    Err(BridgeError::Connect(format!(
        "无法连接 Q-IDE：{}",
        errors.join("；")
    )))
}

/// Q-IDE 没在运行时后台拉起同目录下的主程序。
///
/// `QIDE_APP` 可以指定主程序路径；`QIDE_AUTO_LAUNCH=0` 关闭自动启动。
fn wake_qide() -> Result<(), String> {
    if auto_launch_disabled() {
        return Err("Q-IDE 未运行；QIDE_AUTO_LAUNCH 已关闭自动启动，请手动启动 Q-IDE".to_string());
    }
    let app = app_executable()?;
    // 设置里关掉接口时拉起也没用，直接给出可操作的提示。
    if agent_interface_disabled(&app) {
        return Err(
            "外部 agent 接口在设置里被关闭（layout.txt: ai_agent_enabled=0），不会自动启动 Q-IDE；请在设置窗口「AI → 外部 agent（MCP）」里打开"
                .to_string(),
        );
    }
    // 刚拉起过就只等端点，不再开第二个实例。
    if recently_woken() {
        return Ok(());
    }
    Command::new(&app)
        .arg("--background")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("后台启动 {} 失败：{error}", app.display()))?;
    Ok(())
}

fn wait_for_qide(tool: &str, arguments: &Value, first_error: &str) -> Result<String, String> {
    let started = Instant::now();
    loop {
        std::thread::sleep(WAKE_POLL_INTERVAL);
        match try_call(tool, arguments) {
            Ok(content) => return Ok(content),
            Err(BridgeError::Tool(error)) => return Err(error),
            Err(BridgeError::Connect(error)) => {
                if started.elapsed() >= WAKE_TIMEOUT {
                    return Err(format!(
                        "已尝试后台启动 Q-IDE，但 {} 秒内没有连上。请确认 Q-IDE 能正常启动，且设置「AI → 外部 agent（MCP）」里的「启用外部 agent 接口」是打开的。最初的错误：{first_error}；最后一次：{error}",
                        WAKE_TIMEOUT.as_secs()
                    ));
                }
            }
        }
    }
}

fn auto_launch_disabled() -> bool {
    auto_launch_disabled_value(std::env::var("QIDE_AUTO_LAUNCH").ok().as_deref())
}

fn auto_launch_disabled_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim),
        Some(value)
            if value.eq_ignore_ascii_case("0")
                || value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("off")
                || value.eq_ignore_ascii_case("no")
    )
}

fn recently_woken() -> bool {
    let mut last = match LAST_WAKE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    match *last {
        Some(at) if at.elapsed() < WAKE_DEBOUNCE => true,
        _ => {
            *last = Some(Instant::now());
            false
        }
    }
}

fn app_executable() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("QIDE_APP") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("QIDE_APP 指向的程序不存在：{}", path.display()));
    }
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .ok_or_else(|| "找不到 qide-mcp.exe 所在目录".to_string())?;
    for name in APP_NAMES {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "在 {} 里找不到 Q-IDE 主程序（{}），可以用环境变量 QIDE_APP 指定",
        dir.display(),
        APP_NAMES.join("、")
    ))
}

/// 主程序同目录的 `layout.txt` 里「启用外部 agent 接口」是否被关掉。
fn agent_interface_disabled(app: &Path) -> bool {
    let Some(dir) = app.parent() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(dir.join("layout.txt")) else {
        return false;
    };
    text.lines().any(|line| line.trim() == "ai_agent_enabled=0")
}

enum BridgeError {
    /// 端口连不上、读写失败或响应不合法，可以尝试下一个端点。
    Connect(String),
    /// Q-IDE 正常响应，但工具执行失败。
    Tool(String),
}

fn call_endpoint(endpoint: &Endpoint, tool: &str, arguments: Value) -> Result<String, BridgeError> {
    let address = SocketAddr::from(([127, 0, 0, 1], endpoint.port));
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
        .map_err(|error| BridgeError::Connect(format!("连接失败：{error}")))?;
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));
    let request = json!({
        "token": endpoint.token,
        "id": 1,
        "tool": tool,
        "arguments": arguments,
    });
    writeln!(stream, "{request}")
        .map_err(|error| BridgeError::Connect(format!("发送请求失败：{error}")))?;
    let _ = stream.flush();

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| BridgeError::Connect(format!("读取 Q-IDE 响应失败：{error}")))?;
    let response: Value = serde_json::from_str(line.trim())
        .map_err(|error| BridgeError::Connect(format!("Q-IDE 响应不是合法 JSON：{error}")))?;
    let content = response
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(content)
    } else if content.is_empty() {
        Err(BridgeError::Tool("Q-IDE 返回错误".to_string()))
    } else {
        Err(BridgeError::Tool(content))
    }
}

fn read_endpoints() -> Vec<Endpoint> {
    let mut endpoints = Vec::new();
    for path in endpoint_candidates() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(port) = value
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
        else {
            continue;
        };
        let Some(token) = value.get("token").and_then(Value::as_str) else {
            continue;
        };
        endpoints.push(Endpoint {
            port,
            token: token.to_string(),
        });
    }
    endpoints
}

fn endpoint_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(base) = std::env::var_os("APPDATA").or_else(|| std::env::var_os("LOCALAPPDATA")) {
        paths.push(PathBuf::from(base).join(LEGACY_DIR).join(ENDPOINT_FILE));
    }
    paths.push(std::env::temp_dir().join(format!("{LEGACY_DIR}-{ENDPOINT_FILE}")));
    paths
}

fn jsonrpc_result(id: Value, result: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
    .to_string()
}

fn jsonrpc_error(id: Value, code: i64, message: String) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message},
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{agent_interface_disabled, auto_launch_disabled_value};

    #[test]
    fn auto_launch_can_be_switched_off_by_env() {
        for value in ["0", "false", "FALSE", " off ", "no"] {
            assert!(
                auto_launch_disabled_value(Some(value)),
                "{value} 应当关闭自动启动"
            );
        }
        for value in ["1", "true", "yes", "on", "", "maybe"] {
            assert!(
                !auto_launch_disabled_value(Some(value)),
                "{value} 不应关闭自动启动"
            );
        }
        // 没有配置时默认自动拉起 Q-IDE。
        assert!(!auto_launch_disabled_value(None));
    }

    #[test]
    fn agent_interface_disabled_reads_layout_txt() {
        let dir = std::env::temp_dir().join(format!("qide-mcp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let app = dir.join("first.exe");
        assert!(!agent_interface_disabled(&app), "没有 layout.txt 时按开启处理");

        std::fs::write(dir.join("layout.txt"), "ai_agent_enabled=0\n").expect("写布局文件");
        assert!(agent_interface_disabled(&app));
        std::fs::write(dir.join("layout.txt"), "ai_agent_enabled=1\n").expect("写布局文件");
        assert!(!agent_interface_disabled(&app));

        std::fs::remove_dir_all(&dir).ok();
    }
}

