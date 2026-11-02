import { withAlpha } from "../common/color";
import {
  formatBalance,
  formatCost,
  formatTokens,
  showsBalance,
  stageClass,
  stageLabel,
} from "../common/dsh";
import { dshInnerMaxWidth } from "../common/metrics";
import type { DshSnapshot, MediaSnapshot } from "../common/types";

/**
 * 胶囊态：左侧媒体簇（18×18 缩略封面 + 四根跳动音柱），右侧 dsh 簇。
 *
 * - 没有媒体：媒体簇整体不占位（不是 `visibility: hidden`），dsh 簇在胶囊内居中；
 * - 没有 dsh：dsh 簇与分隔线一起隐藏，胶囊回到原来的纯媒体形态；
 * - 两者都有：媒体在左、dsh 在右，中间一条 1px 竖线。
 *
 * 未展开时不显示任何取色光晕——光晕只属于展开卡片。
 */
export class PillView {
  private readonly root: HTMLElement;
  private readonly media: HTMLElement;
  private readonly cover: HTMLElement;
  private readonly bars: HTMLElement;
  private readonly dshEl: HTMLElement;
  private readonly dshDot: HTMLElement;
  private readonly dshStage: HTMLElement;
  private readonly dshTokens: HTMLElement;
  private readonly dshCost: HTMLElement;
  private readonly dshBalance: HTMLElement;

  /** 数据层面是否存在成本 / 余额（折叠只在此基础上做减法）。 */
  private costAvailable = false;
  private balanceAvailable = false;

  constructor(root: HTMLElement) {
    this.root = root;
    this.media = root.querySelector<HTMLElement>("#pillMedia")!;
    this.cover = root.querySelector<HTMLElement>("#pillCover")!;
    this.bars = root.querySelector<HTMLElement>("#pillBars")!;
    this.dshEl = root.querySelector<HTMLElement>("#pillDsh")!;
    this.dshDot = root.querySelector<HTMLElement>("#pillDshDot")!;
    this.dshStage = root.querySelector<HTMLElement>("#pillDshStage")!;
    this.dshTokens = root.querySelector<HTMLElement>("#pillDshTokens")!;
    this.dshCost = root.querySelector<HTMLElement>("#pillDshCost")!;
    this.dshBalance = root.querySelector<HTMLElement>("#pillDshBalance")!;
  }

  setMedia(snapshot: MediaSnapshot | null): void {
    const artwork = snapshot?.ArtworkUrl ?? null;

    if (artwork) {
      this.cover.style.backgroundImage = `url("${artwork}")`;
      this.cover.hidden = false;
      this.bars.classList.remove("center");
    } else {
      this.cover.style.backgroundImage = "";
      this.cover.hidden = true;
      this.bars.classList.add("center");
    }

    // 没有媒体时整块媒体簇退出布局，胶囊只留给 dsh
    this.media.hidden = snapshot === null;
    this.bars.style.display = snapshot ? "flex" : "none";

    this.setPlaying(snapshot?.IsPlaying ?? false);
  }

  /**
   * 写入 dsh 内容并返回它在胶囊里的自然宽度（DIP，已按折叠降级收敛）。
   * 返回 0 表示当前不显示 dsh 簇。
   */
  setDsh(snapshot: DshSnapshot | null): number {
    if (!snapshot) {
      this.dshEl.hidden = true;
      this.root.classList.remove("dsh-only");
      return 0;
    }

    this.dshDot.className = `dsh-dot ${stageClass(snapshot)}`;
    this.dshStage.textContent = stageLabel(snapshot);

    // 会话刚开始时没有用量，别显示一个孤零零的 0
    this.dshTokens.hidden = snapshot.TotalTokens <= 0;
    this.dshTokens.textContent = formatTokens(snapshot.TotalTokens);

    this.costAvailable = snapshot.SessionCost > 0;
    this.dshCost.textContent = formatCost(snapshot.SessionCost, snapshot.Currency);

    this.balanceAvailable = showsBalance(snapshot);
    // 胶囊里只放金额本身，不加「余」字
    this.dshBalance.textContent = this.balanceAvailable
      ? formatBalance(snapshot.Balance ?? 0, snapshot.BalanceCurrency)
      : "";

    this.applyFolding(true, true);

    this.dshEl.hidden = false;
    this.root.classList.toggle("dsh-only", this.media.hidden);

    return this.measure();
  }

  /**
   * 测量 dsh 簇的自然宽度：先按全字段测，超宽就依次折叠「余额 → 成本」，
   * 保证阶段词与 token 永远可见。整个测量是同步读一次布局，无动画开销。
   */
  private measure(): number {
    const limit = dshInnerMaxWidth();

    this.applyFolding(true, true);
    this.dshEl.classList.add("measuring");

    let width = this.dshEl.offsetWidth;
    if (width > limit) {
      this.applyFolding(true, false);
      width = this.dshEl.offsetWidth;
    }
    if (width > limit) {
      this.applyFolding(false, false);
      width = this.dshEl.offsetWidth;
    }

    this.dshEl.classList.remove("measuring");
    return Math.min(width, limit);
  }

  /** 折叠只做减法：数据本身没有的字段永远不会被测量打开。 */
  private applyFolding(showCost: boolean, showBalance: boolean): void {
    this.dshCost.hidden = !(this.costAvailable && showCost);
    this.dshBalance.hidden = !(this.balanceAvailable && showBalance);
  }

  private setPlaying(playing: boolean): void {
    this.bars.classList.toggle("playing", playing);
    this.bars.classList.toggle("frozen", !playing);
  }
}

/**
 * 径向渐变：中心峰值 → 55% 处为峰值的 `midRatio` → 边缘全透明。
 * 三档渐变 stop（胶囊 midRatio = 0.32，卡片光斑 = 0.34）。
 */
export function radialGlow(color: string, peakOpacity: number, midRatio = 0.32): string {
  const middle = peakOpacity * midRatio;
  return `radial-gradient(circle closest-side, ${withAlpha(color, peakOpacity)} 0%, ${withAlpha(
    color,
    middle,
  )} 55%, ${withAlpha(color, 0)} 100%)`;
}
