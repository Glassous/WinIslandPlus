import type { IslandMetrics, IslandSizePreset } from "./types";

/**
 * DeepSeek Harness 相关尺寸（DIP），与 `src-tauri/src/metrics.rs` 逐值一致。
 *
 * - `DSH_ROW_HEIGHT`：展开卡片底部的 dsh 信息块高度（两行：阶段行 + 数据行）；
 *   窗口高度始终按「基础卡片 + 这一块」预留，dsh 出现时只让形状在内部生长。
 * - `DSH_PILL_MIN/MAX`：胶囊里 dsh 区的宽度区间（自适应内容后夹取）。
 * - `DSH_ONLY_CARD_HEIGHT`：没有媒体、只有 dsh 时的紧凑卡片高度。
 */
export const DSH_ROW_HEIGHT = 66;
export const DSH_PILL_MIN = 96;
export const DSH_PILL_MAX = 132;
export const DSH_ONLY_CARD_HEIGHT = 132;
/** 胶囊左右内边距合计（与 `.pill { padding: 0 9px }` 对应）。 */
export const PILL_PADDING = 18;
/** 胶囊里媒体簇与 dsh 簇之间的间距（不再有分隔线）。 */
export const PILL_GAP = 10;

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** dsh 内容在胶囊里允许的最大自然宽度（超过就折叠次要字段）。 */
export function dshInnerMaxWidth(): number {
  return DSH_PILL_MAX - PILL_PADDING;
}

/**
 * 胶囊宽度：有媒体时在基础宽度上追加 dsh 区，没有媒体时 dsh 区自成胶囊。
 * `dshInnerWidth` 为 0 表示没有 dsh 内容。
 */
export function pillWidthFor(
  m: IslandMetrics,
  hasMedia: boolean,
  dshInnerWidth: number,
): number {
  if (dshInnerWidth <= 0) return m.PillWidth;
  if (!hasMedia) {
    return clamp(dshInnerWidth + PILL_PADDING, DSH_PILL_MIN, DSH_PILL_MAX);
  }
  return m.PillWidth + clamp(dshInnerWidth + PILL_GAP, DSH_PILL_MIN, DSH_PILL_MAX);
}

/** 展开卡片高度：dsh 行出现时追加一行，无媒体时回落到紧凑高度。 */
export function cardHeightFor(
  m: IslandMetrics,
  hasMedia: boolean,
  hasDsh: boolean,
): number {
  if (!hasMedia) return hasDsh ? DSH_ONLY_CARD_HEIGHT : m.CardHeight;
  return m.CardHeight + (hasDsh ? DSH_ROW_HEIGHT : 0);
}

/** 窗口高度：始终预留 dsh 行，避免运行时改窗口尺寸产生空白帧。 */
export function windowHeight(m: IslandMetrics, dshEnabled: boolean): number {
  return m.CardHeight + (dshEnabled ? DSH_ROW_HEIGHT : 0);
}

/** 胶囊固定为 2:1（宽度是高度的两倍），展开态约 1.8:1。 */
export function metricsFor(
  preset: IslandSizePreset,
  customWidth: number,
): IslandMetrics {
  switch (preset) {
    case "Small":
      return {
        PillWidth: 64,
        PillHeight: 32,
        CardWidth: 300,
        CardHeight: 188,
        PillCorner: 16,
        CardCorner: 26,
      };
    case "Large":
      return {
        PillWidth: 76,
        PillHeight: 38,
        CardWidth: 380,
        CardHeight: 196,
        PillCorner: 19,
        CardCorner: 30,
      };
    case "Custom":
      return fromCustom(customWidth);
    default:
      return {
        PillWidth: 68,
        PillHeight: 34,
        CardWidth: 336,
        CardHeight: 188,
        PillCorner: 17,
        CardCorner: 28,
      };
  }
}

function fromCustom(width: number): IslandMetrics {
  const w = Number.isFinite(width) ? Math.min(Math.max(width, 60), 96) : 68;
  const pillHeight = w / 2;
  const cardWidth = Math.min(Math.max(w * 4.9, 300), 420);
  const cardHeight = Math.min(Math.max(cardWidth * 0.56, 188), 236);
  return {
    PillWidth: w,
    PillHeight: pillHeight,
    CardWidth: cardWidth,
    CardHeight: cardHeight,
    PillCorner: pillHeight / 2,
    CardCorner: Math.min(Math.max(cardHeight * 0.15, 22), 32),
  };
}

export function islandWidth(m: IslandMetrics, expanded: boolean): number {
  return expanded ? m.CardWidth : m.PillWidth;
}

export function islandHeight(m: IslandMetrics, expanded: boolean): number {
  return expanded ? m.CardHeight : m.PillHeight;
}

export function islandCorner(m: IslandMetrics, expanded: boolean): number {
  return expanded ? m.CardCorner : m.PillCorner;
}
