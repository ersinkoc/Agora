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

### The agent's mesh server (`mcp_servers`)

The node passes exactly one MCP server in `session/start` `mcpServers`, at every start, restart and resume: the agent's own mesh
server, `{name: "agora-mesh", command: <the node's own program>, args: ["mesh", "stdio", "--socket", <the agent's mesh socket>]}`,
the same server a terminal agent gets through its manifest's `[mcp]` (`agent-manifest.md`). The node registers the agent with the
mesh before the adapter starts, so the socket is the agent's own, private (owner-only), and whatever connects to it speaks as that
agent. A crash restart keeps the endpoint; a stop, a start that fails and an agent given up on take it away (the agent leaves the
mesh, its socket is removed); a resume after a stop joins again with a fresh one. The field is left out when the node has no mesh
endpoint for the agent (an agent never fails to start because of the mesh). An adapter hands the server to its agent in whatever
form the agent takes MCP servers; the mesh server runs below the adapter, inside its confinement: on a node that confines agents
offline (ADR-0047 `worktree-offline`), the adapter and what it starts may reach only that one mesh socket (and the sockets in its
private temp folder), never another agent's. On Windows a confined adapter cannot be started yet, so an adapter agent's endpoint
carries no confined identity.

## Capabilities an adapter may declare

`chat`, `terminal` (adapter exposes a PTY the node should attach), `diff`, `approvals`, `plan`, `resume`,
`usage`, `cancel`, `mesh.deliver` (adapter can deliver mesh messages to the agent natively), `set_mode` and `set_model` (ADR-0058:
the adapter takes `session/set_option` for that kind; the node never sends it to an adapter that did not declare it).

## Methods

| Direction | Method | Purpose |
| --- | --- | --- |
| node → adapter | `session/prompt` | Send user or mesh input |
| node → adapter | `session/cancel` | Interrupt current turn |
| node → adapter | `approval/resolve` | Answer a permission request (carries the verified decision) |
| node → adapter | `mesh/deliver` | Deliver a mesh message |
| node → adapter | `session/set_option` | ADR-0058: `{kind: SESSION_OPTION_KIND_MODE \| SESSION_OPTION_KIND_MODEL, id}`, one of the ids the adapter reported in `session/options`. Empty result once the agent took it; otherwise an error whose `data` is `{code: "adapter.option_refused", params: {id}}` |
| adapter → node | `state/update` | `working`, `blocked`, `idle`, with optional message |
| adapter → node | `chat/append` | Structured transcript items (text, tool call, plan, diff, and since ADR-0058 a notice: `{code, params}`, never text) |
| adapter → node | `approval/request` | Ask permission; node routes to the approvals queue |
| adapter → node | `usage/report` | Model id, tokens, reported cost if known, quota status if known; since ADR-0058 `contextTokens` and `contextWindow`, the context's level and size now (0 when unknown) |
| adapter → node | `session/options` | ADR-0058: `{modes[], models[], currentMode, currentModel}`, each option `{id, name, description, skipsPermissions}`. Sent at the start and whenever something in it changes; each replaces the last |
| adapter → node | `session/reported` | Native session id for resume |
| adapter → node | `log` | Diagnostic log line (never user-facing text) |

### What the node does with notices, options and errors (ADR-0058)

- **Notices.** A `chat/append` notice becomes a conversation entry of kind `CHAT_KIND_NOTICE` with `code` and `params` and no text.
  The node keeps it only when the code is dotted lowercase words (`[a-z0-9_]+(\.[a-z0-9_]+)+`, at most 64 bytes); it keeps at most 8
  parameters whose keys are such words (at most 32 bytes) and cuts each value at 1 KiB. Clients render a notice from their locale
  files by its code; the text of a parameter (a stderr tail) is untrusted and shown as text.
- **Options.** The node keeps the last `session/options` of each agent, bounded (at most 200 modes and 200 models; an option with an
  empty or longer than 200-byte id, or a repeated id, is dropped; names cut at 200 bytes, descriptions at 500; a current id that is
  not in its list is dropped; `skipsPermissions` is kept for modes only) and records it as the event `AgentSessionOptions` when it
  changed. `agent.set_mode` / `agent.set_model` are refused with `protocol.capability_missing` for a terminal agent or an adapter that
  did not declare the switch, and with `protocol.invalid_message` (`field`: `mode` / `model`) for an id the agent did not offer;
  only then is `session/set_option` sent. An agent cannot call either: no mesh tool reaches them.
