# Visual pass, November 2026 (roadmap M1.4)

Every main screen of the web window, in the dark and the light theme, at a phone width (390 × 844, a touch screen) and a desktop width
(1440 × 900), measured against [visual-language.md](../ux/visual-language.md) (ADR-0024): design tokens only, the UI vocabulary, 44 px
touch targets on the phone, no text cut off, and contrast. The screenshots are in
[`docs/screenshots/visual-pass-2026-11/`](../screenshots/visual-pass-2026-11/) as `<screen>-<width>-<theme>.png`; they are the state
after the fixes of this pass.

## How the pictures were made

`scripts/screenshots-visual-pass.mjs` makes all 86 of them (its header has the commands). It also measures each shot and prints leads:
controls under 44 px on the phone, text cut off without an ellipsis, a page that scrolls sideways, internal words in visible text or in
a control's name, and text below 4.5:1 (3:1 when large) on its own background. The leads were checked against the pictures by eye.

- **Demo data, pinned clock** (the web app's own Vite with no hub, so the demo data stands in for one; the sign-in page from the built
  app as the hub serves it): the clock is fixed at 2026-11-20 10:00 UTC, the language at English, so these are the same on every run on
  the same fonts.
- **Live** (a throw-away hub and node from `scripts/browser-harness.mjs`, a temporary home and a free port; only a shell and
  `agora-fake-agent`): the first run, a terminal, a chat agent, the board, Team chat and a machine asking to join. Their times, port and
  folder names come from the run, so these move a little from run to run.

## Screens

Each row has four pictures (dark and light, desktop and phone) unless it says otherwise.

| Screen | File prefix | Source | What it shows |
| --- | --- | --- | --- |
| Sign-in | `signin` | built app | the code page of a window that is not signed in |
| First run | `first-run` | live | signed in, one machine, no agent: the welcome with Start an agent and Add a machine |
| Home | `home` | demo | several agents: needs you, working, finished, failed, idle, unknown, closed |
| Needs you card | `needs-you-card` | demo | the card that covers the window when an agent asks; on the phone the request in the bottom dock |
| Start an agent | `start-agent` | demo | the start window with machines and known folders |
| Approval | `approval` | demo | an agent that asks, its request in its pane and the Needs you list; on the phone the agent's screen |
| Changes (review) | `changes` | demo | an agent beside its changes; on the phone the changes dialog |
| Files | `files` | demo | the file browser and the file manager; on the phone the files dialog |
| Command palette | `palette` | demo | the palette opened (on the phone from Search) |
| Settings | `settings` | demo | Settings on demo data |
| My machines | `machines` | demo | three machines, one offline |
| Pairing | `pairing` | live | My machines open for new machines, with `agora join` asking from a second home: the card with its number |
| Theme editor | `theme-editor` | demo | the editor beside Home; on the phone its own screen |
| Theme import | `theme-import` | demo | the import dialog over the editor with a VS Code theme |
| Plugin panel | `plugin-panel` | demo | the sample plugin, loaded for development and granted, beside Usage. Desktop only: a phone has no panes |
| Plugin grant | `plugin-grant` | demo | a plugin package loaded from a file, before the person allows what it asks to see |
| Safe mode | `safe-mode` | demo | `?safe` with a plugin's pane in the layout |
| Terminal | `terminal` | live | a shell with a little output |
| Chat agent | `chat` | live | the fake agent's thinking, a tool call, a plan, and a request to run a command |
| Board | `board` | live | Backlog, Doing and Done over real tasks; on the phone the board's screen (`#/board`, from More, finding 15) with the board open |
| Team chat | `team-chat` | live | two messages between agents in the Team chat pane; on the phone its own screen (`#/messages`) |
| More (phone) | `phone-more` | demo | the phone's bottom bar with More open: the board, Team chat and Usage. Phone only |
| Usage (phone) | `usage` | demo | Usage as a phone screen of its own (`#/usage`). Phone only; on a desktop it is a side panel subject |

## Findings

"Fixed" means fixed in this pass; each fix is small and its before and after are named. A follow-up is too big for a pass like this one, or
needs a decision; it has a proposal instead.

