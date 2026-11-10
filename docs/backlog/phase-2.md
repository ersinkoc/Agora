# Phase 2 — Node core (Linux, macOS)

Exit criteria (roadmap): a node can run agents in worktrees and report state, controllable via CLI locally;
node capacity and footprint budgets measured (`design/performance.md` §1).

Prerequisites: Phase 1 merged (protocol, schema, domain model v0, event sequencing) and ADR-0012 and ADR-0013
accepted (both are). Tasks marked `after P2-xx` depend on that task's merge. P2-01 and P2-04 can run in
parallel; most others touch different crates.

## Status (2026-11-07)

| Task | State | Notes |
| --- | --- | --- |
| P2-01 PTY | done | |
| P2-02 process inspection | done | macOS path verified by CI only |
| P2-03 VT backend | done | |
| P2-04 real bench path | done | `channel-throughput` kept so the gate has a stable base; new `pty-vt-throughput`, `terminals-20` |
| P2-05 terminal actor | done | blocks on output; input/resize/kill via `PtyControl` |
| P2-06 git and worktrees | done | |
| P2-07 files | done | polling watcher; native watcher can replace it |
| P2-08 L1 engine | done | |
| P2-09 first manifests | **needs a human** | fixtures must be captured from real agents with dedicated accounts; agents never use real accounts (AGENTS.md) |
| P2-10 supervision | done | |
| P2-11 event buffer | done | |
| P2-12 accounts | blocked on P2-09 | |
| P2-13 resource limits | done | cgroups v2 and Job Objects not yet |
| P2-14 local CLI | done | `agent list` came with ADR-0025 (accepted); `agent kinds`, `agent start|stop|prompt` and `terminal attach` are there too |
| P2-15 footprint | done | all budgets met, see `design/performance.md`; nothing relaxed |

Exit criterion: a node runs agents in worktrees and reports state, controllable via the local CLI
(`agora node run`, `agent start|stop`, `terminal attach`; covered end to end by `crates/agora/tests/cli.rs`), and
the footprint budgets are measured. Open: P2-09/P2-12 (human-captured fixtures), ADR-0025 for `agent list`, and
cross-review by Codex of all Phase 2 PRs (AGENTS.md).

### P2-01 Platform: PTY and process basics (Unix)
- **Crates:** `agora-platform` (`unix.rs`, `linux.rs`, `macos.rs` as needed)
- **Read:** `architecture.md` §2.1, ADR-0013, `AGENTS.md` (platform isolation)
- **Do:** open a PTY, spawn a child in it, resize, read on a dedicated OS thread into a bounded channel,
  write input, kill the process group, report exit status. A platform-neutral trait in the crate root; no
  `#[cfg(target_os)]` outside this crate.
- **Done when:** conformance tests (spawn `sh`, echo, resize, kill) pass on Linux and macOS in CI.

### P2-02 Platform: process inspection
- **Crates:** `agora-platform`
- **After:** P2-01
- **Do:** foreground process name, process tree, and cwd of a terminal's process (`/proc` on Linux,
  `libproc` on macOS).
- **Done when:** a test starts a child that changes directory and runs a known foreground process; both are
  reported correctly on both OSes.

### P2-03 VT trait and first backend
- **Crates:** `agora-vt`
- **Read:** ADR-0012, ADR-0022, `design/performance.md` §3
- **Do:** `Terminal` trait (feed bytes, resize, read cells, read bottom lines, cursor, modes) with an
  `alacritty_terminal` backend; shared conformance tests so another backend can be swapped in.
- **Done when:** conformance tests pass; "bottom lines" never include scrolled-back content.

### P2-04 Replace the bench stand-in with the real path
- **Crates:** `bench/`, `agora-fake-agent`
- **After:** P2-01, P2-03
- **Do:** make the `channel-throughput` scenario run fake agent → PTY → VT backend at 10 MB/s; keep the
  result format; add a scenario for 20 concurrent terminals.
- **Done when:** the CI comparison still gates on regressions with the new scenarios.

### P2-05 Terminal actor in the node
- **Crates:** `agora-node`
- **After:** P2-01, P2-03
- **Read:** ADR-0013, `design/performance.md` §2
- **Do:** one I/O actor per terminal: PTY thread → VT feed → snapshot/subscribe API; bounded buffers, input
  has priority over output; scrollback with a configurable cap.
