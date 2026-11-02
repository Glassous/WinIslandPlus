import { theme } from "../common/theme";
import { radialGlow } from "./pill";

interface Blob {
  el: HTMLElement;
  core: HTMLElement;
  size: number;
}

/**
 * 展开卡片背后的三团取色光斑——铺满整张卡片（而不是只在封面周围一小圈）。
 *
 * - 光斑尺寸约为卡片宽度的 1.5 / 1.37 / 1.25 倍，锚点按百分比分布，
 *   与卡片尺寸解耦，任何尺寸档位都能盖满整张卡；
 * - 播放中缓慢漂移（17 / 23 / 14 s）+ 呼吸（0.92 ↔ 1.08），暂停时完全静止归位；
 * - 整个图层随「是否有封面 + 是否展开」淡入淡出（260 ms），收起态不占用合成器。
 */
export class CardGlow {
  /** 卡片宽高变化不需要重排：锚点用百分比表达。 */
  private static readonly ANCHORS: ReadonlyArray<readonly [number, number]> = [
    [0.32, 0.38],
    [0.82, 0.24],
    [0.66, 0.92],
  ];

  private readonly layer: HTMLElement;
  private readonly blobs: Blob[] = [];
  private readonly strengths = [1.0, 0.8, 0.62];
  private lastWidth = 0;
  private lastHeight = 0;
  private drifting = false;
  private visible = false;
  private primary: string | null = null;

  constructor(layer: HTMLElement) {
    this.layer = layer;

    // 尺寸按「盖满整张卡」定：卡片 336×188 时三团分别约 520 / 460 / 420 DIP
    const sizes = [520, 460, 420];
    for (let index = 0; index < sizes.length; index += 1) {
      const el = document.getElementById(`glow${index + 1}`)!;
      el.style.width = `${sizes[index]}px`;
      el.style.height = `${sizes[index]}px`;

      const core = document.createElement("div");
      core.className = "glow-blob-core";
      el.appendChild(core);

      this.blobs.push({ el, core, size: sizes[index] });
    }

    this.layout();
  }

  /**
   * 跟随卡片实时尺寸：卡片因 dsh 行变高时光斑仍然铺满整张卡。
   * 尺寸没变化时不做任何写入。
   */
  setSize(width: number, height: number): void {
    if (
      Math.abs(width - this.lastWidth) < 0.5 &&
      Math.abs(height - this.lastHeight) < 0.5
    ) {
      return;
    }
    this.lastWidth = width;
    this.lastHeight = height;
    this.layer.style.width = `${width}px`;
    this.layer.style.height = `${height}px`;
  }

  /** 按百分比把三团光斑摆到卡片上（transform 留给漂移动画）。 */
  layout(): void {
    this.blobs.forEach((blob, index) => {
      const anchor = CardGlow.ANCHORS[index];
      blob.el.style.left = `${anchor[0] * 100}%`;
      blob.el.style.top = `${anchor[1] * 100}%`;
      blob.el.style.marginLeft = `${-blob.size / 2}px`;
      blob.el.style.marginTop = `${-blob.size / 2}px`;
    });
  }

  /** 主 / 次色 + 每团各自的强度（1.0 / 0.8 / 0.62）。 */
  setPalette(primary: string | null, secondary: string | null): void {
    this.primary = primary;
    const alt = secondary ?? primary;
    const colors = [primary, alt, primary];

    this.blobs.forEach((blob, index) => {
      const color = colors[index];
      if (!color) {
        blob.core.style.background = "transparent";
        return;
      }

      const peak = theme.glowPeakOpacity * this.strengths[index];
      blob.core.style.background = radialGlow(color, peak, 0.34);
    });
  }

  /** 只有拿到封面主色时才点亮光晕，避免"没有封面却有一团神秘光"。 */
  setActive(visible: boolean, playing: boolean): void {
    this.visible = visible;
    this.layer.style.opacity = visible ? "1" : "0";

    if (visible && playing) {
      this.startDrift();
    } else {
      this.stopDrift();
    }
  }

  private startDrift(): void {
    if (this.drifting) return;
    this.drifting = true;

    this.blobs.forEach((blob, index) => {
      const seconds = theme.glowDriftSeconds[index % theme.glowDriftSeconds.length];
      blob.el.style.animation = `glow-drift-${index} ${seconds}s linear infinite`;
      blob.core.style.animation = `glow-breath ${(seconds * 0.72).toFixed(2)}s linear infinite`;
    });
  }

  private stopDrift(): void {
    if (!this.drifting) return;
    this.drifting = false;

    this.blobs.forEach((blob) => {
      blob.el.style.animation = "";
      blob.core.style.animation = "";
    });
  }

  get isVisible(): boolean {
    return this.visible;
  }

  get palette(): string | null {
    return this.primary;
  }
}
