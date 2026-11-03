/**
 * 单个岛面的几何补间：在 DIP 空间插值位置（x / y）、尺寸（宽 / 高）、
 * 圆角以及胶囊↔卡片的形变进度。
 *
 * 不自己起 rAF——由调用方的单一帧循环统一 `step()`，
 * 这样两个岛面永远在同一帧拿到同一时间戳，并且形状只推送一次。
 *
 * - 展开 300 ms 缓出（cubic-out）、收起 240 ms 缓入缓出（cubic-in-out）；
 * - 被新动画打断时从当前值续算，不会跳帧；
 * - 空闲时不占用任何资源。
 */

export type EasingKind = "out" | "inOut";

/** 岛面在某一时刻的几何（DIP，相对岛窗客户区）。 */
export interface ActorGeometry {
  x: number;
  y: number;
  width: number;
  height: number;
  radius: number;
  /** 0 = 胶囊形态，1 = 卡片形态；用于胶囊层 / 卡片层交叉淡化。 */
  morph: number;
}

const FIELDS = ["x", "y", "width", "height", "radius", "morph"] as const;

function easeOut(t: number): number {
  const inv = 1 - t;
  return 1 - inv * inv * inv;
}

function easeInOut(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

function copy(source: ActorGeometry): ActorGeometry {
  return {
    x: source.x,
    y: source.y,
    width: source.width,
    height: source.height,
    radius: source.radius,
    morph: source.morph,
  };
}

function lerp(from: number, to: number, t: number): number {
  return from + (to - from) * t;
}

export class GeometryTween {
  /** 当前实际值（首帧前全为 0）。 */
  current: ActorGeometry = {
    x: 0,
    y: 0,
    width: 0,
    height: 0,
    radius: 0,
    morph: 0,
  };

  /** 每步写一次 DOM 用；静止时也会在 snap 后触发一次。 */
  onFrame: ((geometry: ActorGeometry) => void) | null = null;

  /** 目标值：用于判断「已经在该位置」时跳过动画。 */
  private target: ActorGeometry = copy(this.current);
  private from: ActorGeometry = copy(this.current);
  private duration = 240;
  private easing: (t: number) => number = easeOut;
  private startedAt = 0;
  private running = false;

  /** 是否已经有过一次落位（没有则需要 snap 而不是动画）。 */
  get initialized(): boolean {
    return this.current.width > 0;
  }

  get isAnimating(): boolean {
    return this.running;
  }

  /** 立即生效，不产生动画。 */
  snap(geometry: ActorGeometry): void {
    this.running = false;
    this.current = copy(geometry);
    this.target = copy(geometry);
    this.onFrame?.(this.current);
  }

  /**
   * 设定目标几何。目标与当前值几乎一致时什么也不做，避免无谓地打断动画。
   * 调用方在 `animate` 之后需要自己 kick 一次帧循环。
   */
  animate(geometry: ActorGeometry, durationMs = 240, easing: EasingKind = "out"): void {
    if (!this.initialized) {
      this.snap(geometry);
      return;
    }

    if (this.settled(geometry)) {
      // 尺寸可能因为打断而有极小残差：一次 snap 抹平，不再起动画
      if (this.running) this.snap(geometry);
      return;
    }

    this.from = copy(this.current);
    this.target = copy(geometry);
    this.duration = Math.max(1, durationMs);
    this.easing = easing === "out" ? easeOut : easeInOut;
    this.startedAt = performance.now();
    this.running = true;
  }

  /** 推进一帧；返回是否仍在运行。 */
  step(now: number): boolean {
    if (!this.running) return false;

    const progress = Math.min(1, Math.max(0, (now - this.startedAt) / this.duration));
    const eased = this.easing(progress);

    for (const field of FIELDS) {
      this.current[field] = lerp(this.from[field], this.target[field], eased);
    }

    this.onFrame?.(this.current);

    if (progress >= 1) {
      this.running = false;
      return false;
    }

    return true;
  }

  /** 当前是否已经在目标几何上（容差 0.5 DIP）。 */
  private settled(geometry: ActorGeometry): boolean {
    for (const field of FIELDS) {
      if (Math.abs(this.target[field] - geometry[field]) > 0.5) return false;
      if (Math.abs(this.current[field] - geometry[field]) > 0.5) return false;
    }
    return true;
  }
}
