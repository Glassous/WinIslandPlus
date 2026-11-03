/**
 * 播放时钟：把后端采样到的「真实播放位置 + 本地时刻」换算成连续、可微调的播放位置。
 *
 * 为什么不能只做一次线性外推：位置只在快照到达时才有真值，两次快照之间全靠本地时钟，
 * 采样延迟、播放器上报的位置粒度、缓冲 / 变速都会让「外推值」与真实播放头缓慢分叉；
 * 而分叉一旦被当成新的基准，就会变成**整首歌**的歌词偏移（唱到后面越差越多）。
 *
 * 这里的做法：
 * - 每次拿到真值就比对一次，偏差很小（< 120 ms）忽略，避免抖动；
 * - 中等偏差不重锚，而是叠一段会自然衰减的偏移量（约 1 秒内平滑消化），
 *   位置连续推进、不跳帧，歌词不会突然弹一下；
 * - 大偏差（用户跳转、换歌、播放器给出异常值）才直接落位——这种时候不落位反而会整首偏。
 */

/** 超过这个偏差就认为是跳转 / 异常，直接落位。 */
const SNAP_MS = 1_500;
/** 小于这个偏差不值得调整（播放器位置粒度通常在百毫秒级）。 */
const IGNORE_MS = 120;
/** 中等偏差的吸收比例：剩下的部分留给衰减过程自己追上。 */
const SMOOTH_GAIN = 0.6;
/** 偏移量上限：防止一次异常值把时钟顶飞。 */
const DRIFT_MAX_MS = 3_000;
/** 偏移量衰减时间常数（毫秒）：越小追得越快，越大越平滑。 */
const DRIFT_TAU_MS = 900;

export class PlaybackClock {
  /** 曲目身份键：键变了就重新锚定。 */
  private key = "";
  /** 锚点：`base` 位置对应本地时刻 `stamp`。 */
  private base = 0;
  private stamp = 0;
  private playing = false;
  private duration = 0;
  /** 待消化的偏差（会按指数衰减），只在播放中有效。 */
  private drift = 0;
  private driftStamp = 0;

  /** 已知的总时长（毫秒，0 表示未知）。 */
  get durationMs(): number {
    return this.duration;
  }

  /** 换曲目 / 停止播放时的整体复位。 */
  clear(): void {
    this.key = "";
    this.base = 0;
    this.stamp = 0;
    this.playing = false;
    this.duration = 0;
    this.drift = 0;
    this.driftStamp = 0;
  }

  /**
   * 与后端真值对位（整份快照与高频位置推送都走这里）。
   *
   * `key` 是曲目身份键，`stampMs` 是后端采样位置的本地时刻（Unix 毫秒）。
   */
  sync(
    key: string,
    positionMs: number,
    stampMs: number,
    playing: boolean,
    durationMs: number,
  ): void {
    if (!Number.isFinite(positionMs) || !Number.isFinite(stampMs) || stampMs <= 0) return;
    if (Number.isFinite(durationMs) && durationMs > 0) this.duration = durationMs;

    // 换歌 / 播放状态翻转：旧的锚点已经没有参考价值
    if (key !== this.key || playing !== this.playing) {
      this.anchor(key, positionMs, stampMs, playing);
      return;
    }

    // 过期的采样（整份快照与位置推送可能交错到达）：丢掉，别把时钟往回拽
    if (stampMs < this.stamp) return;

    // 暂停期间位置是静止的：直接对齐，看不到任何跳动
    if (!playing) {
      this.anchor(key, positionMs, stampMs, false);
      return;
    }

    const error = positionMs - this.position(stampMs);

    // 跳转 / 拖动 / 明显异常：只能落位，硬撑会整首歌偏着
    if (Math.abs(error) >= SNAP_MS) {
      this.anchor(key, positionMs, stampMs, true);
      return;
    }

    if (Math.abs(error) < IGNORE_MS) return;

    // 中等偏差：锚点不动，只叠一段会自然衰减的偏移量，位置连续推进
    this.drift = clamp(
      this.driftAt(stampMs) + error * SMOOTH_GAIN,
      -DRIFT_MAX_MS,
      DRIFT_MAX_MS,
    );
    this.driftStamp = stampMs;
  }

  /** 本地跳转（用户拖完进度条）：立即按目标位置重锚，不再等后端回包。 */
  seek(positionMs: number): void {
    if (!Number.isFinite(positionMs)) return;
    this.base = Math.max(0, positionMs);
    this.stamp = Date.now();
    this.drift = 0;
    this.driftStamp = this.stamp;
  }

  /** 当前推算位置（毫秒）。 */
  position(now: number = Date.now()): number {
    let value = this.base;
    if (this.playing) {
      value += now - this.stamp;
      value += this.driftAt(now);
    }
    if (this.duration > 0 && value > this.duration) value = this.duration;
    return value > 0 ? value : 0;
  }

  private anchor(key: string, positionMs: number, stampMs: number, playing: boolean): void {
    this.key = key;
    this.base = Math.max(0, positionMs);
    this.stamp = stampMs;
    this.playing = playing;
    this.drift = 0;
    this.driftStamp = stampMs;
  }

  private driftAt(now: number): number {
    if (this.drift === 0) return 0;
    const elapsed = Math.max(0, now - this.driftStamp);
    return this.drift * Math.exp(-elapsed / DRIFT_TAU_MS);
  }
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
