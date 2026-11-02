//! 原生托盘图标与右键菜单。
//!
//! 图标在运行时绘制（黑色胶囊 + 绿色状态点），不提交任何二进制资源；
//! 菜单在每次右键时按当前配置重新构建，勾选状态永远是最新的。

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::log;
use crate::metrics::IslandSizePreset;
use crate::settings::PlacementPreset;
use crate::state::{Shared, SharedState};
use crate::{commands, startup};

const TRAY_ID: &str = "winislandplus";

pub fn create(app: &AppHandle, state: &SharedState) {
    let icon = Image::new_owned(tray_icon_rgba(), 32, 32);
    let menu = build_menu(app, state).ok();

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("WinIslandPlus - Windows 11 灵动岛")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            let app = tray.app_handle().clone();
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_user_hidden(&app);
            }
        });

    if let Some(menu) = menu {
        builder = builder.menu(&menu);
    }

    match builder.build(app) {
        Ok(_) => log::info("tray: 托盘图标已创建"),
        Err(err) => log::error(&format!("tray: 创建托盘图标失败 {err}")),
    }
}

/// 配置变化后重建菜单，保证勾选状态与文案最新。
pub fn refresh(app: &AppHandle, state: &Shared) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    match build_menu(app, state) {
        Ok(menu) => {
            if let Err(err) = tray.set_menu(Some(menu)) {
                log::warn(&format!("tray: 刷新菜单失败 {err}"));
            }
        }
        Err(err) => log::error(&format!("tray: 构建菜单失败 {err}")),
    }
}

fn build_menu(app: &AppHandle, state: &Shared) -> tauri::Result<Menu<Wry>> {
    let settings = state.settings.current();
    let user_hidden = state.with_runtime(|runtime| runtime.user_hidden);

    let show_hide = MenuItem::with_id(
        app,
        "show_hide",
        if user_hidden {
            "显示岛屿\t左键"
        } else {
            "隐藏岛屿\t左键"
        },
        true,
        None::<&str>,
    )?;

    let separator = PredefinedMenuItem::separator(app)?;

    let pos_center = CheckMenuItem::with_id(
        app,
        "pos_top_center",
        "顶部居中",
        true,
        settings.placement == PlacementPreset::TopCenter,
        None::<&str>,
    )?;
    let pos_left = CheckMenuItem::with_id(
        app,
        "pos_top_left",
        "顶部左侧",
        true,
        settings.placement == PlacementPreset::TopLeft,
        None::<&str>,
    )?;
    let pos_right = CheckMenuItem::with_id(
        app,
        "pos_top_right",
        "顶部右侧",
        true,
        settings.placement == PlacementPreset::TopRight,
        None::<&str>,
    )?;
    let pos_bottom = CheckMenuItem::with_id(
        app,
        "pos_bottom_center",
        "底部居中",
        true,
        settings.placement == PlacementPreset::BottomCenter,
        None::<&str>,
    )?;
    let pos_custom = CheckMenuItem::with_id(
        app,
        "pos_custom",
        "自定义（设置中拖动）",
        false,
        settings.placement == PlacementPreset::Custom,
        None::<&str>,
    )?;

    let position_menu = Submenu::with_items(
        app,
        "位置",
        true,
        &[&pos_center, &pos_left, &pos_right, &pos_bottom, &pos_custom],
    )?;

    let size_small = CheckMenuItem::with_id(
        app,
        "size_small",
        "小",
        true,
        settings.size == IslandSizePreset::Small,
        None::<&str>,
    )?;
    let size_medium = CheckMenuItem::with_id(
        app,
        "size_medium",
        "中",
        true,
        settings.size == IslandSizePreset::Medium,
        None::<&str>,
    )?;
    let size_large = CheckMenuItem::with_id(
        app,
        "size_large",
        "大",
        true,
        settings.size == IslandSizePreset::Large,
        None::<&str>,
    )?;
    let size_custom = CheckMenuItem::with_id(
        app,
        "size_custom",
        "自定义（设置中调整）",
        false,
        settings.size == IslandSizePreset::Custom,
        None::<&str>,
    )?;

    let size_menu = Submenu::with_items(
        app,
        "尺寸",
        true,
        &[&size_small, &size_medium, &size_large, &size_custom],
    )?;

    let middle_separator = PredefinedMenuItem::separator(app)?;

    let persistent = CheckMenuItem::with_id(
        app,
        "persistent",
        "常驻模式",
        true,
        settings.persistent,
        None::<&str>,
    )?;

    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "开机自启",
        true,
        startup::is_enabled(),
        None::<&str>,
    )?;

    let settings_item = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let exit_item = MenuItem::with_id(app, "exit", "退出", true, None::<&str>)?;

    Menu::with_items(
        app,
        &[
            &show_hide,
            &separator,
            &position_menu,
            &size_menu,
            &middle_separator,
            &persistent,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &settings_item,
            &exit_item,
        ],
    )
}

