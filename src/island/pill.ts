import { withAlpha } from "../common/color";
import type { MediaSnapshot } from "../common/types";

/**
 * 胶囊态：左侧 18×18 缩略封面，右侧四根跳动音柱。
 * 没有封面时波形居中；没有媒体时胶囊完全空白（常驻模式的静止空胶囊）。
 * 未展开时不显示任何取色光晕——光晕只属于展开卡片。
 */
export class PillView {
  private readonly cover: HTMLElement;
  private readonly bars: HTMLElement;

  constructor(root: HTMLElement) {
    this.cover = root.querySelector<HTMLElement>("#pillCover")!;
    this.bars = root.querySelector<HTMLElement>("#pillBars")!;
  }

  setMedia(snapshot: MediaSnapshot | null): void {
    const artwork = snapshot?.ArtworkUrl ?? null;

    if (artwork) {
      this.cover.style.backgroundImage = `url("${artwork}")`;
      this.cover.hidden = false;
      this.bars.classList.remove("center");
    } else {
      this.cover.style.backgroundImage = "";
      this.cover.hidden = true;
      this.bars.classList.add("center");
    }

    // 没有媒体时波形整体隐藏，只剩一个静止的空胶囊
    this.bars.style.display = snapshot ? "flex" : "none";

    this.setPlaying(snapshot?.IsPlaying ?? false);
  }

  private setPlaying(playing: boolean): void {
    this.bars.classList.toggle("playing", playing);
    this.bars.classList.toggle("frozen", !playing);
  }
}

/**
 * 径向渐变：中心峰值 → 55% 处为峰值的 `midRatio` → 边缘全透明。
 * 三档渐变 stop（胶囊 midRatio = 0.32，卡片光斑 = 0.34）。
 */
export function radialGlow(color: string, peakOpacity: number, midRatio = 0.32): string {
  const middle = peakOpacity * midRatio;
  return `radial-gradient(circle closest-side, ${withAlpha(color, peakOpacity)} 0%, ${withAlpha(
    color,
    middle,
  )} 55%, ${withAlpha(color, 0)} 100%)`;
}
