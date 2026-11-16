import { describe, expect, it } from 'vitest';
import { terminalFontFamily } from './fonts';

describe('terminalFontFamily', () => {
  const theme = "ui-monospace, 'SF Mono', 'JetBrains Mono', Menlo, Consolas, monospace";

  it('keeps the theme font first, so letters do not change', () => {
    expect(terminalFontFamily(theme).startsWith("ui-monospace, 'SF Mono', 'JetBrains Mono', Menlo, Consolas, ")).toBe(true);
  });

  it('lists the icon fonts after the theme font and before the final generic monospace', () => {
    const f = terminalFontFamily(theme);
    expect(f.indexOf("'Symbols Nerd Font Mono'")).toBeGreaterThan(f.indexOf('Consolas'));
    expect(f.indexOf("'JetBrainsMono Nerd Font Mono'")).toBeGreaterThan(-1);
    expect(f.endsWith(', monospace')).toBe(true);
    expect(f.lastIndexOf('Nerd Font')).toBeLessThan(f.lastIndexOf(', monospace'));
    // The theme's own trailing generic is not repeated in the middle of the list.
    expect(f.split(', ').filter((p) => p === 'monospace')).toHaveLength(1);
  });

  it('quotes names with spaces and leaves single words bare', () => {
    const f = terminalFontFamily(theme);
    expect(f).toContain("'FiraCode Nerd Font Mono'");
    expect(f).toContain("'MesloLGS NF'");
    expect(f).toContain("'JetBrainsMono NFM'");
    expect(f).toContain(', Menlo, ');
  });

  it('works when the theme gave nothing or no generic family', () => {
    for (const none of ['', '   ', 'monospace']) {
      const f = terminalFontFamily(none);
      expect(f.startsWith("'Symbols Nerd Font Mono', ")).toBe(true);
      expect(f.endsWith(', monospace')).toBe(true);
      expect(f).not.toMatch(/(^|,)\s*,/);
    }
    expect(terminalFontFamily('Fira Code').startsWith('Fira Code, ')).toBe(true);
    expect(terminalFontFamily('Fira Code').endsWith(', monospace')).toBe(true);
  });
});
