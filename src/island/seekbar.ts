import { api } from "../common/ipc";

export interface SeekBarCallbacks {
  onScrubStarted?: () => void;
  onScrubMoved?: (fraction: number) => void;
  onSeekRequested?: (fraction: number) => void;
  onUserActed?: () => void;
}

/**
 * 自绘进度条。
 *
 * 不使用原生 `<input type=range>`（依赖焦点与激活状态，在悬浮窗里不可靠）；
 * 拖动改为「按下后按光标位置轮询」——位置与左键状态都从 Win32 直接取，
 * 因此即使 WebView2 不派发 pointermove 也能稳定工作。
 */
export class SeekBar {
  private readonly root: HTMLElement;
  private readonly fill: HTMLElement;
  private readonly thumb: HTMLElement;
  private readonly callbacks: SeekBarCallbacks;

  private fraction = 0;
  private interactable = true;
  private scrubbing = false;
  private pressFraction = 0;
  private pressX = 0;
  private timer = 0;

  constructor(root: HTMLElement, callbacks: SeekBarCallbacks = {}) {
    this.root = root;
    this.fill = root.querySelector<HTMLElement>("#seekFill")!;
    this.thumb = root.querySelector<HTMLElement>("#seekThumb")!;
    this.callbacks = callbacks;

    root.addEventListener("pointerdown", this.onPointerDown);
    root.addEventListener("pointermove", this.onPointerMove);
    root.addEventListener("pointerup", this.onPointerUp);
    root.addEventListener("pointercancel", this.onPointerUp);
    root.addEventListener("pointerenter", () => root.classList.add("hover"));
    root.addEventListener("pointerleave", () => {
      if (!this.scrubbing) root.classList.remove("hover");
    });
  }

  setInteractable(value: boolean): void {
    this.interactable = value;
    this.root.classList.toggle("disabled", !value);
  }

  /** 外部推进进度（拖动中忽略，避免抖动）。 */
  setProgress(fraction: number): void {
    if (this.scrubbing) return;
    this.fraction = clamp01(fraction);
    this.render();
  }

  private onPointerDown = (event: PointerEvent): void => {
    if (!this.interactable || event.button !== 0) return;

    // 阻止冒泡：拖动进度条不应该让整个岛收起
    event.stopPropagation();

    this.scrubbing = true;
    this.root.classList.add("hover");
    this.pressFraction = this.fractionFromClientX(event.clientX);
    this.pressX = event.clientX;
    this.fraction = this.pressFraction;
    this.render();

    this.callbacks.onScrubStarted?.();
    this.callbacks.onScrubMoved?.(this.fraction);
    this.startPolling();
  };

  private onPointerMove = (event: PointerEvent): void => {
    if (this.scrubbing) {
      event.stopPropagation();
    }
  };

  private onPointerUp = (event: PointerEvent): void => {
    if (!this.scrubbing) return;
    event.stopPropagation();
    this.complete();
  };

  private startPolling(): void {
    this.stopPolling();
    this.timer = window.setInterval(() => void this.pollCursor(), 33);
  }

  private stopPolling(): void {
    if (this.timer) {
      window.clearInterval(this.timer);
      this.timer = 0;
    }
  }

  private async pollCursor(): Promise<void> {
    if (!this.scrubbing) {
      this.stopPolling();
      return;
    }

    let probe: { X: number; Y: number; Down: boolean } | null = null;
    try {
      probe = await api.cursorProbe();
    } catch {
      probe = null;
    }

    if (!probe) return;

    if (!probe.Down) {
      this.complete();
      return;
    }

    const width = this.root.getBoundingClientRect().width;
    if (width <= 1) return;

    const next = clamp01(this.pressFraction + (probe.X - this.pressX) / width);
    if (Math.abs(next - this.fraction) < 0.0005) return;

    this.fraction = next;
    this.render();
    this.callbacks.onScrubMoved?.(this.fraction);
  }

  private complete(): void {
    this.scrubbing = false;
    this.stopPolling();
    this.root.classList.remove("hover");
    this.callbacks.onSeekRequested?.(this.fraction);
    this.callbacks.onUserActed?.();
  }

  private fractionFromClientX(clientX: number): number {
    const rect = this.root.getBoundingClientRect();
    if (rect.width <= 0) return 0;
    return clamp01((clientX - rect.left) / rect.width);
  }

  private render(): void {
    const width = this.root.getBoundingClientRect().width || 1;
    this.fill.style.width = `${Math.max(0, this.fraction * width)}px`;

    const thumbWidth = 10;
    const x = this.fraction * width - thumbWidth / 2;
    this.thumb.style.left = `${Math.min(Math.max(x, -thumbWidth / 2), width - thumbWidth / 2)}px`;
  }
}

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}
