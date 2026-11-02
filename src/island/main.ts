import { api, events, on } from "../common/ipc";
import { cardHeightFor, metricsFor, pillWidthFor } from "../common/metrics";
import { currentPosition } from "../common/time";
import type {
  DshSnapshot,
  IslandMetrics,
  IslandSettings,
  MediaSnapshot,
} from "../common/types";
import { IslandAnimator } from "./animator";
import { MediaCard } from "./card";
import { CardGlow } from "./glow";
import { PillView } from "./pill";

const islandEl = document.getElementById("island")!;
const pillEl = document.getElementById("pillView")!;
const cardEl = document.getElementById("cardView")!;

let settings: IslandSettings | null = null;
let metrics: IslandMetrics = metricsFor("Medium", 68);
let media: MediaSnapshot | null = null;
let dsh: DshSnapshot | null = null;
let dshEnabled = true;
let expanded = false;
let bottomAnchored = false;
let dragEnabled = false;

/**
 * 当前布局目标值。胶囊宽度与卡片高度会随 dsh 的出现 / 消失变化，
 * 单独抽出来，`onFrame` 里的透明度交叉淡化比才不会写死基础尺寸。
 */
const layout = {
  pillW: metrics.PillWidth,
  cardW: metrics.CardWidth,
  cardH: metrics.CardHeight,
};

const animator = new IslandAnimator();
animator.onFrame = onFrame;

const pill = new PillView(pillEl);
const glow = new CardGlow(document.getElementById("glowLayer")!);
const card = new MediaCard(cardEl, {
  onPlayPause: () => void api.togglePlayPause(),
  onPrevious: () => void api.previous(),
  onNext: () => void api.next(),
  onSeek: (fraction) => {
    const duration = media?.DurationMs ?? 0;
    if (duration <= 0) return;
    void api.seek(duration * fraction);
  },
  onUserActed: () => {
    // 用户主动操作后 2 秒内不因"暂停"自动收起（由后端统一判断，这里只上报时间戳并刷新进度）
    void api.notifyUserActed();
    if (media && media.DurationMs > 0) {
      card.setProgress(currentPosition(media), media.DurationMs);
    }
  },
});

// ---------------- 右键屏蔽 ----------------
// 岛窗永不激活，WebView2 的默认上下文菜单会盖在胶囊 / 卡片上，
// 这里直接吞掉右键菜单事件（覆盖胶囊态与展开态，同一个 DOM）。
document.addEventListener(
  "contextmenu",
  (event) => {
    event.preventDefault();
    event.stopPropagation();
  },
  true,
);

// ---------------- 形状同步 ----------------

let lastShape = { left: -1, top: -1, width: -1, height: -1, radius: -1 };

function updateShape(widthDip: number, heightDip: number, cornerDip: number): void {
  const scale = window.devicePixelRatio || 1;
  const windowWidth = Math.max(1, Math.round(window.innerWidth * scale));
  const windowHeight = Math.max(1, Math.round(window.innerHeight * scale));

  const width = Math.max(1, Math.round(widthDip * scale));
  const height = Math.max(1, Math.round(heightDip * scale));
  const left = Math.max(0, Math.round((windowWidth - width) / 2));
  const top = bottomAnchored ? Math.max(0, windowHeight - height) : 0;
  const radius = Math.max(1, Math.round(cornerDip * scale));

  if (
    Math.abs(left - lastShape.left) < 1 &&
    Math.abs(top - lastShape.top) < 1 &&
    Math.abs(width - lastShape.width) < 1 &&
    Math.abs(height - lastShape.height) < 1 &&
    Math.abs(radius - lastShape.radius) < 1
  ) {
    return;
  }

  lastShape = { left, top, width, height, radius };
  void api.setShape({ Left: left, Top: top, Width: width, Height: height, Radius: radius });
}

// ---------------- 动画帧 ----------------

