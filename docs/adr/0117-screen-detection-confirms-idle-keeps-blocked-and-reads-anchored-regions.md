# 0117. Screen detection confirms idle, keeps blocked, and reads anchored regions and progress reports
Status: Accepted
Date: 2026-11-26
Accepted: 2026-11-27 (maintainer approval in chat).

Touches ADR-0006 (append-only: manifest engine 7, no protocol message), ADR-0018, ADR-0019 and ADR-0044 (signed lists, the release
key), ADR-0022 and `docs/design/performance.md` (detection is a hot path), ADR-0057 (`[[ready]]`/`[[typed]]` gate typing a prompt),
ADR-0095 (a dispatch looks at the live screen), `docs/specs/agent-manifest.md`, `crates/agora-adapters/src/rules.rs` and `pacer.rs`,
`crates/agora-vt` (the `Terminal` trait) and the threat model (T6).

## Context

Terminal (L1) agents get their state from screen rules (`docs/specs/agent-manifest.md`, `crates/agora-adapters/src/rules.rs`). The
engine has regions `title`, `bottom_lines(n)` and `bottom_non_empty_lines(n)`, conditions `all`/`any`/`not`, a priority order with file
order breaking ties (a stable sort in `Detector::new`, spec "Ties in priority are broken by file order"), "no matching rule is no
opinion", the adaptive `Pacer`, `[[ready]]`/`[[typed]]` (ADR-0057), and `agora agent explain`, which decides exactly as detection does.
Title rules for Claude Code and Codex are in review (PR #1204) and need no engine change.

A comparison with a comparable agent workspace, another terminal multiplexer for coding agents (its public design was read, no code was taken), found
detection layers Agora does not have (study of 2026-11-25, section G2):

- **Timing guards.** Another workspace delays working→idle until three checks 100 ms apart agree, re-announces blocked every 800 ms, and allows
  3 s at start. Agora applies every reading at once (`tick_agent`: `set_state(entry, state, Origin::Screen, ..)` on each check). A
  redraw that briefly shows the prompt box between two tool calls can read as idle, and a dialog being redrawn can read as not
  blocked for one check; each such flicker would be an `AgentStateChanged` on the wire and in the hub's event store. (No event log
  of such a flicker is at hand; the timed captures of the build measure it.)
- **Regions relative to the input box.** Another workspace reads the inside of the prompt box, the rows above it, and what is below the last
  horizontal rule. With only bottom-anchored regions, an old approval question that scrolled to just above the input box can still be
  inside `bottom_lines(n)` and read as blocked after it was answered.
- **Rules that freeze the state.** Another workspace has a "skip state update" rule: while a transcript viewer or model picker is open, the state does not
  change. In Agora "no match" keeps the state, but a full-screen view shows old transcript text, and a lower rule (a prompt marker, an
  "esc to interrupt" line in the transcript) can match it.
- **OSC 9;4 progress reports.** Some agents send the terminal progress sequence `ESC ] 9 ; 4 ; <state> ; <percent>`; the study says
  Claude Code sends state 0 when idle. `agora-vt` ignores it (`alacritty_terminal` does not report OSC 9). No shipped fixture holds
  that sequence today (checked: the captured Claude Code 2.1.319 screens have OSC 0 titles only), so the claim is not yet evidence.
- **Rule updates without a release.** Another workspace fetches detection manifests from its website, unsigned. Agora reads only built-in
  manifests and the node's manifests folder.

Constraints that shape the answer:

- Detection never reads the user-visible viewport or scrollback (AGENTS.md); every region reads the live screen grid or terminal
  state such as the title.
- Agents are data: a new layout must be a manifest, not engine code for one agent.
- The manifest is append-only: `ENGINE_VERSION` is 6; an unknown key inside `[[rules]]` and an unknown region refuse the manifest
  (the built-in kind it meant to replace stays), an unknown top-level table is ignored.
- A wrong `idle` is the dangerous direction: an agent waiting for the person looks free. A second `Blocked` on the wire is a second
  notification once the hub's 60 s dedupe window has passed (`agora-hub::notify::DEDUPE_MS`; PR #1198 keeps a node fact that leaves
  the state as it was from sending one).
- Detection runs under the terminal lock (`TerminalHandle::inspect`) for every running terminal agent, visible or not. There is no
  detection benchmark in `bench/` today (it measures PTY throughput into the VT backend, history, transcripts and footprint).

## Decision

