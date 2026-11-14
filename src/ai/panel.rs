//! AI 编程面板：默认停靠在窗口右侧的对话面板，也可以拖到左侧。
//!
//! 这一层只负责界面（消息气泡、工具活动、写入确认、输入、滚动、按钮）；网络、协议与
//! 工具层分别在 [`crate::ai::client`]、[`crate::ai::stream`]、[`crate::ai::tools`] 里，
//! 移植到别的程序时只要重写这一层。
//!
//! 一轮对话的流程：
//!
//! ```text
//! send() ──► run_turn()  ──流式──► finish_turn()
//!                                     │
//!               ┌─────────────────────┴─────────────────────┐
//!               │ 没有工具调用                              │ 有工具调用
//!               ▼                                           ▼
//!            结束                                    process_tools()
//!                                                   ├ 只读 → 结果回给模型
//!                                                   └ 写文件 → 等用户确认
//!                                                              │
//!                                                   ┌──────────┴──────────┐
//!                                                   ▼                     ▼
//!                                               同意 → 写盘           拒绝 → 告诉模型
//!                                                   └──────────┬──────────┘
//!                                                              ▼
//!                                                         run_turn() 再来一轮
//! ```

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::dock::{
    BasePanel, DragPanel, Panel as DockPanel, PanelEvent, PanelId, TabGroup,
};
use gpui_kit::component::input::{Enter, Escape, Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::*;
use gpui_kit::prelude::{FluentBuilder as _, InteractiveElement as _};
use gpui_kit::*;

use crate::ai::client::{self, ChatMessage, ChatRequest, Role, StreamItem, ToolCall};
use crate::ai::markdown::{self, Segment};
use crate::ai::stream::ToolCallDelta;
use crate::ai::tools;
use crate::app::CodeWindow;
use crate::outline::{ClosePanelAction, DragStartAction};
use crate::panels::AI_PANEL;
use crate::secret_store;
use crate::settings::{AI_MODEL_OPTIONS, AiApproval, AiRoundLimit, SettingsStore};
use crate::theme::{self as ui_theme, Palette};

/// 代码块折叠时先显示多少行。
const CODE_PREVIEW_LINES: usize = 5;

/// 后台线程送回来的流式事件。
enum StreamEvent {
    /// 一段增量文本。
    Delta(String),
    /// 工具调用参数的一段增量。
    ToolCalls(Vec<ToolCallDelta>),
    /// 全部收完。
    Done,
    /// 失败原因。
    Failed(String),
}

/// 按 `index` 累积流式工具调用（参数会被切成很多片段）。
#[derive(Default)]
struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccum {
    fn finish(self, index: usize) -> ToolCall {
        ToolCall {
            id: if self.id.is_empty() {
                format!("call_{index}")
            } else {
                self.id
            },
            name: self.name,
            arguments: self.arguments,
        }
    }
}

/// 对话里的一项：消息本体 + 界面上的一行补充说明（工具结果用）。
struct ChatEntry {
    message: ChatMessage,
    note: Option<String>,
}

impl ChatEntry {
    fn new(message: ChatMessage) -> Self {
        Self {
            message,
            note: None,
        }
    }

    fn tool(call_id: &str, content: String, note: String) -> Self {
        Self {
            message: ChatMessage::tool(call_id, content),
            note: Some(note),
        }
    }
}

/// 等用户确认的写文件操作。
struct PendingWrite {
    call_id: String,
    summary: String,
    path: PathBuf,
    content: String,
    done: String,
}

/// 等用户确认的脚本/调试操作。
struct PendingAction {
    call_id: String,
    summary: String,
    action: tools::AppAction,
}

/// 右侧 AI 编程面板。
pub struct AiPanel {
    app: WeakEntity<CodeWindow>,
    /// 对话输入框。
    input: Entity<InputState>,
    group: Option<WeakEntity<TabGroup>>,
    focus: FocusHandle,
    /// 对话历史，发给接口的就是这一串；最后一条可能就是正在生成的助手消息。
    messages: Vec<ChatEntry>,
    /// 正在等接口返回。
    pending: bool,
    /// 最近一次失败的原因（配置缺失、网络错误、接口报错……）。
    error: Option<String>,
    /// 请求序号：清空对话后，旧请求的回复会被丢弃。
    request_id: u64,
    /// 消息区的滚动位置。
    scroll: ScrollHandle,
    /// 哪个代码块被展开成了完整内容：(消息下标, 段落下标)。
    expanded_code: HashSet<(usize, usize)>,
    /// 等用户确认的写文件操作。
    approval: Option<PendingWrite>,
    /// 等用户确认的脚本执行操作。
    action_approval: Option<PendingAction>,
    /// 同一轮里还没执行的工具调用。
    tool_queue: VecDeque<ToolCall>,
    /// 这次提问已经连续跑了多少轮工具，用来防死循环。
    tool_rounds: usize,
    /// 当前请求的停止标志；用户点「停止」时置真。
    cancel: Option<Arc<AtomicBool>>,
    /// 不是错误的提示（如「已停止」「达到轮数上限」），用淡色显示。
    notice: Option<String>,
    /// 设置变化（底栏切换交互方式、轮数或模型）后重绘。
    _settings_subscription: Subscription,
}

impl AiPanel {
    pub fn new(app: WeakEntity<CodeWindow>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("向 AI 描述你的需求"));
        let settings_subscription = cx.observe(&SettingsStore::entity(cx), |_, _, cx| cx.notify());
        Self {
            app,
            input,
            group: None,
            focus: cx.focus_handle(),
            messages: Vec::new(),
            pending: false,
            error: None,
            request_id: 0,
            scroll: ScrollHandle::new(),
            expanded_code: HashSet::new(),
            approval: None,
            action_approval: None,
            tool_queue: VecDeque::new(),
            tool_rounds: 0,
            cancel: None,
            notice: None,
            _settings_subscription: settings_subscription,
        }
    }

    /// 把焦点交给对话输入框（活动栏点开 AI 面板时用）。
    pub fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
        // 面板刚加进 dock 时输入框还没上树，下一帧再确认一次焦点。
        let input = self.input.clone();
        window.on_next_frame(move |window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx));
        });
    }

    /// 把输入框里的内容发给 DeepSeek。
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.approval.is_some() || self.action_approval.is_some() {
            return;
        }
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.connection(cx).is_none() {
            cx.notify();
            return;
        }

        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        // 发完继续留在输入框里，方便接着问下一句。
        self.input.update(cx, |input, cx| input.focus(window, cx));
        self.messages.push(ChatEntry::new(ChatMessage::user(text)));
        self.error = None;
        self.notice = None;
        self.cancel_current();
        self.tool_rounds = 0;
        self.run_turn(window, cx);
    }

    /// 让当前请求的停止标志生效（如果有）。
    fn cancel_current(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// 检查 API 地址与密钥；不完整时把原因写进 `error`。
    fn connection(&mut self, cx: &Context<Self>) -> Option<(client::Endpoint, String, String)> {
        let settings = SettingsStore::snapshot(cx);
        let endpoint = match client::endpoint_from(settings.ai.effective_base_url()) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                self.error = Some(error);
                return None;
            }
        };
        let Some(api_key) = secret_store::load_api_key() else {
            self.error =
                Some("还没配置 API 密钥：请到「设置 → AI」里填写 DeepSeek 的 API 密钥".to_string());
            return None;
        };
        Some((
            endpoint,
            settings.ai.effective_model().to_string(),
            api_key,
        ))
    }

    /// 发一轮请求：先占一条空的助手消息，流式文本往里追加。
    fn run_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let round_limit = SettingsStore::snapshot(cx).ai.round_limit;
        if round_limit.reached(self.tool_rounds) {
            self.pending = false;
            self.cancel_current();
            self.notice = Some(format!(
                "已达到 {} 工具调用上限，已停止。可在底栏切换轮数、选择「一直执行」，或直接发送消息继续。",
                round_limit.short_label()
            ));
            self.scroll.scroll_to_bottom();
            cx.notify();
            return;
        }
        let Some((endpoint, model, api_key)) = self.connection(cx) else {
            self.pending = false;
            cx.notify();
            return;
        };

        self.messages
            .push(ChatEntry::new(ChatMessage::assistant("")));
        self.pending = true;
        self.request_id += 1;
        let request_id = self.request_id;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());

        // 把当前打开的文件带上，模型才能针对真实代码给出修改。
        let context = self
            .app
            .upgrade()
            .and_then(|app| app.read_with(cx, |this, _| this.ai_file_context()))
            .map(|(path, content)| client::file_context(&path, &content));
        // 当前语言决定加载 ai-rules/ 里的哪份固定规则（vb.md / rust.md / default.md）。
        let language = self
            .app
            .upgrade()
            .map(|app| app.read_with(cx, |this, _| this.ai_language()));
        let messages: Vec<ChatMessage> = self.messages[..self.messages.len() - 1]
            .iter()
            .map(|entry| entry.message.clone())
            .collect();
        // 推理模型不支持函数调用，这种时候不带 tools，只当普通聊天。
        let tool_list = tools::tools_for_model(&model);
        let request = ChatRequest {
            endpoint,
            model,
            api_key,
            messages,
            context,
            language,
            tools: tool_list,
            cancel,
        };
        self.scroll.scroll_to_bottom();

        // 网络请求会阻塞，放到后台线程；内容和工具调用都用 mpsc 搬回界面线程。
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let events = sender.clone();
            let mut on_event = |item: StreamItem| {
                let event = match item {
                    StreamItem::Delta(text) => StreamEvent::Delta(text),
                    StreamItem::ToolCalls(calls) => StreamEvent::ToolCalls(calls),
                };
                let _ = events.send(event);
            };
            match client::ask_stream(&request, &mut on_event) {
                Ok(()) => {
                    let _ = sender.send(StreamEvent::Done);
                }
                Err(error) => {
                    let _ = sender.send(StreamEvent::Failed(error));
                }
            }
        });

        cx.spawn_in(window, async move |this, cx| {
            let mut buffered = String::new();
            let mut calls: BTreeMap<usize, ToolCallAccum> = BTreeMap::new();
            loop {
                let mut outcome: Option<Result<(), String>> = None;
                loop {
                    match receiver.try_recv() {
                        Ok(StreamEvent::Delta(text)) => buffered.push_str(&text),
                        Ok(StreamEvent::ToolCalls(deltas)) => {
                            for delta in deltas {
                                let slot = calls.entry(delta.index).or_default();
                                if let Some(id) = delta.id {
                                    slot.id = id;
                                }
                                if let Some(name) = delta.name {
                                    slot.name = name;
                                }
                                if let Some(arguments) = delta.arguments {
                                    slot.arguments.push_str(&arguments);
                                }
                            }
                        }
                        Ok(StreamEvent::Done) => {
                            outcome = Some(Ok(()));
                            break;
                        }
                        Ok(StreamEvent::Failed(error)) => {
                            outcome = Some(Err(error));
                            break;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            outcome = Some(Err("请求被中断，没有收到回复".to_string()));
                            break;
                        }
                    }
                }

                let finished = outcome.is_some();
                if !buffered.is_empty() || finished {
                    let chunk = std::mem::take(&mut buffered);
                    if finished {
                        let calls: Vec<ToolCall> = std::mem::take(&mut calls)
                            .into_iter()
                            .map(|(index, accum)| accum.finish(index))
                            .collect();
                        let outcome = outcome.expect("finished 时一定有结果");
                        let _ = this.update_in(cx, |panel, window, cx| {
                            // 期间点过「清空」的话，这次回复已经作废。
                            if panel.request_id != request_id {
                                return;
                            }
                            if !chunk.is_empty() {
                                panel.append_to_reply(&chunk);
                            }
                            panel.finish_turn(outcome, calls, window, cx);
                        });
                        return;
                    }
                    let _ = this.update_in(cx, |panel, _window, cx| {
                        if panel.request_id != request_id {
                            return;
                        }
                        panel.append_to_reply(&chunk);
                        panel.scroll.scroll_to_bottom();
                        cx.notify();
                    });
                }

                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
            }
        })
        .detach();
    }

    /// 把一段增量文本追加到正在生成的助手消息后面。
    fn append_to_reply(&mut self, text: &str) {
        if let Some(entry) = self.messages.last_mut()
            && matches!(entry.message.role, Role::Assistant)
        {
            entry.message.content.push_str(text);
        }
    }

    /// 一轮结束：收起「正在思考」，把工具调用排进队列。
    fn finish_turn(
        &mut self,
        outcome: Result<(), String>,
        calls: Vec<ToolCall>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = false;
        self.cancel = None;

        if let Err(error) = outcome {
            // 一条字都没收到就把空消息去掉，只留错误说明。
            self.drop_empty_reply();
            self.error = Some(error);
            cx.notify();
            return;
        }

        if calls.is_empty() {
            if self.drop_empty_reply() {
                self.error = Some("接口没有返回任何内容".to_string());
            }
            self.scroll.scroll_to_bottom();
            cx.notify();
            return;
        }

        if let Some(entry) = self.messages.last_mut() {
            entry.message.tool_calls = calls.clone();
        }
        self.tool_rounds += 1;
        self.tool_queue.extend(calls);
        self.process_tools(window, cx);
    }

    /// 最后一条助手消息是空的就删掉；返回是否删了。
    fn drop_empty_reply(&mut self) -> bool {
        let empty = self.messages.last().is_some_and(|entry| {
            matches!(entry.message.role, Role::Assistant)
                && entry.message.content.trim().is_empty()
                && entry.message.tool_calls.is_empty()
        });
        if empty {
            self.messages.pop();
        }
        empty
    }

    /// 依次执行工具：只读的直接回给模型，写文件的停下来等用户确认。
    fn process_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(call) = self.tool_queue.pop_front() {
            let Some(app) = self.app.upgrade() else {
                self.tool_queue.clear();
                self.error = Some("主窗口已关闭，工具调用被取消".to_string());
                cx.notify();
                return;
            };
            let plan = app.read_with(cx, |this, cx| {
                let root = this.ai_project_root().unwrap_or_default();
                let read = |path: &Path| this.file_content(path);
                let output = || this.ai_output_tail(cx);
                let debug_state = || this.ai_debug_state();
                let ctx = tools::PlanCtx {
                    root: &root,
                    read: &read,
                    output: &output,
                    debug_state: &debug_state,
                };
                tools::plan(&ctx, &call)
            });

            match plan {
                tools::ToolPlan::Read { content, summary } => {
                    self.push_tool_result(&call.id, content, summary);
                }
                tools::ToolPlan::Write {
                    path,
                    content,
                    summary,
                    done,
                } => {
                    let pending = PendingWrite {
                        call_id: call.id.clone(),
                        summary,
                        path,
                        content,
                        done,
                    };
                    if SettingsStore::snapshot(cx).ai.approval == AiApproval::Auto {
                        // 写盘要跳出面板的 update，见 `spawn_write`。
                        self.scroll.scroll_to_bottom();
                        cx.notify();
                        self.spawn_write(pending, window, cx);
                        return;
                    }
                    self.approval = Some(pending);
                    self.scroll.scroll_to_bottom();
                    cx.notify();
                    return;
                }
                tools::ToolPlan::Action { action, summary } => {
                    if action.requires_confirmation()
                        && SettingsStore::snapshot(cx).ai.approval == AiApproval::Ask
                    {
                        self.action_approval = Some(PendingAction {
                            call_id: call.id.clone(),
                            summary,
                            action,
                        });
                        self.scroll.scroll_to_bottom();
                        cx.notify();
                        return;
                    }
                    self.spawn_action(call.id, summary, action, window, cx);
                    return;
                }
            }
        }

        // 工具都执行完了：带着结果再问一轮。
        self.run_turn(window, cx);
    }

    /// 把待写入的内容交给主窗口落盘。
    ///
    /// 一定要放到新的任务里执行：`write_ai_file` 打开新文件时会触发 `save_layout`，
    /// 而保存布局要去读 Dock 里的每个面板（包括 AI 面板本身）。如果在 AI 面板自己的
    /// `update` 里直接写，GPUI 会因为「实体正在更新中又被读取」直接 panic。
    fn spawn_write(&mut self, pending: PendingWrite, window: &mut Window, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let request_id = self.request_id;
        // 写盘期间算作「忙」，避免用户再发一条消息打断消息顺序。
        self.pending = true;
        let PendingWrite {
            call_id,
            summary,
            path,
            content,
            done,
        } = pending;
        cx.spawn_in(window, async move |this, cx| {
            let result = app.update_in(cx, |this, window, cx| {
                this.write_ai_file(&path, &content, window, cx)
            });
            let text = match result {
                Ok(Ok(())) => done,
                Ok(Err(error)) => format!("写入失败：{error}"),
                Err(_) => "主窗口已关闭，写入被取消".to_string(),
            };
            this.update_in(cx, |panel, window, cx| {
                // 期间点过「新建对话 / 清空」的话，这次工具结果已经没用了。
                if panel.request_id != request_id {
                    cx.notify();
                    return;
                }
                panel.pending = false;
                panel.push_tool_result(&call_id, text, summary);
                cx.notify();
                panel.process_tools(window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// 在主窗口实体更新之外执行脚本/调试操作，避免与 AI 面板形成重入更新。
    fn spawn_action(
        &mut self,
        call_id: String,
        summary: String,
        action: tools::AppAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        let request_id = self.request_id;
        self.pending = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = app.update_in(cx, |app, window, cx| {
                app.ai_execute_action(action, window, cx)
            });
            let text = match result {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => format!("操作失败：{error}"),
                Err(_) => "主窗口已关闭，操作被取消".to_string(),
            };
            this.update_in(cx, |panel, window, cx| {
                if panel.request_id != request_id {
                    cx.notify();
                    return;
                }
                panel.pending = false;
                panel.push_tool_result(&call_id, text, summary);
                cx.notify();
                panel.process_tools(window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn push_tool_result(&mut self, call_id: &str, content: String, note: String) {
        self.messages.push(ChatEntry::tool(call_id, content, note));
        self.scroll.scroll_to_bottom();
    }

    /// 同意写入：落盘后把结果告诉模型，继续后面的工具。
    fn approve(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pending) = self.action_approval.take() {
            cx.notify();
            self.spawn_action(pending.call_id, pending.summary, pending.action, window, cx);
            return;
        }
        let Some(pending) = self.approval.take() else {
            return;
        };
        cx.notify();
        self.spawn_write(pending, window, cx);
    }

    /// 拒绝写入：告诉模型文件没动，让它换个办法或结束。
    fn reject(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pending) = self.action_approval.take() {
            self.push_tool_result(
                &pending.call_id,
                "用户拒绝了这次脚本/调试操作，未执行。".to_string(),
                format!("已拒绝：{}", pending.summary),
            );
            cx.notify();
            self.process_tools(window, cx);
            return;
        }
        let Some(pending) = self.approval.take() else {
            return;
        };
        self.push_tool_result(
            &pending.call_id,
            "用户拒绝了这次写入，文件没有改动。".to_string(),
            format!("已拒绝：{}", pending.summary),
        );
        cx.notify();
        self.process_tools(window, cx);
    }

    /// 停止当前请求：网络请求、工具循环和待确认的写入都取消。
    fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.pending
            && self.approval.is_none()
            && self.action_approval.is_none()
            && self.cancel.is_none()
        {
            return;
        }
        self.cancel_current();
        self.request_id += 1;
        self.pending = false;
        self.approval = None;
        self.action_approval = None;
        self.tool_queue.clear();
        self.tool_rounds = 0;
        self.close_unfinished_tool_calls();
        self.drop_empty_reply();
        self.notice = Some("已停止".to_string());
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// 停止时给还没拿到结果的工具调用补一条结果，保证历史符合接口协议。
    fn close_unfinished_tool_calls(&mut self) {
        let Some(index) = self.messages.iter().rposition(|entry| {
            matches!(entry.message.role, Role::Assistant) && !entry.message.tool_calls.is_empty()
        }) else {
            return;
        };
        let answered: HashSet<String> = self.messages[index + 1..]
            .iter()
            .filter(|entry| matches!(entry.message.role, Role::Tool))
            .map(|entry| entry.message.tool_call_id.clone())
            .collect();
        let missing: Vec<String> = self.messages[index]
            .message
            .tool_calls
            .iter()
            .map(|call| call.id.clone())
            .filter(|id| !answered.contains(id))
            .collect();
        for id in missing {
            self.push_tool_result(&id, "用户停止了本次执行。".to_string(), "已停止".to_string());
        }
    }

    /// 清空当前对话（要连输入框一起清就用「新建对话」）。正在进行的请求结果会被丢弃。
    fn clear(&mut self, cx: &mut Context<Self>) {
        self.cancel_current();
        self.request_id += 1;
        self.messages.clear();
        self.error = None;
        self.notice = None;
        self.pending = false;
        self.approval = None;
        self.action_approval = None;
        self.tool_queue.clear();
        self.tool_rounds = 0;
        self.expanded_code.clear();
        cx.notify();
    }

    /// 新建对话：清空内容与输入框，滚动回到顶部，焦点放回输入框。
    fn new_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear(cx);
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.scroll.set_offset(point(px(0.0), px(0.0)));
        self.focus_input(window, cx);
    }

    /// 展开 / 收起某个代码块。
    fn toggle_code(&mut self, message: usize, segment: usize, cx: &mut Context<Self>) {
        if !self.expanded_code.remove(&(message, segment)) {
            self.expanded_code.insert((message, segment));
        }
        cx.notify();
    }

    /// 一条消息：用户靠右、AI 靠左，各是一个圆角气泡。
    ///
    /// AI 的回复按围栏代码块切成若干段：说明文字整段显示（可选中复制），
    /// 代码块只露前几行，需要时再展开。
    fn message_bubble(
        &self,
        index: usize,
        message: &ChatMessage,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        let is_user = matches!(message.role, Role::User);
        let font_size = chat_font_size(cx);
        let bubble_bg = if is_user {
            palette.menu_item_active
        } else {
            palette.side_panel_raised_bg
        };

        let thinking = !is_user && self.pending && message.content.is_empty();
        let content: AnyElement = if thinking {
            div()
                .text_size(font_size)
                .text_color(rgb(palette.muted_text))
                .child("正在思考…")
                .into_any_element()
        } else if is_user {
            text_block(segment_id(index, 0), &message.content, palette, font_size)
        } else {
            let segments = markdown::split(&message.content);
            if segments.is_empty() {
                div().text_size(font_size).into_any_element()
            } else {
                let mut blocks = v_flex().gap_2().min_w_0();
                for (segment_index, segment) in segments.into_iter().enumerate() {
                    blocks = blocks.child(self.segment_block(
                        index,
                        segment_index,
                        segment,
                        palette,
                        font_size,
                        cx,
                    ));
                }
                blocks.into_any_element()
            }
        };

        h_flex()
            .w_full()
            .when(is_user, |this| this.justify_end())
            .child(
                v_flex()
                    .min_w_0()
                    .max_w(relative(0.85))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(8.0))
                    .bg(rgb(bubble_bg))
                    .text_size(font_size)
                    .text_color(rgb(palette.sidebar_text))
                    .child(content),
            )
            .into_any_element()
    }

    /// 回复里的一段：说明文字交给 Markdown 渲染；代码块用深色圆角底框单独成块。
    fn segment_block(
        &self,
        message: usize,
        segment: usize,
        seg: Segment,
        palette: &Palette,
        font_size: Pixels,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (language, code) = match seg {
            Segment::Text(text) => {
                return text_block(segment_id(message, segment), &text, palette, font_size);
            }
            Segment::Code { language, code, .. } => (language, code),
        };

        let line_count = markdown::line_count(&code);
        let expanded = self.expanded_code.contains(&(message, segment));
        let collapsible = line_count > CODE_PREVIEW_LINES;
        let collapsed = collapsible && !expanded;

        // 没写语言时给个 "text"，代码块顶部才会留出放按钮的那一条。
        let language = if language.is_empty() {
            "text".to_string()
        } else {
            language
        };
        let body = if collapsed {
            let preview: Vec<&str> = code.lines().take(CODE_PREVIEW_LINES).collect();
            format!("```{language}\n{}\n…\n```", preview.join("\n"))
        } else {
            format!("```{language}\n{code}\n```")
        };

        // 按钮浮在代码块右上角（那一行本来就被语言标签占着），不额外占高度。
        let write_code = code.clone();
        let controls = h_flex()
            .absolute()
            .top(px(4.0))
            .right(px(4.0))
            .items_center()
            .gap_1()
            .child(
                Clipboard::new(copy_id(message, segment))
                    .value(code.clone())
                    .tooltip("复制整段代码"),
            )
            .child(
                Button::new(write_id(message, segment))
                    .ghost()
                    .xsmall()
                    .label("写入")
                    .tooltip("把这段代码写入本地文件")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let app = this.app.clone();
                        let code = write_code.clone();
                        let _ = app.update(cx, |this, cx| this.apply_ai_code(&code, window, cx));
                    })),
            )
            .when(collapsible, |this| {
                this.child(
                    Button::new(expand_id(message, segment))
                        .ghost()
                        .xsmall()
                        .label(if expanded { "收起" } else { "展开" })
                        .tooltip(if expanded {
                            "收起代码".to_string()
                        } else {
                            format!("展开全部 {line_count} 行")
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_code(message, segment, cx)
                        })),
                )
            });

        div()
            .relative()
            .min_w_0()
            .child(
                TextView::markdown(segment_id(message, segment), body)
                    .text_size(font_size)
                    .style(code_text_style(palette)),
            )
            .child(controls)
            .into_any_element()
    }
}

impl BasePanel for AiPanel {
    fn panel_name(&self) -> &'static str {
        AI_PANEL
    }

    fn visible(&self, cx: &App) -> bool {
        SettingsStore::snapshot(cx).appearance.show_ai()
    }

    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.group = Some(group);
    }

    fn on_removed(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.group = None;
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl DockPanel for AiPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some("AI 编程".into())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "AI 编程"
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for AiPanel {}

impl Focusable for AiPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AiPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_theme(cx);
        let chat_font = chat_font_size(cx);
        let panel_id = PanelId::from(cx.entity().entity_id());
        let drag = self
            .group
            .as_ref()
            .and_then(|group| group.upgrade())
            .map(|group| DragPanel::new(panel_id, group.read(cx).node()));
        let drag_start = {
            let app = self.app.clone();
            Some(Rc::new(move |cx: &mut App| {
                let _ = app.update(cx, |this, cx| this.begin_panel_drag(panel_id, cx));
            }) as DragStartAction)
        };

        let close_panel = Rc::new(move |_: &mut Window, cx: &mut App| {
            SettingsStore::update(cx, |settings| settings.appearance.set_ai(false));
        }) as ClosePanelAction;

        let title = div()
            .id("ai-title")
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .items_center()
            .child("AI 编程");
        let title = if let Some(drag) = drag {
            title.on_drag(drag, move |drag, offset, _, cx| {
                cx.stop_propagation();
                if let Some(on_drag_start) = drag_start.clone() {
                    on_drag_start(cx);
                }
                drag.set_drag_offset(offset);
                drag.set_preview_size(size(px(96.0), px(30.0)));
                cx.new(|_| crate::panel_drag::PanelDragPreview::new("AI 编程"))
            })
        } else {
            title
        };

        let new_button = Button::new("ai-new")
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .tooltip("新建对话")
            .on_click(cx.listener(|this, _, window, cx| this.new_conversation(window, cx)));
        let clear_button = Button::new("ai-clear")
            .ghost()
            .xsmall()
            .label("清空")
            .tooltip("清空当前对话内容")
            .disabled(
                self.messages.is_empty()
                    && self.error.is_none()
                    && !self.pending
                    && self.approval.is_none()
                    && self.action_approval.is_none(),
            )
            .on_click(cx.listener(|this, _, _, cx| this.clear(cx)));

        let header = h_flex()
            .h(px(ui_theme::EXPLORER_HEADER_HEIGHT))
            .px(px(16.0))
            .items_center()
            .justify_between()
            .text_xs()
            .text_color(rgb(palette.sidebar_text))
            .child(title)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(new_button)
                    .child(clear_button)
                    .child(close_button(palette, close_panel.clone())),
            );

        let empty = self.messages.is_empty()
            && !self.pending
            && self.error.is_none()
            && self.notice.is_none()
            && self.approval.is_none()
            && self.action_approval.is_none();
        let body: AnyElement = if empty {
            placeholder(palette, chat_font).into_any_element()
        } else {
            let bubbles: Vec<AnyElement> = self
                .messages
                .iter()
                .enumerate()
                .map(|(index, entry)| match entry.message.role {
                    Role::Tool => tool_row(entry, &palette, chat_font),
                    // 只调用工具、没有正文的助手消息不必占一个空气泡。
                    Role::Assistant
                        if !self.pending && entry.message.content.trim().is_empty() =>
                    {
                        div().into_any_element()
                    }
                    _ => self.message_bubble(index, &entry.message, &palette, &*cx),
                })
                .collect();
            let mut list = v_flex().gap_2().w_full().children(bubbles);
            if let Some(pending) = &self.approval {
                list = list.child(approval_card(pending, &palette, chat_font, cx));
            }
            if let Some(pending) = &self.action_approval {
                list = list.child(action_approval_card(pending, &palette, chat_font, cx));
            }
            if let Some(error) = &self.error {
                list = list.child(
                    div()
                        .text_size(chat_font)
                        .text_color(rgb(palette.error))
                        .child(error.clone()),
                );
            }
            if let Some(notice) = &self.notice {
                list = list.child(
                    div()
                        .text_size(chat_font)
                        .text_color(rgb(palette.muted_text))
                        .child(notice.clone()),
                );
            }
            div()
                .id("ai-messages")
                .flex_1()
                .w_full()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .bg(rgb(palette.side_panel_bg))
                .p(px(10.0))
                .child(list)
                .into_any_element()
        };

        let busy = self.pending || self.approval.is_some() || self.action_approval.is_some();
        let focused = self.input.read(cx).has_selection_focus(window, cx);
        let mut input = Input::new(&self.input)
            .xsmall()
            .cleanable(true)
            .bg(rgb(palette.side_panel_bg))
            // 不要组件自带的聚焦外圈：焦点只用更亮的边框表示。
            .focus_ring(false);
        if focused {
            input = input.border_color(rgb(palette.sidebar_text));
        }
        let escape = close_panel.clone();
        let send_label = if self.pending { "思考中…" } else { "发送" };
        let stop_button = Button::new("ai-stop")
            .ghost()
            .xsmall()
            .label("停止")
            .text_color(rgb(palette.error))
            .tooltip("停止当前 AI 请求")
            .on_click(cx.listener(|this, _, _window, cx| this.stop(cx)));
        let input_row = h_flex()
            .gap_1()
            .items_center()
            .px(px(8.0))
            .py(px(6.0))
            .border_t_1()
            .border_color(rgba(palette.border))
            .bg(rgb(palette.side_panel_bg))
            .on_action(move |_: &Escape, window, cx| escape(window, cx))
            .on_action(cx.listener(|this, _: &Enter, window, cx| this.send(window, cx)))
            .child(div().flex_1().min_w_0().child(input))
            .child(
                Button::new("ai-send")
                    .ghost()
                    .xsmall()
                    .label(send_label)
                    .disabled(busy)
                    .tooltip("回车也可以发送")
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
            )
            .when(busy, |this| this.child(stop_button));

        // 底栏：交互方式 + 工具轮数 + 当前 AI 模型，点开都是下拉菜单。
        // 文字用比正文更淡的颜色，免得两行按钮抢了对话的注意力。
        let settings = SettingsStore::snapshot(cx);
        let approval = settings.ai.approval;
        let approval_label = approval.short_label();
        let footer_text = rgb(palette.muted_text);
        let mode_button = Button::new("ai-mode")
            .ghost()
            .xsmall()
            .label(approval_label)
            .text_color(footer_text)
            .child(Icon::new(IconName::ChevronDown).xsmall().text_color(footer_text))
            .tooltip("AI 写入文件、运行或推进脚本前是否先询问")
            // 底栏在面板最下面，菜单要往上弹：BottomLeft = 开在触发按钮上方、左对齐。
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
                let mut menu = menu;
                for (value, label) in AiApproval::OPTIONS {
                    let value = *value;
                    let checked = AiApproval::from_str(value) == approval;
                    menu = menu.item(
                        PopupMenuItem::new(*label)
                            .checked(checked)
                            .on_click(move |_, _, cx| {
                                let mode = AiApproval::from_str(value);
                                SettingsStore::update(cx, |settings| settings.ai.approval = mode);
                            }),
                    );
                }
                menu
            });
        let round_limit = settings.ai.round_limit;
        let round_button = Button::new("ai-rounds")
            .ghost()
            .xsmall()
            .label(round_limit.short_label())
            .text_color(footer_text)
            .child(Icon::new(IconName::ChevronDown).xsmall().text_color(footer_text))
            .tooltip("AI 一次提问最多连续执行多少轮工具调用")
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
                let mut menu = menu;
                for (value, label) in AiRoundLimit::OPTIONS {
                    let value = *value;
                    let checked = AiRoundLimit::from_str(value) == round_limit;
                    menu = menu.item(
                        PopupMenuItem::new(*label)
                            .checked(checked)
                            .on_click(move |_, _, cx| {
                                let limit = AiRoundLimit::from_str(value);
                                SettingsStore::update(cx, |settings| {
                                    settings.ai.round_limit = limit
                                });
                            }),
                    );
                }
                menu
            });
        let model_label = settings.ai.effective_model().to_string();
        let model_button = Button::new("ai-model")
            .ghost()
            .xsmall()
            .label(model_label)
            .text_color(footer_text)
            .child(Icon::new(IconName::ChevronDown).xsmall().text_color(footer_text))
            .tooltip("当前使用的 AI 模型（DeepSeek）")
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, cx| {
                let current = SettingsStore::snapshot(cx).ai.effective_model().to_string();
                let mut entries: Vec<(String, String)> = AI_MODEL_OPTIONS
                    .iter()
                    .map(|(value, label)| (value.to_string(), label.to_string()))
                    .collect();
                if !entries.iter().any(|(value, _)| value == &current) {
                    entries.push((current.clone(), format!("{current}（自定义）")));
                }
                let mut menu = menu;
                for (value, label) in entries {
                    let checked = value == current;
                    let picked = value.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(checked)
                            .on_click(move |_, _, cx| {
                                let picked = picked.clone();
                                SettingsStore::update(cx, |settings| settings.ai.model = picked);
                            }),
                    );
                }
                menu
            });
        let footer = h_flex()
            .px(px(8.0))
            .pb(px(6.0))
            .gap_1()
            .items_center()
            .bg(rgb(palette.side_panel_bg))
            .child(mode_button)
            .child(round_button)
            .child(model_button);

        div()
            .size_full()
            .p(px(ui_theme::PANEL_GAP / 2.0))
            .on_action(move |_: &Escape, window, cx| close_panel(window, cx))
            .child(
                v_flex()
                    .h_full()
                    .w_full()
                    .bg(rgb(palette.sidebar_bg))
                    .rounded(px(ui_theme::PANE_RADIUS))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .overflow_hidden()
                    .child(header)
                    .child(body)
                    .child(input_row)
                    .child(footer),
            )
    }
}

