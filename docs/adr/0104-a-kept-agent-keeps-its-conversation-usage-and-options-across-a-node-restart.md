# 0104. A kept agent keeps its conversation, usage and options across a node restart
Status: Accepted
Date: 2026-11-23
Accepted: 2026-11-23 (the maintainer, in the merger chat: "Accept and build" to the question that summarised the options fix, the appended `AgentStarted.same_process` field, the node transcript writer and the stop handler, in that build order)

Origin: issues #931, #930 and #1080, all follow-ups of #907 / PR #928 and #932 (roadmap M1.1, RM-34, RM-47). Since ADR-0093, the terminal
holder keeps a structured agent's process running across a node restart, and the next node takes it up. The process and its session
survive, but three things the window shows about it do not: its conversation, its usage counters, and (sometimes) its modes and options.

Touches ADR-0089 (chat history on the node's disk; Accepted, not built), ADR-0093 (structured agents survive a node restart; decision 3
says what is taken from the holder's replay), ADR-0079 (the holder), ADR-0090 (a complete conversation; its open question 3), ADR-0058
and ADR-0033 (usage and the cost report), ADR-0054 and ADR-0055 (resync announcements), ADR-0060 (resume), ADR-0101 (kept "always"
answers), ADR-0006 (append-only schema), ADR-0022 and `docs/design/performance.md`, `docs/specs/holder-link.md`,
`docs/specs/protocol.md`, `docs/specs/adapter-protocol.md`. ADR-0103 (Proposed) is a related shape for the hub's Team chat. File and line
references are at main `9760e901`.

## Context

### #931: the conversation is empty after a node restart

- The node keeps each structured agent's transcript in memory only: "kept in memory and gone when the node restarts"
  (`crates/agora-node/src/chat.rs:3-4`), at most 500 entries or 256 KiB (`chat.rs:28-29`). A client that subscribes gets that ring as a
  `ChatSnapshot` and then live entries (`docs/specs/protocol.md:451-456`).
- The next node starts with an empty ring for an agent it takes up, so the window and the terminal client show no earlier turns.
  ADR-0093 "Context" names this ("the in-memory chat history is gone until ADR-0089's persistent history is built"), and so does the
  holder-link spec (`docs/specs/holder-link.md:288-289`).
- The ring holds more than the adapter said: the person's own prompts (role USER) and the entries the node makes itself (permission
  entries from its approvals queue, the node's notices; `chat.rs:11`, `NODE_NOTICES` at `chat.rs:52`). None of these ever pass
  through the adapter's stdout, so nothing the holder keeps can rebuild them.
- Entry ids start again at 1 in the new node (`Transcript::add`, `chat.rs:464-473`). The spec says "a client ignores an id it already
  holds" (`protocol.md:456`), so ids that restart can collide with ids a window still holds.
- ADR-0089 (Accepted 2026-11-19, `docs/capabilities.md:81`: not built) decides an on-disk transcript per agent and paged reads. As
  written, the existing ring snapshot "stays as it is" and older turns are read through a new `chat.history` command (decision 4), which
  needs its schema PR first (its build order step 1). It does not say that a node fills the ring from the file when it takes an agent up.

### #930: the usage counters start again from 0

- A node's start records `NodeOnline`, and the hub ends every running agent of that node (`crates/agora-hub/src/projection.rs:421-433`).
  The node then says each agent it took up from the holder up again with `AgentStarted` (`EventSink::agent_taken_up`,
  `crates/agora-node/src/daemon.rs:535-567`), for terminal and structured agents alike.
- The hub's fold of `AgentStarted` inserts a fresh `AgentView`: `input_tokens: 0`, `output_tokens: 0`, an empty `model` and
  `SessionView::default()`, which also clears the reported cost and the context level (`projection.rs:541-577`). The web's fold does the
  same (`tokens: 0`, `startedMs: at`; `apps/web/src/net/view.ts:296-308`).
- **Wider than the issue says.**
  - A resync announcement (ADR-0054, ADR-0055) re-states every running agent with the same `AgentStarted` (`announce_present`,
    `daemon.rs:512`), so a hub that only missed some events also zeroes the counters of agents that never stopped.
  - The hub's cost report (`usage.report`, ADR-0033) is built from the same per-agent counters (`Hub::usage_report`,
    `crates/agora-hub/src/hub.rs:235-272`). The window's cost total and per-project cost (`useUsageReport`, `Console`) therefore lose a
    kept agent's earlier usage too.
  - The window's Usage panel (`UsagePanel`, `apps/web/src/net/usageView.ts:20-33`) sums the same per-agent `tokens` and measures
    active time from `startedMs`, and both start again at the take-up. At this commit the panel has no per-day rows, so contrary to
    the issue it is affected too.
  - Resume (ADR-0060) resets the counters too. That is a new process, and whether it should keep them is open question 3.
