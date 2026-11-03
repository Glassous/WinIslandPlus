import { api } from "../common/ipc";

export interface SeekBarCallbacks {
  onScrubStarted?: () => void;
  onScrubMoved?: (fraction: number) => void;
  onSeekRequested?: (fraction: number) => void;
  onUserActed?: () => void;
}

/** 光标轮询周期：WebView2 在未激活窗口上可能丢 pointermove，靠它兜底。 */
const POLL_MS = 33;
/** 松手后等待后端真实快照归位的窗口期：这段时间里旧进度不能把拖拽结果拉回去。 */
const SEEK_LOCK_MS = 700;
/** 外部进度与目标位置的容差：进入容差即认为后端已跟上，提前解锁。 */
const SEEK_LOCK_TOLERANCE = 0.02;
/** 按下后的宽限期：这段时间内轮询不把「左键未按下」当成松手。 */
const RELEASE_GRACE_MS = 120;
/** 滑块宽度（与 CSS 的 .seek-thumb 一致），用于把滑块完整收在轨道内。 */
const THUMB_WIDTH = 10;

/**
 * 自绘进度条。
 *
 * 不使用原生 `<input type=range>`（依赖焦点与激活状态，在悬浮窗里不可靠）。
 * 拖动同时走两条通道：
 * 1) 标准指针事件（带 pointer capture，光标移出进度条也继续跟手）；
 * 2) 按下之后按光标位置轮询——位置与左键状态都从 Win32 直接取，
 *    因此即使 WebView2 不派发 pointermove / pointerup 也能稳定工作。
 *
 * 两条通道都用「绝对光标位置 ÷ 轨道宽度」换算，不做相对位移累加，
 * 否则任一通道的坐标基准有偏差就会整体跑飞。
 */
export class SeekBar {
  private readonly root: HTMLElement;
  private readonly fill: HTMLElement;
  private readonly thumb: HTMLElement;
  private readonly callbacks: SeekBarCallbacks;

  private fraction = 0;
  private interactable = true;

  private scrubbing = false;
  private pointerId = -1;
  private pressAt = 0;
  private timer = 0;
  private polling = false;

  /** 松手后锁定的目标位置与解锁时刻。 */
  private lockFraction = 0;
  private lockUntil = 0;

  constructor(root: HTMLElement, callbacks: SeekBarCallbacks = {}) {
    this.root = root;
    this.fill = root.querySelector<HTMLElement>("#seekFill")!;
    this.thumb = root.querySelector<HTMLElement>("#seekThumb")!;
    this.callbacks = callbacks;

    root.addEventListener("pointerdown", this.onPointerDown);
    root.addEventListener("pointermove", this.onPointerMove);
    root.addEventListener("pointerup", this.onPointerUp);
    root.addEventListener("pointercancel", this.onPointerCancel);
    // 不可拖动（直播流 / 播放器不接受外部跳转）时连悬停效果都不给：
    // 滑块不出现，用户一眼就知道这条进度条拖不动
    root.addEventListener("pointerenter", () => {
      if (this.interactable) root.classList.add("hover");
    });
    root.addEventListener("pointerleave", () => {
      if (!this.scrubbing) root.classList.remove("hover");
    });

    // 兜底：指针捕获失败（或事件只冒到 window）时，拖动与收尾依然成立
    window.addEventListener("pointermove", this.onWindowMove, { passive: true });
    window.addEventListener("pointerup", this.onWindowUp);
    window.addEventListener("pointercancel", this.onWindowUp);
  }

  setInteractable(value: boolean): void {
    if (this.interactable === value) return;

    this.interactable = value;
    this.root.classList.toggle("disabled", !value);

    // 拖动途中被禁用（例如后端刚判定这个播放器不支持跳转）：直接中止，
    // 不能走 complete() —— 那会把最后的位置当成一次跳转请求发出去
    if (!value && this.scrubbing) this.abort();
  }

  /** 外部推进进度（拖动中、以及松手后尚未归位时忽略，避免抖动 / 回弹）。 */
  setProgress(fraction: number): void {
    if (this.scrubbing) return;

    const next = clamp01(fraction);
    if (this.lockUntil > 0) {
      if (Math.abs(next - this.lockFraction) > SEEK_LOCK_TOLERANCE) {
        // 后端还没跟到目标位置：这期间的进度都基于旧快照，直接丢弃
        if (performance.now() < this.lockUntil) return;
      }
      this.lockUntil = 0;
    }

    this.fraction = next;
    this.render();
  }

