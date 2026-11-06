//! 命令面板：命令清单与按名字模糊匹配。
//!
//! 这里只放纯数据与匹配逻辑（便于单测），真正执行由 `CodeWindow::run_command` 按 `id` 分发。

use crate::workspace;

/// 命令面板里的一条命令。
pub struct Command {
    /// 执行用的标识，`CodeWindow::run_command` 里按它分发。
    pub id: &'static str,
    /// 显示与匹配用的名字。
    pub label: &'static str,
    /// 快捷键提示（没有就空串）。
    pub shortcut: &'static str,
}

/// 命令面板里能搜到的全部命令。
pub const COMMANDS: &[Command] = &[
    Command {
        id: "file.new",
        label: "新建文件",
        shortcut: "Ctrl+N",
    },
    Command {
        id: "file.open",
        label: "打开文件",
        shortcut: "Ctrl+O",
    },
    Command {
        id: "file.save",
        label: "保存",
        shortcut: "Ctrl+S",
    },
    Command {
        id: "file.save-as",
        label: "另存为",
        shortcut: "Ctrl+Shift+S",
    },
    Command {
        id: "file.save-all",
        label: "保存全部",
        shortcut: "Ctrl+Alt+S",
    },
    Command {
        id: "file.close",
        label: "关闭编辑器",
        shortcut: "",
    },
    Command {
        id: "file.close-all",
        label: "关闭所有编辑器",
        shortcut: "",
    },
    Command {
        id: "file.reopen",
        label: "重新打开关闭的标签",
        shortcut: "Ctrl+Shift+T",
    },
    Command {
        id: "edit.find",
        label: "查找",
        shortcut: "Ctrl+F",
    },
    Command {
        id: "edit.replace",
        label: "替换",
        shortcut: "Ctrl+H",
    },
    Command {
        id: "edit.comment",
        label: "切换行注释",
        shortcut: "Ctrl+/",
    },
    Command {
        id: "edit.move-up",
        label: "上移行",
        shortcut: "Alt+↑",
    },
    Command {
        id: "edit.move-down",
        label: "下移行",
        shortcut: "Alt+↓",
    },
    Command {
        id: "edit.indent",
        label: "增加缩进",
        shortcut: "Tab",
    },
    Command {
        id: "edit.outdent",
        label: "减少缩进",
        shortcut: "Shift+Tab",
    },
    Command {
        id: "edit.select-all",
        label: "全选",
        shortcut: "Ctrl+A",
    },
    Command {
        id: "goto.file",
        label: "转到文件",
        shortcut: "Ctrl+P",
    },
    Command {
        id: "goto.line",
        label: "转到行或符号",
        shortcut: "Ctrl+G",
    },
    Command {
        id: "goto.definition",
        label: "转到定义",
        shortcut: "F12",
    },
    Command {
        id: "goto.references",
        label: "查找引用",
        shortcut: "Shift+F12",
    },
    Command {
        id: "view.explorer",
        label: "资源管理器",
        shortcut: "",
    },
    Command {
        id: "view.outline",
        label: "代码大纲",
        shortcut: "",
    },
    Command {
        id: "view.search",
        label: "项目内搜索面板",
        shortcut: "Ctrl+Shift+F",
    },
    Command {
        id: "view.output",
        label: "输出面板",
        shortcut: "",
    },
    Command {
        id: "view.status-bar",
        label: "状态栏",
        shortcut: "",
    },
    Command {
        id: "view.theme",
        label: "切换主题",
        shortcut: "",
    },
    Command {
        id: "view.fullscreen",
        label: "全屏",
        shortcut: "",
    },
    Command {
        id: "run.start",
        label: "运行脚本",
        shortcut: "F5",
    },
    Command {
        id: "run.check",
        label: "检查语法",
        shortcut: "",
    },
    Command {
        id: "run.stop",
        label: "停止脚本",
        shortcut: "Shift+F5",
    },
    Command {
        id: "run.debug",
        label: "调试运行",
        shortcut: "Ctrl+F5",
    },
    Command {
        id: "run.pause",
        label: "暂停调试",
        shortcut: "",
    },
    Command {
        id: "run.resume",
        label: "继续调试",
        shortcut: "F11",
    },
    Command {
        id: "run.step",
        label: "单步调试",
        shortcut: "F10",
    },
    Command {
        id: "run.to-cursor",
        label: "运行到光标处",
        shortcut: "Ctrl+F10",
    },
    Command {
        id: "debug.toggle-breakpoint",
        label: "切换断点",
        shortcut: "F9",
    },
    Command {
        id: "debug.clear-breakpoints",
        label: "清除所有断点",
        shortcut: "Ctrl+Shift+F9",
    },
];

/// 按输入的文字找最匹配的命令；空输入或不匹配返回 `None`。
pub fn best_command(query: &str) -> Option<&'static Command> {
    if query.trim().is_empty() {
        return None;
    }
    let labels: Vec<String> = COMMANDS
        .iter()
        .map(|command| command.label.to_string())
        .collect();
    let label = workspace::best_match(query, &labels)?;
    COMMANDS
        .iter()
        .find(|command| command.label == label.as_str())
}
