# Architecture Decision Records

One file per decision: `NNNN-short-title.md`. Statuses: `Proposed`, `Accepted`, `Superseded by NNNN`.
Accepted ADRs are never edited except to mark them superseded. To change a decision, write a new ADR.
`Proposed` ADRs are recommendations awaiting the maintainer's decision; do not build on them as final.

Template:

```
# NNNN. Title
Status: Proposed | Accepted | Superseded by NNNN
Date: YYYY-MM-DD

## Context
## Decision
## Consequences
```

| ADR | Title | Status |
| --- | --- | --- |
| 0001 | Rust for node, hub, and TUI | Accepted |
| 0002 | Hub-and-spoke topology with autonomous nodes | Accepted |
| 0003 | Project as the top-level entity; worktree per agent | Accepted |
| 0004 | Integration levels L0–L3; prefer structured state | Accepted |
| 0005 | Web-first client, Tauri desktop, TUI secondary | Accepted |
| 0006 | Schema-first, append-only protocol | Accepted |
| 0007 | Localization with Fluent; codes not text in APIs | Accepted |
| 0008 | Mesh via an MCP server injected into agents | Accepted |
| 0009 | Schema format: Protocol Buffers as the IDL | Accepted |
| 0010 | Wire encoding per surface | Accepted |
| 0011 | Transport: WebSocket over TLS first, QUIC later | Accepted |
| 0012 | Terminal emulator backend | Accepted |
| 0013 | Async runtime | Accepted |
| 0014 | Hub storage | Accepted |
| 0015 | Identity and device enrollment | Accepted |
| 0016 | License | Accepted |
| 0017 | Web client framework | Accepted |
| 0018 | Adapter registry and signing | Accepted |
| 0019 | Updates and version skew | Accepted |
| 0020 | Nodes enforce policy; approvals are signed by the user | Accepted |
| 0021 | models.dev as the model and price catalog | Accepted |
| 0022 | Performance budgets are requirements | Accepted |
| 0023 | Optional direct data path between client and node | Accepted |
| 0024 | Simplicity and visual quality are requirements | Accepted |
| 0025 | `agent.list` command for node-local control | Accepted |
| 0026 | Event acknowledgement and control/event stream frame kinds | Accepted |
| 0027 | Hub snapshot, client subscription and enrollment messages | Accepted |
| 0028 | Approval signatures: ECDSA P-256 | Accepted |
| 0029 | Command routing and terminal relay through the hub | Accepted |
| 0030 | `agent.kinds` command: which agents a node can start | Accepted |
| 0031 | Mesh messages between agents on different nodes | Accepted |
| 0032 | Machines reachable only by SSH: the hub keeps a reverse tunnel | Accepted |
| 0033 | `usage.report`: tokens and estimated cost for clients | Accepted |
| 0034 | Installing and enrolling a node on an SSH machine from the hub | Accepted |
| 0035 | `refs` on messages between nodes | Accepted |
| 0036 | Web UI stack: Tailwind CSS and Radix primitives | Accepted |
| 0037 | `git.log` command: recent commits of an agent's workspace | Accepted |
| 0038 | Chat transcript stream for agents without a terminal | Accepted |
| 0039 | A shared board for tasks, channels and leases across nodes | Accepted |
| 0040 | Shared boards in the client snapshot, and leases kept across a hub restart | Accepted |
| 0041 | User-signed grants for sensitive commands routed through the hub | Accepted |
| 0042 | Token usage read from the files an agent already writes | Accepted |
| 0043 | The terminal client: scope, order and dependencies | Accepted |
| 0044 | Installing adapters and manifests from the registry | Accepted |
| 0045 | The Windows node: terminals, a control channel and a process view | Accepted |
| 0046 | Mobile is the web client, installable as a PWA | Accepted |
| 0047 | Agent sandboxing with the operating system's own profiles | Accepted |
| 0048 | The desktop app is a thin Tauri shell around the hub's page | Accepted |
| 0049 | Team mode: users, roles and an audit trail first, Postgres only when SQLite is not enough | Accepted |
| 0050 | Windows agent confinement: a restricted token with a per-agent SID | Accepted |
| 0051 | No Web Push: ntfy and the PWA link are the push channel | Accepted |
| 0052 | Agents run in the selected directory; separate worktrees are optional | Accepted |
| 0053 | Browsing a machine's folders, and forgetting closed agents | Accepted |
| 0054 | A node that lost unacknowledged events resynchronizes instead of stalling | Accepted |
| 0055 | A resync keeps a running agent's board work | Accepted |
| 0056 | One hub; every node machine can reach the hub's panel | Accepted |
| 0057 | An agent can start and wait for other agents, with the user's approval | Accepted |
| 0058 | Show an adapter's errors and notices, and choose its model and mode | Accepted |
| 0059 | Launch options, including "do not ask" modes, chosen when an agent starts | Accepted |
| 0060 | A node keeps a session ledger and offers Resume after a restart | Accepted |
| 0061 | Trusting your devices on a machine in one step | Accepted |
