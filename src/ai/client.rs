//! DeepSeek 网络层：地址解析、请求体拼装、应答解析，以及 Windows 下的 WinHTTP 调用。
//!
//! 这一层不依赖任何界面代码，可以整体搬到别的程序里用。
//!
//! 隐私：API Key 只出现在这一次请求的 `Authorization` 头里，不写日志、不进错误信息，
//! 头部缓冲区发完立刻清零；网络调用是阻塞的，必须在后台线程里跑。
//!
//! 流式用法：
//!
//! ```ignore
//! let mut on_event = |item: StreamItem| { /* 追加文本 / 累积工具调用 */ };
//! ask_stream(&request, &mut on_event)?;
//! ```

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use crate::ai::stream::ToolCallDelta;

/// 模型请求调用的一次工具。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 原始 JSON 参数（模型给的是一段字符串）。
    pub arguments: String,
}

/// 对话里的一条消息（不含系统提示）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// `role = assistant` 时，这一轮要调用的工具。
    pub tool_calls: Vec<ToolCall>,
    /// `role = tool` 时，对应哪一次工具调用。
    pub tool_call_id: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self::plain(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain(Role::Assistant, content)
    }

    /// 工具执行结果，回给模型。
    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: call_id.into(),
        }
    }

    fn plain(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: String::new(),
        }
    }
}

/// 消息角色。系统提示由 [`request_body`] 自动补上，不放进对话历史。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// 工具执行结果。
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

/// 流式回调收到的一项内容。
pub enum StreamItem {
    /// 一段增量文本。
    Delta(String),
    /// 工具调用参数的增量。
    ToolCalls(Vec<ToolCallDelta>),
}

/// 系统提示的身份部分；具体语言与工具规则来自程序目录下的 `ai-rules/`。
pub const SYSTEM_PROMPT: &str = "你是 Q-IDE 编辑器内置的编程助手。用简体中文回答，代码要完整，只聊与编程和当前任务相关的内容。";

/// 上下文里最多带多少个字符的当前文件内容。
const CONTEXT_LIMIT: usize = 40_000;

/// 一次请求（系统提示 + 文件上下文 + 对话历史）大致允许多少字符。
///
/// 接口按 token 计长度（DeepSeek 的窗口是 64K token），这里用字符数做保守估算：
/// 中文大约 1 字符 1 token，代码更省，7.2 万字符折算后仍留有余量。
pub const REQUEST_CHAR_BUDGET: usize = 72_000;

/// 截断时单条消息至少保留多少字符，免得把一条消息切得太碎。
const MIN_CLIP_CHARS: usize = 2_000;

/// 把对话历史压进预算里，返回（要发出去的历史, 丢掉的条数, 截断的条数）。
///
/// 先按「整条丢」处理：只在用户消息处下刀，`assistant(tool_calls)` 和它的
/// `tool` 结果不会被拆散；实在还超就把最长的几条消息截一半并标上截断说明。
pub fn fit_history(messages: &[ChatMessage], budget: usize) -> (Vec<ChatMessage>, usize, usize) {
    let total = |list: &[ChatMessage]| list.iter().map(message_chars).sum::<usize>();
    if total(messages) <= budget {
        return (messages.to_vec(), 0, 0);
    }

    let mut start = 0;
    for (index, message) in messages.iter().enumerate() {
        if !matches!(message.role, Role::User) {
            continue;
        }
        start = index;
        if total(&messages[index..]) <= budget {
            break;
        }
    }
    let mut kept: Vec<ChatMessage> = messages[start..].to_vec();
    let dropped = start;

    let mut clipped = 0;
    while total(&kept) > budget {
        let longest = kept
            .iter()
            .enumerate()
            .filter(|(_, message)| message.content.chars().count() > MIN_CLIP_CHARS)
            .max_by_key(|(_, message)| message.content.chars().count())
            .map(|(index, _)| index);
        let Some(index) = longest else { break };
        let count = kept[index].content.chars().count();
        let keep = (count / 2).max(MIN_CLIP_CHARS);
        let head: String = kept[index].content.chars().take(keep).collect();
        kept[index].content = format!("{head}\n…（内容过长，已截断，原文共 {count} 个字符）");
        clipped += 1;
    }

    (kept, dropped, clipped)
}

