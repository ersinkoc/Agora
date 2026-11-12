# 0055. A resync keeps a running agent's board work
Status: Proposed
Date: 2026-11-13

Amends ADR-0054 §3 (the resync announcement). Everything else in ADR-0054 stands.

## Context
ADR-0054 lets a node that lost unacknowledged events resynchronize. Its announcement starts with `NodeOnline`, which the hub
reads as "this node's process started again": the hub ends all the node's agents in its view, drops its approvals and
terminals, and (`Boards::node_restarted`) gives every task and lease those agents held on a shared board (ADR-0039, ADR-0040)
back to the board. The announcement then re-adds the agents that still run, but their board work is already gone. So after a
resync, an agent that is still editing a file has lost its lease on it, and another machine's agent may take the file.

There is a second, quieter path to the same loss. Between the announcement's `NodeOnline` and the `AgentStarted` that re-adds
an agent, the hub's view shows that agent as ended. The board matches its members to the view's running agents on every
board call (`Boards::sync`). A call from another machine that arrives in that window unregisters the agent and frees its
leases, even if `NodeOnline` itself did not.

`NodeOnline` as a blanket reset is needed only when the hub has missed a real restart. In every other case the node knows
exactly what changed, because its supervisor keeps every agent it ran in this process: running agents are running, and
ended ones ended.

## Decision
The resync announcement states facts. It is a reset only when the hub missed this process's start. **No schema change.**

1. **`NodeOnline` only for a start the hub will never see.** The node remembers the `seq` of the last `NodeOnline` it
   recorded (at process start, or in a catch-up or resync announcement). The announcement begins with `NodeOnline` only when
   that event is in the lost range: at or after the hub's `from_seq` and older than the oldest retained event. Then the
   agents before it really died, and ADR-0054 applies unchanged. If the start event is still retained, the replay delivers
   it in order. If the hub already has it, the hub already reset what came before.
2. **Without `NodeOnline`, every agent of this process is stated once.**
   - An agent that is running is re-announced as in ADR-0054.
   - An agent that has ended gets `AgentExited` with the last exit code the node recorded for it, or the stop convention
     `i32::MIN` when none was recorded, and `TerminalClosed` for its terminal.
   - Workspaces and waiting approvals are stated as in ADR-0054.
   - Approvals that ended recently get their `ApprovalResolved` again. The node keeps the last 256 in memory, so one answered
     inside the lost window does not stay listed as waiting.
   The hub's view then never shows a running agent as ended, the board keeps its tasks and leases, and the board's next
   `sync` releases the work of agents that really ended.
3. **If the announcement itself pushes the start event out of the buffer** (a buffer so full that the announcement evicts
   it), the node appends a full ADR-0054 announcement led by `NodeOnline`. This is rare, and in that case it falls back to
   the old behaviour.
4. The hub needs no change: `NodeOnline` keeps its meaning, a real restart. The re-stated `AgentExited`,
   `TerminalClosed` and `ApprovalResolved` are idempotent in the projection.

## Alternatives considered
- **Hub-side: skip `node_restarted` for a `NodeOnline` on a resynced connection.** The hub cannot tell the announcement's
  `NodeOnline` from a real restart that is among the replayed events. It would also still have the window in which the
  view shows running agents as ended.
- **New events marking the announcement's start and end**, with agents "unconfirmed" in between. This is exact, but it is a
  schema addition, and the projection and the board would both need a third agent state.
- **Re-announcing leases from the node.** The board is the hub's record (ADR-0039); the node has no copy to send.

## Consequences
- After a resync, a running agent keeps its shared-board tasks and leases, and no other machine can take its files in the
  meantime.
- Agents that ended inside the lost window show as ended with their exit code, and their board work is released at the next
  board call, as for any ended agent.
- The node keeps, in memory, the last exit code of each agent and the last 256 resolved approvals.
- The ADR-0054 limits that remain: a working copy created inside the lost window, resource reports and git status, and usage
  counters that restart.