function onFrame(width: number, height: number, corner: number): void {
  islandEl.style.width = `${width}px`;
  islandEl.style.height = `${height}px`;
  islandEl.style.borderRadius = `${corner}px`;

  // 交叉淡化的分母用「当前布局」而不是基础尺寸：胶囊被 dsh 拉长后比例依然正确
  const span = Math.max(1, layout.cardW - layout.pillW);
  const t = Math.min(1, Math.max(0, (width - layout.pillW) / span));

  const pillOpacity = Math.min(1, Math.max(0, 1 - t * 2.2));
  const cardOpacity = Math.min(1, Math.max(0, (t - 0.45) / 0.55));

  pillEl.style.opacity = `${pillOpacity}`;
  pillEl.style.visibility = pillOpacity <= 0.01 ? "hidden" : "visible";

  // 卡片跟随实时宽高一起生长：内容在生长的容器里重新布局，而不是被硬裁开
  cardEl.style.width = `${width}px`;
  cardEl.style.height = `${height}px`;
  cardEl.style.opacity = `${cardOpacity}`;
  cardEl.style.visibility = cardOpacity <= 0.01 ? "hidden" : "visible";

  updateShape(width, height, corner);
}

// ---------------- 渲染 ----------------

function applyMetrics(next: IslandMetrics): void {
  metrics = next;
  const root = document.documentElement.style;
  root.setProperty("--pill-w", `${next.PillWidth}px`);
  root.setProperty("--pill-h", `${next.PillHeight}px`);
  root.setProperty("--card-w", `${next.CardWidth}px`);
  root.setProperty("--card-h", `${next.CardHeight}px`);
}

/** dsh 视图里参与展示的字段；只有它变化才重新测量宽度。 */
function dshSignature(snapshot: DshSnapshot): string {
  return [
    snapshot.Stage,
    snapshot.StageLabel,
    snapshot.Model,
    snapshot.Tool,
    snapshot.AuthMode,
    snapshot.TotalTokens,
    snapshot.SessionCost,
    snapshot.Currency,
    snapshot.Balance,
    snapshot.BalanceCurrency,
  ].join("\u001f");
}

let currentDshSignature = "";
let currentMediaPresence: boolean | null = null;
let dshInnerWidth = 0;

/**
 * 把 dsh 内容写进胶囊与卡片，并返回胶囊里 dsh 簇的自然宽度。
 * 用签名缓存避免每次媒体进度变化都触发一次强制布局测量。
 */
function syncDshViews(): number {
  const hasMedia = media !== null;
  const active = dshEnabled ? dsh : null;
  const signature = active ? dshSignature(active) : "";

  if (signature === currentDshSignature && hasMedia === currentMediaPresence) {
    return dshInnerWidth;
  }

  currentDshSignature = signature;
  currentMediaPresence = hasMedia;

  dshInnerWidth = pill.setDsh(active);
  card.setDsh(active);
  card.setMediaVisible(hasMedia);

  return dshInnerWidth;
}

/** 重新解算布局目标：胶囊宽度 / 卡片高度都可能因 dsh 而变化。 */
function rebuildLayout(animate: boolean, durationMs = 280): void {
  const inner = syncDshViews();
  const hasMedia = media !== null;

  layout.cardW = metrics.CardWidth;
  layout.pillW = pillWidthFor(metrics, hasMedia, inner);
  layout.cardH = cardHeightFor(metrics, hasMedia, inner > 0);

  // 光晕铺满整张卡片：卡片因 dsh 行变高时光斑也要跟着摊开
  glow.setSize(layout.cardW, layout.cardH);

  if (expanded) {
    animateTo(layout.cardW, layout.cardH, metrics.CardCorner, animate, durationMs);
  } else {
    animateTo(layout.pillW, metrics.PillHeight, metrics.PillCorner, animate, 260);
  }
}

/** 目标值与当前值一致时不动画，避免无谓的打断。 */
function animateTo(
  width: number,
  height: number,
  corner: number,
  animate: boolean,
  durationMs: number,
): void {
  if (!animate || animator.currentWidth <= 0) {
    animator.snap(width, height, corner);
    return;
  }

  const steady =
    Math.abs(animator.currentWidth - width) < 0.5 &&
    Math.abs(animator.currentHeight - height) < 0.5 &&
    Math.abs(animator.currentCorner - corner) < 0.5;
  if (steady) return;

  animator.animate(width, height, corner, durationMs, "out");
}

function renderMedia(snapshot: MediaSnapshot | null): void {
  media = snapshot;

  pill.setMedia(snapshot);
  card.update(snapshot);

  glow.setPalette(snapshot?.AccentColor ?? null, snapshot?.AccentColorAlt ?? null);
  glow.setActive(Boolean(snapshot?.ArtworkUrl) && expanded, snapshot?.IsPlaying ?? false);

  rebuildLayout(true);

  // 媒体消失：只有在没有 dsh 可展示时才自动收起
  if (!snapshot && expanded && !hasDshContent()) {
    setExpanded(false);
  }
}

