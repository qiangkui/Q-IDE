//! 外部 agent 的工具执行：状态读取、打开文件、安全命令与需要确认的写入/删除/移动。
//!
//! 传输层见 [`crate::agent`]；`qide-mcp` 通过本机回环端口把 MCP 的 `tools/call` 送进来。

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::SyncSender;
use std::time::Duration;

use serde_json::{Value, json};

use super::dialogs::DialogCallback;
use super::*;
use crate::agent::AgentReply;
use crate::ai::client::ToolCall;
use crate::ai::tools::{self, AppAction, ToolPlan};

/// 一次工具调用的结果：立即完成，或等待用户在确认框里操作。
enum AgentOutcome {
    Done(Result<String, String>),
    Pending,
}

/// 需要用户确认的修改操作。
enum AgentMutation {
    Write {
        path: PathBuf,
        content: String,
        summary: String,
        done: String,
    },
    AiAction {
        action: crate::ai::tools::AppAction,
        summary: String,
    },
}

impl AgentMutation {
    fn description(&self) -> String {
        match self {
            Self::Write { summary, .. } => format!("外部 agent 请求{summary}。"),
            Self::AiAction { summary, .. } => format!("外部 agent 请求{summary}。"),
        }
    }

    fn apply(
        &self,
        this: &mut CodeWindow,
        window: &mut Window,
        cx: &mut Context<CodeWindow>,
    ) -> Result<String, String> {
        match self {
            Self::Write {
                path,
                content,
                done,
                ..
            } => {
                this.write_ai_file(path, content, window, cx)?;
                Ok(done.clone())
            }
            Self::AiAction { action, .. } => this.ai_execute_action(action.clone(), window, cx),
        }
    }
}

