//! 主程序的命令行参数。
//!
//! 主程序是 GUI 子系统，平时双击启动不需要参数；但把 `.vb` 文件的“打开方式”
//! 指到 `first.exe` 时，Windows 会把文件路径当作第一个参数传进来，所以这里
//! 至少要把路径接住。解析部分保持纯函数，方便单元测试。

use std::path::{Path, PathBuf};

use crate::title_bar::APP_TITLE;

/// 用法说明：`--help` 与参数出错时都会显示它。
pub const USAGE: &str = "\
用法：first.exe [选项] [路径...]

路径可以是文件（打开为标签页）或文件夹（加入资源管理器）。

选项：
    -h, --help       显示本说明后退出
    -V, --version    显示版本号后退出
    --               其后的参数一律当作路径

不带路径时按上次保存的布局启动。";

/// 解析后的启动命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 正常开窗，并打开这些路径（可以为空，表示只恢复上次布局）。
    Open(Vec<String>),
    Help,
    Version,
    /// 参数有误，字符串是具体原因。
    Invalid(String),
}

/// 命令行里给出的路径在磁盘上是什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    File,
    Dir,
    Missing,
}

/// 解析命令行（不含程序名本身）。
pub fn parse<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut paths = Vec::new();
    // `--` 之后不再识别选项：文件名本身可能以 `-` 开头。
    let mut literal = false;
    for arg in args {
        let arg = arg.as_ref();
        if literal {
            paths.push(arg.to_string());
            continue;
        }
        match arg {
            "--" => literal = true,
            "-h" | "--help" | "/?" => return Command::Help,
            "-V" | "--version" => return Command::Version,
            // 单个 `-` 不算选项，当作路径处理。
            _ if arg.len() > 1 && arg.starts_with('-') => {
                return Command::Invalid(format!("未知选项 {arg}"));
            }
            _ => paths.push(arg.to_string()),
        }
    }
    Command::Open(paths)
}

/// 按磁盘现状分类：`is_file` / `is_dir` 都不成立时算作不存在。
pub fn classify(path: &Path) -> Target {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => Target::Dir,
        Ok(_) => Target::File,
        Err(_) => Target::Missing,
    }
}

/// 文件夹是否已经落在某个资源管理器根目录里（是的话不必再添加）。
pub fn covered_by_roots(roots: &[PathBuf], dir: &Path) -> bool {
    roots.iter().any(|root| dir.starts_with(root))
}

/// 版本号，例如 `Q-IDE 0.1.0`。
pub fn version() -> String {
    format!("{APP_TITLE} {}", env!("CARGO_PKG_VERSION"))
}

/// 提示信息的语气。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
}

/// 显示帮助 / 版本 / 参数错误。
///
/// 从命令行启动时（控制台父进程）写到那个控制台；从资源管理器双击启动时
/// 没有控制台，改用消息框，否则用户什么都看不到。
pub fn report(text: &str, level: Level) {
    #[cfg(windows)]
    {
        if attach_parent_console() {
            write_to_console(text);
            return;
        }
        message_box(text, level);
    }
    #[cfg(not(windows))]
    {
        let _ = level;
        println!("{text}");
    }
}

/// 挂到父进程的控制台上：GUI 子系统默认没有控制台，`println!` 会直接丢掉。
#[cfg(windows)]
fn attach_parent_console() -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    /// `ATTACH_PARENT_PROCESS`：`(DWORD)-1`。
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;

    // SAFETY: 只是附加到父进程控制台，失败返回 0（例如从资源管理器启动）。
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

/// 按 UTF-8 输出到已经附加的控制台。
///
/// 中文控制台默认是 GBK 代码页，直接写 UTF-8 字节会变成乱码；代码页属于控制台
/// 而不属于进程（cmd 也在用同一个），所以写完立刻恢复成原来的。
#[cfg(windows)]
fn write_to_console(text: &str) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleOutputCP() -> u32;
        fn SetConsoleOutputCP(code_page: u32) -> i32;
    }
    /// UTF-8 代码页。
    const UTF8_CODE_PAGE: u32 = 65001;

    // SAFETY: 只读写当前控制台的输出代码页。
    unsafe {
        let previous = GetConsoleOutputCP();
        SetConsoleOutputCP(UTF8_CODE_PAGE);
        println!("{text}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        if previous != 0 && previous != UTF8_CODE_PAGE {
            SetConsoleOutputCP(previous);
        }
    }
}

#[cfg(windows)]
fn message_box(text: &str, level: Level) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(window: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    /// `MB_OK`、`MB_ICONINFORMATION`、`MB_ICONWARNING`、`MB_SETFOREGROUND`。
    const MB_OK: u32 = 0;
    const MB_ICONINFORMATION: u32 = 0x40;
    const MB_ICONWARNING: u32 = 0x30;
    const MB_SETFOREGROUND: u32 = 0x1_0000;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let icon = match level {
        Level::Info => MB_ICONINFORMATION,
        Level::Warning => MB_ICONWARNING,
    };
    let text = wide(text);
    let caption = wide(APP_TITLE);
    // SAFETY: 两个字符串都以 0 结尾且在本调用期间有效。
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | icon | MB_SETFOREGROUND,
        );
    }
}
