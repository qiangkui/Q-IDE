//! 快捷键一览：帮助菜单里的「快捷键一览」对话框用这份表。
//!
//! 表格与 `docs/快捷键.md` 保持同一份内容；`commands::COMMANDS` 里带快捷键的命令
//! 必须原样出现在这里（`shortcuts_tests` 会逐条比对），这样命令面板与一览表不会各说各话。

/// 一览表里的一行：按键 + 功能。
pub struct Entry {
    pub keys: &'static str,
    pub action: &'static str,
}

/// 一览表的一个分区。
pub struct Section {
    pub title: &'static str,
    pub entries: &'static [Entry],
}

const FILE_ENTRIES: &[Entry] = &[
    Entry {
        keys: "Ctrl+N",
        action: "新建文件",
    },
    Entry {
        keys: "Ctrl+O",
        action: "打开文件",
    },
    Entry {
        keys: "Ctrl+S",
        action: "保存",
    },
    Entry {
        keys: "Ctrl+Shift+S",
        action: "另存为",
    },
    Entry {
        keys: "Ctrl+Alt+S",
        action: "保存全部",
    },
    Entry {
        keys: "Ctrl+Shift+T",
        action: "重新打开关闭的标签",
    },
];

const EDIT_ENTRIES: &[Entry] = &[
    Entry {
        keys: "Ctrl+Z / Ctrl+Y",
        action: "撤销 / 重做",
    },
    Entry {
        keys: "Ctrl+X / Ctrl+C / Ctrl+V",
        action: "剪切 / 复制 / 粘贴",
    },
    Entry {
        keys: "Ctrl+A",
        action: "全选",
    },
    Entry {
        keys: "Ctrl+F",
        action: "查找",
    },
    Entry {
        keys: "Ctrl+H",
        action: "替换",
    },
    Entry {
        keys: "Ctrl+/",
        action: "切换行注释",
    },
    Entry {
        keys: "Alt+↑",
        action: "上移行",
    },
    Entry {
        keys: "Alt+↓",
        action: "下移行",
    },
    Entry {
        keys: "Tab",
        action: "增加缩进",
    },
    Entry {
        keys: "Shift+Tab",
        action: "减少缩进",
    },
    Entry {
        keys: "Ctrl+Space",
        action: "手动唤出补全",
    },
];

const TAB_ENTRIES: &[Entry] = &[
    Entry {
        keys: "Ctrl+Tab",
        action: "下一个标签",
    },
    Entry {
        keys: "Ctrl+Shift+Tab",
        action: "上一个标签",
    },
];

const GOTO_ENTRIES: &[Entry] = &[
    Entry {
        keys: "Ctrl+P",
        action: "转到文件",
    },
    Entry {
        keys: "Ctrl+G",
        action: "转到行或符号",
    },
    Entry {
        keys: "Ctrl+Shift+P",
        action: "命令面板",
    },
    Entry {
        keys: "F12",
        action: "转到定义",
    },
    Entry {
        keys: "Shift+F12",
        action: "查找引用",
    },
];

const VIEW_ENTRIES: &[Entry] = &[
    Entry {
        keys: "Ctrl+Shift+F",
        action: "项目内搜索面板",
    },
    Entry {
        keys: "Ctrl+=",
        action: "放大字号",
    },
    Entry {
        keys: "Ctrl+-",
        action: "缩小字号",
    },
    Entry {
        keys: "Ctrl+0",
        action: "重置字号",
    },
];

const RUN_ENTRIES: &[Entry] = &[
    Entry {
        keys: "F5",
        action: "运行脚本",
    },
    Entry {
        keys: "Shift+F5",
        action: "停止脚本",
    },
    Entry {
        keys: "Ctrl+F5",
        action: "调试运行",
    },
    Entry {
        keys: "F11",
        action: "继续调试",
    },
    Entry {
        keys: "F10",
        action: "单步调试",
    },
    Entry {
        keys: "Ctrl+F10",
        action: "运行到光标处",
    },
    Entry {
        keys: "F9",
        action: "切换断点",
    },
    Entry {
        keys: "Ctrl+Shift+F9",
        action: "清除所有断点",
    },
];

const HELP_ENTRIES: &[Entry] = &[Entry {
    keys: "F1",
    action: "快捷键一览",
}];

const DIALOG_ENTRIES: &[Entry] = &[
    Entry {
        keys: "Enter",
        action: "等同「确定」，或在输入框里执行",
    },
    Entry {
        keys: "Esc",
        action: "取消 / 关闭对话框",
    },
    Entry {
        keys: "再次按打开它的键",
        action: "例如对话框里再按 Ctrl+P / Ctrl+G / Ctrl+Shift+P 等于取消",
    },
];

const MOUSE_ENTRIES: &[Entry] = &[
    Entry {
        keys: "标签右键",
        action: "关闭 / 关闭其它 / 关闭右侧 / 关闭全部 / 全部保存 / 复制路径",
    },
    Entry {
        keys: "编辑器右键",
        action: "撤销/重做、剪切/复制/粘贴、上下移行、注释、缩进、断点、转到定义、查找引用",
    },
    Entry {
        keys: "资源管理器条目右键",
        action: "打开 / 重命名 / 删除 / 复制路径 / 刷新；文件夹另有新建，根目录可移除",
    },
    Entry {
        keys: "资源管理器左侧圆点",
        action: "进入选择模式：点条目切换选中，标题栏只保留删除与退出选择",
    },
    Entry {
        keys: "拖动条目到文件夹",
        action: "移动文件（不能拖进自己或子目录，目标同名会拒绝）",
    },
    Entry {
        keys: "拖动条目到标签栏",
        action: "打开该文件",
    },
    Entry {
        keys: "单击输出面板「第 N 行…」",
        action: "跳到对应代码行",
    },
    Entry {
        keys: "拖动面板标题栏",
        action: "停靠到左/右侧或底部，布局会被保存",
    },
];

/// 一览表的全部内容（分区按显示顺序排列）。
pub const SECTIONS: &[Section] = &[
    Section {
        title: "文件",
        entries: FILE_ENTRIES,
    },
    Section {
        title: "编辑",
        entries: EDIT_ENTRIES,
    },
    Section {
        title: "标签",
        entries: TAB_ENTRIES,
    },
    Section {
        title: "跳转",
        entries: GOTO_ENTRIES,
    },
    Section {
        title: "视图与字号",
        entries: VIEW_ENTRIES,
    },
    Section {
        title: "运行与调试",
        entries: RUN_ENTRIES,
    },
    Section {
        title: "帮助",
        entries: HELP_ENTRIES,
    },
    Section {
        title: "对话框与输入框",
        entries: DIALOG_ENTRIES,
    },
    Section {
        title: "鼠标操作",
        entries: MOUSE_ENTRIES,
    },
];