1. **Timing guards, a `[detection]` table (engine 7).** Optional, top-level, so an older engine ignores it and detects as today:
   ```toml
   [detection]
   idle_confirm_ms = 300    # working -> idle only after every check for this long reads idle; 0..=2000, default 0
   blocked_hold_ms = 800    # blocked -> working/idle only after no blocked rule matched for this long; 0..=5000, default 0
   ```
   - **Idle is confirmed, blocked is not delayed.** A reading of `idle` (or `done`) while the state is `working` starts a pending
     change; it is applied when every check during `idle_confirm_ms` read the same state, and dropped by any other reading. A
     reading of `blocked` or `working` is applied at once, as today. Only the transition away from `working` toward rest is delayed.
   - **Blocked is kept, not re-announced.** While `blocked`, a reading of another state is applied only after no blocked rule matched
     on any check for `blocked_hold_ms`. Input the person sends to the terminal (keys, a paste, an answer) ends the hold at once, so an
     answered prompt shows `working` without the delay. Nothing is sent again while the state stays `blocked`: the node never
     re-emits a state it already announced (a periodic re-announce is rejected, see Alternatives).
   - **The pacer during a pending change.** While a change is pending, the next check is due at the visible floor (100 ms) whether the
     terminal is visible or not, and the pacer's back-off does not apply; the pacer records "changed" only when a change is applied.
     So a visible terminal shows idle within about `idle_confirm_ms` plus one check of the first idle reading, and a hidden one costs
     a few extra checks per transition.
   - **Where it lives.** A pure gate in `agora-adapters` next to `Pacer` (`fn offer(&mut self, reading, now) -> Option<AgentState>`,
     clock passed in, no I/O), owned by the node's per-agent entry. Node facts (exit, stop, crash, restart) and reported states
     bypass it and clear anything pending. `agora agent explain` reads one screen and is not affected; `terminal_readiness`
     (ADR-0095) and the ADR-0057 typing check keep their own fresh look at the screen and do not wait for the gate.
   - **Defaults.** An absent table or `0` keeps today's behaviour. Start with 300 ms for idle confirmation and 800 ms for blocked hold
     in the opted-in `claude-code` and `codex` manifests. Validate those starting values with timed captures before enabling them.

2. **Hold rules (`state = "hold"`, engine 7).** A `[[rules]]` entry may say `state = "hold"`: when it is the highest-priority
   matching rule, detection stops there and the state stays as it is (other workspaces call this a "skip state update" rule). It differs from "no
   match" because it stops lower rules that would match the old text a full-screen view shows. A manifest using it says
   `min_engine = 7` (an older engine would refuse the unknown state value anyway; `min_engine` makes the refusal name the reason).
   - `Detector::detect` returns one of three outcomes: a state, a hold (with the rule id), or no opinion. At the three call sites in
     `agora-node::supervisor`: the paced check keeps the state and drops any pending change of decision 1; `terminal_readiness`
     (ADR-0095) and the typing snapshot treat a hold as `unknown` and not ready, so no prompt is typed into a transcript viewer or a
     picker. `agora agent explain` prints "held by rule X".
   - **A hold never outranks blocked.** Validation refuses a manifest where a hold rule's priority is equal to or above the priority
     of any `blocked` rule of the same manifest, so a permission question drawn over a viewer still reads as blocked. The shipped
     manifests leave room for this: every `blocked` rule is at 900 or 950, `working` at 800 and `idle` at 100, so a hold between 800
     and 900 ranks under every blocked rule and over the rules that would read old text.
   - Hold is not allowed in `[[ready]]` or `[[typed]]` (they have `state = "idle"` only).

3. **Anchored regions (engine 7).** A manifest defines anchors, rows found by a regex on the live screen, and rules read regions
   relative to them. This is the generic form of the named regions other workspaces use (inside the prompt box, above it, after the last horizontal rule):
   ```toml
   [[anchor]]
   id = "box_top"                 # a-z, 0-9, _ ; at most 32; at most 8 anchors
   regex = '^╭─'                  # matched against each row of the live screen, bottom row first
   search = 40                    # how many rows from the bottom to look at; 1..=200, default 40

   [[anchor]]
   id = "box_bottom"
   regex = '^╰─'

   [[rules]]
   id = "permission_in_box"
   state = "blocked"
   priority = 900
   region = "between(box_top, box_bottom)"   # also: below(box_top), above(box_top, 6)
   all = [{ contains = "Do you want" }]
   ```
   - `below(a)` is every row under the lowest row matching `a`; `above(a, n)` is the `n` non-empty rows right above it (`n` 1..=200);
     `between(a, b)` is the rows strictly between the lowest row matching `b` and the lowest row matching `a` above it. Anchor rows
     themselves are not in the region. Those named regions are recipes: the prompt box is `between` its top and bottom border, an
     approval left above the box is `above(box_top, n)`, "after the last horizontal rule" is `below` an anchor `^[─━]{8,}$`, and
     Codex's input is `below` an anchor on its `› ` marker.
   - **Live screen only.** Anchors are looked for in the bottom `search` rows of the screen grid, never scrollback and never a viewport
     a person scrolled to. An anchor not found gives an empty region; every rule needs a positive condition, so an empty region never
     matches and the outcome is "no opinion".
   - An unknown anchor id in a region, a duplicate id, or a regex that does not compile refuses the manifest. Anchor regexes follow
     the rule regex limits (linear-time engine, size limit). The `[[anchor]]` table alone would be ignored by an older engine, but the
     regions that use it would refuse the manifest there, so a manifest with anchors says `min_engine = 7`.
   - Anchored regions may be used in `[[rules]]` only, not in `[[ready]]`/`[[typed]]` or `input_box`, until a later decision: those
     gate typing a prompt (ADR-0057) and their shape was reviewed for the bottom-anchored regions.

