# Adapter protocol (L3)

An adapter is a process, in any language, that connects one agent kind to Agora. The node starts it and talks
JSON-RPC 2.0 over stdio (newline-delimited). Message types come from `schema/` (ADR-0010).

## Lifecycle

```
node → adapter  initialize { protocol_generation, node_capabilities, locale }
adapter → node  result     { adapter_name, version, agent_kind, capabilities[] }
node → adapter  session/start   { project_root, worktree, env, mcp_servers[], resume_session_id? }
adapter → node  result          { session_id }
...                              notifications in both directions
node → adapter  session/stop    { session_id, reason }
node → adapter  shutdown
```

## Capabilities an adapter may declare

`chat`, `terminal` (adapter exposes a PTY the node should attach), `diff`, `approvals`, `plan`, `resume`,
`usage`, `cancel`, `mesh.deliver` (adapter can deliver mesh messages to the agent natively).

## Methods

| Direction | Method | Purpose |
| --- | --- | --- |
| node → adapter | `session/prompt` | Send user or mesh input |
| node → adapter | `session/cancel` | Interrupt current turn |
| node → adapter | `approval/resolve` | Answer a permission request (carries the verified decision) |
| node → adapter | `mesh/deliver` | Deliver a mesh message |
| adapter → node | `state/update` | `working`, `blocked`, `idle`, with optional message |
| adapter → node | `chat/append` | Structured transcript items (text, tool call, plan, diff) |
| adapter → node | `approval/request` | Ask permission; node routes to the approvals queue |
| adapter → node | `usage/report` | Model id, tokens, reported cost if known, quota status if known |
| adapter → node | `session/reported` | Native session id for resume |
| adapter → node | `log` | Diagnostic log line (never user-facing text) |

## Rules

- Adapters never receive hub credentials or other agents' data; the node mediates everything.
- Adapters must tolerate unknown fields and methods (ignore, do not crash).
- A crashing adapter is restarted with backoff; the agent shows `unknown` until it recovers.
- First-party adapters are the reference implementations and conformance test targets.

## Host implementation (`agora-adapters::host`)

The node-side host runs an adapter as a child process and speaks this protocol over its stdio. Implemented behavior beyond the
spec text: an adapter starts with a **cleared environment** plus `PATH` and what the node passes (it never inherits the node's
secrets by accident); lines longer than 4 MiB are discarded whole; malformed lines, unknown notifications and unexpected
requests are ignored and counted (`ignored_lines`), never fatal; a crash fails calls in flight with `Exited` and ends the event
stream with `Exited`; calls have timeouts. JSON uses the proto3 mapping (lowerCamelCase, enum names or numbers, 64-bit integers
as strings or numbers on input). `agora-fake-agent --adapter` is the reference fake adapter used in tests.
The supervisor restarts a crashed adapter with the same backoff and budget as a crashed terminal agent (`RestartPolicy`): the
agent is `unknown` meanwhile, the session is resumed with `resumeSessionId` when the adapter reported a native session id,
and after the budget is spent the agent stays stopped (`GaveUp`). A user stop cancels a pending restart. Not yet: the
registry of installed adapters.
