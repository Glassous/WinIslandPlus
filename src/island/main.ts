import { api, events, on } from "../common/ipc";
import { metricsFor } from "../common/metrics";
import { currentPosition } from "../common/time";
import type { IslandMetrics, IslandSettings, MediaSnapshot } from "../common/types";
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
let expanded = false;
let bottomAnchored = false;
let dragEnabled = false;

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

  const span = Math.max(1, metrics.CardWidth - metrics.PillWidth);
  const t = Math.min(1, Math.max(0, (width - metrics.PillWidth) / span));

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

function renderMedia(snapshot: MediaSnapshot | null): void {
  media = snapshot;

  pill.setMedia(snapshot);
  card.update(snapshot);

  glow.setPalette(snapshot?.AccentColor ?? null, snapshot?.AccentColorAlt ?? null);
  glow.setActive(Boolean(snapshot?.ArtworkUrl) && expanded, snapshot?.IsPlaying ?? false);

  if (!snapshot && expanded) {
    setExpanded(false);
  }
}

function setExpanded(next: boolean, animate = true): void {
  if (next && !media) return;
  if (expanded === next) return;

  expanded = next;
  void api.setExpanded(next);

  cardEl.style.pointerEvents = next ? "auto" : "none";
  pillEl.style.pointerEvents = next ? "none" : "auto";

  glow.setActive(next && Boolean(media?.ArtworkUrl), media?.IsPlaying ?? false);

  const width = next ? metrics.CardWidth : metrics.PillWidth;
  const height = next ? metrics.CardHeight : metrics.PillHeight;
  const corner = next ? metrics.CardCorner : metrics.PillCorner;

  if (animate) {
    animator.animate(width, height, corner, next ? 300 : 240, next ? "out" : "inOut");
  } else {
    animator.snap(width, height, corner);
  }
}

function toggleExpanded(): void {
  if (!media) return;
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

  on<IslandSettings>(events.settingsChanged, (next) => {
    settings = next;
    const updated = metricsFor(next.Size, next.CustomPillWidth);
    const sizeChanged =
      updated.CardWidth !== metrics.CardWidth ||
      updated.PillWidth !== metrics.PillWidth ||
      updated.CardHeight !== metrics.CardHeight;

    applyMetrics(updated);

    const bottom = next.BottomAnchored || next.Placement === "BottomCenter";
    if (bottom !== bottomAnchored) {
      bottomAnchored = bottom;
      islandEl.classList.toggle("bottom", bottom);
    }

    if (!animator.isAnimating) {
      animator.snap(
        expanded ? updated.CardWidth : updated.PillWidth,
        expanded ? updated.CardHeight : updated.PillHeight,
        expanded ? updated.CardCorner : updated.PillCorner,
      );
    } else if (sizeChanged) {
      animator.snap(
        expanded ? updated.CardWidth : updated.PillWidth,
        expanded ? updated.CardHeight : updated.PillHeight,
        expanded ? updated.CardCorner : updated.PillCorner,
      );
    }
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
  bottomAnchored = boot.BottomAnchored || boot.Settings.Placement === "BottomCenter";
  islandEl.classList.toggle("bottom", bottomAnchored);

  applyMetrics(metricsFor(boot.Settings.Size, boot.Settings.CustomPillWidth));

  islandEl.style.opacity = "0";
  renderMedia(boot.Media ?? null);
  animator.snap(metrics.PillWidth, metrics.PillHeight, metrics.PillCorner);

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
