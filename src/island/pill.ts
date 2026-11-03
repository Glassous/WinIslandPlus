import { PILL_PADDING } from "../common/metrics";
import type { MediaSnapshot, PillRightStyle } from "../common/types";

/**
 * 媒体岛的胶囊层。
 *
 * 左侧恒为 18×18 缩略封面（没有封面时整块让位）；右侧按设置三选一，互斥生效：
 *
 * - `Spectrum`（默认）：四根跳动音柱，播放中跳动、暂停时静止；没有封面时频谱在胶囊里居中；
 * - `ProgressRing`：播放进度圈，从 12 点方向起顺时针走满一圈；
 * - `Lyrics`：当前一句歌词，胶囊宽度按这句歌词的自然宽度解算，并且**只向右生长**，
 *   左侧（封面 / 胶囊左沿）保持不动，避免换句时整条胶囊左右晃。
 *
 * 与 dsh 岛并存时媒体岛收成正圆（`circle` 类）：圆内只有专辑图 / 频谱，没有「右侧」，
 * 进度圈与歌词都不显示。
 *
 * 未展开时不显示任何取色光晕——光晕只属于展开的媒体卡片。
 */
export class MediaPillView {
  private readonly actor: HTMLElement;
  private readonly band: HTMLElement;
  private readonly cover: HTMLElement;
  private readonly bars: HTMLElement;
  private readonly ringFill: SVGCircleElement;
  private readonly lyricText: HTMLElement;

  private style: PillRightStyle = "Spectrum";
  /** 胶囊里正在显示的歌词（空串表示没有可显示的句子）。 */
  private lyric = "";
  /** 内容自然宽度缓存（DIP）；负数表示需要重新测量。 */
  private naturalWidth = -1;

  constructor(actor: HTMLElement) {
    this.actor = actor;
    this.band = actor.querySelector<HTMLElement>("#mediaPill")!;
    this.cover = actor.querySelector<HTMLElement>("#pillCover")!;
    this.bars = actor.querySelector<HTMLElement>("#pillBars")!;
    this.ringFill = actor.querySelector<SVGCircleElement>("#pillRingFill")!;
    this.lyricText = actor.querySelector<HTMLElement>("#pillLyricText")!;
    this.setRightStyle("Spectrum");
  }

  /** 右侧样式：三选一，切换后需要调用方重新布局（宽度可能变化）。 */
  setRightStyle(style: PillRightStyle): void {
    this.style = style;
    this.actor.classList.toggle("side-spectrum", style === "Spectrum");
    this.actor.classList.toggle("side-ring", style === "ProgressRing");
    this.actor.classList.toggle("side-lyrics", style === "Lyrics");
    if (style !== "Lyrics") this.actor.classList.remove("truncated");
    // 换样式后歌词可能刚出现 / 刚消失，宽度需要重算
    this.naturalWidth = -1;
  }

  get rightStyle(): PillRightStyle {
    return this.style;
  }

  /** 切到正圆形态（两个岛同排时的媒体岛）。 */
  setCircle(circle: boolean): void {
    this.actor.classList.toggle("circle", circle);
  }

  /** 播放进度（0~1）：只在进度圈模式下写 DOM。 */
  setProgress(fraction: number): void {
    if (this.style !== "ProgressRing") return;

    const value = Number.isFinite(fraction) ? Math.min(1, Math.max(0, fraction)) : 0;
    // pathLength=100：dashoffset 就是「还没唱到的百分比」
    this.ringFill.style.strokeDashoffset = (100 - value * 100).toFixed(2);
  }

  /**
   * 写入当前一句歌词。返回 true 表示宽度可能变了，调用方应重新布局。
   *
   * 这里只写文本并作废测量缓存；真正测量发生在 `lyricPillWidth()` 里，
   * 因为宽度上限取决于当前尺寸档位（由调用方提供）。
   */
  setLyric(text: string): boolean {
    const next = text.trim();
    if (next === this.lyric) return false;

    this.lyric = next;
    this.lyricText.textContent = next;
    this.naturalWidth = -1;
    // 只有歌词样式会因文案改变宽度；其它样式不用惊动布局
    return this.style === "Lyrics";
  }

  /**
   * 歌词模式下胶囊应有的总宽度（DIP）；返回 0 表示按标准胶囊宽度布局。
   *
   * 宽度 = 内容自然宽度（封面 + 间距 + 歌词）+ 左右内边距，夹在
   * `[baseWidth, maxWidth]` 之间：短歌词不缩成比标准胶囊更窄，超长的按上限截断。
   */
  lyricPillWidth(baseWidth: number, maxWidth: number): number {
    if (this.style !== "Lyrics") return 0;

    const inner = this.measureInner();
    if (inner <= 0) {
      this.actor.classList.remove("truncated");
      return 0;
    }

    const natural = inner + PILL_PADDING;
    const width = Math.min(
      Math.max(natural, baseWidth),
      Math.max(baseWidth, maxWidth),
    );

    // 被上限截断时给右侧加一点渐隐，读起来像「还有下文」而不是被硬切
    this.actor.classList.toggle("truncated", natural > width + 0.5);

    return width;
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

    // 有封面：封面 + 右侧元素 / 圆内只有封面；没封面：频谱居中（恢复改动前的样式）
    this.actor.classList.toggle("has-art", Boolean(artwork));
    this.actor.classList.toggle("empty", snapshot === null);
    this.bars.classList.toggle("center", !artwork);

    // 封面有无会改变「封面 + 歌词」的总宽度
    this.naturalWidth = -1;

    this.setPlaying(snapshot?.IsPlaying ?? false);
  }

  private setPlaying(playing: boolean): void {
    this.bars.classList.toggle("playing", playing);
    this.bars.classList.toggle("frozen", !playing);
  }

  /**
   * 内容自然宽度（DIP）。测量期间把胶囊切成 `width: max-content` 的绝对定位元素，
   * 拿到的就是「不被容器夹住」的真实内容宽度；每个歌词 / 样式 / 封面变化只测一次。
   */
  private measureInner(): number {
    if (this.lyric.length === 0) return 0;
    if (this.naturalWidth >= 0) return this.naturalWidth;

    this.band.classList.add("measuring");
    const width = this.band.offsetWidth;
    this.band.classList.remove("measuring");

    // 0 通常意味着此刻还量不到（窗口尚未显示 / 字体未就绪）：
    // 不写缓存，下一次重新布局再量一次，别把「没有宽度」固化成结论
    if (width > 0) this.naturalWidth = width;
    return width;
  }
}
