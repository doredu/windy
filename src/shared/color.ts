// Converts a "#rrggbb" hex color (as produced by <input type="color">) plus
// an alpha value (0-1) into a CSS rgba() string.
export function hexToRgba(hex: string, alpha: number): string {
  const match = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!match) return `rgba(30, 30, 34, ${alpha})`;
  const r = parseInt(match[1].slice(0, 2), 16);
  const g = parseInt(match[1].slice(2, 4), 16);
  const b = parseInt(match[1].slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