/// 还没有对话时居中显示的提示。
fn placeholder(palette: Palette, font_size: Pixels) -> impl IntoElement {
    v_flex()
        .flex_1()
        .w_full()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap_2()
        .px(px(16.0))
        .bg(rgb(palette.side_panel_bg))
        .child(Icon::new(IconName::Bot).text_color(rgb(palette.muted_text)))
        .child(
            div()
                .text_size(font_size)
                .text_color(rgb(palette.sidebar_text))
                .child("AI 编程助手"),
        )
        .child(
            div()
                .text_size(font_size)
                .text_color(rgb(palette.muted_text))
                .child("在下面输入需求，按回车发给 DeepSeek"),
        )
}

/// 工具调用的一行活动记录。
fn tool_row(entry: &ChatEntry, palette: &Palette, font_size: Pixels) -> AnyElement {
    let note = entry
        .note
        .clone()
        .unwrap_or_else(|| "工具调用".to_string());
    h_flex()
        .w_full()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_shrink_0()
                .text_size(font_size)
                .text_color(rgb(palette.muted_text))
                .child("·"),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(font_size)
                .text_color(rgb(palette.muted_text))
                .child(note),
        )
        .into_any_element()
}

/// 写入确认卡片。
fn approval_card(
    pending: &PendingWrite,
    palette: &Palette,
    font_size: Pixels,
    cx: &Context<AiPanel>,
) -> AnyElement {
    v_flex()
        .w_full()
        .gap_2()
        .p(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgb(palette.side_panel_raised_bg))
        .child(
            div()
                .text_size(font_size)
                .text_color(rgb(palette.sidebar_text))
                .child("AI 想修改文件"),
        )
        .child(
            div()
                .text_size(font_size)
                .text_color(rgb(palette.muted_text))
                .child(pending.summary.clone()),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("ai-approve")
                        .small()
                        .label("同意写入")
                        .on_click(cx.listener(|this, _, window, cx| this.approve(window, cx))),
                )
                .child(
                    Button::new("ai-reject")
                        .small()
                        .label("拒绝")
                        .on_click(cx.listener(|this, _, window, cx| this.reject(window, cx))),
                ),
        )
        .into_any_element()
}

