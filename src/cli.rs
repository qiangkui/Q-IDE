//! `qide-run` 的命令行逻辑：解析参数、跑脚本、按行输出事件。
//!
//! 这一层只依赖 [`crate::interpreter`] 与 [`crate::encoding`]，不碰界面代码：
//! 命令行、外部 agent、将来可能的管道服务都能复用它。
//!
//! 输出有两种形态：
//! - 纯文本（默认）：脚本的 `Print` / `Debug.Print` 走 stdout，诊断走 stderr，
//!   这样 `qide-run run a.vb > out.txt` 只拿到脚本输出；
//! - `--json`：每一行一个 JSON 对象（JSONL），供程序解析。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::interpreter::{self, RunOptions};

/// 成功。
pub const EXIT_OK: i32 = 0;
/// 语法错误（`check` 也用它表示有问题）。
pub const EXIT_SYNTAX: i32 = 2;
/// 运行时错误。
pub const EXIT_RUNTIME: i32 = 3;
/// 超时或被 Ctrl+C 中断。
pub const EXIT_INTERRUPTED: i32 = 4;
/// 读不到脚本、线程起不来、脚本线程崩溃等 I/O 或内部错误。
pub const EXIT_IO: i32 = 5;
/// 参数用法错误。
pub const EXIT_USAGE: i32 = 64;

/// 中断标志：Ctrl+C 与 `--timeout` 都置它，解释器在语句边界检查。
static INTERRUPT: AtomicBool = AtomicBool::new(false);

const USAGE: &str = "\
qide-run —— 不开界面的 VB 脚本运行器

用法：
  qide-run run <脚本> [选项]      运行脚本
  qide-run check <脚本> [选项]    只做语法与语义检查（不执行）
  qide-run <脚本> [选项]          等价于 run（给文件关联用）

选项：
  --json                按 JSONL 输出事件（供外部 agent / 程序解析）
  -q, --quiet           不输出脚本的 Print / Debug.Print
  --max-steps <N>       覆盖步数上限（默认 20000000）
  --max-depth <N>       覆盖过程递归上限（默认 200）
  --timeout <时长>      超时后中断脚本，例如 30s、500ms、2m（默认不限制）
  --include-dir <目录>  #Include 在脚本目录找不到时再搜这里
  --arg <文本>          脚本里 Command() 返回的参数
  --ui / --no-ui        允许 / 禁止 MsgBox、InputBox 弹窗
                        （默认：人直接运行时允许；--json 或输出被重定向时禁止）
  --pause / --no-pause  结束时等待回车（默认规则同上；双击运行会自动等待）
  -h, --help            显示本帮助
  -V, --version         显示版本
";

/// 子命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// 运行脚本。
    Run,
    /// 只解析 + 语义检查。
    Check,
}

/// 解析好的一组参数。
#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    pub command: Command,
    pub script: PathBuf,
    pub json: bool,
    pub quiet: bool,
    pub max_steps: Option<u64>,
    pub max_depth: Option<usize>,
    pub timeout: Option<Duration>,
    pub include_dir: Option<PathBuf>,
    pub command_line: String,
    /// `--ui` / `--no-ui`；`None` 表示按「是不是人直接运行」决定。
    pub ui: Option<bool>,
    /// `--pause` / `--no-pause`；`None` 表示按「是不是人直接运行」决定。
    pub pause: Option<bool>,
}

impl Args {
    /// 解析后用：是否允许弹窗。
    pub fn allow_ui(&self) -> bool {
        self.ui.unwrap_or(false)
    }
}

/// 解析结果：要么执行，要么只是看帮助/版本。
#[derive(Clone, Debug, PartialEq)]
pub enum Invocation {
    Execute(Args),
    Help,
    Version,
}

