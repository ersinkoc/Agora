# 0039. A shared board for tasks, channels and leases across nodes
Status: Proposed
Date: 2026-11-09

## Context
Phase 5's exit criterion: two agents on different machines complete a task with a handoff and no file collisions. Today every node
has its own broker: tasks, `#channels` and file leases are node-local, and only direct messages cross nodes (ADR-0031, ADR-0035).
So two agents working on the same project from two machines cannot see each other's tasks, cannot hand a task over, and can both
edit the same path in their own checkouts. Nodes stay autonomous (an agent never depends on a live hub), the hub only routes and
stores references, and mesh content is untrusted.

## Decision
**The unit is the project, and its board lives in the hub.** A project is identified machine-independently (`project_id`, already
in `CheckoutDiscovered`). The hub keeps, per project, a *board*: the task list (with dependencies, owner, status), the `#channels`
named inside that project, and the path leases. A node's mesh tools keep working exactly as today for an agent whose project is
only on that node; the board is used when the project has checkouts on two or more online nodes (the hub tells each node, in the
answer to its first board call, whether the project is shared; until then calls stay local).

**Authority and ordering.** The hub is the single writer of a board, so claims and leases are decided in one place and in one
order: `task_claim` and `lease_acquire` are atomic there (all-or-nothing for several paths, as now). Leases carry a TTL and are
renewed by the holder's node while the agent runs; a node that goes offline loses its leases when the TTL ends (the hub never holds
a lease for a dead machine longer than that). The board is stored in the hub's SQLite file with the rest of its state and is part of
its backup.

**Failing closed.** When the hub is unreachable, a shared project's board calls fail with a clear error (`board.unavailable`) and
the agent keeps working in its own worktree with its local tools; nothing about running an agent depends on the board. Edits are
not blocked by the board: a lease is advice agents follow through the mesh (as with local leases today), and the final guard
against two agents' work colliding is still git (each agent has its own worktree and branch).

**Protocol** (new messages and numbers only, ADR-0006): node → hub commands on the existing node-to-hub command path (the one
`MeshPost` uses, ADR-0031) `BoardCall { project_id, tool, arguments_json }` answered with `BoardResult { ok_json | error }`, where
`tool` is one of the mesh tools that touch shared state (`task_*`, `lease_*`, channel send/read, `handoff`). The hub stamps the
caller's `origin` node and agent as for `MeshPost`, so a node cannot act for another node's agent. Board changes are announced to
the owner's clients as events (`TaskChanged`, `LeaseGranted`, `LeaseReleased`, `HandoffCreated` already exist) so the console can
show the board. A handoff is a task owner change plus a message to the receiving agent, both in one hub transaction.

**What does not change.** The hub still cannot widen node policy or answer approvals; board content is untrusted text like any mesh
content (clients render it as plain text); `mesh.remote = false` on a node (ADR-0031) also stops it from using shared boards.

## Consequences
- Hub: a board per project in the store, atomic task and lease operations, TTL expiry, a command handler on the node connection.
- Node: the mesh server routes shared-project tool calls through the uplink (as `MeshPost` does) and renews leases; local-only
  projects are untouched.
- Web: a board view (tasks, who holds which paths) per project in the console.
- Tests: two nodes and a hub in one process, claim races, lease expiry when a node dies, handoff atomicity, hub-down behaviour.

## Alternatives considered
- Leader node per project: no hub dependency for the board, but needs election, a leader that can sleep or leave, and a second
  routing path between nodes (the hub already carries node-to-node traffic).
- CRDTs on every node: converges without a single writer, but task claims and path leases are exclusive by nature; a CRDT cannot
  say "only one of you got it".
- Keep everything node-local and sync through git (a `TASKS.md` in the repository): zero new infrastructure, but no atomic claims, no
  leases and no handoff notice; it is what people do today and is the failure this phase exists to fix.
