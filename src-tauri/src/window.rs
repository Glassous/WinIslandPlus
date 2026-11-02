//! 岛窗的原生窗口技巧集合：置顶、不抢焦点、不进任务栏 / Alt-Tab、按形状点击穿透。
//!
//! 所有失败都不抛出，只记日志并回退，保证主流程可用。

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::WebviewWindow;

use crate::interop::{self, Hwnd};
use crate::log;

/// 取窗口的 HWND（跨版本安全：走 raw-window-handle，不依赖 Tauri 内部 `windows` 版本）。
pub fn hwnd_of(window: &WebviewWindow) -> Option<Hwnd> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
        _ => None,
    }
}

/// 把窗口变成「悬浮 HUD」：工具窗口 + 不激活 + 置顶 + 关闭系统圆角与边框绘制。
pub fn apply_overlay_styles(hwnd: Hwnd) {
    unsafe {
        let ex_style = interop::GetWindowLongPtrW(hwnd, interop::GWL_EXSTYLE);
        let ex_style =
            ex_style | interop::WS_EX_TOOLWINDOW | interop::WS_EX_NOACTIVATE;
        interop::SetWindowLongPtrW(hwnd, interop::GWL_EXSTYLE, ex_style);

        // 窗口类背景画刷会让窗口边缘露出白底（浅色主题下）
        interop::SetClassLongPtrW(hwnd, interop::GCLP_HBRBACKGROUND, 0);

        // 去掉系统边框位：否则客户区会比窗口矩形内缩，那一圈会被画成浅色高光
        let mut style = interop::GetWindowLongPtrW(hwnd, interop::GWL_STYLE);
        style &= !(interop::WS_CAPTION
            | interop::WS_THICKFRAME
            | interop::WS_BORDER
            | interop::WS_DLGFRAME
            | interop::WS_SYSMENU
            | interop::WS_MINIMIZEBOX
            | interop::WS_MAXIMIZEBOX);
        style |= interop::WS_POPUP | interop::WS_CLIPCHILDREN | interop::WS_CLIPSIBLINGS;
        interop::SetWindowLongPtrW(hwnd, interop::GWL_STYLE, style);

        interop::SetWindowPos(
            hwnd,
            0,
            0,
            0,
            0,
            0,
            interop::SWP_FRAMECHANGED
                | interop::SWP_NOMOVE
                | interop::SWP_NOSIZE
                | interop::SWP_NOZORDER
                | interop::SWP_NOACTIVATE,
        );

        // 形状由前端逐像素圆角决定，系统圆角必须关掉（否则会与内容互相打架）
        let corner = interop::DWMWCP_DONOTROUND;
        interop::DwmSetWindowAttribute(
            hwnd,
            interop::DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );

        // 不要任何窗口边框：逐像素透明的前提下，DWM 描的那一圈会变成一圈黑边
        let border = interop::DWMWA_COLOR_NONE;
        interop::DwmSetWindowAttribute(
            hwnd,
            interop::DWMWA_BORDER_COLOR,
            &border as *const u32 as *const _,
            std::mem::size_of::<u32>() as u32,
        );

        // 让 DWM 完全不渲染非客户区（系统边框 / 阴影）
        let policy = interop::DWMNCRP_DISABLED;
        interop::DwmSetWindowAttribute(
            hwnd,
            interop::DWMWA_NCRENDERING_POLICY,
            &policy as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );

        let dark = 1i32;
        interop::DwmSetWindowAttribute(
            hwnd,
            interop::DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
    }

    ensure_topmost(hwnd);

    let (before, after) = reassert_overlay_styles(hwnd);
    if before != after {
        log::info(&format!("interop: 扩展样式 0x{before:X} -> 0x{after:X}"));
    }
}

/// 拦截 `WM_MOUSEACTIVATE`，保证点击岛体不会把用户的当前窗口切走。
unsafe extern "system" fn island_subclass_proc(
    hwnd: Hwnd,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _subclass_id: usize,
    _ref_data: isize,
) -> isize {
    if msg == interop::WM_MOUSEACTIVATE {
        return interop::MA_NOACTIVATE;
    }
    interop::DefSubclassProc(hwnd, msg, wparam, lparam)
}

pub fn install_subclass(hwnd: Hwnd) {
    let ok = unsafe { interop::SetWindowSubclass(hwnd, island_subclass_proc, 1, 0) };
    if ok == 0 {
        log::warn("interop: 子类化岛窗失败（点击可能抢焦点）");
    }
}

pub fn ensure_topmost(hwnd: Hwnd) {
    unsafe {
        interop::SetWindowPos(
            hwnd,
            interop::HWND_TOPMOST,
            0,
            0,
            0,
            0,
            interop::SWP_NOMOVE | interop::SWP_NOSIZE | interop::SWP_NOACTIVATE,
        );
    }
}

pub fn is_topmost(hwnd: Hwnd) -> bool {
    unsafe {
        (interop::GetWindowLongPtrW(hwnd, interop::GWL_EXSTYLE) & interop::WS_EX_TOPMOST) != 0
    }
}

pub fn set_bounds(hwnd: Hwnd, x: i32, y: i32, width: i32, height: i32) {
    unsafe {
        interop::SetWindowPos(
            hwnd,
            0,
            x,
            y,
            width,
            height,
            interop::SWP_NOZORDER | interop::SWP_NOACTIVATE | interop::SWP_NOSENDCHANGING,
        );
    }
}

pub fn show_no_activate(hwnd: Hwnd) {
    unsafe {
        interop::ShowWindow(hwnd, interop::SW_SHOWNOACTIVATE);
    }
}

pub fn hide(hwnd: Hwnd) {
    unsafe {
        interop::ShowWindow(hwnd, interop::SW_HIDE);
    }
}

pub fn is_visible(hwnd: Hwnd) -> bool {
    unsafe { interop::IsWindowVisible(hwnd) != 0 }
}

/// 自己管理点击穿透样式位，不走框架的窗口标志机（避免它把 NOACTIVATE / TOOLWINDOW 覆盖掉）。
///
/// `WS_EX_TRANSPARENT | WS_EX_LAYERED` 常开：切换穿透时不需要改动窗口区域，
/// 逐像素透明的 WebView2 内容在 layered 下依然正常合成。
pub fn set_click_through(hwnd: Hwnd, ignore: bool) {
    unsafe {
        let ex = interop::GetWindowLongPtrW(hwnd, interop::GWL_EXSTYLE);
        let mut next = ex | interop::WS_EX_TOOLWINDOW | interop::WS_EX_NOACTIVATE | interop::WS_EX_LAYERED;
        if ignore {
            next |= interop::WS_EX_TRANSPARENT;
        } else {
            next &= !interop::WS_EX_TRANSPARENT;
        }

        if next != ex {
            interop::SetWindowLongPtrW(hwnd, interop::GWL_EXSTYLE, next);
        }
    }
}

/// 把样式位重新压一遍（框架或其它进程可能改写），并返回实际值用于日志。
pub fn reassert_overlay_styles(hwnd: Hwnd) -> (isize, isize) {
    unsafe {
        let before = interop::GetWindowLongPtrW(hwnd, interop::GWL_EXSTYLE);
        let next = before
            | interop::WS_EX_TOOLWINDOW
            | interop::WS_EX_NOACTIVATE
            | interop::WS_EX_TOPMOST
            | interop::WS_EX_LAYERED;
        if next != before {
            interop::SetWindowLongPtrW(hwnd, interop::GWL_EXSTYLE, next);
        }
        (before, interop::GetWindowLongPtrW(hwnd, interop::GWL_EXSTYLE))
    }
}

/// 窗口矩形 → 客户区矩形的偏移与差值（物理像素）。
pub fn frame_thickness(hwnd: Hwnd) -> (i32, i32, i32, i32) {
    let Some(window_rect) = interop::window_rect(hwnd) else {
        return (0, 0, 0, 0);
    };
    let Some(client_rect) = interop::client_rect(hwnd) else {
        return (0, 0, 0, 0);
    };

    let mut origin = interop::POINT::default();
    if unsafe { interop::ClientToScreen(hwnd, &mut origin) } == 0 {
        return (0, 0, 0, 0);
    }

    (
        origin.x - window_rect.left,
        origin.y - window_rect.top,
        window_rect.width() - client_rect.width(),
        window_rect.height() - client_rect.height(),
    )
}

/// 普通窗口（设置窗）的原生标题栏跟随深浅色，并套上 Win11 系统圆角。
pub fn apply_titlebar_theme(hwnd: Hwnd, dark: bool) {
    unsafe {
        let value = if dark { 1i32 } else { 0i32 };
        interop::DwmSetWindowAttribute(
            hwnd,
            interop::DWMWA_USE_IMMERSIVE_DARK_MODE,
            &value as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );

        let corner = interop::DWMWCP_ROUND;
        interop::DwmSetWindowAttribute(
            hwnd,
            interop::DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );

        // 触发非客户区重算，标题栏才会立即换色
        interop::SetWindowPos(
            hwnd,
            0,
            0,
            0,
            0,
            0,
            interop::SWP_FRAMECHANGED
                | interop::SWP_NOMOVE
                | interop::SWP_NOSIZE
                | interop::SWP_NOZORDER
                | interop::SWP_NOACTIVATE,
        );
    }
}

/// 把普通窗口提到最前（从第二个实例唤出设置窗时进程没有前台权限）。
pub fn bring_to_front(hwnd: Hwnd) {
    unsafe {
        interop::SetWindowPos(
            hwnd,
            interop::HWND_TOPMOST,
            0,
            0,
            0,
            0,
            interop::SWP_NOMOVE | interop::SWP_NOSIZE,
        );
        interop::SetWindowPos(
            hwnd,
            interop::HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            interop::SWP_NOMOVE | interop::SWP_NOSIZE | interop::SWP_NOACTIVATE,
        );
        interop::SetForegroundWindow(hwnd);
    }
}
