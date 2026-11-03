import { currentPosition, formatRemaining, formatTime, trackKeyOf } from "../common/time";
import type { LyricsSnapshot, MediaSnapshot } from "../common/types";
import {
  activeLineIndex,
  decodeLyricsHex,
  lineProgress,
  parseLrc,
  retimeLyrics,
  type LyricLine,
} from "./lyrics";
import { SeekBar } from "./seekbar";

export interface CardCallbacks {
  onPlayPause: () => void;
  onPrevious: () => void;
  onNext: () => void;
  onSeek: (fraction: number) => void;
  onUserActed: () => void;
  /**
   * 当前句变化（含「无歌词」→ 空串）：胶囊的歌词模式靠它同步文案。
   *
   * 卡片与胶囊共用同一条时间轴，胶囊不需要自己再解析一遍歌词。
   */
  onLyricLine?: (text: string) => void;
}

/** 歌词滚动换位时长（与 `.lyric-roll` 的 transform transition 保持一致）。 */
const LYRIC_ROLL_MS = 300;
/** 歌词视口预置的行数：当前句 + 下一句。 */
const LYRIC_ROWS = 2;
/** 歌词行缓存上限：够「切走再切回」用，不会无界增长。 */
const LYRICS_CACHE_LIMIT = 24;
/** 视口行高兜底值（CSS 变量读不到时用）。 */
const LYRIC_ROW_FALLBACK = 15;

/** 一行歌词的 DOM：底字 + 覆盖在上面的「已唱」高亮层。 */
interface LyricRow {
  line: HTMLElement;
  base: HTMLElement;
  fill: HTMLElement;
}

/**
 * 展开态媒体面板：封面 + 标题 / 艺术家 / 歌词 + 右侧均衡器、进度行、三个纯图标按钮。
 * 所有交互都基于指针事件（不使用原生控件），因为岛窗永不激活。
 */
export class MediaCard {
  private readonly coverImage: HTMLElement;
  private readonly title: HTMLElement;
  private readonly artist: HTMLElement;
  private readonly lyric: HTMLElement;
  private readonly lyricRoll: HTMLElement;
  private readonly lyricRows: LyricRow[];
  private readonly eqBars: HTMLElement;
  private readonly playButton: HTMLElement;
  private readonly positionText: HTMLElement;
  private readonly remainingText: HTMLElement;
  private readonly seekBar: SeekBar;

  private playing = false;
  /** 当前曲目身份键；歌词事件不匹配即忽略。 */
  private currentKey = "";
  /** 当前曲目的专辑名（无歌词时的占位文本）与时长（最后一句的兜底结尾）。 */
  private album = "";
  private durationMs = 0;
  private lastPositionMs = 0;

  private lyricLines: LyricLine[] = [];
  private lyricIndex = -1;
  /** 两行视口当前承载的文本，用来判断能不能「滚动换位」而不是硬切。 */
  private lyricRowText: [string, string] = ["", ""];
  /** 正在显示当前句的那一行（滚动过程中会临时切到下一行）。 */
  private activeRow = 0;
  private lyricProgress = 0;
  private lyricRollTimer = 0;
  private lyricRollEnd: ((event: TransitionEvent) => void) | null = null;
  private readonly reduceMotion: boolean;
  /** 歌词行缓存：媒体 → 收起 → 重新展开时立刻恢复，不用等后端再回一次。 */
  private readonly lyricsCache = new Map<string, LyricLine[]>();
  /** 当前句变化时通知外层（胶囊歌词模式）。 */
  private readonly onLyricLine?: (text: string) => void;

  constructor(root: HTMLElement, callbacks: CardCallbacks) {
    this.onLyricLine = callbacks.onLyricLine;
    this.coverImage = root.querySelector<HTMLElement>("#coverImageHost")!;
    this.title = root.querySelector<HTMLElement>("#titleText")!;
    this.artist = root.querySelector<HTMLElement>("#artistText")!;
    this.lyric = root.querySelector<HTMLElement>("#lyricText")!;
    this.lyricRoll = root.querySelector<HTMLElement>("#lyricRoll")!;
    this.eqBars = root.querySelector<HTMLElement>("#eqBars")!;
    this.playButton = root.querySelector<HTMLElement>("#playButton")!;
    this.positionText = root.querySelector<HTMLElement>("#positionText")!;
    this.remainingText = root.querySelector<HTMLElement>("#remainingText")!;

    this.lyricRows = [];
    root
      .querySelectorAll<HTMLElement>("#lyricText .lyric-line")
      .forEach((node) => {
        const base = node.querySelector<HTMLElement>(".lyric-base");
        const fill = node.querySelector<HTMLElement>(".lyric-fill");
        if (base && fill) this.lyricRows.push({ line: node, base, fill });
      });

    this.reduceMotion =
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    const seekRoot = root.querySelector<HTMLElement>("#seekBar")!;
    this.seekBar = new SeekBar(seekRoot, {
      onScrubStarted: () => callbacks.onUserActed(),
      onSeekRequested: (fraction) => callbacks.onSeek(fraction),
      onUserActed: () => callbacks.onUserActed(),
    });

    this.wireButton(root.querySelector<HTMLElement>("#prevButton")!, callbacks.onPrevious, callbacks);
    this.wireButton(this.playButton, callbacks.onPlayPause, callbacks);
    this.wireButton(root.querySelector<HTMLElement>("#nextButton")!, callbacks.onNext, callbacks);
  }

