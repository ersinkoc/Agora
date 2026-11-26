# Visual language and simplicity

Two requirements that pull against each other: the product must look striking, and it must never feel
complicated. The answer is the same for both: a small number of strong visual ideas, applied consistently,
with everything else hidden until it is needed (ADR-0024).

## 1. Simplicity

### Three layers of depth (progressive disclosure)
| Layer | Question it answers | What is visible |
| --- | --- | --- |
| **Glance** | "Does anything need me?" | Home: attention strip + fleet of agent tiles. Nothing else. |
| **Focus** | "What is this agent doing?" | One agent full-size, its terminal or chat, one context panel. |
| **Deep** | "Let me inspect or change details" | Files, review, mesh, task board, usage, settings; opened on demand. |

A new user should be able to live entirely in Glance and Focus. Deep is never required to get value.

### UI vocabulary is not engineering vocabulary
Users see plain words; internal terms stay in code and docs (`glossary.md` lists both).

| Internal | In the UI |
| --- | --- |
| Node | Machine |
| Hub | (invisible; "Your Agora") |
| Worktree / branch | Workspace (branch shown on hover) |
| Integration level L0–L3 | Never shown; only "Terminal" and "Chat" views |
| Approval | Needs your OK |
| Lease | Locked by ⟨agent⟩ |
| Mesh | Team chat |
| Agent state `blocked` | Needs you |
| Agent state `done` | Finished |

### Complexity budget
- At most **one primary action** per screen.
- At most **two visible side panels** at once; others are reached through tabs or the command palette.
- Navigation is at most **two levels** deep (project → agent). Machines are a filter, not a level.
- **Every setting must justify itself.** A setting exists only when no default works for most users;
  "settings are a failure of defaults".
- **Zero-config start.** Agent kinds are detected, names generated, colors assigned.
  The user chooses machine, project/folder and agent; creating a separate working copy is optional (ADR-0052).

### Measurable goals
- **Time to first agent:** from install to a running agent in the window ≤ 60 seconds.
- **First-glance test:** a new user identifies which agent needs them within 5 seconds of opening the window.
- **Usability sessions:** at least five users per phase exit from Phase 3 onward, findings filed as issues.

## 2. Visual identity

### Principles
- **Dark-first, light-equal.** Developers live in dark themes; light theme gets the same care.
- **Calm surfaces, vivid state.** Neutral, low-contrast chrome; color is reserved for agent state and the
  single brand accent. When everything is calm, the one agent that needs you is impossible to miss.
  Code (the file browser and the review screen) is content, not chrome: it gets muted syntax colours, derived from the
  theme's own colours mixed toward its text colour (`--syntax-*` in `app.css`), never as loud as a state colour. A format-2 theme
  may set its own syntax colours ([themes.md](themes.md), ADR-0113); they are checked for readability on the code blocks.
  The review screen's changed lines are content too, and they are the one content colour that is not derived from the theme:
  `--diff-removed` (red, `#f87171` on dark and `#a82117` on light, in `tokens.css`) is fixed and not a theme token, so a theme file
  cannot set it. A deleted line is red, an added line is green (the Finished green), amber stays "needs you" and is never used
  for a deletion. A changed line says so by shape as well as colour: a `-` or `+` glyph and a 2 px bar on its left edge. The red
  reads at 4.5:1 or better on every built-in surface and on its own tinted row; the web tests check it. This documents a
  content-colour exception to "colour is reserved for state"; it does not change the state-colour rule.
  The same fixed red also names a chrome role, **risk** (`--color-risk` in `app.css`, the same colour as `--danger` below), for the sign of an agent that acts without asking
  (ADR-0059, ADR-0090 Amendment A), a lightning bolt with no words whose meaning is its tooltip and accessible name. It is never amber, so
  it cannot be read as "needs you" or as the quota chip (an amber warning sign that always shows its percentage); shape (bolt against
  triangle) and the words in the tooltip carry the difference too, never the colour alone.
- **State is color + shape + motion + text.** Never color alone (color-blind safe, WCAG 2.2 AA).
- **Colour is a surface, never a stripe** (ADR-0102). State and identity show as soft tinted backgrounds, hairline borders, pills and
  avatars, never as a thick bar on one side of a card, row or tile. The only edge bars are content conventions: the diff's 2 px line bar
  above, and a quote in a chat. A soft tint of a state colour is allowed as the background of the thing in that state.
