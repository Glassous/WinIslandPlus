import {
  formatBalance,
  formatCost,
  formatTokens,
  showsBalance,
  stageClass,
  stageKey,
  stageLabel,
} from "../common/dsh";
import { formatRemaining, formatTime } from "../common/time";
import type { DshSnapshot, MediaSnapshot } from "../common/types";
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
  private readonly root: HTMLElement;
  private readonly coverImage: HTMLElement;
  private readonly title: HTMLElement;
  private readonly artist: HTMLElement;
  private readonly eqBars: HTMLElement;
  private readonly playIcon: HTMLElement;
  private readonly positionText: HTMLElement;
  private readonly remainingText: HTMLElement;
  private readonly seekBar: SeekBar;

  private readonly dsh: HTMLElement;
  private readonly dshDot: HTMLElement;
  private readonly dshStage: HTMLElement;
  private readonly dshTool: HTMLElement;
  private readonly dshModel: HTMLElement;
  private readonly dshTokens: HTMLElement;
  private readonly dshCost: HTMLElement;
  private readonly dshBalance: HTMLElement;

  constructor(root: HTMLElement, callbacks: CardCallbacks) {
    this.root = root;
    this.coverImage = root.querySelector<HTMLElement>("#coverImageHost")!;
    this.title = root.querySelector<HTMLElement>("#titleText")!;
    this.artist = root.querySelector<HTMLElement>("#artistText")!;
    this.eqBars = root.querySelector<HTMLElement>("#eqBars")!;
    this.playIcon = root.querySelector<HTMLElement>("#playIcon")!;
    this.positionText = root.querySelector<HTMLElement>("#positionText")!;
    this.remainingText = root.querySelector<HTMLElement>("#remainingText")!;

    this.dsh = root.querySelector<HTMLElement>("#cardDsh")!;
    this.dshDot = root.querySelector<HTMLElement>("#cardDshDot")!;
    this.dshStage = root.querySelector<HTMLElement>("#cardDshStage")!;
    this.dshTool = root.querySelector<HTMLElement>("#cardDshTool")!;
    this.dshModel = root.querySelector<HTMLElement>("#cardDshModel")!;
    this.dshTokens = root.querySelector<HTMLElement>("#cardDshTokens")!;
    this.dshCost = root.querySelector<HTMLElement>("#cardDshCost")!;
    this.dshBalance = root.querySelector<HTMLElement>("#cardDshBalance")!;

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

  /** 没有媒体时收起媒体区块，卡片只留底部 dsh 面板（紧凑布局）。 */
  setMediaVisible(visible: boolean): void {
    this.root.classList.toggle("no-media", !visible);
  }

  /**
   * 写入卡片底部的 dsh 行。
   * `null` 表示没有 dsh：行高收起到 0，卡片高度由调用方同步缓动回基础值。
   */
  setDsh(snapshot: DshSnapshot | null): void {
    if (!snapshot) {
      this.dsh.classList.remove("on");
      return;
    }

    this.dshDot.className = `dsh-dot ${stageClass(snapshot)}`;
    this.dshStage.textContent = stageLabel(snapshot);

    // 工具名只在「调用工具 / 等待确认」阶段有意义，空闲时不要挂着上一次的名字
    const stage = stageKey(snapshot);
    const showTool = stage === "tool" || stage === "waiting";
    const tool = showTool ? snapshot.Tool?.trim() ?? "" : "";
    this.dshTool.hidden = tool.length === 0;
    this.dshTool.textContent = tool;

    this.dshModel.textContent = snapshot.Model;
    this.dshModel.hidden = snapshot.Model.trim().length === 0;

    const hasTokens = snapshot.TotalTokens > 0;
    this.dshTokens.hidden = !hasTokens;
    this.dshTokens.textContent = `${formatTokens(snapshot.TotalTokens)} tok`;

    const hasCost = snapshot.SessionCost > 0;
    this.dshCost.hidden = !hasCost;
    this.dshCost.textContent = formatCost(snapshot.SessionCost, snapshot.Currency);

    const hasBalance = showsBalance(snapshot);
    this.dshBalance.hidden = !hasBalance;
    this.dshBalance.textContent = hasBalance
      ? `余额 ${formatBalance(snapshot.Balance ?? 0, snapshot.BalanceCurrency)}`
      : "";

    this.dsh.classList.add("on");
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
