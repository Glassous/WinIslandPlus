import { withAlpha } from "../common/color";
import { theme } from "../common/theme";

/**
 * 展开卡片背后的取色流光：整张卡片垫一层非常淡的封面色调，再叠两团缓慢漂移的光斑，
 * 最后一束极淡的高光斜着扫过——目标是「被封面染色的玻璃」，而不是一条打光。
 *
 * - `.glow-tint`：斜向整卡色调，给玻璃定调（最暗也最广）；
 * - `.glow-bloom` / `.glow-bloom-alt`：主色 / 次色两团柔光，径向渐变自带羽化，
 *   呼吸以外还做缓慢位移（17s / 23s，错相），避免"两团光原地闪烁"的机械感；
 * - `.glow-sheen`：一道极淡的斜向高光，21s 一个来回，扫过时才有存在感；
 * - 四个层共用父层的 opacity 做淡入淡出（260 ms），熄灭时只暂停动画，
 *   重新点亮从中断相位继续，不复位。
 */
export class CardGlow {
  private readonly layer: HTMLElement;
  private readonly tint: HTMLElement;
  private readonly bloom: HTMLElement;
  private readonly bloomAlt: HTMLElement;
  private readonly sheen: HTMLElement;
  private readonly animated: HTMLElement[];
  private flowing = false;
  private visible = false;
  private primary: string | null = null;

  constructor(layer: HTMLElement) {
    this.layer = layer;

    this.tint = createLayer("glow-tint");
    this.bloomAlt = createLayer("glow-bloom glow-bloom-alt");
    this.bloom = createLayer("glow-bloom");
    this.sheen = createLayer("glow-sheen");

    // 顺序即层叠顺序：色调在最底，光斑其次，高光最上
    layer.appendChild(this.tint);
    layer.appendChild(this.bloom);
    layer.appendChild(this.bloomAlt);
    layer.appendChild(this.sheen);

    this.animated = [this.tint, this.bloom, this.bloomAlt, this.sheen];
  }

  /** 主色定调整卡与主光斑，次色做反向漂移的第二团光。峰值取自 theme.glowPeakOpacity。 */
  setPalette(primary: string | null, secondary: string | null): void {
    this.primary = primary;
    const peak = theme.glowPeakOpacity;

    this.tint.style.background = tintGradient(primary, secondary, peak);
    this.bloom.style.background = bloomGradient(primary, peak * 0.8);
    this.bloomAlt.style.background = bloomGradient(secondary ?? primary, peak * 0.66);
  }

  /**
   * 只有拿到封面主色时才点亮，避免"没有封面却有一团神秘光"。
   * 点亮期间光一直在动——暂停播放时也保留缓慢的明暗与漂移。
   */
  setActive(visible: boolean): void {
    this.visible = visible;
    this.layer.style.opacity = visible ? "1" : "0";

    if (visible) {
      this.startFlow();
      this.setPaused(false);
    } else {
      // 只暂停不拆动画：淡出过程中停在原地，重新点亮从中断相位继续
      this.setPaused(true);
    }
  }

  private startFlow(): void {
    if (this.flowing) return;
    this.flowing = true;

    this.tint.style.animation = "glow-tint-breathe 16s ease-in-out infinite";
    this.bloom.style.animation = "glow-drift 17s ease-in-out infinite";
    this.bloomAlt.style.animation = "glow-drift-alt 23s ease-in-out infinite";
    this.sheen.style.animation = "glow-sheen 21s cubic-bezier(0.5, 0, 0.5, 1) infinite";
    // 错相：各层周期不同且起始相位错开，明暗变化不会显得机械
    this.bloomAlt.style.animationDelay = "-7s";
    this.sheen.style.animationDelay = "-5s";
  }

  /** 暂停时停在当前相位，重新点亮时从中断处继续。 */
  private setPaused(paused: boolean): void {
    if (!this.flowing) return;
    const state = paused ? "paused" : "";
    for (const el of this.animated) el.style.animationPlayState = state;
  }

  get isVisible(): boolean {
    return this.visible;
  }

  get palette(): string | null {
    return this.primary;
  }
}

function createLayer(className: string): HTMLElement {
  const el = document.createElement("div");
  el.className = className;
  return el;
}

/** 整卡色调：左上略浓、往右下收成透明，给玻璃定一个斜向的冷暖基调。 */
function tintGradient(primary: string | null, secondary: string | null, peak: number): string {
  if (!primary && !secondary) return "transparent";
  const top = primary ?? secondary!;
  const bottom = secondary ?? primary!;
  return `linear-gradient(156deg, ${withAlpha(top, peak * 0.5)} 0%, ${withAlpha(
    bottom,
    peak * 0.3,
  )} 46%, ${withAlpha(top, 0)} 86%)`;
}

/** 一团柔光：中心最浓，向外逐段衰减到全透明（径向渐变自带羽化，不用 blur）。 */
function bloomGradient(color: string | null, peak: number): string {
  if (!color) return "transparent";
  return `radial-gradient(closest-side at 50% 50%, ${withAlpha(color, peak)} 0%, ${withAlpha(
    color,
    peak * 0.62,
  )} 32%, ${withAlpha(color, peak * 0.3)} 58%, ${withAlpha(color, peak * 0.1)} 80%, ${withAlpha(
    color,
    0,
  )} 100%)`;
}
