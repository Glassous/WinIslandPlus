//! 配置读写：`%LOCALAPPDATA%\WinIslandPlus\settings.json`。
//!
//! 字段名统一 PascalCase，与前端 `src/common/types.ts` 一一对应。
//! 采用原子写入（临时文件 + 替换）、350 ms 防抖、损坏备份回落。

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::log;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlacementPreset {
    #[default]
    TopCenter,
    TopLeft,
    TopRight,
    BottomCenter,
    Custom,
}

/// 胶囊右侧显示什么：三者互斥，只生效一个。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PillRightStyle {
    /// 跳动频谱（默认）。
    #[default]
    Spectrum,
    /// 播放进度圈。
    ProgressRing,
    /// 当前一句歌词（胶囊宽度按文字长度向右伸展）。
    Lyrics,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "PascalCase", default)]
pub struct IslandSettings {
    pub placement: PlacementPreset,
    /// 自由落位锚点横向比例（相对显示器工作区，0~1）。
    pub anchor_x_ratio: f64,
    /// 自由落位锚点纵向比例（0~1）。
    pub anchor_y_ratio: f64,
    /// true 表示以锚点下沿为基准向上生长。
    pub bottom_anchored: bool,
    pub monitor_left: i32,
    pub monitor_top: i32,
    pub monitor_width: i32,
    pub monitor_height: i32,
    pub size: crate::metrics::IslandSizePreset,
    /// 自定义胶囊宽度（DIP），高度恒为该值的一半。
    pub custom_pill_width: f64,
    pub custom_pill_height: f64,
    pub persistent: bool,
    /// 胶囊右侧样式（频谱 / 进度圈 / 歌词），旧配置缺字段时为频谱。
    #[serde(default)]
    pub pill_right_style: PillRightStyle,
    pub auto_start: bool,
    pub auto_collapse_on_pause: bool,
    pub idle_collapse: bool,
    pub idle_collapse_seconds: f64,
    /// 是否接入 DeepSeek Harness 状态（旧配置缺字段时由 `default_dsh_enabled` 补 true）。
    #[serde(default = "default_dsh_enabled")]
    pub dsh_enabled: bool,
}

fn default_dsh_enabled() -> bool {
    true
}

impl Default for IslandSettings {
    fn default() -> Self {
        Self {
            placement: PlacementPreset::TopCenter,
            anchor_x_ratio: 0.5,
            anchor_y_ratio: 0.0,
            bottom_anchored: false,
            monitor_left: 0,
            monitor_top: 0,
            monitor_width: 0,
            monitor_height: 0,
            size: crate::metrics::IslandSizePreset::Medium,
            custom_pill_width: 68.0,
            custom_pill_height: 34.0,
            persistent: false,
            pill_right_style: PillRightStyle::Spectrum,
            auto_start: false,
            auto_collapse_on_pause: true,
            idle_collapse: true,
            idle_collapse_seconds: 2.5,
            dsh_enabled: true,
        }
    }
}

impl IslandSettings {
    /// 左右 / 底部的留白（DIP）。
    pub const EDGE_MARGIN: f64 = 8.0;

    /// 顶部落位时额外让出的距离（DIP），顶部实际留白 = 8 + 3 = 11 DIP。
    pub const TOP_GAP: f64 = 3.0;

