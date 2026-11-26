# 0113. Deep themes and community themes, and a theme is always data
Status: Accepted
Date: 2026-11-25

Accepted: 2026-11-25 (the maintainer, in the merger chat: "Accept and build"). The open questions were not answered; the proposed answers stand.

Extends ADR-0068 decision 5 (themes are data) and `docs/ux/themes.md` (format version 1). Answers ADR-0068's open question on
syncing themes per person. Touches ADR-0024 (simplicity and visual quality), ADR-0102 (colour rules in `visual-language.md`) and
the hub's page policy (`crates/agora-hub/src/web.rs`).

## Context

The maintainer wants the window to be restyled much further than it can be today, as far as the most customizable agent workspaces go: deep visual customization, themes
made and shared by a community, and plugins (ADR-0114).

**What exists.** Theme format 1 is built in the web window (`themes.md`). A theme is a JSON file of about 25 tokens: surfaces, text,
accent, the five state colours, two font lists, the base text size, the spacing unit and four corner radii. Every value is checked
(colours `#rrggbb`, sizes in ranges, font lists from a safe alphabet), the readability pairs must pass WCAG AA or the theme is not
applied, and tokens are set one by one with `style.setProperty`, so a value can never become CSS of its own. Themes are imported
and exported as files and kept in the browser. The terminal palette follows the scheme, the terminal client has no themes
(ADR-0068 rung 5), and nothing is shared through the hub.

**What others do** (research of 2026-11-25, sources in the PR):
- **In the most customizable agent workspaces, flexibility comes from chrome and layout, not from more colours.** One such
  terminal app has 19 colour tokens with light and
  dark variants. Its desktop app has a pure-data theme (`background`, `foreground`, `selection`, `cursor`, 16 ANSI colours,
  optional panel overrides). What looks different between its gallery entries is borders, gaps, padding, bar positions, status
  segments and per-block backgrounds, set in layouts.
- **Zed, Helix and Zellij themes are pure data against a schema.**
- **Every theme incident found was code labelled as a theme, not bad theme data:**
  - the 2025–2026 VS Code "GlassWorm" colour themes declared JavaScript entry points;
  - a Neovim colorscheme is a script;
  - in one agent workspace, a layout can load a code extension.
- **CSS is not inert.** Attribute selectors with `url()` read page data out character by character, and Mermaid CVE-2022-31108
  turned a theme variable into CSS injection. CSS can also hide or restyle a warning.

## Decision

1. **A theme stays data.**
   - A theme, or a theme pack (decision 5), never contains or references code.
   - It never contains raw CSS, `url()`, remote fonts or images from the network.
   - Every value is one of a closed set of kinds (colour, size in a range, a value from a list, a font list of installed fonts)
     and is applied per token as today.
   - A file with an unknown field or kind is refused whole, with every problem listed (as in format 1).
