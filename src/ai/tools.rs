//! AI 工具层：告诉模型能做哪些事，并把一次调用算成「只读结果」或「待写入的文件」。
//!
//! 这一层不碰界面、也不直接写盘：路径解析与内容计算是纯逻辑，真正落盘由界面层
//! （`CodeWindow`）执行，这样打开着的标签、未保存内容、编码都能一起处理好。

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

/// 单个工具的定义。
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema 文本。
    pub parameters: &'static str,
}

/// 发给模型的 `tools` 数组。
pub fn tools_json() -> Value {
    let tools: Vec<Value> = specs()
        .iter()
        .map(|spec| {
            json!({
                "type": "function",
                "function": {
                    "name": spec.name,
                    "description": spec.description,
                    "parameters": serde_json::from_str::<Value>(spec.parameters)
                        .unwrap_or_else(|_| json!({"type": "object"})),
                }
            })
        })
        .collect();
    Value::Array(tools)
}

/// DeepSeek 的推理模型（`deepseek-reasoner`）暂不支持工具调用。
pub fn model_supports_tools(model: &str) -> bool {
    !model.trim().to_ascii_lowercase().contains("reasoner")
}

/// 按模型挑 `tools` 数组：不支持函数的模型给空数组，请求体里就不会带 `tools`。
pub fn tools_for_model(model: &str) -> Value {
    if model_supports_tools(model) {
        tools_json()
    } else {
        Value::Array(Vec::new())
    }
}

pub fn specs() -> &'static [ToolSpec] {
    &[
        ToolSpec {
            name: "read_file",
            description: "读取项目里的文件，返回带行号的文本。文件很大时用 start/end 指定行范围。",
            parameters: r#"{
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对项目根目录的路径"},
                    "start": {"type": "integer", "description": "起始行（1 基，可选）"},
                    "end": {"type": "integer", "description": "结束行（含，可选）"}
                },
                "required": ["path"]
            }"#,
        },
        ToolSpec {
            name: "list_files",
            description: "列出项目里的文件（相对路径，最多 300 个）。",
            parameters: r#"{
                "type": "object",
                "properties": {
                    "dir": {"type": "string", "description": "子目录（可选，默认项目根目录）"}
                }
            }"#,
        },
        ToolSpec {
            name: "replace_in_file",
            description: "把文件里一段**唯一**的原文替换成新内容，其余内容不动。请先 read_file 确认原文。",
            parameters: r#"{
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "old": {"type": "string", "description": "文件里原样出现且只出现一次的片段"},
                    "new": {"type": "string", "description": "替换后的内容"}
                },
                "required": ["path", "old", "new"]
            }"#,
        },
        ToolSpec {
            name: "write_file",
            description: "写入整个文件（新建，或整体重写已有文件）。只想改一处时优先用 replace_in_file。",
            parameters: r#"{
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string", "description": "完整文件内容"}
                },
                "required": ["path", "content"]
            }"#,
        },
        ToolSpec {
            name: "check_syntax",
            description: "对文件做语法与语义检查，返回问题列表（第几行、什么错误）。改完代码后用它自查。",
            parameters: r#"{
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }"#,
        },
        ToolSpec {
            name: "read_output",
            description: "读取输出面板最近的内容（运行脚本的输出与报错都在这里）。",
            parameters: r#"{"type": "object", "properties": {}}"#,
        },
    ]
}

/// 工具执行时需要的环境。
pub struct PlanCtx<'a> {
    /// 项目根目录：所有相对路径按它解析，且不允许跑出它之外。
    pub root: &'a Path,
    /// 读文件当前内容：优先编辑器里未保存的版本，取不到再读磁盘。
    pub read: &'a dyn Fn(&Path) -> Option<String>,
    /// 输出面板最近的文本。
    pub output: &'a dyn Fn() -> String,
}

/// 一次调用的执行计划。
pub enum ToolPlan {
    /// 只读工具：直接把 `content` 作为工具结果交给模型。
    Read { content: String, summary: String },
    /// 要写文件：等用户确认后由界面层落盘。
    Write {
        path: PathBuf,
        content: String,
        summary: String,
        /// 写成功后回给模型的说明。
        done: String,
    },
}

fn read_failure(summary: String, content: String) -> ToolPlan {
    ToolPlan::Read { content, summary }
}

/// 解析参数；不是合法 JSON 时返回一句给模型看的错误。
fn arguments(call: &crate::ai::client::ToolCall) -> Result<Value, String> {
    serde_json::from_str(&call.arguments)
        .map_err(|_| "参数不是合法 JSON，请重新发起调用".to_string())
}

