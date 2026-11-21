# What this commit supports

One table, four separate questions per capability (architecture review R7): **was it decided** (the ADR and its status), **is it
built on `main`**, **what checks it automatically**, and **where it was seen on a real machine** (a dated note in the repository). An
accepted ADR is a decision, not a feature: a row says "not built" until the code is on `main`. Evidence on a real machine is cited only
from notes in this repository; "none recorded" means none is written down here, not that it fails.

`scripts/check-docs.sh` checks that every `ADR-NNNN Accepted` / `ADR-NNNN Proposed` below says what the ADR file says. When you change
what a row claims, change the row in the same pull request (AGENTS.md: docs move with code).

L1 is an agent in a terminal read from its screen (a manifest); L2/L3 are structured agents (ACP, adapters) that report their own state
(ADR-0004). The two are different levels of support and are listed apart.

## Machines and the hub

| Capability | Decision | Built on `main` | Automated evidence | Real machines |
| --- | --- | --- | --- | --- |
| Node on Linux and macOS: terminals, agents, git, files | ADR-0002 Accepted | yes (phase 2) | CI on ubuntu-latest and macos-latest | [LAN run 2026-11-11](backlog/lan-test-findings.md): Linux and macOS nodes |
| Node on Windows: ConPTY, job objects, named pipe | ADR-0045 Accepted | yes (W1–W5, [phase 7](backlog/phase-7.md)) | CI on windows-latest | LAN run 2026-11-11: Windows 11 node with real Claude Code and Codex |
| Hub with the web window, many machines | ADR-0005 Accepted, ADR-0029 Accepted | yes (phase 3) | hub and server tests; `scripts/e2e-*.mjs` in headless Chromium | LAN run 2026-11-11: a Linux hub |
| A machine is online while its link is (silence noticed in 25 s) | ADR-0070 Accepted | yes | hub tests; `scripts/e2e-link-silent.mjs` | none recorded here |
| Old machines shown apart; forget one, clean up old ones | ADR-0073 Accepted | hub, CLI, web and terminal client yes | hub, server and CLI tests; web tests | none recorded here |
| `agora` on a fresh machine starts a hub and a node here | ADR-0062 Accepted | rung 1 yes; restart after a crash yes; agents come back after a restart yes (asked once, 8 at a time, 16 at most, the node policy wins); start at login yes on Windows (logon task), on Linux with systemd (user unit) and on macOS (LaunchAgent); joining a hub by its address pairs (ADR-0065); finding hubs on the network is `agora join` on its own, over IPv4 broadcast only (on an IPv6-only network use `agora join ADDRESS`) | front-door tests (`crates/agora/tests/front_door.rs`), login service against a fake `systemctl` (`crates/agora/tests/service_systemd.rs`) | none recorded here |
| Pairing: find a hub on the network, approve on the hub | ADR-0065 Accepted | yes (#664, #700, #705, #709, step 4 #710: `agora pair`, `agora join`, the request in the window and the terminal client) | hub, CLI, web and terminal-client tests | none recorded here |
| Machines reachable only over SSH | ADR-0032 Accepted, ADR-0034 Accepted | yes | tests against a stand-in `ssh` | [real SSH run 2026-11-09](reviews/2026-11-09-real-ssh-run.md): a private loopback `sshd`; no other OS as target |
| Direct client ↔ node path for terminals | ADR-0023 Accepted | not built (terminal streams go through the hub relay) | — | — |
| `agora update` (self-update) | ADR-0019 Accepted | yes (replaces the program; not a live handoff) | update tests | none recorded here |

## Agents

| Capability | Decision | Built on `main` | Automated evidence | Real machines |
| --- | --- | --- | --- | --- |
| L1: terminal agents from manifests (built-in: claude-code, codex, wrongstack) | ADR-0004 Accepted | yes | manifest fixture screens; adapter tests | [phase 4](backlog/phase-4.md) P4-05: Claude Code and Codex manifests done, WrongStack open; LAN run 2026-11-11 |
| L2: ACP agents (built-in: opencode-acp, wrongstack-acp, antigravity-acp) | ADR-0004 Accepted | yes | tests against the fake ACP agent | none recorded here |
| L3: adapter processes, installed from the signed registry | ADR-0004 Accepted, ADR-0044 Accepted | yes | adapter host tests and the conformance transcript | none recorded here |
| Agents run in the selected folder; worktrees optional | ADR-0052 Accepted | yes | node and web tests | none recorded here |
| Launch options, including "do not ask" modes | ADR-0059 Accepted | yes | node and web tests | none recorded here |
| Start with model, effort and mode | ADR-0066 Accepted | node and hub yes (a start with settings is signed; the approval card lists them); the start window's picker yes (`LaunchSettings.tsx`); the terminal client's start picker not built | node, hub and web tests | none recorded here |
| Agent login and accounts on a node ([design](design/agent-accounts.md)) | no ADR (design only) | not built (the manifest table is parsed; nothing runs a login) | — | — |
| Install and update coding agents on a machine from the window | ADR-0087 Accepted | partly: rungs 1-2 (the agent checks in `agora doctor` and the signed agent-package list, #868) and rung 3 (`agora agent install`, binary download with a typed local approval, #871); install from the window not built | adapter and CLI tests | none recorded here |
| A missing agent kind says why | ADR-0069 Accepted | yes (protocol, web start window, terminal client) | protocol fixtures; web and terminal-client tests | none recorded here |
| Resume after the node restarts (session ledger) | ADR-0060 Accepted | yes | node tests | none recorded here |
| An agent starts helpers, with the user's signed approval | ADR-0057 Accepted | yes | node, hub and web tests | none recorded here |
| Workspace clean-up and disk limits | ADR-0064 Accepted | not built | — | — |
| Agent confinement (sandbox) | ADR-0047 Accepted, ADR-0050 Accepted | Linux and macOS yes; Windows built but off by default; one level per node (the per-project and per-kind choice of ADR-0047 decision 1 is not built); `worktree-offline` on macOS only | sandbox tests on each OS | [macOS 2026-11-14](reviews/2026-11-14-macos-confinement-and-terminal-client.md): every way out refused; Claude Code and Codex run confined; [Windows 2026-11-21](reviews/2026-11-21-windows-confinement-m23.md): the shell kind, file boundaries, git and loopback previews work, but Node.js cannot start a child process (#895), the node's own `agora.exe` cannot run (#896) and adapter/ACP agents cannot be confined (#897) |

## People, approvals and coordination

| Capability | Decision | Built on `main` | Automated evidence | Real machines |
| --- | --- | --- | --- | --- |
| Approvals signed by a device the node trusts | ADR-0020 Accepted, ADR-0028 Accepted | yes | node approval tests | none recorded here |
| Trusting your devices in one step | ADR-0061 Accepted | yes | hub, node, web and terminal-client tests | none recorded here |
| Team mode: people, roles, audit | ADR-0049 Accepted | partly ([phase 7](backlog/phase-7.md) P7-09 in progress) | hub tests | none recorded here |
| Agents message each other across machines (mesh) | ADR-0008 Accepted, ADR-0031 Accepted | yes; a reply's link across machines not built | mesh tests | LAN run 2026-11-11: cross-node messages |
| A reply keeps its link across machines | ADR-0074 Accepted | not built | — | — |
| Shared project board (tasks, leases) | ADR-0039 Accepted, ADR-0040 Accepted | yes; a change is answered only once stored; a lease is renewed only when its agent takes it again (the node's own renewal of ADR-0039 is not built) | board and hub tests, fault tests | LAN run 2026-11-11: shared-board behavior |

## Updates, secrets and confinement (ADR-0075 to ADR-0084)

| Capability | Decision | Built on `main` | Automated evidence | Real machines |
| --- | --- | --- | --- | --- |
| The node updates in place without stopping agents | ADR-0075 Accepted | partly: step 1 only (the mesh bridge reconnects, #639); the later steps are not built | mesh bridge tests | none recorded here |
| A read-only metrics token | ADR-0076 Accepted | yes (#637) | hub tests | none recorded here |
| The Windows binary is code-signed | ADR-0077 Accepted | partly: the release workflow signs through SignPath once it is set up (#652); no signed release recorded here | none (the workflow is not run without the account) | none recorded here |
| A confined agent gets a git configuration the node wrote | ADR-0078 Accepted | yes (#638) | node tests | none recorded here |
| A terminal holder keeps agents alive when the node stops | ADR-0079 Accepted | Linux and macOS: yes for an agent actually started through the holder (Unix core #760, #761; the service unit keeps it #774). The node runs an agent itself, and it ends with the node, when it is confined, when no holder can be used (a holder that would not admit this node, a variable that is not UTF-8), or when its start fell back for another reason ([holder-link](specs/holder-link.md)); Windows: primitives only (#697), no running holder | platform, node and `crates/agora/tests/hold*.rs` tests | `blackview`, 2026-11-20 (ADR-0093 "Origin"): a 0.3.26 node restart kept a Claude Code terminal agent in the same holder (the structured agents of that run are ADR-0093's case); [Windows 2026-11-21](reviews/2026-11-21-windows-adr0079-remeasure.md): the ADR's Windows premises (a pseudoconsole outliving its reader, cancelling a pending read, breakaway under Task Scheduler) measured again and unchanged |
| The file browser does not show secret files | ADR-0080 Accepted | yes (node and review #636; web and terminal client #665) | node, web and terminal-client tests | none recorded here |
| A replayed revocation does not undo the owner's re-trust | ADR-0081 Accepted | yes (#673) | node tests | none recorded here |
| A confined agent cannot use the node's control socket | ADR-0082 Accepted | yes (#674; what it does not close: #678, threat model) | node tests | none recorded here |
| A confined agent reaches only the unix sockets it is given | ADR-0083 Accepted | partly: socket-bearing variables are removed by default at a confined terminal start on every OS (the owner can keep one with `sandbox.pass_env`); macOS: an allow-list of sockets (#702); Linux: abstract sockets scoped from Landlock ABI 6, pathname sockets not restricted; Windows: not established | node and platform tests; a macOS-only test runs every profile through `sandbox-exec` | macOS 27.2.1, 2026-11-17 (ADR-0083 "Built"): own socket connects, another refused, resolver and a confined `git commit` work |
| A confined agent gets its login from a secret the node holds | ADR-0084 Accepted | yes (#699); a policy name the manifest does not declare is not passed and is not refused at load (ADR-0084 "Built" note); `agora doctor` names it only for a kind whose manifest declares some `credential_env` | node tests | none recorded here |

## Since ADR-0085

| Capability | Decision | Built on `main` | Automated evidence | Real machines |
| --- | --- | --- | --- | --- |
| Machine and hub certificates renew themselves | ADR-0085 Accepted | yes (hub side #780, machine side #783); Amendment A (a machine whose clock is behind) not built | hub, node and CLI tests | none recorded here |
| Any machine can be the hub, with takeover | ADR-0086 Accepted | not built | — | — |
| Preview an agent's local web server | ADR-0088 Accepted | schema only (#768) | protocol fixtures | — |
| A structured agent's chat history survives the node, read in pages | ADR-0089 Accepted | not built | — | — |
| A structured agent's conversation is complete (thinking, tools, plan, effort) | ADR-0090 Accepted | yes: node and adapters, hub step 5 (#781), web; skip-permissions switch through a signed approval (Amendment); the terminal client's view not built; the adapter's current mode is not checked against `skip_permissions` at start (backlog P2-16) | node, adapter, hub and web tests | none recorded here |
| Plan limits of subscription agents | ADR-0091 Accepted | Codex yes (#787, #788); other tools not built | node, hub and web tests | none recorded here |
| The ACP registry as a source of agent kinds | ADR-0092 Accepted | step 1 (conversion, #832) | adapter tests | none recorded here |
| Structured agents survive a node restart | ADR-0093 Accepted | Linux and macOS, unconfined agents started through a holder of link version 2: the process survives (steps 1-2, #800); a version-1 holder cannot keep it; a confined adapter is the node's own, ends with it, and comes back only as the resume policy allows (it may stay interrupted for a manual resume); Windows not built | node and platform tests | none recorded here |
| A plain terminal shows the agent program running in it | ADR-0094 Accepted | not built | — | — |
| A project's kanban board; a task dispatched to an agent | ADR-0095 Accepted | read-only board (#858), the broker's claim (#859), the schema (`TaskDispatch`, `TaskDispatched`, `task.dispatch`), the node handler (#883: claim, data-wrapped prompt never over a running turn, audit event) and "Give to an agent" in the web window (a Backlog card and the command palette; the project's running agents, helpers under their parent; a confirmation naming agent, project and machine; the window learns that a machine lacks `task.dispatch` from the refusal, since it cannot read capabilities) and "Start a new agent for this task" (step 3: the ordinary start window for the task's project, then the same confirmation for the new agent); the terminal client's board and "Give to an agent" (`b`, then Enter or `g` on a Backlog task; no "start a new agent" there), and a person cannot start a helper under a parent (helpers stay agent-made, ADR-0057) | web, mesh, node and terminal-client tests; browser e2e with `agora-fake-agent` (`scripts/e2e-board-give.mjs`, `scripts/e2e-board-start.mjs`) | [Windows 11 2026-11-21](reviews/2026-11-21-windows-dispatch-conpty.md): a probe terminal agent through ConPTY; the bracketed paste arrived whole, nothing typed over a working agent; real agents and the refusal path not tested |
| A ready helper agent | ADR-0096 Accepted | not built | — | — |

## The window

| Capability | Decision | Built on `main` | Automated evidence | Real machines |
| --- | --- | --- | --- | --- |
| Review screen and file browser | ADR-0036 Accepted | yes | web tests; `scripts/e2e-changes*.mjs` | none recorded here |
| A diff names every changed file even when cut | ADR-0071 Accepted | not built (the window says how many files are not shown) | — | — |
| A shared terminal takes the smallest window's size | ADR-0072 Accepted | yes | hub and web tests | none recorded here |
| Phone-sized window | ADR-0046 Accepted, ADR-0067 Accepted | yes | `scripts/e2e-phone.mjs` in emulated Chromium | not run on a real phone |
| Screen password for remote sign-in | ADR-0067 Accepted | yes (rung 1) | hub tests | none recorded here |
| Tailscale (rung 3): `agora remote tailscale\|status\|off`, public-address rule, doctor checks | ADR-0067 Accepted | yes (rung 3) | `crates/agora/tests/remote_cli.rs` (fake `tailscale`), hub unit tests | never run against a real tailnet or a real VPS |
| Trusted reverse proxy or tunnel (rung 4): `agora remote proxy`, Host/Origin checks, real client address | ADR-0067 Accepted | yes (rung 4), without a window banner and without starting the connector | `crates/agora/tests/remote_cli.rs` (a real hub process, fake proxy headers), `crates/agora-hub/tests/server.rs`, hub unit tests | never run against a real tunnel |
| Layouts and themes | ADR-0068 Accepted | layouts and themes yes; panel registry, keybindings and the terminal client not built | web tests; visual baselines | none recorded here |
| Terminal client (TUI) | ADR-0043 Accepted, ADR-0063 Accepted | yes | terminal-client tests | [macOS 2026-11-14](reviews/2026-11-14-macos-confinement-and-terminal-client.md): list, tabs, readers, a real Claude Code agent |
| Desktop app | ADR-0048 Accepted | partly ([phase 7](backlog/phase-7.md) P7-06 in progress) | builds on three systems | none recorded here |

Performance figures, measured and not, are in [design/performance.md](design/performance.md).
