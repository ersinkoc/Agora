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

- **Hub-and-spoke.** Nodes connect to the hub; clients connect to the hub. Clients never connect to nodes
  directly. (ADR-0002)
- **Outbound by default.** Nodes dial the hub (WSS or QUIC with mTLS), so nodes behind NAT/firewalls work
  without opening ports.
- **SSH bootstrap.** For machines where the user cannot or will not install a service, the hub connects
  over SSH, uploads the matching binary if needed, and speaks the node protocol over the SSH channel's stdio.
- **Autonomous nodes.** If the hub is unreachable, nodes keep agents running, keep enforcing leases they
  already granted, and buffer events locally. On reconnect they replay buffered events in order.
- **Single binary.** `agora hub`, `agora node`, `agora tui`, and CLI subcommands are one executable per
  platform. The web client's static assets are embedded in the hub.
- **Hub deployment modes.** Local (tiny, SQLite, single user), server (team, Postgres), cloud (multi-tenant,
  later). The domain model always has an owner/team so modes do not diverge.

## 2. Three planes

### 2.1 Runtime plane (node)

Owns everything that touches the machine:

- **Terminals:** PTY (Unix) / ConPTY (Windows) via `agora-platform`; one I/O actor per terminal.
- **Terminal emulation:** `agora-vt` exposes a trait; the backend is replaceable (see open questions).
- **Process inspection:** foreground process, process tree, cwd.
- **Files:** lazy directory listing, watch, read, write (writes go through lease checks).
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
Delivery to an agent that does not poll MCP falls back to the integration level's input path (ACP
prompt, or PTY input as a last resort) and is marked as such in the audit log.

## 3. Domain model (agora-core)

```
Owner/Team ─┬─ Node ── Checkout ── Worktree
            └─ Project ─┬─ Agent (kind, node, worktree, level, state, session)
                        ├─ Terminal (node, purpose: agent | shell | service)
                        ├─ Channel / Message
                        ├─ Task
                        └─ Lease
```

- **Project identity** is machine-independent: normalized remote URL + root commit hash. A project with no
  remote uses root commit only. (ADR-0003)
- **Isolation:** each agent gets its own worktree and branch by default. Sharing a worktree is opt-in and
  requires leases.
- **Ownership:** nodes are authoritative for runtime facts (process alive, terminal output, file contents,
  git state). The hub is authoritative for coordination facts (tasks, leases granted, approvals, messages).
- **Event sourcing at the hub:** hub state is a projection of events from nodes and clients. This gives the
  audit log, reconnect replay, and multi-client consistency from one mechanism.

## 4. Protocol

Schema-first, defined in `schema/`, generated into Rust, TypeScript, Go, and Python. (ADR-0006)

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

Clients choose a format at handshake:

- **Raw VT + periodic snapshot:** node forwards bytes; the client runs its own emulator. Default for web/desktop.
- **Rendered cell deltas:** node renders and sends diffs. Default for TUI and low-bandwidth links.

Terminal streams are relayed by the hub without decoding or storing them, or carried on a direct client ↔ node
path when available (ADR-0023). Remote terminals support predictive local echo. See `design/performance.md`.

## 5. Clients

- **Web (primary)**, embedded in the hub binary; **desktop** is the same app in Tauri. (ADR-0005)
- **TUI (secondary)** for SSH-only access; supports projects, agents, terminals, approvals; file explorer
  and diff are simplified.
- **CLI** for scripting; every CLI command maps 1:1 to a protocol command.

Main window layout: see `ux/main-window.md`.

## 6. Security

- Device enrollment: each node is approved once and receives a client certificate (mTLS).
- Per-node and per-project policies (for example "agents on this node need approval for shell commands").
- Approvals queue: permission requests from L2/L3 agents (and detected blocked prompts from L1) appear
  centrally and can be answered from any client, including mobile.
- Secrets stay on nodes. The hub stores references only.
- Append-only audit log: who/what/where/approved-by for every command, write, approval, and message.
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
| Windows | Phase 7 (ADR-0045, started) | ConPTY terminals with a job object per terminal, a named-pipe control channel (owner-only), toolhelp for process names, children and stats; no working directory of another process (the PEB is undocumented). Tested on CI with test agents; not yet with a real agent on a real machine |

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
