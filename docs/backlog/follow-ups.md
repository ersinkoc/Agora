# Follow-ups (from 2026-11-26)

Small leftovers, suspected bugs and flakes that used to be GitHub issues. From 2026-11-26 the repository keeps no open issues or pull
requests: work lands on `main`, and what is left is listed here or in the [roadmap](roadmap.md). An item is removed in the commit that
fixes it; one that needs a decision says **decision**, one that needs another machine says **win** or **mac**.

Sources: #1150 (the low-priority list from the 2026-11-23 reviews onward), #1125 and #895, all closed with a link here.

## Bugs and design gaps

- **Shared MCP servers for Codex and WrongStack** (ADR-0122 step 3b): Codex takes MCP servers only as `-c mcp_servers.NAME...` overrides,
  which would put a filled secret on its command line (readable by other processes of the machine); find a way that keeps secrets off
  the command line (its config file under a node-owned `CODEX_HOME` overlay, or an env var it reads) and check it on a real version
  before its manifest gets a `shared_flag`. WrongStack's `--mcp-config` has not been checked for a second file either.
- **`agora node trust --from-hub` outside a terminal** (the maintainer, 2026-11-27, through Claude Code's `!`): it lists the devices and
  their fingerprints, then stops with "not an interactive terminal". Print the ready command for the listed devices
  (`--yes --fingerprint A --fingerprint B`) under the list, so the person compares and runs it in one step.
- **A CSP error in the window's console on every load** (seen by the maintainer, 2026-11-27): `long.js` (bundled through protobufjs) tries a
  tiny WebAssembly module for 64-bit multiplication, the hub's page policy refuses it (no `'wasm-unsafe-eval'`, on purpose), and the
  library falls back to plain JavaScript. Harmless, but noisy: take the probe out of the bundle (a `long` build without it, or an alias),
  never by widening the page policy.
- **The hub's own CA is not name-constrained**: a person who adds it to a device's trusted roots to silence the warning trusts it for
  every site. Constrain new CAs to the hub's names, and steer people to `agora remote tailscale --serve` (a real certificate) first.
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
  23 ms.
  - **Why** (probe on WHITE 2026-11-27, PowerShell 7.7 from the Store):
    - `pwsh` on `PATH` is an App Execution Alias (reparse tag `0x8000001B`).
    - Starting it activates the package: the process runs from `C:\Program Files\WindowsApps\…\pwsh.exe`, shows the shell as its
      parent, and is in a job of its own, not the terminal's (`IsProcessInJob`: ours false, any true).
    - What `pwsh` then starts breaks away into **no job at all** (any false). Terminating the terminal's job leaves `pwsh` running.
  - **What can hold them, after the fact:**
    - `AssignProcessToJobObject(terminal job, pwsh)` fails with error 5: a process already in a job joins only an empty job or one in its
      own job tree.
    - A **fresh empty job** does take `pwsh`, nested under the package's job.
    - A child in no job joins the terminal's job directly.
    - Children `pwsh` starts after that are again in no job, so holding them needs a repeated sweep: per terminal, every descendant of
      the shell found outside its job goes into it, and a process already in another job goes into a fresh empty job with the same limits
      and kill-on-close.
    - Anything started and ended between two sweeps, or orphaned by an ended parent, is missed.
  - **decision:** build that sweep (with the 1 s process snapshot `foreground_pid` already reads), or document the gap as a limit of the
    job on Windows.
- **Performance budgets missed on Windows 11** (ADR-0022, tracked, not relaxed): node cold start 2 760 ms cold / 197 ms warm against
  200 ms (mostly serial `git.exe` spawns; the side-by-side identity queries are merged, not measured on Windows yet); keystroke → echo p95
  18.21 / 17.45 ms against 16 ms (likely the ~15.6 ms timer tick in ConPTY, not measured alone).

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

- `preview::a_connection_carries_bytes_both_ways_unchanged_and_reaches_only_its_port` (tests/preview.rs:570, `accepted == probed + 1`):
  failed once in CI on ubuntu-latest at a7cb134f (reported by the Windows session, 2026-11-27); later runs on main passed. The count of
  connections the echo server accepted likely races the node's port probe (a probe that lands after `probed` was read). The same run's
  `web` job failed at "Install Chromium" (infrastructure).

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
