//! `InputBox` / `MsgBox` 对话框的测试。
//!
//! 模板测试只检查内存里的字节布局（对齐与字段顺序错了系统会直接拒绝或画错控件）。
//! 另外 5 个测试会真的建出系统对话框（约半秒后自动关掉），默认用 `#[ignore]`
//! 跳过，避免日常 `cargo test` 弹出窗口；需要时单独运行：
//!
//! ```text
//! cargo test dialog_tests -- --ignored
//! ```

#![cfg(windows)]

use crate::interpreter::dialog::platform::{Template, build_input_template, build_message_template};

/// 把 4 字节承载的模板还原成 16 位单位，方便查找文字。
fn units(template: &Template) -> Vec<u16> {
    template
        .words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<u8>>()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u16::from_le_bytes(*chunk))
        .collect()
}

fn contains_text(template: &Template, text: &str) -> bool {
    let wanted: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    units(template).windows(wanted.len()).any(|window| window == wanted)
}

#[test]
fn template_header_describes_a_modal_dialog() {
    let template = build_input_template("输入", "请输入名称");
    // DLGTEMPLATE：style、dwExtendedStyle、cdit、x、y、cx、cy。
    assert_eq!(template.words[1], 0, "扩展样式应为 0");
    assert_eq!(template.words[2] & 0xFFFF, 4, "四个控件");
    let style = template.words[0];
    assert_eq!(style & 0x8000_0000, 0x8000_0000, "WS_POPUP");
    assert_eq!(style & 0x00C0_0000, 0x00C0_0000, "WS_CAPTION");
    assert_eq!(style & 0x0000_0080, 0x0000_0080, "DS_MODALFRAME");
    assert_eq!(style & 0x0000_0800, 0x0000_0800, "DS_CENTER");
}

#[test]
fn first_item_follows_the_dialog_header() {
    let title = "标题";
    let template = build_input_template(title, "请输入");
    // 18 字节固定头部 + 菜单 2 + 窗口类 2 + 标题（含结尾 0）。
    let expected = 18 + 2 + 2 + (title.encode_utf16().count() + 1) * 2;
    assert_eq!(template.items.first().copied(), Some(expected));
}

#[test]
fn every_item_starts_on_a_dword_boundary() {
    let template = build_input_template("输入", "请输入名称");
    assert_eq!(template.items.len(), 4);
    for offset in &template.items {
        assert_eq!(offset % 4, 0, "控件头部要按 4 字节对齐：{offset}");
    }
    assert!(
        template
            .items
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "控件偏移应当递增"
    );
}

#[test]
fn texts_are_stored_as_utf16() {
    let template = build_input_template("标题文字", "请输入名称：");
    assert!(contains_text(&template, "标题文字"), "对话框标题");
    assert!(contains_text(&template, "请输入名称："), "提示文字");
    assert!(contains_text(&template, "确定"));
    assert!(contains_text(&template, "取消"));
}

#[test]
fn message_template_lists_the_buttons() {
    let buttons = [(1u16, "是"), (2u16, "否")];
    let template = build_message_template("标题", "继续吗？", &buttons);
    assert_eq!(template.items.len(), 3, "一个提示加两个按钮");
    assert!(
        template.items.iter().all(|offset| offset % 4 == 0),
        "控件头部要按 4 字节对齐"
    );
    assert!(contains_text(&template, "继续吗？"));
    assert!(contains_text(&template, "是"));
    assert!(contains_text(&template, "否"));
}

/// 这些测试会真的建出系统对话框（约半秒后自己关掉），
/// 所以串行执行，也避免互相把对方的窗口关掉。
static DIALOG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

mod win {
    use std::ffi::c_void;

    pub const WM_COMMAND: u32 = 0x0111;
    pub const IDOK: usize = 1;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> *mut c_void;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
        fn PostMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn GetWindowLongW(hwnd: *mut c_void, index: i32) -> i32;
    }

    /// 是否置顶（`WS_EX_TOPMOST`）：脚本线程弹的窗口必须置顶，否则会被编辑器挡住。
    pub fn is_topmost(hwnd: *mut c_void) -> bool {
        const GWL_EXSTYLE: i32 = -20;
        const WS_EX_TOPMOST: i32 = 0x0000_0008;
        (unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } & WS_EX_TOPMOST) != 0
    }

    pub fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 按标题找本进程弹出的对话框。
    pub fn find_dialog(title: &str) -> *mut c_void {
        let class = wide("#32770");
        let title = wide(title);
        unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) }
    }

    pub fn is_visible(hwnd: *mut c_void) -> bool {
        (unsafe { IsWindowVisible(hwnd) }) != 0
    }

    pub fn post(hwnd: *mut c_void, message: u32, wparam: usize) {
        unsafe { PostMessageW(hwnd, message, wparam, 0) };
    }
}