fn text_arg(args: &Value, name: &str) -> Option<String> {
    args.get(name).and_then(|value| value.as_str()).map(str::to_string)
}

fn int_arg(args: &Value, name: &str) -> Option<usize> {
    args.get(name)
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
}

/// 执行一次工具调用，算成计划；不写盘、不改任何状态。
pub fn plan(ctx: &PlanCtx<'_>, call: &crate::ai::client::ToolCall) -> ToolPlan {
    if !specs().iter().any(|spec| spec.name == call.name) {
        return read_failure(
            format!("未知工具 {}", call.name),
            format!("错误：没有名为 `{}` 的工具", call.name),
        );
    }
    let args = match arguments(call) {
        Ok(args) => args,
        Err(error) => return read_failure("参数错误".to_string(), format!("错误：{error}")),
    };

    match call.name.as_str() {
        "read_file" => read_file(ctx, &args),
        "list_files" => list_files(ctx, &args),
        "replace_in_file" => replace_in_file(ctx, &args),
        "write_file" => write_file(ctx, &args),
        "check_syntax" => check_syntax(ctx, &args),
        "read_output" => read_output(ctx),
        other => read_failure(
            format!("未知工具 {other}"),
            format!("错误：没有名为 `{other}` 的工具"),
        ),
    }
}

fn read_file(ctx: &PlanCtx<'_>, args: &Value) -> ToolPlan {
    let Some(raw) = text_arg(args, "path") else {
        return read_failure("参数错误".to_string(), "错误：缺少 path".to_string());
    };
    let path = match resolve(ctx.root, &raw) {
        Ok(path) => path,
        Err(error) => return read_failure("路径不合法".to_string(), format!("错误：{error}")),
    };
    let Some(text) = (ctx.read)(&path) else {
        return read_failure(
            format!("读取 {raw} 失败"),
            format!("错误：读不到文件 `{raw}`"),
        );
    };

    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let start = int_arg(args, "start").unwrap_or(1).max(1);
    let end = int_arg(args, "end").unwrap_or(start + 399).min(total).max(start.min(total));
    let shown = &lines[(start - 1).min(total)..end.min(total)];

    let mut content = String::new();
    for (offset, line) in shown.iter().enumerate() {
        content.push_str(&format!("{:>5}| {line}\n", start + offset));
    }
    if content.is_empty() {
        content.push_str("（空文件）\n");
    }
    if end < total {
        content.push_str(&format!("…… 还有 {} 行，需要时用 start/end 继续读\n", total - end));
    }
    let summary = format!(
        "读取 {}（第 {}–{} 行／共 {} 行）",
        display_path(ctx.root, &path),
        start,
        end.min(total),
        total
    );
    ToolPlan::Read { content, summary }
}

fn list_files(ctx: &PlanCtx<'_>, args: &Value) -> ToolPlan {
    let dir = match text_arg(args, "dir") {
        Some(raw) if !raw.trim().is_empty() => match resolve(ctx.root, &raw) {
            Ok(path) => path,
            Err(error) => return read_failure("路径不合法".to_string(), format!("错误：{error}")),
        },
        _ => ctx.root.to_path_buf(),
    };
    let files = collect_files(ctx.root, &dir, 300);
    let summary = format!("列出 {} 个文件", files.len());
    if files.is_empty() {
        return ToolPlan::Read {
            content: "（没有文件）".to_string(),
            summary,
        };
    }
    ToolPlan::Read {
        content: files.join("\n"),
        summary,
    }
}

fn replace_in_file(ctx: &PlanCtx<'_>, args: &Value) -> ToolPlan {
    let (Some(raw), Some(old), Some(new)) = (
        text_arg(args, "path"),
        text_arg(args, "old"),
        text_arg(args, "new"),
    ) else {
        return read_failure(
            "参数错误".to_string(),
            "错误：需要 path、old、new 三个参数".to_string(),
        );
    };
    if old.is_empty() {
        return read_failure("参数错误".to_string(), "错误：old 不能为空".to_string());
    }
    let path = match resolve(ctx.root, &raw) {
        Ok(path) => path,
        Err(error) => return read_failure("路径不合法".to_string(), format!("错误：{error}")),
    };
    let Some(text) = (ctx.read)(&path) else {
        return read_failure(
            format!("读取 {raw} 失败"),
            format!("错误：读不到文件 `{raw}`"),
        );
    };
    let count = text.matches(&old).count();
    if count == 0 {
        return read_failure(
            format!("{raw} 没有匹配的原文"),
            "错误：文件里找不到这段原文，请先 read_file 看看当前内容".to_string(),
        );
    }
    if count > 1 {
        return read_failure(
            format!("{raw} 的原文不唯一"),
            format!("错误：这段原文出现了 {count} 次，请给出更长、唯一的片段"),
        );
    }
    let updated = text.replacen(&old, &new, 1);
    let display = display_path(ctx.root, &path);
    ToolPlan::Write {
        path,
        content: updated,
        summary: format!("修改 {display}（替换 1 处）"),
        done: format!("已修改 {display}（替换 1 处）"),
    }
}

