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
| P2-16 adapter starting in a skipping mode | done for `forbid` and `confined_only` | start refused, later stopped with a notice; under `allow` such an agent still runs unasked (open, see below) |

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
  the original flow creates a worktree per agent (new selected-directory launches supersede this default in ADR-0052 / #235). Never depend on a hub connection.
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

### P2-16 An adapter that starts in a mode that skips permissions
- **Crates:** `agora-node`
- **After:** ADR-0090 steps 3 and 4
- **Do:** when an adapter reports its first `session/options` or `session/config`, judge the CURRENT mode (and any current boolean or
  select the node's rule flags, `agora_adapters::perm`) against the `skip_permissions` policy, as `AgentSetMode` is judged. Today only a
  change the user asks for is checked: an adapter that starts in a bypass mode (its own default, or a saved setting) is not refused.
- **Done when:** under `forbid` such an agent is stopped or switched to a safe mode with a notice, and a test with the fake ACP agent
  starting in a bypass mode proves it.
- **Status: done for a refusing policy.** Under `forbid`, and `confined_only` for an agent not launched confined, the node judges what
  an adapter agent has in use with the rule a switch is judged by (`skip_in_use_refused` in `supervisor.rs`): its current mode or model,
  or a config option's value (a boolean by its value: one whose words do not say which value protects counts at either value). Each
  report is judged as sent, before it is bounded, so an id in use that is not in its list is judged on the id alone. A start or resume
  first reads the adapter's first reports (up to 2 s) and is refused with `agent.option_forbidden` (the id in use, the policy) before the
  agent can take a prompt; nothing runs. Later (a restart after a crash, a take-up from the holder, a switch the agent makes itself) the
  agent is **stopped** (no safe mode is chosen for it: the node cannot know which of an agent's modes asks, and a stop needs no trust
  in the adapter): its process killed off the supervisor's lock, no restart, its reports not kept or passed on, no queued prompt sent,
  out of the mesh, its adapter's requests and skip cards withdrawn, and its conversation gets the node's notice
  `agent.skip_permissions_refused` (`policy`, `setting` `mode`, `model` or `config`). No prompt reaches such an agent meanwhile (the
  node refuses it), nor one whose first reports have not come (`adapter.no_answer`, call `session`). Tests:
  `crates/agora-node/tests/skip_approval.rs` (start refused for a mode, an option and an unlisted mode, `confined_only`, a late first
  report, a self-switch, a queued prompt in the refusing tick, a resume, a restart after a crash, `allow`), `crates/agora/tests/acp.rs`
  (the fake ACP agent with `--start-bypass`), `crates/agora/tests/hold_node_pipes.rs` (take-up), and the unit test of the judge.
  **Open:** under `allow` such an agent runs without anyone being asked (shown "Acts freely"); whether the owner should be asked then
  (ADR-0090 Amendment A covers only a switch asked for from a window, and says an agent cannot request one for itself) is a decision
  for the maintainer. Also for the maintainer: under `forbid` a real kind whose own default mode skips (or one the classifier cannot
  place, such as `auto` or `acceptEdits`) is refused at every start with no way to choose a safe mode; whether the start should offer
  one, or the node pass a safe mode at start, is open.