  get seek(): SeekBar {
    return this.seekBar;
  }

  update(snapshot: MediaSnapshot | null, positionMs?: number): void {
    if (!snapshot) {
      this.title.textContent = "暂无媒体";
      this.artist.textContent = "";
      this.artist.style.visibility = "hidden";
      this.coverImage.style.backgroundImage = "";
      this.coverImage.style.opacity = "0";
      this.eqBars.style.visibility = "hidden";
      this.positionText.textContent = "0:00";
      this.remainingText.textContent = "-0:00";
      this.seekBar.setInteractable(false);
      this.seekBar.setProgress(0);
      this.setPlayingState(false);
      this.currentKey = "";
      this.album = "";
      this.durationMs = 0;
      this.lastPositionMs = 0;
      this.resetLyrics("");
      return;
    }

    this.title.textContent = snapshot.Title.trim() ? snapshot.Title : "未知曲目";

    const hasArtist = snapshot.Artist.trim().length > 0;
    this.artist.textContent = snapshot.Artist;
    this.artist.style.visibility = hasArtist ? "visible" : "hidden";
    this.eqBars.style.visibility = "visible";

    if (snapshot.ArtworkUrl) {
      this.coverImage.style.backgroundImage = `url("${snapshot.ArtworkUrl}")`;
      this.coverImage.style.opacity = "1";
    } else {
      this.coverImage.style.backgroundImage = "";
      this.coverImage.style.opacity = "0";
    }

    this.setPlayingState(snapshot.IsPlaying);

    // 位置由调用方给（播放时钟算出来的），没给就退回快照里的采样值
    const position = positionMs ?? currentPosition(snapshot);
    this.durationMs = snapshot.DurationMs;
    this.lastPositionMs = position;
    this.album = snapshot.AlbumTitle;
    this.positionText.textContent = formatTime(position);
    this.remainingText.textContent = formatRemaining(position, snapshot.DurationMs);
    // 时长未知（直播流）或播放器实测不接受外部跳转时，进度条只展示不可拖
    this.seekBar.setInteractable(snapshot.DurationMs > 0 && snapshot.Seekable !== false);

    const key = trackKeyOf(snapshot);
    if (key !== this.currentKey) {
      this.currentKey = key;
      this.restoreLyrics(key);
    } else if (this.lyricLines.length > 0) {
      // 同一首歌但时长补齐了：把最后一句的结束时间拉伸到曲尾
      retimeLyrics(this.lyricLines, this.durationMs);
    }
  }

  /** 外部推进进度（拖动中被忽略）。 */
  setProgress(positionMs: number, durationMs: number): void {
    if (durationMs <= 0) return;
    if (durationMs !== this.durationMs) {
      this.durationMs = durationMs;
      retimeLyrics(this.lyricLines, durationMs);
    }
    this.lastPositionMs = positionMs;
    this.seekBar.setProgress(positionMs / durationMs);
    this.positionText.textContent = formatTime(positionMs);
    this.remainingText.textContent = formatRemaining(positionMs, durationMs);
  }

  /** 点击播放/暂停的瞬间乐观翻转图标，不等后端事件（真实快照到达后自然校正）。 */
  togglePlayingOptimistic(): void {
    this.setPlayingState(!this.playing);
  }

  /**
   * 歌词检索完成推送：与当前曲目不符时只入缓存不影响画面，
   * 等切回该曲目时再立刻取用（媒体 → 收起 → 展开的来回切换因此不会丢歌词）。
   */
  setLyrics(snapshot: LyricsSnapshot): void {
    const key = trackKeyOf(snapshot);
    const text = snapshot.Lrc ?? (snapshot.LrcHex ? decodeLyricsHex(snapshot.LrcHex) : "");
    // 带上曲名：QQ / 网易云的歌词开头常有「歌名 - 歌手」这行头部信息，解析时丢掉
    const lines = text.length > 0 ? parseLrc(text, 0, snapshot.Title) : [];

    this.cacheLyrics(key, lines);

    if (key !== this.currentKey) return;
    if (lines.length === 0) {
      this.resetLyrics(snapshot.AlbumTitle);
      return;
    }
    this.applyLyrics(lines);
  }