4. **A `progress` region (engine 7), only after evidence.** The terminal keeps the last OSC 9;4 report the program sent: a state digit
   (0 removed, 1 normal, 2 error, 3 indeterminate, 4 paused) and a percent (0..=100). Region `progress` is the text `<state>;<percent>`
   (`"0;0"`, `"3;0"`, `"1;45"`), empty when nothing was reported. Rules match it with `contains`/`regex` like any region.
   - `agora-vt`: the `Terminal` trait gains `fn progress(&self) -> Option<Progress>` with a default of `None` (other backends stay
     valid). The alacritty backend reads it in its existing OSC scanner (`Scan` in `alacritty.rs`), which already bounds OSC payloads;
     only a payload starting `9;4;` and at most 16 bytes long is parsed, anything else is left to the parser as today.
   - The value is reset when the process restarts or is resumed, and when the program sends state 0. It is terminal state like the
     title, not screen text, so the viewport rule is not touched.
   - **Gate:** the first build step captures the real Claude Code and Codex in Agora's own terminal (its `TERM` and environment) and
     looks for the sequence. If neither sends it there, this decision is not built and the ADR records why.

5. **Tie-breaking stays as it is and covers the new rules.** All rules of a manifest, hold rules and rules on anchored or progress
   regions included, are one list ordered by descending priority with file order breaking ties; the first match decides. Added:
   the shipped-manifest test fails when, on any shipped fixture, two matching rules share the highest matching priority and say
   different states (the file order would decide silently). `agora agent explain` already prints the order.

6. **Signed rule updates, off by default (engine 7, last).** A node may take newer detection rules for a built-in kind from a signed
   list, without a release:
   - **Envelope and key.** The same envelope and check as the agent-package list (`registry::open_signed`: Ed25519 over the payload
     bytes, `now < expires_ms`, `version >= last seen`). No key is built into the binary today: the agent-package list is opened under
     a root key the owner places (`<AGORA_HOME>/agent-list-root-key` or `--root-key`, 64 hex characters, `crates/agora/src/agentcmd.rs`),
     and `agora update` checks an SSH signature under the release key the owner pinned (`<AGORA_HOME>/release-key`; the published key
     is `docs/security/release-key.pub`, an `ssh-ed25519` key). The rule list uses the same model: a root key file on the node
     (`<AGORA_HOME>/rules-root-key`), and the first-party list is signed with the release key's Ed25519 key pair (the 32-byte public
     key inside `release-key.pub`), so no new key pair is made. The owner pins the public key on the node.
     Payload: `{version, expires_ms, kinds: [{kind, manifest_version, min_engine, rules_toml}]}`, at most 64 kinds and 256 KiB;
     unknown fields refuse the list.
   - **Only rule sections.** `rules_toml` may hold `[detection]`, `[[anchor]]` and `[[rules]]`, nothing else; any other key refuses
     the entry. `[[ready]]`, `[[typed]]`, `input_box`, `[process]`, launch options and settings, `[mcp]`, `[hooks]`, `[sandbox]`,
     `[accounts]`, `[resume]`, `[session]` and `[usage]` are never changed by an update: they start programs, write files or gate
     typing a prompt.
   - **Applies only where it is newer.** An entry replaces the built-in kind's rule sections when its `manifest_version` is newer than
     the built-in manifest's `version` and its `min_engine` is supported; the result is validated like any manifest, and an entry
     that fails is skipped and named by `agora doctor`. A file in the node's manifests folder still replaces the whole kind. Order:
     registry, built-in, signed update, manifests folder.
   - **Who fetches.** Only the node, only when its policy says `manifests.updates = on` (default off), from the URL in the node's own
     configuration (default: the release site). The hub never sends rules and cannot turn this on (it cannot widen node policy). An
     expired or unverifiable list leaves the built-in rules in force, never a stale update. The node keeps working offline with the
     last list that verified until it expires.
   - This needs a release-side step that replays every shipped fixture against the updated rules before signing. Until that exists,
     nothing is signed and nothing is fetched.