  private onPointerDown = (event: PointerEvent): void => {
    if (event.button !== 0) return;

    // 不可拖动：吞掉这次点击的冒泡，别让它被当成「点空白处」把卡片收起来
    if (!this.interactable) {
      event.stopPropagation();
      return;
    }

    // 阻止冒泡：拖动进度条不应该让整个岛收起
    event.stopPropagation();
    // 阻止默认行为：悬浮窗未激活，别让它去做文本选择 / 拖拽
    event.preventDefault();

    this.scrubbing = true;
    this.lockUntil = 0;
    this.pressAt = performance.now();
    this.pointerId = event.pointerId;
    this.root.classList.add("hover");

    // 捕获指针：光标拖到轨道外、甚至岛外，pointermove / pointerup 依然回到这里
    try {
      this.root.setPointerCapture(event.pointerId);
    } catch {
      // 没有指针捕获时还有轮询兜底
    }

    this.callbacks.onScrubStarted?.();
    this.scrubTo(event.clientX);
    this.startPolling();
  };

  private onPointerMove = (event: PointerEvent): void => {
    if (!this.scrubbing) return;
    event.stopPropagation();
    this.scrubTo(event.clientX);
  };

  private onPointerUp = (event: PointerEvent): void => {
    if (!this.scrubbing) return;
    event.stopPropagation();
    this.complete();
  };

  private onPointerCancel = (): void => {
    if (!this.scrubbing) return;
    this.complete();
  };

  private onWindowMove = (event: PointerEvent): void => {
    if (!this.scrubbing || event.pointerId !== this.pointerId) return;
    this.scrubTo(event.clientX);
  };

  private onWindowUp = (event: PointerEvent): void => {
    if (!this.scrubbing || event.pointerId !== this.pointerId) return;
    this.complete();
  };

  /** 把 fraction 落到给定视口横坐标上（按下 / 拖动 / 轮询共用）。 */
  private scrubTo(clientX: number): void {
    const rect = this.root.getBoundingClientRect();
    if (rect.width <= 1) return;

    const next = clamp01((clientX - rect.left) / rect.width);
    if (Math.abs(next - this.fraction) < 0.0005) return;

    this.fraction = next;
    this.render();
    this.callbacks.onScrubMoved?.(this.fraction);
  }

  private startPolling(): void {
    this.stopPolling();
    this.timer = window.setInterval(() => void this.pollCursor(), POLL_MS);
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
    // 上一次探测还没返回就跳过这一拍，避免请求堆积
    if (this.polling) return;

    this.polling = true;
    let probe: { X: number; Y: number; Down: boolean } | null = null;
    try {
      probe = await api.cursorProbe();
    } catch {
      probe = null;
    }
    this.polling = false;

    if (!this.scrubbing) return;
    if (!probe) return;

    // 刚按下的头几拍不把「左键未按下」当成松手：按下瞬间的异步键状态可能还没翻转
    const pressedFor = performance.now() - this.pressAt;
    if (!probe.Down) {
      if (pressedFor > RELEASE_GRACE_MS) this.complete();
      return;
    }

    this.scrubTo(probe.X);
  }

  private complete(): void {
    if (!this.scrubbing) return;

    this.abort();

    // 松手后的归位锁定：真实快照回来之前不让旧进度覆盖拖拽结果
    this.lockFraction = this.fraction;
    this.lockUntil = performance.now() + SEEK_LOCK_MS;

    this.callbacks.onSeekRequested?.(this.fraction);
    this.callbacks.onUserActed?.();
  }

  /** 中止拖动（不发跳转请求）：禁用、窗口失焦等场景用。 */
  private abort(): void {
    this.scrubbing = false;
    this.stopPolling();
    this.root.classList.remove("hover");
    this.lockUntil = 0;

    if (this.pointerId >= 0) {
      try {
        this.root.releasePointerCapture(this.pointerId);
      } catch {
        // 指针已经不在捕获中，忽略
      }
      this.pointerId = -1;
    }
  }

  private render(): void {
    const percentage = (clamp01(this.fraction) * 100).toFixed(2);
    this.fill.style.width = `${percentage}%`;
    // 滑块中心对齐进度点，两端不越出轨道
    this.thumb.style.left = `clamp(0px, calc(${percentage}% - ${THUMB_WIDTH / 2}px), calc(100% - ${THUMB_WIDTH}px))`;
  }
}

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}