- `AgentUsageReported` is a **growth**, not a total (`schema/agora/v1/events.proto:192-202`; summed at `projection.rs:634-650`; the node
  merges reports for 5 s before logging, `crates/agora-node/src/usage_merge.rs`). The node cannot re-state a total through it: the hub
  would add the total to what it already has.
- The hub still holds the earlier totals after `NodeOnline` (that arm clears `running`, `state` and the "always" answers, not the
  counters). Only the `AgentStarted` fold throws them away.

### #1080: options read but not recorded before a node stops are lost to the next node

- An adapter does not report `session/options`, `session/config` or `session/commands` again by itself, and a kept adapter is not
  initialised again. So the node keeps the last ones it recorded in `<data>/sessions/kept-<id>.pb` (#932; `KeptSession`,
  `crates/agora-node/src/ledger.rs:153-163`; `holder-link.md:278-289`). The next node puts them back on take-up (`take_up_held`,
  `daemon.rs:2848-2857`; `adopt_held_pipe`, `crates/agora-node/src/supervisor.rs:1088-1091`).
- That file is written only after the whole path has run: the holder gives the line (`PipeRead`), the host parses it, the supervision
  tick records the event, the ledger observes it, and the ledger's writer thread writes the file. The holder counts a line as given (`seen`)
  at the first step (`holder-link.md:253-256`), so a report that is past step one and not yet at the last is lost:
  - **On a clean stop** (the `Node` dropped in-process), `Supervisor::shutdown` detaches each held adapter (`supervisor.rs:3971-3994`;
    `Adapter::detach` joins the reader, `crates/agora-adapters/src/host.rs:891-903`). Events still in the host's channel are dropped
    with it.
  - **On a kill**, everything in flight is lost, including a `PipeData` reply that never reached the node.
  - **In production every stop is a kill.** `agora node` has no stop handler: it sleeps after starting (`crates/agora/src/main.rs:850-851`),
    and the supervisor passes `SIGTERM` on to it (`crates/agora/src/supervise.rs:117`; only the supervisor and the holder catch
    `SIGTERM`, `catch_stop_requests`). So `agora down`, a service restart and the restart after an update (`agora update` only replaces
    the executable) end the node without unwinding, as `daemon.rs:137-140` says for the usage merge. `Node::drop` and
    `Supervisor::shutdown` run only when a node is dropped in-process (tests).
- On take-up, `adopt_held_pipe` reads `pipe.seen` only for approvals (`host::pending_approvals`, `host.rs:1291-1313`; ADR-0093
  decision 3). Reproduced in the issue: a probe once showed `options=true config=false` in the kept file just before the node stopped
  (1 run in 15), and the next node never learned `auto_approve`.
- **The failure is on the unsafe side.** `current_mode` and the config options decide `skips_permissions_now`. Suppose an agent switches
  itself to a mode that skips permissions just before its node is killed. The next node then shows it as one that asks. The option
  checks (`prepare_set_option` / `prepare_set_config`) also answer `agent.option_unknown` for an option the agent offers.
- **Ordering is simpler than the issue feared.** The node's copy of options, config and commands is written only from the adapter's own
  reports (`supervisor.rs:2291-2320`), and cleared when the adapter's process ends or is replaced (`supervisor.rs:2344`, `2449`).
  `session/set_option` and `session/set_config` drop the reply (`host.rs:449-479`: `.map(drop)`), and the adapter protocol says an
  adapter re-sends a report whenever something in it changes (`docs/specs/adapter-protocol.md:61-63`). So for the same process, the
  newest report of each kind the adapter wrote is always the truth, and nothing the node does on its own can be newer.
- These three reports **replace** the previous one, so reading one twice changes nothing. Chat items and usage reports **append**. Reading
  them twice would show a turn twice or count tokens twice, so they cannot be taken from `seen` the same way.

### What a fix may use