/// 解析命令行参数（不含程序名本身）。
pub fn parse_args(args: &[String]) -> Result<Invocation, String> {
    let mut command = None;
    let mut script = None;
    let mut json = false;
    let mut quiet = false;
    let mut max_steps = None;
    let mut max_depth = None;
    let mut timeout = None;
    let mut include_dir = None;
    let mut command_line = String::new();
    let mut ui = None;
    let mut pause = None;

    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => return Ok(Invocation::Help),
            "-V" | "--version" => return Ok(Invocation::Version),
            "--json" => json = true,
            "-q" | "--quiet" => quiet = true,
            "--ui" => ui = Some(true),
            "--no-ui" => ui = Some(false),
            "--pause" => pause = Some(true),
            "--no-pause" => pause = Some(false),
            "--max-steps" => {
                let value = take_value(args, &mut index, arg)?;
                max_steps = Some(parse_positive(&value, arg)?);
            }
            "--max-depth" => {
                let value = take_value(args, &mut index, arg)?;
                max_depth = Some(parse_positive(&value, arg)? as usize);
            }
            "--timeout" => {
                let value = take_value(args, &mut index, arg)?;
                timeout = Some(parse_duration(&value)?);
            }
            "--include-dir" => {
                let value = take_value(args, &mut index, arg)?;
                include_dir = Some(PathBuf::from(value));
            }
            "--arg" => command_line = take_value(args, &mut index, arg)?,
            other if other.starts_with('-') => {
                return Err(format!("无法识别的选项 `{other}`"));
            }
            other => match command {
                None => match other {
                    "run" => command = Some(Command::Run),
                    "check" => command = Some(Command::Check),
                    // 文件关联只传一个路径：`qide-run "脚本.vb"` 等价于 `run 脚本.vb`。
                    _ if looks_like_script(other) => {
                        command = Some(Command::Run);
                        script = Some(PathBuf::from(other));
                    }
                    _ => {
                        return Err(format!("未知的子命令 `{other}`（只支持 run / check）"));
                    }
                },
                Some(_) => {
                    if script.is_none() {
                        script = Some(PathBuf::from(other));
                    } else {
                        return Err(format!("多余的参数 `{other}`"));
                    }
                }
            },
        }
        index += 1;
    }

    let Some(command) = command else {
        return Err("缺少子命令（run 或 check）".to_string());
    };
    let Some(script) = script else {
        return Err("缺少脚本路径".to_string());
    };
    Ok(Invocation::Execute(Args {
        command,
        script,
        json,
        quiet,
        max_steps,
        max_depth,
        timeout,
        include_dir,
        command_line,
        ui,
        pause,
    }))
}

/// 是不是一个脚本路径（文件关联直接传路径时用来和子命令区分）。
fn looks_like_script(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [".vb", ".vbs", ".bas", ".vba"]
        .iter()
        .any(|extension| lower.ends_with(extension))
        || std::path::Path::new(text).is_file()
}

fn take_value(args: &[String], index: &mut usize, option: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("`{option}` 后面缺少值"))
}

fn parse_positive(value: &str, option: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("`{option}` 需要一个正整数，实际是 `{value}`"))
}

/// 解析时长：`30`/`30s` 是秒，`500ms` 是毫秒，`2m` 是分钟。
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let (number, scale) = if let Some(rest) = text.strip_suffix("ms") {
        (rest, 1.0)
    } else if let Some(rest) = text.strip_suffix('s') {
        (rest, 1_000.0)
    } else if let Some(rest) = text.strip_suffix('m') {
        (rest, 60_000.0)
    } else {
        (text, 1_000.0)
    };
    let value = number
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("无法解析时长 `{text}`"))?;
    let millis = value * scale;
    if !millis.is_finite() || millis <= 0.0 {
        return Err(format!("时长必须大于 0：`{text}`"));
    }
    Ok(Duration::from_millis(millis as u64))
}

/// 进程入口：返回退出码。
pub fn main_entry() -> i32 {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    setup_console();
    install_ctrl_c_handler();
    // 人直接双击/在控制台里跑：允许弹窗，并且结束时等一次回车
    // （否则双击打开 .vb 时窗口一闪就没了，看不到任何输出）。
    let json = raw.iter().any(|arg| arg == "--json");
    let interactive = !json && owned_console();
    let mut parsed_pause = None;

    let code = match parse_args(&raw) {
        Ok(Invocation::Help) => {
            print!("{USAGE}");
            let _ = std::io::stdout().flush();
            EXIT_OK
        }
        Ok(Invocation::Version) => {
            println!("qide-run {}", env!("CARGO_PKG_VERSION"));
            EXIT_OK
        }
        Ok(Invocation::Execute(mut args)) => {
            parsed_pause = args.pause;
            args.ui = Some(args.ui.unwrap_or(interactive));
            match args.command {
                Command::Run => run_command(&args),
                Command::Check => check_command(&args),
            }
        }
        Err(message) => {
            eprintln!("参数错误：{message}\n\n{USAGE}");
            EXIT_USAGE
        }
    };
    // 参数错误也停一下：双击时否则只会闪一个窗口，用户看不到哪里写错了。
    if parsed_pause.unwrap_or_else(|| pause_requested(&raw, interactive)) {
        wait_for_enter();
    }
    code
}