/// 菜单事件集中处理（菜单项 id 与上面构建时一致）。
pub fn handle_menu_event(app: &AppHandle, id: &str) {
    let state = app.state::<SharedState>();

    match id {
        "show_hide" => toggle_user_hidden(app),
        "pos_top_center" => set_placement(app, &state, PlacementPreset::TopCenter),
        "pos_top_left" => set_placement(app, &state, PlacementPreset::TopLeft),
        "pos_top_right" => set_placement(app, &state, PlacementPreset::TopRight),
        "pos_bottom_center" => set_placement(app, &state, PlacementPreset::BottomCenter),
        "size_small" => set_size(app, &state, IslandSizePreset::Small),
        "size_medium" => set_size(app, &state, IslandSizePreset::Medium),
        "size_large" => set_size(app, &state, IslandSizePreset::Large),
        "persistent" => {
            state
                .settings
                .update(|settings| settings.persistent = !settings.persistent);
            state.sync_visibility(app);
            state.notify_settings(app);
            refresh(app, &state);
        }
        "autostart" => {
            let target = !startup::is_enabled();
            if startup::set_enabled(target) {
                state
                    .settings
                    .update(|settings| settings.auto_start = target);
                state.notify_settings(app);
                refresh(app, &state);
            }
        }
        "settings" => commands::open_settings(app),
        "exit" => {
            log::info("tray: 用户选择退出");
            state.stop();
            app.exit(0);
        }
        _ => {}
    }
}

fn toggle_user_hidden(app: &AppHandle) {
    let state = app.state::<SharedState>();
    let hidden = state.with_runtime(|runtime| {
        runtime.user_hidden = !runtime.user_hidden;
        runtime.user_hidden
    });

    if hidden {
        state.set_expanded(false);
    }

    state.sync_visibility(app);
    refresh(app, &state);
    log::info(&format!("island: 用户切换显隐 -> hidden={hidden}"));
}

fn set_placement(app: &AppHandle, state: &SharedState, preset: PlacementPreset) {
    state.settings.update(|settings| {
        settings.placement = preset;
        if preset == PlacementPreset::BottomCenter {
            settings.bottom_anchored = true;
        } else if preset != PlacementPreset::Custom {
            settings.bottom_anchored = false;
        }
    });

    state.recompute(app);
    state.sync_visibility(app);
    state.notify_settings(app);
    refresh(app, state);
}

fn set_size(app: &AppHandle, state: &SharedState, preset: IslandSizePreset) {
    state.settings.update(|settings| settings.size = preset);
    state.recompute(app);
    state.notify_settings(app);
    refresh(app, state);
}

// ---------------- 图标绘制 ----------------

fn tray_icon_rgba() -> Vec<u8> {
    const SIZE: u32 = 32;
    const SAMPLES: u32 = 4;

    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut capsule = 0u32;
            let mut border = 0u32;
            let mut dot = 0u32;

            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = x as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SAMPLES as f32;

                    if inside_round_rect(px, py, 4.0, 9.0, 24.0, 14.0, 7.0) {
                        capsule += 1;

                        // 1px 半透明白描边（60/255 alpha）
                        if !inside_round_rect(px, py, 5.0, 10.0, 22.0, 12.0, 6.0) {
                            border += 1;
                        }
                    }

                    let dx = px - 17.5;
                    let dy = py - 16.5;
                    if dx * dx + dy * dy <= 2.5 * 2.5 {
                        dot += 1;
                    }
                }
            }

            let total = (SAMPLES * SAMPLES) as f32;
            let capsule_alpha = capsule as f32 / total;
            if capsule_alpha <= 0.0 {
                continue;
            }

            // 描边叠加在填充之上
            let border_alpha = (border as f32 / total) * (60.0 / 255.0);
            let mut r = 22.0 * (1.0 - border_alpha) + 255.0 * border_alpha;
            let mut g = 22.0 * (1.0 - border_alpha) + 255.0 * border_alpha;
            let mut b = 24.0 * (1.0 - border_alpha) + 255.0 * border_alpha;

            // 绿色状态点再叠一层
            let mix = dot as f32 / total;
            r = r * (1.0 - mix) + 61.0 * mix;
            g = g * (1.0 - mix) + 220.0 * mix;
            b = b * (1.0 - mix) + 132.0 * mix;

            let index = ((y * SIZE + x) * 4) as usize;
            rgba[index] = r as u8;
            rgba[index + 1] = g as u8;
            rgba[index + 2] = b as u8;
            rgba[index + 3] = (capsule_alpha * 255.0) as u8;
        }
    }

    rgba
}

fn inside_round_rect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, radius: f32) -> bool {
    if px < x || py < y || px > x + w || py > y + h {
        return false;
    }

    let cx = px.clamp(x + radius, x + w - radius);
    let cy = py.clamp(y + radius, y + h - radius);
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= radius * radius
}
