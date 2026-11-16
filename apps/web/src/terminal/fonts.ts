// Programs such as WrongStack draw their status lines with icon glyphs from the Unicode private-use area (Powerline arrows, Material
// Design icons): the "Nerd Font" glyphs. An ordinary monospace font has none of them, so they show as empty boxes. The terminal
// therefore keeps the theme's own monospace font for the letters and lists the common Nerd Font families after it: a browser picks the
// font for each glyph from the list in order, so only the missing icons come from a Nerd Font, and a machine without one looks exactly
// as before.
const ICON_FONTS = [
  'Symbols Nerd Font Mono',
  'Symbols Nerd Font',
  'JetBrainsMono Nerd Font Mono',
  'JetBrainsMono NFM',
  'FiraCode Nerd Font Mono',
  'Hack Nerd Font Mono',
  'MesloLGS NF',
  'CaskaydiaCove Nerd Font Mono',
];

const quoted = (name: string) => (/^[A-Za-z0-9-]+$/.test(name) ? name : `'${name}'`);

/** The theme's monospace font list followed by the icon fonts, ending in the generic `monospace` (named once, at the end). */
export function terminalFontFamily(themeMono: string): string {
  const parts = themeMono
    .split(',')
    .map((p) => p.trim())
    .filter(Boolean);
  if (parts[parts.length - 1] === 'monospace') parts.pop();
  return [...parts, ...ICON_FONTS.map(quoted), 'monospace'].join(', ');
}