/// 对话框能建出来、可见，点「确定」能拿回输入框里的文字。
#[test]
#[ignore = "会弹出真实系统对话框；用 --ignored 运行"]
fn dialog_is_visible_and_returns_the_typed_text() {
    use std::time::Duration;

    let _guard = DIALOG_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let closer = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(500));
        let hwnd = win::find_dialog("自检标题");
        assert!(!hwnd.is_null(), "应当能找到对话框窗口");
        assert!(win::is_visible(hwnd), "对话框应当可见");
        assert!(win::is_topmost(hwnd), "对话框应当置顶，否则会被编辑器挡住");
        win::post(hwnd, win::WM_COMMAND, win::IDOK);
    });

    let interrupt = std::sync::atomic::AtomicBool::new(false);
    let result = crate::interpreter::dialog::input_box("请输入名称", "自检标题", "默认", &interrupt);
    closer.join().expect("关闭线程不应 panic");
    assert_eq!(result.as_deref(), Ok("默认"));
}

/// 对话框开着时点「停止脚本」要能立刻关掉它（否则脚本线程会一直等下去）。
#[test]
#[ignore = "会弹出真实系统对话框；用 --ignored 运行"]
fn dialog_closes_when_the_script_is_stopped() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    let _guard = DIALOG_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let interrupt = Arc::new(AtomicBool::new(false));
    let stopper = {
        let interrupt = interrupt.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            let hwnd = win::find_dialog("自检停止");
            assert!(!hwnd.is_null(), "停止前对话框应当还在");
            interrupt.store(true, Ordering::Relaxed);
        })
    };

    let result = crate::interpreter::dialog::input_box("请输入名称", "自检停止", "", &interrupt);
    stopper.join().expect("停止线程不应 panic");
    assert_eq!(result, Err("脚本已停止".to_string()));
    assert!(
        win::find_dialog("自检停止").is_null(),
        "停止后对话框应当关掉"
    );
}

/// 点「否」应当返回 `vbNo`（7）。
#[test]
#[ignore = "会弹出真实系统对话框；用 --ignored 运行"]
fn message_box_returns_the_clicked_button() {
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    let _guard = DIALOG_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let closer = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(500));
        let hwnd = win::find_dialog("自检询问");
        assert!(!hwnd.is_null(), "应当能找到消息框");
        assert!(win::is_topmost(hwnd), "消息框应当置顶");
        win::post(hwnd, win::WM_COMMAND, 7); // IDNO
    });

    let interrupt = AtomicBool::new(false);
    let result = crate::interpreter::dialog::message_box("继续吗？", "自检询问", 4, &interrupt);
    closer.join().expect("关闭线程不应 panic");
    assert_eq!(result, Ok(7));
}

/// `MsgBox` 也会挡住脚本线程：`close_dialogs`（停止脚本时调用）要能关掉它。
#[test]
#[ignore = "会弹出真实系统对话框；用 --ignored 运行"]
fn close_dialogs_closes_a_waiting_message_box() {
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    let _guard = DIALOG_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let interrupt = std::sync::Arc::new(AtomicBool::new(false));
    let message = std::thread::spawn(move || {
        crate::interpreter::dialog::message_box("内容", "自检消息", 0, &interrupt)
    });
    // 等消息框真的建出来，再让它从「停止脚本」这条路被关掉。
    let mut found = false;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(50));
        let hwnd = win::find_dialog("自检消息");
        if !hwnd.is_null() {
            assert!(win::is_visible(hwnd), "消息框应当可见");
            found = true;
            break;
        }
    }
    assert!(found, "消息框应当已经弹出");
    crate::interpreter::ffi::close_dialogs();
    let result = message.join().expect("消息框线程不应 panic");
    assert_eq!(result, Ok(1), "直接关掉窗口按「确定」处理");
}

/// 完整走一遍「脚本里调用 MsgBox」：消息框要弹出来（且置顶），停止脚本要能结束整个脚本。
#[test]
#[ignore = "会弹出真实系统对话框；用 --ignored 运行"]
fn script_message_box_shows_and_stop_ends_the_script() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    let _guard = DIALOG_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let interrupt = Arc::new(AtomicBool::new(false));
    let script = {
        let interrupt = interrupt.clone();
        std::thread::spawn(move || {
            crate::interpreter::run("MsgBox \"你好\"", |_| {}, &interrupt)
        })
    };

    let mut found = false;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(50));
        let hwnd = win::find_dialog("脚本");
        if !hwnd.is_null() {
            assert!(win::is_visible(hwnd), "消息框应当可见");
            assert!(win::is_topmost(hwnd), "消息框应当置顶，否则会被编辑器挡住");
            found = true;
            break;
        }
    }
    assert!(found, "脚本里的 MsgBox 应当弹出");

    interrupt.store(true, std::sync::atomic::Ordering::Relaxed);
    let result = script.join().expect("脚本线程不应 panic");
    let error = result.expect_err("停止后脚本应当以错误结束");
    assert!(error.to_string().contains("脚本已停止"), "{error}");
}
