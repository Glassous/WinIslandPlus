import { formatRemaining, formatTime } from "../common/time";
import type { MediaSnapshot } from "../common/types";
import { SeekBar } from "./seekbar";

export interface CardCallbacks {
  onPlayPause: () => void;
  onPrevious: () => void;
  onNext: () => void;
  onSeek: (fraction: number) => void;
  onUserActed: () => void;
}

/**
 * 展开态媒体面板：封面 + 标题 / 艺术家 + 右侧均衡器、进度行、三个纯图标按钮。
 * 所有交互都基于指针事件（不使用原生控件），因为岛窗永不激活。
 */
export class MediaCard {
  private readonly coverImage: HTMLElement;
  private readonly title: HTMLElement;
  private readonly artist: HTMLElement;
  private readonly eqBars: HTMLElement;
  private readonly playIcon: HTMLElement;
  private readonly positionText: HTMLElement;
  private readonly remainingText: HTMLElement;
  private readonly seekBar: SeekBar;

  constructor(root: HTMLElement, callbacks: CardCallbacks) {
    this.coverImage = root.querySelector<HTMLElement>("#coverImageHost")!;
    this.title = root.querySelector<HTMLElement>("#titleText")!;
    this.artist = root.querySelector<HTMLElement>("#artistText")!;
    this.eqBars = root.querySelector<HTMLElement>("#eqBars")!;
    this.playIcon = root.querySelector<HTMLElement>("#playIcon")!;
    this.positionText = root.querySelector<HTMLElement>("#positionText")!;
    this.remainingText = root.querySelector<HTMLElement>("#remainingText")!;

    const seekRoot = root.querySelector<HTMLElement>("#seekBar")!;
    this.seekBar = new SeekBar(seekRoot, {
      onScrubStarted: () => callbacks.onUserActed(),
      onSeekRequested: (fraction) => callbacks.onSeek(fraction),
      onUserActed: () => callbacks.onUserActed(),
    });

    this.wireButton(root.querySelector<HTMLElement>("#prevButton")!, callbacks.onPrevious, callbacks);
    this.wireButton(root.querySelector<HTMLElement>("#playButton")!, callbacks.onPlayPause, callbacks);
    this.wireButton(root.querySelector<HTMLElement>("#nextButton")!, callbacks.onNext, callbacks);
  }

  get seek(): SeekBar {
    return this.seekBar;
  }

  update(snapshot: MediaSnapshot | null): void {
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

    // E769 = 暂停图标，E768 = 播放图标
    this.playIcon.textContent = snapshot.IsPlaying ? "\uE769" : "\uE768";

    this.positionText.textContent = formatTime(0);
    this.remainingText.textContent = formatRemaining(0, snapshot.DurationMs);
    this.seekBar.setInteractable(snapshot.DurationMs > 0);
  }

  /** 外部推进进度（拖动中被忽略）。 */
  setProgress(positionMs: number, durationMs: number): void {
    if (durationMs <= 0) return;
    this.seekBar.setProgress(positionMs / durationMs);
    this.positionText.textContent = formatTime(positionMs);
    this.remainingText.textContent = formatRemaining(positionMs, durationMs);
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