impl CodeWindow {
    /// 启动外部 agent 的本机回环接口，并把请求轮询到 UI 线程执行。
    pub(crate) fn start_agent_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let endpoint = match crate::agent::start_server(request_tx) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                self.output()
                    .error(cx, format!("外部 agent 接口启动失败：{error}"));
                return;
            }
        };
        let port = endpoint.port;
        self.agent_endpoint = Some(endpoint);
        self.sync_agent_endpoint(cx);
        if SettingsStore::snapshot(cx).ai.agent_enabled {
            self.output().info(
                cx,
                format!("外部 agent 接口已监听 127.0.0.1:{port}（qide-mcp）"),
            );
        }

        cx.spawn_in(window, async move |this, cx| {
            loop {
                while let Ok(request) = request_rx.try_recv() {
                    let crate::agent::AgentRequest {
                        tool,
                        arguments,
                        reply,
                    } = request;
                    let outcome = this.update_in(cx, |this, window, cx| {
                        this.agent_execute(&tool, arguments, reply.clone(), window, cx)
                    });
                    match outcome {
                        Ok(AgentOutcome::Done(result)) => {
                            let reply_value = match result {
                                Ok(content) => AgentReply::ok(content),
                                Err(error) => AgentReply::error(error),
                            };
                            let _ = reply.send(reply_value);
                        }
                        Ok(AgentOutcome::Pending) => {}
                        Err(_) => {
                            let _ = reply.send(AgentReply::error("Q-IDE 主窗口已关闭"));
                        }
                    }
                }
                if this.update(cx, |_, _| ()).is_err() {
                    break;
                }
                // std mpsc 不能唤醒 UI 线程；用 100ms 轮询兼顾延迟与空闲开销。
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
        })
        .detach();
    }

    /// 根据设置启用/停用端点文件；关闭时删掉端点，qide-mcp 会提示先启动 Q-IDE。
    pub(crate) fn sync_agent_endpoint(&self, cx: &mut App) {
        let Some(endpoint) = &self.agent_endpoint else {
            return;
        };
        if SettingsStore::snapshot(cx).ai.agent_enabled {
            if let Err(error) = endpoint.write() {
                self.output()
                    .error(cx, format!("无法写入外部 agent 端点文件：{error}"));
            }
        } else {
            crate::agent::remove_endpoint();
        }
    }

    fn agent_execute(
        &mut self,
        tool: &str,
        arguments: Value,
        reply: SyncSender<AgentReply>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentOutcome {
        if !crate::agent::tool_defined(tool) {
            return AgentOutcome::Done(Err(format!("未知工具：{tool}")));
        }
        let settings = SettingsStore::snapshot(cx).ai.clone();
        if !settings.agent_enabled {
            return AgentOutcome::Done(Err("外部 agent 接口已在设置中关闭".to_string()));
        }
        if settings.agent_audit && !is_read_only_tool(tool) {
            self.output().info(cx, format!("外部 agent 调用 {tool}"));
        }
        let write_allowed = settings.agent_allow_write;
        let auto_confirm = settings.agent_auto_confirm;
        let run_direct = settings.agent_allow_run;

        match tool {
            "qide_write_file" | "qide_replace_in_file" | "qide_delete_path" | "qide_move_path" => {
                if !write_allowed {
                    return AgentOutcome::Done(Err(
                        "外部 agent 写入/删除/移动未开启，请在设置中允许".to_string(),
                    ));
                }
                match self.agent_tool_plan(tool, arguments, cx) {
                    Ok(ToolPlan::Write {
                        path,
                        content,
                        summary,
                        done,
                    }) => self.finish_mutation(
                        AgentMutation::Write {
                            path,
                            content,
                            summary,
                            done,
                        },
                        reply,
                        auto_confirm,
                        window,
                        cx,
                    ),
                    Ok(ToolPlan::Action { action, summary }) => self.finish_mutation(
                        AgentMutation::AiAction { action, summary },
                        reply,
                        auto_confirm,
                        window,
                        cx,
                    ),
                    Ok(ToolPlan::Read { content, .. }) => AgentOutcome::Done(Err(content)),
                    Err(error) => AgentOutcome::Done(Err(error)),
                }
            }
            "qide_run_command" => {
                AgentOutcome::Done(self.agent_run_command(arguments, write_allowed, window, cx))
            }
            "qide_check_syntax" => {
                self.check_syntax(cx);
                AgentOutcome::Done(Ok("已检查语法，结果见输出面板".to_string()))
            }
            "qide_run_script" | "qide_start_debug" | "qide_debug_continue" | "qide_debug_step" => {
                let (action, summary) = match tool {
                    "qide_run_script" => (AppAction::RunScript, "运行当前 VB 脚本"),
                    "qide_start_debug" => (AppAction::StartDebug, "调试运行当前 VB 脚本"),
                    "qide_debug_continue" => (AppAction::ContinueDebug, "继续调试"),
                    _ => (AppAction::StepDebug, "单步执行"),
                };
                self.agent_ai_action(action, summary, !run_direct, reply, window, cx)
            }
            "qide_debug_pause" => {
                self.agent_ai_action(AppAction::PauseDebug, "暂停脚本", true, reply, window, cx)
            }
            "qide_stop_execution" => self.agent_ai_action(
                AppAction::StopExecution,
                "停止脚本",
                true,
                reply,
                window,
                cx,
            ),
            "qide_set_breakpoint" => match agent_breakpoint_action(arguments) {
                Ok(action) => self.agent_ai_action(action, "设置断点", true, reply, window, cx),
                Err(error) => AgentOutcome::Done(Err(error)),
            },
            "qide_add_watch" => match agent_watch_action(arguments) {
                Ok(action) => {
                    self.agent_ai_action(action, "添加监视表达式", true, reply, window, cx)
                }
                Err(error) => AgentOutcome::Done(Err(error)),
            },
            _ => AgentOutcome::Done(match tool {
                "qide_get_app_state" => Ok(self.agent_app_state(cx)),
                "qide_list_commands" => Ok(agent_commands_json()),
                "qide_read_output" => Ok(agent_output_tail(&self.ai_output_tail(cx))),
                "qide_get_debug_state" => Ok(self.ai_debug_state()),
                "qide_list_files" | "qide_read_file" => self.agent_file_tool(tool, arguments, cx),
                "qide_open_file" => self.agent_open_file(arguments, window, cx),
                _ => Err(format!("工具 {tool} 暂未实现")),
            }),
        }
    }

    /// 运行脚本与调试推进默认要确认；设置里打开「允许运行脚本和调试」后直接执行。
    fn agent_ai_action(
        &mut self,
        action: AppAction,
        summary: &str,
        confirm: bool,
        reply: SyncSender<AgentReply>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentOutcome {
        if confirm && action.requires_confirmation() {
            self.confirm_mutation(
                AgentMutation::AiAction {
                    action,
                    summary: summary.to_string(),
                },
                reply,
                window,
                cx,
            );
            AgentOutcome::Pending
        } else {
            AgentOutcome::Done(self.ai_execute_action(action, window, cx))
        }
    }

    fn finish_mutation(
        &mut self,
        mutation: AgentMutation,
        reply: SyncSender<AgentReply>,
        auto_confirm: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentOutcome {
        if auto_confirm {
            AgentOutcome::Done(mutation.apply(self, window, cx))
        } else {
            self.confirm_mutation(mutation, reply, window, cx);
            AgentOutcome::Pending
        }
    }

    /// 用自带 AI 的同一套工具计划做参数与路径校验，返回写入计划或主窗口动作。
    fn agent_tool_plan(&self, tool: &str, arguments: Value, cx: &App) -> Result<ToolPlan, String> {
        let root = self
            .ai_project_root()
            .ok_or_else(|| "没有打开项目目录".to_string())?;
        let read = |path: &Path| self.file_content(path);
        let output = || self.ai_output_tail(cx);
        let debug_state = || self.ai_debug_state();
        let ctx = tools::PlanCtx {
            root: &root,
            read: &read,
            output: &output,
            debug_state: &debug_state,
        };
        let name = tool.strip_prefix("qide_").unwrap_or(tool);
        let call = ToolCall {
            id: "agent".to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
        };
        match tools::plan(&ctx, &call) {
            ToolPlan::Read { content, .. } => Err(content),
            plan => Ok(plan),
        }
    }

    fn confirm_mutation(
        &mut self,
        mutation: AgentMutation,
        reply: SyncSender<AgentReply>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let description = mutation.description();
        let handle = cx.entity().downgrade();
        let confirm_reply = reply.clone();
        let on_confirm: DialogCallback = Rc::new(move |window, cx| {
            let result = handle.update(cx, |this, cx| mutation.apply(this, window, cx));
            let reply = match result {
                Ok(Ok(text)) => AgentReply::ok(text),
                Ok(Err(error)) => AgentReply::error(error),
                Err(_) => AgentReply::error("Q-IDE 主窗口已关闭"),
            };
            let _ = confirm_reply.send(reply);
        });
        let on_cancel: DialogCallback = Rc::new(move |_window, _cx| {
            let _ = reply.send(AgentReply::error("用户取消了操作"));
        });
        show_agent_confirm_dialog(
            window,
            cx,
            "外部 agent 请求",
            description,
            on_confirm,
            on_cancel,
        );
    }

    fn agent_app_state(&self, cx: &App) -> String {
        let roots: Vec<String> = self
            .explorer
            .roots()
            .iter()
            .map(|root| root.path().to_string_lossy().into_owned())
            .collect();
        let open_files: Vec<Value> = self
            .open_files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                json!({
                    "path": file.id,
                    "name": file.name,
                    "active": Some(index) == self.active_file,
                    "dirty": self.file_is_dirty(index),
                })
            })
            .collect();
        let appearance = SettingsStore::snapshot(cx).appearance;
        let (selection_active, selected_paths) = self.explorer_panel.read(cx).selection_snapshot();
        json!({
            "roots": roots,
            "open_files": open_files,
            "active_file": self
                .active_file
                .and_then(|index| self.open_files.get(index))
                .map(|file| file.id.to_string()),
            "explorer_selection": {
                "active": selection_active,
                "paths": selected_paths,
            },
            "panels": {
                "explorer": appearance.show_explorer(),
                "outline": appearance.show_outline(),
                "search": appearance.show_search(),
                "ai": appearance.show_ai(),
                "output": appearance.show_output,
                "status_bar": appearance.show_status_bar,
            },
            "debug": self.ai_debug_state(),
            "output_tail": agent_output_tail(&self.ai_output_tail(cx)),
        })
        .to_string()
    }

    /// 复用自带 AI 的只读文件工具（路径限制、行号范围都在那里处理）。
    fn agent_file_tool(&self, tool: &str, arguments: Value, cx: &App) -> Result<String, String> {
        let root = self
            .ai_project_root()
            .ok_or_else(|| "没有打开项目目录".to_string())?;
        let read = |path: &Path| self.file_content(path);
        let output = || self.ai_output_tail(cx);
        let debug_state = || self.ai_debug_state();
        let ctx = tools::PlanCtx {
            root: &root,
            read: &read,
            output: &output,
            debug_state: &debug_state,
        };
        let name = tool.strip_prefix("qide_").unwrap_or(tool);
        let call = ToolCall {
            id: "agent".to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
        };
        match tools::plan(&ctx, &call) {
            ToolPlan::Read { content, .. } => Ok(content),
            _ => Err(format!("工具 {tool} 不支持只读调用")),
        }
    }

    fn agent_open_file(
        &mut self,
        arguments: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let raw = arguments
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "缺少 path".to_string())?;
        let root = self
            .ai_project_root()
            .ok_or_else(|| "没有打开项目目录".to_string())?;
        let path = tools::resolve(&root, raw)?;
        if !path.is_file() {
            return Err(format!("不是文件：{raw}"));
        }
        let line = arguments
            .get("line")
            .and_then(Value::as_u64)
            .map(|line| line as usize)
            .filter(|line| *line > 0);
        let id = path.to_string_lossy().into_owned();
        self.open_file(&id, window, cx);
        if let Some(line) = line {
            self.go_to_line(line, window, cx);
        }
        Ok(format!("已打开 {raw}"))
    }

    fn agent_run_command(
        &mut self,
        arguments: Value,
        allow_write: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let id = arguments
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "缺少 id".to_string())?;
        if !crate::agent::SAFE_COMMANDS.contains(&id) {
            return Err(format!("命令 {id} 不允许外部 agent 执行"));
        }
        if matches!(id, "file.save" | "file.save-all") && !allow_write {
            return Err("外部 agent 写入未开启，不能保存文件".to_string());
        }
        let needs_active_file =
            id.starts_with("edit.") || matches!(id, "file.save" | "file.save-all");
        if needs_active_file && self.active_file.is_none() {
            return Err("当前没有活动文件".to_string());
        }
        self.run_command(id, window, cx);
        Ok(format!("已执行命令 {id}"))
    }
}

