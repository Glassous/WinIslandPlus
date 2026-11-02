/** `#RRGGBB` → `rgba(r,g,b,a)`；解析失败时回落到给定的兜底色。 */
export function withAlpha(hex: string | null | undefined, alpha: number, fallback = "#465A78"): string {
  const value = normalizeHex(hex) ?? fallback;
  const r = parseInt(value.slice(1, 3), 16);
  const g = parseInt(value.slice(3, 5), 16);
  const b = parseInt(value.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function normalizeHex(hex: string | null | undefined): string | null {
  if (!hex) return null;
  const value = hex.trim();
  if (/^#[0-9a-f]{6}$/i.test(value)) return value;
  if (/^[0-9a-f]{6}$/i.test(value)) return `#${value}`;
  return null;
}
