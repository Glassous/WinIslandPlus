import { api, events, on } from "../common/ipc";
import {
  DSH_ONLY_CARD_HEIGHT,
  PILL_GAP,
  dshPillWidth,
  metricsFor,
  pillLyricMaxWidth,
  rowOffsets,
} from "../common/metrics";
import { theme } from "../common/theme";
import { trackKeyOf } from "../common/time";
import type {
  DshSnapshot,
  IslandMetrics,
  IslandSettings,
  IslandSide,
  LyricsSnapshot,
  MediaSnapshot,
  PillRightStyle,
  PositionPayload,
  ShapeRect,
} from "../common/types";
import { type ActorGeometry, type EasingKind, GeometryTween } from "./animator";
import { MediaCard } from "./card";
import { PlaybackClock } from "./clock";
import { DshCardView } from "./dsh-card";
import { DshPillView } from "./dsh-pill";
import { CardGlow } from "./glow";
import { MediaPillView } from "./pill";

const stageEl = document.getElementById("stage")!;
const mediaActorEl = document.getElementById("mediaActor")!;
const dshActorEl = document.getElementById("dshActor")!;
const mediaPillLayer = document.getElementById("mediaPillLayer")!;
const mediaCardLayer = document.getElementById("mediaCardLayer")!;
const dshPillLayer = document.getElementById("dshPillLayer")!;
const dshCardLayer = document.getElementById("dshCardLayer")!;

let metrics: IslandMetrics = metricsFor("Medium", 68);
let media: MediaSnapshot | null = null;
let dsh: DshSnapshot | null = null;
let dshEnabled = true;
/** 当前展开的岛；同一时刻最多一个。 */
let expanded: IslandSide | null = null;
let bottomAnchored = false;
let dragEnabled = false;
/** 胶囊右侧样式：频谱 / 进度圈 / 歌词，同一时刻只生效一个。 */
let rightStyle: PillRightStyle = "Spectrum";
/** dsh 胶囊内容区的自然宽度（DIP），0 表示没有 dsh。 */
let dshInnerWidth = 0;

/** 歌词换句时的胶囊伸缩时长（毫秒）：比展开动画短一点，换句才跟得上。 */
const LYRIC_WIDTH_MS = 260;

const mediaPill = new MediaPillView(mediaActorEl);
const dshPill = new DshPillView(dshActorEl);
const dshCard = new DshCardView(dshActorEl);
const glow = new CardGlow(document.getElementById("glowLayer")!);

/**
 * 播放时钟：歌词与进度都读它，不再直接线性外推快照。
 * 后端约每 350 ms 推一次真值，时钟只「平滑吸收」小偏差，因此单次异常采样
 * 不会把整首歌的歌词顶偏（详见 clock.ts）。
 */
const clock = new PlaybackClock();

const mediaCard = new MediaCard(document.getElementById("mediaCard")!, {
  onPlayPause: () => {
    // 图标先乐观翻转（后端真实状态由快速轮询在百毫秒内送达并校正）
    mediaCard.togglePlayingOptimistic();
    void api.togglePlayPause();
  },
  onPrevious: () => void api.previous(),
  onNext: () => void api.next(),
  onSeek: (fraction) => {
    const duration = media?.DurationMs ?? 0;
    if (duration <= 0 || !media) return;

    const position = duration * fraction;
    // 乐观推进本地时钟与快照：真实快照要等媒体线程下一次采样才回来，
    // 这中间若继续按旧位置渲染，进度条与歌词会先弹回旧位置再跳过来。
    clock.seek(position);
    media = { ...media, PositionMs: position, PositionStampMs: Date.now() };
    mediaCard.setProgress(position, duration);
    mediaCard.tickLyrics(position);
    void api.seek(position);
  },
  onUserActed: () => {
    // 用户主动操作后 2 秒内不因"暂停"自动收起（由后端统一判断，这里只上报时间戳并刷新进度）
    void api.notifyUserActed();
    if (media && media.DurationMs > 0) {
      const position = clock.position();
      mediaCard.setProgress(position, media.DurationMs);
      mediaCard.tickLyrics(position);
    }
  },
  // 卡片与胶囊共用同一条歌词时间轴：换句时把新的一句交给胶囊
  onLyricLine: (text) => {
    // 胶囊宽度跟着这句歌词走，且只向右生长（左沿固定），所以换句只需要重新解算一次几何
    if (!mediaPill.setLyric(text)) return;
    relayout(true, LYRIC_WIDTH_MS, "out");
  },
});