/// 脚本执行确认卡片。
fn action_approval_card(
    pending: &PendingAction,
    palette: &Palette,
    font_size: Pixels,
    cx: &Context<AiPanel>,
) -> AnyElement {
    v_flex()
        .w_full()
        .gap_2()
        .p(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgb(palette.side_panel_raised_bg))
        .child(
            div()
                .text_size(font_size)
                .text_color(rgb(palette.sidebar_text))
                .child("AI 请求执行脚本操作"),
        )
        .child(
            div()
                .text_size(font_size)
                .text_color(rgb(palette.muted_text))
                .child(format!(
                    "{}。脚本可能产生文件或其他外部副作用。",
                    pending.summary
                )),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("ai-approve-action")
                        .small()
                        .label("同意执行")
                        .on_click(cx.listener(|this, _, window, cx| this.approve(window, cx))),
                )
                .child(
                    Button::new("ai-reject-action")
                        .small()
                        .label("拒绝")
                        .on_click(cx.listener(|this, _, window, cx| this.reject(window, cx))),
                ),
        )
        .into_any_element()
}

/// 消息里某一段的元素 id（同一段每帧都用同一个 id）。
fn segment_id(message: usize, segment: usize) -> SharedString {
    SharedString::from(format!("ai-seg-{message}-{segment}"))
}

