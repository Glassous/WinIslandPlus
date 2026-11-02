//! 单一光标轮询器。
//!
//! 承担四件事，全部基于 `GetCursorPos` / `GetAsyncKeyState`，不依赖 WebView2 的指针事件：
//! 1) 形状外点击穿透（边沿触发 `set_ignore_cursor_events`，避免高频抖动）；
//! 2) 展开态「前台窗口变化 / 岛外按下 / 光标离开超时」自动收起；
//! 3) 设置窗口打开期间的拖动岛体（6 DIP 起拖阈值，松手保存并锁定）；
//! 4) 每 2 秒一次的置顶心跳（部分应用会自己抢 topmost）。

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter};

use crate::interop;
use crate::log;
use crate::placement;
use crate::state::{SharedState, EVENT_COLLAPSE};
use crate::window;

pub fn start(app: AppHandle, state: Arc<crate::state::Shared>) {
    std::thread::Builder::new()
        .name("winisland-poller".into())
        .spawn(move || {
            while !state.shutdown.load(Ordering::SeqCst) {
                let busy = state.with_runtime(|runtime| runtime.expanded || runtime.drag_enabled);
                tick(&app, &state);
                std::thread::sleep(Duration::from_millis(if busy { 16 } else { 33 }));
            }
            log::info("poller: 已退出");
        })
        .ok();
}

fn tick(app: &AppHandle, state: &SharedState) {
    let hwnd = state.hwnd();
    if hwnd == 0 {
        return;
    }

    let Some((cursor_x, cursor_y)) = interop::cursor_position() else {
        return;
    };

    let snapshot = state.snapshot();
    let local_x = (cursor_x - snapshot.window_origin.0) as f64;
    let local_y = (cursor_y - snapshot.window_origin.1) as f64;
    let inside = snapshot.shape.contains(local_x, local_y);

    ensure_visible(hwnd, state);
    update_click_through(state, inside);
    update_topmost_heartbeat(hwnd, state);
    track_playback(app, state, snapshot.expanded);

    if snapshot.expanded {
        watch_expanded(app, state, inside);
    }

    if state.with_runtime(|runtime| runtime.drag_enabled) {
        watch_drag(app, state, (cursor_x, cursor_y), inside);
    }
}

/// 兜底：任何外部因素把岛窗藏起来时，只要"应该显示"就重新显示。
fn ensure_visible(hwnd: interop::Hwnd, state: &SharedState) {
    if !state.should_show() {
        state.with_runtime(|runtime| runtime.hidden_ticks = 0);
        return;
    }

    if window::is_visible(hwnd) {
        state.with_runtime(|runtime| runtime.hidden_ticks = 0);
        return;
    }

    window::show_no_activate(hwnd);
    window::ensure_topmost(hwnd);
    state.with_runtime(|runtime| {
        runtime.hidden_ticks += 1;
        runtime.visible = true;
    });

    let ticks = state.with_runtime(|runtime| runtime.hidden_ticks);
    if ticks == 1 {
        log::info("island: 检测到窗口被隐藏，已重新显示");
    }
}

/// 形状外的点击交还给下方窗口：只在状态变化时切换，避免每帧都改样式位。
fn update_click_through(state: &SharedState, inside: bool) {
    let hwnd = state.hwnd();
    if hwnd == 0 {
        return;
    }

    let (visible, current) = state.with_runtime(|runtime| (runtime.visible, runtime.ignore_cursor));
    let want = visible && !inside;

    if want == current {
        return;
    }

    window::set_click_through(hwnd, want);
    state.with_runtime(|runtime| runtime.ignore_cursor = want);
}

fn update_topmost_heartbeat(hwnd: interop::Hwnd, state: &SharedState) {
    let now = now_ms();
    let visible = state.with_runtime(|runtime| runtime.visible);
    if !visible {
        return;
    }

    let last = state.with_runtime(|runtime| runtime.last_topmost_check_ms);
    if now - last < 2000.0 {
        return;
    }

    state.with_runtime(|runtime| runtime.last_topmost_check_ms = now);

    if !window::is_topmost(hwnd) {
        window::ensure_topmost(hwnd);
        log::info("interop: 置顶丢失，已重新压顶");
    }
}

/// 播放 → 暂停的瞬间：按设置自动收起（用户自己刚点暂停的 2 秒内不收起）。
fn track_playback(app: &AppHandle, state: &SharedState, expanded: bool) {
    let playing = state.media.is_playing();
    let (previous, _) = state.with_runtime(|runtime| {
        let previous = runtime.last_playing;
        runtime.last_playing = playing;
        (previous, ())
    });

    if !expanded {
        return;
    }

    if !(previous == Some(true) && playing == Some(false)) {
        return;
    }

    let settings = state.settings.current();
    if !settings.auto_collapse_on_pause {
        return;
    }

    let last_action = state.with_runtime(|runtime| runtime.last_user_action_ms);
    if now_ms() - last_action <= 2000.0 {
        return;
    }

    collapse(app, state, "pause");
}

