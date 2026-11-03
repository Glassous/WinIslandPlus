//! 共享运行时状态：配置、媒体快照、形状矩形、展开态、拖动解锁态。
//!
//! 所有跨层变更（命令、轮询器、托盘）都经过这里，保证只有一处真相。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};

use crate::dsh::DshService;
use crate::interop::Hwnd;
use crate::media::MediaService;
use crate::metrics::IslandMetrics;
use crate::placement::{self, Anchor, ShapeRect, WorkArea};
use crate::settings::SettingsService;
use crate::window;

pub const EVENT_SETTINGS_CHANGED: &str = "settings://changed";
pub const EVENT_VISIBILITY: &str = "island://visibility";
pub const EVENT_COLLAPSE: &str = "island://collapse";
pub const EVENT_OPEN_SETTINGS: &str = "settings://open";

#[derive(Debug)]
pub struct Runtime {
    pub metrics: IslandMetrics,
    pub bottom_anchored: bool,
    pub work: WorkArea,
    pub scale: f64,
    pub anchor: Anchor,
    /// 岛面形状（相对窗口客户区的物理像素）：媒体岛、dsh 岛，展开时再加一条胶囊行横带。
    pub shapes: Vec<ShapeRect>,
    /// 窗口客户区原点（屏幕物理像素）。
    pub window_origin: (i32, i32),
    pub expanded: bool,
    pub drag_enabled: bool,
    pub dragging: bool,
    pub user_hidden: bool,
    pub visible: bool,
    pub ignore_cursor: bool,
    /// 展开时的前台窗口（用于"切走即收起"）。
    pub foreground_before_expand: Hwnd,
    pub any_button_down: bool,
    /// 最近一次展开的时刻（用于岛外点击的宽限期）。
    pub expanded_at_ms: f64,
    /// 光标离开岛体的起始时刻（0 表示当前在岛内）。
    pub idle_since_ms: f64,
    /// 上一次媒体播放状态，用于检测"播放 → 暂停"的瞬间。
    pub last_playing: Option<bool>,
    /// 用户最后一次主动操作卡片（按钮 / 进度条）的时刻。
    pub last_user_action_ms: f64,
    /// 连续检测到"应该显示却被隐藏"的轮询次数（用于一次性日志）。
    pub hidden_ticks: i32,
    pub last_topmost_check_ms: f64,
    /// 拖动状态
    pub drag_pointer_down: bool,
    pub drag_button_last: bool,
    pub drag_start_cursor: (i32, i32),
    pub drag_start_anchor: (f64, f64),
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            metrics: IslandMetrics::for_preset(crate::metrics::IslandSizePreset::Medium, 68.0),
            bottom_anchored: false,
            work: WorkArea::default(),
            scale: 1.0,
            anchor: Anchor {
                center_x: 0.0,
                edge_y: 0.0,
                bottom_anchored: false,
            },
            shapes: Vec::new(),
            window_origin: (0, 0),
            expanded: false,
            drag_enabled: false,
            dragging: false,
            user_hidden: false,
            visible: false,
            ignore_cursor: false,
            foreground_before_expand: 0,
            any_button_down: false,
            expanded_at_ms: 0.0,
            idle_since_ms: 0.0,
            last_playing: None,
            last_user_action_ms: 0.0,
            hidden_ticks: 0,
            last_topmost_check_ms: 0.0,
            drag_pointer_down: false,
            drag_button_last: false,
            drag_start_cursor: (0, 0),
            drag_start_anchor: (0.0, 0.0),
        }
    }
}

pub struct Shared {
    pub settings: SettingsService,
    pub media: MediaService,
    pub dsh: DshService,
    pub runtime: Mutex<Runtime>,
    pub hwnd: Mutex<Hwnd>,
    pub shutdown: AtomicBool,
}

pub type SharedState = Arc<Shared>;

impl Shared {
    pub fn new(settings: SettingsService, media: MediaService, dsh: DshService) -> Self {
        Self {
            settings,
            media,
            dsh,
            runtime: Mutex::new(Runtime::default()),
            hwnd: Mutex::new(0),
            shutdown: AtomicBool::new(false),
        }
    }

    pub fn hwnd(&self) -> Hwnd {
        self.hwnd.lock().map(|guard| *guard).unwrap_or(0)
    }

    pub fn set_hwnd(&self, hwnd: Hwnd) {
        if let Ok(mut guard) = self.hwnd.lock() {
            *guard = hwnd;
        }
    }

    pub fn with_runtime<T>(&self, f: impl FnOnce(&mut Runtime) -> T) -> T {
        let mut guard = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        let guard = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        RuntimeSnapshot {
            metrics: guard.metrics,
            bottom_anchored: guard.bottom_anchored,
            scale: guard.scale,
            anchor: guard.anchor,
            shapes: guard.shapes.clone(),
            window_origin: guard.window_origin,
            work: guard.work,
            expanded: guard.expanded,
        }
    }

