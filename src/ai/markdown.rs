//! 极简 Markdown 分段：把 AI 回复拆成「说明文字」和「围栏代码块」。
//!
//! 不是完整的 Markdown 解析器，只认 ``` / ~~~ 围栏——够用来把长代码折叠起来，
//! 说明文字仍然整段交给 Markdown 渲染。纯逻辑，可单测、可移植。

/// 回复里的一段内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    /// 普通说明文字（可能是 Markdown）。
    Text(String),
    /// 围栏代码块。
    Code {
        /// 围栏后面写的语言（可能为空）。
        language: String,
        /// 代码正文，行之间用 `\n` 连接，末尾不带换行。
        code: String,
        /// 代码块是否已经收尾；流式输出时最后一块可能还没写完。
        closed: bool,
    },
}

/// 代码块的行数（空代码算 0 行）。
pub fn line_count(code: &str) -> usize {
    if code.is_empty() {
        0
    } else {
        code.lines().count()
    }
}

/// 按围栏代码块把正文切开；没有围栏时返回一个 [`Segment::Text`]。
pub fn split(content: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut code: Option<(String, Vec<String>)> = None;

    for line in content.lines() {
        if is_fence(line) {
            match code.take() {
                Some((language, lines)) => segments.push(Segment::Code {
                    language,
                    code: lines.join("\n"),
                    closed: true,
                }),
                None => {
                    push_text(&mut segments, &mut text);
                    code = Some((fence_language(line), Vec::new()));
                }
            }
            continue;
        }

        match &mut code {
            Some((_, lines)) => lines.push(line.to_string()),
            None => {
                text.push_str(line);
                text.push('\n');
            }
        }
    }

    match code {
        Some((language, lines)) => segments.push(Segment::Code {
            language,
            code: lines.join("\n"),
            closed: false,
        }),
        None => push_text(&mut segments, &mut text),
    }
    segments
}

fn push_text(segments: &mut Vec<Segment>, text: &mut String) {
    if !text.trim().is_empty() {
        segments.push(Segment::Text(std::mem::take(text)));
    } else {
        text.clear();
    }
}

/// 行首（允许缩进）是 ``` 或 ~~~ 就算围栏。
fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// 围栏后面的语言名，取第一个词。
fn fence_language(line: &str) -> String {
    line.trim_start()
        .trim_start_matches(['`', '~'])
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}