7. **Docs.** `docs/specs/agent-manifest.md` gets `[detection]`, `hold`, `[[anchor]]` and its regions, `progress` (if built), the
   tie-break test and the update list; `min_engine` notes say engine 7; `docs/design/performance.md` gets the detection benchmark; the
   threat model's T6 row covers the update list and hold rules.

## Performance

- **Baseline first.** A detection benchmark in `bench/`: every shipped fixture replayed on 50 terminals, measuring the time per
  detection tick and how long the terminal lock is held. Each later step reports against it (AGENTS.md: hot-path changes need a
  benchmark result in the commit).
- **Timing gate:** a few comparisons per check and one `Instant` per pending change; extra checks only while a change is pending.
- **Hold rules:** none; they are rules.
- **Anchored regions:** one pass over at most `search` rows per anchor per tick, cached per tick like region text today. The build
  takes one snapshot of the bottom rows (the largest `search` or `n` any rule needs) per tick and derives every region from it,
  instead of each region reading the grid again; matching may then run after the terminal lock is released, which shortens the lock
  hold for every manifest. Bounds: 8 anchors, 200 rows.
- **Progress:** a byte comparison in the OSC scanner that already runs on every PTY read; no allocation unless a `9;4;` payload
  arrives. This is the PTY read path, so the step carries a PTY throughput result from the existing bench.
- **Signed updates:** off the hot path (a fetch and a parse when the list changes).

## Security

- A manifest is third-party input (T6). Hold rules could freeze a state; anchors and regions could steer which text is read. Bounded
  by: validation (hold below every blocked rule; anchors bounded in number, rows and regex size), the "no opinion" rule for a missing
  anchor, and the typing gates (`[[ready]]`/`[[typed]]`, `input_box`) staying on bottom-anchored regions and out of updates.
- Any program in the terminal can print text that matches a rule, set the title, or send OSC 9;4; this is the existing screen-trust
  level (a state read from the screen is shown as such, `reported: false`) and decides nothing about approvals.
- Signed updates: a compromised hub or mirror cannot inject rules (Ed25519 under the root key pinned on the node, rollback and expiry
  checks, node-side opt-in). A stolen release key could ship rules that make a blocked agent read idle on nodes that opted in; it can already
  sign binaries, so the key's custody (ADR-0019) is the control. Updates never touch what starts a program, writes a file or types.

## Alternatives considered

- **Re-announce blocked periodically.** Every re-announce is an event in the hub's store and on the wire, and the notifier
  would notify again after its 60 s window. A hold inside the node gives the same stability with one event.
- **One engine-wide debounce for every manifest.** Changes the behaviour of every manifest, community ones included, without fixtures
  for their timing. Per-manifest opt-in lets the measured fixtures determine where the guards apply.
- **Named prompt-box regions as engine built-ins.** Each new box style would need engine code. Anchors express the same
  regions as data.
- **Regions that read the whole recent screen or its top non-empty lines.** They read rows that can be above the live bottom of the buffer; not
  taken.
- **Unsigned remote manifests or rules pushed by the hub.** Either lets a party other than the release key change what a node
  believes about its agents; the hub cannot widen node policy (ADR-0020).
- **Leave flicker to the window** (smooth it in the client). The events would still be stored and sent, and the hub's notifier and
  every client would each need the same logic.

## Build order

1. Detection benchmark in `bench/` (no engine change) and the tie-break test of decision 5 with its spec sentence.
2. Timing gate (decision 1): pure gate with tests on an injected clock, supervisor wiring (pacer, input ends the blocked hold, node
   facts clear it), then measured values in the shipped manifests.
3. Hold rules (decision 2) with the three call sites and `explain`.
4. Anchored regions (decision 3) and the per-tick row snapshot, with the benchmark result.
5. Progress region (decision 4): the evidence capture first; build only if the sequence is seen.
6. Signed rule updates (decision 6), after a release step that signs a list exists.

Ship each step in its own commit. The first of steps 2 to 5 raises `ENGINE_VERSION` to 7.

## Verification