// ---------------- 双岛几何 ----------------

/** 一帧里两个岛面的目标位置与可见性。 */
interface LayoutTarget {
  media: ActorGeometry;
  dsh: ActorGeometry;
  mediaVisible: boolean;
  dshVisible: boolean;
  /** 媒体岛是否收成正圆（两个岛并存且媒体未展开）。 */
  mediaCircle: boolean;
}

const mediaTween = new GeometryTween();
const dshTween = new GeometryTween();

/** 当前布局目标（形状推送与命中判定都以它为准）。 */
let target: LayoutTarget = {
  media: { x: 0, y: 0, width: 0, height: 0, radius: 0, morph: 0 },
  dsh: { x: 0, y: 0, width: 0, height: 0, radius: 0, morph: 0 },
  mediaVisible: false,
  dshVisible: false,
  mediaCircle: false,
};

function hasDshContent(): boolean {
  return dshEnabled && dsh !== null;
}

/**
 * 媒体胶囊的几何（歌词模式下的伸缩都收敛在这里）。
 *
 * 三种样式的胶囊位置完全一致：x 恒为标准胶囊居中时的左沿，所以封面（以及
 * 胶囊里的一切）在切换样式、换句时都不会左右移动。
 * 歌词样式只改宽度：按这句歌词解算，宽度走补间**只向右生长**，
 * 右沿最多伸到胶囊行右端（胶囊行就是卡片宽度），超出的部分由胶囊裁掉。
 */
function mediaPillGeometry(m: IslandMetrics, pillY: number): ActorGeometry {
  // 左沿固定：与频谱 / 进度圈模式下的标准胶囊同一位置
  const left = (m.CardWidth - m.PillWidth) / 2;

  if (rightStyle !== "Lyrics") {
    return {
      x: left,
      y: pillY,
      width: m.PillWidth,
      height: m.PillHeight,
      radius: m.PillCorner,
      morph: 0,
    };
  }

  const width = Math.max(
    m.PillWidth,
    mediaPill.lyricPillWidth(m.PillWidth, pillLyricMaxWidth(m.CardWidth, m.PillWidth)),
  );

  return {
    x: left,
    y: pillY,
    width,
    height: m.PillHeight,
    radius: m.PillCorner,
    morph: 0,
  };
}

/**
 * 解算两个岛面的目标几何（DIP，相对岛窗客户区）。
 *
 * 窗口按行切分：胶囊行贴屏幕边缘，卡片行在它内侧，中间隔 `ROW_GAP`。
 * - 只有一个岛：胶囊 / 卡片都居中，与原来的单岛行为一致；
 * - 两个都缩着：dsh 胶囊在左、媒体正圆在右，整组居中；
 * - 展开：被展开者变成卡片长在卡片行，另一个留在胶囊行并居中；
 *   此时胶囊行只剩一个岛，媒体回到普通胶囊形态（封面在左、右侧元素在右）。
 * - 歌词样式：胶囊左沿固定、宽度按整句歌词**只向右**生长（见 `mediaPillGeometry`）。
 */
