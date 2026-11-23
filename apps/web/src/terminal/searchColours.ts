import { contrast } from '../themes/tokens';

// The terminal's search matches take the theme's accent, never amber (amber means only "needs you", ADR-0102). xterm wants opaque
// `#rrggbb` backgrounds, so the accent is mixed toward the terminal's background here rather than given an alpha: every match is a soft
// accent, the current one a stronger accent with a full accent border. The terminal's text stays readable (4.5:1) on both; for a theme
// whose accent would make the current match too strong for that, the current match steps back toward the background until it is.

const HEX = /^#[0-9a-f]{6}$/i;

/** A computed CSS colour (`#rrggbb`, `#rgb`, `rgb(…)` or `rgba(…)`) as opaque `#rrggbb`, or undefined when it is not one of those. */
export function toHex(css: string): string | undefined {
  const v = css.trim().toLowerCase();
  if (HEX.test(v)) return v;
  if (/^#[0-9a-f]{3}$/.test(v)) return `#${[...v.slice(1)].map((c) => c + c).join('')}`;
  const m = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/.exec(v);
  if (!m) return undefined;
  return `#${[m[1], m[2], m[3]].map((n) => Math.max(0, Math.min(255, Math.round(Number(n)))).toString(16).padStart(2, '0')).join('')}`;
}

/** `a` laid over `b` at `share` (0–1), as opaque `#rrggbb`. */
export function mixHex(a: string, b: string, share: number): string {
  const ch = (h: string, i: number) => Number.parseInt(h.slice(i, i + 2), 16);
  return `#${[1, 3, 5].map((i) => Math.round(ch(a, i) * share + ch(b, i) * (1 - share)).toString(16).padStart(2, '0')).join('')}`;
}

export const MATCH_SHARE = 0.25;
export const ACTIVE_SHARE = 0.5;

/** The search decorations for a terminal drawn with `text` on `background`, from the theme's `accent` (all `#rrggbb`). */
export function searchDecorations(accent: string, background: string, text: string) {
  const match = mixHex(accent, background, MATCH_SHARE);
  let share = ACTIVE_SHARE;
  while (share > MATCH_SHARE + 0.1 && contrast(text, mixHex(accent, background, share)) < 4.5) share -= 0.05;
  const active = mixHex(accent, background, share);
  return { matchBackground: match, matchOverviewRuler: mixHex(accent, background, 0.6), activeMatchBackground: active, activeMatchBorder: accent, activeMatchColorOverviewRuler: accent };
}
