# Themes: write a theme file

A **theme** is a small JSON file of colours, sizes and a few closed choices. It never holds code, CSS, `url()` or anything from the
network. The web window and the terminal client read the same file, with the same rules, so a file is accepted or refused alike in
both. Write format 2 (below); format 1 files are still read.

## Where it goes

- **Terminal client:** save it as `themes/NAME.json` in the Agora folder (`$AGORA_HOME`, else `~/.agora`). Every `*.json` file
  directly in that folder is listed (at most 200, by name). In `agora tui`, press `t` in the agent list (or **Theme…** in the
  commands): a file that cannot be used shows every reason. The choice is kept in `tui-theme` (one line: `terminal`,
  `built-in ID` or `file dark NAME.json` / `file light NAME.json`). The file in use is read again when it changes.
- **Web window:** the person opens **Edit theme** (command palette, Ctrl K) and chooses **Import…**. The window keeps its themes in
  the browser, not in a folder.

## Format 2

```json
{
  "agora-theme": 2,
  "name": "Harbour",
  "inherits": "slate",
  "dark": {
    "tokens": { "accent": "#5cc8c2" },
    "regions": { "sidebar": { "surface": "#0b1016" } },
    "terminal": { "background": "#0b1016", "foreground": "#d8e1ea" },
    "syntax": { "keyword": "#ff9e8f" }
  },
  "light": { "tokens": { "accent": "#0b6e69", "on-accent": "#ffffff" } },
  "shape": { "radius-m": "10px" },
  "text": { "font-mono": "'JetBrains Mono', monospace", "weight": 400, "line-height": "normal", "ligatures": true },
  "chrome": { "pane-borders": "always", "pane-gap": 8, "tab-style": "pill", "density": "compact" }
}
```

No other field is allowed, at any level. Every value is optional except `agora-theme`, `name`, and a look (`dark`, `light` or
`inherits`).

- `agora-theme`: `2`. A higher number is refused as "made by a newer version".
- `name`: 1 to 60 characters.
- `inherits`: a built-in theme to start from: `dark`, `light`, `high-contrast`, `large-text`, `slate`, `sand`. Values the file
  leaves out come from it. Without it they come from the default dark or light theme.
- `dark`, `light`: the colours of that scheme. A theme with both fills both. Each holds:
  - `tokens`: `surface-0` (background), `surface-1` (panels), `surface-2` (raised surfaces), `border`, `text`, `text-muted`,
    `accent`, `on-accent`, and the agent states `state-working`, `state-needs-you`, `state-finished`, `state-idle`,
    `state-unknown`.
  - `regions`: parts of the window, each with a `surface`, `text` and `line` colour: `sidebar`, `tab-bar`, `pane-bar`, `status-line`,
    `cards`, `inputs`, `buttons`, `agent-entries`, `person-entries`, `code-blocks`.
  - `terminal`: `background`, `foreground`, `cursor`, `selection`, and the 16 colours `black`, `red`, `green`, `yellow`, `blue`,
    `magenta`, `cyan`, `white`, `bright-black`, `bright-red`, `bright-green`, `bright-yellow`, `bright-blue`, `bright-magenta`,
    `bright-cyan`, `bright-white`.
  - `syntax`: code colours: `keyword`, `string`, `comment`, `number`, `function`, `type`, `attr`, `meta`.
- `shape`: `text-16` (the base text size, 16–24 px), `space-1` (the spacing unit, 3–6 px), and corners `radius-xs` (0–16 px),
  `radius-s` (0–24 px), `radius-m` (0–32 px), `radius-l` (0–40 px). A size is `"18px"` or the number `18`.
- `text`: `font-ui` and `font-mono` (installed font names separated by commas, at most 200 characters of letters, digits, spaces
  and `_ . , ' " -`), `weight` (300, 400 or 500), `line-height` (`"normal"` or a number from 1.2 to 1.8), `ligatures` (`true` or
  `false`).
- `chrome`: the window's frame, below.

A **colour** is `#rgb` or `#rrggbb`. Named colours, `rgb()` and transparency are refused. Other text sizes and spacings
(`text-12`, `space-2`, ...) follow from `text-16` and `space-1` and are refused.

## Chrome

| Key | Values | What it does |
| --- | --- | --- |
| `pane-borders` | `auto` (default), `always`, `off` | lines between and around panes |
| `pane-gap` | 0–16 px (default 0) | space between panes |
| `line-width` | 0–2 px (default 1) | the width of the window's lines |
| `tab-bar` | `top` (default), `bottom` | where a pane's bar sits |
| `tab-style` | `underline` (default), `filled`, `pill` | how the tab in front looks |
| `density` | `normal` (default), `compact`, `roomy` | the height of bars, rows and cards |
| `state-display` | `symbol` (default), `dot`, `word` | how an agent's state shows in lists and tabs |
| `icons` | `lines` (default) | the icon set |
| `shadow` | `soft` (default), `none`, `strong` | menus and overlays |
| `motion` | `system` (default), `reduced`, `off` | animation |

The terminal client draws `pane-borders`, `pane-gap`, `line-width` and `density`, the `sidebar`, `tab-bar` and `status-line`
regions, and the terminal colours. It lists what else a theme sets as "not used in the terminal client".

## What a theme can never change

A file is refused as a whole when any of these is true:

- It names what needs the person, by these names anywhere: `danger`, `on-danger`, `diff-removed`, `approval-card`, `approval`,
  `focus-ring`, `focus`, `needs-you`, `attention`, `plugin-marker`.
- A value (the name too) contains `url(`, `@import`, `;`, `}`, `expression(` or `<`.
- It is larger than 256 KiB. (At most 50 problems are listed, then how many more.)
- Text is hard to read: `text`, `text-muted` and `accent` need 4.5:1 on each surface, `on-accent` 4.5:1 on `accent`, and each
  state colour 3:1 on each surface. A region it colours needs its text at 4.5:1 on its surface; a region with a surface of its own
  also needs `text-muted` and `accent` at 4.5:1 and each state at 3:1 on it. The terminal's `foreground` needs 4.5:1 on its
  `background` when the theme sets either.

Some pairs are only warnings (the theme is used, the warning is listed): each of the 16 terminal colours at 3:1 on the terminal's
background, the selection, and colours on the web window's soft backgrounds.

## Example

The smallest useful theme: Slate with a different accent and rounded tabs.

```json agora-theme
{
  "agora-theme": 2,
  "name": "Slate teal",
  "inherits": "slate",
  "dark": { "tokens": { "accent": "#5cc8c2" } },
  "chrome": { "pane-borders": "always", "tab-style": "pill" }
}
```
