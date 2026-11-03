import { withAlpha } from "../common/color";
import type { MediaSnapshot } from "../common/types";

/**
 * 媒体岛的胶囊层。
 *
 * - 只有一个岛时是原样的胶囊：左侧 18×18 缩略封面、右侧四根跳动音柱；
 *   没有封面时封面整体让位，频谱在胶囊里居中；
 * - 与 dsh 岛并存时收成正圆（`circle` 类）：直径等于胶囊高度，圆内嵌一枚圆角矩形
 *   专辑图；没有封面时圆内就是居中的频谱。
 *
 * 未展开时不显示任何取色光晕——光晕只属于展开的媒体卡片。
 */
export class MediaPillView {
  private readonly actor: HTMLElement;
  private readonly cover: HTMLElement;
  private readonly bars: HTMLElement;

  constructor(actor: HTMLElement) {
    this.actor = actor;
    this.cover = actor.querySelector<HTMLElement>("#pillCover")!;
    this.bars = actor.querySelector<HTMLElement>("#pillBars")!;
  }

  /** 切到正圆形态（两个岛同排时的媒体岛）。 */
  setCircle(circle: boolean): void {
    this.actor.classList.toggle("circle", circle);
  }

  setMedia(snapshot: MediaSnapshot | null): void {
    const artwork = snapshot?.ArtworkUrl ?? null;

    if (artwork) {
      this.cover.style.backgroundImage = `url("${artwork}")`;
      this.cover.hidden = false;
    } else {
      this.cover.style.backgroundImage = "";
      this.cover.hidden = true;
    }

    // 有封面：封面 + 右侧频谱 / 圆内只有封面；没封面：频谱居中（恢复改动前的样式）
    this.actor.classList.toggle("has-art", Boolean(artwork));
    this.bars.classList.toggle("center", !artwork);

    // 没有媒体时整簇音柱一起收掉；否则交还给 CSS
    this.bars.style.display = snapshot ? "" : "none";
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
