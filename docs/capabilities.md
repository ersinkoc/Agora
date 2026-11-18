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
| `agora` on a fresh machine starts a hub and a node here | ADR-0062 Accepted | rung 1 yes; restart after a crash yes; agents come back after a restart yes (asked once, 8 at a time, 16 at most, the node policy wins); start at login yes on Windows (logon task), on Linux with systemd (user unit) and on macOS (LaunchAgent); join and pairing not built | front-door tests (`crates/agora/tests/front_door.rs`), login service against a fake `systemctl` (`crates/agora/tests/service_systemd.rs`) | none recorded here |
| Pairing: find a hub on the network, approve on the hub | ADR-0065 Accepted | not built | — | — |
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
| Install and update coding agents on a machine from the window | ADR-0087 Proposed | not built, ADR proposed | — | — |
| A missing agent kind says why | ADR-0069 Accepted | yes (protocol, web start window, terminal client) | protocol fixtures; web and terminal-client tests | none recorded here |
| Resume after the node restarts (session ledger) | ADR-0060 Accepted | yes | node tests | none recorded here |
| An agent starts helpers, with the user's signed approval | ADR-0057 Accepted | yes | node, hub and web tests | none recorded here |
| Workspace clean-up and disk limits | ADR-0064 Accepted | not built | — | — |
| Agent confinement (sandbox) | ADR-0047 Accepted, ADR-0050 Accepted | Linux and macOS yes; Windows built but off by default | sandbox tests on each OS | [macOS 2026-11-14](reviews/2026-11-14-macos-confinement-and-terminal-client.md): every way out refused; Claude Code and Codex run confined |

## People, approvals and coordination

| Capability | Decision | Built on `main` | Automated evidence | Real machines |
| --- | --- | --- | --- | --- |
| Approvals signed by a device the node trusts | ADR-0020 Accepted, ADR-0028 Accepted | yes | node approval tests | none recorded here |
| Trusting your devices in one step | ADR-0061 Accepted | yes | hub, node, web and terminal-client tests | none recorded here |
| Team mode: people, roles, audit | ADR-0049 Accepted | partly ([phase 7](backlog/phase-7.md) P7-09 in progress) | hub tests | none recorded here |
| Agents message each other across machines (mesh) | ADR-0008 Accepted, ADR-0031 Accepted | yes; a reply's link across machines not built | mesh tests | LAN run 2026-11-11: cross-node messages |
| A reply keeps its link across machines | ADR-0074 Accepted | not built | — | — |
| Shared project board (tasks, leases) | ADR-0039 Accepted, ADR-0040 Accepted | yes; a change is answered only once stored | board and hub tests, fault tests | LAN run 2026-11-11: shared-board behavior |

## Updates, secrets and confinement (ADR-0075 to ADR-0084)

| Capability | Decision | Built on `main` | Automated evidence | Real machines |
| --- | --- | --- | --- | --- |
| The node updates in place without stopping agents | ADR-0075 Accepted | partly: step 1 only (the mesh bridge reconnects, #639); the later steps are not built | mesh bridge tests | none recorded here |
| A read-only metrics token | ADR-0076 Accepted | yes (#637) | hub tests | none recorded here |
| The Windows binary is code-signed | ADR-0077 Accepted | partly: the release workflow signs through SignPath once it is set up (#652); no signed release recorded here | none (the workflow is not run without the account) | none recorded here |
| A confined agent gets a git configuration the node wrote | ADR-0078 Accepted | yes (#638) | node tests | none recorded here |
| A terminal holder keeps agents alive when the node stops | ADR-0079 Accepted | not built | — | — |
| The file browser does not show secret files | ADR-0080 Accepted | yes (node and review #636; web and terminal client #665) | node, web and terminal-client tests | none recorded here |
| A replayed revocation does not undo the owner's re-trust | ADR-0081 Accepted | yes (#673) | node tests | none recorded here |
| A confined agent cannot use the node's control socket | ADR-0082 Accepted | not built on `main` (in review: #674) | — | — |
| A confined agent reaches only the unix sockets it is given | ADR-0083 Accepted | not built | — | — |
| A confined agent gets its login from a secret the node holds | ADR-0084 Accepted | not built | — | — |

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
