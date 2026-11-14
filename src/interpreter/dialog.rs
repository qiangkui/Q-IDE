//! 脚本里可用的对话框：`MsgBox` 与 `InputBox`。
//!
//! 两个对话框都用内存里的对话框模板（`DLGTEMPLATE`）自己建、自己跑消息循环，原因：
//! - 脚本跑在后台线程，不是前台线程；用 `MessageBoxW` 时窗口会被创建在编辑器窗口
//!   后面，用户看不到还以为脚本卡住了；
//! - 自己跑循环才能在「停止脚本」时立刻关掉对话框（`MsgBox` 会一直等用户点按钮）。

use std::sync::atomic::AtomicBool;

/// `MsgBox` 的按钮 id（与 Win32 一致）。
const IDOK: u16 = 1;
const IDCANCEL: u16 = 2;
const IDABORT: u16 = 3;
const IDRETRY: u16 = 4;
const IDIGNORE: u16 = 5;
const IDYES: u16 = 6;
const IDNO: u16 = 7;

/// 按 `buttons` 参数给出按钮表，以及「直接关掉窗口」时的返回值。
fn message_buttons(flags: i64) -> (Vec<(u16, &'static str)>, i64) {
    match flags & 0x7 {
        1 => (vec![(IDOK, "确定"), (IDCANCEL, "取消")], 2),
        2 => (
            vec![(IDABORT, "中止"), (IDRETRY, "重试"), (IDIGNORE, "忽略")],
            3,
        ),
        3 => (vec![(IDYES, "是"), (IDNO, "否"), (IDCANCEL, "取消")], 2),
        4 => (vec![(IDYES, "是"), (IDNO, "否")], 7),
        5 => (vec![(IDRETRY, "重试"), (IDCANCEL, "取消")], 2),
        _ => (vec![(IDOK, "确定")], 1),
    }
}

/// `MsgBox`：返回用户按下的按钮（`vbOK`=1、`vbCancel`=2、`vbAbort`=3、`vbRetry`=4、
/// `vbIgnore`=5、`vbYes`=6、`vbNo`=7）。
pub fn message_box(
    prompt: &str,
    title: &str,
    flags: i64,
    interrupt: &AtomicBool,
) -> Result<i64, String> {
    let (buttons, close_value) = message_buttons(flags);

    #[cfg(windows)]
    {
        platform::message_box(prompt, title, &buttons, close_value, interrupt)
    }
    #[cfg(not(windows))]
    {
        let _ = (prompt, title, buttons, interrupt);
        Err("当前平台不支持 MsgBox".into())
    }
}

/// `InputBox(prompt[, title][, default])`：返回用户输入的文字；点取消时返回空字符串（与 VB 一致）。
///
/// `interrupt` 是「停止脚本」的信号：对话框开着时也会去查它，用户点停止就能马上关掉。
pub fn input_box(
    prompt: &str,
    title: &str,
    default: &str,
    interrupt: &AtomicBool,
) -> Result<String, String> {
    #[cfg(windows)]
    {
        platform::input_box(prompt, title, default, interrupt)
    }
    #[cfg(not(windows))]
    {
        let _ = (prompt, title, default, interrupt);
        Err("当前平台不支持 InputBox".into())
    }
}

#[cfg(windows)]
pub(crate) mod platform {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    // 对话框与控件样式。
    const WS_POPUP: u32 = 0x8000_0000;
    const WS_CAPTION: u32 = 0x00C0_0000;
    const WS_SYSMENU: u32 = 0x0008_0000;
    const WS_CHILD: u32 = 0x4000_0000;
    const WS_VISIBLE: u32 = 0x1000_0000;
    const WS_BORDER: u32 = 0x0080_0000;
    const WS_TABSTOP: u32 = 0x0001_0000;
    const DS_MODALFRAME: u32 = 0x0000_0080;
    const DS_CENTER: u32 = 0x0000_0800;
    const DS_SETFOREGROUND: u32 = 0x0000_0100;
    const ES_AUTOHSCROLL: u32 = 0x0000_0080;
    const SS_NOPREFIX: u32 = 0x0000_0080;
    const BS_DEFPUSHBUTTON: u32 = 0x0000_0001;

    // 预定义的控件类（对话框模板里的序号）。
    const BUTTON_CLASS: u16 = 0x0080;
    const EDIT_CLASS: u16 = 0x0081;
    const STATIC_CLASS: u16 = 0x0082;

    const IDC_PROMPT: u16 = 1001;
    const IDC_EDIT: u16 = 1002;
    const IDCANCEL: u16 = 2;

    const WM_INITDIALOG: u32 = 0x0110;
    const WM_COMMAND: u32 = 0x0111;
    const WM_CLOSE: u32 = 0x0010;
    const EM_SETSEL: u32 = 0x00B1;

    const SW_SHOW: i32 = 5;
    const PM_REMOVE: u32 = 0x0001;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    /// `HWND_TOPMOST`：脚本线程不是前台线程，不置顶就会被编辑器窗口挡住。
    const HWND_TOPMOST: isize = -1;
    /// 输入、消息等所有队列事件。
    const QS_ALLINPUT: u32 = 0x04FF;
    /// 等消息的超时：顺便去查「停止脚本」标志。
    const WAIT_SLICE_MS: u32 = 50;

    /// 对话框模板：`words` 交给系统，`items` 是各控件头部的字节偏移（都按 4 字节对齐）。
    pub(crate) struct Template {
        pub words: Vec<u32>,
        /// 只有测试读它。
        #[cfg_attr(not(test), allow(dead_code))]
        pub items: Vec<usize>,
    }

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_i16(bytes: &mut Vec<u8>, value: i16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// 以 UTF-16（0 结尾）写入一段文字。
    fn push_text(bytes: &mut Vec<u8>, text: &str) {
        for unit in text.encode_utf16() {
            push_u16(bytes, unit);
        }
        push_u16(bytes, 0);
    }

    fn align_dword(bytes: &mut Vec<u8>) {
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
    }

    /// 写一个控件；`class` 是预定义控件类的序号。
    fn push_item(
        bytes: &mut Vec<u8>,
        items: &mut Vec<usize>,
        style: u32,
        bounds: (i16, i16, i16, i16),
        id: u16,
        class: u16,
        text: &str,
    ) {
        align_dword(bytes);
        items.push(bytes.len());
        let (x, y, cx, cy) = bounds;
        push_u32(bytes, style);
        push_u32(bytes, 0); // 扩展样式
        push_i16(bytes, x);
        push_i16(bytes, y);
        push_i16(bytes, cx);
        push_i16(bytes, cy);
        push_u16(bytes, id);
        push_u16(bytes, 0xFFFF); // 后面跟预定义类序号
        push_u16(bytes, class);
        push_text(bytes, text);
        push_u16(bytes, 0); // 附加创建数据长度
    }

    /// 写对话框头：样式、控件个数、标题。
    fn push_header(bytes: &mut Vec<u8>, style: u32, count: u16, size: (i16, i16), title: &str) {
        push_u32(bytes, style);
        push_u32(bytes, 0); // 扩展样式
        push_u16(bytes, count);
        push_i16(bytes, 0); // x
        push_i16(bytes, 0); // y
        push_i16(bytes, size.0);
        push_i16(bytes, size.1);
        push_u16(bytes, 0); // 无菜单
        push_u16(bytes, 0); // 默认窗口类
        push_text(bytes, title);
    }

    /// 收尾：系统按 4 字节读取模板，用 u32 承载保证起始地址对齐。
    fn finish(bytes: &mut Vec<u8>, items: Vec<usize>) -> Template {
        align_dword(bytes);
        let words = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect();
        Template { words, items }
    }

    /// 「提示 + 单行输入框 + 确定/取消」的模板。
    pub(crate) fn build_input_template(title: &str, prompt: &str) -> Template {
        let mut bytes: Vec<u8> = Vec::new();
        let mut items: Vec<usize> = Vec::new();

        push_header(
            &mut bytes,
            WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_CENTER | DS_SETFOREGROUND,
            4,
            (210, 94),
            title,
        );
        push_item(
            &mut bytes,
            &mut items,
            WS_CHILD | WS_VISIBLE | SS_NOPREFIX,
            (10, 10, 190, 32),
            IDC_PROMPT,
            STATIC_CLASS,
            prompt,
        );
        push_item(
            &mut bytes,
            &mut items,
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
            (10, 46, 190, 14),
            IDC_EDIT,
            EDIT_CLASS,
            "",
        );
        push_item(
            &mut bytes,
            &mut items,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
            (100, 70, 44, 14),
            super::IDOK,
            BUTTON_CLASS,
            "确定",
        );
        push_item(
            &mut bytes,
            &mut items,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            (150, 70, 44, 14),
            IDCANCEL,
            BUTTON_CLASS,
            "取消",
        );

        finish(&mut bytes, items)
    }

    /// 「提示 + 一排按钮」的模板（`MsgBox` 用）。
    pub(crate) fn build_message_template(
        title: &str,
        prompt: &str,
        buttons: &[(u16, &'static str)],
    ) -> Template {
        const WIDTH: i16 = 220;
        const BUTTON_WIDTH: i16 = 50;
        const BUTTON_GAP: i16 = 6;

        let mut bytes: Vec<u8> = Vec::new();
        let mut items: Vec<usize> = Vec::new();

        push_header(
            &mut bytes,
            WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_CENTER | DS_SETFOREGROUND,
            buttons.len() as u16 + 1,
            (WIDTH, 84),
            title,
        );
        push_item(
            &mut bytes,
            &mut items,
            WS_CHILD | WS_VISIBLE | SS_NOPREFIX,
            (10, 10, WIDTH - 20, 40),
            IDC_PROMPT,
            STATIC_CLASS,
            prompt,
        );
        // 按钮从右往左排：第一个按钮在最右边。
        let count = buttons.len() as i16;
        for (index, (id, label)) in buttons.iter().enumerate() {
            let offset = count - 1 - index as i16;
            let x = WIDTH - 10 - BUTTON_WIDTH - offset * (BUTTON_WIDTH + BUTTON_GAP);
            push_item(
                &mut bytes,
                &mut items,
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
                (x, 58, BUTTON_WIDTH, 14),
                *id,
                BUTTON_CLASS,
                label,
            );
        }

        finish(&mut bytes, items)
    }

    /// 对话框期间共享的数据；对话框和调用者在同一个线程上，所以用 thread_local 传。
    struct DialogState {
        /// 输入框的初始文字（0 结尾的 UTF-16）。
        default: Vec<u16>,
        /// 是否有输入框（`InputBox` 有，`MsgBox` 没有）。
        has_edit: bool,
        /// 输入框里的结果。
        result: String,
        /// 按下的按钮 id；0 表示窗口被直接关掉。
        button: i32,
        /// 是否按了按钮（否则是取消/关闭）。
        answered: bool,
    }

    thread_local! {
        static CURRENT: Cell<*mut DialogState> = const { Cell::new(std::ptr::null_mut()) };
    }

    type Hwnd = *mut c_void;
    type DialogProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn CreateDialogIndirectParamW(
            instance: *mut c_void,
            template: *const u16,
            parent: Hwnd,
            procedure: Option<DialogProc>,
            init: isize,
        ) -> Hwnd;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
        fn SetForegroundWindow(hwnd: Hwnd) -> i32;
        fn SetWindowPos(
            hwnd: Hwnd,
            insert_after: Hwnd,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        fn IsWindow(hwnd: Hwnd) -> i32;
        fn IsDialogMessageW(hwnd: Hwnd, message: *mut Msg) -> i32;
        fn PeekMessageW(
            message: *mut Msg,
            hwnd: Hwnd,
            filter_min: u32,
            filter_max: u32,
            remove: u32,
        ) -> i32;
        fn TranslateMessage(message: *const Msg) -> i32;
        fn DispatchMessageW(message: *const Msg) -> isize;
        fn MsgWaitForMultipleObjects(
            count: u32,
            handles: *const Hwnd,
            wait_all: i32,
            milliseconds: u32,
            wake_mask: u32,
        ) -> u32;
        fn SetDlgItemTextW(hwnd: Hwnd, id: i32, text: *const u16) -> i32;
        fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, max: i32) -> i32;
        fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
        fn GetDlgItem(hwnd: Hwnd, id: i32) -> Hwnd;
        fn SetFocus(hwnd: Hwnd) -> Hwnd;
        fn SendDlgItemMessageW(
            hwnd: Hwnd,
            id: i32,
            message: u32,
            wparam: usize,
            lparam: isize,
        ) -> isize;
    }

    /// `MSG`；字段顺序与平台结构体一致。
    #[repr(C)]
    #[derive(Default)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        point: [i32; 2],
        private: u32,
    }

    unsafe extern "system" fn dialog_proc(
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        _lparam: isize,
    ) -> isize {
        match message {
            WM_INITDIALOG => {
                let pointer = CURRENT.with(Cell::get);
                if pointer.is_null() {
                    unsafe { DestroyWindow(hwnd) };
                    return 1;
                }
                let state = unsafe { &*pointer };
                if state.has_edit {
                    unsafe {
                        SetDlgItemTextW(hwnd, IDC_EDIT as i32, state.default.as_ptr());
                        // 选中初始文字，用户直接输入就替换掉。
                        SendDlgItemMessageW(hwnd, IDC_EDIT as i32, EM_SETSEL, 0, -1);
                        SetFocus(GetDlgItem(hwnd, IDC_EDIT as i32));
                    }
                }
                1
            }
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as i32;
                if id == IDCANCEL as i32 {
                    unsafe { DestroyWindow(hwnd) };
                    1
                } else if (1..=7).contains(&id) {
                    unsafe { answer(hwnd, id) };
                    1
                } else {
                    0
                }
            }
            WM_CLOSE => {
                unsafe { DestroyWindow(hwnd) };
                1
            }
            _ => 0,
        }
    }

    /// 记下按下的按钮（有输入框时顺便读出文字）并关掉对话框。
    unsafe fn answer(hwnd: Hwnd, id: i32) {
        let pointer = CURRENT.with(Cell::get);
        if pointer.is_null() {
            unsafe { DestroyWindow(hwnd) };
            return;
        }
        let state = unsafe { &mut *pointer };
        if state.has_edit && id == super::IDOK as i32 {
            // 取长度要用 GetWindowTextLengthW：`GetDlgItemTextW(.., NULL, 0)` 只会返回 0。
            let edit = unsafe { GetDlgItem(hwnd, IDC_EDIT as i32) };
            let length = unsafe { GetWindowTextLengthW(edit) };
            let mut buffer = vec![0u16; length as usize + 1];
            let copied = unsafe { GetWindowTextW(edit, buffer.as_mut_ptr(), buffer.len() as i32) };
            state.result = String::from_utf16_lossy(&buffer[..copied as usize]);
        }
        state.button = id;
        state.answered = true;
        unsafe { DestroyWindow(hwnd) };
    }

    /// 建出对话框并跑消息循环，直到用户回答、关掉窗口或点了「停止脚本」。
    fn run(
        template: &Template,
        state: &mut DialogState,
        interrupt: &AtomicBool,
    ) -> Result<(), String> {
        let previous = CURRENT.with(|current| current.replace(state as *mut DialogState));
        // 脚本线程没有「本线程的活动窗口」，所以不设父窗口：
        // 编辑器窗口保持可操作，对话框靠置顶保证能看见。
        let hwnd = unsafe {
            CreateDialogIndirectParamW(
                GetModuleHandleW(std::ptr::null()),
                template.words.as_ptr().cast(),
                std::ptr::null_mut(),
                Some(dialog_proc),
                0,
            )
        };
        if hwnd.is_null() {
            CURRENT.with(|current| current.set(previous));
            return Err("无法显示对话框".into());
        }
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST as Hwnd,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
        }

        let mut stopped = false;
        while unsafe { IsWindow(hwnd) } != 0 {
            unsafe {
                MsgWaitForMultipleObjects(0, std::ptr::null(), 0, WAIT_SLICE_MS, QS_ALLINPUT)
            };
            let mut message = Msg::default();
            while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0
            {
                if unsafe { IsDialogMessageW(hwnd, &mut message) } == 0 {
                    unsafe {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            }
            if interrupt.load(Ordering::Relaxed) {
                stopped = true;
                unsafe { DestroyWindow(hwnd) };
            }
        }
        CURRENT.with(|current| current.set(previous));

        if stopped {
            return Err(crate::interpreter::debug::STOPPED_MESSAGE.to_string());
        }
        Ok(())
    }

    pub(crate) fn input_box(
        prompt: &str,
        title: &str,
        default: &str,
        interrupt: &AtomicBool,
    ) -> Result<String, String> {
        let template = build_input_template(title, prompt);
        let mut state = DialogState {
            default: default.encode_utf16().chain(std::iter::once(0)).collect(),
            has_edit: true,
            result: String::new(),
            button: 0,
            answered: false,
        };
        run(&template, &mut state, interrupt)?;

        Ok(if state.answered && state.button == super::IDOK as i32 {
            state.result
        } else {
            String::new()
        })
    }

    pub(crate) fn message_box(
        prompt: &str,
        title: &str,
        buttons: &[(u16, &'static str)],
        close_value: i64,
        interrupt: &AtomicBool,
    ) -> Result<i64, String> {
        let template = build_message_template(title, prompt, buttons);
        let mut state = DialogState {
            default: Vec::new(),
            has_edit: false,
            result: String::new(),
            button: 0,
            answered: false,
        };
        run(&template, &mut state, interrupt)?;

        Ok(if state.answered {
            i64::from(state.button)
        } else {
            close_value
        })
    }
}
