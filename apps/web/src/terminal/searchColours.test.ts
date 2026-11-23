import { describe, expect, it } from 'vitest';
import { BUILT_IN, baseTokens, contrast } from '../themes/tokens';
import { mixHex, searchDecorations, toHex } from './searchColours';

const HEX = /^#[0-9a-f]{6}$/;

describe('terminal search colours', () => {
  it('reads a computed colour as #rrggbb', () => {
    expect(toHex('rgb(139, 147, 255)')).toBe('#8b93ff');
    expect(toHex('rgba(79, 70, 229, 0.5)')).toBe('#4f46e5');
    expect(toHex('rgb(79 70 229)')).toBe('#4f46e5');
    expect(toHex('#ABC')).toBe('#aabbcc');
    expect(toHex(' #4F46E5 ')).toBe('#4f46e5');
    expect(toHex('oklab(0.5 0 0)')).toBeUndefined();
    expect(toHex('')).toBeUndefined();
  });

  it('mixes two colours into an opaque #rrggbb', () => {
    expect(mixHex('#ffffff', '#000000', 0)).toBe('#000000');
    expect(mixHex('#ffffff', '#000000', 1)).toBe('#ffffff');
    expect(mixHex('#ffffff', '#000000', 0.5)).toBe('#808080');
    expect(mixHex('#8b93ff', '#09090b', 0.25)).toMatch(HEX);
  });

  // Dark and Light are the two schemes; the other built-in themes are checked the same way.
  for (const theme of BUILT_IN) {
    it(`draws readable matches from the accent in ${theme.id}, the current one stronger, never amber`, () => {
      const { accent, 'surface-0': bg, text, 'state-needs-you': amber } = theme.tokens;
      const d = searchDecorations(accent, bg, text);
      for (const c of [d.matchBackground, d.matchOverviewRuler, d.activeMatchBackground, d.activeMatchBorder, d.activeMatchColorOverviewRuler]) expect(c).toMatch(HEX);
      // The terminal's text stays readable on a match and on the current match.
      expect(contrast(text, d.matchBackground)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(text, d.activeMatchBackground)).toBeGreaterThanOrEqual(4.5);
      // A match stands out from the terminal's background, and the current one clearly more (and it has a full accent border).
      expect(contrast(d.matchBackground, bg)).toBeGreaterThanOrEqual(1.4);
      expect(contrast(d.activeMatchBackground, bg)).toBeGreaterThan(contrast(d.matchBackground, bg) * 1.4);
      expect(d.activeMatchBorder).toBe(accent);
      expect([d.matchBackground, d.activeMatchBackground, d.activeMatchBorder]).not.toContain(amber);
    });
  }

  it('steps the current match back toward the background when a theme accent would make its text unreadable', () => {
    const { 'surface-0': bg, text } = baseTokens('dark');
    const d = searchDecorations('#ffffff', bg, text);
    expect(contrast(text, d.activeMatchBackground)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(d.activeMatchBackground, bg)).toBeGreaterThan(contrast(d.matchBackground, bg));
  });
});
