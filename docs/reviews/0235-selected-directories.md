# Selected working folders — issue #235 / PR #236

ADR-0052 was accepted after the maintainer reviewed the concrete selected-folder and portable-identity proposal and
instructed implementation. This change adds directory launch; it leaves published `AgentStart` semantics intact.

## Result

- Web: choose machine → project/folder → use folder → agent → start. Existing folder is the default; an isolated Git
  copy is an explicit checkbox. CLI: `agora agent start KIND --cwd PATH [--worktree]`. No node restart for new folders.
- Git subdirectories and non-Git folders work. Several sessions share a directory without overwriting user MCP files;
  private integration configs remain per session. Stop/failure preserves the selected directory and its edits/branch.
- Optional `.agent-project.json` contains a random `project_id`. Explicit `project init --cwd PATH` or the launch checkbox
  creates it without overwriting or committing. Carrying it to clones groups the project across nodes, preserving paths
  and sessions. A changed confirmed association requires explicit signed confirmation. Identity does not sync files.
- New workspace state feeds files, Git queries and the project filter; shared changes are listed once. A new adapter
  opens its conversation directly in the focused view instead of an empty terminal.
- Directory requests bind exact node/path/project/kind/options/expiry with a trusted-device signature and persisted replay
  prevention. Generic start grants cannot substitute a folder. Scoped devices cannot register arbitrary folders.

Wire messages, numeric tags and signature encoding are enumerated in `../specs/protocol.md`. No new dependencies.

## Verification (Ubuntu, 2026-11-11)

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass.
- New real-node fake-agent tests cover exact cwd with spaces/subdirectories, dirty files/config preservation, two shared
  sessions with distinct integration configs, manual terminal/adapter resume, non-Git folders after node startup, failed
  start cleanup, and explicit worktree creation. No real agent/accounts/tokens are used.
- P-256 tests reject field substitution, expiry, cross-node use, absent signatures, restricted devices and replay after a
  restart. Mixed-version router tests refuse directory commands before forwarding when the node did not negotiate
  `workspace.directory`, while retaining legacy requests. Hub scope tests hide ungranted workspaces even when another node carries the same portable project ID.
- Web: 80 tests pass, including cross-node project grouping without session/path merging and workspace snapshot reload.
  TypeScript and production build pass. i18n lint, docs checks and additive-schema comparison against main pass. Existing
  protocol golden files are unchanged; new messages have new fixtures.
- Browser checks use a disposable hub/node on port 17443, separate from the active local installation. The existing
  sign-in/terminal/chat/signed-approval flow and the selected-directory flow use Chromium and fake agents/shell fixtures.
  Screenshot commands and artifacts are listed below.

## Browser performance

`PW_DIR=<playwright-core directory> node scripts/perf-browser.mjs 2` on the same machine, with 20 fixture agents:

| Measurement | Observed |
| --- | --- |
| Keystroke to screen text (99 samples) | p50 2.9 ms, p95 14.1 ms |
| Keystroke to next animation frame | p50 13.2 ms, p95 29.9 ms |
| 2 MB flood | 0.4 s, 4.6 MB/s; zero long tasks |
| Select among 20 agents to terminal prompt (12 samples) | p50 50 ms, p95 97 ms |

Text echo and terminal snapshot meet their respective targets in this run. Headless next-frame pacing exceeds the 16 ms
interaction target; the flood achieved 4.6 MB/s, so this run does not establish the 10 MB/s rendering budget. Budgets are
unchanged. The later four-terminal flood completion check timed out after 180 s (zero observed long tasks, 17 MB JS
heap); aggregate throughput is inconclusive and is not counted as a passing rendering-budget check. Shared-directory Git status is polled once per path; legacy and new workspace bindings receive separate events.

## Screenshots and reproduction

Build fake binary and web assets, then run:

```
cargo build -p agora -p agora-fake-agent
npm --prefix apps/web run gen
npm --prefix apps/web run build
PW_DIR=<playwright-core directory> node scripts/e2e-browser.mjs
PW_DIR=<playwright-core directory> AGORA_SCREENSHOTS=docs/screenshots/selected-directories node scripts/e2e-directories.mjs
```

Both themes: [selected folder (dark)](../screenshots/selected-directories/selected-folder-dark.png),
[selected folder (light)](../screenshots/selected-directories/selected-folder-light.png),
[shared sessions (dark)](../screenshots/selected-directories/shared-sessions-dark.png),
[shared sessions (light)](../screenshots/selected-directories/shared-sessions-light.png),
[Git option (dark)](../screenshots/selected-directories/git-folder-dark.png),
[Git option (light)](../screenshots/selected-directories/git-folder-light.png),
[Turkish (dark)](../screenshots/selected-directories/turkish-folder-dark.png),
[Turkish (light)](../screenshots/selected-directories/turkish-folder-light.png).

## Limits and handoff

- New directory inspection/launch initially supports unrestricted node-trusted owner devices; project-limited members
  are refused. Workspace read queries remain project-scoped for members/viewers. Legacy clients/TUI keep `AgentStart`.
- A fixed-file-only MCP manifest cannot safely redirect its integration config in a shared directory; use the explicit
  isolated copy for that manifest. Built-in CLI/environment-injected integrations retain separate session files.
- Existing external terminal processes are not adopted. Project IDs do not synchronize edits or prevent two agents'
  shell commands from editing the same file. Leases apply to writes mediated by the node.
- Linux checks ran locally. macOS/Windows checks are CI responsibilities; no platform implementation was changed here.
- Local activation waited until all four existing real-agent sessions had stopped. The old executable and hub database
  were backed up, then the release preview was installed and both services restarted. A live Turkish-browser smoke check
  confirms sign-in, the online Ubuntu node, the selected project folder and shared-identity option, without starting an agent.
  No running session was killed or migrated. The unmerged preview is reversible; cross-review/maintainer merge still apply.
- PR requires cross-review and a maintainer merge under AGENTS.md. This agent does not merge it.
