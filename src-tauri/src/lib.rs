//! WinIslandPlus：面向 Windows 11 的灵动岛。
//!
//! 启动顺序：日志 → 配置 → 单实例 → 岛窗原生样式 → 落位 → 媒体线程 → 托盘 → 轮询器。

mod commands;
mod dsh;
mod interop;
mod log;
mod lyrics;
mod media;
mod metrics;
mod placement;
mod poller;
mod settings;
mod startup;
mod state;
mod tray;
mod window;

use std::sync::Arc;

use tauri::Manager;

use crate::dsh::DshService;
use crate::lyrics::LyricsService;
use crate::media::MediaService;
use crate::settings::SettingsService;
use crate::state::{Shared, SharedState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    log::init();

    let settings = SettingsService::new();

    // 让配置里的自启状态与注册表实际状态保持一致
    let actual_autostart = startup::is_enabled();
    if settings.current().auto_start != actual_autostart {
        settings.update(|current| current.auto_start = actual_autostart);
    }

    let media = MediaService::new();
    let dsh = DshService::new();
    let lyrics = LyricsService::new();
    let shared: SharedState = Arc::new(Shared::new(settings, media.clone(), dsh.clone()));
    let setup_state = shared.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log::info("app: 已有实例在运行，唤出设置窗口");
            commands::open_settings(app);
        }))
        .manage(shared)
        .on_menu_event(|app, event| {
            tray::handle_menu_event(app, event.id().0.as_str());
        })
        .setup(move |app| {
            let handle = app.handle().clone();

            if let Some(window) = state::island_window(&handle) {
                if let Some(hwnd) = window::hwnd_of(&window) {
                    setup_state.set_hwnd(hwnd);
                    window::apply_overlay_styles(hwnd);
                    window::install_subclass(hwnd);

                    // 初始全穿透：等前端把第一帧形状推过来之后再按命中判定
                    window::set_click_through(hwnd, true);
                    setup_state.with_runtime(|runtime| runtime.ignore_cursor = true);
                } else {
                    log::error("app: 无法取得岛窗句柄");
                }
            }

            setup_state.recompute(&handle);
            media.start(handle.clone(), lyrics.clone());
            lyrics.start(handle.clone());
            dsh.start(handle.clone());
            tray::create(&handle, &setup_state);
            poller::start(handle.clone(), setup_state.clone());

            log::info("app: 启动完成");
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if window.label() == "island" {
                    api.prevent_close();
                }
            }
            tauri::WindowEvent::ScaleFactorChanged { .. } => {
                if window.label() == "island" {
                    let app = window.app_handle();
                    let state = app.state::<SharedState>();
                    state.recompute(app);
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::app_info,
            commands::island_ready,
            commands::set_island_shapes,
            commands::set_expanded,
            commands::set_dragging,
            commands::notify_user_acted,
            commands::media_toggle_play_pause,
            commands::media_next,
            commands::media_previous,
            commands::media_seek,
            commands::get_settings,
            commands::cursor_probe,
            commands::replace_settings,
            commands::save_settings,
            commands::cancel_settings,
            commands::close_settings_window,
            commands::get_startup_enabled,
            commands::set_startup_enabled,
            commands::open_settings_window,
            commands::toggle_island_visible,
            commands::quit_app,
            commands::frontend_log,
        ])
        .build(tauri::generate_context!())
        .expect("WinIslandPlus 启动失败")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<SharedState>();
                state.dsh.stop();
                state.stop();
                state.settings.flush();
                log::info("app: 已退出");
            }
        });
}