/// 一条消息按字符数估算有多长（含工具调用的名字与参数）。
fn message_chars(message: &ChatMessage) -> usize {
    message.content.chars().count()
        + message
            .tool_calls
            .iter()
            .map(|call| call.name.chars().count() + call.arguments.chars().count())
            .sum::<usize>()
}

/// 历史被裁剪后，在系统提示末尾说明一句，让模型知道上下文不完整。
fn trim_notice(dropped: usize, clipped: usize) -> Option<String> {
    if dropped == 0 && clipped == 0 {
        return None;
    }
    let mut text = String::from("（提示：为控制请求长度，");
    if dropped > 0 {
        text.push_str(&format!("更早的 {dropped} 条对话已省略"));
        if clipped > 0 {
            text.push('，');
        }
    }
    if clipped > 0 {
        text.push_str(&format!("{clipped} 条过长的内容已截断"));
    }
    text.push_str("；需要原文时用 read_file / read_output 重新读取。）");
    Some(text)
}

/// 把当前打开的文件拼成给模型看的上下文。
///
/// 文件太长时只带前面一部分：在行尾断开（不会把一行代码劈成两半），
/// 并写明总长度与后续怎么读，避免模型以为看到的就是全文。
pub fn file_context(path: &str, content: &str) -> String {
    let mut text = String::from("以下是编辑器当前打开的文件，供你参考：\n");
    text.push_str(path);
    text.push_str("\n\n```\n");
    let total = content.chars().count();
    if total > CONTEXT_LIMIT {
        let head: String = content.chars().take(CONTEXT_LIMIT).collect();
        // 尽量停在整行边界上，实在没有换行才按字符截。
        let cut = head.rfind('\n').map(|index| index + 1).unwrap_or(head.len());
        text.push_str(&head[..cut]);
        let shown = head[..cut].chars().count();
        text.push_str(&format!(
            "\n…（文件过长，只带上了前 {shown} 个字符，全文共 {total} 个字符；\
             需要后面的内容请用 read_file 工具按行范围读取）"
        ));
    } else {
        text.push_str(content);
    }
    text.push_str("\n```");
    text
}

/// 解析后的接口地址。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    /// 请求路径，已经带上 `/chat/completions`。
    pub path: String,
}

/// 一次对话请求的全部参数。
///
/// 故意不实现 `Debug`：里面带着 API Key，避免被顺手打印出去。
pub struct ChatRequest {
    pub endpoint: Endpoint,
    pub model: String,
    pub api_key: String,
    pub messages: Vec<ChatMessage>,
    /// 附在系统提示后面的上下文（当前打开的文件等）。
    pub context: Option<String>,
    /// 当前文件的语言（`vb` / `rust` / `json` / `text` …），用来选择 `ai-rules/` 里的规则。
    pub language: Option<String>,
    /// 发给模型的 `tools` 数组（空数组表示不带工具）。
    pub tools: Value,
    /// 用户点「停止」时置真；网络层在每个数据块之间检查。
    pub cancel: Arc<AtomicBool>,
}

/// 把设置里的「API 地址」解析成主机、端口与请求路径。
///
/// 支持 `https://api.deepseek.com`、`https://api.deepseek.com/v1` 以及带端口的
/// 自建地址（如 `http://127.0.0.1:8080/v1`）。
pub fn endpoint_from(base_url: &str) -> Result<Endpoint, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let (secure, rest) = if let Some(rest) = trimmed.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err("API 地址要以 http:// 或 https:// 开头".to_string());
    };

    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err("API 地址里缺少主机名".to_string());
    }

    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty() && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) =>
        {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("API 地址里的端口不对：{port}"))?;
            (host, port)
        }
        _ => (authority, if secure { 443 } else { 80 }),
    };

    Ok(Endpoint {
        secure,
        host: host.to_string(),
        port,
        path: format!("{path}/chat/completions"),
    })
}

