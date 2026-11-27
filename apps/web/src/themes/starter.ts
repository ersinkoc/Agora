// Starter themes (roadmap M5.2, docs/ux/themes.md "Starter themes"): project-original and well-known palettes as theme files of format 2. Each is a
// plain theme file in `starter/`, read by the same function as any imported file (file.ts `deserialise`), so it holds to the floor and
// the readability rules like every other theme. Third-party themes use only the palette's published colour values, never its theme
// files; their licences and notices are in THIRD_PARTY.md. The terminal client reads the same files (crates/agora-tui/src/theme/starter.rs).
import abyss from './starter/abyss.json?raw';
import arctic from './starter/arctic.json?raw';
import aurora from './starter/aurora.json?raw';
import blossom from './starter/blossom.json?raw';
import bonsai from './starter/bonsai.json?raw';
import catppuccinMocha from './starter/catppuccin-mocha.json?raw';
import cobalt from './starter/cobalt.json?raw';
import dawn from './starter/dawn.json?raw';
import dracula from './starter/dracula.json?raw';
import ember from './starter/ember.json?raw';
import graphite from './starter/graphite.json?raw';
import gruvbox from './starter/gruvbox.json?raw';
import lagoon from './starter/lagoon.json?raw';
import lavender from './starter/lavender.json?raw';
import lilac from './starter/lilac.json?raw';
import meadow from './starter/meadow.json?raw';
import mint from './starter/mint.json?raw';
import mulberry from './starter/mulberry.json?raw';
import parchment from './starter/parchment.json?raw';
import porcelain from './starter/porcelain.json?raw';
import rosePine from './starter/rose-pine.json?raw';
import sepia from './starter/sepia.json?raw';
import tokyoNight from './starter/tokyo-night.json?raw';
import ultraviolet from './starter/ultraviolet.json?raw';
import { deserialise } from './file';
import type { Theme } from './tokens';

// Where a starter theme's colours come from: the palette's project and its licence (an SPDX id), shown beside the theme.
export type StarterSource = NonNullable<Theme['starter']>;

// The files, in the order the picker lists them. The id is the file's name, with `starter-` before it.
export const STARTER_FILES: readonly { id: string; text: string; source: StarterSource }[] = [
  { id: 'abyss', text: abyss, source: { project: 'Abyss', licence: 'Apache-2.0' } },
  { id: 'arctic', text: arctic, source: { project: 'Arctic', licence: 'Apache-2.0' } },
  { id: 'aurora', text: aurora, source: { project: 'Aurora', licence: 'Apache-2.0' } },
  { id: 'blossom', text: blossom, source: { project: 'Blossom', licence: 'Apache-2.0' } },
  { id: 'bonsai', text: bonsai, source: { project: 'Bonsai', licence: 'Apache-2.0' } },
  { id: 'catppuccin-mocha', text: catppuccinMocha, source: { project: 'Catppuccin', licence: 'MIT' } },
  { id: 'cobalt', text: cobalt, source: { project: 'Cobalt', licence: 'Apache-2.0' } },
  { id: 'dawn', text: dawn, source: { project: 'Dawn', licence: 'Apache-2.0' } },
  { id: 'dracula', text: dracula, source: { project: 'Dracula', licence: 'MIT' } },
  { id: 'ember', text: ember, source: { project: 'Ember', licence: 'Apache-2.0' } },
  { id: 'graphite', text: graphite, source: { project: 'Graphite', licence: 'Apache-2.0' } },
  { id: 'gruvbox', text: gruvbox, source: { project: 'Gruvbox', licence: 'MIT' } },
  { id: 'lagoon', text: lagoon, source: { project: 'Lagoon', licence: 'Apache-2.0' } },
  { id: 'lavender', text: lavender, source: { project: 'Lavender', licence: 'Apache-2.0' } },
  { id: 'lilac', text: lilac, source: { project: 'Lilac', licence: 'Apache-2.0' } },
  { id: 'meadow', text: meadow, source: { project: 'Meadow', licence: 'Apache-2.0' } },
  { id: 'mint', text: mint, source: { project: 'Mint', licence: 'Apache-2.0' } },
  { id: 'mulberry', text: mulberry, source: { project: 'Mulberry', licence: 'Apache-2.0' } },
  { id: 'parchment', text: parchment, source: { project: 'Parchment', licence: 'Apache-2.0' } },
  { id: 'porcelain', text: porcelain, source: { project: 'Porcelain', licence: 'Apache-2.0' } },
  { id: 'rose-pine', text: rosePine, source: { project: 'Rosé Pine', licence: 'MIT' } },
  { id: 'sepia', text: sepia, source: { project: 'Sepia', licence: 'Apache-2.0' } },
  { id: 'tokyo-night', text: tokyoNight, source: { project: 'Tokyo Night', licence: 'Apache-2.0' } },
  { id: 'ultraviolet', text: ultraviolet, source: { project: 'Ultraviolet', licence: 'Apache-2.0' } },
];

// The starter themes, read and checked once. A file that the reader refuses is left out (a test holds every file to no problems and
// no warnings, so none is). They are read-only like the built-in themes: Customize makes a copy.
export const STARTER: readonly Theme[] = STARTER_FILES.flatMap(({ id, text, source }) => {
  const r = deserialise(text);
  return r.ok ? [{ id: `starter-${id}`, ...r.theme, builtIn: true, starter: source }] : [];
});