2. **Format 2 goes deeper where the look actually comes from.** `"agora-theme": 2`; format 1 files are still read.
   - **Both schemes in one file:** `light` and `dark` sections, and `inherits` (a built-in theme's id) so a theme can be a few
     lines.
   - **Terminal palette:** the 16 ANSI colours, background, foreground, cursor and selection, used by every terminal pane.
   - **Regions:** surface, text and line colours per region:
     - sidebar
     - tab bar
     - pane bar
     - status line
     - cards
     - inputs
     - buttons
     - the conversation's agent and person entries
     - code blocks
     - syntax colours (today derived; a theme may set them, and they still default to the derived ones)
   - **Chrome, as values from lists, never CSS:**
     - pane borders (auto, always, off)
     - pane gaps (0–16 px)
     - line width (0–2 px)
     - tab bar at the top or bottom, and tab style (underline, filled, pill)
     - density (compact, normal, roomy)
     - how a state is shown (dot, symbol, word: always with its word in the tooltip and to screen readers)
     - one of the built-in icon sets
     - shadow strength (none, soft, strong)
     - motion (follow the system, reduced, off)
   - **Text:** the UI and mono font lists (installed fonts only), weight, line height, and ligatures on or off.
3. **What no theme can change (the floor).** A theme can make the window look like anything, but it can never make the window
   lie or hide what needs the person:
   - **Not themeable:**
     - the danger red and the removed-line colour (`--danger`, `--diff-removed`);
     - the "needs you" treatment (shape, word and position, not only colour);
     - the frame and text of an approval card, including the action it asks about;
     - the focus ring;
     - the plugin marker (ADR-0114).
   - **Floors that hold under any theme:** text no smaller than 12 px and hit targets no smaller than the platform minimum.
   - **The readability check covers every new pair:** each region's text on its surface, the terminal's foreground on its
     background, and states on every surface. A failing theme is shown with its problems and not applied (as today).
     ANSI colours other than the foreground are only warned about, since programs choose them.
4. **Every client reads the same file.**
   - **Web and desktop window:** CSS variables, as today.
   - **Phone layout:** the same tokens.
   - **Terminal client:** maps colours to what the terminal supports (truecolor, 256 or 16) and the chrome values it can draw
     (borders, gaps, density). It ignores the rest, visibly, in its theme list.
5. **Theme packs: a theme with its layout and keys.**
   - A pack is one JSON file bundling one or more themes, saved layouts (`layout.md`) and, once ADR-0068 rung 4 exists,
     keybindings.
   - A layout in a pack names panels only by id. A panel the window does not have (a plugin's panel, ADR-0114, or a newer
     client's) shows the existing "unknown panel" placeholder. A pack never installs or enables anything.
6. **Sharing.**
   - **Files, as today:** import, paste, drag-drop and export, plus "Export as pack".
   - **Per person through the hub:** the person's themes, packs and choice, so the phone and the laptop agree.
     - The hub keeps them as an opaque, size-capped (256 KiB per person) settings value and never reads them; every client
       validates on load.
     - This needs a schema message, added append-only.
   - **A community catalog,** off until the owner turns it on:
     - **Where themes live:** a public repository of theme and pack files under the project.
     - **Review:** every addition is reviewed, and its CI checks the schema, the readability floor and that there are no
       extra fields.
     - **Fetching:** the **hub** fetches the index and the files on the owner's command (`agora hub themes update`, like
       `agora hub prices update`). The window never loads anything from another origin, and its page policy is unchanged.
     - **Installing** copies the data. An installed theme changes only when the person picks a newer version.
7. **The editor grows with the format.** It gets every new token and chrome value, a live preview of each region (terminal
   included), the floor shown as locked rows with the reason, and "Export as pack". The panel stays `theme` (layout.md).

## Alternatives considered

- **Raw CSS themes** (Obsidian): the most freedom, and the reason people like it. Rejected: CSS can read page data out and can
  hide or fake an approval or warning. Obsidian needs a page policy and still cannot stop restyling.
- **Themes as code** (Neovim colorschemes) or **themes shipped as extensions** (VS Code): rejected; the theme incidents found were
  all of this kind.
- **Only more colour tokens:** cheap, but it misses what makes the most customizable workspaces look different (chrome and layout).
- **The browser fetches the catalog:** rejected; it needs the page policy opened to another origin, and the hub already fetches
  outside data for prices.

## Build order

1. Format 2 in the web window: both schemes, terminal palette, regions, chrome values, the floor and its checks, editor rows.
2. Packs (themes, layouts and keybindings). Done: `docs/ux/themes.md`, "Theme packs": the pack format, Import pack… with a preview
   and Export as pack… in the theme editor, and the terminal client reading a pack's themes from its themes folder.
3. The terminal client reads format 2 (ADR-0068 rung 5). Done: `docs/ux/themes.md`, "The terminal client".
4. Per-person sync through the hub (schema message, fixtures). Done: `docs/ux/themes.md`, "Keeping themes the same on your devices":
   `UserSettingsGet` and `UserSettingsSet` (the hub keeps each person's value opaque, at most 256 KiB, versioned; hub schema 17), the
   window's merge by lasting id with removal marks, packs kept as files and offered through the Import pack preview, and "Keep my themes
   the same on my devices" in Settings > This device.
5. The catalog repository and `agora hub themes update`. Hub side done: `agora hub themes on|off|status|update`, `ThemeCatalogList` and
   `ThemeCatalogGet`, and the Catalog section of the theme editor (`docs/ux/themes.md`, "The community catalog", which also gives the
   repository layout the hub expects). The catalog repository itself is the maintainer's to create.

## Verification

- **Refused:** a theme with any of `url(`, `@import`, `;`, `}`, `expression(`, `<` in any value, or with an unknown field, is
  refused, and no `style` property is set.
- **The floor:** a theme that sets the danger colour, the approval card's colours or the focus ring is refused with the reason.
  A theme that passes leaves the approval card pixel-identical in a screenshot test.
- **Readability:** fails for each new pair below AA; ANSI colours only warn.
- **Round trips:** format 1 files open unchanged. A format 2 file exported and imported is identical.
- **Terminal client:** a truecolor theme on a 16-colour terminal maps every colour to a readable one.
- **Sync:** a settings value over 256 KiB is refused by the hub. A value a client cannot read leaves the built-in theme in use
  and shows why.
- **Screenshots** in both default themes and one community theme (ADR-0024).

## Open questions

- **Background images from a local file**, stored with the theme as data and size-capped. Proposed: not in format 2.
- **Who may install from the catalog:** every person for themselves, or only the owner? Proposed: every person, once the owner
  has turned the catalog on.
- **Where the catalog lives:** the project's own repository, or a separate one? Proposed: separate.
