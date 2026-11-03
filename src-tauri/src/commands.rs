//! IPC 命令层：前端与后端之间唯一的入口，保持极薄——只做参数校验与状态转交。

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::dsh::DshSnapshot;
use crate::log;
use crate::media::MediaSnapshot;
use crate::metrics::IslandMetrics;
use crate::placement::ShapeRect;
use crate::settings::IslandSettings;
use crate::state::{self, SharedState};
use crate::window;
use crate::{startup, tray};

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Bootstrap {
    pub settings: IslandSettings,
    pub media: Option<MediaSnapshot>,
    pub dsh: Option<DshSnapshot>,
    pub metrics: IslandMetrics,
    pub scale: f64,
    pub bottom_anchored: bool,
    pub visible: bool,
    pub version: String,
    pub executable_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AppInfo {
    pub version: String,
    pub executable_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct CursorProbe {
    /// 相对岛窗客户区的 CSS 像素坐标。
    pub x: f64,
    pub y: f64,
    /// 左键是否按下（拖动进度条时判断松手）。
    pub down: bool,
}

#[tauri::command]
pub fn bootstrap(state: State<'_, SharedState>) -> Bootstrap {
    let snapshot = state.snapshot();
    let settings = state.settings.current();
    let dsh = if settings.dsh_enabled {
        state.dsh.current()
    } else {
        None
    };
    Bootstrap {
        settings,
        media: state.media.current(),
        dsh,
        metrics: snapshot.metrics,
        scale: snapshot.scale,
        bottom_anchored: snapshot.bottom_anchored,
        visible: state.should_show(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        executable_path: startup::executable_path(),
    }
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        executable_path: startup::executable_path(),
    }
}

/// 前端首帧绘制完成：此时才显示窗口，避免启动瞬间的白闪。
#[tauri::command]
pub fn island_ready(app: AppHandle, state: State<'_, SharedState>) {
    log::info(&format!(
        "island: 前端首帧就绪 hwnd={} persistent={} media={}",
        state.hwnd(),
        state.settings.current().persistent,
        state.media.current().is_some()
    ));
    state.recompute(&app);
    state.sync_visibility(&app);
}

/// 前端每帧推来的可见形状：媒体岛、dsh 岛，展开时再加一条胶囊行横带。
///
/// 多于四块时截断：布局里本来就只有这些块，多出来的只可能是异常数据。
#[tauri::command]
pub fn set_island_shapes(state: State<'_, SharedState>, shapes: Vec<ShapeRect>) {
    let mut shapes = shapes;
    shapes.truncate(4);
    state.with_runtime(|runtime| runtime.shapes = shapes);
}

#[tauri::command]
pub fn set_expanded(app: AppHandle, state: State<'_, SharedState>, expanded: bool) {
    state.set_expanded(expanded);
    if expanded {
        state.sync_visibility(&app);
    }
}

#[tauri::command]
pub fn set_dragging(state: State<'_, SharedState>, dragging: bool) {
    state.with_runtime(|runtime| runtime.dragging = dragging);
}

/// 用户主动操作了卡片（按钮 / 进度条）：2 秒内不因"暂停"自动收起。
#[tauri::command]
pub fn notify_user_acted(state: State<'_, SharedState>) {
    state.note_user_action();
}

#[tauri::command]
pub fn media_toggle_play_pause(state: State<'_, SharedState>) -> bool {
    state.media.toggle_play_pause()
}

#[tauri::command]
pub fn media_next(state: State<'_, SharedState>) -> bool {
    state.media.next()
}

#[tauri::command]
pub fn media_previous(state: State<'_, SharedState>) -> bool {
    state.media.previous()
}

#[tauri::command]
pub fn media_seek(state: State<'_, SharedState>, position_ms: f64) -> bool {
    state.media.seek(position_ms)
}

#[tauri::command]
pub fn get_settings(state: State<'_, SharedState>) -> IslandSettings {
    state.settings.current()
}

/// 光标探针：进度条拖动期间由前端轮询，避免依赖 WebView2 的 pointermove。
#[tauri::command]
pub fn cursor_probe(state: State<'_, SharedState>) -> Option<CursorProbe> {
    let (x, y) = crate::interop::cursor_position()?;
    let snapshot = state.snapshot();
    let scale = if snapshot.scale > 0.0 {
        snapshot.scale
    } else {
        1.0
    };

    Some(CursorProbe {
        x: (x - snapshot.window_origin.0) as f64 / scale,
        y: (y - snapshot.window_origin.1) as f64 / scale,
        down: crate::interop::is_left_button_down(),
    })
}

/// 设置窗口的「即时预览」：整体替换后立即生效。
#[tauri::command]
pub fn replace_settings(
    app: AppHandle,
    state: State<'_, SharedState>,
    settings: IslandSettings,
) -> IslandSettings {
    let updated = state.settings.replace(settings);
    state.recompute(&app);
    state.sync_visibility(&app);
    state.notify_settings(&app);
    tray::refresh(&app, &state);
    updated
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<'_, SharedState>, settings: IslandSettings) {
    state.settings.replace(settings);
    state.settings.flush();
    state.recompute(&app);
    state.sync_visibility(&app);
    state.notify_settings(&app);
    tray::refresh(&app, &state);
    close_settings_window(app);
}

#[tauri::command]
pub fn cancel_settings(app: AppHandle, state: State<'_, SharedState>, settings: IslandSettings) {
    state.settings.replace(settings);
    state.settings.flush();
    state.recompute(&app);
    state.sync_visibility(&app);
    state.notify_settings(&app);
    tray::refresh(&app, &state);
    close_settings_window(app);
}

#[tauri::command]
pub fn close_settings_window(app: AppHandle) {
    if let Some(window) = state::settings_window(&app) {
        let _ = window.close();
    }
}

#[tauri::command]
pub fn get_startup_enabled() -> bool {
    startup::is_enabled()
}

#[tauri::command]
pub fn set_startup_enabled(state: State<'_, SharedState>, enabled: bool) -> bool {
    let ok = startup::set_enabled(enabled);
    if ok {
        state.settings.update(|settings| settings.auto_start = enabled);
    }
    ok
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle) {
    open_settings(&app);
}

#[tauri::command]
pub fn toggle_island_visible(app: AppHandle) -> bool {
    let state = app.state::<SharedState>();
    let hidden = state.with_runtime(|runtime| {
        runtime.user_hidden = !runtime.user_hidden;
        runtime.user_hidden
    });

    if hidden {
        state.set_expanded(false);
    }

    state.sync_visibility(&app);
    !hidden
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    let state = app.state::<SharedState>();
    log::info("app: 请求退出");
    state.stop();
    state.settings.flush();
    app.exit(0);
}

#[tauri::command]
pub fn frontend_log(level: String, message: String) {
    match level.as_str() {
        "error" => log::error(&format!("frontend: {message}")),
        "warn" => log::warn(&format!("frontend: {message}")),
        _ => log::info(&format!("frontend: {message}")),
    }
}

/// 打开（或唤出）设置窗口；窗口关闭即锁定岛体位置。
pub fn open_settings(app: &AppHandle) {
    let state = app.state::<SharedState>();

    if let Some(window) = state::settings_window(app) {
        let _ = window.show();
        let _ = window.set_focus();
        if let Some(hwnd) = window::hwnd_of(&window) {
            window::bring_to_front(hwnd);
        }
        state.with_runtime(|runtime| runtime.drag_enabled = true);
        return;
    }

    let result = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("WinIslandPlus 设置")
        .inner_size(560.0, 720.0)
        .min_inner_size(480.0, 560.0)
        .resizable(true)
        .center()
        .build();

    match result {
        Ok(window) => {
            log::info("settings: 已创建设置窗口");

            if let Some(hwnd) = window::hwnd_of(&window) {
                window::apply_titlebar_theme(hwnd, startup::is_dark_mode());
                window::bring_to_front(hwnd);
            }

            let _ = window.set_focus();
            state.with_runtime(|runtime| runtime.drag_enabled = true);
            let _ = app.emit(state::EVENT_OPEN_SETTINGS, true);

            let app_handle = app.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    let state = app_handle.state::<SharedState>();

                    // 若关闭窗口时还按着岛体拖动，先把最后一段位移落盘
                    crate::poller::force_end_drag(&app_handle, &state);
                    state.with_runtime(|runtime| {
                        runtime.drag_enabled = false;
                        runtime.dragging = false;
                    });

                    log::info("settings: 已关闭，位置已锁定");
                    let _ = app_handle.emit(state::EVENT_OPEN_SETTINGS, false);
                }
            });
        }
        Err(err) => log::error(&format!("settings: 创建设置窗口失败 {err}")),
    }
}
