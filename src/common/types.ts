/**
 * 前后端共享契约。
 *
 * 字段名统一 PascalCase，Rust 侧用 `#[serde(rename_all = "PascalCase")]` 序列化，
 * 与 `src-tauri/src/settings.rs`、`src-tauri/src/media/mod.rs` 的结构体一一对应。
 */

export type PlacementPreset =
  | "TopCenter"
  | "TopLeft"
  | "TopRight"
  | "BottomCenter"
  | "Custom";

export type IslandSizePreset = "Small" | "Medium" | "Large" | "Custom";

/**
 * 胶囊右侧显示什么：三种样式互斥，同一时刻只有一个生效（`src-tauri/src/settings.rs` 同名枚举）。
 */
export type PillRightStyle = "Spectrum" | "ProgressRing" | "Lyrics";

/**
 * DeepSeek Harness 的执行阶段。
 * 由状态文件写入方给出；未知取值前端按 `idle` 处理。
 */
export type DshStage = "idle" | "thinking" | "tool" | "waiting" | "done";

/** 唯一持久化契约。所有值域收敛由 Rust 侧 Normalize 完成。 */
export interface IslandSettings {
  Placement: PlacementPreset;
  /** 自由落位锚点横向比例（相对显示器工作区，0~1）。 */
  AnchorXRatio: number;
  /** 自由落位锚点纵向比例（0~1）。 */
  AnchorYRatio: number;
  /** true 表示以锚点下沿为基准向上生长。 */
  BottomAnchored: boolean;
  MonitorLeft: number;
  MonitorTop: number;
  MonitorWidth: number;
  MonitorHeight: number;
  Size: IslandSizePreset;
  /** 自定义胶囊宽度（DIP），高度恒为该值的一半。 */
  CustomPillWidth: number;
  CustomPillHeight: number;
  Persistent: boolean;
  /** 胶囊右侧样式（默认 `Spectrum`）。 */
  PillRightStyle: PillRightStyle;
  AutoStart: boolean;
  AutoCollapseOnPause: boolean;
  IdleCollapse: boolean;
  IdleCollapseSeconds: number;
  /** 是否接入 DeepSeek Harness 状态（默认开启）。 */
  DshEnabled: boolean;
}

/** 胶囊态 / 展开态的尺寸几何（DIP）。 */
export interface IslandMetrics {
  PillWidth: number;
  PillHeight: number;
  CardWidth: number;
  CardHeight: number;
  PillCorner: number;
  CardCorner: number;
}

/** 某一时刻的媒体状态快照（不可变）。 */
export interface MediaSnapshot {
  Title: string;
  Artist: string;
  AlbumTitle: string;
  /** 来源应用显示名（如 Spotify）。 */
  SourceAppName: string;
  IsPlaying: boolean;
  /** 采样的播放位置（毫秒）。 */
  PositionMs: number;
  /** 总时长（毫秒），0 表示无法跳转。 */
  DurationMs: number;
  /** 采样 <see cref="PositionMs"/> 的本地时刻（Unix 毫秒）。 */
  PositionStampMs: number;
  /** 封面 data URL；无封面时为 null。 */
  ArtworkUrl: string | null;
  /** 封面版本号，变化即代表需要换图。 */
  ArtworkVersion: number;
  /** 封面主色 `#RRGGBB`。 */
  AccentColor: string | null;
  /** 封面次色 `#RRGGBB`。 */
  AccentColorAlt: string | null;
  /**
   * 播放器是否接受外部跳转。
   *
   * `false` 表示实测过「跳转指令发出去了、位置却没动」——这类应用的进度条
   * 拖了也没反应，前端据此禁用拖动（仍正常显示进度）。
   */
  Seekable: boolean;
}

/**
 * 高频位置推送（不含封面与文本，可以每几百毫秒发一次）。
 *
 * 歌词与进度靠它持续对位：只依赖整份快照的话，两次快照之间只能本地外推，
 * 累积偏差会把整首歌的歌词顶偏。
 */
export interface PositionPayload {
  PositionMs: number;
  PositionStampMs: number;
  DurationMs: number;
  IsPlaying: boolean;
}