  /** 按播放位置推进歌词（由 80ms 进度循环驱动，与是否展开无关）。 */
  tickLyrics(positionMs: number): void {
    if (this.lyricLines.length === 0) return;
    this.lastPositionMs = positionMs;

    const index = activeLineIndex(this.lyricLines, positionMs);
    if (index !== this.lyricIndex) {
      this.lyricIndex = index;
      this.showLineAt(index);
    }

    if (index >= 0) {
      this.paintProgress(lineProgress(this.lyricLines[index], positionMs));
    }
  }

  private setPlayingState(playing: boolean): void {
    this.playing = playing;
    this.playButton.classList.toggle("playing", playing);
  }

  private applyLyrics(lines: LyricLine[]): void {
    this.lyricLines = lines;
    this.lyricIndex = -1;
    retimeLyrics(lines, this.durationMs);

    // 先强制重排一次：位置可能已经落在中间某句上，直接从该句开始显示
    this.tickLyrics(this.lastPositionMs);
  }

  /** 切歌 / 回到某首歌时：先看缓存，有就立刻恢复，没有才退回专辑名占位。 */
  private restoreLyrics(key: string): void {
    const cached = this.lyricsCache.get(key);
    if (cached && cached.length > 0) {
      this.applyLyrics(cached);
      return;
    }
    this.resetLyrics(this.album);
  }

  private cacheLyrics(key: string, lines: LyricLine[]): void {
    if (key.length === 0) return;

    if (this.lyricsCache.size >= LYRICS_CACHE_LIMIT && !this.lyricsCache.has(key)) {
      const oldest = this.lyricsCache.keys().next();
      if (!oldest.done) this.lyricsCache.delete(oldest.value);
    }

    this.lyricsCache.set(key, lines);
  }

  /** 切到第 index 句（-1 表示首句之前）。 */
  private showLineAt(index: number): void {
    const current = index >= 0 ? displayText(this.lyricLines[index]) : "";

    const nextIndex = index + 1;
    const next =
      nextIndex >= 0 && nextIndex < this.lyricLines.length
        ? displayText(this.lyricLines[nextIndex])
        : "";

    this.onLyricLine?.(current);
    this.showLine(current, next, true);
  }

  /**
   * 换句：视口里上行是当前句、下行是下一句。
   * 下行正好就是新的当前句时整卷上移一行（上下句滚动换位），
   * 否则（跳转、切歌、首帧）直接归位，避免闪过一句错词。
   */
  private showLine(current: string, next: string, animate: boolean): void {
    this.cancelLyricRoll();
    this.lyric.hidden = false;
    this.lyric.classList.remove("fallback");

    if (this.lyricRows.length < LYRIC_ROWS) return;

    const canRoll =
      animate &&
      !this.reduceMotion &&
      this.lyricRowText[1] === current &&
      this.lyricRowText[0] !== current;

    if (!canRoll) {
      this.snap(current, next);
      return;
    }

    // 预置在下的那一行成为新的当前句：滚动途中先把它点亮，旧句随之变暗
    this.lyricRows[0].line.classList.remove("is-current");
    this.lyricRows[1].line.classList.add("is-current");
    this.activeRow = 1;

    this.lyricRoll.style.transform = `translateY(${-this.rowHeight()}px)`;

    const finish = () => {
      this.cancelLyricRoll();
      // 归位时由 snap 直接把当前句的高亮写到位，不再走 paintProgress
      // （否则会从上一句残留的进度过渡一次，表现为「先整句点亮再缩回去」）
      this.snap(current, next);
    };

    const onEnd = (event: TransitionEvent) => {
      if (event.propertyName !== "transform") return;
      this.lyricRollEnd = null;
      finish();
    };

    this.lyricRollEnd = onEnd;
    this.lyricRoll.addEventListener("transitionend", onEnd);
    // 兜底：transitionend 没来（窗口隐藏、行高为 0 等）也不能卡在半途
    this.lyricRollTimer = window.setTimeout(() => {
      this.lyricRollEnd = null;
      this.lyricRoll.removeEventListener("transitionend", onEnd);
      finish();
    }, LYRIC_ROLL_MS + 80);
  }

  /** 无动画地把两行放到目标状态（滚动结束后也走这里归位）。 */
  private snap(current: string, next: string): void {
    if (this.lyricRows.length < LYRIC_ROWS) return;

    this.lyricRowText = [current, next];
    this.setText(this.lyricRows[0], current);
    this.setText(this.lyricRows[1], next);
    this.lyricRows[0].line.classList.add("is-current");
    this.lyricRows[1].line.classList.remove("is-current");
    this.activeRow = 0;

    // 文本换人时必须同步重置高亮层：否则新句会继承上一句的 clip-path（往往已经接近满格），
    // 表现为「先整句点亮，再退回正确位置」。这里关掉过渡直接写入，避免吃到 CSS 过渡。
    this.paintProgress(this.lyricProgress, this.lyricRows[0], true);
    this.resetFill(this.lyricRows[1]);

    this.lyricRoll.classList.add("snap");
    this.lyricRoll.style.transform = "translateY(0)";
    // 强制回流，让「归位」不参与 transform 过渡
    void this.lyricRoll.offsetWidth;
    this.lyricRoll.classList.remove("snap");
  }