/// 拼出 OpenAI 兼容的请求体：系统提示（可带上下文）+ 完整对话历史 + 工具清单。
///
/// `stream` 为真时开启 SSE 流式返回（接口会一段段地推增量文本）。
/// 历史太长（对话越积越多、工具输出又大）时会先裁剪，并在系统提示里注明，
/// 免得请求超出模型的上下文窗口被接口直接拒绝。
#[allow(dead_code)] // 测试与没有语言信息的调用方使用；界面统一走 `request_body_for`。
pub fn request_body(
    model: &str,
    messages: &[ChatMessage],
    stream: bool,
    context: Option<&str>,
    tools: &Value,
) -> String {
    request_body_for(None, model, messages, stream, context, tools)
}

/// 同 [`request_body`]，额外按当前文件语言加载 `ai-rules/` 里的固定规则。
pub fn request_body_for(
    language: Option<&str>,
    model: &str,
    messages: &[ChatMessage],
    stream: bool,
    context: Option<&str>,
    tools: &Value,
) -> String {
    let mut parts = vec![SYSTEM_PROMPT.to_string()];
    let rules = crate::ai::rules::rules_for_language(language);
    if !rules.trim().is_empty() {
        parts.push(rules);
    }
    if let Some(context) = context.filter(|context| !context.trim().is_empty()) {
        parts.push(context.to_string());
    }
    let mut system = parts.join("\n\n");
    // 系统提示与文件上下文先占掉预算，剩下的才是对话历史能用的长度。
    let budget = REQUEST_CHAR_BUDGET.saturating_sub(system.chars().count());
    let (history, dropped, clipped) = fit_history(messages, budget);
    if let Some(notice) = trim_notice(dropped, clipped) {
        system.push_str("\n\n");
        system.push_str(&notice);
    }

    let mut all = Vec::with_capacity(history.len() + 1);
    all.push(json!({"role": "system", "content": system}));
    for message in &history {
        all.push(message_json(message));
    }

    let mut body = json!({"model": model, "messages": all, "stream": stream});
    if tools.as_array().is_some_and(|items| !items.is_empty()) {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("auto");
    }
    body.to_string()
}

/// 一条消息按 OpenAI 的格式序列化：工具调用与工具结果各有自己的字段。
fn message_json(message: &ChatMessage) -> Value {
    match message.role {
        Role::Tool => json!({
            "role": "tool",
            "tool_call_id": message.tool_call_id,
            "content": message.content,
        }),
        Role::Assistant if !message.tool_calls.is_empty() => json!({
            "role": "assistant",
            "content": message.content,
            "tool_calls": message
                .tool_calls
                .iter()
                .map(|call| json!({
                    "id": call.id,
                    "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments},
                }))
                .collect::<Vec<_>>(),
        }),
        _ => json!({"role": message.role.as_str(), "content": message.content}),
    }
}

/// 解析非流式应答（也用于流式请求失败时读回来的错误体）。
///
/// 只保留消息文本，绝不回显请求头，避免把密钥带进界面或日志。
pub fn parse_reply(status: u16, body: &str) -> Result<String, String> {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return Err(format!("接口返回的内容不是合法 JSON（HTTP {status}）")),
    };

    if let Some(message) = value.pointer("/error/message").and_then(|v| v.as_str()) {
        return Err(format!("{message}（HTTP {status}）"));
    }
    if !(200..300).contains(&status) {
        return Err(format!("请求失败（HTTP {status}）"));
    }

    let content = value
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if content.is_empty() {
        return Err("接口没有返回任何内容".to_string());
    }
    Ok(content)
}

/// 发送一次流式对话请求（阻塞）。
///
/// 文本增量与工具调用增量都通过 `on_event` 回调；全部收完返回 `Ok(())`。
/// 必须在后台线程调用：`on_event` 也在那个线程里执行。
pub fn ask_stream(
    request: &ChatRequest,
    on_event: &mut dyn FnMut(StreamItem),
) -> Result<(), String> {
    http::post_stream(request, on_event)
}

#[cfg(target_os = "windows")]
mod http {
    use std::ffi::c_void;
    use std::io;
    use std::sync::atomic::Ordering;

