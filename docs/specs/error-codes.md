# Error codes

Errors are `{ code, params }`. `code` is a stable, dotted, lowercase string. `params` is a flat map of strings
and numbers used by clients to render localized messages (`locales/<lang>/errors.ftl`, key = code with dots
replaced by dashes). Codes are part of the public protocol: never rename or reuse one.

| Code | Params | Meaning |
| --- | --- | --- |
| `auth.unauthenticated` | — | No or invalid credentials |
| `auth.forbidden` | `action` | Authenticated but not allowed |
| `auth.device_revoked` | `device_id` | Device certificate revoked |
| `protocol.unsupported_generation` | `offered`, `supported` | No common protocol generation |
| `protocol.capability_missing` | `capability` | Peer lacks a required capability |
| `protocol.invalid_message` | `field` | Message failed validation |
| `node.offline` | `node_id` | Target node not connected |
| `node.policy_denied` | `node_id`, `rule` | Node local policy rejected the action |
| `node.resource_limit` | `resource`, `limit` | Limit reached (agents, memory, disk) |
| `project.not_found` | `project_id` | |
| `worktree.dirty` | `path` | Uncommitted changes block the operation |
| `worktree.exists` | `branch` | |
| `worktree.unmerged` | `branch` | The worktree's branch has commits that exist nowhere else |
| `agent.not_found` | `agent_id` | |
| `agent.kind_unknown` | `kind` | No manifest or adapter for this kind |
| `agent.not_authenticated` | `kind`, `node_id` | Agent account not logged in on that node |
| `agent.blocked` | `agent_id` | Agent waits for input; prompt not sent |
| `agent.mode_unsupported` | `kind`, `mode` | e.g. chat mode for an L1 agent |
| `approval.expired` | `approval_id` | |
| `approval.signature_invalid` | `approval_id` | |
| `lease.held` | `path`, `holder_agent_id`, `expires_at` | |
| `mesh.recipient_unknown` | `to` | |
| `files.not_found` | `path` | |
| `files.too_large` | `path`, `limit` | |
| `adapter.signature_invalid` | `package` | |
| `adapter.crashed` | `kind`, `exit_code` | |
| `internal` | `incident_id` | Unexpected failure; details only in logs |
