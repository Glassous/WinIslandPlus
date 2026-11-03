import {
  formatBalance,
  formatCost,
  formatTokens,
  showsBalance,
  stageClass,
  stageKey,
  stageLabel,
} from "../common/dsh";
import type { DshSnapshot } from "../common/types";

/**
 * dsh 岛的展开卡片：图标 + 阶段行（含当前工具名）与数据行（token / 成本 / 余额 / 模型）。
 *
 * 视觉沿用原先「无媒体时的紧凑 dsh 面板」，但现在是独立的一张卡，
 * 不再附在媒体控件下方，也不再参与卡片高度计算。
 */
export class DshCardView {
  private readonly dot: HTMLElement;
  private readonly stage: HTMLElement;
  private readonly tool: HTMLElement;
  private readonly model: HTMLElement;
  private readonly tokens: HTMLElement;
  private readonly cost: HTMLElement;
  private readonly balance: HTMLElement;

  constructor(actor: HTMLElement) {
    this.dot = actor.querySelector<HTMLElement>("#dshCardDot")!;
    this.stage = actor.querySelector<HTMLElement>("#dshCardStage")!;
    this.tool = actor.querySelector<HTMLElement>("#dshCardTool")!;
    this.model = actor.querySelector<HTMLElement>("#dshCardModel")!;
    this.tokens = actor.querySelector<HTMLElement>("#dshCardTokens")!;
    this.cost = actor.querySelector<HTMLElement>("#dshCardCost")!;
    this.balance = actor.querySelector<HTMLElement>("#dshCardBalance")!;
  }

  /** `null` 表示当前没有 dsh 内容（卡片由调用方收起）。 */
  setDsh(snapshot: DshSnapshot | null): void {
    if (!snapshot) return;

    this.dot.className = `dsh-dot ${stageClass(snapshot)}`;
    this.stage.textContent = stageLabel(snapshot);

    // 工具名只在「调用工具 / 等待确认」阶段有意义，空闲时不要挂着上一次的名字
    const stage = stageKey(snapshot);
    const showTool = stage === "tool" || stage === "waiting";
    const tool = showTool ? snapshot.Tool?.trim() ?? "" : "";
    this.tool.hidden = tool.length === 0;
    this.tool.textContent = tool;

    this.model.textContent = snapshot.Model;
    this.model.hidden = snapshot.Model.trim().length === 0;

    const hasTokens = snapshot.TotalTokens > 0;
    this.tokens.hidden = !hasTokens;
    this.tokens.textContent = `${formatTokens(snapshot.TotalTokens)} tok`;

    const hasCost = snapshot.SessionCost > 0;
    this.cost.hidden = !hasCost;
    this.cost.textContent = formatCost(snapshot.SessionCost, snapshot.Currency);

    const hasBalance = showsBalance(snapshot);
    this.balance.hidden = !hasBalance;
    this.balance.textContent = hasBalance
      ? `余额 ${formatBalance(snapshot.Balance ?? 0, snapshot.BalanceCurrency)}`
      : "";
  }
}