function computeLayout(): LayoutTarget {
  const m = metrics;
  const rows = rowOffsets(m, bottomAnchored);
  const pillH = m.PillHeight;
  const cardW = m.CardWidth;
  const hasMedia = media !== null;
  const hasDsh = hasDshContent();
  const dshW = hasDsh ? Math.max(1, dshPillWidth(dshInnerWidth)) : 0;
  const circle = pillH;

  // 缺省：两个岛都以胶囊形态居中（只有一个时即为「保持现状」）
  const mediaPill = mediaPillGeometry(m, rows.pillY);
  const dshPillTarget: ActorGeometry = {
    x: (cardW - dshW) / 2,
    y: rows.pillY,
    width: dshW,
    height: pillH,
    radius: m.PillCorner,
    morph: 0,
  };
  const mediaCircleTarget: ActorGeometry = {
    x: (cardW - circle) / 2,
    y: rows.pillY,
    width: circle,
    height: circle,
    radius: circle / 2,
    morph: 0,
  };

  let mediaTarget = mediaPill;
  let dshTarget = dshPillTarget;

  // 两个岛并存且都没展开：dsh 在左、媒体正圆在右
  if (hasMedia && hasDsh && expanded === null) {
    const groupW = dshW + PILL_GAP + circle;
    const startX = (cardW - groupW) / 2;
    dshTarget = { ...dshPillTarget, x: startX };
    mediaTarget = { ...mediaCircleTarget, x: startX + dshW + PILL_GAP };
  }

  if (expanded === "media" && hasMedia) {
    mediaTarget = {
      x: 0,
      y: rows.cardY,
      width: cardW,
      height: m.CardHeight,
      radius: m.CardCorner,
      morph: 1,
    };
  } else if (expanded === "dsh" && hasDsh) {
    dshTarget = {
      x: 0,
      // 贴底时卡片行在上方，dsh 卡靠行的下沿（紧挨胶囊行）
      y: rows.cardY + (bottomAnchored ? m.CardHeight - DSH_ONLY_CARD_HEIGHT : 0),
      width: cardW,
      height: DSH_ONLY_CARD_HEIGHT,
      radius: m.CardCorner,
      morph: 1,
    };
    // 胶囊行只剩媒体岛：回到「只有音乐播放时」的胶囊（封面在左、频谱在右），
    // 已经由缺省的 mediaPill 居中，不需要再摆成正圆。
  }

  return {
    media: mediaTarget,
    dsh: dshTarget,
    // 没有媒体也没有 dsh 时仍然显示一枚空胶囊（常驻行为）
    mediaVisible: hasMedia || !hasDsh,
    dshVisible: hasDsh,
    // 只有两岛同排（都没展开）时媒体才收成正圆
    mediaCircle: hasMedia && hasDsh && expanded === null,
  };
}

// ---------------- 补间与渲染 ----------------

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

/** 把一条几何写进岛面：位置用 transform，尺寸 / 圆角直接写，两层按 morph 交叉淡化。 */
function paintActor(
  el: HTMLElement,
  pillLayer: HTMLElement,
  cardLayer: HTMLElement,
  geometry: ActorGeometry,
): void {
  el.style.transform = `translate3d(${geometry.x}px, ${geometry.y}px, 0)`;
  el.style.width = `${Math.max(0, geometry.width)}px`;
  el.style.height = `${Math.max(0, geometry.height)}px`;
  el.style.borderRadius = `${Math.max(0, geometry.radius)}px`;

  // 胶囊层先淡出、卡片层后淡入：中途不会出现两层同时可见的糊影
  const pillOpacity = clamp01(1 - geometry.morph * 2.2);
  const cardOpacity = clamp01((geometry.morph - 0.45) / 0.55);

  pillLayer.style.opacity = `${pillOpacity}`;
  pillLayer.style.visibility = pillOpacity <= 0.01 ? "hidden" : "visible";
  cardLayer.style.opacity = `${cardOpacity}`;
  cardLayer.style.visibility = cardOpacity <= 0.01 ? "hidden" : "visible";
}

mediaTween.onFrame = (geometry) =>
  paintActor(mediaActorEl, mediaPillLayer, mediaCardLayer, geometry);
dshTween.onFrame = (geometry) =>
  paintActor(dshActorEl, dshPillLayer, dshCardLayer, geometry);

/** 后端命中判定用的形状（每帧最多推两块，值不变就不发）。 */
let lastShapeKey = "";
let frameHandle = 0;

function toShapeRect(geometry: ActorGeometry): ShapeRect {
  const scale = window.devicePixelRatio || 1;
  return {
    Left: Math.round(geometry.x * scale),
    Top: Math.round(geometry.y * scale),
    Width: Math.max(1, Math.round(geometry.width * scale)),
    Height: Math.max(1, Math.round(geometry.height * scale)),
    Radius: Math.max(0, Math.round(geometry.radius * scale)),
  };
}

/** 展开态下胶囊行（整条卡片宽度）也算岛内，避免点击穿透与误判的「岛外点击」。 */
function pillRowShape(): ShapeRect {
  const rows = rowOffsets(metrics, bottomAnchored);
  return toShapeRect({
    x: 0,
    y: rows.pillY,
    width: metrics.CardWidth,
    height: metrics.PillHeight,
    radius: metrics.PillCorner,
    morph: 0,
  });
}