/// 结束时要等回车吗：显式开关优先，否则按「是不是人直接运行」。
fn pause_requested(raw: &[String], interactive: bool) -> bool {
    if raw.iter().any(|arg| arg == "--no-pause") {
        return false;
    }
    if raw.iter().any(|arg| arg == "--pause") {
        return true;
    }
    interactive
}

/// 是不是「人直接运行」：stdout 连着控制台（没有被管道接走），
/// 并且这个控制台是给自己新开的（双击启动时控制台里只有自己；
/// 从 cmd 启动时还有 cmd 进程）。
fn owned_console() -> bool {
    #[cfg(windows)]
    {
        use std::ffi::c_void;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetStdHandle(kind: u32) -> *mut c_void;
            fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
            fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        }
        /// `STD_OUTPUT_HANDLE`：`(DWORD)-11`。
        const STD_OUTPUT_HANDLE: u32 = u32::MAX - 10;

        let mut mode = 0u32;
        // SAFETY: 只查询句柄属性；句柄无效时返回 0，正好表示「输出被重定向」。
        unsafe {
            if GetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), &mut mode) == 0 {
                return false;
            }
            let mut buffer = [0u32; 8];
            let count = GetConsoleProcessList(buffer.as_mut_ptr(), buffer.len() as u32);
            count == 1
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 结束后等一次回车，避免双击运行时窗口一闪就关掉。
fn wait_for_enter() {
    eprintln!();
    eprint!("按回车键退出…");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

/// 跑一次脚本。
fn run_command(args: &Args) -> i32 {
    let source = match read_script(&args.script) {
        Ok(text) => text,
        Err(message) => {
            eprintln!("{message}");
            return EXIT_IO;
        }
    };
    let path = absolute(&args.script);
    let sink = Arc::new(Sink::new(args.json, args.quiet));
    sink.event(
        json!({
            "type": "start",
            "script": path.display().to_string(),
            "command_line": args.command_line,
        }),
        Some(format!("> 运行 {}", path.display())),
    );

    let mut options = RunOptions {
        script_path: Some(path.clone()),
        include_dir: args.include_dir.clone(),
        allow_ui: args.allow_ui(),
        command_line: args.command_line.clone(),
        ..RunOptions::default()
    };
    if let Some(steps) = args.max_steps {
        options.max_steps = steps;
    }
    if let Some(depth) = args.max_depth {
        options.max_depth = depth;
    }

    let (sender, receiver) = mpsc::channel::<(i32, u64)>();
    let worker_sink = sink.clone();
    let spawned = interpreter::spawn_script_thread(move || {
        let started = Instant::now();
        let mut emit = |line: &str| worker_sink.line(line);
        let result = interpreter::run_with(&source, &mut emit, &INTERRUPT, options);
        let elapsed = started.elapsed().as_millis() as u64;
        // 被中断时不发 `finished`：最终事件由主线程统一发，才能区分超时与 Ctrl+C。
        let code = if INTERRUPT.load(Ordering::SeqCst) {
            EXIT_INTERRUPTED
        } else {
            worker_sink.finished(&result, elapsed);
            exit_code(&result)
        };
        let _ = sender.send((code, elapsed));
    });
    if let Err(error) = spawned {
        eprintln!("无法启动脚本线程：{error}");
        return EXIT_IO;
    }

    let limit = args.timeout;
    let mut timed_out = false;
    let (code, elapsed) = match limit {
        None => match receiver.recv() {
            Ok(outcome) => outcome,
            Err(_) => return report_dead_worker(&sink),
        },
        Some(limit) => match receiver.recv_timeout(limit) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Disconnected) => return report_dead_worker(&sink),
            Err(RecvTimeoutError::Timeout) => {
                // 先让脚本在语句边界自己收尾，收不了就由进程退出兜底。
                timed_out = true;
                INTERRUPT.store(true, Ordering::SeqCst);
                receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap_or((EXIT_INTERRUPTED, limit.as_millis() as u64))
            }
        },
    };

    if code == EXIT_INTERRUPTED {
        let (plain, reason) = if timed_out {
            (
                format!(
                    "脚本超时（超过 {}），已中断",
                    format_duration(limit.unwrap_or_default())
                ),
                "timeout",
            )
        } else {
            ("脚本已中断".to_string(), "signal")
        };
        sink.event(
            json!({
                "type": "interrupted",
                "reason": reason,
                "timeout_ms": limit.map(|value| value.as_millis() as u64),
                "elapsed_ms": elapsed,
            }),
            Some(plain),
        );
    }
    code
}

