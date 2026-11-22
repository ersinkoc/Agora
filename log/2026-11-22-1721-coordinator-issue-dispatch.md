From: coordinator (maintainer-directed)
To: linux (Claude), windows (Claude), macos (Claude)
Re: log/2026-11-22-1705-linux-0334-update-plan.md, log/2026-11-22-1629-linux-backlog-plan.md
Kind: backlog organization & issue dispatch

Current state: `#979` is merged; the 0.3.34 release only waits on the three remaining open PRs (`#1001` for `#924`, `#998` for `#933`, `#999` for `#925`).

## 1. Immediate gate (all machines)
1. **linux (Claude, merger):** Finish merging `#1001`, `#998`, `#999` once Codex clears `#998`/`#999`, cut `v0.3.34`, post here when signed assets are up, and update `blackview`.
2. **windows (Claude) & macos (Claude):** Once `v0.3.34` assets are announced here, run the `0.3.34` update checklist from `log/2026-11-22-1705-linux-0334-update-plan.md` and report `agora --version`, `agora status`, and hub visibility.

## 2. Post-release / non-conflicting issue lanes (one task, one worktree, one branch)

### linux (Claude, `blackview`) — `next` queue
Take these sequentially after `0.3.34` ships:
1. **#926** (`test: launch_options confined_only_needs_the_agents_confinement fails on blackview (control_token_reachable)`): reproduce on `blackview` and determine whether the test assumes a clean home or the host leaks the token path.
2. **#996** (`hub: a failed NodeLink { up: false } write leaves a machine that left shown online`): decide recovery (retry or deferred unrecorded-end flush) in `crates/agora-hub/src/hub.rs` + failing-store test.
3. **#942** (`pairing: an approval from the window leaves the hub open and the pair open terminal still asking (RM-07)`): ensure window approval closes pairing and ends the `pair open` prompt cleanly.
4. **#904** (`team mode: a grant needs an internal project id the window never shows; unknown names are accepted silently (RM-26)`): surface/accept project names or ids clearly, reject unknown grants with a reason, fix non-owner empty state and wording, list `hub user` in `agora --help`.

### windows (Claude, `WHITE`) — `windows` lane
Can start on fresh `origin/main` worktrees now (or right after the `0.3.34` update) without touching the release PRs:
1. **#973** & **#972** (test fixes):
   - `#973`: update `history_comes_before_the_snapshot_only_when_asked_and_never_with_a_resync` to accept both ConPTY line-ending/escape patterns.
   - `#972`: gate `sh`-requiring tests on `sh` being on `PATH` (in `crates/agora-node/src/control.rs` and `crates/agora-tui/tests/history_replay.rs`).
2. **#896** (confinement M2.3): allow confined agents to read+execute the node's `agora.exe` (or place the mesh bridge in a Low-token-readable location) so `agora mesh stdio` works, with a test.
3. **#895** & **#898** (confinement M2.3): investigate which `libuv` `CreateProcess` flag/attribute fails with `EPERM` under the Low token (and run the Rust spawn probe from `#898`); clean up the `System32` grant log noise and document the wildcard-port bind rule.
4. **#897** (confinement M2.3): `confine_command Unsupported` for structured/adapter agents — prepare the decision/ADR option (confine vs. explicit typed error + docs) before implementing.

### macos (Claude) — `tracking` / Mac lane
After updating to `0.3.34`:
1. **#830** (Mac `hold` rig & `LocalStream` isolation):
   - Measure the Mac `hold` rig flake rate on current `main` (specifically `a_buffer_is_not_lost_when_the_link_breaks_during_attach`).
   - Move the macOS peer-closed `set_read_timeout` `InvalidInput` handling from `holder_link::receive_budget` into `agora-platform`'s `LocalStream` (propagating other errors).

## 3. Parked / needs maintainer decision first
- **`needs-adr`:** `#931` (kept structured agent chat history empty after node restart) and `#930` (kept agent usage counters reset to 0 after node restart) — do not implement without an accepted ADR.
- **`low-priority`:** `#1000`, `#993`, `#983` — hold until the lanes above are clear.

Please reply on `comms` when you claim an issue (and reference the issue/PR number) so lanes stay conflict-free.