| # | Screen | Problem | Rule | Where |
| --- | --- | --- | --- | --- |
| 1 | Safe mode | A plugin's pane showed its internal id: "Unknown panel “plugin.sample-agents.list”: this window cannot show it", in its bar, its body and its menu's name | UI vocabulary, no internal terms | Fixed: the bar says **Plugin panel**, the pane "Plugins are off in safe mode. Leave safe mode to see this panel." (or, outside safe mode, that the plugin is not added in this browser). `safe-mode-desktop-*` |
| 2 | Files | The file manager's tooltips said "Changing files needs your **approval** on the machine" | UI vocabulary ("Needs your OK") | Fixed: "needs your OK". `files-desktop-*` |
| 3 | Command palette (phone) | Rows were 40 px tall on a touch screen | 44 px touch targets | Fixed: 44 px under a coarse pointer. `palette-phone-*` |
| 4 | Changes (phone) | Unified / Side by side was the screen's own radio group, 20 px tall | 44 px; one widget kit | Fixed: the kit's SegmentedControl, 44 px on touch. `changes-phone-*` |
| 5 | Changes (phone) | "Show 1 unchanged line" was 26 px tall | 44 px | Fixed for a diff drawn whole; a long diff drawn in a window keeps its row height (finding 20). `changes-phone-*` |
| 6 | Plugin grant (phone) | Each checkbox row was 40 px | 44 px | Fixed: the row is the target. `plugin-grant-phone-*` |
| 7 | Start an agent (phone) | "Create a shared project identity" was 19 px | 44 px | Fixed. `start-agent-phone-*` |
| 8 | Theme editor (phone) | Colour swatches were 36 px wide | 44 px | Fixed: 44 px wide on touch. `theme-editor-phone-*` |
| 9 | Changes, Files | Line numbers were the secondary ink at 70 %: 3.2:1 on light, 4.3:1 on dark, 2.9:1 on a deleted line | Contrast 4.5:1 | Fixed: the secondary ink at full strength (4.5:1 or better). `changes-*`, `files-*` |
| 10 | Home (phone) | A closed agent's row was faded to 70 %: its folder name read 3.3:1 on light, 4.2:1 on dark | Contrast | Fixed: the row is not faded; the name is in the secondary ink. `home-phone-*` |
| 11 | Agent list (desktop) | The filter's hint was cut off: "Filter by agent, machine or projec" | No text cut off; short sidebar text | Fixed: the hint is "Filter"; its accessible name keeps the whole sentence. `home-desktop-*` |
| 12 | Theme editor, agent list, Settings | Section titles in small capitals: THEME, PREVIEW, COLOURS, AGENTS, THIS DEVICE, MY MACHINES | Section titles are sentence case | Fixed in the follow-up branch (`claude/visual-followups`): the global `h2`, `.ide-section`, the Agents title and Settings' side index are sentence case at the strong step (14, semibold, no letter spacing). `theme-editor-*`, `home-desktop-*`, `settings-desktop-*` |
| 13 | Every page | A language menu ("Browser default") in the title bar of every page, with no label, and again in Settings | Complexity budget: every setting justifies itself | Fixed in the follow-up branch: no language menu in any title bar; Settings (Appearance), the palette ("Language: …") and the sign-in page keep it. Every `*-desktop-*` with a title bar |
| 14 | Pairing | Two primary buttons on My machines while a machine asks: Add a machine and Numbers match…; the open box's **Close** does not say what it closes | One primary action per screen | Follow-up: Add a machine becomes secondary while a request is shown; Close reads "Stop allowing new machines" |
| 15 | Board, Team chat, Usage (phone) | The phone has no way to the board, Team chat or Usage: no panes, and the dock has Search, Start, Machines and Settings | Deep layer reached on demand | Fixed in the follow-up branch: **More** in the phone's bottom bar (rows of 44 px) opens the board (`#/board`), Team chat (`#/messages`) and Usage (`#/usage`), each a screen with Back as the theme editor. `phone-more-phone-*`, `board-phone-*`, `team-chat-phone-*`, `usage-phone-*` |
| 16 | Chat agent (phone) | With a request open, the request card, the Needs you strip, the composer and the dock leave about two lines for the conversation | Focus layer: the agent's work is visible | Fixed in the follow-up branch: while the bar holds the request, the card above folds to one line (icon, the ask's title, a chevron) that answers nothing; a tap on it or the bar's Review and decide opens it in place, whose answers wait a fresh dwell. `chat-phone-*`, `approval-phone-*` |
| 17 | Changes (light) | Word-level marks in a changed line: text on the green mark 4.0:1, on the red mark 3.7:1 | Contrast | Fixed in the follow-up branch: the marks are the `--tint-finished` / `--tint-danger` fill with a 2 px underline in the role's colour; the weakest code ink (muted) keeps 5.7:1 on them in light, 5.9:1 in dark. `changes-*` |
| 18 | Agent list (desktop) | A machine row shows the memory Agora uses ("30 MB") beside its agent count | Short sidebar text | Fixed in the follow-up branch: the machine row shows its agent count only; memory stays on My machines. `first-run-desktop-*` |
| 19 | Home (demo only) | The demo window's desktop Home says "0 Machines online" and Usage lists Laptop, Build server and Windows PC twice, because the demo gives the desktop no machines and some demo agents no machine id | Not a product bug; misleading pictures | Follow-up: the demo passes its machines to the desktop view too |
| 20 | Changes (phone, long diffs) | In a diff drawn in a window (very long), "Show unchanged lines" keeps the fixed row height | 44 px | Follow-up: a taller row for the gap on touch |
| 21 | Theme import | The preview's "$ npm test" line is 3.9:1 | Contrast | No change: it is the imported theme's own comment colour, shown as it is so the person can judge it |
| 22 | Terminal, chat | "Copy agent address laptop/agent-1" names the agent's id | UI vocabulary | No change: the agent address is a documented, copyable address (main-window.md) |

## What still looks off

Honest notes from looking at the pictures, beyond the table:

- **Repeated requests.** One request can be on screen three times on a desktop (the agent's pane, a Needs you pane, the side panel) and
  twice on a phone (the list and the dock). Each is by design on its own; together they are heavy. Worth a look in the approvals redesign;
  left for it (not a small change). On a phone agent's screen the card now folds to one line while the dock holds it (finding 16).
- **Tab names.** With three tabs open at 1440 px the pane tabs cut agent names short ("w… · rose") while the strip has room. The room was
  the slot of a fourth tab, cut off and not drawn, and the three drawn sat at their floor because the strip overflowed anyway. Fixed in the
  follow-up branch: tabs shrink only when that makes them all fit; otherwise each keeps its whole name and the strip scrolls. `home-desktop-*`
- **The import dialog** lists VS Code setting names (`editor.background`, `semanticHighlighting`). They are the source app's words, but
  they read as engineering text.
- **Colour.** Calm surfaces hold everywhere; colour comes from state pills and the amber Needs you. The light theme is as careful as the
  dark one. No thick coloured bars were found; the only edge bars are the diff's 2 px line marks.