fn write_file(ctx: &PlanCtx<'_>, args: &Value) -> ToolPlan {
    let (Some(raw), Some(content)) = (text_arg(args, "path"), text_arg(args, "content")) else {
        return read_failure(
            "参数错误".to_string(),
            "错误：需要 path 和 content".to_string(),
        );
    };
    let path = match resolve(ctx.root, &raw) {
        Ok(path) => path,
        Err(error) => return read_failure("路径不合法".to_string(), format!("错误：{error}")),
    };
    let display = display_path(ctx.root, &path);
    let lines = content.lines().count();
    ToolPlan::Write {
        path,
        content,
        summary: format!("写入 {display}（{lines} 行）"),
        done: format!("已写入 {display}（{lines} 行）"),
    }
}

fn check_syntax(ctx: &PlanCtx<'_>, args: &Value) -> ToolPlan {
    let Some(raw) = text_arg(args, "path") else {
        return read_failure("参数错误".to_string(), "错误：缺少 path".to_string());
    };
    let path = match resolve(ctx.root, &raw) {
        Ok(path) => path,
        Err(error) => return read_failure("路径不合法".to_string(), format!("错误：{error}")),
    };
    let Some(text) = (ctx.read)(&path) else {
        return read_failure(
            format!("读取 {raw} 失败"),
            format!("错误：读不到文件 `{raw}`"),
        );
    };
    let (report, issues) = syntax_report(&text, Some(&path));
    let display = display_path(ctx.root, &path);
    let summary = if issues == 0 {
        format!("检查 {display}：没有发现问题")
    } else {
        format!("检查 {display}：{issues} 个问题")
    };
    ToolPlan::Read {
        content: report,
        summary,
    }
}

fn read_output(ctx: &PlanCtx<'_>) -> ToolPlan {
    const TAIL_LINES: usize = 80;
    let text = (ctx.output)();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    let tail = lines[start..].join("\n");
    let summary = format!("读取输出面板（最近 {} 行）", lines.len() - start);
    ToolPlan::Read {
        content: if tail.is_empty() {
            "（输出面板是空的）".to_string()
        } else {
            tail
        },
        summary,
    }
}

/// 语法 + 语义检查，返回给模型看的文本与问题数量。
fn syntax_report(source: &str, path: Option<&Path>) -> (String, usize) {
    match crate::interpreter::parse_with_source_map(source, path) {
        Ok((program, map)) => {
            let issues = crate::interpreter::sema::analyze(&program)
                .into_iter()
                .map(|issue| map.remap(issue.line, &issue.message))
                .collect::<Vec<_>>();
            if issues.is_empty() {
                ("没有发现语法或语义问题。".to_string(), 0)
            } else {
                let text = issues
                    .iter()
                    .map(|(line, message)| format!("第 {line} 行: {message}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                (text, issues.len())
            }
        }
        Err(error) => (
            format!("第 {} 行: {}", error.line(), error.message()),
            1,
        ),
    }
}

fn collect_files(root: &Path, dir: &Path, limit: usize) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            if files.len() >= limit {
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if matches!(name.as_str(), "target" | "node_modules" | "vendor" | "build" | "dist") {
                    continue;
                }
                stack.push(path);
            } else {
                files.push(display_path(root, &path));
            }
        }
    }
    files.sort();
    files
}

/// 把相对路径解析成绝对路径，并保证不会跑到项目目录之外。
pub fn resolve(root: &Path, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("没有给路径".to_string());
    }
    let path = PathBuf::from(raw);
    let joined = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    let normalized = normalize(&joined);
    if !normalized.starts_with(normalize(root)) {
        return Err(format!("路径不在项目目录内：{raw}"));
    }
    Ok(normalized)
}

/// 目录显示用的相对路径（统一用 `/` 分隔）。
fn display_path(root: &Path, path: &Path) -> String {
    let root = normalize(root);
    path.strip_prefix(&root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 纯文本地规整 `..` 与 `.`（文件可能还不存在，不能用 canonicalize）。
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
