# Follow-ups (from 2026-11-26)

Small leftovers, suspected bugs and flakes that used to be GitHub issues. From 2026-11-26 the repository keeps no open issues or pull
requests: work lands on `main`, and what is left is listed here or in the [roadmap](roadmap.md). An item is removed in the commit that
fixes it; one that needs a decision says **decision**, one that needs another machine says **win** or **mac**.

Sources: #1150 (the low-priority list from the 2026-11-23 reviews onward), #1125 and #895, all closed with a link here.

## Bugs and design gaps

- **Task ownership across receiver reattachment** (ADR-0109): when the receiver's node process restarts instead of reconnecting,
  the existing board restart path releases its claims even if a holder keeps the run and its inbox. Preserve the claim of a
  still-running receiver across holder reattachment without giving it to a replacement run.
- **Durable node handoff writes**: the node writes, syncs and prunes records while it holds the broker mutex. Measure worst-case contention before moving I/O out of the lock.
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
- **Confinement on Linux, residuals of #1146:** a grant that leads below a closed path (anything inside the node's data folder) passes
  both checks (no no-follow walk like macOS's #545); where `/home` links into `/var` (Fedora Atomic), the default `/var` read grant holds
  the node's folder and every confined start is refused (needs a design: leave the home folder out of that grant).
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
- **`ask` to an agent on another machine** answers `mesh.recipient_unknown` (mesh-tools.md: `ask` stays node-local), although
  `agents_list` shows that id; a code that says the target is on another machine would explain it (seen from mac, 2026-11-26).

- **One memory limit for the node and every agent on Linux** (blackview, 2026-11-26 23:09): `agora service` runs the node, the holder
  and every agent (and what they start, builds included) in the one `agora-node.service` cgroup. When it reached 21.7 GB, systemd-oomd
  killed the whole unit (8 processes): the node and the agents the holder was meant to keep. Give each held agent its own transient
  scope (`systemd-run --user --scope`, platform code), so pressure ends one agent and never the node or the others; measure first.
- **WrongStack agents have no mesh** (external, WrongStack 1.7.22, 2026-11-27): `wstack --mcp-config <file>` starts Agora's
  `agora mesh stdio` and completes `initialize` and `tools/list`, but WrongStack never adds the server's tools to its catalog
  (`tool_search` finds none; `tool_use` says "not found" for every name). Reproduced with a one-tool stdio server; a project
  `.mcp.json` is not read and `wstack mcp add` refuses ids outside its own catalog. Nothing to change on Agora's side; a note went to
  WrongStack's developer. Recheck on the next WrongStack release (the `wrongstack` manifest's `[mcp]` stays as it is).

## Windows (**win**)

- **Node.js agents cannot spawn when confined** (was #895; ADR-0105). Cause in libuv's `src/win/pipe.c` (the server end of the stdio pipe
  is created with NULL security attributes). Filed upstream as libuv/libuv#5464 with a fix proposed in libuv/libuv#5467. Done when a
  Node.js release carrying the fix is verified confined on Windows (ADR-0105 decision 4); until then the docs say Node-based agents do not
  work confined on Windows.
- **Holder link G2:** widen `holder_link_write_bound`'s window if it ever flakes (none seen).
- **Shutdown-safe state:** a real log-off / `shutdown /r` run of #1231's session-end window (on hold until the maintainer says go).
- **Store PowerShell leaves the terminal's job:** `pwsh` installed from the Store (WindowsApps, the default install) started in a terminal,
  and everything it starts, is outside that terminal's job object (WHITE 2026-11-27: `cmd` -> `pwsh` -> `claude`/`codex`, the job lists
  only `cmd`; Windows PowerShell 5.1 and `cmd` children are inside). So the job's limits (`make_job`: process memory, CPU time) do not
  reach them. A stop still ends them: closing the pseudoconsole ended `pwsh` + `claude` after 1.3 s and `pwsh` + the `codex` tree after
  23 ms. Find out why (packaged-app activation) and whether a nested job or a per-process limit can hold them.
- **Performance budgets missed on Windows 11** (ADR-0022, tracked, not relaxed): node cold start 2 760 ms cold / 197 ms warm against
  200 ms (mostly serial `git.exe` spawns; the side-by-side identity queries are merged, not measured on Windows yet); keystroke → echo p95
  18.21 / 17.45 ms against 16 ms (likely the ~15.6 ms timer tick in ConPTY, not measured alone).