    /// 收敛所有数值到合法区间，避免坏配置导致窗口跑出屏幕或尺寸异常。
    pub fn normalize(&mut self) {
        self.custom_pill_width = if self.custom_pill_width.is_finite() {
            self.custom_pill_width.clamp(60.0, 96.0)
        } else {
            68.0
        };
        self.custom_pill_height = self.custom_pill_width / 2.0;

        self.anchor_x_ratio = if self.anchor_x_ratio.is_finite() {
            self.anchor_x_ratio.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.anchor_y_ratio = if self.anchor_y_ratio.is_finite() {
            self.anchor_y_ratio.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.idle_collapse_seconds = if self.idle_collapse_seconds.is_finite() {
            self.idle_collapse_seconds.clamp(1.0, 15.0)
        } else {
            2.5
        };
    }

}

struct Inner {
    current: Mutex<IslandSettings>,
    path: PathBuf,
    debounce: Mutex<Option<Sender<()>>>,
}

/// 配置服务（克隆成本极低，内部 `Arc` 共享）。
#[derive(Clone)]
pub struct SettingsService {
    inner: Arc<Inner>,
}

impl SettingsService {
    pub fn new() -> Self {
        let path = log::ensure_root().join("settings.json");
        let current = load(&path);

        let (tx, rx) = channel::<()>();
        let inner = Arc::new(Inner {
            current: Mutex::new(current),
            path: path.clone(),
            debounce: Mutex::new(Some(tx)),
        });

        // 防抖落盘线程：收到信号后等待 350ms 静默期再写盘。
        let writer = inner.clone();
        std::thread::Builder::new()
            .name("winisland-settings".into())
            .spawn(move || {
                while rx.recv().is_ok() {
                    loop {
                        match rx.recv_timeout(Duration::from_millis(350)) {
                            Ok(()) => continue,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    flush_inner(&writer);
                }
            })
            .ok();

        Self { inner }
    }

    pub fn current(&self) -> IslandSettings {
        self.inner
            .current
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// 原地修改配置并返回最新值（自动 Normalize + 防抖落盘）。
    pub fn update<F: FnOnce(&mut IslandSettings)>(&self, mutate: F) -> IslandSettings {
        let updated = {
            let Ok(mut guard) = self.inner.current.lock() else {
                return IslandSettings::default();
            };
            mutate(&mut guard);
            guard.normalize();
            guard.clone()
        };

        self.save_debounced();
        updated
    }

    /// 整体替换（设置窗口保存 / 预览）。
    pub fn replace(&self, settings: IslandSettings) -> IslandSettings {
        let mut settings = settings;
        settings.normalize();

        if let Ok(mut guard) = self.inner.current.lock() {
            *guard = settings.clone();
        }

        self.save_debounced();
        settings
    }

    pub fn save_debounced(&self) {
        if let Ok(guard) = self.inner.debounce.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(());
            }
        }
    }

    /// 立刻落盘（退出、拖动结束等关键时刻）。
    pub fn flush(&self) {
        flush_inner(&self.inner);
    }
}

fn flush_inner(inner: &Inner) {
    let snapshot = match inner.current.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return,
    };

    let json = match serde_json::to_string_pretty(&snapshot) {
        Ok(text) => text,
        Err(err) => {
            log::error(&format!("settings: 序列化失败 {err}"));
            return;
        }
    };

    if let Some(parent) = inner.path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let temp = inner.path.with_extension("json.tmp");
    if let Err(err) = fs::write(&temp, json.as_bytes()) {
        log::error(&format!("settings: 写入临时文件失败 {err}"));
        return;
    }

    if let Err(err) = fs::rename(&temp, &inner.path) {
        log::error(&format!("settings: 替换配置文件失败 {err}"));
        let _ = fs::remove_file(&temp);
    }
}

fn load(path: &PathBuf) -> IslandSettings {
    if path.exists() {
        match read_settings(path) {
            Ok(settings) => {
                log::info("settings: 已加载");
                return settings;
            }
            Err(err) => {
                log::error(&format!("settings: 加载失败，回退默认配置（{err}）"));
                backup_broken(path);
                return IslandSettings::default();
            }
        }
    }

    log::info("settings: 首次运行，使用默认配置");
    IslandSettings::default()
}

fn read_settings(path: &PathBuf) -> Result<IslandSettings, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let mut settings: IslandSettings =
        serde_json::from_str(&text).map_err(|err| err.to_string())?;
    settings.normalize();
    Ok(settings)
}

fn backup_broken(path: &PathBuf) {
    let stamp = format!("{}{}", log::date_stamp(), log::time_stamp());
    let backup = path.with_extension(format!("json.broken-{stamp}"));
    if fs::rename(path, &backup).is_ok() {
        log::warn("settings: 已备份损坏配置");
    }
}
