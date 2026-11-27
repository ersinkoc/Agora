# Themes: design tokens as data (ADR-0068 rung 2, ADR-0113 format 2)

![The theme editor beside the agents, with its preview, dark](../screenshots/adr113-theme-editor-dark.png)
![The same in light](../screenshots/adr113-theme-editor-light.png)
![A change that would be hard to read is shown, not applied](../screenshots/adr68-theme-contrast-warning-dark.png)

A **theme** is a small file of **design tokens** (the variables in `apps/web/src/design/tokens.css`). Every colour, font, size,
corner and spacing a component draws comes from a token (`visual-language.md` §3), so a theme restyles the whole window without
code. This page is the file format's specification and the web window's behaviour. Rung 2 covers the web window; the terminal
client reads the same files ([below](#the-terminal-client), ADR-0113 build step 3). Themes and the choice are kept in the browser and,
through the hub, the same on all the person's devices ([below](#keeping-themes-the-same-on-your-devices), build step 4): the hub keeps them
as the person's own opaque value and never reads them. The hub also keeps the [community catalog](#the-community-catalog), which it
fetches on its owner's command (build step 5).

**Format 2** ([ADR-0113](../adr/0113-deep-themes-and-community-themes-stay-data.md), below) goes deeper where the look comes from:
both schemes in one file, the terminal's palette, the colours of the window's parts, and the window's chrome (pane borders and
gaps, the tab bar, density, how a state is shown, shadows, motion), always as values from closed lists, never CSS. A theme stays
data: it never holds code, CSS, `url()` or anything from the network, and it can never change what needs you ([the floor](#the-floor-what-no-theme-can-change)).

A **theme pack** ([below](#theme-packs), ADR-0113 decision 5) is one file with themes, saved layouts and keys, to share a look with
the arrangement and keys it was made for.

## Format, version 1

A theme file is JSON:

```json
{
  "agora-theme": 1,
  "name": "Night",
  "scheme": "dark",
  "tokens": { "accent": "#ff9f43", "on-accent": "#1a1000", "text-16": "18px" }
}
```

- `agora-theme`: the format version: `1` here, `2` [below](#format-version-2). A file with a higher number is refused with
  "made by a newer version", nothing of it is guessed. Format 1 files are still read as they always were, with one change the
  floor asks for: a `text-16` under 16 px is shown at 16 px with a warning (`theme-warning-text-lifted`), so no text is under
  12 px. Export writes format 2.
- `name`: 1 to 60 characters. Control and text-direction characters are removed before it is shown, and a format-2 name is judged
  after that removal, so a name those characters split (`ur` + a direction mark + `l(`) is refused. When a format-1 theme is
  stored or exported as format 2, the pieces format 2 refuses (`url(`, `@import`, `;`, `}`, `expression(`, `<`) are removed from
  its name, so every theme format 1 accepts stays one format 2 accepts.
- `scheme`: `"dark"` or `"light"`. It decides which slot the theme fills (below), the browser's `color-scheme`, shadows and
  the terminal's 16-colour palette.
- `tokens`: any of the tokens below. Tokens left out come from the built-in **Dark** or **Light** theme of the same scheme, so
  a theme can be one line. Exporting writes every token, so a file reads the same in a client whose defaults differ.

No other field is allowed.

### Tokens

| Token | What it styles (as the editor names it) | Value |
| --- | --- | --- |
| `surface-0` | Background (the window, terminals) | colour |
| `surface-1` | Panels (bars, cards) | colour |
| `surface-2` | Raised surfaces (selected rows, buttons) | colour |
| `border` | Lines | colour |
| `text` | Text | colour |
| `text-muted` | Secondary text | colour |
| `accent` | Accent (the brand colour, links, focus) | colour |
| `on-accent` | Text on accent | colour |
| `state-working`, `state-needs-you`, `state-finished`, `state-idle`, `state-unknown` | the agent states | colour |
| `font-ui` | Interface font | font list |
| `font-mono` | Terminal and code font | font list |
| `text-16` | Text size: the base size; every text size scales with it | 12–24 px |
| `space-1` | Spacing: the 4 px grid unit; every gap and padding is a multiple | 3–6 px |
| `radius-xs`, `radius-s`, `radius-m`, `radius-l` | Corners | 0–16, 0–24, 0–32, 0–40 px |

- A **colour** is `#rgb` or `#rrggbb` (stored as lowercase `#rrggbb`). Named colours, `rgb()` and transparency are not
  accepted: contrast must be computable exactly.
- A **font list** is family names separated by commas, quoted or not (`'JetBrains Mono', monospace`); at most 200 characters
  of letters, digits, spaces and `_ . , ' " -`. Nothing else, so a value can never end the CSS declaration it is put in.
- A **size** is `"18px"` or the number `18`, within the token's range.
- `text-12`, `text-13`, `text-14`, `text-20`, `text-28` and `space-2` … `space-8` exist in `tokens.css` but **follow** from
  `text-16` and `space-1`; a file that sets one is refused with "follows from `text-16`: set `text-16` instead".
- In format 1, motion (`motion-*`) and `shadow` are not themeable: motion follows the system's reduced-motion setting, shadows
  follow the scheme. Format 2 has `motion` and `shadow` chrome values.

Sizes stay relative to the browser's own text size: `text-16: 20px` makes the root font size 125 % of the browser's, and the
window's text and spacing (rem-based) grow with it. The terminals keep their own text size (A−/A+).

## Format, version 2

![A format-2 theme: its own terminal palette and parts, gaps and lines around every pane, rounded tabs, compact density (dark)](../screenshots/adr113-sample-harbour-dark.png)
![The same theme's light colours, from the same file](../screenshots/adr113-sample-harbour-light.png)

```json
{
  "agora-theme": 2,
  "name": "Harbour",
  "inherits": "slate",
  "dark": {
    "tokens": { "accent": "#5cc8c2" },
    "regions": { "sidebar": { "surface": "#0b1016" }, "person-entries": { "surface": "#14302f", "line": "#2b6b68" } },
    "terminal": { "background": "#0b1016", "foreground": "#d8e1ea", "red": "#ff7b72" },
    "syntax": { "keyword": "#ff9e8f" }
  },
  "light": { "tokens": { "accent": "#0b6e69", "on-accent": "#ffffff" } },
  "shape": { "radius-m": "10px" },
  "text": { "font-mono": "'JetBrains Mono', monospace", "weight": 400, "line-height": "normal", "ligatures": true },
  "chrome": { "pane-borders": "always", "pane-gap": 8, "tab-style": "pill", "density": "compact" }
}
```

Fields (no other is allowed, at any level):

- `agora-theme`: `2`. `name`: as in format 1.
- `inherits` (optional): the id of a built-in theme (`dark`, `light`, `high-contrast`, `large-text`, `slate`, `sand`). Values the
  file leaves out come from it: its colours for its own scheme, and its shape and fonts for both. Without it they come from the
  default **Dark** or **Light**. A file with `inherits` and no scheme sections is that theme with the file's chrome and text.
- `dark`, `light` (at least one, or `inherits`): the colours of that scheme. A theme with both fills both slots (below); one with
  one fills that one. Each holds:
  - `tokens`: the 13 colour tokens of format 1 (surfaces, lines, text, accent, states). Sizes and fonts are not here.
  - `regions`: the colours of the window's parts, each `{ "surface", "text", "line" }`, each optional (below).
  - `terminal`: the terminal's colours, each optional (below).
  - `syntax`: code colours, each optional: `keyword`, `string`, `comment`, `number`, `function`, `type`, `attr`, `meta`. Left out,
    they are mixed from the theme's colours as in format 1 (`app.css` `--syntax-*`).
- `shape`: `text-16` (**16**–24 px: the smallest text is three quarters of it, and no text is under 12 px), `space-1`, and the
  four corners, as in format 1.
- `text`: `font-ui` and `font-mono` (font lists as in format 1: installed fonts only, the same safe alphabet), `weight` (300,
  400 or 500), `line-height` (`"normal"`, each text size's own, or a number from 1.2 to 1.8) and `ligatures` (`true` or `false`;
  the interface and code blocks, not terminals: xterm has no ligatures without an add-on).
- `chrome`: the window's frame (below).

Export writes every token, every shape, text and chrome value, and the regions, terminal and syntax colours the theme sets (not
`inherits`), so a file reads the same in a client whose defaults differ, and a file exported and imported is identical.
Themes are kept in this browser as format-2 files; one stored before this format is a format-1 file and is read as above.

### Regions

| Region | `regions` key | Drawn without a theme in (surface / text / line) |
| --- | --- | --- |
| Agents list (the sidebar) | `sidebar` | Panels / Text / Lines |
| Tabs: the tab in front, and the lines between tabs | `tab-bar` | Background / Text / Lines |
| Pane bars | `pane-bar` | Panels / Text / Lines |
| Status line (the bottom bar) | `status-line` | Panels / Secondary text / Lines |
| Cards (agent tiles, the kit's cards) | `cards` | Panels / Text / Lines |
| Text fields and menus | `inputs` | Panels / Text / Lines |
| Buttons (not the main or red ones) | `buttons` | Raised surfaces / Text / Lines |
| Agent replies in a conversation | `agent-entries` | none (they sit on the pane) / Text / none |
| Your messages in a conversation | `person-entries` | Accent background / Text / the accent's hairline |
| Code blocks in a conversation | `code-blocks` | Panels / Text / Lines |

Each part is a CSS variable (`--sidebar-surface`, …) that the window reads with today's token as its fallback, so nothing
changes until a theme sets one. Agent replies get a box (padding, a corner, a line) only when a theme colours them.

### Terminal

`background`, `foreground`, `cursor`, `selection` and the 16 colours `black`, `red`, `green`, `yellow`, `blue`, `magenta`,
`cyan`, `white` and their `bright-` forms. Left out, a terminal draws as before: the pane's background, the theme's text, the
accent as cursor and selection, and the window's palette for the scheme. Every terminal pane takes them, and takes them again on
every theme change (`agora-theme` event). The selection is drawn at 40 % over the background, fainter in a terminal not in focus,
so text keeps its own colour on top. The terminal's frame and its search matches follow the terminal's background and text.

### Chrome

Every value is a word from a list or whole pixels in a range. The theme sets it on `<html>` as a data attribute (the list's own
word, never the file's text) or as a number variable (`--pane-gap`, `--line-width`), and the stylesheet reads it; a theme never
builds CSS.

| Key | Values (default first) | What it does |
| --- | --- | --- |
| `pane-borders` | `auto`, `always`, `off` | Auto: a line between panes that touch (with a gap, the gap alone). Always: a line around every pane. Off: none. |
| `pane-gap` | 0–16 px (0) | Space between panes; the background shows through. |
| `line-width` | 0–2 px (1) | The window's lines: between and around panes, under pane bars, the header, the status line, the Agents list's edge. |
| `tab-bar` | `top`, `bottom` | Where a pane's bar (its tabs or title) sits. |
| `tab-style` | `underline`, `filled`, `pill` | The tab in front: an accent line, filled with the accent (its state mark on a chip of the background), or rounded tabs with no lines between them. |
| `density` | `normal`, `compact`, `roomy` | The height of pane bars and tabs, rows and cards, with a mouse or trackpad only (under a finger every target keeps the platform's size). The Agents list's rows and a "needs you" tile keep their size. |
| `state-display` | `symbol`, `dot`, `word` | How an agent's state shows in lists and tabs. The word is always in the tooltip and read aloud; "Needs you" always keeps its own symbol. |
| `icons` | `lines` | The icon set. There is one so far; the key is there so a theme can name it. |
| `shadow` | `soft`, `none`, `strong` | The one elevation (menus, overlays); cards and panels stay flat. |
| `motion` | `system`, `reduced`, `off` | Follow the system's reduced-motion setting; reduced: the marks that pulse or breathe (a state, a working dot, a loading row) stand still; off: no animation or transition at all. |

## The floor: what no theme can change

A theme can make the window look like anything, but it can never make it lie or hide what needs you (ADR-0113 decision 3). A file
that sets any of these, by these names anywhere in it, is refused with the reason (`theme-error-floor`, `theme-floor-*`):

| Name in a file | What it is |
| --- | --- |
| `danger`, `on-danger` | The one red for errors and danger, and its label (`--danger`, `--on-danger`) |
| `diff-removed` | The removed-line colour (the same red) |
| `approval-card`, `approval` | A request from an agent: its frame, colours and text, and the action it asks about |
| `focus-ring`, `focus` | The focus ring |
| `needs-you`, `attention` | The "needs you" treatment: shape, word and place, not only colour |
| `plugin-marker` | The mark on what a plugin adds (ADR-0114) |

The names are matched whole: `state-needs-you` (the colour of the state) and `accent` (which the focus ring is drawn in) are
format-1 tokens and stay themeable, checked for readability as before. Beyond the names:

- **No text under 12 px**: `text-16` is at least 16 px in format 2 (`theme-error-text-floor`); format 1 is lifted (above).
- **The window keeps it out of the regions, the text values and the fonts.** A request card, the "Needs you" card and tile, and
  anything drawn in the "needs you" tone draw their buttons, cards, fields and code in the base tokens whatever the theme's regions
  say, and keep the window's own fonts (never a theme's font list), text weight, line height and ligatures (`app.css`, the floor
  rules). The "Plugin" marker beside a plugin panel's name (ADR-0114, `plugins.md`) is held by the same rules. A state display never replaces the "needs you" symbol; with the filled tab style a state's mark sits on a chip of the
  background, where every state is held to 3:1. Density never changes a "needs you" tile, and never applies under a finger (coarse
  pointer), so every hit target keeps the platform's minimum. What the floor does not hold: the base text size (it only grows,
  16–24 px, so text can only get larger), the base colours (`state-needs-you`, `accent`, surfaces, checked for readability as in
  format 1), the window's layout around a card (gaps, lines, the bar's place), and motion: `reduced` and `off` still the "needs you"
  mark's pulse as the system's reduced-motion setting does; its shape, word and place stay. A theme that
  sets every region, chrome and text value and inherits the base colours leaves a request card pixel-identical (checked with
  pixelmatch on the right panel's card in both schemes for this change).
- **A file is at most 256 KiB** (`theme-error-too-large`, the per-person settings cap of ADR-0113), and at most 50 problems are
  listed, then "…and N more" (`theme-error-more`).
- **Never CSS**: a value anywhere (the name too) containing `url(`, `@import`, `;`, `}`, `expression(` or `<` is refused
  (`theme-error-unsafe`), and no style property is set.

The editor lists the floor as locked rows, each with its reason.

![The window's chrome rows and the locked floor in the editor](../screenshots/adr113-theme-floor-dark.png)

## Reading a theme (validation)

Every theme from outside (an imported file, this browser's storage, a change in the editor) goes through one function that
never throws and reports **every** problem at once, each with a message from `locales/<lang>/web.ftl`:

1. Not JSON (`theme-error-json`); not an object with an integer `agora-theme` ≥ 1 (`theme-error-invalid`); a newer version
   (`theme-error-newer`).
2. An unknown field (`theme-error-unknown-field`), a bad name (`theme-error-name`) or scheme (`theme-error-scheme`).
3. Each token: unknown (`theme-error-unknown-token`), derived (`theme-error-derived-token`), or a value that is not allowed
   (`theme-error-bad-color`, `-bad-size` with the range, `-bad-font`).
4. **Readability** (WCAG 2.2 AA), once every colour is valid: `text`, `text-muted` and `accent` need **4.5:1** on each of the
   three surfaces, `on-accent` 4.5:1 on `accent` (contrast minimum, 1.4.3); each state colour needs **3:1** on each surface
   (non-text contrast, 1.4.11: states are always colour + shape + word). A failing pair is reported as
   "Secondary text on Panels is 1.6:1; it needs at least 4.5:1 to be readable" (`theme-error-contrast`; the ratio is rounded
   down, so a shown 4.5 always passes).
5. **Text on coloured backgrounds** (#1021), once the theme is otherwise accepted: the widget kit's soft backgrounds for
   accent, working, needs you, finished, the red and idle (`--tint-*`, ADR-0102) are mixed from the theme's own colours
   (`color-mix(in oklab)`, 14 % into `surface-1` in a dark theme, 10 % in a light one; the red is the fixed one). The check mixes
   them the same way (`themes/tints.ts`, the math the kit's contrast test uses) and asks **4.5:1** of the role's own colour,
   `text` and `text-muted` on each. A failing pair is reported in the same sentence, naming the background: "Idle on Idle
   background is 3.4:1; it needs at least 4.5:1 to be readable" (`theme-error-contrast` with `theme-tint-*`, the red as
   `theme-colour-danger`).
6. **The kit's other colours** (#1054), with the same shared math and the same minimums the kit's contrast test holds the
   built-in themes to (`design/kitContrast.test.ts`, ADR-0102 item 6):
   - the fixed red as words (errors, a red text button) on each surface, and an agent's initial (`text`) on its badge (the
     agent colour mixed 22 % into `surface-1`), at **4.5:1**: "Text on Mint agent badge is 4.4:1; it needs at least 4.5:1 to be
     readable" (`theme-error-contrast` with `theme-colour-*`, `theme-avatar`);
   - each **control indicator** at **3:1** (WCAG 1.4.11): a switch's track, thumb and outline, the chosen option's ring (red
     on a risky choice), a selected card or count's ring, a filled button, the focus ring on every coloured background, a role's icon on its chip
     (mixed 22 %). These pairs are one table (`themes/tints.ts` `INDICATORS`), which the kit test holds equal to the rules in
     `app.css`. A failing pair names the control and says it must be seen, not read: "Icon: Idle on Idle icon background is
     2.7:1; it needs at least 3:1 to be seen" (`theme-error-control`, `theme-control-*`). A pair that item 4 or another
     warning already asks at least as much of is reported there only (an accent below 3:1 on a surface is already refused).

7. **Format 2** (`themes/file.ts`): every problem of 1–3 at every level of the file, named by where it is
   (`dark.regions.sidebar.text`): an unknown field (`theme-error-unknown-field`), a value of the wrong kind
   (`theme-error-field-color`, `-size`, `-list` with the allowed words, `-line-height`, `-switch`, `-group`), a token in the wrong
   group ("`text-16` goes in `shape`", `theme-error-moved-token`), an `inherits` that is not a built-in theme, a theme with no
   scheme (`theme-error-no-look`), anything the floor refuses, and any unsafe value. Then, for each scheme it has (a problem names
   its scheme when the theme has both, `theme-issue-in-scheme`):
   - the pairs of 4 on its own tokens;
   - each region it colours: its text on its surface at **4.5:1**; on a surface of its own, also secondary text and the accent at
     4.5:1 and every state at **3:1** (`theme-error-contrast`, or `theme-error-seen` for a state);
   - the terminal's text on its background at **4.5:1**, when the theme sets either;
   - warnings only: each of the 16 colours the theme sets, or all of them on a background it sets, at **3:1** on the terminal's
     background (not the one the background stands in for: black on a dark background, white on a light one; programs choose these
     colours); the cursor at 3:1; the selection (drawn at 40 % over the background) at 1.3:1 against it, so one that is the
  background is noticed; a syntax colour it sets at 4.5:1 on the code blocks; and 5 and 6.

A theme with any problem in 1–4 or the refusals of 7 is refused as a whole: an import adds nothing, a stored one is dropped (the defaults stand in),
an edit is not applied. A problem in 5 or 6 is a **warning**: the theme is accepted, saved and shown, and the warning is listed.
These pairs came after people had saved themes under 1–4 alone (a state colour needs only 3:1 on a surface but 4.5:1 as words
on its own background), and refusing would drop such a stored theme on the next load.

## Built-in themes

| Theme | Scheme | What it is |
| --- | --- | --- |
| **Dark** | dark | the default dark (`tokens.css`) |
| **Light** | light | the default light (`tokens.css`) |
| **High contrast** | dark | pure black and white, visible lines, a yellow accent; text at least 15:1 |
| **Large text** | dark | Dark with text and spacing a quarter larger, softer corners |
| **Slate** | dark | a cool blue-grey with a teal accent |
| **Sand** | light | a warm paper tone with a terracotta accent and softer corners |

![High contrast](../screenshots/adr68-theme-high-contrast-dark.png)
![Large text](../screenshots/adr68-theme-large-text-dark.png)
![Slate](../screenshots/adr68-theme-slate-dark.png)
![Sand](../screenshots/adr68-theme-sand-light.png)

Built-in themes cannot be changed; a test keeps Dark and Light equal to `tokens.css` and every built-in theme readable.

## Starter themes

![The visual theme chooser in dark mode](../screenshots/theme-chooser-dark.png)
![The visual theme chooser in light mode](../screenshots/theme-chooser-light.png)
![The theme picker with its Starter themes group (Dark theme list, dark)](../screenshots/starter-themes-picker-dark.png)
![The same in light](../screenshots/starter-themes-picker-light.png)
![Gruvbox on the window, dark](../screenshots/starter-theme-gruvbox-dark.png)
![Gruvbox, light, from the same file](../screenshots/starter-theme-gruvbox-light.png)
![Catppuccin Mocha](../screenshots/starter-theme-catppuccin-mocha-dark.png)
![Dracula](../screenshots/starter-theme-dracula-dark.png)
![Rosé Pine](../screenshots/starter-theme-rose-pine-dark.png)
![Tokyo Night](../screenshots/starter-theme-tokyo-night-dark.png)
![Arctic](../screenshots/starter-theme-arctic-light.png)
![Aurora](../screenshots/starter-theme-aurora-dark.png)
![Blossom](../screenshots/starter-theme-blossom-light.png)
![Cobalt](../screenshots/starter-theme-cobalt-dark.png)
![Ember](../screenshots/starter-theme-ember-dark.png)
![Lavender](../screenshots/starter-theme-lavender-dark.png)
![Lilac](../screenshots/starter-theme-lilac-light.png)
![Mint](../screenshots/starter-theme-mint-light.png)
![Sepia](../screenshots/starter-theme-sepia-light.png)

The six built-in themes and 24 starter themes bring the collection to 30 themes. The starter themes include 19 project-original
palettes and five well-known palettes (roadmap M5.2).
Each starter theme is an ordinary format-2 file in `apps/web/src/themes/starter/` (no other field, no
`inherits`), read by the same function as an imported file (`themes/starter.ts`), so the floor and every readability rule hold as
for any theme; a test asks each file for **no problem and no warning** in the web window's reader and in the terminal client's. They
are listed in the picker under **Starter themes** and in the command palette (**Theme: ⟨name⟩**), read-only like a built-in theme
(**Customize** makes a copy), and named after their palette. The editor says where the colours come from and under which licence.

| Theme | Schemes | Colours from (licence) |
| --- | --- | --- |
| **Abyss** | dark | project-original (Apache-2.0) |
| **Arctic** | light | project-original (Apache-2.0) |
| **Aurora** | dark | project-original (Apache-2.0) |
| **Blossom** | light | project-original (Apache-2.0) |
| **Bonsai** | dark | project-original (Apache-2.0) |
| **Catppuccin Mocha** | dark | `catppuccin/palette` (MIT) |
| **Cobalt** | dark | project-original (Apache-2.0) |
| **Dawn** | light | project-original (Apache-2.0) |
| **Dracula** | dark | `dracula/dracula-theme`'s palette, its terminal colours from `dracula/alacritty` (MIT) |
| **Ember** | dark | project-original (Apache-2.0) |
| **Graphite** | dark | project-original (Apache-2.0) |
| **Gruvbox** | dark, light | `morhetz/gruvbox` (MIT/X11; `gruvbox-community/gruvbox`'s LICENSE, MIT) |
| **Lagoon** | light | project-original (Apache-2.0) |
| **Lavender** | dark | project-original (Apache-2.0) |
| **Lilac** | light | project-original (Apache-2.0) |
| **Meadow** | light | project-original (Apache-2.0) |
| **Mint** | light | project-original (Apache-2.0) |
| **Mulberry** | dark | project-original (Apache-2.0) |
| **Parchment** | light | project-original (Apache-2.0) |
| **Porcelain** | light | project-original (Apache-2.0) |
| **Rosé Pine** | dark (main) | `rose-pine/rose-pine-palette` (MIT) |
| **Sepia** | light | project-original (Apache-2.0) |
| **Tokyo Night** | dark (Night) | `folke/tokyonight.nvim`, its generated `extras/lua/tokyonight_night.lua` (Apache-2.0) |
| **Ultraviolet** | dark | project-original (Apache-2.0) |

The original palettes use the project's Apache-2.0 licence and were designed directly against the readability floor. For the
five third-party palettes:

- **Only the palette's published colour values** (hex) are used, never a theme file of the project, and no colour is mixed, lightened
  or made up. The notices are in `THIRD_PARTY.md`.
- **Each slot takes the palette's own colour for it**: the background, panels and raised surfaces from its background shades, text
  and secondary text from its text shades, the accent and the five states from its accents (mostly working blue, needs you yellow,
  finished green; Rosé Pine, which has no green, has Iris for working and Foam for finished, Dracula has cyan for working). The
  terminal's 16 colours are the project's own terminal colours where it publishes them (Catppuccin's `palette.json`, Tokyo Night's
  generated extras, Dracula's Alacritty port), else the palette's colour of that hue (Gruvbox's normal and bright shades, Rosé Pine's
  roles). Code colours follow the palette's own roles where it names them, else its colour of a similar hue.
- **When that colour does not pass a rule, the next colour of the same palette that does is used** (for example a lighter grey for
  "bright black", the text colour for secondary text on Rosé Pine, Dracula's darker `#21222c` for the panels). When none does, the
  slot is left out of the file and Agora's own colour stands in, as for any theme: Tokyo Night's and Dracula's comment colour in code,
  Gruvbox light's "finished" and four of its code colours.
- **A variant is left out when it cannot pass with no warning this way**, rather than bending a rule. For each, what its own colours
  miss, and what is still short with Agora's colours standing in for those (as the readers report it):
  - **Catppuccin Latte**: no accent of the palette but Mauve and Red reaches 4.5:1 on its backgrounds (Green 2.9:1, Yellow 2.3:1,
    Blue 4.3:1 on Base), so every state colour and most terminal colours would be Agora's; even then Agora's bright green and bright
    yellow are 2.9:1 and 2.5:1 on Base, and "needs you" on its own background 3.8:1.
  - **Tokyo Night Day**: its text is 4.5:1 on its main background only (3.9:1 on its sidebar colour) and no secondary text or accent
    of the palette reaches 4.5:1 on its surfaces; with Agora's standing in, secondary text on raised surfaces is still 4.0:1 and the
    accent on the panels 4.2:1.
  - **Nord**: Agora's fixed red is below 4.5:1 on every Nord background but its darkest (3.6:1 on Polar Night 1); with every surface
    Polar Night 0, the red on its error background is 3.6:1 and Frost blue as "working" on its own background 3.7:1.
  - **Rosé Pine Dawn**: its secondary text (Subtle, 4.0:1) and three of the five states (Gold 2.0:1, Foam 3.1:1, Love 3.8:1) are
    below the minimum on Base; with Agora's standing in, Agora's bright yellow is 2.6:1 on Base and "needs you" on its own background
    4.4:1.
  - **Solarized dark**: Agora's fixed red on its error background is 4.4:1 even with every surface Base03, its darkest; its accents
    on their own backgrounds are 4.4:1 or less, and secondary text on them 4.3:1.
  - **Solarized light**: no accent reaches 4.5:1 on Base3 (Orange 4.2:1 at best) and its body text is 4.1:1 (Base01, its emphasised
    text, 4.9:1 on Base3 but 4.3:1 on Base2); with Agora's standing in, "needs you" on its own background is 4.2:1.

  So Gruvbox remains the only light variant among the third-party palettes: the other light variants are lower in contrast than the
  rules allow. The project-original palettes add ten light choices.

`scripts/screenshots-starter-themes.mjs` takes the shots above from the demo window (the picker's list is shown in the page by giving
the select a size for the shot: a browser cannot capture the list the system draws).

## Choosing (per person)

- The title bar has one **Choose theme** palette button in the web window and desktop shell. It opens a modal gallery with a miniature
  preview of every theme, Dark and Light tabs, the current choice, and **Follow the system**, **Dark** and **Light** mode controls.
  Choosing a card applies it immediately and keeps the gallery open for comparison. Scroll the cards to see more and select
  **Done** to close the gallery.
- Each scheme has **its theme**: "Light theme" and "Dark theme" in the editor. Following the system switches between the two
  by itself (Sand by day, Slate by night); Dark or Light shows that scheme's theme. Choosing a theme of the other scheme while
  one is forced moves to that scheme, so the choice shows. A format-2 theme with both schemes is listed under both, and choosing
  or importing it makes it the theme of both.
- The editor's two lists group the themes as **Built-in**, **Starter themes** and **Your themes**.
- The command palette (Ctrl K) lists **Theme: ⟨name⟩** for every theme, and **Edit theme**.
- Stored in this browser's `localStorage`: `agora.theme` (the scheme, as before this rung), `agora.theme.choice.v1` (the two
  themes) and `agora.themes.v1` (the person's own themes, at most 20). Storage that is missing, full or blocked costs only the
  memory: every choice still holds for the visit. Another tab of the same browser applies a change at once (live reload).
- **Reset to defaults** goes back to following the system with Light and Dark; the person's themes are kept.
- The two themes chosen (not the scheme) and the person's own themes are kept the same on their devices
  ([below](#keeping-themes-the-same-on-your-devices)); whether this browser follows the system or is forced dark or light stays its own,
  since a phone and a laptop often differ.

## Keeping themes the same on your devices

ADR-0113 decision 6, build step 4. The person's **own themes**, the **packs** they imported and the **theme choice** follow them to
every device they sign in to on the same hub, so the phone and the laptop agree.

![Settings, This device, Appearance: "Keep my themes the same on my devices" (dark)](../screenshots/adr113-theme-sync-setting-dark.png)
![The same in light](../screenshots/adr113-theme-sync-setting-light.png)

**On the hub.** The hub keeps one value per person under the key `themes` (`UserSettingsGet` and `UserSettingsSet`,
[protocol.md](../specs/protocol.md#per-person-settings-adr-0113)): opaque bytes it never parses, checks or shows, at most **256 KiB**
(`settings.too_large` over that), keyed by the session's person, so nobody reads or writes another person's. Each write raises the
value's **version**; a write based on an older version is refused (`settings.stale`), so two windows never overwrite each other unseen.
A person's writes are limited to **60 a minute** across their windows (`rate.limited` past that, with one Activity row however many
are refused). Writes are rows of the person's Activity ("Kept their themes the same on their devices") with the outcome and nothing of the value; a
read writes none, and neither does a stale write (it changed nothing). Removing a person forgets their value. The hub's database moves to schema 17 for it.

**In the window.**

![The theme editor's "Your packs and devices": a pack imported on another device, to open or forget (dark)](../screenshots/adr113-theme-synced-packs-dark.png)
![The same in light](../screenshots/adr113-theme-synced-packs-light.png)

- **Settings > This device > Appearance**, "**Keep my themes the same on my devices**": on by default (a switch per browser,
  `agora.themes.sync.v1`). Under it, where it stands: kept the same; off; this browser only, because the window has no hub (the demo)
  or an **older hub** that does not know the messages (it refuses one as `auth.forbidden` with the action `None`, and the window asks
  no more on that connection); too large;
  saved by a newer version; or the hub could not be reached (tried again at the next change). The switch is off and disabled without a
  hub, with an older hub, and in **safe mode**, which never syncs: nothing is read from the hub and nothing is written.
- **When.** On sign-in (once the window has its first view), the window reads the value, checks it, merges it with this browser and
  writes the result when it differs. A change here (a theme made, edited, imported, installed, added from a pack or deleted; a pack
  kept or forgotten; a theme chosen) is written **1.5 seconds** after the last one. The window reads again whenever it **comes back
  into view** and when its connection comes back, which is when another device's change shows. A write refused as stale reads again, merges and writes again (three times
  at most).
- **Checked on load.** Every theme in the value goes through the theme file's reader (`readTheme`, the rules on this page), every pack
  through the pack reader (`readPack`). **Something the window cannot read is not applied**: the theme editor's **Your packs and
  devices** section names it with its problems and offers **Remove from my devices**; until then it is written back as it was, so a
  theme from a newer window survives an older one. A value written by a newer version of the format is not read and never written over.
- **Merging** is the union by a **lasting id** each theme and pack gets where it is first made (24 hex characters, kept beside the theme
  in `agora.themes.v1` with the time of its last edit; never in the theme's file, so an export is the theme alone):
  - A theme on both sides takes the **newer edit**; on a tie, this browser's.
  - **No theme of this browser is dropped silently.** A theme deleted on another device leaves a dated mark (`agora.themes.removed.v1`,
    the newest 200); the mark removes the theme here only when it is newer than the theme's last edit here, so a theme edited here
    since stays (and comes back on the other device). A theme deleted here does not come back from another device.
  - A theme that does not fit this browser's **20** waits: it is listed in Your packs and devices ("doesn't fit here"), stays in the
    value for the devices that have room, and arrives here once one of yours is deleted.
  - **The choice** (each scheme's theme, a built-in by its id or one of yours by its lasting id) takes the newer one
    (`agora.theme.choice.at.v1`), and the hub's on a tie, so two devices never keep writing their own over each other's. A choice made
    before this version counts as made at time 1, the defaults (never chosen) as 0, so a new device takes an existing one's choice. A newer choice naming a theme this browser cannot show keeps this browser's choice on screen and stays
    the newer one for the devices that can.
  - Times are each device's clock, believed only up to the hub's now (from its `Welcome`) plus **5 minutes**: a later edit time, removal
    or choice time (a device whose clock is far ahead) is cut back to that and written back cut back, so it cannot stay "the newer one"
    for long. A clock that is behind still makes its edits look older than they are.
- **One browser, two people.** The hub sends each person's random **tag** with their value, and the browser keeps the tag it last
  synced with (`agora.themes.sync.tag.v1`). When someone else signs in to the same browser and their value has another tag (or none),
  the browser merges **nothing** of its own into it: their themes, removals, choice and packs replace what this browser held (the
  previous person's are in their own value already) and nothing goes up. **Two tabs at once:** the last to sign in owns the browser's
  store. A tab signed in as someone else that sees the browser claimed for another tag (its storage event, or before any read, write or
  change it would apply) stops: it reads, writes and applies nothing, and Settings says "Theme sync paused: another person signed in to
  this browser in another tab" until that tab is signed in again or reloaded. **What remains:** the first time a browser syncs at all (no
  tag kept yet, for example the first sign-in after updating), what it held is merged into the value of whoever signs in, as when one
  person turns this on; a browser two people used before this version merges into whoever signs in first.
- **Packs.** A pack imported with **Import pack…** is kept as the file it was (`agora.packs.v1`, the 8 most recent; a ninth replaces the
  oldest), and is synced like a theme. **Your packs and devices** lists them with what each holds: **Open…** shows the same Import pack
  preview with nothing ticked (nothing of a pack is applied on another device until the person picks its parts there), and **Forget**
  removes it from every device (what was added from it stays). The pack's themes the person added are already among their themes.
- **Too large.** Before writing, the window measures the value; over 256 KiB it writes nothing, changes nothing here and says so.

**The value**, format 1 (JSON, written in one order, by lasting id, so the same themes give the same bytes on every device and a value
read and written back unchanged is not written):

```json
{
  "agora-theme-sync": 1,
  "themes": [{ "uid": "3fa1…", "edited": 1795600000000, "file": { "agora-theme": 2, "name": "Harbour", "…": "…" }, "catalog": { "id": "harbour", "version": 1 } }],
  "themesRemoved": [{ "uid": "77c0…", "at": 1795600100000 }],
  "packs": [{ "uid": "a01b…", "added": 1795600000000, "file": { "agora-pack": 1, "name": "Harbour set", "…": "…" } }],
  "packsRemoved": [],
  "choice": { "dark": "u:3fa1…", "light": "sand", "at": 1795600000000 }
}
```

`apps/web/src/themes/sync.ts` (the value, the merge and when it reads and writes), `kept.ts` (kept packs), `SyncPanels.tsx` (the switch
and the section). `scripts/e2e-theme-sync.mjs` runs it in two real browsers signed in as the same person on a real hub: a theme
imported in one and a choice made in the other reach the other, and nothing is asked of any origin but the hub's.

## The theme editor

A panel of the window (panel id `theme`, `layout.md`), so the window it styles stays in view. It opens beside the pane in
focus (as a tab there when the layout has no room for another pane), from **Edit theme** in the palette, the **Appearance**
section of **This device**, or a pane's **Show** menu. On a phone, which has no panels (ADR-0067 rung 2, [phone.md](phone.md)),
the same two ways open it as a screen of its own (`#/theme`) with Back.

- **Preview** (at the top): a small copy of each part of the window (the Agents list, tabs, a pane bar, an agent reply, your
  message, a code block, a card, a button, a text field, the status line) drawn by the window's own rules, and a few lines of a
  terminal in the theme's palette, so a change shows there as on the window.
- The fields are grouped: **Colours** (of the scheme on screen; a theme with one scheme offers "Add Light colours" or "Add Dark
  colours", a theme with both says which it shows), **Parts of the window** (three optional colours per region), **Terminal**
  (the four and the 16), **Code colours**, **Text and shape** (fonts, weight, line height, ligatures, sizes, corners) and
  **Window** (the chrome values, as segmented choices and sliders). An optional colour left empty shows the colour it follows and
  can be cleared again.
- **Always the same**: the floor as locked rows, each with its reason.

- Built-in themes are shown read-only with **Customize**, which makes a copy "⟨name⟩ (mine)" and selects it.
- On one of your themes every token is a field: colours as a swatch and a hex field, fonts as text, sizes as sliders. A change
  that is valid is saved and shown on the whole window at once; a value still being typed is kept in the field while the window
  keeps the last good one.
- **Readability** lists every pair below its minimum; while any is listed the change is **not applied**, so the window never
  becomes unreadable while it is being edited. The warnings of 5 and 6 (text or a control that is hard to see only in the
  kit's colours) are listed under "Applied, but some text or controls would be hard to see in places"
  (`theme-contrast-warning`): the change is applied. When surface pairs fail too (not applied), they follow under "Also hard
  to see in places" (`theme-contrast-also`). An imported theme with such warnings is added; when it is the theme of the
  scheme not on screen, its warnings are listed under the buttons after "Added. When it is shown, some text or controls
  would be hard to see in places" (`theme-import-warning`), since its Readability is not shown. With nothing to warn about,
  Readability says "All text and controls are easy to see (WCAG AA)".

  ![Applied, with a warning about Idle on its own background (Dark)](../screenshots/theme-tint-warning-dark.png)
  ![The same in Light](../screenshots/theme-tint-warning-light.png)
  ![Applied, with warnings about an agent's badge and Idle's icon on its chip (Dark)](../screenshots/theme-kit-warning-dark.png)
  ![Idle's icon on its chip in Light](../screenshots/theme-kit-warning-light.png)

  All of these are taken by `scripts/screenshots-themes.mjs` from the demo window (it turns the "Needs you" card off for
  the shot, `agora.attention.mode`).
- **Import…** reads a theme file of either format (refused with its reasons, or added and selected for every scheme it has),
  **Import from another app…** reads another terminal's or editor's colours ([below](#import-from-another-app)), **Export** saves
  the theme on screen as `theme.json` in format 2, **Delete theme** removes one of yours (its schemes fall back to Light and Dark),
  **Reset to defaults** as above. **Import pack…** and **Export as pack…** are [theme packs](#theme-packs).

## Import from another app

![A VS Code colour theme, shown with what changed on the way, before it is added (Dark)](../screenshots/theme-import-dark.png)
![A Ghostty theme with a light background, with the theme check's warnings (Light)](../screenshots/theme-import-light.png)
![The VS Code theme added: the window and the terminal in its colours, the request card as always](../screenshots/theme-import-added-dark.png)
![The Ghostty theme added as the light theme](../screenshots/theme-import-added-light.png)

**Import from another app…** in the editor reads the colours of another app's colour file and makes a format-2 theme of them
(roadmap M5.1; web and desktop window only: no CLI, no hub, no schema). Nothing is added until **Add theme**: first a dialog shows
the terminal's colours, every note on what was left out or guessed, and what the theme check says.

| App | File | Read |
| --- | --- | --- |
| iTerm2 | `.itermcolors` (an XML property list) | `Ansi 0 Color` … `Ansi 15 Color`, `Background`, `Foreground`, `Cursor` and `Selection Color`: float components with `Alpha Component` and `Color Space` (sRGB; Display P3 converted to sRGB; anything else read as sRGB, with a note). A file with only `(Dark)` and `(Light)` sets uses the dark one. |
| Alacritty | `.toml`, and the `.yml` of before 0.13 | `colors.primary` `background`/`foreground`, `colors.cursor.cursor`, `colors.selection.background`, `colors.normal.*`, `colors.bright.*`; `#rrggbb` or `0xrrggbb`; `CellForeground`/`CellBackground` as the text or background colour; YAML anchors on maps (`colors: *scheme`). |
| Ghostty | a theme file (`key = value`, often no extension) | `palette = N=#rrggbb` for 0–15, `background`, `foreground`, `cursor-color`, `selection-background`; hex with or without `#`. |
| Windows Terminal | one `schemes` entry, a list of them, or a whole `settings.json` (the first scheme) | `background`, `foreground`, `cursorColor`, `selectionBackground`, the eight colours (`purple` is magenta) and their `bright` forms. |
| VS Code | a colour theme (`colors`, `tokenColors`) | `terminal.ansi*`, `terminal.background`/`foreground` (else `panel.background` or `editor.background`, and `editor.foreground`, with a note), `terminalCursor.foreground`, `terminal.selectionBackground`; code colours from `tokenColors` scopes (`keyword`, `string`, `comment`, `constant.numeric`, `entity.name.function`, `entity.name.type`, `entity.other.attribute-name`, `meta.preprocessor`, most specific first). |

- **Which app** is told by the file's extension, else by its first lines. JSON files may have comments and trailing commas.
- **One scheme.** The background decides it by the theme check's own rule (dark when white reads better on it). The theme fills that
  scheme only, and the dialog says so; the other scheme's theme stays as it is.
- **The window's colours.** Format 2 has no rule that makes the window's colours from a terminal palette (a scheme's colours left out
  come from Dark or Light), so the import makes them, conservatively, and says so: the background is the window's, the panels and
  raised surfaces a little of the text mixed into it, the lines a little more; the text is the terminal's, the secondary text the text
  mixed toward the background; the accent and the states are the palette's blue, yellow and green. Each is held to the pair the
  theme check asks of it (4.5:1, 3:1 for a state, on every surface), mixed toward the text until it is. Regions, chrome, text and
  shape are left to their defaults. When the file's own text is not readable on its background, the window keeps Agora's colours and
  the theme check refuses the terminal's pair, as it would in a hand-written file.
- **Transparency.** Format 2 has none: a see-through colour is mixed over the background; the selection's transparency is dropped
  (Agora draws the selection see-through itself); a see-through background is drawn solid. Each is a note.
- **Notes**: the scheme, how the window's colours were made, colours the file does not give (Agora's own are used), colours that
  could not be read, and what the file has that a theme has no place for (the first six names, then how many more). A name from a
  file is shown as text, cleaned of control and direction characters.
- **The same door as a theme file.** The converter builds the format-2 file key by key from closed lists, never copying a key or value
  of the input through; then the file goes through the store's own import, so the size cap, unknown fields, the floor, unsafe pieces,
  the readability pairs and the 20-theme limit hold exactly as for a theme file. A name from the file (the VS Code or Windows Terminal
  `name`, else the file name) loses control and direction characters and the pieces a theme refuses (`;`, `<`, `url(` …).
- **Bounded.** A file over 256 KiB is refused before it is read. The readers (`themes/import/read.ts`) are small hand-written subsets
  (TOML, YAML, Ghostty's lines, JSON with comments) that read line by line, keep keys in a `Map`, and stop at 4096 keys; a property
  list goes through the browser's own XML parser, which loads no external DTD, entity or stylesheet (the dialog's screenshot script
  checks that importing an iTerm2 file makes no request, and that one naming a DTD and a stylesheet on the page's own server fetches
  neither), after the text is refused if it declares an entity or a DTD of its own, nests deeper
  than eight, or has more than 20 000 elements. A fuzz test throws random and mutated files at every converter.
- Taken by `scripts/screenshots-theme-import.mjs` from the demo window.

## Theme packs

A **pack** is one JSON file that bundles one or more themes, saved layouts ([layout.md](layout.md)) and keybindings
([keybindings.md](keybindings.md)), so a look can be shared with the arrangement and the keys it was made for
([ADR-0113](../adr/0113-deep-themes-and-community-themes-stay-data.md) decision 5, build step 2). Like a theme it is data and is never
run: a pack never contains or references code, never names an agent, and **never installs or enables anything**, plugins included.

![Import pack: what the pack holds, nothing ticked, so nothing changes yet (dark)](../../apps/web/visual/pack-import-desktop-dark.png)
![The same in light](../../apps/web/visual/pack-import-desktop-light.png)
![Merging the pack's keys: which of yours they replace and which keys would do two things (dark)](../../apps/web/visual/pack-keys-desktop-dark.png)
![The same in light](../../apps/web/visual/pack-keys-desktop-light.png)
![Export as pack: pick the themes, saved layouts and keys that go in (dark)](../../apps/web/visual/pack-export-desktop-dark.png)
![The same in light](../../apps/web/visual/pack-export-desktop-light.png)

### Format, version 1

```json
{
  "agora-pack": 1,
  "name": "Harbour set",
  "themes": [{ "agora-theme": 2, "name": "Harbour", "inherits": "slate", "dark": { "tokens": { "accent": "#5cc8c2" } } }],
  "layouts": [{ "name": "Review", "layout": { "version": 1, "root": { "type": "split", "direction": "row", "ratios": [0.5, 0.5],
    "children": [{ "type": "pane", "id": "p1", "panel": "changes", "bind": { "follow": "selection" } },
                 { "type": "pane", "id": "p2", "panel": "plugin.clock.face" }] } } }],
  "keybindings": { "version": 1, "bindings": [{ "command": "palette.open", "keys": ["ctrl+p"] }] }
}
```

- `agora-pack`: the format version, `1`. A higher number is refused with "made by a newer version"; nothing of it is guessed.
- `name`: the pack's name, 1 to 60 characters, with the rules of a theme's name (control and text-direction characters removed, none
  of `url(`, `@import`, `;`, `}`, `expression(`, `<`). Shown as text.
- `themes` (optional): up to **20** themes, each a whole [format 2](#format-version-2) theme file. A format-1 theme in a pack is
  refused (a window writes format 2).
- `layouts` (optional): up to **50** saved layouts, each `{ "name": …, "layout": <layout version 1> }` with a name of 1 to 60
  characters, unique in the pack.
- `keybindings` (optional): one [keybindings file](keybindings.md#the-keybindings-file-version-1), version 1.
- No other field is allowed. A pack with nothing in it is refused.

### Reading a pack (validation)

One function (`apps/web/src/themes/pack.ts` `readPack`) reads every pack. It never throws, and like a theme file it is refused
**whole** for any problem, with **every problem listed at once** (at most 50, then how many more), each one saying which theme or
layout it is in:

1. Larger than **256 KiB** (a theme file's cap): refused before it is read. Not JSON, not an object with an integer `agora-pack`
   ≥ 1, or newer: refused.
2. An unknown field at the top, a missing or bad name, a part that is not a list (or, for keybindings, an object), more than 20
   themes or 50 layouts.
3. **Themes** are read by the theme reader (`readTheme`) with all its rules: unknown fields at every level, closed value kinds,
   unsafe pieces, the floor and readability. A theme's warnings are shown in the preview, as for an import.
4. **Layouts** are held to the closed set of layout version 1 at every level (`version` and `root`; a pane's `type`, `id`,
   `panel` and `bind`; a tab group's `type`, `active` and `children`; a split's `type`, `direction`, `ratios` and `children`),
   then read by the layout reader (`parseLayout`) with its rules and limits (32 panes, 6 levels). The window reads its own stored
   layouts more leniently (it keeps a newer client's fields); a pack is someone else's file. In a pack:
   - a panel is named **only by id**: a built-in's word (`terminal`), or a plugin's `plugin.<plugin id>.<panel id>` (ADR-0114),
     or a newer client's of the same shape; any other text is refused, so nothing else reaches the "unknown panel" placeholder;
   - a pane may follow what needs you or the selection, but **never names an agent** (`bind.agent` is refused): an agent is on
     someone's own machine;
   - a pane id is letters, digits, `_` and `-`, at most 64.
5. **Keybindings** are read by the keybindings reader (`readKeysValue`) with all its rules, so **the fixed keys stay fixed**: the
   safe-mode key, moving around the window, a terminal's copy and paste and editing keys can't be bound, `window.safe-mode` can't be
   named, and no command answers a request (`request.*`, `approval.*`, …). The part is capped at the keybindings file's own 64 KiB.

### Import pack

**Import pack…** in the theme editor takes a pasted pack or a file (a file over the cap is refused before it is read) and shows a
**preview**: the pack's name, then each part with what it holds: each theme with its schemes (and how many things are hard to see
in it), each layout with its number of panes, the keybindings with how many commands they set (and those this window does not
have). **Nothing is ticked**, and nothing is applied until the person ticks a part and confirms; Back and Cancel change nothing.

- **Themes** are **added** to the person's own themes. None is chosen: the theme on screen stays, and an added one is chosen as any
  other. When the ticked themes do not all fit (20 of your own), the preview says how many fit and nothing is added.
- **Layouts** are **added** as saved layouts (the layout menu's **Saved layouts…**). A name that is taken gets a number
  ("Review (2)"), so a saved layout is never replaced; when they do not all fit (50 saved layouts), nothing is added.
- **A layout that names a panel this window does not draw** says so in the preview. A plugin's panel shows only once that plugin is
  installed and allowed ([plugins.md](plugins.md)), which a pack never does; until then the pane shows the "unknown panel"
  placeholder, as does a panel of a newer client.
- **Keybindings** are **merged**: each command the pack names takes the pack's keys; every other command keeps yours. Before
  anything is kept, a confirmation shows **which of your own keys they replace** (yours, then the pack's) and **which keys would
  then do two things** (a key on two commands, or one that starts another's chord), with Back. A merge that could not be kept as a
  keybindings file (over 200 entries or 64 KiB) is refused, so the person's own keys are never lost on the next load.
- Every part ticked is checked before any is kept, so a part that cannot be applied leaves all of them as they were. The last step
  says what was added: the themes, the layouts with the names they were saved under, the keys merged.

### Export as pack

**Export as pack…** in the theme editor asks for the pack's name (the theme on screen's, or "My pack" for a built-in theme) and
which **themes** (your own first, then the built-in ones; the one on screen is ticked), **saved layouts** and **your keys** go in,
and saves `theme-pack.json`. A pane bound to one of your agents is written unbound (its panel stays; following what needs you or the
selection is kept), since a pack names panels only. What is written is read back before it is saved, so a pack over the cap is
never written. Reading an exported pack gives the same themes (as Export writes them), the same layouts (unbound, in the layout
reader's normal form) and the same keys.

### Where packs are read

- **The web and desktop window:** the theme editor's Import pack… (above). A pack imported there is kept and offered on the person's
  other devices ([above](#keeping-themes-the-same-on-your-devices)), never applied there until they pick its parts.
- **The terminal client:** a pack in its themes folder gives its themes ([below](#the-terminal-client)). Its layouts and keys are
  the window's and are listed as not used there.
- **The community catalog** holds themes only: a catalog entry that is a pack would need a `kind` in the index and in
  `ThemeCatalogEntry` (a schema change, so an ADR) and the hub's check to read layouts and keys, which it does not (see "Not yet").

## The terminal client

`agora tui` reads the same theme files, format 1 and 2, with the same rules (ADR-0113 decision 4, build step 3;
`crates/agora-tui/src/theme/`).

![The terminal client in Slate, a request's box on top: drawn in the terminal's own colours, as are the "needs you" marks](../screenshots/adr113-tui-slate-dark.png)
![The same in Sand (light), on a dark terminal](../screenshots/adr113-tui-sand-light.png)
![Slate on a 16-colour terminal: every pair mapped to one that reads](../screenshots/adr113-tui-slate-16-colours.png)
![The sample theme above: its terminal palette, and lines under the tab bar and above the footer (pane-borders: always)](../screenshots/adr113-tui-harbour-chrome.png)
![The theme list on the sample theme, drawn in it, with what the terminal client does not use](../screenshots/adr113-tui-theme-list.png)
![A file that cannot be used, with every reason](../screenshots/adr113-tui-theme-list-refused.png)

These are drawn by the client into ratatui's test screen and shown in a browser (the terminal's own colours as #1c1c1c and
#d0d0d0), so the cells are exactly what the client writes.

- **Where themes come from.** The six built-in themes above, the [starter themes](#starter-themes) (the same files, compiled in,
  `crates/agora-tui/src/theme/starter.rs`; a row per scheme, and the list says where the colours come from), and every `*.json` file
  directly in the **themes folder**:
  `themes` in the Agora folder, so `~/.agora/themes` (or `$AGORA_HOME/themes`), beside the client's device key. The theme in use
  is kept in `tui-theme` there (one line: `terminal`, `built-in <id>`, `starter <dark|light> <id>`, `file <dark|light> <file name>` or
  `pack <dark|light> <theme number from 0> <file name>`). The first entry,
  **Your terminal's colours**, is no theme: the client draws exactly as it did before themes (the default).
- **Theme packs.** A `*.json` file in the folder that is a [theme pack](#theme-packs) gives each of its themes as rows of the list
  (one per scheme, as a theme file). Under a pack's theme the list says which pack and file it is in ("In the pack "Harbour set"
  (file set.json): theme 1 of 2") and what else the pack holds that this client does not use ("Also in the pack, not used in the
  terminal client: 2 saved layouts; keys for 3 commands"): this client has no layout model or keybindings yet. The pack's own
  fields are held to the closed set and its name to the name rules, and every theme is read by the theme reader, so one bad theme
  makes the whole pack a file that "can't be used", with every reason. The layouts and keybindings are checked only for their form
  (a list of at most 50 objects, an object with a `bindings` list), since their rules are the window's; a pack the window refuses
  for a layout or a key can still give this client its themes, each of which passed the theme rules. A pack with no theme is a file
  that can't be used, and says its layouts and keys are for the web window. Live reload covers a pack's file too.
- **Choosing.** `t` in the agents list, or **Theme…** in the commands (Ctrl K), opens the theme list: whole-screen, drawn in the
  theme the keyboard is on, so a theme shows before it is chosen. Enter uses it at once and keeps it for the next start; q or Esc
  goes back. A theme with both schemes is two rows ("Harbour · dark", "Harbour · light"): a terminal has no light or dark setting
  to follow, so the person picks one. A viewer may choose a theme too: it changes nothing on the hub.
- **Live reload.** The file in use is asked for its size and time once a second (nothing is read unless they changed) and read
  again when it changed. A file that can no longer be used leaves the terminal's own colours with a notice ("press t to see why");
  the choice is kept, so the theme comes back when the file is fixed. The folder itself is read when the list opens. No file-watch
  dependency is used.
- **Reading.** The file's bytes are decoded as the web's import decodes them (a leading byte-order mark dropped, a byte that is not
  UTF-8 read as U+FFFD). One function that never panics reads a file and reports every problem at once, in the web's words
  (`locales/<lang>/tui.ftl`, `tui-theme-error-*`): the size cap (asked of the file system before anything is read), unknown fields
  at every level, closed value kinds, the floor, unsafe pieces, and the readability pairs of items 1–4 and 7, so **a file is
  accepted or refused alike in both clients**, including for a region this client does not draw. The warnings are the ones about
  what this client draws: the 16 colours and the selection on the terminal's background. The web's warnings about its widget kit
  (items 5 and 6), the cursor and code colours are not repeated. The list shows a refused file as "can't be used", with every
  reason below it, and never offers it.
- **Colours.** The client detects what the terminal shows as it always has (`COLORTERM`, `TERM`, the Windows host; `NO_COLOR`
  turns colour off, and with it every theme: the list says so). With 24-bit colour every value is used as it is. With the
  256-colour palette each colour is the nearest entry of the cube or grey ramp (never the first 16, which the user's own terminal
  theme decides); with 16 colours, the nearest of the 16 by xterm's values. Colours are mapped **in pairs**: each text colour is
  the nearest entry that still reads at 4.5:1 on its mapped background (a line at 1.2:1), so a truecolor theme stays readable on
  a 16-colour terminal; the mapping is computed once per theme, never per frame. A test holds every built-in theme and a sample
  file to this in all three modes.
- **What it draws.** The agents list in `sidebar`, the tab row in `tab-bar`, the footer in `status-line` (each region's
  background and text, the secondary text on it), the inspector in the panels' colours, the window and the welcome in the
  background, the selected row and the tab in front in the accent with the text on accent (instead of inverted), the dividers in
  the lines' colour. Terminal panes take the theme's terminal palette, or without it the pane's background, the theme's text and
  the scheme's 16 colours: a program's default colours and its 16 colours, and the selection at 40 % over the background (inverted
  where it would not stand out). With 256 or 16 colours each of the 16 (but the one the background stands in for) is one that can
  be seen on the mapped background at 3:1: with 16 colours the terminal's own red, green… where it can, else the nearest that can
  (on a light theme's white, the palette's own white, yellow and green cannot).
- **Chrome it draws.** `pane-borders` (auto: the dividers between panes; always: also a line under the tab bar and above the
  footer; off: blank dividers), `line-width` (0 none, 1 `│`, 2 `┃`), `pane-gap` (above 0 the divider column is a blank gap) and
  `density` (roomy: a column clear on each side of the agents list's and the inspector's text; compact and normal are how the
  client always draws).
- **Not used here, and listed as such.** Under the selected theme the list says "Not used in the terminal client: …" for what that
  theme sets that this client cannot draw: text size, spacing and corners (the terminal's cells), fonts and text style (the
  terminal's font), the other regions' colours, the cursor colour (the user's terminal's own), code colours, and the tab bar's
  position, the tab style, the state display, icons, shadows and motion. Large text, for example, is Dark's colours here.
- **The floor.** The request box (the box that asks for an answer, `app::dialog`) is cleared to the terminal's own colours and
  drawn as before, whatever the theme; every "needs you" mark (the `!` and the words in the agents list, a waiting tab's label,
  the tab row's count, the welcome's list, the theme list's banner) keeps its fixed colour and bold and sits on the terminal's own
  background, so no theme's surface can hide it (on the tab in front, and in the bar of the pane in focus in a tiled layout,
  that is its `!`, the rest being in the accent). A test compares those cells, character, colours and attributes, with and
  without each built-in theme and the sample file, in all three colour modes. The other screens (the board, files, review, the
  folder picker and the plain list when the output is piped) still draw in the terminal's own colours.

## The community catalog

Community themes, off until the hub's owner turns them on ([ADR-0113](../adr/0113-deep-themes-and-community-themes-stay-data.md)
decision 6, build step 5). The **hub** fetches the catalog, and only when its owner asks; a window never loads anything from another
origin, and its page policy is unchanged.

![The catalog in the theme editor: Harbour installed with a newer version, Meadow previewed (Dark)](../screenshots/adr113-theme-catalog-dark.png)
![The same in Light](../screenshots/adr113-theme-catalog-light.png)

### On the hub: `agora hub themes`

Run on the hub's machine by its owner (the hub folder is the owner's alone); each change is in the audit trail as the owner's
(`theme_catalog.on`, `theme_catalog.off`, `theme_catalog.update` with `ok`, `partial` or `failed`).

- `agora hub themes on` / `off`: whether windows show the catalog. Off when never set.
- `agora hub themes status`: on or off, how many themes, when and from where they were fetched.
- `agora hub themes update [--url URL]`: fetches the catalog's index and every theme it lists, checks them, and keeps the accepted
  ones in the hub's database, replacing the catalog it had in one step. `--url` is the address of the catalog's `index.json`; it is
  remembered for the next update. **The project's catalog repository does not exist yet**, so until the owner gives an address there is
  none: the built-in default is a placeholder (`https://themes.agora.invalid/index.json`, a name that can never resolve) and `update`
  without `--url` says so and fetches nothing. Exit 0 when every theme was kept; 1 when the catalog could not be read (nothing
  changed) or some theme was refused (the others are kept, and each refusal is printed with its reason, a refused theme's with every
  reason the theme rules give, in the same words as the terminal client's theme list). Updating works while the catalog is off, so the
  owner can look before turning it on.

What the hub checks (`crates/agora-hub/src/theme_catalog.rs`):

- **The address.** `https://`, or `http://` only to `localhost` or `127.0.0.1`.
- **Redirects are not followed.** A 3xx answer is a refusal (of the whole update for the index, of that theme for a file). The owner
  gives the final address (for a GitHub repository, its `raw.githubusercontent.com` address); a redirect could otherwise take the hub
  somewhere the owner never named.
- **Where a theme file comes from.** Only a relative path under the index's own folder, on the same scheme, host and port: letters,
  digits, `.`, `_`, `-` and `/`, ending in `.json`, with no empty, `.` or `..` part. The index cannot send the hub to another address.
- **Sizes and time.** The index at most 256 KiB, each theme at most 256 KiB (the theme rules' own cap), at most 200 themes and 4 MiB of
  themes in all (themes past it are refused). Connecting, and each read or write, at most 10 seconds; the whole update at most 2
  minutes. These hold inside TLS too: every read and write of the socket gets only the time left, so a server that sends a TLS record
  a byte at a time is given up at the deadline. A theme file that cannot be reached in time, or whose server answers 429 (too many
  requests) or a 5xx status, stops the update with nothing changed, so a network or server failure half way never drops themes that
  are fine. Any other status refuses that theme alone.
- **What is printed** from the catalog (ids, field names, a theme's problems) has every control character and every invisible or
  direction-changing character written as a visible `\u{…}`, so a catalog cannot send the owner's terminal escape sequences.
- **The index** is read strictly: a field it does not know, at the top or in an entry, refuses the index or that entry (below).
- **Each theme** must have the SHA-256 the index gives (of the file's bytes as served), be UTF-8, and pass the theme rules: the
  terminal client's reader ([above](#the-terminal-client)), which accepts and refuses a file exactly as the web window does. The file
  is kept exactly as fetched.

### In the window

Once the owner turned the catalog on, the theme editor shows a **Catalog** section to every person signed in to the hub, whatever
their role (a viewer too: installing changes only their own browser). It asks the hub once when the editor opens (no polling); nothing
is shown while the catalog is off, the hub is an older one, or the window is not connected. The demo window (no hub) shows a sample
catalog of two themes.

- Each theme shows its name, author and licence, as text, and its version, or **Installed**, or **Version N available**.
- **Preview** fetches the theme from the hub, checks it with the window's own rules, and draws a small sample in the theme's own
  colours (its panels, text, accent, a working mark and a terminal); nothing changes until the person installs it. A theme the window
  refuses lists its reasons and cannot be installed.
- **Install** copies the theme into the person's own themes (the 20-theme limit applies, with its message), selects it as an import
  does, and remembers where it came from: the catalog's id and the version, kept beside the theme in `agora.themes.v1`
  (`{"id", "file", "catalog": {"id", "version"}}`), never in the theme's file, so an export is the theme alone.
- **An installed theme never changes by itself.** When the catalog has a higher version, the theme says **Version N available** and
  offers **Update to version N**, which replaces that theme in place (same place in the list, still chosen; a scheme the new version no
  longer has goes back to the default). Editing an installed theme keeps where it came from.
- The editor asks when it opens: a catalog the owner turns on while the editor is open shows the next time it is opened.

`scripts/e2e-theme-catalog.mjs` runs all of this in a real browser on a real hub: the owner's `update` from a catalog served on the
machine, nothing shown while off, the list once on, a preview and an install, and no request to any origin but the hub's.

### The gallery

[ADR-0120](../adr/0120-a-gallery-for-the-community-catalogs.md): both community catalogs on one screen, **Gallery** (`#/catalog`,
`#/catalog/plugins`), opened from Settings (Appearance), the theme editor's catalog section, the Plugins dialog and the command palette
(`gallery.open`). Not offered in safe mode.

![The Gallery: two themes drawn in their own colours, Harbour's details open (Dark)](../screenshots/adr120-gallery-dark.png)
![The same in Light](../screenshots/adr120-gallery-light.png)
![On a phone (Dark)](../screenshots/adr120-gallery-phone-dark.png)

- **Themes** are cards, each drawn in the theme's own colours from its file (fetched from the hub three at a time and checked by the
  window's rules, as Preview does; a theme the window refuses says so and cannot be installed). **Install** (or **Update to version N**)
  works in one step as in the editor; an installed one offers **Use**. **Details** opens a side panel with the theme large and, for a
  theme with both, a Dark / Light switch.
- **Plugins** are cards with the first screenshot the hub kept (or the name's first letter), what the plugin asks to see, and its state
  on this hub (not added, added, or a newer version out). **Details** shows every screenshot with its alt text, the description, and the
  command the hub's owner runs (`agora hub plugins install|upgrade ID`) with **Copy the command**; nothing installs a plugin from the
  window. A screenshot is shown only as an image: the window checks its first bytes again and draws it from a `data:` address with its
  fixed type.
- **Search** matches every typed word in the name, author or description. **Sort**: Popular, New, Name, Author; it opens on Popular when
  any entry has a popularity, else New when any has a day, else Name. Popularity is the number the catalog publishes (format 2), shown
  as "Popularity N" with the hint that Agora counts nothing; New uses the day it was first listed.
- Without a hub (or from a hub older than the gallery) it says the catalog cannot be shown; while a catalog is off it says so. The demo
  window shows the sample theme catalog.

### The protocol

`ThemeCatalogList` and `ThemeCatalogGet { id }` (`schema/agora/v1/commands.proto`, [protocol.md](../specs/protocol.md#the-community-theme-catalog-adr-0113)),
answered by the hub itself. The list answers `on: false` and nothing else while the catalog is off (not a refusal, so a window that opens
the editor writes no row in the owner's Activity); `ThemeCatalogGet` answers the file as text, or `theme.catalog_off` or
`theme.not_found`.

### The catalog repository the hub expects

A separate public repository (ADR-0113's proposed answer), to be created by the maintainer. Every addition is reviewed, and its CI
checks the schema, the readability floor and that there are no extra fields (decision 6). The hub needs only static files:

```
agora-themes/
  index.json                 the index the owner's --url points at
  themes/
    harbour.json             one theme file per entry, format 1 or 2 (this page)
    meadow.json
  README.md, LICENSE, CI     the repository's own; never fetched
```

`index.json`:

```json
{
  "agora-theme-catalog": 1,
  "themes": [
    {
      "id": "harbour",
      "name": "Harbour",
      "version": 2,
      "author": "Ada Example",
      "licence": "MIT",
      "sha256": "<64 lowercase hex digits: the SHA-256 of themes/harbour.json as served>",
      "file": "themes/harbour.json"
    }
  ]
}
```

- `agora-theme-catalog`: the index format, `1`. A higher number is refused whole ("update Agora"). No other top-level field.
- `id`: lowercase letters, digits and `-` (not first), at most 40; unique. It names the theme across versions.
- `name` (at most 60 characters) and `author` (at most 100): plain text, no control, format or direction-changing character, no
  space at either end.
- `version`: a whole number from 1 to 1 000 000 that only grows; a higher number is a newer version, offered to people who installed
  an older one.
- `licence`: an SPDX licence identifier or expression (letters, digits, `.`, `-`, `+`, spaces and brackets), at most 64.
- `sha256`: of the file's bytes exactly as served (no line-ending conversion on the way).
- `file`: the relative path described above.
- No other field in an entry; an entry with one, or with a field missing or of the wrong kind, is refused alone.

**Format 2** (ADR-0120, read by this version and later; an older hub refuses a format-2 index whole) is `"agora-theme-catalog": 2` with
two optional fields per entry, for the gallery's sorting:

- `added`: the day the catalog first listed the theme, `YYYY-MM-DD` (a real day from 1970); not changed by later versions. A day after
  the day of the update (UTC) refuses that entry.
- `popularity`: a whole number from 0 to 1 000 000 000 that the catalog's maintainers set when they publish (for example the stars of
  the theme's source repository at release time). Agora never counts installs and never reports anything; this number is the catalog's.

The repository's CI should run the same checks before merging: the index's rules above, each file's SHA-256, and the theme rules,
for example with `agora tui`'s reader or the web window's (`apps/web/src/themes/file.ts`), so a merged theme is never refused by a hub.

## Not yet (later steps of ADR-0113)

- **Syncing** a pack's saved layouts and keybindings as the person's own (step 4 keeps the pack file and offers it on their other devices;
  layouts and keys stay per browser), and a change on another device showing at once (the window reads again when it comes back into
  view; the hub sends nothing by itself).
- **The catalog repository itself** (step 5's hub side is built, [above](#the-community-catalog)): the maintainer creates it; the hub
  then fetches it with `--url`. **Packs in the catalog** are left out: an entry that is a pack needs a `kind` field in the index and in
  `ThemeCatalogEntry` (a schema change, which needs an ADR), the hub's check to read a pack's layouts and keys (it has only the theme
  rules), and the editor's install to show a pack's preview.
- In the terminal client: a pack's layouts and keybindings (it has neither yet; ADR-0068 rung 5).
- More than one icon set (the `icons` key has one value, `lines`); ligatures in terminals; background images (left open by
  ADR-0113, whose answer is: not in format 2).
- The phone layout's own parts as regions: it uses the tokens and the chrome values that apply to it.
- A themes folder watched on disk for the desktop app: the browser has no folder, so "live reload" there is the editor and other
  tabs (the terminal client has its folder, [above](#the-terminal-client)).
- In the terminal client: the board, files, review and folder-picker screens in the theme's colours, and following a light or
  dark terminal by itself.
