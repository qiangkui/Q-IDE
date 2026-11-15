//! 输出面板里的终端：输入命令、执行并显示结果。
//!
//! 每条命令用 `cmd /C` 在后台线程执行，输出通过 mpsc 搬回界面线程。
//! `cd` / `cls` 在本地处理，因此可以连续切换目录、清屏。

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::InteractiveElement as _;
use gpui_kit::*;

use crate::encoding;
use crate::theme::{self as ui_theme, Palette};

/// 终端最多保留多少行，避免命令输出把界面拖慢。
const MAX_LINES: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineKind {
    /// 用户输入的命令。
    Command,
    /// 命令的标准输出。
    Output,
    /// 命令的标准错误或本地错误。
    Error,
    /// 终端自身的提示（例如退出码）。
    System,
}

struct TerminalLine {
    kind: LineKind,
    text: String,
}

enum TerminalEvent {
    Chunk { stderr: bool, text: String },
    Finished(Option<i32>),
}

/// 终端模型与视图。
pub struct TerminalState {
    input: Entity<InputState>,
    lines: Vec<TerminalLine>,
    /// 还没遇到换行的一段输出；命令结束时再刷出来。
    pending: String,
    pending_kind: LineKind,
    cwd: PathBuf,
    running: bool,
    scroll: ScrollHandle,
    /// 已执行过的命令，供上下方向键翻阅。
    history: Vec<String>,
    /// 当前翻阅到的历史位置；等于 `history.len()` 表示正在输入新命令。
    history_index: usize,
    /// 翻阅历史前用户正在输入的内容。
    history_draft: String,
}

impl TerminalState {
    pub fn new(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        Self {
            input,
            lines: Vec::new(),
            pending: String::new(),
            pending_kind: LineKind::Output,
            cwd,
            running: false,
            scroll: ScrollHandle::new(),
            history: Vec::new(),
            history_index: 0,
            history_draft: String::new(),
        }
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.lines.clear();
        self.pending.clear();
        cx.notify();
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// 记下刚执行的命令，并把历史游标放回末尾。
    fn remember_command(&mut self, command: &str) {
        if self.history.last().map(String::as_str) != Some(command) {
            self.history.push(command.to_string());
        }
        self.history_index = self.history.len();
        self.history_draft.clear();
    }

    fn history_prev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index == self.history.len() {
            self.history_draft = self.input.read(cx).value().to_string();
        }
        if self.history_index > 0 {
            self.history_index -= 1;
        }
        let text = self.history[self.history_index].clone();
        self.input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        cx.notify();
    }

