//! 胶囊态 / 展开态的尺寸几何（DIP）。
//!
//! 与前端 `src/common/metrics.ts` 保持逐值一致；后端用它决定窗口尺寸与落位。

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IslandSizePreset {
    Small,
    #[default]
    Medium,
    Large,
    Custom,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct IslandMetrics {
    pub pill_width: f64,
    pub pill_height: f64,
    pub card_width: f64,
    pub card_height: f64,
    pub pill_corner: f64,
    pub card_corner: f64,
}

impl IslandMetrics {
    /// 胶囊固定为 2:1（宽度是高度的两倍）；展开态约 1.8:1。
    pub fn for_preset(preset: IslandSizePreset, custom_width: f64) -> Self {
        match preset {
            IslandSizePreset::Small => Self {
                pill_width: 64.0,
                pill_height: 32.0,
                card_width: 300.0,
                card_height: 188.0,
                pill_corner: 16.0,
                card_corner: 26.0,
            },
            IslandSizePreset::Large => Self {
                pill_width: 76.0,
                pill_height: 38.0,
                card_width: 380.0,
                card_height: 196.0,
                pill_corner: 19.0,
                card_corner: 30.0,
            },
            IslandSizePreset::Custom => Self::from_custom(custom_width),
            IslandSizePreset::Medium => Self {
                pill_width: 68.0,
                pill_height: 34.0,
                card_width: 336.0,
                card_height: 188.0,
                pill_corner: 17.0,
                card_corner: 28.0,
            },
        }
    }

    fn from_custom(width: f64) -> Self {
        let width = if width.is_finite() {
            width.clamp(60.0, 96.0)
        } else {
            68.0
        };

        let pill_height = width / 2.0;
        let card_width = (width * 4.9).clamp(300.0, 420.0);
        let card_height = (card_width * 0.56).clamp(188.0, 236.0);

        Self {
            pill_width: width,
            pill_height,
            card_width,
            card_height,
            pill_corner: pill_height / 2.0,
            card_corner: (card_height * 0.15).clamp(22.0, 32.0),
        }
    }

}
