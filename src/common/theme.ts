/** 岛体的视觉常量：材质、光晕与动画节奏的唯一定义处。 */
export const theme = {
  /** 岛体底色：纯黑不透明。 */
  islandBackground: "#0B0B0C",

  /** 光斑中心的最大不透明度（只做氛围，不干扰阅读）。 */
  glowPeakOpacity: 0.46,
  /** 呼吸缩放区间：明暗也要看得出在动，但不能抢过文字。 */
  glowScaleMin: 0.9,
  glowScaleMax: 1.1,
  /** 暂停 / 无封面时光晕淡出时长（毫秒）。 */
  glowFadeMs: 260,
  /** 换歌时颜色过渡时长（毫秒）。 */
  glowColorFadeMs: 320,

  /**
   * 每团光斑横向 / 纵向漂移一周的周期（秒）。
   * x 与 y 故意取不同值，两条正弦叠加后光路才会持续游走、不显重复。
   */
  glowFlowSeconds: [
    { x: 15, y: 11 },
    { x: 12, y: 17 },
    { x: 19, y: 13 },
    { x: 11, y: 15 },
    { x: 17, y: 12 },
  ] as const,

  /** 没有封面 / 取色失败时的中性冷色。 */
  glowFallbackPrimary: "#465A78",
  glowFallbackSecondary: "#2C3A52",

  /** 展开 / 收起时长（毫秒）。 */
  expandMs: 300,
  collapseMs: 240,
} as const;