function hasDshContent(): boolean {
  return dshEnabled && dsh !== null;
}

function setExpanded(next: boolean, animate = true): void {
  if (next && !media && !hasDshContent()) return;
  if (expanded === next) return;

  expanded = next;
  void api.setExpanded(next);

  cardEl.style.pointerEvents = next ? "auto" : "none";
  pillEl.style.pointerEvents = next ? "none" : "auto";

  glow.setActive(next && Boolean(media?.ArtworkUrl), media?.IsPlaying ?? false);

  const width = next ? layout.cardW : layout.pillW;
  const height = next ? layout.cardH : metrics.PillHeight;
  const corner = next ? metrics.CardCorner : metrics.PillCorner;

  if (animate) {
    animator.animate(width, height, corner, next ? 300 : 240, next ? "out" : "inOut");
  } else {
    animator.snap(width, height, corner);
  }
}

function toggleExpanded(): void {
  if (!media && !hasDshContent()) return;
  // 设置窗口打开期间岛体是"可拖动"状态，点击不再切换
  if (dragEnabled) return;
  setExpanded(!expanded);
}

// ---------------- 输入 ----------------

let pointerDown = false;

islandEl.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  pointerDown = true;
});

islandEl.addEventListener("pointerup", (event) => {
  if (event.button !== 0 || !pointerDown) return;
  pointerDown = false;
  toggleExpanded();
});

islandEl.addEventListener("pointercancel", () => {
  pointerDown = false;
});

// ---------------- 事件订阅 ----------------

function bindEvents(): void {
  on<MediaSnapshot | null>(events.mediaChanged, (snapshot) => {
    renderMedia(snapshot ?? null);
  });

  on<DshSnapshot | null>(events.dshChanged, (snapshot) => {
    dsh = snapshot ?? null;
    rebuildLayout(true);

    if (!dsh && expanded && !media) {
      setExpanded(false);
    }
  });

  on<IslandSettings>(events.settingsChanged, (next) => {
    settings = next;
    dshEnabled = next.DshEnabled !== false;

    const updated = metricsFor(next.Size, next.CustomPillWidth);
    const sizeChanged =
      updated.PillWidth !== metrics.PillWidth ||
      updated.CardWidth !== metrics.CardWidth ||
      updated.CardHeight !== metrics.CardHeight ||
      updated.PillCorner !== metrics.PillCorner ||
      updated.CardCorner !== metrics.CardCorner;

    applyMetrics(updated);

    const bottom = next.BottomAnchored || next.Placement === "BottomCenter";
    if (bottom !== bottomAnchored) {
      bottomAnchored = bottom;
      islandEl.classList.toggle("bottom", bottom);
    }

    // 尺寸档位变化直接落位；只有 dsh 引起的宽高变化才走缓动
    rebuildLayout(!sizeChanged);
  });

  on<string>(events.collapse, () => setExpanded(false));

  on<boolean>(events.visibility, (visible) => {
    if (!visible) {
      setExpanded(false, false);
    }
  });

  on<boolean>(events.openSettings, (open) => {
    dragEnabled = open;
  });
}

// ---------------- 启动 ----------------

async function init(): Promise<void> {
  const boot = await api.bootstrap();

  settings = boot.Settings;
  dshEnabled = boot.Settings.DshEnabled !== false;
  dsh = boot.Dsh ?? null;
  bottomAnchored = boot.BottomAnchored || boot.Settings.Placement === "BottomCenter";
  islandEl.classList.toggle("bottom", bottomAnchored);

  applyMetrics(metricsFor(boot.Settings.Size, boot.Settings.CustomPillWidth));

  islandEl.style.opacity = "0";
  // 首帧直接用 renderMedia 走一遍完整布局（animator 尚未有当前值，会走 snap）
  renderMedia(boot.Media ?? null);

  bindEvents();
  startProgressLoop();

  // 首帧样式已经写进 DOM：直接点亮并请求显示窗口。
  // 注意不能用 requestAnimationFrame 等待——窗口在显示前不会调度动画帧，会死锁。
  islandEl.style.opacity = "1";
  await api.islandReady();
}

function startProgressLoop(): void {
  window.setInterval(() => {
    if (!expanded || !media || media.DurationMs <= 0) return;
    card.setProgress(currentPosition(media), media.DurationMs);
  }, 80);
}

void init().catch((error) => {
  void api.log("error", `island 初始化失败: ${String(error)}`);
});
