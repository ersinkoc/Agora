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
| 0062 | `agora up`: one command (or one login) brings a machine back, on every OS | Accepted |
| 0063 | The terminal client on ratatui and crossterm | Accepted |
| 0064 | Workspace hygiene: shared first, worktrees cleaned, the disk never fills | Accepted |
| 0065 | Pairing: find a hub, enter a number, connected | Accepted |
| 0066 | Start an agent with its model, effort and mode, from the window and from another agent | Accepted |
| 0067 | Remote access: a screen password, Tailscale and Cloudflare Tunnel, and a phone-sized window | Accepted |
| 0068 | A customizable workspace: tiled layouts, themes and replaceable panels, in every client | Accepted |
| 0069 | A missing agent kind says why, and a trust hint names the real binary | Accepted |
| 0070 | The hub owns whether a machine is online | Accepted |
| 0071 | A diff names every changed file, even when the text is cut | Accepted |
| 0072 | A shared terminal takes the size of the smallest window looking at it | Accepted |
| 0073 | Old machines are shown as old, and can be cleaned up | Accepted |
| 0074 | A reply keeps its link to the message it answers, across machines | Accepted |
| 0075 | The node updates in place without stopping its agents | Accepted |
| 0076 | A token that can only read the hub's numbers | Accepted |
| 0077 | The Windows binary is code-signed | Accepted |
| 0078 | A confined agent gets a git configuration the node wrote | Accepted |
| 0079 | A terminal holder keeps agents alive when the node stops | Accepted |
| 0080 | The file browser does not show secret files | Accepted |
| 0081 | A replayed revocation does not undo the owner's re-trust | Accepted |
| 0082 | A confined agent cannot use the node's control socket | Accepted |
| 0083 | A confined agent reaches only the unix sockets it is given | Accepted |
| 0084 | A confined agent gets its login from a secret the node holds | Accepted |
| 0085 | Machine and hub certificates renew themselves | Accepted |
| 0086 | Any machine can be the hub: one authoritative hub with takeover | Accepted |
| 0087 | Install and update coding agents on a machine from the window | Accepted |
| 0088 | Preview an agent's local web server in the window | Accepted |
| 0089 | The chat history of a structured agent survives the node and is read in pages | Accepted |
| 0090 | A structured agent's conversation is complete: thinking, tool calls with status, plan progress, effort, cancel and usage | Accepted |
| 0091 | The plan limits of subscription agents are shown from the tools' own output | Accepted |
| 0092 | The ACP registry is a source of agent kinds | Accepted |
| 0093 | Structured (ACP) agents survive a node restart | Accepted |
| 0094 | A plain terminal shows the agent program running in it | Accepted |
| 0095 | A project has a kanban board, and a task can be dispatched to an agent | Accepted |
| 0096 | Agora offers a ready helper agent instead of a coding-agent loop of its own | Accepted |
| 0098 | Generated Git configuration belongs to the node | Accepted |
| 0099 | A spike for a native desktop client in Rust on GPUI (stage 0 accepted; replacing the Tauri shell rejected 2026-11-22) | Accepted |
| 0100 | The terminal repaint carries the program's modes and its history | Accepted |
| 0101 | A structured agent can be allowed always for its session | Accepted |
| 0102 | Red is the one danger colour; amber means only "needs you" | Accepted |
| 0103 | A window gets recent Team chat history | Accepted |
| 0104 | A kept agent keeps its conversation, usage and options across a node restart | Accepted |
| 0105 | Confined Node.js agents on Windows: keep the restricted token, fix the stdio pipe in libuv | Accepted |
| 0106 | A window is told the role of the person it signed in as | Accepted |
| 0107 | `agent.list` lists the agents a node found interrupted | Accepted |
| 0108 | Forgetting an agent in the window reaches its node, which deletes its saved conversation | Accepted |
| 0109 | Recover a cross-node task handoff after a hub restart | Accepted |
| 0110 | A prompt or decision the adapter does not take is a stable error | Accepted |
| 0111 | Adapters report where they cut text, so redaction sees every edge | Accepted |
| 0112 | An agent can see every machine and agent of its hub | Accepted |
| 0113 | Deep themes and community themes, and a theme is always data | Accepted |
| 0114 | Community plugins run sandboxed in the window and ask for what they see | Accepted |
| 0115 | A repository on a case-insensitive host is one project, whatever the case of its URL | Accepted |
| 0119 | A helper started on another machine | Proposed |