- **Start failures.** When `initialize` or `session/start` fails with an error whose `data.code` is `adapter.agent_exited`,
  `adapter.agent_refused` or `adapter.option_refused`, the node answers the client with that code and only that code's parameters
  (`docs/specs/error-codes.md`), as strings cut at 600 bytes (`exit_code` is `?` when the adapter sent none). Any other failure is
  still `internal`.
- **Usage.** `usage/report` becomes `AgentUsageReported` with the tokens as before, the cost growth (`cost_micros`, only with a
  three-letter `currency`) and the context's level and size.

## Rules

- Adapters never receive hub credentials or other agents' data; the node mediates everything.
- Adapters must tolerate unknown fields and methods (ignore, do not crash).
- A crashing adapter is restarted with backoff; the agent shows `unknown` until it recovers.
- First-party adapters are the reference implementations and conformance test targets.

## Host implementation (`agora-adapters::host`)

The node-side host runs an adapter as a child process and speaks this protocol over its stdio. Implemented behavior beyond the
spec text: an adapter starts with a **cleared environment** plus `PATH`, the variables that say where the user's own folders are
(`HOME`, `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`, `XDG_*`, the language, terminal and Windows system-folder variables) and what the
node passes, so the agent it wraps finds its own configuration and login (it never inherits the node's secrets by accident:
API keys, tokens, `SSH_AUTH_SOCK`, the temporary and runtime folders (`TMPDIR`, `TEMP`, `XDG_RUNTIME_DIR`) and proxy and CA variables are not on the list; a manifest passes what its agent needs through `process.env`); lines longer than 4 MiB are discarded whole; malformed lines, unknown notifications and unexpected
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
| `session/update` `usage_update` | `usage/report` when the cost grew or the context's level or size changed: `costMicros` is how much the session's running cost grew since the last report (in micro-units of its `currency`, only when it grew), `model` the current model, tokens 0, `contextTokens` and `contextWindow` the update's `used` and `size` (ADR-0058) |
| `session/new` `modes` and `models` | `session/options` (ADR-0058) right after the session started: `availableModes` (`id`, `name`, `description`) and `availableModels` (`modelId`, `name`, `description`), at most 200 of each, ids over 200 bytes dropped, names cut at 200 bytes and descriptions at 500; `currentModeId` and `currentModelId`. A mode whose description says `without asking` or `approve ... automatically`, whose id or name has the whole word `auto`, or whose id or name (lowercase, letters and digits only) contains `bypass`, `yolo`, `danger`, `skippermission`, `nopermission`, `noapproval`, `fullaccess`, `autoapprove`, `autoaccept`, `acceptedits`, `acceptall` or `unsafe` is sent with `skipsPermissions`: ACP has no field for it, so the bridge errs on the side of asking the user |
| `session/update` `current_mode_update` | `session/options` again with the new current mode |
| `agent_thought_chunk`, `available_commands_update`, other updates | dropped |

Node → bridge → agent: `session/start` → `session/new` (or `session/load` when the agent said it can and a resume id is known),
with the node's `mcpServers` (the agent's mesh server) as ACP stdio servers `{name, command, args, env: []}` (at most 16; an entry
without a name or command, or with an argument that is not a string, is dropped). ACP requires every agent to take stdio servers, so
they are always sent unless the agent's `initialize` says `agentCapabilities.mcpCapabilities.stdio: false`;
`session/prompt` → `session/prompt` with one text block; `session/cancel` → the ACP `session/cancel` notification;
`session/set_option` → ACP `session/set_mode { sessionId, modeId }` or `session/set_model { sessionId, modelId }` for an id the
agent offered (anything else is refused by the bridge without asking the agent), answered once the agent answered, and followed by
`session/options` with the new current mode or model; an agent's error answer is `adapter.option_refused`;
`approval/resolve` → the agent's own option (`allow_once`, else `allow_always`, else the first option for an approval;
`reject_once`, else `reject_always`, else `cancelled` for a denial). The bridge offers no capabilities: `fs/*` and `terminal/*`
requests from the agent are refused (the agent works in its own worktree), and mesh delivery to ACP agents goes through the mesh MCP
endpoint like for any agent (the server the bridge passed in `session/new`). If the agent exits the bridge exits, which the node sees as the adapter exiting (restart policy as
for L3); before that the bridge reports why (below). The bridge declares `usage`, `set_mode` and `set_model` among its capabilities. The fake ACP agent (`agora-fake-agent --acp`) is the test double; real agents (Claude Code, Codex, WrongStack) need their
manifests and fixtures from real accounts (P2-09).

### Turn and exit reasons

The bridge never writes text for people. When something went wrong it gives a stable code and parameters (values are strings):