/// 脚本线程没回传结果就退出了（多半是解释器内部 panic）。
fn report_dead_worker(sink: &Sink) -> i32 {
    sink.event(
        json!({
            "type": "finished", "ok": false, "kind": "internal",
            "message": "脚本线程异常退出（可能是解释器内部错误）",
        }),
        Some("脚本线程异常退出（可能是解释器内部错误）".to_string()),
    );
    EXIT_IO
}

/// 只做语法与语义检查。
fn check_command(args: &Args) -> i32 {
    let source = match read_script(&args.script) {
        Ok(text) => text,
        Err(message) => {
            eprintln!("{message}");
            return EXIT_IO;
        }
    };
    let path = absolute(&args.script);
    let sink = Sink::new(args.json, args.quiet);
    match interpreter::parse_with_include_dir(
        &source,
        Some(&path),
        args.include_dir.as_deref(),
    ) {
        Ok((program, map)) => {
            let issues: Vec<(usize, String)> = interpreter::sema::analyze(&program)
                .into_iter()
                .map(|issue| map.remap(issue.line, &issue.message))
                .collect();
            for (line, message) in &issues {
                sink.event(
                    json!({"type": "issue", "line": line, "message": message}),
                    Some(format!("第 {line} 行: {message}")),
                );
            }
            sink.event(
                json!({
                    "type": "checked",
                    "ok": issues.is_empty(),
                    "issues": issues.len(),
                    "script": path.display().to_string(),
                }),
                Some(if issues.is_empty() {
                    format!("{}：没有发现问题", path.display())
                } else {
                    format!("{}：{} 个问题", path.display(), issues.len())
                }),
            );
            if issues.is_empty() {
                EXIT_OK
            } else {
                EXIT_SYNTAX
            }
        }
        Err(interpreter::Error::Parse(error)) => {
            sink.event(
                json!({"type": "finished", "ok": false, "kind": "parse", "line": error.line, "message": error.message}),
                Some(format!("第 {} 行语法错误: {}", error.line, error.message)),
            );
            EXIT_SYNTAX
        }
        Err(interpreter::Error::Runtime(error)) => {
            sink.event(
                json!({"type": "finished", "ok": false, "kind": "runtime", "line": error.line, "message": error.message, "number": error.number}),
                Some(format!("第 {} 行运行时错误: {}", error.line, error.message)),
            );
            EXIT_RUNTIME
        }
    }
}

fn exit_code(result: &Result<(), interpreter::Error>) -> i32 {
    match result {
        Ok(()) => EXIT_OK,
        Err(interpreter::Error::Parse(_)) => EXIT_SYNTAX,
        Err(interpreter::Error::Runtime(_)) => EXIT_RUNTIME,
    }
}

/// 读脚本并按 BOM / UTF-8 / GBK 探测解码（与编辑器打开文件一致）。
fn read_script(path: &Path) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("读不到脚本 {}：{error}", path.display()))?;
    Ok(crate::encoding::decode(&bytes).text)
}

