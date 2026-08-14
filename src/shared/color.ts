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

// Popup text/icon colors are hardcoded light (#eee-family) to suit the
// default dark background -- but popup_bg_color is a free color picker with
// no bounds. Without this, choosing a light background leaves all popup
// text unreadable (light-on-light). Uses the standard WCAG relative
// luminance formula to decide whether the background counts as "light".
export function isLightColor(hex: string): boolean {
  const match = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!match) return false;
  const r = parseInt(match[1].slice(0, 2), 16) / 255;
  const g = parseInt(match[1].slice(2, 4), 16) / 255;
  const b = parseInt(match[1].slice(4, 6), 16) / 255;
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return luminance > 0.6;
}
