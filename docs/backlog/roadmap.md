# Roadmap (from 2026-11-21)

The order the work is done in. One milestone at a time, top to bottom; inside a milestone, the tasks in their order unless a task waits
on a gate. Each task is one or more PRs; each PR is reviewed independently (and by Codex) before it merges. `docs/capabilities.md` stays
the record of what is built; this file is the plan, and a task is ticked here in the PR that finishes it.

Gates, written in the task:
- **schema**: adds protocol messages or fields; the maintainer approves the schema step before it is built (AGENTS.md).
- **decision**: the maintainer chooses (an ADR, a dependency, a default).
- **human**: needs a person, a real account or a real device; agents never use real accounts or tokens.
- **mac** / **win**: needs the macOS or Windows machine and its session (asked through comms).

Owner: the merger session (Claude, Linux) builds unless the task says otherwise.

Where things stand: 62 capabilities in `docs/capabilities.md`, about 34 built, 18 partly, 10 not built; 42 cite evidence from a real
machine written down. So the order is: finish what is in flight, prove what exists on real machines, finish what is half built, then the
new features.

---

## M0. In flight: the kanban board gives a task to an agent (ADR-0095)

- [x] **M0.1 Schema step** (#882, merged): `TaskDispatch`, `TaskDispatchResult`, `TaskDispatched`, `task.dispatch`, `task.dispatch_limited`.
- [x] **M0.2 Node handler** (#883, merged)
  - [x] claim on the node's broker or the hub board as the agent; data-wrapped prompt; `TaskDispatched` with the prompt's SHA-256
  - [x] never typed over a turn: the structured agents' queue; one held prompt per terminal agent, sent as a bracketed paste
  - [x] review fixes: stale agent list in `tick`, re-decide after a slow claim, Unknown holds when the kind can say idle, held prompts
    expire after 15 minutes, reservation tokens, readiness from the live screen, one-task board lookup, explicit stop settles queued tasks,
    held-prompt checks paced like detection
  - [x] four independent review rounds and Codex's review clean, CI green on all three OSes; a real Windows 11 run (WHITE, ConPTY):
    the paste arrives whole and nothing is typed over a working agent
- [x] **M0.3 Window: "Give to an agent"** (#885)
  - [x] a card action on the board (Backlog cards only), also in the command palette (a task and an agent are searchable)
  - [x] the picker lists the project's running agents, helpers under their parent (decision 8); other projects are never offered
  - [x] the confirmation names agent, project and machine in full, and says "acts without asking" for an agent that skips permissions
  - [x] the answer: "sent", "waits for the agent's turn to end", or the refusal sentence from `locales/` (all `task.*`, `agent.queue_full`, `task.dispatch_limited`)
  - [x] `taskDispatch` in `apps/web/src/net/grant.ts` `AGENT_KINDS` (a limited device asks for an agent grant, not a machine grant)
  - [x] a window without the capability hides the action (`task.dispatch` not negotiated): the window cannot read capabilities, so it
    hides the action for a machine once it was refused there as `protocol.capability_missing` (`task.dispatch`) or, from a node too old to
    decode it, `protocol.invalid_message` (`command`); forgotten when that machine goes offline and on every reconnect
  - [x] tests (unit, and a browser e2e with `agora-fake-agent`, run against a build with #883), screenshots in both themes, 44 px targets, keyboard path
- [x] **M0.4 Start a new agent for a task** (ADR-0095 step 3; window: #889, merged; terminal client: #893)
  - [x] "Start a new one" in the picker opens the ordinary start (kind, machine, folder), then dispatches to the new agent
  - [x] under a parent: the ordinary ADR-0057 helper start (signed approval as an agent-made request): the window has no person-made helper start, so it stays the agent's ADR-0057 path and is not offered in the picker
  - [x] the TUI: dispatch from its board list (same command): `b` opens the board, Enter or `g` gives a Backlog task
- [x] **M0.5 Release 0.3.33** (#888, tag `v0.3.33`; 0.3.34 followed, #1002, tag `v0.3.34`; all three machines updated to it: [2026-11-22 follow-ups](../reviews/2026-11-22-real-machine-followups.md))
  - [x] release notes; tag; the release workflow (five targets, `.sha256` and `.sig`)
  - [x] the hub machine (blackview) updated, signature checked; its five agents came back after the restart
  - [x] win updated and reported (0.3.33, `agora doctor` clean)
  - [x] mac updated and reported ([macOS 2026-11-21](../reviews/2026-11-21-macos-update-0333.md))
- [x] **M0.6 ADR-0097** (#874, **decision**): the key that can sign approvals is out of the hub-served page's reach (closes T55)
  - [x] closed, not now (#874 closed 2026-11-22); T55 stays "not mitigated"

## M1. Prove it on real machines

Most capabilities have tests but no recorded run on a real machine. This milestone writes that evidence down, finds what breaks, and
fixes it.

- [ ] **M1.1 Real-machine checklist**
  - [x] one checklist line per capability in `docs/capabilities.md` (what to do, what to see), in `docs/reviews/real-machine-checklist.md`
    - RM-01 to RM-65 cover every row of `docs/capabilities.md` (RM-61 to RM-65 added for ADR-0107, ADR-0101, ADR-0103, ADR-0100 and ADR-0102)
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
- [ ] **M1.4 Visual pass** (ADR-0024)
  - [ ] headless Chromium screenshots of every main screen in both themes, at phone and desktop width; a page that lists them
  - [ ] fix what does not follow `docs/ux/visual-language.md` (tokens only, UI vocabulary, no internal terms)
- [ ] **M1.5 Review debt**
  - [ ] Codex reviews still owed on merged PRs: list them, review, follow-up PRs for findings
  - [ ] the evidence audit task (#708): the scopes not yet covered

## M2. Finish what is half built (all ADRs Accepted)

- [ ] **M2.1 Install coding agents from the window** (ADR-0087)
  - [ ] **decision**: an archive reader for `.tar.gz` / `.zip` (a dependency) or binary-only for now
  - [ ] the window's "Add an agent": the signed list, what is installed where, install with the machine's typed approval
  - [ ] update and roll back from the window (the CLI already keeps the previous version)
- [ ] **M2.2 Pickers everywhere** (ADR-0066): the terminal client's start picker for model, effort and mode
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
- [ ] **M2.11 Layouts** (ADR-0068): panel registry, keybindings, the terminal client
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
- [ ] **M3.4 A plain terminal shows its agent** (ADR-0094, **schema** for step 2): Unix lookup, manifest `[match]`, **win** lookup, the label
- [ ] **M3.5 The diff names every changed file** (ADR-0071, **schema**): `GitDiffResult.paths`, the review screen says what is not shown
- [ ] **M3.6 A reply keeps its link across machines** (ADR-0074, **schema**): `MeshPost.message_id` / `reply_to`, the broker and hub checks
- [ ] **M3.7 Workspace clean-up and disk limits** (ADR-0064): the node cleans what it created, "Clean up workspaces", build output apart, disk watch, the cap
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
