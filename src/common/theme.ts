/** 岛体的视觉常量：材质、光晕与动画节奏的唯一定义处。 */
export const theme = {
  /** 岛体底色：纯黑不透明。 */
  islandBackground: "#0B0B0C",

  /** 流光主光斑的最大不透明度（只做氛围，不干扰阅读）。 */
  glowPeakOpacity: 0.58,

  /** 展开 / 收起时长（毫秒）。 */
  expandMs: 300,
  collapseMs: 240,
} as const;
