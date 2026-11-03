import type { IslandMetrics, IslandSizePreset } from "./types";

/**
 * 双岛布局相关尺寸（DIP），与 `src-tauri/src/metrics.rs` 逐值一致。
 *
 * - `DSH_PILL_MIN/MAX`：dsh 胶囊的宽度区间（按内容自适应后夹取）；
 * - `DSH_ONLY_CARD_HEIGHT`：dsh 独立展开卡片的高度；
 * - `ROW_GAP`：胶囊行与卡片行之间的间距（展开的卡片长在「下一行」）。
 */
export const DSH_PILL_MIN = 96;
export const DSH_PILL_MAX = 132;
export const DSH_ONLY_CARD_HEIGHT = 132;
/** 胶囊左右内边距合计（与 `.actor > .pill-layer { padding: 0 9px }` 对应）。 */
export const PILL_PADDING = 18;
/** 同一行内两个岛之间的间距。 */
export const PILL_GAP = 8;
/** 胶囊行与卡片行之间的间距。 */
export const ROW_GAP = 8;

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** dsh 胶囊里允许的内容最大自然宽度（超过就折叠次要字段）。 */
export function dshInnerMaxWidth(): number {
  return DSH_PILL_MAX - PILL_PADDING;
}

/**
 * dsh 胶囊宽度：内容自然宽度 + 左右内边距，夹在 `DSH_PILL_MIN`~`DSH_PILL_MAX`。
 * `dshInnerWidth` 为 0 表示没有 dsh 内容（返回 0，调用方按"不显示 dsh 岛"处理）。
 */
export function dshPillWidth(dshInnerWidth: number): number {
  if (dshInnerWidth <= 0) return 0;
  return clamp(dshInnerWidth + PILL_PADDING, DSH_PILL_MIN, DSH_PILL_MAX);
}

/** 胶囊行与卡片行在窗口内的纵向起点（DIP，贴底时整行镜像）。 */
export function rowOffsets(
  m: IslandMetrics,
  bottomAnchored: boolean,
): { pillY: number; cardY: number } {
  const windowHeight = m.PillHeight + ROW_GAP + m.CardHeight;
  if (bottomAnchored) {
    // 卡片行在上、胶囊行贴窗口下沿
    return { pillY: windowHeight - m.PillHeight, cardY: 0 };
  }
  return { pillY: 0, cardY: m.PillHeight + ROW_GAP };
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
