/** 岛体的视觉常量：材质、光晕与动画节奏的唯一定义处。 */
export const theme = {
  /** 岛体底色：纯黑不透明。 */
  islandBackground: "#0B0B0C",

  /** 光斑中心的最大不透明度（只做氛围，不干扰阅读）。 */
  glowPeakOpacity: 0.46,
  /** 呼吸缩放区间。 */
  glowScaleMin: 0.92,
  glowScaleMax: 1.08,
  /** 暂停 / 无封面时光晕淡出时长（毫秒）。 */
  glowFadeMs: 260,
  /** 换歌时颜色过渡时长（毫秒）。 */
  glowColorFadeMs: 320,

  /** 三团光斑漂移一周的时长（秒），错相位避免同步呼吸。 */
  glowDriftSeconds: [17, 23, 14] as const,

  /** 没有封面 / 取色失败时的中性冷色。 */
  glowFallbackPrimary: "#465A78",
  glowFallbackSecondary: "#2C3A52",

  /** 展开 / 收起时长（毫秒）。 */
  expandMs: 300,
  collapseMs: 240,
} as const;
