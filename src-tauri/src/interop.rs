//! 全部 Win32 互操作声明集中在此。
//!
//! 直接用 `extern "system"` 声明而不用 `windows` crate 的封装，原因有二：
//! 1) 这些调用都是"失败就回退"的窗口技巧，错误处理比类型安全更重要；
//! 2) 避免与 Tauri 内部的 `windows` 版本发生类型不匹配。

#![allow(non_snake_case, dead_code)]

use std::ffi::c_void;

pub type Hwnd = isize;

// ---------------- 常量 ----------------

pub const GWL_STYLE: i32 = -16;
pub const GWL_EXSTYLE: i32 = -20;
pub const GCLP_HBRBACKGROUND: i32 = -10;

pub const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
pub const WS_EX_TOPMOST: isize = 0x0000_0008;
pub const WS_EX_TRANSPARENT: isize = 0x0000_0020;
pub const WS_EX_LAYERED: isize = 0x0008_0000;
pub const WS_EX_NOACTIVATE: isize = 0x0800_0000;

pub const WS_POPUP: isize = 0x8000_0000u32 as i32 as isize;
pub const WS_CAPTION: isize = 0x00C0_0000;
pub const WS_THICKFRAME: isize = 0x0004_0000;
pub const WS_BORDER: isize = 0x0080_0000;
pub const WS_DLGFRAME: isize = 0x0040_0000;
pub const WS_SYSMENU: isize = 0x0008_0000;
pub const WS_MINIMIZEBOX: isize = 0x0002_0000;
pub const WS_MAXIMIZEBOX: isize = 0x0001_0000;
pub const WS_CLIPCHILDREN: isize = 0x0200_0000;
pub const WS_CLIPSIBLINGS: isize = 0x0400_0000;

pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const SWP_NOACTIVATE: u32 = 0x0010;
pub const SWP_FRAMECHANGED: u32 = 0x0020;
pub const SWP_NOSENDCHANGING: u32 = 0x0400;

pub const HWND_TOPMOST: isize = -1;
pub const HWND_NOTOPMOST: isize = -2;

pub const SW_HIDE: i32 = 0;
pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOWNOACTIVATE: i32 = 4;

pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_NCHITTEST: u32 = 0x0084;
pub const WM_MOUSEACTIVATE: u32 = 0x0021;
pub const MA_NOACTIVATE: isize = 3;
pub const HTTRANSPARENT: isize = -1;
pub const WM_DPICHANGED: u32 = 0x02E0;

pub const DWMWA_NCRENDERING_POLICY: u32 = 2;
pub const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
pub const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
pub const DWMWA_BORDER_COLOR: u32 = 34;

pub const DWMWCP_DONOTROUND: i32 = 1;
pub const DWMWCP_ROUND: i32 = 2;
pub const DWMNCRP_DISABLED: i32 = 1;

/// 岛体底色 0x0B0B0C 的 COLORREF 形式（0x00BBGGRR）。
pub const ISLAND_BORDER_COLOR: u32 = 0x000C_0B0B;

/// `DWMWA_BORDER_COLOR` 的特殊值：不绘制窗口边框。
pub const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;

pub const VK_LBUTTON: i32 = 0x01;
pub const VK_RBUTTON: i32 = 0x02;
pub const VK_MBUTTON: i32 = 0x04;

pub const MONITOR_DEFAULTTONEAREST: u32 = 2;
pub const MONITOR_DEFAULTTOPRIMARY: u32 = 1;