fn watch_expanded(app: &AppHandle, state: &SharedState, inside: bool) {
    let foreground = interop::foreground_window();
    let before = state.with_runtime(|runtime| runtime.foreground_before_expand);

    if foreground != 0 && foreground != before {
        collapse(app, state, "foreground");
        return;
    }

    let any_down = interop::is_any_mouse_button_down();
    let last = state.with_runtime(|runtime| runtime.any_button_down);

    if any_down && !last && !inside {
        state.with_runtime(|runtime| runtime.any_button_down = any_down);
        collapse(app, state, "outside-click");
        return;
    }

    state.with_runtime(|runtime| runtime.any_button_down = any_down);

    let settings = state.settings.current();
    if !settings.idle_collapse || inside {
        state.with_runtime(|runtime| runtime.idle_since_ms = 0.0);
        return;
    }

    // 用时间戳而不是轮询次数：轮询周期会随负载浮动，按次数计时会系统性偏长
    let limit_ms = (settings.idle_collapse_seconds * 1000.0).max(300.0);
    let since = state.with_runtime(|runtime| {
        if runtime.idle_since_ms <= 0.0 {
            runtime.idle_since_ms = now_ms();
        }
        runtime.idle_since_ms
    });

    if now_ms() - since > limit_ms {
        collapse(app, state, "idle");
    }
}

fn collapse(app: &AppHandle, state: &SharedState, reason: &str) {
    if !state.with_runtime(|runtime| runtime.expanded) {
        return;
    }

    state.set_expanded(false);
    let _ = app.emit(EVENT_COLLAPSE, reason);
    log::info(&format!("island: 自动收起（{reason}）"));
}

fn watch_drag(app: &AppHandle, state: &SharedState, cursor: (i32, i32), inside: bool) {
    let down = interop::is_left_button_down();
    let (pointer_down, last_down) =
        state.with_runtime(|runtime| (runtime.drag_pointer_down, runtime.drag_button_last));

    if !pointer_down {
        if down && !last_down && inside {
            state.with_runtime(|runtime| {
                runtime.drag_pointer_down = true;
                runtime.dragging = false;
                runtime.drag_start_cursor = cursor;
                runtime.drag_start_anchor = (runtime.anchor.center_x, runtime.anchor.edge_y);
            });
        }

        state.with_runtime(|runtime| runtime.drag_button_last = down);
        return;
    }

    if !down {
        end_drag(app, state, true);
        state.with_runtime(|runtime| runtime.drag_button_last = false);
        return;
    }

    let (start_cursor, start_anchor, dragging) = state.with_runtime(|runtime| {
        (
            runtime.drag_start_cursor,
            runtime.drag_start_anchor,
            runtime.dragging,
        )
    });

    let dx = (cursor.0 - start_cursor.0) as f64;
    let dy = (cursor.1 - start_cursor.1) as f64;

    // 6 DIP 起拖阈值：滤掉按下时的细微抖动
    let threshold = 6.0 * state.snapshot().scale;
    if !dragging && dx.abs() + dy.abs() < threshold {
        return;
    }

    if !dragging {
        log::info("input: 开始拖动");
    }

    state.with_runtime(|runtime| {
        runtime.dragging = true;
        runtime.anchor.center_x = start_anchor.0 + dx;
        runtime.anchor.edge_y = start_anchor.1 + dy;
    });

    state.apply_placement(app);
}

/// 外部（关闭设置窗口）强制结束一次进行中的拖动，并保存位置。
pub fn force_end_drag(app: &AppHandle, state: &SharedState) {
    end_drag(app, state, true);
    state.settings.flush();
}

fn end_drag(app: &AppHandle, state: &SharedState, persist: bool) {
    let dragging = state.with_runtime(|runtime| {
        let was_dragging = runtime.dragging;
        runtime.dragging = false;
        runtime.drag_pointer_down = false;
        was_dragging
    });

    if !dragging || !persist {
        return;
    }

    let snapshot = state.snapshot();
    let scale = snapshot.scale;
    let pill_height = snapshot.metrics.pill_height * scale;
    let pill_top = if snapshot.bottom_anchored {
        snapshot.anchor.edge_y - pill_height
    } else {
        snapshot.anchor.edge_y
    };

    let center_x = snapshot.anchor.center_x;
    let work = state.with_runtime(|runtime| runtime.work);

    let updated = state
        .settings
        .update(|settings| placement::capture_from_anchor(settings, work, scale, center_x, pill_top, pill_height));
    state.settings.flush();

    // 上下半区切换会改变生长方向：同步内存锚点，否则重启后会差一个胶囊高度
    state.with_runtime(|runtime| {
        runtime.bottom_anchored = updated.bottom_anchored;
        runtime.anchor.bottom_anchored = updated.bottom_anchored;
        runtime.anchor.edge_y = if updated.bottom_anchored {
            pill_top + pill_height
        } else {
            pill_top
        };
    });

    state.apply_placement(app);
    state.notify_settings(app);
    crate::tray::refresh(app, state.as_ref());

    log::info(&format!(
        "input: 位置已保存 centerX={center_x:.1} top={pill_top:.1} ratio=({:.4},{:.4}) bottom={}",
        updated.anchor_x_ratio, updated.anchor_y_ratio, updated.bottom_anchored
    ));
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}
