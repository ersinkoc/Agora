# Findings from the three-machine LAN test (2026-11-11)

A Linux hub with nodes on Linux, macOS and Windows 11, real shells, and real Claude Code and Codex on the Windows node. Messages in the `comms`
branch (`docs/fordevs/`). Each row is something the run showed that no CI run had.

| # | Finding | State |
| --- | --- | --- |
| L1 | A bundled macOS app without `NSLocalNetworkUsageDescription` (and with no stable code identity) cannot reach a hub on the LAN and macOS never asks | fixed in #250 (plist key and an ad-hoc identity) |
| L2 | The desktop shell saved a hub it could not verify and said nothing; it also sent the TLS name `localhost` for an IP | fixed in #242 |
| L3 | `agora tui` mapped an IP address to `localhost`, so it could not reach a hub by IP | fixed in #243 |
| L4 | Windows clients that use the system TLS stack (schannel: `curl.exe`, probably others) refuse the hub's certificate: `the revocation status is unknown` (it has no CRL or OCSP). `--ssl-no-revoke` passes | open: decide whether the hub certificate needs a revocation story, or document it; Edge and Firefox not yet checked |
| L5 | The Windows node id seen in the UI is the machine's host name, while mesh addresses use the node id from enrollment (`win/agent-1`, not `WHITE/agent-1`) | implemented in #263: show enrolled machine IDs and selectable/copyable canonical agent addresses; two-node same-hostname browser check preserves distinct navigation and project counts |
| L6 | A new manifest file is not seen by a running node until it restarts (`agent kinds`) | fixed: the node reads its manifest folder again before it lists or starts agents (a kind whose file was deleted stays until restart) |
| L7 | Claude Code's trust prompt is reported as "Needs you" on a wide terminal and as Idle after the terminal was narrowed to about 40 columns, while the screen still shows the prompt: the rule may depend on line wrapping | fixed by #259's whitespace-aware phrase matching; #279 brings #261's real Windows 24x20 regression capture onto main. #261 was merged into the feature branch, so its fixtures were absent from main |
| L8 | Codex's folder-trust screen is rendered correctly but reported as Idle: the codex manifest has no rule for it | wide-screen rule already shipped; #279 brings #261's short-screen rule and real Windows 24x20 capture onto main. Both trust captures detect `blocked`; on a short terminal the menu can still be below the visible screen |
| L9 | The macOS process readers (`process_name`, `process_cwd`, `process_tree`, `process_stats`) pass their tests but nothing outside `agora-platform` calls them, so no event or UI column shows them | partly: `process_tree` and `process_stats` now give the running agents' memory in `NodeResources.memory_used_bytes` (sent on count change and when it moved a lot, #334), shown on the machine's row in the window's agent tree; `process_cwd` still unused |
| L10 | Windows: a confined agent's `git commit` fails when the main checkout is under the user profile (git `stat`s every folder above the ref it locks); ADR-0050 work item | fixed by ADR-0050 Amendment B (`Users` in the restricting list covers `C:\Users`) with the node's narrow Windows profile: `winsandbox.rs::a_confined_agent_commits_in_its_worktree_under_the_profile` commits in a worktree under `%TEMP%`, and fails without `refs/` writable |
| L11 | Windows: granting an agent a large `sandbox.read` folder (`~\.rustup`, about 100k files) takes 20 to 30 s per agent start because the permission is propagated to every file | open: documented in the sandbox recipes; consider non-propagating grants |
| L12 | Messages between nodes work in all six directions over the LAN hub (`send_message` to `<node>/<agent>`; inbox `provenance.node`) | verified. The shared board too, on `mac2` (macOS) and `win` (Windows 11) over the LAN hub: one claim winner (`task.not_open` for the other), all-or-nothing leases, `handoff` moves the task and delivers its message (`provenance.node`); with `win` killed, `mac2` keeps the board, a new task gets a new id and a handoff to `win` answers `node.offline`; after the restart `win` sees every task, the one made while it was down included (#266, #268, #271 on the hub) |
| B1 | Shared board (ADR-0039): when one of two nodes drops, the project is no longer "on two online nodes", so the remaining node falls back to its local broker and the hub's tasks vanish from its view; tasks made meanwhile stay local and are never merged | fixed: ADR-0039 amendment A (accepted): the hub keeps answering shared while the board holds a task that is not done or a live lease |
| B2 | Id collision: such a local task gets `t-1`, the same id as a different task on the hub board | fixed with B1 |
| B3 | A handoff to an offline node answers `mesh.recipient_unknown` (it ran locally) instead of `node.offline` | fixed with B1 |
| B4 | A node that crashes and restarts reuses its agent ids (`agent-1`), and the hub board still showed the dead agent's claim as the new agent's | fixed: the hub gives back the tasks and leases of a node's agents when the node announces a new process start |
| B5 | `lease.held` carries an empty `expires_at` | fixed: the error carries the conflicting lease's end in ms, as the file-write error already did |
| B6 | Local → shared: a project used on one node keeps its tasks and leases on that node's local broker; when a second node with a checkout comes online the board takes over, the first node's agents lose sight of their own claimed task, the board's first task is `t-1` again, and a lease held locally is not seen by the board: measured, `nb` was granted `README.md` while `na` still held it (two holders) | open: ADR-0039 says work made locally before sharing stays local; needs that sentence changed (move unfinished local tasks and live leases to the board when it takes over, or refuse to switch while local leases are live) |
| B7 | Two same-identity project folders selected after both daemons start are ignored by shared-board eligibility; the peer sees no task and acquires a path already held on the first node | fixed in #291: checkout/workspace/session membership drives both eligibility and production join discovery, with a real two-node regression and project lifecycle tests |


## Confined agents can reach unix sockets (2026-11-11)

- **L21** Under `sandbox = worktree` (and the first draft of `worktree-offline`) a confined agent can **connect to any unix socket** it can name.
  Measured on Linux (kernel 7.0, Landlock ABI 8): a confined process connected to a socket outside every allowed folder. Landlock only governs
  path-based socket connects from ABI 9 (`LANDLOCK_ACCESS_FS_RESOLVE_UNIX`); the node uses ABI 1 rights. Seatbelt on macOS allows them too unless a rule
  says otherwise. Consequence: a tmux server socket (`tmux -S … new-window 'cmd'`) or the user's `SSH_AUTH_SOCK` gives the agent a command or
  credentials outside the confinement. Needs a decision (maintainer): on macOS close it like `worktree-offline` does (own mesh socket and private temp only;
  `git push` over SSH then needs the owner to list the agent socket's folder); on Linux use `RESOLVE_UNIX` when the kernel has it and say plainly on older
  kernels that sockets are not confined. Until then the threat model lists it as an open gap.

## Windows web window pass (2026-11-11)

- **L14** The `win` node is shown under the PC's host name, not the node id. It puts a personal name in every screenshot.
- **L15** Two boards with the same display name (different `origin` spellings) cannot be told apart in the UI. Implemented in #286: cards retain logical project IDs, expose selectable identities and expandable machine/folder context, and use identity rather than display name as their React key.
- **L16** The audit page cannot tell sessions apart (every row says "owner"); a device column would answer "who".
  Fixed: every row a session makes names it by a short id (the first 6 hex digits of its token's hash, schema 10), the sign-in row
  included, so two windows of one person show different ones; rows from before, and the hub's own, have none.
  Screenshots: `docs/screenshots/audit-session-{light,dark}.png`.
- **L17** Audit rows "Watched a terminal … Sent, no answer yet" stay unresolved after a node restart.
  Fixed: the row stayed `sent` when the client left before the answer (a reload, as after a node restart, or a closed tab); the
  hub dropped the command with the client session. It now waits for each such answer until its deadline and records the node's
  outcome, `node.offline` or `node.timeout` (reproduced on 0.3.5 with Start then reload: `AgentLaunch` stayed `sent`, now `ok`).
  Cleanup visits the earliest deadline first, even when live polling reordered the requests; waiting for a newer command must not
  turn an older command's late reply into a success (#395).
- **L18** "Recent messages" says agents have not written to each other yet after a hub restart (mesh history is not re-read).
- **L19** The web app takes its language only from `navigator.language`; there is no in-app switch, so Turkish needs a `tr-TR` browser. Implemented in #284: sign-in, Home and Focus offer a remembered browser-language override; live terminals and unfinished forms survive switching.
- **L20** A refused or abandoned start leaves its `agent/<name>` branch behind: `Supervisor::abandon` removes the worktree but not the branch.
  Fixed for failures after node-owned worktree creation in #314: terminal, injection, selected-worktree and adapter failures roll back the
  unchanged unused branch only after worktree removal succeeds. Changed, symbolic, checked-out and existing branches stay.
  - Not a bug: "an agent killed from outside stays `running`". The restart policy started it again. What is missing is UX: the restart is
    not shown, so it looks as if nothing happened.
- **L22** A confined agent started in a **selected checkout** (`agent start --project` with no `--worktree` starts in the checkout itself)
  gets the whole checkout writable, `.git` included.
  - It can write `.git/config` and `.git/hooks`. This is already listed as open in T8 and ADR-0052.
  - New here: the consequence does not wait for the user. The node itself runs `git -C <folder> status` unconfined (`agora-node/src/git.rs`)
    whenever changes are shown. A `core.fsmonitor` command set in `.git/config` runs during `status`; I checked this with git and the
    node's environment (`LC_ALL=C`, `GIT_OPTIONAL_LOCKS=0`).
  - The same held for `git diff` drivers (`diff.external`, `textconv`); #300 turns them off for the node's git too.
  - Windows' narrow profile covers only worktrees (`.git` is a file there). On Linux and macOS, the worktree profile makes the shared git
    folder writable (`common_git_dir`).
  - Done:
    - #300 runs the node's own git with fsmonitor, hooks, external diff and textconv off.
    - #303 keeps the shared `config` and `hooks` read-only for macOS worktrees.
    - #307 blanks the filter drivers the repository's own config defines.
    - #310 pins the node's git to the Git folders recorded when the node made the worktree.
  - Still open (T8b in the threat model):
    - On Linux, a worktree agent can still write the shared `config` and `hooks`.
    - A selected checkout's `.git` stays writable to its agent (T8, ADR-0052), so git that the user runs there follows what the agent
      wrote.
- **L23** Windows, real Codex under confinement (`AGORA_WINDOWS_CONFINEMENT=1`). It does not run, for three reasons.
  1. The manifest launches `codex` by bare name, and the node adds the program's folder to `read` only for a path. A per-user install is
     unreadable to the restricted token, so the agent shows as started and stops at once, with no reason shown.
  2. `sandbox.read` on the whole install tree (about 811k files) made the start take 232 s and the stop 256 s, and the restart policy
     repeated the start.
  3. Codex must read and write `%USERPROFILE%\.codex`. That folder is refused with `label_failed`, because Codex's own `.sandbox-bin` folder
     does not give the user WRITE_OWNER.

  Also measured: applying and removing a propagated grant re-derives the inherited entries of that tree from its parent, which drops stale
  third-party entries that are not restored afterwards. Open: maintainer decisions (program folder, Codex's home, exit reason, cost of large
  folders), see the `comms` report of 2026-11-11 19:25 UTC.

## Hub, node and window under outages (Windows, 2026-11-14)

Measured with the real binaries and Chromium (`comms`: `log/2026-11-14-0115-windows-link-resilience-findings.md`). With the hub dead
the node keeps every agent running and catches up 1.5 s after the hub is back (guarded by `crates/agora/tests/hub_dial.rs`
`a_node_keeps_working_without_the_hub_and_catches_up_when_it_appears`; the browser scenario is `scripts/e2e-link-resilience.mjs`).

- **R1** Closed by ADR-0070 (hub-owned presence): the hub writes `NodeLink` up when a node's connection opens (a reconnect
  included, so a node the hub closed as silent is Online again by itself with its agents untouched) and down when it ends or
  is closed after 25 s of silence; the node pings every 10 s, drops a silent hub after 25 s, gives up a hung dial after 25 s
  and redials at once after a suspension; the window pings every 15 s and starts over after 10 s without an answer; offline
  machines show "last seen". Design and constants: `docs/design/observability.md` (Presence, Silence); acceptance:
  `scripts/e2e-link-silent.mjs`; tests: `crates/agora-hub/tests/server.rs` (silent, frozen, replaced, old-node cadence),
  `crates/agora-hub/src/projection.rs`, `crates/agora-node/src/uplink_client.rs`, `apps/web/src/net/live-resilience.test.ts`.
  The original finding: a silent link was not noticed (process frozen or asleep, TCP open): the window shows a frozen node online, and
  nodes and windows do not reconnect to a frozen hub until it runs again (then within about 1 s, as before). A first version
  (node and hub dropping a link silent for 25 s, the window pinging every 15 s) was taken out of 0.3.8 after real runs:
  on `754a7b1` a node frozen 40 s or 3 min was never shown offline (the hub closed the silent link and recorded nothing; an
  earlier version recorded it offline and it then stayed offline after it reconnected), and after the hub was frozen 40 s or
  3 min the open window stayed on "Connecting to your hub…" for 45 to 60 s after the hub resumed (0.5 s on `d669d6a` and main)
  (macOS and Windows probes). Needed: a hub-owned presence event (an ADR), so a hub-closed silent link comes back Online by
  itself. Done in 0.3.8: the window's connect attempt made while the hub was frozen (TCP accepted, no answer, no browser error;
  still not live 60 s after a 40 s freeze on Windows) is given up after 8 s and retried (`OPEN_WITHIN_MS`); an open connection
  has no deadline.
- **R2** The window came back up to 30 s after the hub. Fixed: at most 5 s with jitter, at once on `online`, visible, focus and "Retry now".
- **R3** A `delivery_failed` notice did not say which message failed. Fixed: `reply_to` is the failed message's id. A resend queue after
  reconnect is not done (a behaviour change for an ADR).
- **R4** `agent start` after `project init` printed "The request was not valid (field: directory.identity_changed)". Fixed: the CLI names
  `--accept-project-id-change` (`cli-invalid-directory-identity_changed`); the wire error is unchanged.
