# Error codes

Errors are `{ code, params }`. `code` is a stable, dotted, lowercase string. `params` is a flat map of strings
and numbers used by clients to render localized messages (`locales/<lang>/errors.ftl`, key = code with dots
replaced by dashes). Codes are part of the public protocol: never rename or reuse one.

The terminal client's remove action preserves the hub's error parameters and renders its refusal from `errors.ftl` in the
selected language, including `agent.running` and `auth.forbidden`. Unknown codes remain visible as a fallback; a lost connection
or a local response timeout has a localized retry message.

| Code | Params | Meaning |
| --- | --- | --- |
| `auth.unauthenticated` | — | No or invalid credentials |
| `auth.forbidden` | `action` | Authenticated but not allowed |
| `auth.device_revoked` | `device_id` | Device certificate revoked |
| `protocol.unsupported_generation` | `offered`, `supported` | No common protocol generation |
| `protocol.capability_missing` | `capability` | Peer lacks a required capability |
| `protocol.invalid_message` | `field` | Message failed validation |
| `node.offline` | `node_id` | Target node not connected |
| `node.unknown` | `node_id` | The hub has never seen this node, or the request named none (ADR-0029) |
| `node.timeout` | `node_id` | The node did not answer a routed command in time (ADR-0029) |
| `node.policy_denied` | `node_id`, `rule` | Node local policy rejected the action |
| `node.resource_limit` | `resource`, `limit` | Limit reached (agents, memory, disk) |
| `project.not_found` | `project_id` | |
| `worktree.dirty` | `path` | Uncommitted changes block the operation |
| `worktree.exists` | `branch` | |
| `worktree.unmerged` | `branch` | The worktree's branch has commits that exist nowhere else |
| `agent.not_found` | `agent_id` | |
| `agent.kind_unknown` | `kind` | No manifest or adapter for this kind |
| `agent.publisher_revoked` | `kind`, `publisher` | The adapter of this kind was installed from a registry publisher that the registry has since revoked (ADR-0044) |
| `agent.sandbox_unavailable` | `kind`, `level`, `reason` | The node's policy confines agents (`sandbox = level`, ADR-0047) and this machine, or this kind of agent, cannot be confined that way; nothing was started. `reason` (added with ADR-0050 Amendment B; older nodes omit it): `unsupported` (this machine or kind cannot be confined at that level), `broad_write_grant` (a folder the agent would be given, or something below it, lets `Users` or `Everyone` write), `label_failed` (a folder the agent would write cannot be labelled for confined agents) |
| `agent.not_authenticated` | `kind`, `node_id` | Agent account not logged in on that node |
| `agent.blocked` | `agent_id` | Agent waits for input; prompt not sent |
| `agent.mode_unsupported` | `kind`, `mode` | e.g. chat mode for an L1 agent |
| `agent.running` | `agent_id` | the agent still runs, so it cannot be forgotten (ADR-0053) |
| `approval.expired` | `approval_id` | |
| `approval.signature_invalid` | `approval_id` | |
| `approval.grant_required` | `command` | ADR-0041: the node's `signed_commands` policy lists this command and no live grant covers it |
| `lease.held` | `path`, `holder_agent_id`, `expires_at` | `expires_at` is when the conflicting lease ends, in milliseconds since the epoch |
| `board.unavailable` | `project_id` | A shared project's board cannot be reached (hub unreachable), or a lease or claim waits while the project's nodes hand over their local work (up to 5 s, ADR-0039 amendment B); the agent keeps working locally (ADR-0039) |
| `mesh.recipient_unknown` | `to` | |
| `mesh.rate_limited` | — | Too many messages from this agent in the rate window |
| `mesh.body_too_large` | `limit` | Message body over the size limit; pass a reference instead |
| `mesh.too_many_refs` | `limit` | Too many references on one message |
| `mesh.too_many_asks` | `limit` | Too many unanswered `ask`s from this agent |
| `mesh.timeout` | — | An `ask` got no reply in time |
| `task.not_found` | `task_id` | No such task in the caller's project |
| `task.not_open` | `task_id` | The task is already claimed or done |
| `task.blocked` | `task_id`, `depends_on` | A dependency of the task is not done |
| `task.not_owner` | `task_id` | Only the claiming agent may do this to the task |
| `lease.not_found` | `lease_id` | No such lease held by the caller |
| `lease.invalid_path` | `path` | Empty path or one containing `..` |
| `files.not_found` | `path` | |
| `files.too_large` | `path`, `limit` | |
| `files.outside_root` | `path` | Path escapes the worktree (`..`, absolute, or a symlink leading out) |
| `adapter.signature_invalid` | `package` | |
| `adapter.crashed` | `kind`, `exit_code` | |
| `adapter.agent_exited` | `stage`, `exit_code`, `stderr_tail` | ADR-0058: the agent behind an adapter (an ACP agent behind the bridge) exited before it was ready. `stage` is `initialize` or `session`; `exit_code` is its exit code, `?` when it had none; `stderr_tail` the last lines of its error output with secrets removed (docs/specs/adapter-protocol.md), possibly empty |
| `adapter.agent_refused` | `stage` | ADR-0058: the agent behind an adapter answered `initialize` or the new session (`stage`) with an error |
| `adapter.option_refused` | `id` | ADR-0058: the agent did not switch to that mode or model |
| `internal` | `incident_id` | Unexpected failure; details only in logs |
