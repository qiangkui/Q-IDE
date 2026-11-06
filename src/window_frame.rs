//! 窗口外框：使用系统自带的边框缩放，但隐藏可见的标题栏与边框颜色。
//!
//! 只保留 `WS_THICKFRAME`，把窗口外沿 1px 边框的颜色改成和背景一致，
//! 视觉上几乎不可见；同时修正最大化时客户区盖住任务栏的问题。

use gpui_kit::Window;

/// 去掉系统标题栏，并把外沿边框染成 `background`（0xRRGGBB）。
#[cfg(target_os = "windows")]
pub fn setup(window: &Window, background: u32) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;

    const DWMWA_BORDER_COLOR: u32 = 34;
    const GWL_STYLE: i32 = -16;
    const WS_CAPTION: i32 = 0x00c0_0000;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_FRAMECHANGED: u32 = 0x0020;

    // COLORREF：0x00BBGGRR。
    let border_color = background;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowLongW(hwnd: *mut c_void, index: i32) -> i32;
        fn SetWindowLongW(hwnd: *mut c_void, index: i32, value: i32) -> i32;
        fn SetWindowPos(
            hwnd: *mut c_void,
            insert_after: *mut c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut c_void,
            attribute: u32,
            value: *const c_void,
            size: u32,
        ) -> i32;
    }

    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn SetWindowSubclass(
            hwnd: *mut c_void,
            proc: Option<MaximizeSubclassProc>,
            id: usize,
            data: usize,
        ) -> i32;
    }

    // The inherent `Window::window_handle` returns gpui's own handle, so call
    // the trait method explicitly to get the platform (HWND) handle.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as *mut c_void;

    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        SetWindowLongW(hwnd, GWL_STYLE, style & !WS_CAPTION);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );

        // Windows 11 才有 DWMWA_BORDER_COLOR；旧系统调用失败会被忽略。
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border_color as *const _ as _,
            std::mem::size_of::<u32>() as u32,
        );

        // gpui 默认的最大化尺寸铺满整块屏幕，会盖住任务栏；这里改成工作区。
        SetWindowSubclass(hwnd, Some(maximize_to_work_area), 1, 0);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn setup(_: &Window, _: u32) {}

#[cfg(target_os = "windows")]
type MaximizeSubclassProc =
    unsafe extern "system" fn(*mut std::ffi::c_void, u32, usize, isize, usize, usize) -> isize;

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Default)]
struct WinPoint {
    x: i32,
    y: i32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Default)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Default)]
struct MinMaxInfo {
    reserved: WinPoint,
    max_size: WinPoint,
    max_position: WinPoint,
    min_track_size: WinPoint,
    max_track_size: WinPoint,
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Default)]
struct MonitorInfo {
    cb_size: u32,
    monitor: WinRect,
    work: WinRect,
    flags: u32,
}

/// 让最大化后的窗口客户区正好等于显示器工作区（保留任务栏）。
#[cfg(target_os = "windows")]
unsafe extern "system" fn maximize_to_work_area(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    _data: usize,
) -> isize {
    use std::ffi::c_void;

    const WM_GETMINMAXINFO: u32 = 0x0024;
    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    const SM_CXSIZEFRAME: i32 = 32;
    const SM_CYSIZEFRAME: i32 = 33;
    const SM_CXPADDEDBORDER: i32 = 92;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDpiForWindow(hwnd: *mut c_void) -> u32;
        fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
        fn MonitorFromWindow(hwnd: *mut c_void, flags: u32) -> *mut c_void;
        fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
    }

    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn DefSubclassProc(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
    }

    // 先让 gpui 自己的窗口过程处理（它会写入最小尺寸限制）。
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    if msg != WM_GETMINMAXINFO {
        return result;
    }

    unsafe {
        let mut monitor_info = MonitorInfo {
            cb_size: std::mem::size_of::<MonitorInfo>() as u32,
            ..Default::default()
        };
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut monitor_info) == 0 {
            return result;
        }

        let dpi = GetDpiForWindow(hwnd);
        let padded_border = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
        let frame_x = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + padded_border;
        let frame_y = GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + padded_border;
        let work = &monitor_info.work;
        let min_max = &mut *(lparam as *mut MinMaxInfo);

        // 窗口矩形比工作区大一圈边框，客户区正好落在工作区里。
        min_max.max_position.x = work.left - monitor_info.monitor.left - frame_x;
        min_max.max_position.y = work.top - monitor_info.monitor.top - frame_y;
        min_max.max_size.x = (work.right - work.left) + frame_x * 2;
        min_max.max_size.y = (work.bottom - work.top) + frame_y * 2;
    }

    result
}
