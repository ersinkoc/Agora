// Starter themes (roadmap M5.2, docs/ux/themes.md "Starter themes"): well-known palettes, ported as theme files of format 2. Each is a
// plain theme file in `starter/`, read by the same function as any imported file (file.ts `deserialise`), so it holds to the floor and
// the readability rules like every other theme. Only the palette's published colour values are used, never its theme files; their
// licences and notices are in THIRD_PARTY.md. The terminal client reads the same files (crates/agora-tui/src/theme/starter.rs).
import catppuccinMocha from './starter/catppuccin-mocha.json?raw';
import dracula from './starter/dracula.json?raw';
import gruvbox from './starter/gruvbox.json?raw';
import rosePine from './starter/rose-pine.json?raw';
import tokyoNight from './starter/tokyo-night.json?raw';
import { deserialise } from './file';
import type { Theme } from './tokens';

// Where a starter theme's colours come from: the palette's project and its licence (an SPDX id), shown beside the theme.
export type StarterSource = NonNullable<Theme['starter']>;

// The files, in the order the picker lists them. The id is the file's name, with `starter-` before it.
export const STARTER_FILES: readonly { id: string; text: string; source: StarterSource }[] = [
  { id: 'catppuccin-mocha', text: catppuccinMocha, source: { project: 'Catppuccin', licence: 'MIT' } },
  { id: 'dracula', text: dracula, source: { project: 'Dracula', licence: 'MIT' } },
  { id: 'gruvbox', text: gruvbox, source: { project: 'Gruvbox', licence: 'MIT' } },
  { id: 'rose-pine', text: rosePine, source: { project: 'Rosé Pine', licence: 'MIT' } },
  { id: 'tokyo-night', text: tokyoNight, source: { project: 'Tokyo Night', licence: 'Apache-2.0' } },
];

// The starter themes, read and checked once. A file that the reader refuses is left out (a test holds every file to no problems and
// no warnings, so none is). They are read-only like the built-in themes: Customize makes a copy.
export const STARTER: readonly Theme[] = STARTER_FILES.flatMap(({ id, text, source }) => {
  const r = deserialise(text);
  return r.ok ? [{ id: `starter-${id}`, ...r.theme, builtIn: true, starter: source }] : [];
});