- **Done when:** a test drives `agora-fake-agent` through a terminal and reads consistent snapshots; a
  flooding terminal does not delay input to another.

### P2-06 Git and worktrees
- **Crates:** `agora-node`
- **Read:** ADR-0003, `architecture.md` §3
- **Do:** discover checkouts, compute project identity (`agora-core`), create and remove worktrees and
  branches, status and diff; refuse to remove a worktree with unmerged work without an explicit flag.
- **Done when:** tests on temporary repositories cover create/remove, dirty detection (`worktree.dirty`),
  and an existing branch (`worktree.exists`).

### P2-07 Files: list, read, write, watch
- **Crates:** `agora-node`
- **After:** P2-06
- **Do:** lazy directory listing, bounded reads (`files.too_large`), writes that go through lease checks
  (`lease.held`), and change watching with batched `FilesChanged` events.
- **Done when:** tests cover path escape attempts (`..`, symlinks out of the worktree) and a lease conflict.

### P2-08 L0 and L1 adapter engine
- **Crates:** `agora-adapters`
- **Read:** `specs/agent-manifest.md`, ADR-0004
- **Do:** parse and validate manifests, evaluate screen rules against `agora-vt` bottom lines, state
  arbitration with reported state (`agora-core::Agent::observe`), adaptive detection interval.
- **Done when:** fixture screens produce the expected states; a hidden terminal still gets detection.

### P2-09 First manifests
- **Crates:** `manifests/agents/`, fixtures
- **After:** P2-08
- **Do:** `claude-code`, `codex`, `wrongstack` manifests with fixture screens captured from the real
  agents (manually, with dedicated accounts; never commit tokens or private content).
- **Done when:** each manifest's fixtures pass in CI, and rules read only the live bottom of the buffer.

### P2-10 Agent supervision
- **Crates:** `agora-node`
- **After:** P2-05, P2-06, P2-08
- **Do:** start, stop, resume (native session ref, then layout-only fallback), restart policy with backoff;
  every agent gets its own worktree by default. Never depend on a hub connection.
- **Done when:** integration test with `agora-fake-agent`: start in a worktree, state changes reported, stop,
  resume.

### P2-11 Local event buffer
- **Crates:** `agora-node`, `agora-core`
- **After:** P2-10
- **Read:** `specs/protocol.md` §3, `agora-core::events`
- **Do:** persist unacknowledged events to disk (append-only), replay in order after reconnect, cap size.
- **Done when:** a test kills and restarts the node and replays exactly the events after the last ack.

### P2-12 Agent accounts
- **Crates:** `agora-node`, `agora-adapters`
- **After:** P2-09
- **Read:** `design/agent-accounts.md`, threat model T7
- **Do:** isolated config directories per account, login flow in a terminal, status checks raising
  `agent.not_authenticated`; secrets only in the OS keychain or the agent's config, never in events.
- **Done when:** tests with the fake agent cover login, status ok/failed, and two accounts side by side.

### P2-13 Resource limits
- **Crates:** `agora-node`, `agora-platform`
- **After:** P2-10
- **Read:** `design/resource-limits.md`
- **Do:** max agents and terminals, output rate coalescing, worktree disk usage reporting; cgroups v2 on
  Linux and rlimits on macOS where available (platform code in `agora-platform`).
- **Done when:** exceeding a limit returns `node.resource_limit` with params.

### P2-14 Local CLI control
- **Crates:** `agora` (binary), `agora-node`
- **After:** P2-10
- **Do:** `agora node run` and CLI subcommands over a local socket (`agent start|stop|list`,
  `terminal attach`); each maps 1:1 to a protocol command.
- **Done when:** the exit criterion above is demonstrable by script.

### P2-15 Footprint measurement
- **Crates:** `bench/`
- **After:** P2-14
- **Do:** scenarios for 50 concurrent terminals, idle CPU, base and per-terminal memory, cold start.
- **Done when:** results are compared to the budgets in `design/performance.md` §1 and recorded; the
  document is recalibrated if a budget is unreachable (never silently relaxed).
