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
and after the budget is spent the agent stays stopped (`GaveUp`). A user stop cancels a pending restart and ends the adapter process (`session/stop`, then `shutdown`, then kill). Not yet: the
registry of installed adapters.

## L2: ACP agents through the bridge (`agora-adapters::acp`)
An agent that speaks the Agent Client Protocol over stdio needs no adapter. Its manifest says `[structured] acp = { argv = [...] }`
and the node runs `agora acp-bridge -- <argv>` as the adapter: the bridge is the ACP *client* towards the agent and an adapter
towards the node, so chat items, approvals, state and cancel work as for any L3 agent and nothing else knows about ACP.
`agora agent kinds` reports such a kind as level L2.

| ACP (agent → client) | Adapter protocol (bridge → node) |
| --- | --- |
| `session/update` `agent_message_chunk` | `chat/append` text, chunks joined for up to 150 ms (or 4 KiB) |
| `session/update` `tool_call` (title, `rawInput`) | `chat/append` tool call; a `diff` in its content (or in `tool_call_update`) becomes a diff item (old and new text shown as a whole-file replacement) |
| `session/update` `plan` | `chat/append` plan, one step per entry |
| `session/request_permission` | `approval/request` (action: the tool call's title, detail: its input cut at 300 characters) and state `blocked` |
| answer to `session/prompt` (stop reason) | state `idle`; the prompt itself set `working`. A turn that failed, was stopped by the agent or said nothing carries a reason (below) |
| `session/update` `usage_update` with a `cost` | `usage/report`: `costMicros` is how much the session's running cost grew since the last report (in micro-units of its `currency`), `model` the `currentModelId` from `session/new`, tokens 0. `used` and `size` (the context window) have no field and are dropped |
| `agent_thought_chunk`, `available_commands_update`, `current_mode_update`, other updates | dropped |

Node → bridge → agent: `session/start` → `session/new` (or `session/load` when the agent said it can and a resume id is known);
`session/prompt` → `session/prompt` with one text block; `session/cancel` → the ACP `session/cancel` notification;
`approval/resolve` → the agent's own option (`allow_once`, else `allow_always`, else the first option for an approval;
`reject_once`, else `reject_always`, else `cancelled` for a denial). The bridge offers no capabilities: `fs/*` and `terminal/*`
requests from the agent are refused (the agent works in its own worktree), and mesh delivery to ACP agents goes through the mesh MCP
endpoint like for any agent. If the agent exits the bridge exits, which the node sees as the adapter exiting (restart policy as
for L3); before that the bridge reports why (below). The bridge declares `usage` among its capabilities. The `models` and `modes`
lists of the `session/new` answer are not offered for choice: that needs an adapter method for them, which is a schema change. The fake ACP agent (`agora-fake-agent --acp`) is the test double; real agents (Claude Code, Codex, WrongStack) need their
manifests and fixtures from real accounts (P2-09).

### Turn and exit reasons

The bridge never writes text for people. When something went wrong it gives a stable code and parameters (values are strings):

| Code | Params | When | Where |
| --- | --- | --- | --- |
| `adapter.turn_empty` | | The agent answered the prompt with `end_turn` (or no stop reason) and sent nothing: no text, tool call, plan, diff or permission request. opencode does this when its provider refuses the request | the closing `state/update` `idle` (`message`) and a `log` line (`LOG_LEVEL_WARN`) |
| `adapter.turn_stopped` | `stop_reason` (`refusal`, `max_tokens`, `max_turn_requests`, ...; `other` for anything that is not a lowercase word) | The agent stopped the turn itself | same |
| `adapter.turn_failed` | `rpc_code` | The agent answered `session/prompt` with a JSON-RPC error | same |
| `adapter.agent_exited` | `stage` (`initialize`, `session`, `turn`, `idle`, `start`), `exit_code` (when it exited with one), `stderr_tail` | The agent's output ended. Every request the node still waits for (`initialize`, `session/start`) is answered with a JSON-RPC error whose `data` is `{code, params}` with the stage of that request; a `log` line (`LOG_LEVEL_ERROR`) carries the same for the stage it was in. Then the bridge exits | error `data` (`HostError::reason` on the node side) and `log` |

A `log` line's `message` is the JSON `{"code": ..., "params": {...}}`. `stderr_tail` is at most the last 5 non-empty lines and 400
bytes of the agent's stderr (cut at the front, marked `…`), with escape sequences and control characters removed, the home folder
written `~`, long runs of letters and digits (20 or more, with both) and the word after `key=`, `token:`, `Bearer ` and the like
replaced by `[redacted]`. The agent's stderr is still copied to the bridge's own, as before.

Not yet shown to people: the node keeps `log` lines and the `message` of a state out of the transcript and the event log, and maps a
failed adapter start to `internal`. Showing these reasons needs a transcript item or event field for them (a schema change, so an
ADR) and the node mapping `adapter.agent_exited` to an error code of `docs/specs/error-codes.md`.
