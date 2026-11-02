import { theme } from "../common/theme";
import { radialGlow } from "./pill";

/** 一个光源：锚点（百分比）、直径、相对主题峰值的强度。 */
interface LightSource {
  anchor: readonly [number, number];
  size: number;
  strength: number;
}

interface Blob {
  el: HTMLElement;
  flow: HTMLElement;
  core: HTMLElement;
  size: number;
}

/**
 * 展开卡片背后的取色流光——多个小光源沿着卡片边缘缓慢游走。
 *
 * - 光源数量、尺寸、锚点集中在 `SOURCES`：目前 5 个，直径 150~260 DIP，
 *   全部小于卡片，卡片中部与文字区保持干净的深色底；
 * - 动态流动：每个光源拆成两层，横向 / 纵向各自用一组正弦往返关键帧，
 *   两轴周期不同 → 合成一条不断游走的闭合光路，看不到折返硬拐点；
 *   半周期 5~9 s、振幅 30~62 DIP，肉眼能明确看到光在移动；
 * - 呼吸（0.9 ↔ 1.1）与漂移错开周期，让明暗也在变；
 * - 只要点亮（有封面 + 展开）就在动，与播放 / 暂停无关；
 * - 图层整体随「是否有封面 + 是否展开」淡入淡出（260 ms），
 *   熄灭时动画只暂停、不拆掉，重新点亮从中断的相位继续。
 */
export class CardGlow {
  /** 5 个光源分布在四角与两侧，中间留白。 */
  private static readonly SOURCES: readonly LightSource[] = [
    { anchor: [0.12, 0.2], size: 260, strength: 1.0 },
    { anchor: [0.95, 0.5], size: 220, strength: 0.8 },
    { anchor: [0.3, 1.0], size: 200, strength: 0.6 },
    { anchor: [0.72, 0.04], size: 170, strength: 0.5 },
    { anchor: [0.74, 0.94], size: 150, strength: 0.4 },
  ];

  private readonly layer: HTMLElement;
  private readonly blobs: Blob[] = [];
  private lastWidth = 0;
  private lastHeight = 0;
  private flowing = false;
  private visible = false;
  private primary: string | null = null;

  constructor(layer: HTMLElement) {
    this.layer = layer;

    CardGlow.SOURCES.forEach((source, index) => {
      const el = document.createElement("div");
      el.className = "glow-blob";
      el.style.width = `${source.size}px`;
      el.style.height = `${source.size}px`;

      const flow = document.createElement("div");
      flow.className = "glow-blob-flow";

      const core = document.createElement("div");
      core.className = "glow-blob-core";
      core.style.setProperty("--glow-scale-min", String(theme.glowScaleMin));
      core.style.setProperty("--glow-scale-max", String(theme.glowScaleMax));

      flow.appendChild(core);
      el.appendChild(flow);
      layer.appendChild(el);

      this.blobs.push({ el, flow, core, size: source.size });
      this.place(index);
    });
  }

  /**
   * 跟随卡片实时尺寸：卡片因 dsh 行变高时光源仍然贴着卡片边缘。
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

  /** 按百分比把一个光源摆到卡片上（transform 留给流光动画）。 */
  private place(index: number): void {
    const blob = this.blobs[index];
    const anchor = CardGlow.SOURCES[index].anchor;
    blob.el.style.left = `${anchor[0] * 100}%`;
    blob.el.style.top = `${anchor[1] * 100}%`;
    blob.el.style.marginLeft = `${-blob.size / 2}px`;
    blob.el.style.marginTop = `${-blob.size / 2}px`;
  }

  /** 主 / 次色交替分配，每个光源按自己的强度取峰值透明度。 */
  setPalette(primary: string | null, secondary: string | null): void {
    this.primary = primary;
    const alt = secondary ?? primary;

    this.blobs.forEach((blob, index) => {
      const color = index % 2 === 0 ? primary : alt;
      if (!color) {
        blob.core.style.background = "transparent";
        return;
      }

      const peak = theme.glowPeakOpacity * CardGlow.SOURCES[index].strength;
      blob.core.style.background = radialGlow(color, peak, 0.34);
    });
  }

  /**
   * 只有拿到封面主色时才点亮流转，避免"没有封面却有一团神秘光"。
   * 点亮期间光一直在流动——暂停播放时也保留缓慢的呼吸感。
   */
  setActive(visible: boolean): void {
    this.visible = visible;
    this.layer.style.opacity = visible ? "1" : "0";

    if (visible) {
      this.startFlow();
      this.setPaused(false);
    } else {
      // 只暂停不拆动画：淡出过程中光源停在原地，不会突然跳回锚点
      this.setPaused(true);
    }
  }

  private startFlow(): void {
    if (this.flowing) return;
    this.flowing = true;

    this.blobs.forEach((blob, index) => {
      const cycle = theme.glowFlowSeconds[index % theme.glowFlowSeconds.length];
      // 关键帧按 4 组循环：振幅不同，光路不会给人"机械重复"的感觉
      const variant = index % 4;
      blob.el.style.animation = `glow-flow-x-${variant} ${cycle.x}s ease-in-out infinite`;
      blob.flow.style.animation = `glow-flow-y-${variant} ${cycle.y}s ease-in-out infinite`;

      // 明暗呼吸与位移错相：同一光源不会"到点就闪一下"
      const breath = ((cycle.x + cycle.y) / 2).toFixed(1);
      blob.core.style.animation = `glow-breath ${breath}s ease-in-out infinite`;
      blob.core.style.animationDelay = `${(-index * 1.7).toFixed(2)}s`;
    });
  }

  /** 暂停时停在当前相位，重新点亮时从中断处继续。 */
  private setPaused(paused: boolean): void {
    if (!this.flowing) return;
    const state = paused ? "paused" : "";

    this.blobs.forEach((blob) => {
      blob.el.style.animationPlayState = state;
      blob.flow.style.animationPlayState = state;
      blob.core.style.animationPlayState = state;
    });
  }

  get isVisible(): boolean {
    return this.visible;
  }

  get palette(): string | null {
    return this.primary;
  }
}
