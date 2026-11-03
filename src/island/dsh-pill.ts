import {
  formatBalance,
  formatCost,
  formatTokens,
  showsBalance,
  stageClass,
  stageLabel,
} from "../common/dsh";
import { dshInnerMaxWidth } from "../common/metrics";
import type { DshSnapshot } from "../common/types";

/**
 * dsh 岛的胶囊层：状态点 + 阶段 + token /（成本）/（余额）。
 *
 * 宽度按内容自适应，超宽时依次折叠「余额 → 成本」，保证阶段词与 token 永远可见；
 * 折叠只做减法——数据本身没有的字段不会被测量打开。
 */
export class DshPillView {
  private readonly cluster: HTMLElement;
  private readonly dot: HTMLElement;
  private readonly stage: HTMLElement;
  private readonly tokens: HTMLElement;
  private readonly cost: HTMLElement;
  private readonly balance: HTMLElement;

  /** 数据层面是否存在成本 / 余额（折叠只在此基础上做减法）。 */
  private costAvailable = false;
  private balanceAvailable = false;

  constructor(actor: HTMLElement) {
    this.cluster = actor.querySelector<HTMLElement>("#pillDsh")!;
    this.dot = actor.querySelector<HTMLElement>("#pillDshDot")!;
    this.stage = actor.querySelector<HTMLElement>("#pillDshStage")!;
    this.tokens = actor.querySelector<HTMLElement>("#pillDshTokens")!;
    this.cost = actor.querySelector<HTMLElement>("#pillDshCost")!;
    this.balance = actor.querySelector<HTMLElement>("#pillDshBalance")!;
  }

  /**
   * 写入 dsh 内容并返回胶囊内容区的自然宽度（DIP，已按折叠降级收敛）。
   * 返回 0 表示当前不显示 dsh 岛。
   */
  setDsh(snapshot: DshSnapshot | null): number {
    if (!snapshot) return 0;

    this.dot.className = `dsh-dot ${stageClass(snapshot)}`;
    this.stage.textContent = stageLabel(snapshot);

    // 会话刚开始时没有用量，别显示一个孤零零的 0
    this.tokens.hidden = snapshot.TotalTokens <= 0;
    this.tokens.textContent = formatTokens(snapshot.TotalTokens);

    this.costAvailable = snapshot.SessionCost > 0;
    this.cost.textContent = formatCost(snapshot.SessionCost, snapshot.Currency);

    this.balanceAvailable = showsBalance(snapshot);
    // 胶囊里只放金额本身，不加「余」字
    this.balance.textContent = this.balanceAvailable
      ? formatBalance(snapshot.Balance ?? 0, snapshot.BalanceCurrency)
      : "";

    return this.measure();
  }

  /**
   * 测量自然宽度：先按全字段测，超宽就依次折叠「余额 → 成本」。
   * 整个测量是同步读一次布局，没有动画开销；只在快照签名变化时调用。
   */
  private measure(): number {
    const limit = dshInnerMaxWidth();

    this.applyFolding(true, true);
    this.cluster.classList.add("measuring");

    let width = this.cluster.offsetWidth;
    if (width > limit) {
      this.applyFolding(true, false);
      width = this.cluster.offsetWidth;
    }
    if (width > limit) {
      this.applyFolding(false, false);
      width = this.cluster.offsetWidth;
    }

    this.cluster.classList.remove("measuring");
    return Math.min(width, limit);
  }

  /** 折叠只做减法：数据本身没有的字段永远不会被测量打开。 */
  private applyFolding(showCost: boolean, showBalance: boolean): void {
    this.cost.hidden = !(this.costAvailable && showCost);
    this.balance.hidden = !(this.balanceAvailable && showBalance);
  }
}