// ---------------- 结构体 ----------------

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RECT {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MONITORINFO {
    pub cb_size: u32,
    pub rc_monitor: RECT,
    pub rc_work: RECT,
    pub dw_flags: u32,
}

impl Default for MONITORINFO {
    fn default() -> Self {
        Self {
            cb_size: std::mem::size_of::<MONITORINFO>() as u32,
            rc_monitor: RECT::default(),
            rc_work: RECT::default(),
            dw_flags: 0,
        }
    }
}

pub type SubclassProc = unsafe extern "system" fn(
    hwnd: Hwnd,
    msg: u32,
    wparam: usize,
    lparam: isize,
    subclass_id: usize,
    ref_data: isize,
) -> isize;

// ---------------- 声明 ----------------

#[link(name = "user32")]
extern "system" {
    pub fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    pub fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    pub fn SetClassLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    pub fn SetWindowPos(
        hwnd: Hwnd,
        insert_after: Hwnd,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
    pub fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
    pub fn IsWindowVisible(hwnd: Hwnd) -> i32;
    pub fn GetClientRect(hwnd: Hwnd, rect: *mut RECT) -> i32;
    pub fn GetWindowRect(hwnd: Hwnd, rect: *mut RECT) -> i32;
    pub fn ClientToScreen(hwnd: Hwnd, point: *mut POINT) -> i32;
    pub fn ScreenToClient(hwnd: Hwnd, point: *mut POINT) -> i32;
    pub fn GetCursorPos(point: *mut POINT) -> i32;
    pub fn GetAsyncKeyState(key: i32) -> i16;
    pub fn GetDpiForWindow(hwnd: Hwnd) -> u32;
    pub fn MonitorFromWindow(hwnd: Hwnd, flags: u32) -> isize;
    pub fn MonitorFromPoint(point: POINT, flags: u32) -> isize;
    pub fn GetMonitorInfoW(monitor: isize, info: *mut MONITORINFO) -> i32;
    pub fn GetForegroundWindow() -> Hwnd;
    pub fn SetForegroundWindow(hwnd: Hwnd) -> i32;
    pub fn FillRect(hdc: isize, rect: *const RECT, brush: isize) -> i32;
    pub fn RegisterWindowMessageW(name: *const u16) -> u32;
    pub fn GetWindowThreadProcessId(hwnd: Hwnd, process_id: *mut u32) -> u32;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateSolidBrush(color: u32) -> isize;
    pub fn DeleteObject(object: isize) -> i32;
}

#[link(name = "dwmapi")]
extern "system" {
    pub fn DwmSetWindowAttribute(
        hwnd: Hwnd,
        attribute: u32,
        value: *const c_void,
        size: u32,
    ) -> i32;
    pub fn DwmGetWindowAttribute(hwnd: Hwnd, attribute: u32, value: *mut c_void, size: u32) -> i32;
}

#[link(name = "comctl32")]
extern "system" {
    pub fn SetWindowSubclass(
        hwnd: Hwnd,
        proc_: SubclassProc,
        subclass_id: usize,
        ref_data: isize,
    ) -> i32;
    pub fn RemoveWindowSubclass(hwnd: Hwnd, proc_: SubclassProc, subclass_id: usize) -> i32;
    pub fn DefSubclassProc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: Hwnd,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

// ---------------- 安全包装 ----------------

pub fn cursor_position() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return None;
    }
    Some((point.x, point.y))
}

pub fn is_left_button_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON) & 0x8000u16 as i16) != 0 }
}

pub fn is_any_mouse_button_down() -> bool {
    unsafe {
        (GetAsyncKeyState(VK_LBUTTON) & 0x8000u16 as i16) != 0
            || (GetAsyncKeyState(VK_RBUTTON) & 0x8000u16 as i16) != 0
            || (GetAsyncKeyState(VK_MBUTTON) & 0x8000u16 as i16) != 0
    }
}

pub fn foreground_window() -> Hwnd {
    unsafe { GetForegroundWindow() }
}

pub fn window_rect(hwnd: Hwnd) -> Option<RECT> {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        None
    } else {
        Some(rect)
    }
}

pub fn client_rect(hwnd: Hwnd) -> Option<RECT> {
    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
        None
    } else {
        Some(rect)
    }
}

/// 用系统默认浏览器打开链接；只接受 https，避免被拼进来的任意命令。
pub fn open_url(url: &str) -> bool {
    if !url.starts_with("https://") {
        return false;
    }

    let wide = to_wide(url);
    let operation = to_wide("open");
    let result = unsafe {
        ShellExecuteW(
            0,
            operation.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW 返回值 > 32 才算成功。
    result > 32
}

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
