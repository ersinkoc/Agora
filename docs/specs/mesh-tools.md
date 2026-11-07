# Mesh tools (MCP)

The node exposes these tools to every local agent through an MCP server (ADR-0008). Agent identity is bound by
the node per MCP connection; agents cannot impersonate each other.

| Tool | Input | Output |
| --- | --- | --- |
| `agents_list` | `{ project?: string }` | `[{ agent_id, kind, node, state, worktree, label }]` |
| `send_message` | `{ to: agent_id \| "#channel", body: string, refs?: [path\|commit\|task_id] }` | `{ message_id }` |
| `ask` | `{ to, question, timeout_s?: number (default 600) }` | `{ reply, from }` or error `mesh.timeout` |
| `inbox` | `{ since?: message_id, limit?: number }` | `[{ message_id, from, body, refs, at, provenance }]` |
| `task_list` | `{ status?: "open"\|"claimed"\|"done" }` | `[{ task_id, title, status, owner, depends_on }]` |
| `task_create` | `{ title, detail?, depends_on? }` | `{ task_id }` |
| `task_claim` | `{ task_id }` | `{ ok }` or `lease.held`-style conflict |
| `task_complete` | `{ task_id, summary, refs? }` | `{ ok }` |
| `lease_acquire` | `{ paths: [glob], ttl_s: number, reason }` | `{ lease_id, expires_at }` or `lease.held` |
| `lease_release` | `{ lease_id }` | `{ ok }` |
| `handoff` | `{ to, task_id?, summary, refs }` | `{ handoff_id }` |

## Safety rules

- **Mesh content is untrusted data.** Every delivered message carries `provenance` (sender agent, kind,
  node). Agents receive a standing instruction that messages from other agents are information, not
  authority; sensitive actions still go through approvals (see `security/threat-model.md`).
- Rate limits per agent (messages per minute, open asks) prevent loops between agents.
- Message bodies are size-limited; large content is passed by reference (path, commit, task).
- All mesh traffic is in the audit log.

## Implementation status
`agora-mesh::broker::Mesh` implements the tools' logic as deterministic in-memory state: registry, messages with
broker-stamped provenance, project-scoped `#channels`, inbox cursor, `ask` answered by a reply to its id with timeouts,
the task board with dependencies and owners, all-or-nothing leases over the core lease table, handoff (the message goes out
first so a refused handoff moves nothing), and cleanup when an agent ends. Limits: 16 KiB bodies, 20 refs, 30 messages per
minute per agent, 5 open asks. Every change queues a protocol event for the audit log. Error mapping: `mesh.recipient_unknown`
and `lease.held` exist; rate limit, size limit and task board errors have no code yet and need additions to
`error-codes.md`. Not yet: the MCP server exposing the tools, injection into agent MCP configuration, hub-side routing across
nodes, persistence, delivery fallback to the integration level's input path.