/// 代码块「展开 / 收起」按钮的 id。
fn expand_id(message: usize, segment: usize) -> SharedString {
    SharedString::from(format!("ai-expand-{message}-{segment}"))
}

/// 代码块「复制」按钮的 id。
fn copy_id(message: usize, segment: usize) -> SharedString {
    SharedString::from(format!("ai-copy-{message}-{segment}"))
}

/// 代码块「写入文件」按钮的 id。
fn write_id(message: usize, segment: usize) -> SharedString {
    SharedString::from(format!("ai-write-{message}-{segment}"))
}

/// 说明文字：Markdown 渲染，可拖选后用 Ctrl+C 复制。
fn text_block(
    id: impl Into<ElementId>,
    text: &str,
    palette: &Palette,
    font_size: Pixels,
) -> AnyElement {
    TextView::markdown(id, text.to_string())
        .text_size(font_size)
        .text_color(rgb(palette.sidebar_text))
        .style(code_text_style(palette))
        .into_any_element()
}

/// AI 对话区的字号，来自「设置 → AI」。
fn chat_font_size(cx: &App) -> Pixels {
    px(SettingsStore::snapshot(cx).ai.font_size as f32)
}

/// 代码块底色：在气泡底色上每个通道再压暗一点，暗色 / 浅色主题都合适。
fn code_background(base: u32) -> u32 {
    let dim = |shift: u32| ((base >> shift) & 0xff).saturating_sub(0x12);
    (dim(16) << 16) | (dim(8) << 8) | dim(0)
}

/// 代码块样式：更深的底色 + 圆角边框，和说明文字明显区分开。
fn code_text_style(palette: &Palette) -> TextViewStyle {
    let code_block = StyleRefinement::default()
        .bg(rgb(code_background(palette.sidebar_bg)))
        .border_1()
        .border_color(rgba(palette.border))
        .rounded(px(6.0));
    TextViewStyle::default().code_block(code_block)
}

fn close_button(palette: Palette, close_panel: ClosePanelAction) -> impl IntoElement {
    div()
        .id("ai-close-panel")
        .flex_shrink_0()
        .size(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(rgba(palette.hover)))
        .child(
            Icon::new(IconName::PanelRightClose)
                .xsmall()
                .text_color(rgb(palette.sidebar_text)),
        )
        .on_click(move |_, window, cx| close_panel(window, cx))
}
