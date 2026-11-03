import { withAlpha } from "../common/color";
import { theme } from "../common/theme";

/**
 * 展开卡片背后的取色光——铺在卡片底部的一整条光带，向上照射并逐渐变暗。
 *
 * - 两层光带：主色（较高，76%）+ 次色叠加（较矮，48%），
 *   都是 `linear-gradient(to top)`：底沿最亮，向上渐暗直至全透明；
 * - 点亮期间两层光带以错开的周期（6.5s / 9s）做缓慢呼吸
 *   （scaleY + 不透明度），让光有生命感但不抢注意力；
 * - 图层整体随「是否有封面 + 是否展开」淡入淡出（260 ms），
 *   熄灭时动画只暂停、不拆掉，重新点亮从中断的相位继续。
 */
export class CardGlow {
  private readonly layer: HTMLElement;
  private readonly beam: HTMLElement;
  private readonly beamAlt: HTMLElement;
  private flowing = false;
  private visible = false;
  private primary: string | null = null;

  constructor(layer: HTMLElement) {
    this.layer = layer;

    this.beamAlt = document.createElement("div");
    this.beamAlt.className = "glow-beam-alt";

    this.beam = document.createElement("div");
    this.beam.className = "glow-beam";

    layer.appendChild(this.beamAlt);
    layer.appendChild(this.beam);
  }

  /** 主色作主光带、次色叠加，峰值取自 theme.glowPeakOpacity。 */
  setPalette(primary: string | null, secondary: string | null): void {
    this.primary = primary;
    const peak = theme.glowPeakOpacity;
    this.beam.style.background = beamGradient(primary, peak);
    this.beamAlt.style.background = beamGradient(secondary ?? primary, peak * 0.55);
  }

  /**
   * 只有拿到封面主色时才点亮，避免"没有封面却有一团神秘光"。
   * 点亮期间光一直在呼吸——暂停播放时也保留缓慢的明暗变化。
   */
  setActive(visible: boolean): void {
    this.visible = visible;
    this.layer.style.opacity = visible ? "1" : "0";

    if (visible) {
      this.startFlow();
      this.setPaused(false);
    } else {
      // 只暂停不拆动画：淡出过程中光带停在原地，重新点亮从中断相位继续
      this.setPaused(true);
    }
  }

  private startFlow(): void {
    if (this.flowing) return;
    this.flowing = true;

    this.beam.style.animation = "glow-beam-breath 6.5s ease-in-out infinite";
    this.beamAlt.style.animation = "glow-beam-breath 9s ease-in-out infinite";
    // 错相：两层呼吸不同步，明暗变化不会显得机械
    this.beamAlt.style.animationDelay = "-3s";
  }

  /** 暂停时停在当前相位，重新点亮时从中断处继续。 */
  private setPaused(paused: boolean): void {
    if (!this.flowing) return;
    const state = paused ? "paused" : "";
    this.beam.style.animationPlayState = state;
    this.beamAlt.style.animationPlayState = state;
  }

  get isVisible(): boolean {
    return this.visible;
  }

  get palette(): string | null {
    return this.primary;
  }
}

/** 向上照射的光带：底沿最亮，向上逐渐变暗直至全透明。 */
function beamGradient(color: string | null, peak: number): string {
  if (!color) return "transparent";
  return `linear-gradient(to top, ${withAlpha(color, peak)} 0%, ${withAlpha(
    color,
    peak * 0.5,
  )} 34%, ${withAlpha(color, peak * 0.16)} 66%, ${withAlpha(color, 0)} 100%)`;
}