  private setText(row: LyricRow, text: string): void {
    row.base.textContent = text;
    row.fill.textContent = text;
  }

  /**
   * 句内进度：只用 clip-path 从左侧露出「已唱」高亮层。
   *
   * `immediate` 为 true 时先关掉过渡再写入——换句 / 跳转时用，
   * 否则新的 clip-path 会从上一句残留的值补间过来（看起来像「先点亮再退回」）。
   */
  private paintProgress(
    progress: number,
    row: LyricRow | undefined = this.lyricRows[this.activeRow],
    immediate = false,
  ): void {
    this.lyricProgress = clamp01(progress);
    if (!row) return;

    const clip = clipFor(this.lyricProgress);
    if (!immediate) {
      row.fill.style.clipPath = clip;
      return;
    }

    row.fill.style.transition = "none";
    row.fill.style.clipPath = clip;
    // 强制回流：让这次写入成为新的过渡起点，而不是被 CSS 过渡补间
    void row.fill.offsetWidth;
    row.fill.style.removeProperty("transition");
  }

  /** 把一行的高亮层直接收成「一点没唱」（不走过渡，也不影响当前句的进度值）。 */
  private resetFill(row: LyricRow): void {
    row.fill.style.transition = "none";
    row.fill.style.clipPath = clipFor(0);
    void row.fill.offsetWidth;
    row.fill.style.removeProperty("transition");
  }

  /** 清空歌词；有专辑名时以弱化样式占位，否则整行隐藏。 */
  private resetLyrics(album: string): void {
    this.cancelLyricRoll();
    this.lyricLines = [];
    this.lyricIndex = -1;
    this.lyricRowText = ["", ""];
    this.activeRow = 0;
    this.lyricProgress = 0;

    // 胶囊的歌词模式跟着清空：没有歌词时它退回标准宽度（频谱占位）
    this.onLyricLine?.("");

    const fallback = album.trim();
    this.lyric.hidden = fallback.length === 0;
    this.lyric.classList.toggle("fallback", fallback.length > 0);

    if (this.lyricRows.length < LYRIC_ROWS) return;

    this.setText(this.lyricRows[0], fallback);
    this.setText(this.lyricRows[1], "");
    this.lyricRows[0].line.classList.add("is-current");
    this.lyricRows[1].line.classList.remove("is-current");
    this.resetFill(this.lyricRows[0]);
    this.resetFill(this.lyricRows[1]);

    this.lyricRoll.classList.add("snap");
    this.lyricRoll.style.transform = "translateY(0)";
    void this.lyricRoll.offsetWidth;
    this.lyricRoll.classList.remove("snap");
  }

  private rowHeight(): number {
    const row = this.lyricRows[0];
    const height = row ? row.line.getBoundingClientRect().height : 0;
    return height > 0 ? height : LYRIC_ROW_FALLBACK;
  }

  private cancelLyricRoll(): void {
    if (this.lyricRollTimer) {
      window.clearTimeout(this.lyricRollTimer);
      this.lyricRollTimer = 0;
    }
    if (this.lyricRollEnd) {
      this.lyricRoll.removeEventListener("transitionend", this.lyricRollEnd);
      this.lyricRollEnd = null;
    }
  }

  private wireButton(
    host: HTMLElement,
    action: () => void,
    callbacks: CardCallbacks,
  ): void {
    host.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      // 阻止冒泡：点按钮不应该让整个岛收起
      event.stopPropagation();
      host.classList.add("pressed");
    });

    host.addEventListener("pointerup", (event) => {
      event.stopPropagation();
      host.classList.remove("pressed");
      callbacks.onUserActed();
      action();
    });

    host.addEventListener("pointerleave", () => host.classList.remove("pressed"));
    host.addEventListener("pointercancel", () => host.classList.remove("pressed"));
  }
}

/** 空句（间奏）显示音符，避免歌词区空一大块。 */
function displayText(line: LyricLine): string {
  return line.text.length > 0 ? line.text : "♪";
}

/** 句内进度 → 高亮层的 clip-path：从左侧露出 progress 比例。 */
function clipFor(progress: number): string {
  return `inset(0 ${((1 - clamp01(progress)) * 100).toFixed(2)}% 0 0)`;
}

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}
