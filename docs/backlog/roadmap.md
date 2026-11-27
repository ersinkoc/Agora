# Roadmap (from 2026-11-21, updated 2026-11-26)

The order the work is done in. One milestone at a time, top to bottom; inside a milestone, the tasks in their order unless a task waits
on a gate. From 2026-11-26 work lands straight on `main` after local tests (fmt, clippy, the affected tests, the web checks, docs
checks); CI runs once on `main` and a break is fixed forward. Risky changes still get an independent review first. `docs/capabilities.md`
stays the record of what is built; this file is the plan, and a task is ticked here in the commit that finishes it.

Small leftovers, suspected bugs and flaky tests (formerly GitHub issues) are in [follow-ups](follow-ups.md).

Gates, written in the task:
- **schema**: adds protocol messages or fields; the maintainer approves the schema step before it is built (AGENTS.md).
- **decision**: the maintainer chooses (an ADR, a dependency, a default).
- **human**: needs a person, a real account or a real device; agents never use real accounts or tokens.
- **mac** / **win**: needs the macOS or Windows machine and its session (asked through comms).

Owner: the merger session (Claude, Linux) builds unless the task says otherwise.

Where things stand (2026-11-26, evening): 0.3.40 is released (the theme catalog, hub-stored plugins with grants, theme packs, themes
that follow a person, the signed plugin catalog, sandboxed plugin panels, theme format 2, the terminal client's themes, layouts and
keybindings, CLI scripting, the fleet view, shutdown-safe node state, lease renewal, the skip-permissions stop). blackview and `win` run
it. What is left of M0 is decisions. The order from here: the community milestone (M5), agent coverage (M6), and the half-built and
Accepted work (M2, M3, M7), with real-machine proof (M1) alongside.
The [architecture gap review](../reviews/2026-11-26-architecture-gap-review.md) is the source of M5 and M6; its gap numbers (G1 to G15)
are cited in the tasks.

---

## M0. Land what is open (2026-11-26)

The earlier M0 (the kanban board gives a task to an agent, ADR-0095 steps 1 to 3, release 0.3.33) is done; see git history of this file.

- [x] **M0.1 The schema stack** (**schema**: the maintainer approves; merge in this order; the hub database moves to version 18)
  - [x] #1225 the theme catalog the hub fetches (ADR-0113 step 5; migration 15)
  - [x] #1226 plugins stored on the hub, with per-person grants (ADR-0114 step 3; migration 16; **decision**: viewers set their own
    plugin grants, which narrows ADR-0049's "viewers have no commands")
  - [x] #1228 theme packs (ADR-0113 step 2; data only, stacked on #1225)
  - [x] #1233 my themes, packs and theme choice follow me across devices (ADR-0113 step 4; migration 17)
  - [x] #1237 the signed community plugin catalog (ADR-0114 step 4; migration 18); **human**: create the catalog repositories and the
    catalog signing key
- [x] **M0.2 Reviewed, merged**
  - [x] #1242 the terminal client: Stop turn, effort and settings in the chat, the start picker, a keybindings file (ADR-0090 step 7,
    ADR-0066, ADR-0068 decision 6; in independent review)
  - [x] #1243 the node renews an agent's file leases while it runs (ADR-0039; review fixes in progress)
  - [x] #1244 a refusing skip-permissions policy stops an agent that starts in, or switches to, a mode that skips (P2-16; needs an
    independent review; **decision**: what `allow` does)