/** 某首歌的歌词检索结果（后端在线检索后推送；无歌词时 Lrc / LrcHex 均为 null）。 */
export interface LyricsSnapshot {
  Title: string;
  Artist: string;
  AlbumTitle: string;
  /** 原始 LRC 文本（带时间轴）；无歌词时为 null。 */
  Lrc: string | null;
  /**
   * 本地歌词文件的原始字节（小写十六进制）。
   * 本地文件编码混杂（UTF-8 / UTF-16 / GBK），由前端 `TextDecoder` 解码。
   */
  LrcHex: string | null;
  /** 歌词来源（`qq` / `netease` / `local`），用于排查时间对不上的问题。 */
  Origin: "qq" | "netease" | "local" | null;
}

/**
 * DeepSeek Harness 某一时刻的状态快照（不可变）。
 *
 * 由 `dsh-status.json` 解析而来：Rust 侧 `src-tauri/src/dsh/mod.rs` 负责
 * 定位文件、判定活跃、汇总 token 与增量推送，前端只做展示。
 */
export interface DshSnapshot {
  /** 执行阶段。 */
  Stage: DshStage;
  /** 展示用短词（写入方给中文）；为空时前端按 Stage 兜底。 */
  StageLabel: string;
  Model: string;
  /** `deepseek-official` / `openrouter` / ... */
  Provider: string;
  /** 最近一次工具调用的名字（自动探测给出），展开态展示。 */
  Tool: string;
  /** `official` / `web` / `api`：仅 official / web 才显示余额。 */
  AuthMode: string;
  SessionId: string;
  InputTokens: number;
  OutputTokens: number;
  CacheReadTokens: number;
  /** 会话累计 token（input + output + cacheRead）。 */
  TotalTokens: number;
  SessionCost: number;
  Currency: string;
  /** 账户余额；写入方未提供时为 null（该段整体不占位）。 */
  Balance: number | null;
  BalanceCurrency: string;
  /** 写入时刻（Unix 毫秒），用于活跃判定。 */
  UpdatedAtMs: number;
}

/**
 * 岛面当前可见形状（相对窗口客户区的物理像素）。
 *
 * 媒体岛与 dsh 岛各自一块：胶囊态两块都是胶囊（媒体也可能是正圆，用 Radius 表达），
 * 展开时一块是卡片、另一块是留在胶囊行里的胶囊。
 */
export interface ShapeRect {
  Left: number;
  Top: number;
  Width: number;
  Height: number;
  /** 圆角半径（物理像素）。 */
  Radius: number;
}

/** 两个岛面：媒体岛与 DeepSeek Harness 岛。 */
export type IslandSide = "media" | "dsh";

/** 岛窗首帧渲染完成后拉取的一次性状态。 */
export interface Bootstrap {
  Settings: IslandSettings;
  Media: MediaSnapshot | null;
  /** DeepSeek Harness 状态；未检测到 dsh 时为 null。 */
  Dsh: DshSnapshot | null;
  Metrics: IslandMetrics;
  /** 当前显示器缩放（DPI / 96）。 */
  Scale: number;
  /** true 表示岛体在窗口内贴底对齐（向上生长）。 */
  BottomAnchored: boolean;
  /** 当前是否应当显示岛体（有媒体或常驻，且未被用户隐藏）。 */
  Visible: boolean;
  Version: string;
  /** 可执行文件路径（设置窗口底部展示）。 */
  ExecutablePath: string;
}

/** 展开态自动收起的原因。 */
export type CollapseReason = "foreground" | "outside-click" | "idle";

export const ISLAND_LABEL = "island";
export const SETTINGS_LABEL = "settings";

export const EVENTS = {
  mediaChanged: "media://changed",
  mediaPosition: "media://position",
  lyricsChanged: "lyrics://changed",
  dshChanged: "dsh://changed",
  settingsChanged: "settings://changed",
  visibility: "island://visibility",
  collapse: "island://collapse",
  expanded: "island://expanded",
  openSettings: "settings://open",
} as const;
