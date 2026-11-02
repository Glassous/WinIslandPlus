/**
 * 单实例缓动驱动器：以 requestAnimationFrame 在 DIP 空间插值 宽 / 高 / 圆角。
 *
 * - 展开 300 ms 缓出（cubic-out）、收起 240 ms 缓入缓出（cubic-in-out），
 *   宽 / 高 / 圆角三者同步插值；
 * - 被新动画打断时从当前值续算，不会跳帧；
 * - 空闲时不占用任何资源。
 */

export type EasingKind = "out" | "inOut";

function easeOut(t: number): number {
  const inv = 1 - t;
  return 1 - inv * inv * inv;
}

function easeInOut(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

export class IslandAnimator {
  currentWidth = 0;
  currentHeight = 0;
  currentCorner = 0;

  onFrame: ((width: number, height: number, corner: number) => void) | null = null;
  onComplete: (() => void) | null = null;

  private fromWidth = 0;
  private fromHeight = 0;
  private fromCorner = 0;
  private toWidth = 0;
  private toHeight = 0;
  private toCorner = 0;
  private duration = 240;
  private easing: (t: number) => number = easeOut;
  private startedAt = 0;
  private handle = 0;
  private running = false;

  get isAnimating(): boolean {
    return this.running;
  }

  /** 立即生效，不产生动画。 */
  snap(width: number, height: number, corner: number): void {
    this.cancel();
    this.currentWidth = width;
    this.currentHeight = height;
    this.currentCorner = corner;
    this.onFrame?.(width, height, corner);
  }

  animate(
    width: number,
    height: number,
    corner: number,
    durationMs = 240,
    easing: EasingKind = "out",
  ): void {
    if (this.currentWidth <= 0) {
      this.snap(width, height, corner);
      return;
    }

    this.fromWidth = this.currentWidth;
    this.fromHeight = this.currentHeight;
    this.fromCorner = this.currentCorner;
    this.toWidth = width;
    this.toHeight = height;
    this.toCorner = corner;
    this.duration = Math.max(1, durationMs);
    this.easing = easing === "out" ? easeOut : easeInOut;

    this.cancel();
    this.startedAt = performance.now();
    this.running = true;
    this.handle = requestAnimationFrame(this.tick);
  }

  private cancel(): void {
    if (this.handle) {
      cancelAnimationFrame(this.handle);
      this.handle = 0;
    }
    this.running = false;
  }

  private tick = (now: number): void => {
    if (!this.running) return;

    const progress = Math.min(1, Math.max(0, (now - this.startedAt) / this.duration));
    const eased = this.easing(progress);

    this.currentWidth = this.fromWidth + (this.toWidth - this.fromWidth) * eased;
    this.currentHeight = this.fromHeight + (this.toHeight - this.fromHeight) * eased;
    this.currentCorner = this.fromCorner + (this.toCorner - this.fromCorner) * eased;

    this.onFrame?.(this.currentWidth, this.currentHeight, this.currentCorner);

    if (progress >= 1) {
      this.running = false;
      this.handle = 0;
      this.onComplete?.();
      return;
    }

    this.handle = requestAnimationFrame(this.tick);
  };
}
