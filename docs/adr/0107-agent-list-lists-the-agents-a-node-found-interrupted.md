# 0107. `agent.list` lists the agents a node found interrupted
Status: Proposed
Date: 2026-11-23

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.

Origin: issue #1121, found while fixing #1120 (PR #1122, merged as 1d02ecda).

Touches ADR-0006 (append-only schema: one appended field, no new message), ADR-0025 (`agent.list`), ADR-0060 (the session ledger,
`AgentInterrupted`, Resume and Start again) and `docs/specs/protocol.md` (§ session ledger).

## Context

After a node restart, `agent.list` answers from the supervisor plus, since #1122, the agents the ledger keeps as **ended**
(`Ledger::ended`): `running` false, their last state, their folder as `worktree_id`. An agent the ledger finds **interrupted** (it was
running when the node stopped) is reported to the hub with `AgentInterrupted` (`Event` field 38). The window and the terminal client
already show it from the hub's snapshot (`AgentSnapshot.interrupted`, `resumable`, `still_running`, ADR-0060) and offer Resume or
Start again. Only `agora agent list` on the machine itself does not show it, and that is the command a person reaches for when the
node has no hub (nodes are autonomous; ADR-0025 made `agent.list` for that case). An interrupted agent stays unlisted there until it is
resumed, for up to 14 days.

`agent.list` cannot say "interrupted" today:

- `AgentSummary` is `agent_id, kind, state, worktree_id, terminal_id, running, restarts`. `AgentState` has no interrupted value, and
  an interrupted agent is neither running nor finished.
- **Listing it as an ended row** (`running` false, last state) makes it read exactly like the ended rows #1122 added
  (`agent-3  claude-code  working  stopped`). That is the misreading #1121 is about. It is also wrong for an agent whose process
  outlived the node (`AgentInterrupted.still_running`): that process runs, and `agent.stop` addresses it (`control.rs`, ADR-0060's
  note on an agent that outlives its node), unlike a ledger-only ended row, which answers `agent.not_found`.
- **Borrowing another field** (an empty `terminal_id`, `restarts`, `AGENT_STATE_UNSPECIFIED`) changes a published field's meaning
  (ADR-0006).
- **The CLI reading the node's ledger file** is the "CLI-private" second protocol ADR-0025 rejected. It also only works when the CLI
  and the node share a home (`--socket` may name another node's socket), and it ties the CLI to a file format that is the node's own.

The local control socket has no event stream, so the CLI cannot get `AgentInterrupted` either. No published command answers it.

## Options

1. **An appended `AgentSummary.interrupted` bool** (with `resumable` and `still_running` beside it, or the CLI cannot say what can be
   done). The node lists each interrupted agent as a summary row with `running` false.
   - An old CLI (0.3.36 and earlier) ignores the new fields and prints the row as `<last state>  stopped`. That is the misreading this
     ADR is meant to remove, now shipped to every old CLI that asks a new node.
   - `running` is ambiguous for a still-running survivor: `false` says nothing runs while a process does; `true` says the supervisor
     runs it while it does not, and `agora status` (`frontdoor.rs`, `agents_running`, counts `running`) would count a process the
     node does not supervise.
   - Three fields copy what `AgentInterrupted` already defines.
2. **An appended `AgentListResult.interrupted`**: `repeated AgentInterrupted interrupted = 2`. It reuses the published event message as
   it is (`agent_id, kind, project_id, folder, parent_agent_id, resumable, started_at_ms, interrupted_at_ms, last_state,
   still_running`), the same values the node announces to the hub, built from the same ledger entry.
   - An old CLI never sees the rows: it prints what it prints today. A new CLI asking an old node gets an empty list: what it shows
     today. Nothing is misread in either direction.
   - `AgentSummary` and `running` keep their meaning: `agents` stays "what the supervisor runs, and the ended agents the ledger keeps".
     `agents_running` is unchanged.
   - One field, no new message, no new command, no extra round trip.
3. **A separate command** (for example `AgentInterruptedList` with its own result). It reads the same as option 2 for old readers, but
   adds a command and a result message, a second round trip for `agora agent list`, and a capability so a new CLI knows an old node
   cannot answer it.
4. **Leave it.** No schema change. A machine with a hub shows interrupted agents in the window and the terminal client; a machine
   without one shows nothing about them on the command line for 14 days, and `agora agent stop` on a still-running survivor works on an
   id the list never showed.

## Decision (proposed)

Option 2.

1. **Schema.** `AgentListResult` gains `repeated AgentInterrupted interrupted = 2;`, appended. A golden fixture is added for an
   `AgentListResult` holding one summary and one interrupted entry; no published fixture is edited.
2. **Node.** `Cmd::AgentList` fills it from `Ledger::interrupted()` (the entries found interrupted and still offered, oldest id first),
   leaving out any id the supervisor holds. Each entry is the `AgentInterrupted` the node announces for it (the same builder, so
   `resumable` and `still_running` are the values the window shows). Read before the supervisor's lock, as #1122 reads `ended`. An agent
   that is resumed leaves this list and appears in `agents` from the supervisor.
3. **No capability.** The field is in a result, not a request: an old node sends none, an old reader skips it (protobuf unknown field).
   A hub that relays `agent.list` to a window may drop it if the hub is older; windows already have interrupted agents from the
   snapshot and do not need it.
4. **CLI.** `agora agent list` prints one list in id order across both. An interrupted row keeps the four tab-separated columns (id,
   kind, last state, then `interrupted` where the others say `running` or `stopped`) and adds a fifth, what can be done: `resume`
   (`resumable`), `start-again` (not resumable), or `still-running` (the process outlived the node; `agora agent stop <id>` ends it). The
   column values are fixed tokens like the existing `running` and `stopped`, not prose; the CLI's prose (an empty list, errors) stays in
   `locales/`. Lines of existing rows do not change, so scripts that read four columns keep working.
5. **Resume from the CLI is not part of this ADR.** There is no `agora agent resume` today; Resume and Start again are in the window
   and the terminal client. A CLI verb (it sends `AgentResume`, a decision like a start) is a follow-up issue.
6. **Docs.** `docs/specs/protocol.md` § session ledger: the sentence "An interrupted agent is reported with `AgentInterrupted` (below)
   and is not listed" becomes "is also listed in `AgentListResult.interrupted`". CHANGELOG line for the CLI.

## Consequences

- `agora agent list` shows every agent the node knows after a restart: running, ended, and interrupted with what can be done.
- `AgentSummary`'s meaning and every existing CLI line stay as they are; `agora status` counts the same agents.
- One more field to keep forever. It reuses a published message, so its contents are already defined and fixed.
- An interrupted helper (ADR-0057) is listed like any interrupted agent; its `parent_agent_id` is in the entry, and the CLI does not
  print it (a later column could).
