//! 落位计算：把几何换算收敛到纯函数里，保证不同 DPI / 多显示器下行为一致。
//!
//! 约定：锚点 X 为岛体水平中心；纵向锚点取决于 `BottomAnchored`（贴下沿时向上生长）。
//! 所有换算一律使用浮点，避免"取整 → 反解 → 再取整"的累计漂移。

use serde::{Deserialize, Serialize};

use crate::interop;
use crate::metrics::IslandMetrics;
use crate::settings::{IslandSettings, PlacementPreset};

/// 当前可见形状（相对窗口客户区的物理像素）。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "PascalCase")]
pub struct ShapeRect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    /// 圆角半径（物理像素）。
    pub radius: f64,
}

impl ShapeRect {
    /// 圆角矩形命中测试（坐标相对窗口客户区）。
    pub fn contains(&self, x: f64, y: f64) -> bool {
        if self.width <= 0.0 || self.height <= 0.0 {
            return false;
        }

        let local_x = x - self.left;
        let local_y = y - self.top;
        if local_x < 0.0 || local_y < 0.0 || local_x >= self.width || local_y >= self.height {
            return false;
        }

        let radius = self.radius.min(self.width.min(self.height) / 2.0);
        if radius <= 0.5 {
            return true;
        }

        let (center_x, center_y) = if local_x < radius && local_y < radius {
            (radius, radius)
        } else if local_x >= self.width - radius && local_y < radius {
            (self.width - radius, radius)
        } else if local_x < radius && local_y >= self.height - radius {
            (radius, self.height - radius)
        } else if local_x >= self.width - radius && local_y >= self.height - radius {
            (self.width - radius, self.height - radius)
        } else {
            return true;
        };

        let dx = local_x - center_x;
        let dy = local_y - center_y;
        dx * dx + dy * dy <= radius * radius
    }
}

/// 多块形状的命中判定：任一形状包含光标即为命中。
///
/// 胶囊态最多两块（媒体岛 + dsh 岛），展开时是「卡片 + 留在胶囊行里的另一个岛」；
/// 空数组表示当前没有任何可见形状，调用方应按全穿透处理。
pub fn contains_any(shapes: &[ShapeRect], x: f64, y: f64) -> bool {
    shapes.iter().any(|shape| shape.contains(x, y))
}

/// 显示器工作区的物理像素描述。
#[derive(Clone, Copy, Debug, Default)]
pub struct WorkArea {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl WorkArea {
    pub fn right(&self) -> i32 {
        self.left + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.top + self.height
    }
}

/// 岛体锚点：水平中心 + 纵向基准边（上沿或下沿）。
#[derive(Clone, Copy, Debug)]
pub struct Anchor {
    pub center_x: f64,
    pub edge_y: f64,
    pub bottom_anchored: bool,
}

pub fn edge_margin(scale: f64) -> f64 {
    IslandSettings::EDGE_MARGIN * scale
}

pub fn top_inset(scale: f64) -> f64 {
    (IslandSettings::EDGE_MARGIN + IslandSettings::TOP_GAP) * scale
}

pub fn get_work_area(hwnd: interop::Hwnd) -> WorkArea {
    let mut monitor = unsafe { interop::MonitorFromWindow(hwnd, interop::MONITOR_DEFAULTTONEAREST) };
    if monitor == 0 {
        monitor = unsafe {
            interop::MonitorFromPoint(interop::POINT::default(), interop::MONITOR_DEFAULTTOPRIMARY)
        };
    }

    if monitor != 0 {
        let mut info = interop::MONITORINFO::default();
        if unsafe { interop::GetMonitorInfoW(monitor, &mut info) } != 0 {
            return WorkArea {
                left: info.rc_work.left,
                top: info.rc_work.top,
                width: info.rc_work.width(),
                height: info.rc_work.height(),
            };
        }
    }

    WorkArea {
        left: 0,
        top: 0,
        width: 1920,
        height: 1080,
    }
}

pub fn get_scale(hwnd: interop::Hwnd) -> f64 {
    let dpi = unsafe { interop::GetDpiForWindow(hwnd) };
    let dpi = if dpi == 0 { 96 } else { dpi };
    dpi as f64 / 96.0
}

/// 由配置解算锚点（预设 / 自定义比例）。
pub fn resolve_anchor(
    settings: &IslandSettings,
    work: WorkArea,
    scale: f64,
    metrics: &IslandMetrics,
) -> Anchor {
    let pill_width = metrics.pill_width * scale;
    let margin = edge_margin(scale);
    let top_inset = top_inset(scale);
    let default_bottom_anchored =
        settings.bottom_anchored || settings.placement == PlacementPreset::BottomCenter;

    match settings.placement {
        PlacementPreset::TopLeft => Anchor {
            center_x: work.left as f64 + margin + pill_width / 2.0,
            edge_y: work.top as f64 + top_inset,
            bottom_anchored: false,
        },
        PlacementPreset::TopRight => Anchor {
            center_x: work.right() as f64 - margin - pill_width / 2.0,
            edge_y: work.top as f64 + top_inset,
            bottom_anchored: false,
        },
        PlacementPreset::BottomCenter => Anchor {
            center_x: work.left as f64 + work.width as f64 / 2.0,
            edge_y: work.bottom() as f64 - margin,
            bottom_anchored: true,
        },
        _ => Anchor {
            center_x: work.left as f64 + settings.anchor_x_ratio * work.width as f64,
            edge_y: work.top as f64
                + top_inset
                + settings.anchor_y_ratio
                    * ((work.height as f64 - top_inset - margin).max(0.0)),
            bottom_anchored: default_bottom_anchored,
        },
    }
}

/// 把拖动后的胶囊锚点反解为持久化比例（浮点物理像素，不经过取整）。
pub fn capture_from_anchor(
    settings: &mut IslandSettings,
    work: WorkArea,
    scale: f64,
    pill_center_x: f64,
    pill_top: f64,
    pill_height: f64,
) {
    let margin = edge_margin(scale);
    let top_inset = top_inset(scale);

    settings.anchor_x_ratio = if work.width > 0 {
        ((pill_center_x - work.left as f64) / work.width as f64).clamp(0.0, 1.0)
    } else {
        0.5
    };

    // 岛体落在屏幕下半区时改为"以下沿为基准向上生长"，避免底部预设 / 贴底摆放时跳位
    let bottom_anchored = pill_top > work.top as f64 + work.height as f64 / 2.0;
    settings.bottom_anchored = bottom_anchored;

    let anchor_y = if bottom_anchored {
        pill_top + pill_height
    } else {
        pill_top
    };

    let span = work.height as f64 - top_inset - margin;
    settings.anchor_y_ratio = if span > 0.0 {
        ((anchor_y - work.top as f64 - top_inset) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };

    settings.placement = PlacementPreset::Custom;
    settings.monitor_left = work.left;
    settings.monitor_top = work.top;
    settings.monitor_width = work.width;
    settings.monitor_height = work.height;
}
