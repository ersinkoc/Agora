# Follow-ups (from 2026-11-26)

Small leftovers, suspected bugs and flakes that used to be GitHub issues. From 2026-11-26 the repository keeps no open issues or pull
requests: work lands on `main`, and what is left is listed here or in the [roadmap](roadmap.md). An item is removed in the commit that
fixes it; one that needs a decision says **decision**, one that needs another machine says **win** or **mac**.

Sources: #1150 (the low-priority list from the 2026-11-23 reviews onward), #1125 and #895, all closed with a link here.

## Bugs and design gaps

- **A task handoff across a hub restart** (was #1125; ADR-0039, ADR-0109 stage 2). Stage 1 (the append-only run identity, delivery and
  receipt; Amendment A's bounds) is merged. Left: the hub and node persistence paths; a durable node inbox entry bound to the receiver's
  run and kept through holder take-up, with one receipt for retries; the entry keeps its message number (the MCP `inbox` cursor is
  numeric and survives a node restart). Acceptance: a restart between commit and send still delivers, and a lost acknowledgement does not
  make a second inbox entry.
- **A stored handoff record that cannot be read at node start** (#1194 nit): left in place and still counted, so the hub's retry is stored
  as a new entry and the agent can see that handoff twice; needs an I/O error that lasts through the retry. The record write, fsync and
  prune also run under the broker mutex.
- **Board handoffs and members** (**decision**): a handoff made with the hub board's `handoff` tool is stamped `producer_id: "hub"`, so
  `Scope::admit` hides it from a member even when both ends are theirs (`lease_visible` special-cases the hub id; messages and handoffs do
  not).
- **A request restated after its decision.** The announcer copies `approvals.waiting()` and records it later (`EventSink::announce_present`);
  an adapter or skip request is queued before `ApprovalRequested` is recorded. Either way a card can come back stale until the next
  announcement. Cleanest fix: the hub ignores an `ApprovalRequested` for an id it has already seen resolved.
- **Helper-start request race** (node): the re-announce path can send a request between `approvals.request_start` and `requests.insert`
  (`spawn.rs`); decided in that window, it is filed afterwards and never expires.
- **Linked-worktree pinning and a rewritten `commondir`** (needs a proof): with git 2.53, refs (`HEAD`, `refs/heads/<wt>`) may still
  resolve through a rewritten `commondir`; the test checks config and hooks only.
- **Board merge leftovers** (ADR-0115): no `TaskChanged` events on a merge; no `task_moved` notice for a renamed task's old id; the
  board and its `board-merged.*` markers are written separately (a failed marker write merges again on restart, duplicating tasks under
  new ids); a first board's lease whose holder no longer runs wins the path; overlapping but different lease paths are refused silently by
  `import_leases`; owed board-merge notices live in the hub's memory and a hub restart drops them.
- **Confinement on Linux, residuals of #1146:** a grant that leads below a closed path (anything inside the node's data folder) passes
  both checks (no no-follow walk like macOS's #545); where `/home` links into `/var` (Fedora Atomic), the default `/var` read grant holds
  the node's folder and every confined start is refused (needs a design: leave the home folder out of that grant).
- **`.agent-project.json` written in place** (`directory.rs`): a kill between creating and writing it leaves an empty file that refuses the
  folder; write beside it and link into place without replacing.
- **Stop during a pending restart** (control.rs, needs a proof): may emit a second `AgentExited(i32::MIN)` and `TerminalClosed`.
- **`pump`'s lag path** (control.rs, needs a proof): after a restart the resync can reach the next process's handle; likely needs a
  launch-id check.
- **Held terminal end check:** a `list` round trip to the holder on the terminal's actor thread (at most once a second while output
  flows); a hanging holder can stall a busy held terminal for up to 10 s. Learn of a held program's end off that thread.
- **Pairing client** (not proven): a dial timeout while a name lookup runs on the blocking pool can make `agora join` outlast its wait by
  the resolver's timeout; shut the runtime down without waiting.
- **Fleet answer** (F182x, suspected): an offline machine keeps its last `agents_running` while none of its agents are listed; a `project`
  id cut at 160 bytes makes `agents_list {scope:"hub", project}` miss other machines' agents; the hub answers `fleet_status` without
  checking that the asking node's link agreed `mesh.fleet_status.v1`; two clones differing only in case on one older node get commands in
  the latest clone's spelling; a `Repo.GIT` remote does not fold (`.git` stripped case-sensitively).
- **Web and terminal client** (suspected, #1232): a sound waiting for its 1 s gap still plays after `AgentAlerts` stops knowing the fleet;
  sound settings have no storage listener across windows; on Windows AltGr arrives as Ctrl+Alt, so a Ctrl+Alt+Q binding takes AltGr+Q from
  text fields; a chord started outside a terminal completes before the terminal-keeps-its-keys rule; `agora … 2>&1 | head -1` can panic
  writing to a closed stderr; the terminal client's `load_file` checks a theme's size, then reads the whole file.
- **Title rules** (#1204): the stale title after a denied or escaped approval and after Esc mid-turn is not captured (**human**: real
  captures).

## Windows (**win**)

- **Node.js agents cannot spawn when confined** (was #895; ADR-0105). Cause in libuv's `src/win/pipe.c` (the server end of the stdio pipe
  is created with NULL security attributes). Filed upstream as libuv/libuv#5464 with a fix proposed in libuv/libuv#5467. Done when a
  Node.js release carrying the fix is verified confined on Windows (ADR-0105 decision 4); until then the docs say Node-based agents do not
  work confined on Windows.
- **Holder link G2:** widen `holder_link_write_bound`'s window if it ever flakes (none seen).
- **Shutdown-safe state:** a real log-off / `shutdown /r` run of #1231's session-end window (on hold until the maintainer says go).
- **F1500 test variant:** `a_lagged_subscriber_of_a_crashed_process_is_never_repainted_with_the_restarted_one` is Unix-only; a Windows
  variant should fall behind without a flood (a smaller subscriber queue in a test).
- **Performance budgets missed on Windows 11** (ADR-0022, tracked, not relaxed): node cold start 2 760 ms cold / 197 ms warm against
  200 ms (mostly serial `git.exe` spawns; the side-by-side identity queries are merged, not measured on Windows yet); keystroke → echo p95
  18.21 / 17.45 ms against 16 ms (likely the ~15.6 ms timer tick in ConPTY, not measured alone).

## OS shutdown notices (**decision**)

- A logind delay lock on Linux (`Inhibit("shutdown:sleep", …, "delay")` with `PrepareForShutdown`) and a macOS power-off notice, so the
  node writes its state before anything is signalled. Linux needs D-Bus.

## Flaky tests (fix with a proof)

- `mesh_bridge::while_disconnected_a_batch_gets_a_batch_of_errors_and_notifications_get_nothing` ("an array", 1 in 350 at 12 threads).
- `mesh_bridge::the_bridge_ends_when_the_clients_output_is_gone` ("the bridge lingers", 1 in 350).
- `hubcli::store_tests::overlapping_saves_never_mix_two_enrollments` (hubcli.rs:2292, 1 in 40 suite runs).
- `agent_spawn::a_helper_acts_without_asking_only_when_the_signed_decision_names_it` on Windows (agent_spawn.rs:1512): the args record
  can be read before it is fully written.
- `doctor::tests::a_version_probe_that_does_not_end_is_killed_and_counts_as_not_working` on Ubuntu (doctor.rs:1338): timing.
- Windows `control.rs` timing (two tests, seen on 0.3.40's main runs).