| Place | What it is | Durable across | Writes |
| --- | --- | --- | --- |
| The ledger (`sessions/ledger.json`) | one JSON file, replaced whole, at most 4 MiB, "never terminal contents, prompts, or secrets" (`ledger.rs:1-13`) | node restart, reboot | on start, session, end |
| `kept-<id>.pb` beside it | the last options, config, commands and "always" answers of an adapter agent (#932, ADR-0101) | node restart, reboot | off the recording thread, per change |
| ADR-0089's `transcripts/<id>.jsonl` | the agent's transcript, append-only, batched (at most once per second or per 64 KiB), redacted, 0600 | node restart, reboot, resume | batched, off the agent's path |
| The holder's replay (`seen` + `fresh`) | the last 1 MiB of the adapter's raw stdout lines, unparsed (ADR-0093) | node restart only; lost if the holder ends | none (memory) |
| A state blob in the holder | does not exist: a new link frame (link version 3) | node restart only | a link round trip per change |
| The adapter re-announcing | does not exist: a new adapter-protocol request and capability | while the process runs | none |
| The hub's view and log | totals per agent, folded from the hub's events | everything; the hub keeps its log | none new |

## Options

### For the conversation (#931)

- **(C1) ADR-0089's transcript file, and the node seeds the ring from its tail.** ADR-0089's writer as decided. In addition, when a node
  takes an agent up from the holder (and on resume, ADR-0089 decision 5), it reads the file's tail into the ring before the adapter's
  replay is processed. Entry ids continue from the file's last id, which ADR-0089 decision 4 already requires.
  - *Durability.* What was written survives a node restart, a holder end, a resume and a reboot. A stop loses at most the last batch (up
    to 1 s or 64 KiB of entries). Every production stop is a kill today, so that holds until O2's stop handler flushes it.
  - *Disk.* Batched and off-thread, so neither the terminal path nor the chat path waits on the disk (ADR-0089 decision 1). Chat entries
    are not terminal bytes. Seeding reads at most the ring's bound (500 entries or 256 KiB) once per agent at take-up.
  - *Secrets.* As ADR-0089 decision 7: on the node only, 0600, known secret values replaced. Nothing goes to the hub or to the holder.
  - *Compatibility.* Node-local. Every client gets the ring snapshot it gets today, only filled. No schema change and no capability.
    Paging (`chat.history`) stays ADR-0089's later schema step.
  - *Turn in progress.* A tool call or plan entry that changes in place needs the ring's side state (`calls`, `plan`; `chat.rs:74-86`).
    A call keeps its tool name and arguments apart from the entry (`Call`, `chat.rs:67-71`). The entry's text joins them as JSON and can
    be cut at 16 KiB, so the text alone does not always give them back. The writer must therefore store them with a tool-call entry's
    line, or seeding must close a call it cannot rebuild.
  - *Permission entries.* A permission entry is keyed by the approval id the old node minted (`push_permission`, `chat.rs:897-909`). A
    seeded entry still `pending` whose approval the new node does not hold must be marked ended. Otherwise it would wait forever beside
    the entry for the request the take-up asks again (ADR-0093 decision 3).
- **(C2) Rebuild the ring from the holder's `seen` lines.** No disk, no schema. But it lacks the person's prompts and the node-made entries,
  goes back at most 1 MiB of raw output (one long tool output can be all of it), the text is not yet redacted, and the same `chat/append`
  read twice shows twice. It also widens ADR-0093 decision 3 to an appending report. Rejected.
- **(C3) The holder keeps the ring as a blob.** A new link frame (version 3). It is useful only when the node hands the blob over on a
  clean stop (which production never has today, see #1080 above), like ADR-0079's VT snapshot, unless every entry costs a round trip
  to the holder. It is lost when the holder ends, gives nothing to resume, and puts the conversation in a second process. A running version 2 holder stays until all its agents end, so the fix
  would reach a machine late. Rejected.
- **(C4) The adapter replays the conversation on request** (ACP's `session/load` replays it when an agent offers it). It needs a new
  adapter-protocol request on a session that is already open, depends on each agent's support, and still lacks the node's own entries.
  Rejected.

### For the usage counters (#930)

- **(U1) The hub keeps the totals when the node says it is the same process (an appended field).** `AgentStarted` gains a bool that the
  node sets when it re-states an agent whose current process it has already announced under this id: when it takes one up from the
  holder, and in a resync's re-statement. The hub's and the web's fold then keep what they had summed (tokens, the reported cost and its
  currency, the model, the context level, and in the web the time it started) instead of starting from 0.
  - *Durability.* The hub's view is already rebuilt from its log, so the rule gives the same totals after a hub restart.
  - *Disk.* None new on the node. One field on a rare event.
  - *Secrets.* None: counters only.
  - *Compatibility.* An appended proto3 field (ADR-0006). An older hub ignores it and resets, as today. An older node never sets it, so a
    newer hub resets, as today. The web is served by its hub and always matches it. The terminal client reads the hub's snapshot, which
    carries the kept totals. No capability.
  - *Trust (T19).* The hub qualifies the agent by the producer, so a node can keep only its own agents' counters. It could inflate them
    anyway with usage reports, so this widens nothing.
- **(U2) The same carry-over inferred by the hub, with no field.** Any `AgentStarted` for an id the hub already lists from the same
  producer keeps the counters. No schema change. But the hub cannot tell a kept process from a resume or a crash restart of the same id,
  a published event's fold changes meaning silently, and logs written before the change fold differently on a rebuild. Rejected for the
  silent change in meaning (open question 2 asks the maintainer to confirm this reading of ADR-0006).
- **(U3) The node keeps totals in its ledger and re-states them.** `AgentUsageReported` is a growth, so a re-stated total would be added
  to what the hub has. It needs a new event anyway, keeps a second copy of what the hub already holds, and writes the ledger on every usage
  report. Rejected.
- **(U4) A state blob in the holder.** As C3, for a number the hub already has. Rejected.
- **(U5) The adapter re-announces its usage.** Adapters report growth (`usage/report`; the ACP bridge sends the cost's growth), and most
  agents expose no running total, so a re-announced total would need a new report kind, and the hub would still have to be told not to
  add it. Rejected.

### For the options (#1080)

- **(O1) On take-up, the newest report of each kind in the holder's replay wins over the kept file.** Before the `fresh` lines are read
  as new, the node scans `seen` for the last `session/options`, `session/config` and `session/commands` notifications for the agent's
  recorded session (`Entry::adapter_session`). Each one it finds replaces the kept file's copy of that kind, bounded and judged again as a
  live report is (`bounded_options`, `bounded_config`, `bounded_commands`, the 64 KiB rule). A kind with no report in `seen` keeps the
  file's copy. The `fresh` lines then apply as they arrive.
  - *Why the order is sound.* Only adapter reports set these copies, and each replaces the last. So the newest one the holder still has is
    at least as new as the file's. If the holder pushed every report of a kind out of its 1 MiB, the file's copy is the newest left.
  - *Durability.* Covers a kill and a broken reply, as long as the holder runs. The same pass as `pending_approvals` reads the same lines
    once per agent at take-up.
  - *Disk.* None new.
  - *Secrets.* None new. These reports are already kept in the file.
  - *Compatibility.* Node-local. Any version 2 holder works unchanged.
  - *Decision change.* It amends ADR-0093 decision 3 ("nothing but approvals is taken from `seen`") for these three reports, which
    replace. It does not amend it for chat, usage or state, which append.
- **(O2) A stop drains before it detaches, and `agora node` gets a stop handler so that a stop is clean.** `Supervisor::shutdown` takes
  each held adapter's remaining events after `detach` has joined the reader. It records them through the usual path, then flushes the
  ledger's kept writes (`Ledger::flush_all`), the usage merge and the transcript writer (C1). On its own this helps only a node dropped
  in-process, which production never does today (Context). With a `SIGTERM` handler that drops the `Node` within a bounded time, it
  would cover `agora down`, a service restart and the restart after an update. There it closes the gap for chat, state and usage too,
  which O1 cannot. Node-local, with no decision change. The handler changes how every node stop ends, though: it must finish before the supervisor's
  and systemd's timeouts, and a hung join must not keep a node from stopping. So it is its own step (open question 6).
- **(O3) Write the kept file on the reader's thread, before the event is queued.** This puts a disk write on the adapter's read path
  and still loses a line between `PipeRead` and the parse. Rejected.
- **(O4) "Given" becomes "recorded": the node acknowledges lines to the holder** (a new frame, link version 3), so unacknowledged lines
  come back as `fresh`. This is the general fix for in-flight loss, but it makes delivery at least once. Chat and usage would then show and
  count twice unless every report gets an id the node can deduplicate. It would also arrive late on machines with a running version 2
  holder. Rejected for now.
- **(O6) A state blob in the holder.** The node would push each report to the holder as an opaque blob (link version 3). The holder's
  replay already holds the same reports as raw lines, which O1 reads. Rejected.
- **(O5) The adapter re-announces on request.** A new adapter-protocol request (for example `session/restate`), gated by a capability the
  adapter declares in `initialize`. The ledger already keeps those capabilities (`Entry::adapter_capabilities`), so the next node knows
  without `initialize`. The adapter is the source of truth, so this removes the ordering question entirely. But it needs every adapter
  (and Agora's ACP bridge) to keep and re-send its state, third-party adapters lag, and O1 already gives the same answer for the reports
  that matter. Not now (open question 5).

## Decision (proposed)

1. **Options (#1080): O1, then O2.**
   - O1 is the fix, because it covers a kill, and every production stop is one today. On take-up, the newest `session/options`,
     `session/config` and `session/commands` in the holder's `seen` lines, for the recorded session, replace the kept file's copy of
     that kind. They are bounded and judged as a live report is, before `fresh` is read.
   - This amends ADR-0093 decision 3 for these three replacing reports only. Approvals stay as decided. Chat, usage and state are not
     taken from `seen`.
   - O2 comes later as a step of its own: a stop handler for `agora node` and a drain of each held adapter's remaining events, then
     flushes of the ledger's kept writes, the usage merge and the transcript writer. It narrows what a planned stop loses for every
     kind of state.
   - No schema change and no holder-link change.
2. **Usage (#930): U1.**
   - `AgentStarted` (`schema/agora/v1/events.proto`) gains one field at the next free number outside ADR-0059's 30-39 (8 today, checked
     at the schema PR):
     ```
     // ADR-0104: this re-states an agent whose current process this node has announced before under this id (taken up from the
     // holder after a node restart, ADR-0079/0093, or re-stated in a resync, ADR-0054/0055). A hub keeps what it had summed for it
     // (tokens, reported cost, model, context level) instead of starting again from 0. Older hubs ignore it.
     bool same_process = 8;
     ```
   - The node sets it in `agent_taken_up` and in `announce_present`'s re-statement, never for a new start, a resume or a restart
     after a crash.
   - The hub's fold (`projection.rs`) and the web's fold (`view.ts`) keep the counters, the cost and its currency, the model and the
     context level when it is set and the agent is already listed from that producer. Everything else is as today, and the node
     re-states options, config, commands and "always" answers as it does now. No new message, command, error code or capability.
   - One golden fixture is added (an `AgentStarted` with the field) and none is edited.
3. **Conversation (#931): C1.**
   - This refines ADR-0089 without editing it. Its node writer (decision 1) is built as decided, storing the last version per
     `entry_id` (ADR-0090 open question 3's proposed answer).
   - Added here: **a node that takes an agent up from the holder, or resumes it, seeds the agent's ring from the tail of its file**
     before the adapter's replay. It reads at most the ring's bounds and continues the entry ids from the file's last id. Seeding
     rebuilds the ring's side state for calls and plans from what the writer stored, and marks a seeded pending permission entry ended
     when the new node does not hold its approval.
   - Seeding and the writer need no schema, so they may be built before ADR-0089's schema step. Paging (`chat.history`, its schema and
     the web's scroll-back) stays ADR-0089's, in its own order.
   - A file that is missing or cannot be read seeds nothing: the ring starts empty, as today, and the agent is taken up all the same.
     Ids then continue from the highest id the file shows, or from 1 when there is none.

### Build order (each its own PR, after acceptance)

1. **O1: the replay scan for options, config and commands** (`adopt_held_pipe`, `agora-adapters::host`, next to `pending_approvals`),
   plus `docs/specs/holder-link.md` ("Rebuilding what waited on the person" and "Limits, plainly"). Tested through a node kill with
   a report the old node read and did not record: the #1080 reproduction made deterministic.
2. **U1 schema:** `AgentStarted.same_process`, the fixture, and `docs/specs/protocol.md`.
3. **U1 node, hub and web:** the node sets the field, and the hub and web folds keep the totals. Tested with a hub view rebuilt from
   a log with a kept take-up, a resync re-statement, and a new start of a reused id that still resets.
4. **C1:** ADR-0089's node writer, retention and purge, plus the seeding on take-up and resume, with continuous ids. Tested with a kill
   then take-up (earlier turns shown, ids continue), a full disk (agent unaffected, notice once), and a tool call that finishes after the
   restart (one entry, updated in place). Steps 3 and 4 are independent.
5. **O2: the stop handler and the drain**, if open question 6 says yes (`agora node`'s `SIGTERM` path, `Supervisor::shutdown`,
   `Adapter::detach`, the ledger, usage-merge and transcript flushes). Tested through `agora down` with a report in the host's channel,
   and with a hung join that still lets the node stop in time.

## Consequences

- **Fixed.**
  - A kept agent's window shows its earlier turns, its token counts, cost and model, and its current modes and options after a node
    restart.
  - A hub that only missed events no longer zeroes counters on a resync.
  - A mode that skips permissions, switched to just before a kill, is shown and checked as skipping (O1), as long as the holder still
    has the report.
- **Schema.** One appended field and one fixture (step 3). `schema-breaking` stays green.
- **Holder.** Unchanged. No link version 3.
- **Node.**
  - The drain in `shutdown`, the scan in `adopt_held_pipe`, and the `same_process` flag in `agent_up`.
  - ADR-0089's writer and the seeding. One file per structured agent, at most 32 MiB by ADR-0089's default.
- **Hub and web.** One branch in each `AgentStarted` fold. The cost report (`usage.report`) keeps a kept agent's earlier usage with no
  change of its own.
- **Performance.** Nothing on the PTY or VT path. The transcript writer is batched off-thread (ADR-0089). The take-up does a bounded read
  of each file's tail and one parse pass over at most 1 MiB of `seen`, the pass `pending_approvals` already makes. No new event on any
  hot path.
- **Still lost (said plainly in `holder-link.md`).**
  - On any stop until O2's handler exists: the transcript's last unwritten batch, usage the node was still merging (up to 5 s,
    `daemon.rs:137-140`, PR #522), chat and state lines read and not yet recorded, and an "always" answer decided and not yet written to
    the kept file. The agent's process still holds that answer (ADR-0101), so this is a gap in what the window shows, not a change in
    behaviour. With the handler, only a real kill loses these.
  - Options whose every report the holder pushed out, and that the old node read and did not record: in practice none, since the old
    node would have had to read 1 MiB of newer output in the moment before it died.
- **Docs, moved with the build PRs.**
  - `docs/specs/holder-link.md`: the take-up and the limits, and the lines at 288-289 that name #930 and #931.
  - `docs/specs/protocol.md`: the field, and seeded entry ids.
  - `docs/capabilities.md`: ADR-0089's row.
  - The threat model: ADR-0089's transcript row, unchanged by this ADR.

## Alternatives considered

| Alternative | For | Against |
| --- | --- | --- |
| C2: rebuild the conversation from `seen` | no disk, no schema | no user prompts or node entries; 1 MiB of raw output; unredacted; duplicates on an appending report |
| C3 / U4: a state blob in the holder | survives without disk | link version 3; clean stop only unless every change is a round trip; gone with the holder; late on machines with a running v2 holder |
| C4 / O5: the adapter replays or re-announces | the adapter is the source of truth | a new adapter-protocol request and capability; depends on every adapter; still lacks the node's entries (C4) |
| U2: the hub infers "same agent" from the id | no schema change | cannot tell kept from resumed; changes a published event's fold silently; a rebuild of old logs differs |
| U3: totals in the node's ledger, re-stated | node-owned | the usage event is a growth (double count); a new event anyway; a second copy of the hub's totals |
| An enum on `AgentStarted` (new, same process, resumed) instead of a bool | could also give ADR-0093's unbuilt "kept" / "continued" words | resume's usage rule is undecided (open question 3); the window's words need a snapshot field anyway; a bool is enough for what is decided |
| O3: write the kept file on the reader's thread | smaller window | a disk write on the adapter's read path; still loses a line before the parse |
| O4: node acknowledges lines to the holder | closes in-flight loss for everything | at-least-once delivery doubles chat and usage without ids; link version 3; late on machines with a running v2 holder |
| Put the conversation in the ledger | one file | the ledger is replaced whole, bounded to 4 MiB, and holds "never ... prompts" by its own rule |

## Open questions

1. **Seeding before ADR-0089's schema step.** ADR-0089 orders its schema PR first. Proposed: build the writer and the seeding (no schema)
   first, because they fix #931 for every client today. The paging schema then follows ADR-0089's own order.
2. **Is U2 a change in meaning of a published message?** Proposed: yes, which is why U1 adds an explicit field rather than changing how
   every `AgentStarted` folds.
3. **Should a resume (ADR-0060) keep the counters too?** A resume is a new process continuing the same conversation (ADR-0089 decision 5
   shows the earlier turns above a "resumed" marker). Proposed: not in this ADR. `same_process` stays false for a resume, as the name
   says. If the maintainer wants resumed agents to keep their totals, a later ADR adds a separate field or value.
4. **ADR-0093 step 4 ("kept" against "continued" in the window).** The hub could derive "kept" from `same_process` after a `NodeOnline`,
   but the snapshot still needs a field to show it. Proposed: out of scope here; that step keeps its own ADR step.
5. **O5, the adapter re-announcing, later?** Proposed: only if a third-party adapter is found whose reports the holder's 1 MiB regularly
   pushes out. Agora's ACP bridge could then implement it first.
6. **A stop handler for `agora node` (O2).** Today `agora down`, a service restart and the restart after an update end the node like a
   kill, so up to 5 s of merged usage per agent is lost on every planned stop (the follow-up PR #522 names), and so is everything else in flight. Proposed:
   yes, as its own step after O1. The node drops itself on `SIGTERM` with a deadline under the supervisor's and systemd's stop timeouts,
   and exits anyway when the deadline passes. The holder keeps the agents either way.

## Build status

- Step 1 (O1, #1080) is built: `agora_adapters::host::replay` reads the `seen` lines once for the waiting requests and the newest options, config
  and commands of the recorded session (or a report that names none), and `adopt_held_pipe` puts those over the kept file's before bounding and
  judging them. Tested with a node whose reports were read and not recorded before it stopped (`hold_node_pipes`).
- Steps 2 and 3 (U1, #930) are built together, in one PR: `AgentStarted.same_process` (8) with its fixture (`schema/fixtures/same-process/`);
  the node sets it in `agent_taken_up` and in `announce_present`'s re-statement; the hub's fold (`projection.rs`) and the window's
  (`view.ts`) keep the tokens, model, context level and own cost (and in the window the start time) when it is set and the agent is
  listed from that producer. The terminal client needed no code change: it draws the hub's snapshot (a client test pins that a take-up asks for a fresh one). Tested with a view rebuilt from a log
  with a kept take-up, a resync re-statement, a new start of a reused id that still resets, the window's fold, and a node restart in
  `hold_node_pipes` whose hub view and terminal client header still count what the agent used before.
- Step 4 (C1, #931) is built: `crates/agora-node/src/transcript.rs` writes each structured agent's transcript as ADR-0089 decides
  (`<data>/transcripts/<id>.jsonl`, `0600` in a `0700` folder, a header line with the format version, one line per version of an entry with
  the call's tool and arguments and the plan mark beside it, a line for entries taken back out; every line carries the highest id given,
  and a rotated file or a fresh start's new file begins with a line holding only that, so the ids go on when no entry is left).
  - The chat path only marks changed ids; one writer thread takes the batch under the board's lock at most once a second or per 64 KiB,
    and encodes, replaces the node's own secret values and appends after letting it go. It alone writes, rotates and moves files. A
    failed write drops the batch (not retried), keeps the ring and says `chat.history_not_saved` once in that conversation.
  - Secrets: the node's stored values and the environment variables its policy passes to agents, of 8 bytes or more, in every text and
    JSON-escaped in a tool's arguments; a piece of 4 bytes or more at the edge of a text (a value streamed across two entries) or on the
    middle cut of a long output, thought or tool arguments; ids never changed.
  - Bounds: `chat.history_max_bytes` (32 MiB, only lowered, as ADR-0089 decision 2 says "lowers ... the defaults"; the newest half
    kept in a new file that says it was cut), `chat.history_days` (30, only lowered; 0: once the agent ended), counted from when the agent
    ended as the ledger says (the file's time is set to it, so it holds after the ledger forgets the agent), never for one running or
    offered for Resume; 2 GiB per node; `agora history purge`.
  - Seeding: the ring is filled from the file's end (at most 4 MiB) on take-up, before the ticker reads the holder, and on resume (read
    before the resume, applied under the supervisor's lock once it went through, so a refused one changes nothing). Seeded entries are
    held to the live path's shapes and bounds, ids below 2^52. A fresh start's file is moved aside (`<id>.earlier.jsonl`, replacing an
    older one) by the writer before anything of the new session, and its ids go on after it.
  - **Replay rule** (review of #1103, B1): an agent that loads its session on resume replays its conversation (ACP `session/load`,
    passed on by the bridge as chat before it answers the session's start). The node takes what a resumed session said before that
    answer as one replay, and shows it only when the agent's ring is empty (no file, `chat.history_max_bytes = 0`, purged); otherwise
    the ring already holds that conversation, and the replay is neither shown nor written again. This also stops a restart after a
    crash, within one node, from showing the conversation twice.
  - Tested with a kill then take-up (earlier turns with their ids, ids after them), a node that cannot write (one notice, the agent
    unaffected), a tool call that finishes after the restart (one entry, updated in place) and an ACP agent that replays its session on
    two resumes (each turn once, on screen and on disk), in `hold_node_pipes`; a confined agent that cannot read a transcript
    (`sandboxed_agent`); node unit tests and `agora-bench transcript`.
  - Not built: a forget at the hub (`agent.forget`, ADR-0053) is answered by the hub and never reaches the node, so it does not remove the
    file (retention does); the window's "resumed" and "earlier session" markers and paging stay ADR-0089's schema steps; a clean stop
    did not flush the last batch until step 5, now built.
- Step 5 (O2) is built, and with it the ADR: `agora node` catches `SIGTERM`, `SIGINT` and `SIGHUP` on Unix (`agora_platform::catch_stop_requests`,
  a stop watch thread in `crates/agora/src/main.rs`) and stops its node with `Node::stop_keeping_agents`: the supervision loop takes nothing more,
  `Supervisor::let_go_held` lets every held adapter go (`Adapter::let_go`, then its reader is waited for at most `DRAIN_WAIT`, 2 s, as a read of a
  holder that does not answer waits up to the link's 5 s frame time; `Adapter::detach` is bounded the same way) and takes what the readers had
  handed over as a tick does, with no restart and no queued prompt sent; the node records it through the loop's own paths (`supervisor_events`,
  and `chat_event` for the conversation; a waiting request is left to the next node's replay, ADR-0093 decision 3), then writes the transcript's
  last batch (`transcript::flush`), all the usage still being gathered (`EventSink::flush_all_usage`) and the ledger with the kept session files,
  and the process ends by the same signal with its default action (`agora_platform::end_as_stopped`), so the supervisor and systemd see the
  same end as before. It ends no agent: the holder's run on, and the node's own are left as a killed node leaves them (ADR-0060). The whole stop
  is bounded by `NODE_STOP_DEADLINE`, 5 s, after which the stop watch ends the process anyway: under `agora down`'s 10 s wait for the supervisor
  (`frontdoor::stop_role`, also `agora service remove`) and the systemd unit's `TimeoutStopSec=25`. A node dropped in-process drains the same
  way and still ends its own agents. Windows has no clean stop (`TerminateProcess` cannot be caught): there every stop stays a kill. Tested with
  a turn the node had read and not recorded when it stopped, an answer and usage still held by the writer and the merge, and a stopped holder
  (`hold_node_pipes`); with `agora down` twice, once with a stopped holder (`front_door`); and with a detach whose read hangs (`agora-adapters`).
  Still lost: on a kill, as before; on a clean stop, lines a reader still waiting on a holder that does not answer is handed after the wait (the
  holder counts them given), and a waiting request is asked again rather than recorded.
  Review of #1108: a call waiting on a held adapter for its answer when the stop lets it go was given (its line is written): it is answered
  `HostError::Detached` and recorded as sent or decided (a prompt stays in the conversation, a queued prompt being sent is shown sent, a
  delivered decision is recorded as decided), never as not delivered. Control commands are refused with `node.offline` once the stop
  begins, and those already running and the loop's last round are waited for at most 1 s (`LOOP_WAIT`) before the flushes, which come
  before anything else is waited for. A request before the node is up and a second request end the process at once, and a signal the node
  was started ignoring stays ignored (`catch_stop_requests_once`). A node dropped in a process that goes on lets go of its supervisor's
  hooks, which held the mesh in a cycle, so the next node there serves the agents' mesh endpoints. Tested in `hold_node_pipes` (a prompt,
  a queued prompt and a decision in flight; the next node on the mesh), `front_door` (a stop while starting, an ignored hangup) and
  `agora-adapters` (a waiting call answered `Detached`).