function flushShapes(): void {
  const shapes: ShapeRect[] = [];
  if (target.mediaVisible) shapes.push(toShapeRect(mediaTween.current));
  if (target.dshVisible) shapes.push(toShapeRect(dshTween.current));
  // 展开时胶囊行仍然是岛体的一部分：不补这块形状的话，卡片上方那条横带会被判成「岛外」，
  // 点击会穿透到下面的窗口、并把卡片当成「岛外点击」收起来。
  if (expanded !== null) shapes.push(pillRowShape());

  const key = shapes
    .map((r) => `${r.Left},${r.Top},${r.Width},${r.Height},${r.Radius}`)
    .join("|");
  if (key === lastShapeKey) return;

  lastShapeKey = key;
  void api.setShapes(shapes);
}

/** 静止状态下的整体重绘（snap 之后调用，保证形状与 DOM 一致）。 */
function paintAll(): void {
  paintActor(mediaActorEl, mediaPillLayer, mediaCardLayer, mediaTween.current);
  paintActor(dshActorEl, dshPillLayer, dshCardLayer, dshTween.current);
  flushShapes();
}

/** 两个岛面共用一条帧循环：同一时间戳、形状只推一次。 */
function kick(): void {
  if (frameHandle) return;
  frameHandle = requestAnimationFrame(stepFrame);
}

function stepFrame(now: number): void {
  frameHandle = 0;
  const mediaActive = mediaTween.step(now);
  const dshActive = dshTween.step(now);
  flushShapes();
  if (mediaActive || dshActive) kick();
}

function applyActorTarget(
  tween: GeometryTween,
  el: HTMLElement,
  geometry: ActorGeometry,
  visible: boolean,
  animate: boolean,
  durationMs: number,
  easing: EasingKind,
): void {
  if (!visible) {
    el.classList.remove("on");
    return;
  }

  // 刚出现的岛面直接落位再淡入，避免从上一个位置的残影滑过来
  const appearing = !el.classList.contains("on");
  if (!animate || appearing || !tween.initialized) {
    tween.snap(geometry);
  } else {
    tween.animate(geometry, durationMs, easing);
  }
  el.classList.add("on");
}

/**
 * 重新解算并下发两个岛面的目标几何。
 * 切换展开对象时两个岛面在同一调用里各自补间，天然同步。
 */
function relayout(
  animate: boolean,
  durationMs: number = theme.expandMs,
  easing: EasingKind = "out",
): void {
  target = computeLayout();
  mediaPill.setCircle(target.mediaCircle);

  applyActorTarget(
    mediaTween,
    mediaActorEl,
    target.media,
    target.mediaVisible,
    animate,
    durationMs,
    easing,
  );
  applyActorTarget(
    dshTween,
    dshActorEl,
    target.dsh,
    target.dshVisible,
    animate,
    durationMs,
    easing,
  );

  if (mediaTween.isAnimating || dshTween.isAnimating) {
    kick();
  } else {
    paintAll();
  }
}

// ---------------- 内容同步 ----------------

function renderMedia(snapshot: MediaSnapshot | null): void {
  media = snapshot;

  if (snapshot) {
    // 换歌 / 首次出现媒体：时钟按快照落位，卡片首帧的位置就是对的
    clock.sync(
      trackKeyOf(snapshot),
      snapshot.PositionMs,
      snapshot.PositionStampMs,
      snapshot.IsPlaying,
      snapshot.DurationMs,
    );
  } else {
    clock.clear();
  }

  mediaPill.setMedia(snapshot);
  mediaCard.update(snapshot, clock.position());
  glow.setPalette(snapshot?.AccentColor ?? null, snapshot?.AccentColorAlt ?? null);
  glow.setActive(expanded === "media" && Boolean(snapshot?.ArtworkUrl));

  // 媒体没了：媒体卡必须收起；dsh 岛若还在则继续留着
  if (!snapshot && expanded === "media") {
    setExpanded(null);
    return;
  }

  relayout(true);
}

function syncDshContent(): void {
  const active = hasDshContent() ? dsh : null;
  dshInnerWidth = dshPill.setDsh(active);
  dshCard.setDsh(active);
}

/**
 * 切换胶囊右侧样式（频谱 / 进度圈 / 歌词，互斥）。
 * 只写状态与 DOM；重新布局交给调用方（设置窗口的改动最后都会走一次 relayout）。
 */
