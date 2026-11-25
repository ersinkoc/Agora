# Architecture

Status: living document. Decisions referenced here are recorded in `adr/`.

## 1. Topology

```
   Clients (one window)            HUB                              NODES (every machine)
 ┌──────────────────────┐   ┌──────────────────────────┐   ┌───────────────────────────────┐
 │ Web · Desktop (Tauri)│──▶│ identity & device enroll │◀──│ laptop  (macOS)               │
 │ TUI · CLI · API      │   │ event store (state)      │◀──│ vps-1   (Linux, outbound WSS) │
 └──────────────────────┘   │ router · approvals queue │◀──│ office  (Windows)             │
                            │ mesh broker · registry   │◀──│ ssh-only box (SSH bootstrap)  │
                            │ audit log · web assets   │   └───────────────────────────────┘
                            └──────────────────────────┘
```

- **Hub-and-spoke.** Nodes connect to the hub; clients connect to the hub. Clients do not connect to nodes
  directly today (ADR-0002); a direct client ↔ node path for terminals is accepted but not built (ADR-0023). One hub is
  authoritative; a standby that can take over is accepted but not built (ADR-0086).
- **Outbound by default.** Nodes dial the hub (WSS or QUIC with mTLS), so nodes behind NAT/firewalls work
  without opening ports.
- **SSH bootstrap.** For machines where the user cannot or will not install a service, the hub connects
  over SSH: the owner installs the node there once (`agora hub ssh install NAME`, ADR-0034, or by hand), and the node dials
  back through a reverse tunnel the hub holds (`ssh -N -R`, ADR-0032; the tunnel installs nothing). `agora node stdio` remains for
  tools that want the control protocol over SSH (ADR-0011).
- **Autonomous nodes.** If the hub is unreachable, nodes keep agents running, keep enforcing leases they
  already granted, and buffer events locally. On reconnect they replay buffered events in order.
- **Single binary.** `agora hub`, `agora node`, `agora tui`, and CLI subcommands are one executable per
  platform. The web client's static assets are embedded in the hub.
- **Hub deployment modes.** Local (tiny, SQLite, single user), server (team; SQLite until it is not enough, then Postgres, ADR-0049), cloud (multi-tenant,
  later). The domain model always has an owner/team so modes do not diverge.

## 2. Three planes

### 2.1 Runtime plane (node)

Owns everything that touches the machine:

- **Terminals:** PTY (Unix) / ConPTY (Windows) via `agora-platform`; one I/O actor per terminal.
- **Terminal emulation:** `agora-vt` exposes a trait; the backend is replaceable (see open questions).
- **Process inspection:** foreground process, process tree, cwd.
- **Files:** lazy directory listing and read through the protocol. The node's file module has watch and lease-checked
  write helpers that are not yet wired to any command or tool.
- **Git:** checkouts, worktree create/remove, status, diff, branch info.
- **Agent supervision:** start, stop, resume, restart policy.

### 2.2 Agent plane (adapters)

Every agent instance runs through exactly one integration level. (ADR-0004)

#### Integration levels

| Level | Mechanism | State source | What the window shows |
| --- | --- | --- | --- |
| **L0 Generic** | Any CLI in a PTY | None (terminal only) | Native terminal |
| **L1 Manifest** | TOML manifest: process names, screen rules, hook install, resume command, MCP config path | Hooks if installed, else screen rules | Native terminal + state |
| **L2 ACP** | Agent Client Protocol over stdio | Reported by agent | Structured chat, plan, tool calls, diffs, approvals |
| **L3 Adapter** | External process in any language speaking the adapter protocol | Reported by adapter | Whatever capabilities the adapter declares |

Rules:

- Prefer the highest level available for an agent kind. The user can override per agent (for example run
  Claude Code in PTY mode to see its native UI).
- State arbitration: reported state (L2/L3/hooks) beats screen-derived state. Screen rules are a fallback.
- Screen rules read only the live bottom of the buffer, never the scrolled viewport.
- Adapters declare capabilities in their handshake (`chat`, `terminal`, `diff`, `approvals`, `resume`,
  `mcp-injection`, ...). Clients enable UI only for declared capabilities.
- Manifests and adapters are distributed through the **adapter registry** (signed packages), so supporting
  a new agent never requires a new binary release.

### 2.3 Mesh plane (coordination)

The hub runs the mesh broker. Each node exposes an MCP server to its local agents and injects it into the
agent's MCP configuration at launch (any agent with MCP support, independent of integration level). (ADR-0008)

Initial tools exposed to agents:

