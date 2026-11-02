import type { DshSnapshot, DshStage } from "./types";

/**
 * DeepSeek Harness 展示层的小工具：阶段兜底文案、状态点、数字格式化。
 *
 * 只做「怎么显示」，不参与尺寸计算（尺寸在 `metrics.ts`）。
 */

const STAGE_LABELS: Record<DshStage, string> = {
  idle: "空闲",
  thinking: "思考中",
  tool: "调用工具",
  waiting: "等待确认",
  done: "已完成",
};

/** 未知阶段统一按 `idle` 处理，避免脏数据把样式打散。 */
export function stageKey(snapshot: DshSnapshot): DshStage {
  const stage = snapshot.Stage as string;
  return stage in STAGE_LABELS ? (stage as DshStage) : "idle";
}

/** 展示用阶段短词：优先用写入方给的文案。 */
export function stageLabel(snapshot: DshSnapshot | null): string {
  if (!snapshot) return "";
  const label = snapshot.StageLabel?.trim();
  return label ? label : STAGE_LABELS[stageKey(snapshot)];
}

/** 状态点的颜色 / 动效类名（`idle` / `thinking` / `tool` / `waiting` / `done`）。 */
export function stageClass(snapshot: DshSnapshot): DshStage {
  return stageKey(snapshot);
}

export function currencySymbol(currency: string): string {
  switch ((currency || "").toUpperCase()) {
    case "CNY":
    case "RMB":
      return "¥";
    case "USD":
      return "$";
    case "EUR":
      return "€";
    case "JPY":
      return "¥";
    default:
      return currency ? `${currency} ` : "";
  }
}

/** 会话 token：1.2k / 12.4k / 1.25M。 */
export function formatTokens(tokens: number): string {
  if (!Number.isFinite(tokens) || tokens <= 0) return "0";
  if (tokens < 1000) return `${Math.round(tokens)}`;
  if (tokens < 1_000_000) return `${(tokens / 1000).toFixed(1)}k`;
  return `${(tokens / 1_000_000).toFixed(2)}M`;
}

/** 会话成本：小于 1 时保留 4 位（分币级别），否则 2 位。 */
export function formatCost(cost: number, currency: string): string {
  const symbol = currencySymbol(currency);
  if (!Number.isFinite(cost) || cost <= 0) return `${symbol}0`;
  return cost < 1 ? `${symbol}${cost.toFixed(4)}` : `${symbol}${cost.toFixed(2)}`;
}

export function formatBalance(balance: number, currency: string): string {
  const symbol = currencySymbol(currency);
  if (!Number.isFinite(balance)) return "";
  return `${symbol}${balance.toFixed(2)}`;
}

/**
 * 只有官方登录 / 网页登录才展示余额。
 * 写入方没给余额（`Balance` 为 null）时整段不占位。
 */
export function showsBalance(snapshot: DshSnapshot): boolean {
  if (snapshot.Balance === null || !Number.isFinite(snapshot.Balance)) return false;
  const mode = (snapshot.AuthMode || "").toLowerCase();
  return mode === "official" || mode === "web";
}