- [ ] **M0.3 Proposed ADRs** (**decision**)
  - [ ] ADR-0117 (detection engine) and ADR-0118 (agents report their own state): G2; on `main` as Proposed (#1251)
  - [x] ADR-0119 (a helper started on another machine): revised for standing grants and chains (the maintainer's "approve once, then free"), Accepted 2026-11-27 with the defaults; built as M7.1
  - [ ] ADR-0114 Amendment B (plugins can act): on `main` as Proposed (M5.4)
- [x] **M0.4 Other open PRs**: #1188 (Codex's input validation audit) merged on `main`; #1204 (title rules) closed earlier, its
  open point is in [follow-ups](follow-ups.md)
- [x] **M0.5 Release 0.3.40** (2026-11-26), blackview and `win` updated; mac asked. Left: **win**, a real `shutdown /r` check of #1231's
  Windows part, on hold until the maintainer's go

## M1. Prove it on real machines

Most capabilities have tests but no recorded run on a real machine. This milestone writes that evidence down, finds what breaks, and
fixes it.

- [ ] **M1.1 Real-machine checklist**
  - [x] one checklist line per capability in `docs/capabilities.md` (what to do, what to see), in `docs/reviews/real-machine-checklist.md`
    - RM-01 to RM-65 cover every row of `docs/capabilities.md`; RM-61 to RM-65 (ADR-0107, ADR-0101, ADR-0103, ADR-0100, ADR-0102)
      need 0.3.34 to 0.3.38, each named in its entry, and have no run yet; RM-62's no-reprompt check needs a person
  - [x] run it on Linux (blackview), then **mac** and **win** through comms; each run is a dated note in `docs/reviews/`
    ([Linux](../reviews/2026-11-21-real-machine-linux.md), [macOS](../reviews/2026-11-21-real-machine-macos.md),
    [Windows](../reviews/2026-11-21-real-machine-windows.md), all 2026-11-21)
  - [ ] the "Real machines" column cites the note; each failure becomes an issue and a fix PR
    - the column cites all three notes; every failure has an issue; #904 and #942 are still open
- [ ] **M1.2 Real agents** (**human**)
  - [ ] fixture screens from dedicated accounts for the kinds still missing (WrongStack, Gemini CLI, OpenCode, ...), P2-09 / P4-05
  - [ ] the kanban dispatch tried once with real Claude Code and Codex (terminal and structured)
- [ ] **M1.3 Measurements** (ADR-0022)
  - [ ] the open Mac holder follow-up (#830) on **mac**
  - [ ] renderer measurement in a real browser (frame times with many panes), written to `docs/design/performance.md`
  - [ ] the same scenario on **win** (125 % and 150 % scale)
- [x] **M1.4 Visual pass** (ADR-0024; `docs/reviews/2026-11-26-visual-pass.md`)
  - [x] headless Chromium screenshots of every main screen in both themes, at phone and desktop width; a page that lists them
    (`scripts/screenshots-visual-pass.mjs`, `docs/screenshots/visual-pass-2026-11/`)
  - [x] fix what does not follow `docs/ux/visual-language.md` (tokens only, UI vocabulary, no internal terms): eleven small fixes; the
    larger ones are follow-ups in the review's findings table
- [ ] **M1.5 Review debt**
  - [ ] Codex reviews still owed on merged PRs: list them, review, follow-up PRs for findings
  - [ ] the evidence audit task (#708): the scopes not yet covered

## M2. Finish what is half built (all ADRs Accepted)

- [ ] **M2.1 Install coding agents from the window** (ADR-0087)
  - [ ] **decision**: an archive reader for `.tar.gz` / `.zip` (a dependency) or binary-only for now
  - [ ] the window's "Add an agent": the signed list, what is installed where, install with the machine's typed approval
  - [ ] update and roll back from the window (the CLI already keeps the previous version)
- [x] **M2.2 Pickers everywhere** (ADR-0066): the terminal client's start picker for model, effort and mode (#1242); remembering the
  last choice per kind and machine (web window and terminal client; never how the agent acts)
- [ ] **M2.3 Confinement** (ADR-0047, ADR-0050, ADR-0083)
  - [ ] per-project and per-kind choice of the level (decision 1)
  - [ ] **win**: measure the Windows sandbox on real agents, then on by default if it holds
  - [ ] Linux pathname sockets (ADR-0083) and the Windows socket rule
- [ ] **M2.4 Team mode** (ADR-0049, P7-09): the remaining steps in `phase-7.md`, then a two-person run on the LAN (**human**)
- [ ] **M2.5 Desktop app** (ADR-0048, P7-06): the remaining steps; signed and notarized builds wait on M4.2
  - [ ] the desktop stays the Tauri shell, held to native-app quality (ADR-0099 closed 2026-11-22): a terminal that feels native, the
    OS's conventions, performance gates, an embedded browser, and checks on real devices (the terminal fixes of #950 were measured in
    three browser engines in a lab, not on real devices)
- [ ] **M2.6 Agents survive the node on Windows** (ADR-0079, ADR-0093): the Windows holder (**win** measurements first)
- [ ] **M2.7 The node updates in place** (ADR-0075): the exec handoff (Unix), then Windows; agents keep running through an update
- [ ] **M2.8 Plan limits for more tools** (ADR-0091): Claude Code's windows, from its own files, read like Codex's
- [ ] **M2.9 ACP registry** (ADR-0092): the next steps after the conversion (the reviewed list in a release, "Add an agent" reads it)
- [ ] **M2.10 Preview a local web server** (ADR-0088): node forwarder, hub relay, the Browser pane (schema is in)
- [x] **M2.11 Layouts** (ADR-0068): panel registry (#1214), keybindings and the palette (#1224), the terminal client's layouts (#1230);
  the terminal client's keybindings file is in #1242; rung 6 is closed (ADR-0099 stages rejected)
- [ ] **M2.13 The window knows what each machine can do** (**schema**): the web window never reads node capabilities (no Hello
  with the hub, none in the snapshot), so it learns that a machine cannot take a task only from a refusal (#885). Carry each node's
  negotiated capabilities in the snapshot (append-only), then hide actions a machine cannot do before anyone tries
- [x] **M2.14 The terminal repaint carries the program's modes and its history** (ADR-0100, **schema** for step 2)
  - [x] step 1, the modes in the repaint (#953); step 2, the history before the repaint when a window asks (#959); step 3, the web
    window and the terminal client ask for it (#964)
- [ ] **M2.12 Code signing** (ADR-0077, **human**): SignPath for Windows, Apple Developer ID for macOS; one signed release recorded

## M3. New features (Accepted, not built)

- [ ] **M3.1 The Helper agent** (ADR-0096; needs M0.3)
  - [ ] the preset format, one bundled preset, start from the palette; tests with `agora-fake-agent`
  - [ ] the Helper in the dispatch picker; the empty state that opens "Add an agent"
  - [ ] board chores in the client (sort, group, filter); presets from settings
- [ ] **M3.2 The board's Review column and subtasks** (ADR-0095 steps 4–5, **schema**)
  - [ ] schema: "accepted by a person", parent link, links to conversation, workspace and branch
  - [ ] the Review column; a person moves a task to Done or back; subtasks with their limits
- [ ] **M3.3 Chat history survives the node** (ADR-0089, **schema**): node writer and paging, resume and purge, pages in the window
- [x] **M3.4 A plain terminal shows its agent** (ADR-0094, **schema** for step 2, G3)
  - [x] Unix lookup and the node's mark: a program a manifest's `[process] names` lists, started by hand in a `shell` agent, is read
    with its kind's screen rules (state, needs you) until it ends
  - [x] the event and the snapshot field (`AgentRunsInside`, `AgentSnapshot.runs_inside`; schema approved 2026-11-26)
  - [x] the "Claude Code in Shell" chip in the windows and the TUI line
  - [x] the **win** lookup (step 3, measured on WHITE first; ADR-0094 Amendment A)
- [ ] **M3.5 The diff names every changed file** (ADR-0071, **schema**): `GitDiffResult.paths`, the review screen says what is not shown
- [ ] **M3.6 A reply keeps its link across machines** (ADR-0074, **schema**): `MeshPost.message_id` / `reply_to`, the broker and hub checks
- [ ] **M3.7 Workspace clean-up and disk limits** (ADR-0064): the node cleans what it created, "Clean up workspaces", build output apart, disk watch, the cap
  - [x] the node records its worktrees and removes them after the grace period when nothing would be lost; `agora workspaces [clean]`;
    build output of left-behind worktrees; the cap and the free-disk minimum refuse a new worktree (ADR-0064 "Build status")
  - [ ] free space and sizes in the node report, "Needs you" when low, the clean-up buttons in the windows (**schema**);
    `[workspace] clean` in manifests
- [ ] **M3.8 Direct client ↔ node terminal path** (ADR-0023): measured against the relay first; built only if the budget needs it
- [ ] **M3.9 Any machine can be the hub** (ADR-0086, staged)
  - [ ] export and import; the signed hub record (**schema**); manual takeover with partition tests; continuous replication with a drill; automatic takeover (3+ machines)
- [ ] **M3.10 Agent login and accounts** (design only): an ADR first (**decision**), then build
- [x] **M3.11 A structured agent can be allowed always** (ADR-0101, Accepted 2026-11-22; the agent decides how long it keeps the answer)
  - [x] part A, steps 1 to 3 (**schema**, bridge and node, hub): #1007, merged
  - [x] part B, steps 4 to 6 (the window, the terminal client, docs): #1012

## M4. Ready for other people

- [ ] **M4.1 Name, logo, domain** (**human**, `docs/open-questions.md`): then the one mechanical rename
- [ ] **M4.2 Signed releases on every OS** (needs M2.12): install guides that need no warnings clicked away
- [ ] **M4.3 Words**: a native read of the Turkish locale (**human**); the English pass over every screen
- [ ] **M4.4 First-run**: from a fresh machine to an agent running in under five minutes, measured with people (**human**)
- [ ] **M4.5 Remote access**: Cloudflare Tunnel tried with a real tunnel (**human**, `docs/guides/remote-hub.md` part C); a phone over mobile data
- [ ] **M4.6 Guides**: install, add a machine, add an agent, the board, team mode, remote access; each tried by someone who did not write it

## M5. Community: themes and plugins (ADR-0113, ADR-0114; gap review sections 3 and 4)

Built: theme format 2 and its editor (#1206), the terminal client's themes (#1222), the panel registry (#1214), the sandboxed plugin
frame with grants and safe mode (#1218, #1236, #1238), and its evidence audit (#1239). The catalogs and sync are M0.1.

- [ ] **M5.1 Theme import** (no schema): iTerm2, Alacritty, Ghostty, Windows Terminal and VS Code colour files mapped to format 2
  - [x] in the web window's editor: all five formats ([themes](../ux/themes.md#import-from-another-app))
  - [ ] as `agora theme import` (not built: the maintainer asked for the web window only)
- [x] **M5.2 Starter themes**: well-known palettes ported after a licence check: Catppuccin Mocha, Dracula, Gruvbox (dark and light),
  Rosé Pine and Tokyo Night, in both clients ([themes](../ux/themes.md#starter-themes)); Catppuccin Latte, Tokyo Night Day, Nord,
  Rosé Pine Dawn and Solarized were left out: with their own colours (and Agora's in the slots they have none for) they cannot pass
  the readability rules without a warning ([why, per variant](../ux/themes.md#starter-themes))
- [ ] **M5.3 A colour per machine or project** (ADR-0113 amendment, **decision**): a frame colour that keeps a person from typing into the
  wrong machine
- [ ] **M5.4 Plugins can act** (ADR-0114 amendments, **decision** on the order; each one its own amendment, then built)
  - [ ] P1 UI actions: open a pane, focus an agent, prepare a prompt the person reads and sends
  - [ ] P2 palette commands and keybindings declared by a plugin (the safe-mode key stays unbindable)
  - [ ] P3 a status-line segment and notifications the host draws
  - [ ] P4 link handlers: a declared pattern in terminal output opens the plugin's panel
  - [ ] P5 window-side events, read-only; node-side automations need their own ADR (**decision**: now or after P5)
- [x] **M5.5 A gallery** for the theme and plugin catalogs (ADR-0120): previews, sort by popular, new, name or author, search, plugin
  screenshots, one-step theme install (a plugin is still installed by the hub's owner)
- [ ] **M5.6 Distribution** (**decision**): a second, "unreviewed but sandboxed" catalog tier (ADR-0114 decision 7 amendment); members
  installing sandboxed plugins for themselves (ADR-0114 open question)
- [x] **M5.7 Plugin author kit**: reload on save, `agora plugin new` / `check` / `pack` (today `scripts/pack-plugin.mjs`), a typed
  description of the bridge messages ([plugins](../ux/plugins.md#writing-a-plugin): `pack` byte for byte with the mjs, `check` with the
  hub's port of the window's checks, the types held to the window's by `tsc -p plugins`; reload on save where the browser can watch a
  file)
- [ ] **M5.8 Chrome per layout block** and more than one icon set (ADR-0113 follow-ups)

## M6. Agent coverage and detection (gap review section 2)

- [ ] **M6.1 More coding tools out of the box** (G1, **human**): L1 manifests with real fixture screens (Gemini CLI, Copilot CLI,
  Cursor, Amp, Kiro, Droid, Qwen, Cline, ...); the 41 ACP registry kinds checked with real programs. The largest gap.
- [ ] **M6.2 Detection engine and self-reported state** (G2): ADR-0117 and ADR-0118 (#1219, M0.3), then built; the manifests' `hooks`
  table runs
- [x] **M6.3 A plain terminal shows its agent** (G3): M3.4
- [ ] **M6.4 A pushed event stream and a published API description** (G9, **decision**): `agora events` polls every 200 ms today
- [x] **M6.5 A manual written for agents** (G10, no protocol change): a command that prints how to drive and customize Agora, so an
  agent can write a theme, a layout or a plugin for its person
  - [x] `agora manual [TOPIC] [--json]`: `scripting`, `themes`, `layouts`, `keys`, `plugins`, `mesh`, `files` (`docs/manual/`); every
    command line, example and limit in it is checked against the product by tests
- [ ] **M6.6 Images in terminals** (G11, **decision**): kitty graphics and sixel
- [x] **M6.7 Notifications outside the web window** (G13): the terminal client's sounds and the desktop app's own notifications.
  Both halves are done ([main-window.md](../ux/main-window.md#sounds-and-a-finished-mark-that-lasts-until-seen)):
  - the terminal client: the bell, a desktop notification through the terminal, the finished mark;
  - the desktop app: App notifications through Tauri's notification plugin, with a click that opens the agent.

  The desktop half rests on ADR-0048 Amendment A (Accepted 2026-11-26: the hub's page may call one command).

## M7. Accepted, not built (from the 2026-11-26 sweep of every Accepted ADR)

Items already in M2 and M3 are not repeated here.

- [ ] **M7.1 A helper on another machine** (ADR-0057, ADR-0119 Accepted 2026-11-27): schema (approved with the ADR), the target node
  with standing grants, the hub, the asking node, the window
- [x] **M7.2 Leases across a node restart** (ADR-0039): #1243, in 0.3.40
- [x] **M7.3 A node with a clock behind** (ADR-0085 Amendment A): keeps its request, accepts a certificate up to 24 h ahead, and
  `agora doctor` says the clock may be wrong. Left: the "needs to join again" card in the window (**schema** for the card)
- [ ] **M7.4 Where am I signed in** (ADR-0067 rung 5): the session list with revoke; the rung-4 banner
- [ ] **M7.5 Pairing leftovers** (ADR-0065): the owner's devices trusted in the same step, the push notice, discovery over IPv6
  (**schema**)
- [x] **M7.6 Prices and the panel** (ADR-0021, ADR-0056): periodic price-catalog fetch (off until `agora hub prices auto on`), owner
  overrides (`agora hub prices set|unset|show`), manifest `model_map`, `agora panel` and `agora web` opening the browser, an exclusive
  bind of the panel port. Left: the desktop app's stored port, plain `agora` opening the panel on a node-only machine, ADR-0033's
  declared-provider lookup
- [ ] **M7.7 ACP authenticate** (ADR-0092 steps 2 and 3, **human** to verify): the states in the window; the registry update job
- [ ] **M7.8 Live update** (ADR-0019, ADR-0075; G4): the hub hands out signed node binaries after approval; stable and preview channels
  (M2.7 is the handoff)
- [ ] **M7.9 Install channels** (G7, **decision**): a signature-checking install script, a Homebrew tap, a preview channel
- [x] **M7.10 Terminal repaint leftovers** (ADR-0100): origin mode and scroll region restored
- [ ] **M7.11 QUIC and the direct path** (ADR-0011, ADR-0023): measured first (M3.8)
- [ ] **M7.12 A documentation site** (G14, Q15, **decision**)

