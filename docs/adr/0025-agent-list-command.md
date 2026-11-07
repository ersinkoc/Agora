# 0025. `agent.list` command for node-local control
Status: Accepted
Date: 2026-11-07

## Context
Phase 2's exit criterion is a node controllable from a local CLI: `agora agent start|stop|list`, each mapping 1:1
to a protocol command (`docs/backlog/phase-2.md`, P2-14). `start` and `stop` exist (`AgentStart`, `AgentStop`).
There is no command that returns the agents a node is running. Clients normally learn this from the hub's
`Snapshot` plus events (`docs/specs/protocol.md` §5), but a local CLI talks to a node directly, possibly with no
hub at all (nodes are autonomous), so it has neither a snapshot nor an event history: the node's event buffer
only retains unacknowledged events and is not a state query.

Faking it (a CLI-private message, or deriving state from buffered events) would put a second, unspecified
protocol next to the real one.

## Decision
Add one command and its result to `schema/agora/v1/commands.proto`, appended with new field numbers (the schema
stays append-only, ADR-0006):

- `CommandRequest.agent_list = 15` of new message `AgentList { string node_id = 1; }` (empty id: this node).
- `CommandOk.agent_list = 11` of new message `AgentListResult { repeated AgentSummary agents = 1; }`.
- `AgentSummary { agent_id, kind, AgentState state, worktree_id, terminal_id, bool running, uint32 restarts }`.

The node answers from its supervisor's current state. The hub may also use it to rebuild its view of a node after
a gap too long for event replay (`SnapshotRequired`), which keeps one definition of "what is running on this node".
Golden fixtures are added for the three new messages.

## Consequences
- The local CLI and any future tooling can list agents without a hub.
- One more command to keep stable forever; it is deliberately a plain read of existing state, with no new concepts.
- Until this ADR is accepted, `agora agent list` is not implemented; `start`, `stop`, and `terminal attach` do not
  depend on it.