function applyRightStyle(style: PillRightStyle): void {
  const next = style ?? "Spectrum";
  if (next === rightStyle) return;

  rightStyle = next;
  mediaPill.setRightStyle(next);
}

function setExpanded(next: IslandSide | null, animate = true): void {
  if (next === "media" && !media) return;
  if (next === "dsh" && !hasDshContent()) return;
  if (expanded === next) return;

  const wasAnyExpanded = expanded !== null;
  expanded = next;

  // 只在「有 ↔ 无展开」时通知后端：切换岛不重置前台窗口基线
  if (wasAnyExpanded !== (next !== null)) {
    void api.setExpanded(next !== null);
  }

  glow.setActive(next === "media" && Boolean(media?.ArtworkUrl));

  relayout(
    animate,
    next === null ? theme.collapseMs : theme.expandMs,
    next === null ? "inOut" : "out",
  );
}

function toggleSide(side: IslandSide): void {
  // 设置窗口打开期间岛体是"可拖动"状态，点击不再切换
  if (dragEnabled) return;
  setExpanded(expanded === side ? null : side);
}

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

// ---------------- 尺寸变量 ----------------

function applyMetrics(next: IslandMetrics): void {
  metrics = next;
  const root = document.documentElement.style;
  root.setProperty("--pill-w", `${next.PillWidth}px`);
  root.setProperty("--pill-h", `${next.PillHeight}px`);
  root.setProperty("--card-w", `${next.CardWidth}px`);
  root.setProperty("--card-h", `${next.CardHeight}px`);
  root.setProperty("--dsh-card-h", `${DSH_ONLY_CARD_HEIGHT}px`);
}

// ---------------- 输入 ----------------

const actorElements: ReadonlyArray<readonly [IslandSide, HTMLElement]> = [
  ["media", mediaActorEl],
  ["dsh", dshActorEl],
];

let pressedSide: IslandSide | null = null;

for (const [side, el] of actorElements) {
  el.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    pressedSide = side;
  });

  el.addEventListener("pointerup", (event) => {
    if (event.button !== 0 || pressedSide !== side) return;
    pressedSide = null;
    // 岛体自己处理掉的点击不再冒泡给舞台，避免「切换展开对象」又被当成空白处点击
    event.stopPropagation();
    toggleSide(side);
  });

  el.addEventListener("pointercancel", () => {
    pressedSide = null;
  });
}

/**
 * 展开态下点击岛体空白处（胶囊行那条横带）＝收起。
 *
 * 只认「按下也落在空白舞台上」的那一次点击：卡片里的按钮 / 进度条都会阻止冒泡，
 * 进度条拖到卡片外松手也不会误收起。
 */
let stagePressAt = 0;

stageEl.addEventListener("pointerdown", (event) => {
  stagePressAt = event.target === stageEl ? performance.now() : 0;
});

stageEl.addEventListener("pointerup", (event) => {
  const onBlank = stagePressAt > 0 && performance.now() - stagePressAt < 1000;
  stagePressAt = 0;
  if (!onBlank || event.button !== 0 || dragEnabled || expanded === null) return;
  setExpanded(null);
});

// ---------------- 事件订阅 ----------------