    use super::ChatRequest;
    use crate::ai::stream::{SseDecoder, SseEvent};

    /// 用系统代理设置（没有代理时直连）。
    const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
    /// 走 HTTPS。
    const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
    const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
    const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

    fn cancelled(request: &ChatRequest) -> bool {
        request.cancel.load(Ordering::Relaxed)
    }

    #[link(name = "winhttp")]
    unsafe extern "system" {
        fn WinHttpOpen(
            agent: *const u16,
            access_type: u32,
            proxy: *const u16,
            proxy_bypass: *const u16,
            flags: u32,
        ) -> *mut c_void;
        fn WinHttpConnect(
            session: *mut c_void,
            server: *const u16,
            port: u16,
            reserved: u32,
        ) -> *mut c_void;
        fn WinHttpOpenRequest(
            connect: *mut c_void,
            verb: *const u16,
            object: *const u16,
            version: *const u16,
            referrer: *const u16,
            accept_types: *const *const u16,
            flags: u32,
        ) -> *mut c_void;
        fn WinHttpSendRequest(
            request: *mut c_void,
            headers: *const u16,
            headers_len: u32,
            optional: *mut c_void,
            optional_len: u32,
            total_len: u32,
            context: usize,
        ) -> i32;
        fn WinHttpReceiveResponse(request: *mut c_void, reserved: *mut c_void) -> i32;
        fn WinHttpQueryHeaders(
            request: *mut c_void,
            info_level: u32,
            name: *const u16,
            buffer: *mut c_void,
            buffer_len: *mut u32,
            index: *mut u32,
        ) -> i32;
        fn WinHttpReadData(
            request: *mut c_void,
            buffer: *mut c_void,
            to_read: u32,
            read: *mut u32,
        ) -> i32;
        fn WinHttpSetTimeouts(
            handle: *mut c_void,
            resolve: i32,
            connect: i32,
            send: i32,
            receive: i32,
        ) -> i32;
        fn WinHttpCloseHandle(handle: *mut c_void) -> i32;
    }

    /// WinHTTP 句柄的 RAII 包装：离开作用域就关掉。
    struct Handle(*mut c_void);

    impl Handle {
        fn new(handle: *mut c_void) -> Self {
            Self(handle)
        }

        fn raw(&self) -> *mut c_void {
            self.0
        }

        fn is_null(&self) -> bool {
            self.0.is_null()
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: 句柄由 WinHTTP 创建，且只在这里关闭一次。
                let _ = unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 把系统错误码翻成一句能看懂的提示（不包含任何请求内容）。
    fn sys_error(context: &str) -> String {
        let code = io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let hint = match code {
            12002 => "，请求超时",
            12007 => "，域名解析失败",
            12029 | 12030 => "，无法连接到服务器",
            12175 => "，HTTPS 证书校验失败",
            _ => "",
        };
        format!("{context}{hint}（Windows 错误码 {code}）")
    }

    /// 读完整个响应体（错误应答用）。
    fn read_all(handle: *mut c_void) -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let mut read = 0u32;
            let ok = unsafe {
                WinHttpReadData(
                    handle,
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer.len() as u32,
                    &mut read,
                )
            };
            if ok == 0 {
                return Err(sys_error("读取响应内容失败"));
            }
            if read == 0 {
                return Ok(body);
            }
            body.extend_from_slice(&buffer[..read as usize]);
        }
    }

