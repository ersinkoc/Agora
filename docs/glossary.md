# Glossary

Use these terms exactly in code, docs, and protocol. The UI uses plain-language equivalents defined in
`ux/visual-language.md` (for example node → "Machine", approval → "Needs your OK").

| Term | Meaning |
| --- | --- |
| **Hub** | Central service. Aggregates nodes, owns identity, routing, event store, approvals, and mesh broker. Serves the web client. |
| **Node** | Daemon on one machine. Owns terminals, agent processes, files, and git operations on that machine. |
| **Client** | A user interface connected to the hub: web, desktop (Tauri), TUI, or CLI. |
| **Project** | A code repository, identified independently of machine (normalized remote URL + root commit). |
| **Checkout** | A project's working copy on a specific node. |
| **Worktree** | A git worktree created for one agent inside a checkout; the default isolation unit. |
| **Agent** | A running AI coding agent instance attached to a project, on one node, usually in its own worktree. |
| **Agent kind** | The type of agent (claude-code, codex, wrongstack, gemini-cli, ...), defined by a manifest or adapter. |
| **Terminal** | A PTY-backed process on a node (an agent in PTY mode, a shell, a server, a test runner). |
| **Session** | An agent's own conversation identity, used for resume. |
| **Agent state** | `working`, `blocked`, `idle`, `done`, `unknown`. `done` = finished and not yet seen. |
| **Integration level** | How deeply an agent kind is integrated: L0 generic, L1 manifest, L2 ACP, L3 adapter process. |
| **Manifest** | Declarative TOML describing an agent kind: detection, hooks, resume, MCP injection. |
| **Adapter** | An external process (any language) that speaks the adapter protocol for an agent kind (L3). |
| **Mesh** | The agent-to-agent coordination layer: messages, channels, task board, file leases, handoff. |
| **Lease** | A time-limited claim by one agent on a file or path, enforced by the node. |
| **Approval** | A permission request from an agent (run command, write file) routed to the central queue. |
| **Capability** | A named feature advertised during handshake; the other side enables behavior only if present. |
| **Event** | An immutable fact emitted by a node or hub; the hub's state is derived from events. |
| **Price catalog** | Model metadata and per-token prices from models.dev, cached by the hub, with local overrides (ADR-0021). |
| **Quota profile** | Versioned data describing a subscription plan's usage windows and limits, with sources and confidence. |
| **Direct path** | A client ↔ node connection for terminal and file streams, brokered by the hub, bypassing relay (ADR-0023). |
| **Predictive echo** | Client-side display of typed characters before the remote echo arrives, reconciled when it does. |
| **Budget** | A measured performance requirement with a benchmark scenario (ADR-0022). |
