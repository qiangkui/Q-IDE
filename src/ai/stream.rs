//! SSE（`text/event-stream`）增量解析：把接口推来的字节流拆成一条条事件。
//!
//! 这一层不依赖界面、也不依赖网络，纯粹是「字节 → 事件」，可以单独测试和移植。
//!
//! 网络读到的数据可能在任意位置被切断（半个 JSON、半个 UTF-8 汉字），
//! 所以字节先留在缓冲区里，凑齐一整行（`\n`）才解析。

use serde_json::Value;

/// 模型请求调用一次工具时，流式参数的一个片段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallDelta {
    /// 同一条消息里第几个工具调用（从 0 开始）。
    pub index: usize,
    /// 只在第一片里出现。
    pub id: Option<String>,
    /// 只在第一片里出现。
    pub name: Option<String>,
    /// 参数 JSON 的片段，要按顺序拼接。
    pub arguments: Option<String>,
}

/// 一条 SSE 事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SseEvent {
    /// 一段增量文本。
    Delta(String),
    /// 工具调用参数的一段增量。
    ToolCalls(Vec<ToolCallDelta>),
    /// `data: [DONE]`：服务端说完了。
    Done,
    /// 流里夹带的错误（`{"error":{"message": ...}}`）。
    Error(String),
}

/// 增量解码器：反复喂字节，拿回解析出的事件。
#[derive(Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
    done: bool,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 已经收到过 `[DONE]`。
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// 喂入一段字节，返回这次能解析出来的事件。
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        if self.done {
            return Vec::new();
        }
        self.buffer.extend_from_slice(bytes);

        let mut events = Vec::new();
        while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=index).collect();
            let parsed = parse_line(&line);
            let finished = parsed.iter().any(|event| matches!(event, SseEvent::Done));
            events.extend(parsed);
            if finished {
                self.done = true;
                self.buffer.clear();
                break;
            }
        }
        events
    }

    /// 数据流结束时调用：最后一行可能没有换行符。
    pub fn finish(&mut self) -> Vec<SseEvent> {
        if self.done || self.buffer.is_empty() {
            return Vec::new();
        }
        let line = std::mem::take(&mut self.buffer);
        parse_line(&line)
    }
}

/// 解析一行：只看 `data:` 行，注释、`event:`、空行都返回空列表。
pub fn parse_line(line: &[u8]) -> Vec<SseEvent> {
    let line = String::from_utf8_lossy(line);
    let line = line.trim_end_matches(['\r', '\n']);
    let Some(data) = line.strip_prefix("data:") else {
        return Vec::new();
    };
    let data = data.trim();
    if data.is_empty() {
        return Vec::new();
    }
    if data == "[DONE]" {
        return vec![SseEvent::Done];
    }

    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    if let Some(message) = value.pointer("/error/message").and_then(|v| v.as_str()) {
        return vec![SseEvent::Error(message.to_string())];
    }

    let mut events = Vec::new();
    if let Some(calls) = value
        .pointer("/choices/0/delta/tool_calls")
        .and_then(|v| v.as_array())
    {
        let deltas: Vec<ToolCallDelta> = calls
            .iter()
            .filter_map(|call| {
                let index = call.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let id = call.get("id").and_then(|v| v.as_str()).map(str::to_string);
                let name = call
                    .pointer("/function/name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let arguments = call
                    .pointer("/function/arguments")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if id.is_none() && name.is_none() && arguments.is_none() {
                    return None;
                }
                Some(ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments,
                })
            })
            .collect();
        if !deltas.is_empty() {
            events.push(SseEvent::ToolCalls(deltas));
        }
    }

    if let Some(delta) = value
        .pointer("/choices/0/delta/content")
        .and_then(|v| v.as_str())
        && !delta.is_empty()
    {
        events.push(SseEvent::Delta(delta.to_string()));
    }

    events
}