- **Fixtures (only added, never edited):**
  - timing: timed captures of the real agents (bytes with their delays, for example from `script --log-timing`) replayed through the
    gate on an injected clock, with the expected state timeline: a tool-call gap that briefly shows the prompt box stays `working`; a
    redrawn permission dialog stays `blocked`; answering it shows `working` at once;
  - hold: `transcript.screen` and `model_picker.screen` for `claude-code` (and Codex's equivalents), expected "hold", including one
    with old "esc to interrupt" text that a working rule would match without the hold;
  - anchored: an idle screen with an answered approval question just above the input box, expected `idle` (today `blocked` with a
    bottom-only region), and the existing permission screens still `blocked`;
  - progress: raw captures that contain the OSC 9;4 bytes, if the gate of decision 4 passes.
  The `.expect` format gains the value `hold`.
- **Unit tests:** the gate (each transition, the input release, node facts, pacer floor while pending); validation (hold at or above
  a blocked rule, unknown anchor, duplicate anchor, 9 anchors, `search` 201, hold in `[[ready]]`); `explain` equals `detect` on every
  shipped fixture, holds included.
- **Integration (agora-fake-agent):** a scripted agent that flickers between working and the idle box every 150 ms produces one
  `AgentStateChanged`, not one per flicker; a hidden terminal with a pending change is checked at the visible floor until it settles.
- **Signed updates:** fixtures signed by a public test seed (as `tests/fixtures/agent-list/`): a valid list replaces rules; an entry
  with `[process]` is refused; an older `version` is refused; an expired list leaves built-in rules; policy off fetches nothing.
- **Benchmark:** each step records the detection-tick time and lock hold time against the baseline. Measure PTY throughput in the
  progress step. Keep the 300 ms plus network budget for working and blocked and report the intentional idle confirmation delay separately.

## Open questions

The maintainer resolved these questions on 2026-11-27.

- Keep timing guards opt-in through the manifest, with the initial values and evidence in decision 1.
- Keep the existing no-opinion behaviour at startup. Revisit a startup grace period when a timed capture shows it is needed.
- Measure the idle confirmation delay separately from the working and blocked budget.
- Build and enable signed rule updates only after a release pipeline can sign the list. Start with node-local opt-in and an owner-pinned key.
- Reserve anchored regions in `[[ready]]` and `[[typed]]` for a separate review of the typing gate.

## Status of the build

1. Step 1: done (2026-11-28). The detection benchmark (`agora-bench detection`, baseline in `docs/design/performance.md` "Screen
   detection") and the tie-break test of decision 5 in `crates/agora-adapters/tests/shipped_manifests.rs`, with its sentence in
   `docs/specs/agent-manifest.md`. No shipped screen relies on file order to choose a state.
2. Step 2: the gate is built (2026-11-28), not yet enabled in a shipped manifest. `agora_adapters::Gate` (`crates/agora-adapters/src/gate.rs`,
   tests on an injected clock) and the `[detection]` table (`DetectionSpec`, bounds 2000 and 5000 ms); the supervisor offers each
   paced reading to the agent's gate, checks at the visible floor while a change waits, ends a blocked hold when the terminal's input
   count moved (`TerminalHandle::inputs`, every write through `InputHandle` counted), and clears the gate on every state that is not
   the screen's. Review of e1b6b760: the hold is measured from the first reading without the dialog (never from the last check that saw it, so a check
   after a back-off cannot take one redraw for an answer); a check with no opinion drops an idle being confirmed; the person's input is kept
   until a reading without the dialog, but only within 300 ms (agora-ac's read of f4c336b1: an arrow key or a filter typed in a permission
   dialog is followed by the dialog drawn again, and must not end the hold; the node's own typing never reaches a blocked agent), measured from the keystroke itself
   (`TerminalHandle::last_input`); and a move to rest is confirmed from blocked as from working (a frame caught mid-redraw never lasts
   the confirm time), while blocked -> working stays at once; the gate is not asked while the state is the agent's own report; a new process starts its input
   count and the gate afresh; every write of input goes through `InputHandle` (no way to write past the count). `crates/agora-node/tests/detection_gate.rs`:
   glimpses of the input box between working lines are said idle once under `idle_confirm_ms = 600`, each time without. Detection benchmark after it: tick p50 905 µs, one terminal p50 20.1 µs (baseline
   886 µs and 19.6 µs; the gate is in the supervisor, not in `detect`). Left: timed captures of the real Claude Code and Codex, then
   300 and 800 ms in their manifests (the maintainer's starting values, to be confirmed by the captures). **Engine note:** the engine
   was already at 7 (ADR-0122's `shared_flag`); `[detection]` needs no `min_engine` (an older engine ignores it), and hold rules
   (step 3) will raise `ENGINE_VERSION` to 8 and need `min_engine = 8`.