| Tool | Purpose |
| --- | --- |
| `mesh.send(to, body)` | Fire-and-forget message to an agent or channel |
| `mesh.ask(to, question, timeout)` | Message and wait for a reply |
| `mesh.inbox()` | Read pending messages |
| `task.list / claim / complete` | Shared task board per project |
| `lease.acquire(path, ttl) / release` | Exclusive claim on a file or path |
| `handoff(to, summary, refs)` | Transfer a task with context to another agent |
| `agents.list(project)` | Discover peers, their kind, node, and state |

Messages are routed by the hub; agents on different machines communicate transparently.
Agents read messages from the mesh inbox through MCP. The input-path fallback in ADR-0008 is still open.
The approved first prompt for a helper has its own delivery rules (ADR-0057).

## 3. Domain model (agora-core)

```
Owner/Team ─┬─ Node ── Checkout ── Worktree
            └─ Project ─┬─ Agent (kind, node, workspace, level, state, session)
                        ├─ Terminal (node, purpose: agent | shell | service)
                        ├─ Channel / Message
                        ├─ Task
                        └─ Lease
```

- **Project identity:** optional `.agent-project.json` at the Git root (or selected non-Git folder) contains a random
  UUID `project_id`. Only an explicit owner action creates it; committing/copying it groups clones across nodes.
  Otherwise use normalized remote URL + root commit (no remote: root commit alone), so two clones of one repository share
  a shared project (and its board) only when they have the same `origin`: a clone from a local path and one from GitHub are two
  projects (`agora project init` gives them one explicit id). The remote is normalized to `host/path` (host lowercased); on
  `github.com`, `gitlab.com`, `bitbucket.org` and `codeberg.org`, which ignore the case of owner and repository names, the
  path is lowercased too, so clones of `Owner/Repo` and `owner/repo` are one project; every other host keeps the path's case.
  The hub folds an older node's ids for those hosts as it reads its events and board calls, and sends a command about the
  project to that node in the node's own spelling; a node treats a confirmed binding that differs only by this fold as the
  same project (ADR-0115). A standalone folder has a
  node-local identity, without a fabricated Git commit. Metadata never grants access. Changed confirmed bindings
  require explicit owner confirmation. (ADR-0003, ADR-0052)
  The hub resolves board membership from both discovered checkouts and registered selected workspaces, by project identity
  and online node ID; selecting a directory after the daemon starts participates in the same shared board.
  Running agents retain membership in their reported project until they exit, even if the owner confirms a new identity for
  their folder. A metadata change does not strand their existing tasks or leases on a different broker.
- **Workspace:** an owner-selected canonical directory, possibly a subdirectory or non-Git folder. Several agents
  may share one workspace. New `directory.inspect` / `agent.launch` commands use this directory by default;
  explicit worktree mode creates an isolated branch and preserves the selected subdirectory. Legacy `agent.start`
  keeps creating a worktree. Session resume preserves its cwd. The node never cleans or removes user-owned folders.
  After creating a worktree for a pending start, the node retains its branch and initial commit as a rollback receipt.
  Failed injection or process/adapter launch removes that worktree and only its unchanged, unused direct branch;
  failed removal preserves the branch. Ordinary worktree removal and successful sessions preserve their branches.
- **Integration configuration:** directory-mode L1 agents receive separate per-session MCP configuration below the
  node's worktree storage, via existing manifest CLI/environment injection. User files are preserved. Manifests
  whose MCP support requires a fixed in-project config file need explicit worktree mode. L2/L3 hosted sessions
  receive the selected cwd through the existing adapter interface.
- **Shared changes:** a workspace has one diff/status, not a diff attributed to each agent. Leases govern writes
  mediated by the node; they cannot intercept an agent's own shell writes. Project identity groups sessions across
  machines but transfers no files.
- **Ownership:** nodes are authoritative for runtime facts (process alive, terminal output, file contents,
  git state). The hub is authoritative for coordination facts (tasks, leases granted, approvals, messages).
- **Event sourcing at the hub:** hub state is a projection of events from nodes and clients. This gives the
  audit log, reconnect replay, and multi-client consistency from one mechanism.

## 4. Protocol

`schema/` is the source of truth (ADR-0006). The build generates Rust and TypeScript bindings.
Go and Python generation remain open. The L3 Python adapter example uses JSON-RPC over stdio.