    fn history_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history_index >= self.history.len() {
            return;
        }
        self.history_index += 1;
        let text = if self.history_index == self.history.len() {
            std::mem::take(&mut self.history_draft)
        } else {
            self.history[self.history_index].clone()
        };
        self.input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        cx.notify();
    }

    fn scroll_to_bottom(&self) {
        self.scroll.scroll_to_bottom();
    }

    fn push_line(&mut self, kind: LineKind, text: impl Into<String>) {
        self.lines.push(TerminalLine {
            kind,
            text: text.into(),
        });
        self.trim_lines();
    }

    fn trim_lines(&mut self) {
        if self.lines.len() > MAX_LINES {
            let excess = self.lines.len() - MAX_LINES;
            self.lines.drain(0..excess);
        }
    }

    fn flush_pending(&mut self) {
        if !self.pending.is_empty() {
            let text = std::mem::take(&mut self.pending);
            self.push_line(self.pending_kind, text);
        }
    }

    fn append_chunk(&mut self, stderr: bool, text: &str) {
        let kind = if stderr {
            LineKind::Error
        } else {
            LineKind::Output
        };
        if self.pending.is_empty() {
            self.pending_kind = kind;
        }
        for ch in text.chars() {
            match ch {
                '\r' | '\n' => {
                    self.flush_pending();
                    self.pending_kind = kind;
                }
                _ => self.pending.push(ch),
            }
        }
    }

    fn run_command(&mut self, command: String, cx: &mut Context<Self>) {
        let command = command.trim().to_string();
        if command.is_empty() {
            return;
        }
        self.remember_command(&command);
        self.flush_pending();
        self.push_line(
            LineKind::Command,
            format!("{}> {}", self.cwd.display(), command),
        );
        self.scroll_to_bottom();

        let lower = command.to_ascii_lowercase();
        if lower == "cls" || lower == "clear" {
            self.lines.clear();
            self.pending.clear();
            cx.notify();
            return;
        }
        if lower == "cd" || lower == "pwd" {
            self.push_line(LineKind::Output, self.cwd.display().to_string());
            cx.notify();
            return;
        }
        if let Some(raw) = lower.strip_prefix("cd ") {
            let raw = command[raw.len()..].trim().trim_matches('"');
            let path = PathBuf::from(raw);
            let target = if path.is_absolute() {
                path
            } else {
                self.cwd.join(path)
            };
            if target.is_dir() {
                self.cwd = target.canonicalize().unwrap_or(target);
                self.push_line(LineKind::Output, self.cwd.display().to_string());
            } else {
                self.push_line(LineKind::Error, format!("目录不存在：{}", target.display()));
            }
            cx.notify();
            return;
        }

        if self.running {
            self.push_line(LineKind::System, "上一条命令还在运行，请稍候。");
            cx.notify();
            return;
        }
        self.running = true;

        let cwd = self.cwd.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut process = std::process::Command::new("cmd");
            process.arg("/C").arg(&command).current_dir(&cwd);
            // 桌面程序没有控制台；不加这个标志 cmd 会闪一下黑框。
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                process.creation_flags(0x0800_0000);
            }
            match process.output() {
                Ok(output) => {
                    let stdout = encoding::decode(&output.stdout).text;
                    if !stdout.is_empty() {
                        let _ = sender.send(TerminalEvent::Chunk {
                            stderr: false,
                            text: stdout,
                        });
                    }
                    let stderr = encoding::decode(&output.stderr).text;
                    if !stderr.is_empty() {
                        let _ = sender.send(TerminalEvent::Chunk {
                            stderr: true,
                            text: stderr,
                        });
                    }
                    let _ = sender.send(TerminalEvent::Finished(output.status.code()));
                }
                Err(error) => {
                    let _ = sender.send(TerminalEvent::Chunk {
                        stderr: true,
                        text: format!("无法执行命令：{error}\n"),
                    });
                    let _ = sender.send(TerminalEvent::Finished(None));
                }
            }
        });

        cx.spawn(async move |this, cx| {
            loop {
                let mut chunks = Vec::new();
                let mut outcome = None;
                loop {
                    match receiver.try_recv() {
                        Ok(TerminalEvent::Chunk { stderr, text }) => chunks.push((stderr, text)),
                        Ok(TerminalEvent::Finished(code)) => {
                            outcome = Some(code);
                            break;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            outcome = Some(None);
                            break;
                        }
                    }
                }

                let finished = outcome.is_some();
                if !chunks.is_empty() || finished {
                    let _ = this.update(cx, |terminal, cx| {
                        for (stderr, text) in chunks {
                            terminal.append_chunk(stderr, &text);
                        }
                        if finished {
                            terminal.flush_pending();
                            terminal.running = false;
                            if let Some(Some(code)) = outcome
                                && code != 0
                            {
                                terminal.push_line(
                                    LineKind::System,
                                    format!("命令结束，退出码 {code}"),
                                );
                            }
                        }
                        terminal.scroll_to_bottom();
                        cx.notify();
                    });
                }

                if finished {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
            }
        })
        .detach();

        cx.notify();
    }
}

impl Render for TerminalState {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_theme(cx);
        let prompt = format!("{}>", self.cwd.display());
        let rows = self.lines.iter().map(|line| {
            let color = match line.kind {
                LineKind::Command => palette.command,
                LineKind::Output => palette.sidebar_text,
                LineKind::Error => palette.error,
                LineKind::System => palette.muted_text,
            };
            div()
                .whitespace_nowrap()
                .text_xs()
                .text_color(rgb(color))
                .child(line.text.clone())
        });

        v_flex()
            .size_full()
            .bg(rgb(palette.side_panel_bg))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus_input(window, cx)),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => {
                        let command = this.input.read(cx).value().to_string();
                        this.input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.run_command(command, cx);
                        cx.stop_propagation();
                    }
                    "up" => {
                        this.history_prev(window, cx);
                        cx.stop_propagation();
                    }
                    "down" => {
                        this.history_next(window, cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .child(
                div()
                    .id("terminal-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .font_family(ui_theme::MONO_FONT)
                    .text_size(px(12.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .child(
                        v_flex()
                            .children(rows)
                            // 输入行跟着输出一起滚动，就像 Windows 终端里的提示符。
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .whitespace_nowrap()
                                            .text_xs()
                                            .text_color(rgb(palette.success))
                                            .child(prompt),
                                    )
                                    .child(
                                        div().flex_1().min_w_0().child(
                                            Input::new(&self.input)
                                                .xsmall()
                                                .w_full()
                                                .appearance(false)
                                                .focus_ring(false),
                                        ),
                                    ),
                            ),
                    ),
            )
    }
}
