//! 从封面缩略图中提取主色 / 次色，用于专辑图背后的取色光晕。
//!
//! 纯函数、不依赖 UI 线程，输入是 BGRA8 采样缓冲（32×32 量级即可）。
//! 算法：12 个色相桶、按 饱和度×明度 加权选主色，
//! 次色要求与主色色相至少相差 60°，最后提饱和 + 限亮。

const HUE_BUCKETS: usize = 12;
const MIN_SATURATION: f64 = 0.2;
const MIN_VALUE: f64 = 0.12;
const MAX_VALUE: f64 = 0.94;

/// 没有封面 / 取色失败时的中性冷色（低调，不抢内容）。
pub const FALLBACK_PRIMARY: (u8, u8, u8) = (0x46, 0x5A, 0x78);
pub const FALLBACK_SECONDARY: (u8, u8, u8) = (0x2C, 0x3A, 0x52);

#[derive(Clone, Copy, Debug)]
pub struct AccentPair {
    pub primary: (u8, u8, u8),
    pub secondary: (u8, u8, u8),
}

pub fn extract(bgra: &[u8], width: usize, height: usize, stride: usize) -> AccentPair {
    if bgra.is_empty() || width == 0 || height == 0 {
        return fallback();
    }

    let mut weights = [0.0f64; HUE_BUCKETS];
    let mut sum_r = [0.0f64; HUE_BUCKETS];
    let mut sum_g = [0.0f64; HUE_BUCKETS];
    let mut sum_b = [0.0f64; HUE_BUCKETS];
    let mut hues = [0.0f64; HUE_BUCKETS];

    for y in 0..height {
        let row_start = y * stride;
        if row_start + width * 4 > bgra.len() {
            break;
        }

        for x in 0..width {
            let offset = row_start + x * 4;
            let b = bgra[offset];
            let g = bgra[offset + 1];
            let r = bgra[offset + 2];

            let (hue, saturation, value) = to_hsv(r, g, b);
            if saturation < MIN_SATURATION || value < MIN_VALUE || value > MAX_VALUE {
                continue;
            }

            let index = ((hue / 360.0 * HUE_BUCKETS as f64) as usize).min(HUE_BUCKETS - 1);
            let weight = saturation * value;
            weights[index] += weight;
            sum_r[index] += r as f64 * weight;
            sum_g[index] += g as f64 * weight;
            sum_b[index] += b as f64 * weight;
            hues[index] = index as f64 * 360.0 / HUE_BUCKETS as f64 + 180.0 / HUE_BUCKETS as f64;
        }
    }

    let mut primary_index: Option<usize> = None;
    for i in 0..HUE_BUCKETS {
        if weights[i] <= 0.0 {
            continue;
        }
        if primary_index.is_none_or(|current| weights[i] > weights[current]) {
            primary_index = Some(i);
        }
    }

    let Some(primary_index) = primary_index else {
        return fallback();
    };

    let mut secondary_index: Option<usize> = None;
    for i in 0..HUE_BUCKETS {
        if weights[i] <= 0.0 || i == primary_index {
            continue;
        }

        let mut distance = (hues[i] - hues[primary_index]).abs();
        distance = distance.min(360.0 - distance);
        if distance < 60.0 {
            continue;
        }

        if secondary_index.is_none_or(|current| weights[i] > weights[current]) {
            secondary_index = Some(i);
        }
    }

    let primary = normalize(average(&sum_r, &sum_g, &sum_b, &weights, primary_index));
    let secondary = match secondary_index {
        Some(index) => normalize(average(&sum_r, &sum_g, &sum_b, &weights, index)),
        None => shift(primary),
    };

    AccentPair {
        primary,
        secondary,
    }
}

fn average(
    sum_r: &[f64; HUE_BUCKETS],
    sum_g: &[f64; HUE_BUCKETS],
    sum_b: &[f64; HUE_BUCKETS],
    weights: &[f64; HUE_BUCKETS],
    index: usize,
) -> (u8, u8, u8) {
    let weight = weights[index].max(1e-6);
    (
        (sum_r[index] / weight).clamp(0.0, 255.0) as u8,
        (sum_g[index] / weight).clamp(0.0, 255.0) as u8,
        (sum_b[index] / weight).clamp(0.0, 255.0) as u8,
    )
}

/// 提饱和 + 限亮，保证在深色底上通透但不刺眼。
fn normalize(color: (u8, u8, u8)) -> (u8, u8, u8) {
    let (r, g, b) = color;
    let (hue, saturation, value) = to_hsv(r, g, b);
    let saturation = (saturation * 1.18).clamp(0.0, 1.0);
    let value = value.max(0.42).clamp(0.35, 0.85);
    from_hsv(hue, saturation, value)
}

/// 次色兜底：把主色转一个角度，避免两团光斑完全同色。
fn shift(color: (u8, u8, u8)) -> (u8, u8, u8) {
    let (r, g, b) = color;
    let (hue, saturation, value) = to_hsv(r, g, b);
    let shifted = (hue + 54.0) % 360.0;
    from_hsv(
        shifted,
        (saturation * 0.9).clamp(0.0, 1.0),
        (value * 0.85).clamp(0.3, 0.8),
    )
}

fn fallback() -> AccentPair {
    AccentPair {
        primary: FALLBACK_PRIMARY,
        secondary: FALLBACK_SECONDARY,
    }
}

fn to_hsv(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let rf = r as f64 / 255.0;
    let gf = g as f64 / 255.0;
    let bf = b as f64 / 255.0;

    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;

    let value = max;
    let saturation = if max <= 0.0 { 0.0 } else { delta / max };

    if delta <= 1e-6 {
        return (0.0, saturation, value);
    }

    let mut hue = if max == rf {
        60.0 * (((gf - bf) / delta) % 6.0)
    } else if max == gf {
        60.0 * (((bf - rf) / delta) + 2.0)
    } else {
        60.0 * (((rf - gf) / delta) + 4.0)
    };

    if hue < 0.0 {
        hue += 360.0;
    }

    (hue, saturation, value)
}

fn from_hsv(hue: f64, saturation: f64, value: f64) -> (u8, u8, u8) {
    let c = value * saturation;
    let h = hue / 60.0;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = value - c;

    let (r, g, b) = match h {
        h if h < 1.0 => (c, x, 0.0),
        h if h < 2.0 => (x, c, 0.0),
        h if h < 3.0 => (0.0, c, x),
        h if h < 4.0 => (0.0, x, c),
        h if h < 5.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}
