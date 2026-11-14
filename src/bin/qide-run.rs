//! `qide-run`：不开界面的 VB 脚本运行器。
//!
//! 供命令行、批处理与外部 agent 调用：
//!
//! ```text
//! qide-run run script.vb --json
//! qide-run check script.vb --json
//! ```
//!
//! 只编进 `interpreter` 与 `encoding` 两个模块，不链接界面代码，
//! 因此可执行文件很小，也不会带上 GPUI 的全局状态。

#[allow(dead_code)] // 只用到解释器的一部分 API
#[path = "../interpreter/mod.rs"]
mod interpreter;

#[allow(dead_code)] // 只用到解码入口
#[path = "../encoding.rs"]
mod encoding;

#[path = "../cli.rs"]
mod cli;

fn main() {
    std::process::exit(cli::main_entry());
}