## Mesh

- **`agent_output` for a helper on another machine waits in place** (review of a383d034): it holds the agent's mesh connection for up to
  10 s, so that connection's other waiting calls (asks, `inbox_wait`, `agent_wait`) and a `notifications/cancelled` for it are not seen
  meanwhile. Make it a waiting call like `agent_wait` (the link's answer through the spawner's queue, answered by the session's poll,
  dropped on cancel, within the per-connection cap).
- **A person's input ends an `inbox_wait`** (the maintainer's decision relayed by agora-ac, 2026-11-27): when the node writes a person's
  input (a window's or the terminal client's `TerminalInput`, not a mesh-typed prompt) into an agent's terminal while that agent's mesh
  connection has an `inbox_wait` open, answer the wait at once with `[]` so the agent's turn ends and the prompt is read. Needs a signal
  from the supervisor's input path to the agent's mesh connections; until then the person interrupts the call (Esc).

## Previews (ADR-0088)

- **The hub relay (step 3) must carry `Host` and `Origin` unchanged**, as the node does (review of 96786517 and efadc92d), and a TLS-terminating reverse proxy on the
  loopback in front of a hub is not recognised by the node's probe (it shows its own certificate); the per-port deny list below would
  cover it.
- The owner's policy naming ports below 1024 that may be forwarded, and a per-port deny list (decision 3 says "unless the owner names
  them in policy"; today every port below 1024 is refused and nothing else can be denied by policy).
- **Hub step 3:** only the person who opened a forward may connect to it through the hub (the maintainer's decision, 2026-11-27); the node
  accepts a routed `PreviewConnect` for an open forward from its hub (review of d267e547). A local client that reads its control
  connection slowly still holds that connection's other streams while a preview send waits; the hub link avoids it with a local
  connection per preview connection.
- The default budgets are "no limit" for the rate and the forward's total (only the chunk, the window and the connection caps hold):
  choose real defaults once previews run against real dev servers.

## OS shutdown notices (**decision**)

- A logind delay lock on Linux (`Inhibit("shutdown:sleep", …, "delay")` with `PrepareForShutdown`) and a macOS power-off notice, so the
  node writes its state before anything is signalled. Linux needs D-Bus.

## Flaky tests (fix with a proof)

- `preview::one_connection_over_the_cap_is_answered_then_ended_with_budget` (tests/preview.rs:684): the program observed 33
  connections against a cap of 32 once on Linux on 2026-11-27. The isolated retry and a second node integration run passed.

- `skip_approval::a_queued_prompt_never_goes_out_in_the_tick_that_stops_the_agent` (tests/skip_approval.rs:1243, "round 1: the queued
  prompt reached the agent"): 2 in 3 full `agora-node` runs on blackview on 2026-11-27, at 490efc84 plus an unrelated change. A prompt
  reaching an agent its policy stops may be a real race in the P2-16 gate, not only timing: prove which before widening anything.
- `mesh_bridge::while_disconnected_a_batch_gets_a_batch_of_errors_and_notifications_get_nothing` ("an array", 1 in 350 at 12 threads).
- `mesh_bridge::the_bridge_ends_when_the_clients_output_is_gone` ("the bridge lingers", 1 in 350).
- `hubcli::store_tests::overlapping_saves_never_mix_two_enrollments` (hubcli.rs:2292, 1 in 40 suite runs).
- `doctor::tests::a_version_probe_that_does_not_end_is_killed_and_counts_as_not_working` on Ubuntu (doctor.rs:1338): timing.
- Windows `control.rs` timing: one more test seen on 0.3.40's main runs, name not recorded (the stuck-write test's 5 s budget is
  now 20 s).
- `unix::exclusive_bind_tests::the_panel_port_cannot_be_shared_while_it_is_open_and_is_free_again_after` on Linux (unix.rs:1395,
  the negative control `squat(port)` after the drop): 2 in 4 full `agora-platform` runs, never alone; likely another test of the run
  takes the freed port.
- `linux::held_output_tests::output_left_by_a_collected_agent_is_read_in_every_order` (linux.rs:2220): 1 in 7 full `agora-platform`
  runs on blackview, never alone; the panic line was not kept.