/// 相对路径变成绝对路径：解释器按 `script_path` 的父目录解析相对路径与 `App.Path`。
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn format_duration(value: Duration) -> String {
    let millis = value.as_millis();
    if millis.is_multiple_of(60_000) {
        format!("{} 分钟", millis / 60_000)
    } else if millis.is_multiple_of(1_000) {
        format!("{} 秒", millis / 1_000)
    } else {
        format!("{millis} 毫秒")
    }
}

/// 输出汇聚点：脚本线程写每一行，主线程写最终事件，两边共用一个 stdout 锁。
struct Sink {
    json: bool,
    quiet: bool,
    out: Mutex<std::io::Stdout>,
}

impl Sink {
    fn new(json: bool, quiet: bool) -> Self {
        Self {
            json,
            quiet,
            out: Mutex::new(std::io::stdout()),
        }
    }

    /// 脚本的一行输出。
    fn line(&self, text: &str) {
        if self.quiet {
            return;
        }
        self.write(json!({"type": "line", "text": text}), Some(text));
    }

    /// 一条事件：`json` 模式写 JSONL，否则写给人看的 `plain` 文本（没有就跳过）。
    fn event(&self, value: Value, plain: Option<String>) {
        self.write(value, plain.as_deref());
    }

    /// 一轮结束：成功或带错误号失败。
    fn finished(&self, result: &Result<(), interpreter::Error>, elapsed_ms: u64) {
        match result {
            Ok(()) => {
                let plain = format!("脚本执行结束（用时 {} 秒）", elapsed_ms as f64 / 1000.0);
                self.write(
                    json!({"type": "finished", "ok": true, "elapsed_ms": elapsed_ms}),
                    Some(&plain),
                );
            }
            Err(interpreter::Error::Parse(error)) => {
                let plain = format!("第 {} 行语法错误: {}", error.line, error.message);
                self.write(
                    json!({
                        "type": "finished", "ok": false, "kind": "parse",
                        "line": error.line, "message": error.message, "elapsed_ms": elapsed_ms,
                    }),
                    Some(&plain),
                );
            }
            Err(interpreter::Error::Runtime(error)) => {
                let plain = format!(
                    "第 {} 行运行时错误（{}）: {}",
                    error.line, error.number, error.message
                );
                self.write(
                    json!({
                        "type": "finished", "ok": false, "kind": "runtime",
                        "line": error.line, "message": error.message,
                        "number": error.number, "elapsed_ms": elapsed_ms,
                    }),
                    Some(&plain),
                );
            }
        }
    }

    fn write(&self, value: Value, plain: Option<&str>) {
        if self.json {
            let mut out = self.out.lock().unwrap_or_else(|error| error.into_inner());
            let _ = writeln!(out, "{value}");
            let _ = out.flush();
        } else if let Some(text) = plain {
            // 诊断信息走 stderr，保证 stdout 里只有脚本自己打印的内容。
            if value.get("type").and_then(Value::as_str) == Some("line") {
                let mut out = self.out.lock().unwrap_or_else(|error| error.into_inner());
                let _ = writeln!(out, "{text}");
                let _ = out.flush();
            } else {
                eprintln!("{text}");
            }
        }
    }
}

/// 让控制台用 UTF-8：否则中文输出在 GBK 代码页的控制台里是乱码。
fn setup_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetConsoleOutputCP(code_page: u32) -> i32;
            fn SetConsoleCP(code_page: u32) -> i32;
        }
        // SAFETY: 两个函数只改当前控制台的代码页；没有控制台时直接失败，无需处理。
        unsafe {
            SetConsoleOutputCP(65001);
            SetConsoleCP(65001);
        }
    }
}

/// Ctrl+C：置中断标志，让解释器在语句边界停下并正常收尾。
fn install_ctrl_c_handler() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetConsoleCtrlHandler(
                handler: Option<unsafe extern "system" fn(u32) -> i32>,
                add: i32,
            ) -> i32;
        }
        unsafe extern "system" fn handler(_event: u32) -> i32 {
            INTERRUPT.store(true, Ordering::SeqCst);
            1
        }
        // SAFETY: 处理器只写一个静态原子量，可以在任意线程调用。
        unsafe {
            SetConsoleCtrlHandler(Some(handler), 1);
        }
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;