    pub(super) fn post_stream(
        request: &ChatRequest,
        on_event: &mut dyn FnMut(super::StreamItem),
    ) -> Result<(), String> {
        if cancelled(request) {
            return Ok(());
        }
        let agent = wide("Q-IDE");
        let session = Handle::new(unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            )
        });
        if session.is_null() {
            return Err(sys_error("创建网络会话失败"));
        }
        // 单段等不到数据的上限；流式时每来一段都会重新计时。
        unsafe { WinHttpSetTimeouts(session.raw(), 15_000, 15_000, 60_000, 120_000) };

        let host = wide(&request.endpoint.host);
        let connect = Handle::new(unsafe {
            WinHttpConnect(session.raw(), host.as_ptr(), request.endpoint.port, 0)
        });
        if connect.is_null() {
            return Err(sys_error("连接服务器失败"));
        }

        let verb = wide("POST");
        let path = wide(&request.endpoint.path);
        let flags = if request.endpoint.secure {
            WINHTTP_FLAG_SECURE
        } else {
            0
        };
        let handle = Handle::new(unsafe {
            WinHttpOpenRequest(
                connect.raw(),
                verb.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                flags,
            )
        });
        if handle.is_null() {
            return Err(sys_error("创建请求失败"));
        }

        let body = super::request_body_for(
            request.language.as_deref(),
            &request.model,
            &request.messages,
            true,
            request.context.as_deref(),
            &request.tools,
        )
        .into_bytes();
        let mut headers = wide(&format!(
            "Content-Type: application/json\r\nAccept: text/event-stream\r\nAuthorization: Bearer {}\r\n",
            request.api_key
        ));
        let headers_len = (headers.len() - 1) as u32;
        if cancelled(request) {
            headers.fill(0);
            return Ok(());
        }
        let sent = unsafe {
            WinHttpSendRequest(
                handle.raw(),
                headers.as_ptr(),
                headers_len,
                body.as_ptr() as *mut c_void,
                body.len() as u32,
                body.len() as u32,
                0,
            )
        };
        // 这份副本里有密钥，发完立刻抹掉。
        headers.fill(0);
        if sent == 0 {
            return Err(sys_error("发送请求失败"));
        }

        if cancelled(request) {
            return Ok(());
        }
        if unsafe { WinHttpReceiveResponse(handle.raw(), std::ptr::null_mut()) } == 0 {
            return Err(sys_error("等待服务器响应失败"));
        }

        let mut status: u32 = 0;
        let mut status_len = std::mem::size_of::<u32>() as u32;
        let queried = unsafe {
            WinHttpQueryHeaders(
                handle.raw(),
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                &mut status as *mut u32 as *mut c_void,
                &mut status_len,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(sys_error("读取响应状态失败"));
        }

        let status = status as u16;
        if !(200..300).contains(&status) {
            // 失败时接口回的是普通 JSON，整段读回来再解析错误说明。
            let body = read_all(handle.raw())?;
            return Err(super::parse_reply(
                status,
                &String::from_utf8_lossy(&body),
            )
            .err()
            .unwrap_or_else(|| format!("请求失败（HTTP {status}）")));
        }

        let mut decoder = SseDecoder::new();
        let mut buffer = [0u8; 8192];
        'read: loop {
            if cancelled(request) {
                return Ok(());
            }
            let mut read = 0u32;
            let ok = unsafe {
                WinHttpReadData(
                    handle.raw(),
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer.len() as u32,
                    &mut read,
                )
            };
            if ok == 0 {
                return Err(sys_error("读取响应内容失败"));
            }
            if read == 0 {
                break;
            }
            if cancelled(request) {
                return Ok(());
            }
            for event in decoder.push(&buffer[..read as usize]) {
                match event {
                    SseEvent::Delta(text) => on_event(super::StreamItem::Delta(text)),
                    SseEvent::ToolCalls(calls) => {
                        on_event(super::StreamItem::ToolCalls(calls))
                    }
                    SseEvent::Done => break 'read,
                    SseEvent::Error(message) => return Err(message),
                }
            }
        }

        if cancelled(request) {
            return Ok(());
        }
        // 连接提前断掉时，最后一行可能没有换行符，补解析一次。
        if !decoder.is_done() {
            for event in decoder.finish() {
                match event {
                    SseEvent::Delta(text) => on_event(super::StreamItem::Delta(text)),
                    SseEvent::ToolCalls(calls) => {
                        on_event(super::StreamItem::ToolCalls(calls))
                    }
                    SseEvent::Done | SseEvent::Error(_) => break,
                }
            }
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod http {
    use super::ChatRequest;

    pub(super) fn post_stream(
        _request: &ChatRequest,
        _on_event: &mut dyn FnMut(super::StreamItem),
    ) -> Result<(), String> {
        Err("当前平台暂不支持调用 DeepSeek 接口".to_string())
    }
}