The node-local control socket carries protobuf commands in the same binary frames used for the network links.
See [the local socket specification](specs/protocol.md#node-local-control-socket).

Three protocol surfaces, one schema:

| Surface | Between | Content |
| --- | --- | --- |
| Node protocol | node ↔ hub | events, commands, terminal streams, file/git RPC |
| Client protocol | client ↔ hub | snapshots, subscriptions, commands, terminal streams |
| Adapter protocol | node ↔ L3 adapter | lifecycle, state, approvals, messages |

Compatibility rules:

- Handshake exchanges protocol generation and capability lists.
- A published message is never changed. New behavior = new message or new capability.
- New fields are optional; enums carry an `Unknown` fallback.
- Errors are `{ code, params }`. Never human text. (ADR-0007)

### Terminal streaming

Built: the node forwards the program's raw VT bytes and every client (web, desktop, terminal client) runs its own
emulator. When a client subscribes, the node first sends a repaint of the screen with the program's modes, and, when
the client asks, the history before it (ADR-0100).

Terminal streams are relayed by the hub without decoding or storing them. Not built: rendered cell deltas (`terminal.cells`
is named in the schema, and the node refuses it as `protocol.capability_missing`), the direct client ↔ node path
(ADR-0023), and predictive local echo (a design intent in `design/performance.md`).

## 5. Clients

- **Web (primary)**, embedded in the hub binary; **desktop** is the same app in Tauri. (ADR-0005)
- **TUI (secondary)** for SSH-only access; supports projects, agents, terminals, approvals; file explorer
  and diff are simplified.
- **CLI** for scripting. It sends protocol commands and runs local workflows such as setup, service management and updates.

Main window layout: see `ux/main-window.md`.

## 6. Security

- Device enrollment: each node is approved once and receives a client certificate (mTLS).
- Per-node and per-project policies (for example "agents on this node need approval for shell commands").
- Approvals queue: permission requests from L2/L3 agents (and detected blocked prompts from L1) appear
  centrally and can be answered from any client, including mobile.
- Secrets stay on nodes. The hub stores references only.
- Team audit records the acting user and session, action, target machine and outcome for routed commands. Sign-in
  rows use `node` for the sign-in method and client origin. Successful sign-ins record the user and session.
  Failed attempts such as an incorrect code use `-` for the user and leave the session empty.
  Team changes use `owner` and an empty session. For `user.add` and `user.role`, `node` holds the affected
  user ID and `outcome` holds the role. Removal records the affected user ID, and project grant and revoke
  rows record the user and project scope in `node`.
  Routed commands also record the project and agent they act on when the command or the hub's view names them, the
  requested signing device of a signed command, and the device a node verified (`docs/specs/protocol.md`, team audit).
  The event log carries coordination events separately.
- Optional end-to-end encryption of terminal streams between node and client (required for cloud mode).

## 7. Internationalization

- Project Fluent (`.ftl`) for all user-facing text; English is the source locale. (ADR-0007)
- APIs return codes; clients localize. A hub in one locale serves clients in any locale.
- Store UTC and raw values; format in the client's locale.
- Terminal rendering: correct grapheme and East Asian width handling; IME cursor anchoring for CJK input.
- RTL: full support in web/desktop; limited in TUI by nature of terminals (documented limitation).

## 8. Platforms

| Platform | Node | Notes |
| --- | --- | --- |
| Linux | Phase 2 | PTY, /proc for process info |
| macOS | Phase 2 | PTY, libproc for process info |
| Windows | Phase 7 (ADR-0045, started) | ConPTY terminals with a job object per terminal, a named-pipe control channel (owner-only), toolhelp for process names, children and stats; no working directory of another process (the PEB is undocumented). Tested on CI with test agents, and run with real Claude Code and Codex on a Windows 11 node in the [LAN run of 2026-11-11](backlog/lan-test-findings.md) |

## 9. Persistence

| Where | What |
| --- | --- |
| Node | local event buffer, terminal scrollback (opt-in persistence), agent session refs, enrolled cert |
| Hub | event store, projections, users/devices, policies, registry cache |

Restore paths, strongest first: live process (never stopped) → native agent resume via session ref →
layout-only restore with fresh shells in saved directories.

## 10. Cross-cutting concerns

| Topic | Document |
| --- | --- |
| Protocol details | `specs/protocol.md`, `specs/error-codes.md` |
| Agent integration formats | `specs/agent-manifest.md`, `specs/adapter-protocol.md` |
| Mesh tool contracts | `specs/mesh-tools.md` |
| Security and privacy | `security/threat-model.md`, `security/privacy.md`, ADR-0020 |
| Updates and version skew | ADR-0019 |
| Agent accounts on nodes | `design/agent-accounts.md` |
| Usage, cost, and quotas (models.dev, quota profiles) | `design/usage-and-cost.md`, ADR-0021 |
| Notifications | `design/notifications.md` |
| Observability and `agora doctor` | `design/observability.md` |
| Backup and restore | `design/backup-restore.md` |
| Resource limits and cleanup | `design/resource-limits.md` |
| Testing strategy and fake agent | `design/testing.md` |
| Performance budgets and tactics | `design/performance.md`, ADR-0022, ADR-0023 |
| Screens, experience, visual language | `ux/`, `ux/experience.md`, `ux/visual-language.md`, ADR-0024 |