    /// 重新解算 metrics / 工作区 / 缩放 / 锚点，并同步窗口落位。
    pub fn recompute(&self, app: &AppHandle) {
        let settings = self.settings.current();
        let metrics = IslandMetrics::for_preset(settings.size, settings.custom_pill_width);

        let hwnd = self.hwnd();
        let work = if hwnd != 0 {
            placement::get_work_area(hwnd)
        } else {
            WorkArea {
                left: 0,
                top: 0,
                width: 1920,
                height: 1080,
            }
        };
        let scale = if hwnd != 0 { placement::get_scale(hwnd) } else { 1.0 };
        let anchor = placement::resolve_anchor(&settings, work, scale, &metrics);

        self.with_runtime(|runtime| {
            runtime.metrics = metrics;
            runtime.work = work;
            runtime.scale = scale;
            runtime.anchor = anchor;
            runtime.bottom_anchored = anchor.bottom_anchored;
        });

        self.apply_placement(app);
    }

    /// 按当前锚点与卡片尺寸摆放窗口（窗口尺寸固定为「胶囊行 + 间距 + 卡片行」，形状在内部动画）。
    pub fn apply_placement(&self, _app: &AppHandle) {
        let hwnd = self.hwnd();
        if hwnd == 0 {
            return;
        }

        let snapshot = self.snapshot();
        let scale = snapshot.scale;

        let window_width = (snapshot.metrics.card_width * scale).round().max(1.0) as i32;
        // 高度恒定：展开的卡片长在胶囊行的下一行，窗口本身不改尺寸
        let window_height = (snapshot.metrics.window_height() * scale)
            .round()
            .max(1.0) as i32;

        let mut x = (snapshot.anchor.center_x - window_width as f64 / 2.0).round() as i32;
        let mut y = if snapshot.bottom_anchored {
            (snapshot.anchor.edge_y - window_height as f64).round() as i32
        } else {
            snapshot.anchor.edge_y.round() as i32
        };

        let work = snapshot.work;
        x = x.clamp(work.left, (work.right() - window_width).max(work.left));
        y = y.clamp(work.top, (work.bottom() - window_height).max(work.top));

        let (frame_left, frame_top, frame_width, frame_height) = window::frame_thickness(hwnd);

        window::set_bounds(
            hwnd,
            x - frame_left,
            y - frame_top,
            window_width + frame_width,
            window_height + frame_height,
        );

        self.with_runtime(|runtime| {
            runtime.window_origin = (x, y);
        });
    }

    pub fn should_show(&self) -> bool {
        let settings = self.settings.current();
        let user_hidden = self.with_runtime(|runtime| runtime.user_hidden);
        let dsh_active = settings.dsh_enabled && self.dsh.is_active();
        !user_hidden && (self.media.current().is_some() || dsh_active || settings.persistent)
    }

    /// 根据媒体 / 常驻 / 用户显隐决定岛窗是否可见。
    pub fn sync_visibility(&self, app: &AppHandle) {
        let hwnd = self.hwnd();
        if hwnd == 0 {
            return;
        }

        let should_show = self.should_show();
        let already_visible = self.with_runtime(|runtime| runtime.visible);

        if should_show {
            if !already_visible {
                window::show_no_activate(hwnd);
                window::ensure_topmost(hwnd);
                self.with_runtime(|runtime| runtime.visible = true);
                let origin = self.snapshot().window_origin;
                crate::log::info(&format!("island: 显示 origin={origin:?}"));
            }
        } else if already_visible {
            window::hide(hwnd);
            self.with_runtime(|runtime| {
                runtime.visible = false;
                runtime.expanded = false;
            });
        }

        let _ = app.emit(EVENT_VISIBILITY, should_show);
    }

    /// 展开 / 收起：记录前台窗口与按键基线，供轮询器判断"切走即收起"。
    pub fn set_expanded(&self, expanded: bool) {
        let foreground = if expanded {
            crate::interop::foreground_window()
        } else {
            0
        };
        let now = now_ms();

        self.with_runtime(|runtime| {
            runtime.expanded = expanded;
            runtime.foreground_before_expand = foreground;
            // 展开这一下本身就是一次按下：基线置为「按住」，必须先松手再按下才算岛外点击，
            // 否则打开卡片的那次点击会被误判成岛外点击、卡片刚出来就被收起。
            runtime.any_button_down = expanded;
            runtime.expanded_at_ms = if expanded { now } else { 0.0 };
            runtime.idle_since_ms = 0.0;
        });
    }

    /// 用户主动操作了卡片：2 秒内不因"暂停"自动收起。
    pub fn note_user_action(&self) {
        let now = now_ms();
        self.with_runtime(|runtime| runtime.last_user_action_ms = now);
    }

    pub fn notify_settings(&self, app: &AppHandle) {
        let _ = app.emit(EVENT_SETTINGS_CHANGED, self.settings.current());
    }

    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

/// 当前时刻（毫秒，与轮询器同名函数同一基准）。
fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// 运行时状态的只读快照（避免长时间持锁）。
///
/// 形状是变长的（最多两块），所以这里只实现 `Clone`——竞态下也不要紧：
/// 快照只是一帧的拍照，轮询器下一帧会重新取。
#[derive(Clone, Debug)]
pub struct RuntimeSnapshot {
    pub metrics: IslandMetrics,
    pub bottom_anchored: bool,
    pub scale: f64,
    pub anchor: Anchor,
    pub shapes: Vec<ShapeRect>,
    pub window_origin: (i32, i32),
    pub work: WorkArea,
    pub expanded: bool,
}

/// 便捷：取得岛窗（不存在时返回 None）。
pub fn island_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("island")
}

/// 便捷：取得设置窗。
pub fn settings_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("settings")
}