| Code | Params | When | Where |
| --- | --- | --- | --- |
| `adapter.turn_empty` | `stderr_tail` (only when the agent wrote to stderr during the turn) | The agent answered the prompt with `end_turn` (or no stop reason) and sent nothing: no text, tool call, plan, diff or permission request. opencode does this when its provider refuses the request | the closing `state/update` `idle` (`message`), a `log` line (`LOG_LEVEL_WARN`) and a `chat/append` notice (ADR-0058) |
| `adapter.turn_stopped` | `stop_reason` (`refusal`, `max_tokens`, `max_turn_requests`, ...; `other` for anything that is not a lowercase word) | The agent stopped the turn itself | same |
| `adapter.turn_failed` | `rpc_code`, `stderr_tail` (as for `turn_empty`) | The agent answered `session/prompt` with a JSON-RPC error | same |
| `adapter.agent_exited` | `stage` (`initialize`, `session`, `turn`, `idle`, `start`, `option`), `exit_code` (when it exited with one), `stderr_tail` | The agent's output ended. Every request the node still waits for (`initialize`, `session/start`, `session/set_option`) is answered with a JSON-RPC error whose `data` is `{code, params}` with the stage of that request; a `log` line (`LOG_LEVEL_ERROR`) and a `chat/append` notice carry the same for the stage it was in. An agent that died before the node asked anything (`start`) leaves the bridge waiting up to 2 s for the node's first request, which it answers the same way (`initialize` or `session` as its stage). Then the bridge exits | error `data` (`HostError::reason` on the node side), notice and `log` |
| `adapter.agent_refused` | `stage` (`initialize`, `session`); the notice also `rpc_code` and `detail` (the agent's error message, scrubbed and cut like a stderr tail) | The agent answered its own `initialize` or `session/new` (`session/load`) with an error (opencode 2.x: -32603 `ServiceUnavailableError` without `opencode service`) | error `data` of the node's request (`stage` only) and a `chat/append` notice |
| `adapter.option_refused` | `id` | The id was not one the agent offered, or the agent answered `session/set_mode` / `session/set_model` with an error | error `data` of `session/set_option` |

For `turn_empty` and `turn_failed` the bridge waits 100 ms for the last lines and adds what the agent wrote to stderr during that turn,
scrubbed and limited like an exit's tail. A `log` line's `message` is the JSON `{"code": ..., "params": {...}}`. `stderr_tail` is at most the last 5 non-empty lines and 400
bytes of the agent's stderr (cut at the front, marked `…`). Escape sequences and control characters are removed, OSC, DCS, APC and
PM sequences with their whole payload (a hyperlink's address, a window title). The home folder is written `~` (every spelling of it): the bridge's own `HOME` when it has one, and in any case any user's home folder (`/Users/<name>`, `/home/<name>`, `/root`, `C:\Users\<name>`, `C:/Users/<name>`, `/c/Users/<name>`, drive and `Users` in any case, names with dots or spaces), since under a node the bridge runs without `HOME`.
Replaced by `[redacted]`: the user and password of a URL (`https://user:pass@host` → `https://[redacted]@host`); any unbroken run of
32 or more token characters (letters, digits, `_ - + / = .`) that is not a path (a path starts with `/`, `~` or `.`, or has `/`
between parts shorter than 32 and no `+` or `=`), so base64 and letters-only secrets are caught; words of 20 or more characters
with both letters and digits; and the word after `key=`, `token:`, `Bearer ` and the like. This is a best effort, not a guarantee:
an agent can print a secret in a shape none of these rules match.

**Who sees the tail** (a decision, so that widening it is one too):
- Today only the node process: the host turns the `log` line into `AdapterEvent::Log`, which the node drops, and the error `data`
  into `HostError::Rpc`, whose text (the reason code, not the params) the node keeps as a log-only `Spawn` error and answers
  `internal`. Nothing is written to the node's event buffer, sent to the hub, stored or shown.
- The agent's raw stderr (unscrubbed) is still copied to the bridge's own stderr, which the node inherits: whoever reads the node's
  own output (a terminal, a service log) sees it, as before this change.
- When the node maps `adapter.agent_exited` to an error code (follow-up), the scrubbed tail becomes a parameter of the `agent.start`
  error: it then travels node → hub → the client that asked, is shown to that user, and is not kept: the hub's audit trail records
  only the error code, and command results are not events. Putting the tail into an event (stored by the hub, seen by everyone who
  may see the agent) would be a further decision.

`adapter.turn_stopped` and `adapter.turn_failed` are a notice too, like `adapter.turn_empty`. The node still keeps `log` lines and
the `message` of a state out of the transcript and the event log; the notices are what people see (ADR-0058), and a failed start
answers with the adapter's code (above).