function bindEvents(): void {
  on<MediaSnapshot | null>(events.mediaChanged, (snapshot) => {
    renderMedia(snapshot ?? null);
  });

  // 高频位置推送：给时钟持续喂真值。整份快照只在「变化明显」时才发，
  // 只靠它的话两次快照之间只能本地外推，久了歌词就整首偏。
  on<PositionPayload>(events.mediaPosition, (payload) => {
    if (!media) return;

    clock.sync(
      trackKeyOf(media),
      payload.PositionMs,
      payload.PositionStampMs,
      payload.IsPlaying,
      payload.DurationMs,
    );

    if (expanded !== "media") return;
    const duration = payload.DurationMs > 0 ? payload.DurationMs : media.DurationMs;
    if (duration <= 0) return;
    mediaCard.setProgress(clock.position(), duration);
  });

  on<LyricsSnapshot>(events.lyricsChanged, (payload) => {
    mediaCard.setLyrics(payload);
  });

  on<DshSnapshot | null>(events.dshChanged, (snapshot) => {
    dsh = snapshot ?? null;
    syncDshContent();

    if (!hasDshContent() && expanded === "dsh") {
      setExpanded(null);
      return;
    }

    relayout(true);
  });

  on<IslandSettings>(events.settingsChanged, (next) => {
    dshEnabled = next.DshEnabled !== false;
    syncDshContent();
    // 胶囊右侧样式：频谱 / 进度圈 / 歌词三选一，下面的 relayout 会顺带把宽度解算进去
    applyRightStyle(next.PillRightStyle);

    const updated = metricsFor(next.Size, next.CustomPillWidth);
    const sizeChanged =
      updated.PillWidth !== metrics.PillWidth ||
      updated.PillHeight !== metrics.PillHeight ||
      updated.CardWidth !== metrics.CardWidth ||
      updated.CardHeight !== metrics.CardHeight ||
      updated.PillCorner !== metrics.PillCorner ||
      updated.CardCorner !== metrics.CardCorner;

    applyMetrics(updated);

    const bottom = next.BottomAnchored || next.Placement === "BottomCenter";
    if (bottom !== bottomAnchored) {
      bottomAnchored = bottom;
      stageEl.classList.toggle("bottom", bottom);
    }

    if (!hasDshContent() && expanded === "dsh") {
      setExpanded(null, !sizeChanged);
      return;
    }

    // 尺寸档位变化直接落位；只有展开 / 内容引起的几何变化才走缓动
    relayout(!sizeChanged);
  });

  on<string>(events.collapse, () => setExpanded(null));

  on<boolean>(events.visibility, (visible) => {
    if (!visible) {
      setExpanded(null, false);
    }
  });

  on<boolean>(events.openSettings, (open) => {
    dragEnabled = open;
  });
}

// ---------------- 启动 ----------------

async function init(): Promise<void> {
  const boot = await api.bootstrap();

  dshEnabled = boot.Settings.DshEnabled !== false;
  dsh = boot.Dsh ?? null;
  media = boot.Media ?? null;
  bottomAnchored = boot.BottomAnchored || boot.Settings.Placement === "BottomCenter";
  stageEl.classList.toggle("bottom", bottomAnchored);

  applyMetrics(metricsFor(boot.Settings.Size, boot.Settings.CustomPillWidth));
  applyRightStyle(boot.Settings.PillRightStyle);

  // 首帧内容先写好，再一次性落位（tween 尚未初始化，会走 snap）
  if (media) {
    clock.sync(
      trackKeyOf(media),
      media.PositionMs,
      media.PositionStampMs,
      media.IsPlaying,
      media.DurationMs,
    );
  }
  mediaPill.setMedia(media);
  mediaCard.update(media, clock.position());
  syncDshContent();
  glow.setPalette(media?.AccentColor ?? null, media?.AccentColorAlt ?? null);

  document.body.style.opacity = "0";
  relayout(false);

  bindEvents();
  startProgressLoop();
  startShapeHeartbeat();

  // 首帧样式已经写进 DOM：直接点亮并请求显示窗口。
  // 注意不能用 requestAnimationFrame 等待——窗口在显示前不会调度动画帧，会死锁。
  document.body.style.opacity = "1";
  await api.islandReady();
}

/**
 * 形状心跳：每 500 ms 无条件重推一次当前形状。
 *
 * 命中区域完全靠前端推来的矩形，任何一次推送丢失（IPC 抖动）都会让判定停在旧位置——
 * 表现就是「同一块地方时灵时不灵」。这里定期清掉去重键，保证后端最多落后半秒。
 */
function startShapeHeartbeat(): void {
  window.setInterval(() => {
    lastShapeKey = "";
    flushShapes();
  }, 500);
}

function startProgressLoop(): void {
  window.setInterval(() => {
    if (!media) return;
    const duration = media.DurationMs > 0 ? media.DurationMs : clock.durationMs;
    const position = clock.position();

    // 胶囊右侧的进度圈：与卡片进度同步（其它样式里这个调用是空操作）
    if (duration > 0) mediaPill.setProgress(position / duration);

    // 歌词行不受展开限制：保证展开瞬间当前句已就位
    mediaCard.tickLyrics(position);
    if (expanded !== "media" || duration <= 0) return;
    mediaCard.setProgress(position, duration);
  }, 80);
}

void init().catch((error) => {
  void api.log("error", `island 初始化失败: ${String(error)}`);
});
