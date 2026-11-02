import type { IslandMetrics, IslandSizePreset } from "./types";

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