- **One meaning per colour** (ADR-0102). Amber means only "needs you" (plus the quota warning sign, with its triangle and percentage).
  Red is the one **danger** role (`--danger`, the fixed `--diff-removed`, never a theme colour): a failed agent, errors shown to the
  person, deleted files and lines, destructive actions (Forget, Remove, Delete, a destructive dialog's primary button) and acting without
  asking (the risk sign, an Allow that lets an agent act without asking, a switch that turns it on). The accent is the brand: selection,
  primary actions, links. Green is finished and online; blue is working. Neutral facts (an OS, a version, a branch, a count of changed
  files) are grey tags, never a state colour.
  On the screens (redesign PR 3): errors and refused actions are `text-danger`, and so are a failed tool group in the chat, the options
  and notes of a start that would not ask, and the confirm box of a destructive or acts-freely choice (close a working agent, drop a
  queued message, turn asking off); a deleted file's mark is red; change counts are grey tags or chips; an offline machine's dot is the
  idle grey; a "run" step in the chat is the working blue; a list cut short or out of date is a neutral accent-tinted notice; a hint (two
  machines change one project, a device that does not trust this browser) is plain text. The terminal's search matches are the theme's
  accent mixed toward the terminal background (opaque, as xterm needs; recomputed on a theme change), the current one stronger with a
  full accent border, and the terminal's text stays at 4.5:1 on both. Besides "needs you" and the quota sign, amber stays in two places: a
  usage or context meter at 80 % or more (the quota sign's family) and the muted syntax colours mixed from it. A web test lists every
  file that uses the amber token or its classes, with a count and a reason, so a new use has to be one of these.

### Agent state language
| State | Color role | Shape | Motion |
| --- | --- | --- | --- |
| Working | cool accent | filled dot | slow, subtle breathing pulse (respects reduced motion) |
| Needs you | warm amber | ring with notch | gentle attention pulse, sorts to the top |
| Finished | green | check | single settle animation, then still |
| Idle | neutral | hollow dot | none |
| Unknown | muted | dashed ring | none |
| Failed | danger red (ADR-0102) | cross in a ring | none |

**Finished, not seen yet** (an agent that finished while the person was not looking at it) is the Finished green and its check, in a
tinted disc on a row or a tab and a **New** pill on a tile, never amber, a dot on the avatar or a bar; it is not a state of its own and goes
once the agent is seen ([main-window.md](main-window.md#sounds-and-a-finished-mark-that-lasts-until-seen)).

An agent that ended with an error or could not start is **Failed**; one stopped on purpose is not, and stays neutral. The widget kit's
status pill draws Failed in red, and so do Home's tiles and counts (redesign PR 2) and the phone's lists (PR 13); the screens that still
draw it in neutral ink (the agent list) switch over as they adopt the kit.

### Agent identity
- Each agent kind shows its brand mark (provider logos used per their trademark guidelines; models.dev provides
  provider logos for model providers).
- Each agent *instance* gets a stable generated accent and short name ("claude · amber", "codex · teal") so
  several agents of the same kind are distinguishable at a glance. The accent is drawn as the agent's avatar (a soft square with a ring
  and the kind's first letter) beside its name, never as a coloured edge of a tile or row.

### Signature visuals
- **Live tiles:** fleet tiles show a miniature live preview of the agent's terminal or latest chat line,
  rendered from cell snapshots at a low frame rate (performance budget applies).
- **Activity sparklines** per agent and project from output and token rates.
- **Attention strip:** one horizontal strip at the top listing everything that needs the user, oldest first.
- **Smooth focus transition:** a tile expands into the Focus view (shared-element transition ≤ 250 ms).

### Foundations
- **Typography:** one sans family for UI, one monospace for terminals and code; a short type scale
  (12/13/14/16/20/28).
- **Spacing:** 4 px base grid.
- **Type roles:** caption 12 (meta, tags, counts), body 13 (rows, text), strong 14 (card and section titles), title 16 (panel, dialog
  and group titles), page 20 (page titles, stat numbers), display 28 (the pairing number). Section titles are sentence case.
- **Radius and depth:** soft radii (xs 5, s 8, m 12, l 18, and full for pills), almost no shadows; depth from surface tone, not drop
  shadows. **One elevation** (ADR-0102): overlays only (dialogs, menus, the palette, toasts, tooltips) use the raised surface
  (`--surface-raised`) and the one overlay shadow (`--shadow-pop`); pages, panels, cards and tiles stay flat.
- **Where main is today:** the widget kit follows the no-stripe, one-meaning, one-elevation and sentence-case rules above. The screens
  reach them as they adopt the kit (the redesign plan's PRs 2–13). Home has (PR 2): its tiles are flat cards with an avatar and a status
  pill, its counts are stat tiles and what needs you is request cards, with no stripe, glow or moving foot line. Settings has (PR 8), and
  My machines and its forget dialog have (PR 9), their animated links gone. The Agents list, the pane tabs and bars, the rail and the file
  rows carry the avatar and a tinted selection with no bar (PR 7). The chat view, the board and the Usage panel are on the kit
  (PR 12; `docs/ux/screens/polish-redesign-*`): one status pill, the agent avatar in each turn, the acts-freely sign as a red-tinted tag
  beside Mode, board columns with a mark and hue, Usage stat tiles. Until the rest follow, existing overlays (except the command
  palette) use the older `--shadow`, other screens keep their selection bars, and the global `h2` is still an uppercase label. Amber
  means only "needs you" everywhere (PR 3).
- **Motion:** 150–250 ms, ease-out for entering, spring for layout changes; nothing decorative loops except
  the working pulse.
- **Icons:** one outline icon set, one stroke width.

## 3. Design system in code
- **Tokens first:** colors, type, spacing, radius, motion, and state colors defined once in `design/tokens`
  and generated for the web client (CSS variables), desktop, and TUI (truecolor/256-color mapping).
- **Themes are token files** (ADR-0068, `themes.md`): built-in Dark, Light, High contrast, Large text, Slate and Sand, plus
  the person's own, checked for WCAG AA contrast before they are shown (the derived colours below too, as a warning). Text and spacing sizes are rem-based so a theme's
  text size (and the browser's) scales them; components never use pixel font sizes.
- **Widget kit** (`apps/web/src/components/kit/`): one widget per job, and every new or changed screen reuses them instead of setting
  its own padding, border or colour:
  Button (primary, secondary, ghost, danger, danger-ghost; sm and md), StatusPill and Pill, Tag, AgentAvatar (the agent's colour and
  the first letter of its kind: identity lives here, not on a tile's edge), Card, ListRow (the current row is an accent tint, no bar),
  SettingRow, SegmentedControl (a radio group: arrows move and choose; red when its chosen item lets an agent act without asking), Toggle (a switch; red when it lets an agent act without asking),
  StatTile (only Needs you is tinted, and only above zero), Callout (a tinted box with an icon chip, never a side bar), EmptyState
  (icon, title, one line, at most one action; Home's first run, the one page-sized empty state, has a primary and a secondary action),
  CopyChip (a command or id in mono with Copy; where the clipboard is missing or refuses, it selects the text and says to copy it with
  the keyboard), Tooltip, KeyValue and ConfirmDialog (a question before something destructive: an overlay with a red icon chip, the safe
  answer first and focused, the destructive answer as a red button). They use tokens only and logical properties, grow to the 44 px
  touch target on a touch screen, and respect reduced motion.
- **Derived colours** (`app.css`, not theme tokens, so every theme, a user's too, gets them): a soft tint (`--tint-*`) and a hairline
  (`--line-*`) per role (accent, working, needs you, finished, danger, idle), `--danger` and its label colour `--on-danger` (both fixed
  per scheme in `tokens.css`, never the theme's on-accent, so a theme cannot make a filled danger button unreadable), `--surface-raised`,
  `--border-strong`. The chosen segment of a segmented control is ringed in the accent. Text on any tint, the red on every surface and an agent's initial on its badge reach 4.5:1 in every built-in
  theme, and control indicators (a switch's track and thumb, a filled button, a selected ring, the focus ring, a role's icon on its chip)
  reach 3:1; a test computes both from the real tokens. The theme editor computes the same pairs from one shared table
  (`themes/tints.ts`) for a person's own theme and warns about any below its minimum (`themes.md`); the test holds that table equal to
  the kit's rules. The `--line-*` hairlines are
  decoration around a tinted surface whose icon and words carry the meaning, never the only sign of a control or a state, so they are
  only held visible (about 1.5–2:1), not to 3:1.
- **Component gallery:** every component rendered in all states, both themes, and an RTL locale; this is the
  review surface for UI pull requests. The widget kit is its first section.
- **Visual regression tests:** screenshot tests for components and key screens in CI.
- **UI pull requests** include before/after screenshots or a short recording in both themes.

## 4. Brand
Logo, name, and identity (open question Q1) deserve a human designer. Coding agents implement the design
system; they should not invent the brand.
