# 0057. An agent can start and wait for other agents, with the user's approval
Status: Proposed
Date: 2026-11-13

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.

## Context
The mesh (ADR-0031, ADR-0039) lets agents see each other, message, share tasks and hand work over, but an agent cannot start
another one: "open a Codex and give it this task" needs the user. The panel also shows only a flat list; a helper an agent
started would be indistinguishable from one the user started. Competing tools let an agent spawn, read and wait for another.
Starting an agent is sensitive: it runs a process with the user's files and accounts, so ADR-0020 applies (a user-signed
approval verified on the node), and mesh content stays untrusted (an agent can be talked into asking).

## Decision (proposed)
1. **New mesh tools** (capability `mesh.spawn`; the MCP list grows, existing tools do not change):
   - `agent_start { kind, folder?, prompt?, node? }` -> `{ agent_id }` after approval. `folder` defaults to the caller's folder;
     `node` defaults to the caller's node.
   - `agent_wait { agent_id, state, timeout_s? }` -> the state reached, or `mesh.timeout`. Only for agents the caller started.
   - `agent_output { agent_id, lines? }` -> the last lines of the screen text of an agent the caller started (bottom of the live
     buffer only, ADR detection rule; size-limited). Not for other agents.
2. **Every start needs the user's signed approval.** The request appears in the panel as an approval of a new kind
   (`agent.start`) showing who asks (agent, machine), the kind, the folder and the first prompt in full. The node verifies the
   signature before it starts anything; the hub cannot approve for the user. A user may approve once, or "for this agent, up to N
   helpers of this kind in this folder, for this session" (a scoped, signed grant, shown and revocable in the panel).
3. **The helper is never wider than the one that started it.** Its folder is the caller's folder or a worktree of it; any other
   folder is refused (`mesh.folder_not_allowed`) unless the approval names it, and the node's confinement for the helper is at most the
   caller's. Folder identity rules of ADR-0052 apply.
4. **Limits.** Depth 2 (a helper cannot start helpers unless the user's grant says so), 3 running helpers per agent, and the
   node's existing agent limit. Starting is rate-limited per agent like messages.
5. **The tree is visible.** `AgentStarted` gains an optional `parent_agent_id` (append-only; old clients ignore it). The panel and
   TUI show helpers under the agent that started them and label them "started by Claude Code (agent-3)".
6. **The prompt is the helper's task, and is shown to the user** in the approval; the helper still treats later mesh messages as
   information (ADR-0031). All starts, approvals, grants and refusals are audited with the asking agent.
7. **Stopping.** Stopping a parent from the panel asks whether to stop its helpers too (default yes). A parent that exits on its
   own leaves its helpers running; they show under the closed parent until the user stops them.

## Open questions for the maintainer
- Is a scoped grant ("up to N helpers, this session") acceptable, or must every start be approved one by one? (Proposed: both.)
- May a helper be started on another machine, or only on the caller's? (Proposed: another machine only if the approval names it.)
- Should `agent_output` exist at all? Screen text can hold secrets; it is limited to helpers the caller started. (Proposed: yes.)
- Default helper limits (depth 2, 3 per agent)?

## Consequences
Schema: `AgentStarted.parent_agent_id` (new optional field) and an approval kind `agent.start`, new error codes
(`mesh.folder_not_allowed`, `mesh.helper_limit`), fixtures and `protocol.md`; mesh tool list and spec; node: approval check and
limits; hub: audit; clients: tree view. Threat model: new row for an agent talked into starting agents (mitigated by the signed
approval showing the prompt, the scope and depth limits, and the confinement ceiling).
