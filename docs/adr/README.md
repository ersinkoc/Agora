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