fn show_agent_confirm_dialog(
    window: &mut Window,
    cx: &mut App,
    title: &'static str,
    description: String,
    on_confirm: DialogCallback,
    on_cancel: DialogCallback,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let on_confirm = on_confirm.clone();
        let on_cancel = on_cancel.clone();
        let description = description.clone();
        dialog
            .title(div().text_sm().child(title))
            .child(div().text_xs().child(description))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("agent-confirm-cancel")
                            .small()
                            .label("取消")
                            .on_click(move |_, window, cx| {
                                on_cancel(window, cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("agent-confirm-apply")
                            .small()
                            .label("允许执行")
                            .on_click(move |_, window, cx| {
                                on_confirm(window, cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

fn agent_breakpoint_action(arguments: Value) -> Result<AppAction, String> {
    let line = arguments
        .get("line")
        .and_then(Value::as_u64)
        .filter(|line| *line > 0)
        .ok_or_else(|| "line 必须是大于 0 的行号".to_string())? as usize;
    let enabled = arguments
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| "enabled 必须是布尔值".to_string())?;
    let condition = enabled
        .then(|| arguments.get("condition").and_then(Value::as_str))
        .flatten()
        .map(str::trim)
        .filter(|condition| !condition.is_empty())
        .map(str::to_string);
    Ok(AppAction::SetBreakpoint {
        line,
        enabled,
        condition,
    })
}

fn agent_watch_action(arguments: Value) -> Result<AppAction, String> {
    let expression = arguments
        .get("expression")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|expression| !expression.is_empty())
        .ok_or_else(|| "expression 不能为空".to_string())?;
    Ok(AppAction::AddWatch {
        expression: expression.to_string(),
    })
}

fn is_read_only_tool(tool: &str) -> bool {
    matches!(
        tool,
        "qide_get_app_state"
            | "qide_list_commands"
            | "qide_list_files"
            | "qide_read_file"
            | "qide_read_output"
            | "qide_get_debug_state"
    )
}

fn agent_commands_json() -> String {
    let commands: Vec<Value> = crate::commands::COMMANDS
        .iter()
        .map(|command| {
            json!({
                "id": command.id,
                "label": command.label,
                "shortcut": command.shortcut,
            })
        })
        .collect();
    json!(commands).to_string()
}

fn agent_output_tail(text: &str) -> String {
    const TAIL_LINES: usize = 80;
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}
