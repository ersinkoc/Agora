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

ADR-0090 adds five, all additive. The node says in `initialize` `nodeCapabilities` that it takes rich chat (`chat.rich`); an adapter
that sees it may declare: `thought` (it sends `chat/append` `thought` items), `tool_status` (tool call items carry a `callId`, a kind, a
status, locations and output, and later items update the same call), `plan_status` (plan items carry `entries` with status and
priority), `config` (it sends `session/config` and takes `session/set_config`), `commands` (it sends `session/commands`), and `turns`
(it reports `working` or `blocked` when a turn begins and `idle` when it ends, which is what lets the node queue a prompt behind a
turn: an adapter without `turns` is sent every prompt at once, as before; see "Turn state" in `docs/specs/protocol.md`: the node counts an
`idle` only after a `working` or `blocked`, so a stray or repeated one is ignored, and reads `done` or `unknown` as the end of the turn too). An adapter, or a node, without these keeps the behaviour
described before ADR-0090, byte for byte: the ACP bridge sends the new items only to a node that said `chat.rich`.

ADR-0111 adds `chat.cuts`: the adapter reports every place it removes text from a string of a `chat/append` item (the `cuts` field
below). For an adapter that declared it, an item without `cuts` means it cut nothing. The node reads `cuts` only from an adapter that
declared `chat.cuts`; from any other it ignores the field and keeps the behaviour described before ADR-0111.

## Methods

| Direction | Method | Purpose |
| --- | --- | --- |
| node → adapter | `session/prompt` | Send user or mesh input |
| node → adapter | `session/cancel` | Interrupt current turn |
| node → adapter | `approval/resolve` | Answer a permission request (carries the verified decision). ADR-0101, only to an adapter that declared `approval_scopes`: `scope` (`APPROVAL_SCOPE_ONCE` or `APPROVAL_SCOPE_ALWAYS`, the latter only for a request that offered it). An adapter told ONCE must never answer the agent with an option the agent remembers; one that cannot pick exactly the answer named refuses with `-32602`, `data` `{code: "approval.scope_unavailable"}`, and sends the agent nothing. A result means the answer was given to the agent (the ACP bridge answers only once its writer wrote it to the agent's stdin and flushed it); one it could not give, or whose delivery it could not confirm within 5 s (the bridge then ends the agent before answering), is an error (`adapter.approval_undelivered`), and the node then records the request cancelled |
| node → adapter | `mesh/deliver` | Deliver a mesh message |
| node → adapter | `session/set_config` | ADR-0090, capability `config`: `{sessionId, configId, value}`, one of the ids and values the adapter reported in `session/config` (`value` is a choice id for a select, `"true"` or `"false"` for a boolean). Empty result once the agent took it; otherwise an error whose `data` is `{code: "adapter.option_refused", params: {id}}` |
| node → adapter | `session/set_option` | ADR-0058: `{kind: SESSION_OPTION_KIND_MODE \| SESSION_OPTION_KIND_MODEL, id}`, one of the ids the adapter reported in `session/options`. Empty result once the agent took it; otherwise an error whose `data` is `{code: "adapter.option_refused", params: {id}}` |
| adapter → node | `state/update` | `working`, `blocked`, `idle`, with optional message |
| adapter → node | `chat/append` | Structured transcript items (text, tool call, plan, diff, and since ADR-0058 a notice: `{code, params}`, never text). ADR-0090: `{thought: {text}}`; a tool call `{toolCall: {callId, tool?, argumentsJson?, kind?, status?, locations?: [{path, line}], output?}}` where a later item with the same `callId` updates the call (a field it leaves out keeps its value, one it sends replaces it, as ACP's `tool_call_update` does); a plan `{plan: {steps, entries?: [{text, status, priority}]}}` that replaces the earlier plan. ADR-0111 (capability `chat.cuts`): optional `cuts: [{field, at}]` beside the item, see "Reported cuts" |
| adapter → node | `approval/request` | Ask permission; node routes to the approvals queue. ADR-0101: `scopes` (the answers the agent offers: `APPROVAL_SCOPE_ONCE`, `APPROVAL_SCOPE_ALWAYS` for one it remembers; empty means ONCE only), `alwaysLabel` (the agent's own name for its always option) and `alwaysOptionId` (that one option's exact id). The node reads them only from an adapter that declared `approval_scopes`; from any other the request offers ONCE only |
| adapter → node | `usage/report` | Model id, tokens, reported cost if known, quota status if known; since ADR-0058 `contextTokens` and `contextWindow`, the context's level and size now (0 when unknown) |
| adapter → node | `session/options` | ADR-0058: `{modes[], models[], currentMode, currentModel}`, each option `{id, name, description, skipsPermissions}`. Sent at the start and whenever something in it changes; each replaces the last |
| adapter → node | `session/config` | ADR-0090: `{options[]}`, each `{id, name, description, category, type: "select" \| "boolean", current, choices[] {id, name, description, skipsPermissions}, skipsPermissions}`; each report replaces the last |
| adapter → node | `session/commands` | ADR-0090: `{commands[] {name, description, inputHint}}`; each report replaces the last |
| adapter → node | `session/reported` | Native session id for resume |
| adapter → node | `log` | Diagnostic log line (never user-facing text) |

### What the node does with notices, options and errors (ADR-0058)

- **Notices.** A `chat/append` notice becomes a conversation entry of kind `CHAT_KIND_NOTICE` with `code` and `params` and no text.
  The node keeps it only when the code is dotted lowercase words (`[a-z0-9_]+(\.[a-z0-9_]+)+`, at most 64 bytes); it keeps at most 8
  parameters whose keys are such words (at most 32 bytes) and cuts each value at 1 KiB. Clients render a notice from their locale
  files by its code; the text of a parameter (a stderr tail) is untrusted and shown as text.
- **Options.** The node keeps the last `session/options` of each agent, bounded (at most 200 modes and 200 models; an option with an
  empty or longer than 200-byte id, or a repeated id, is dropped; names cut at 200 bytes, descriptions at 500; a current id that is
  not in its list is dropped; `skipsPermissions` is the adapter's flag or the node's own rule, `agora_adapters::perm`, for modes and
  models alike, ADR-0090) and records it as the event `AgentSessionOptions` when it changed. A restarted adapter's report replaces
  nothing until it arrives: the node forgets what the old process reported (options, config options, commands, the queue). `agent.set_mode` / `agent.set_model` are refused with `protocol.capability_missing` for a terminal agent or an adapter that
  did not declare the switch, and with `protocol.invalid_message` (`field`: `mode` / `model`) for an id the agent did not offer;
  only then is `session/set_option` sent. An agent cannot call either: no mesh tool reaches them.
- **ADR-0090 items.** `session/config` is bounded (64 options of 64 choices; an empty, overlong or repeated id dropped; text cut; a
  select's current value that is not a choice dropped; a boolean's is `true` or `false`) and each `skipsPermissions` is the adapter's flag
  or the node's rule; the event `AgentSessionConfig` is recorded when it changed. `session/commands` is bounded (100) and recorded as
  `AgentCommands`. `agent.set_config` is refused with `protocol.capability_missing` for a terminal agent or an adapter that did not
  declare `config`, with `agent.option_unknown` (`id`: the option, or the value) for what was not reported, and with
  `agent.option_forbidden` for a choice that skips permissions when the node's `skip_permissions` policy forbids it; a choice that skips
  under a policy that allows it is not sent either until the owner's signed approval arrives (`agent.approval_needed`, ADR-0090
  Amendment A); only then is `session/set_config` sent. `agent.cancel_turn` sends `session/cancel` to an adapter that declared `cancel`. A prompt that arrives while
  an adapter that declared `turns` has a turn open (a prompt was sent and no `idle` came after) is queued, at most 8 (`agent.queue_full`),
  and sent, one per `idle`, in order. Chat items: a tool item with a `callId` of more than 256 bytes is dropped; the others are described
  in `docs/specs/protocol.md` ("A structured agent's conversation").
- **Start failures.** When `initialize` or `session/start` fails with an error whose `data.code` is `adapter.agent_exited`,
  `adapter.agent_refused` or `adapter.option_refused`, the node answers the client with that code and only that code's parameters
  (`docs/specs/error-codes.md`), as strings cut at 600 bytes (`exit_code` is `?` when the adapter sent none). Any other failure is
  still `internal`. A prompt or the answer to an approval request that the adapter does not take (no answer in time, it exited, or it
  answered with an error) is `internal` too, with `incident_id` `agent.prompt` or `approval.resolve` (#1150).
- **Usage.** `usage/report` becomes `AgentUsageReported` with the tokens as before, the cost growth (`cost_micros`, only with a
  three-letter `currency`) and the context's level and size.

### Reported cuts (ADR-0111)

An adapter that shortens a string of a `chat/append` item before it sends it (a long tool output, arguments or a plan step) says
where, so the node can look for the part of a secret value that a cut may leave on either side (threat model T61). The item's params
carry `cuts`, a list of `{field, at}`:

- `field` is a JSON pointer into the item's params naming the string the cut is in, one of `/text/text`, `/thought/text`,
  `/toolCall/tool`, `/toolCall/argumentsJson`, `/toolCall/output` (a call with a `callId`), `/toolCall/locations/N/path`,
  `/plan/steps/N`, `/plan/entries/N/text`, `/diff/path`, `/diff/unifiedDiff` and `/notice/params/KEY` (`~1` for a `/` in a key, `~0`
  for a `~`), always within the item the params carry (a plan with `entries` is read from its entries, so a cut in `/plan/steps/N`
  of such an item is ignored: report it in `/plan/entries/N/text`);
- `at` is a byte offset in that string as sent where text was removed (a JSON number, or a decimal string as proto3 JSON writes a
  64-bit integer): `0` (its start was dropped), its length (its end was dropped), or a place between two characters inside it (a
  middle was dropped).

At most 64 entries are read per item. Each entry past the 64th, and each one whose `field` is not one of the above for this item or
does not name a string, or whose `at` is past the string's end or inside a character, is ignored on its own: never the item, which is
kept as if the entry were not there. The node logs that it ignored entries once per agent (`adapter.cuts_ignored`, with the agent and
the count, never the text). The node treats each valid place as a place it cut itself: it carries it through its own cleaning and
caps, and masks a fragment (4 bytes or more) of a held secret value that touches or crosses it, in the live stream and in the saved
transcript alike. A place in the agent's words or thinking (`/text/text`, `/thought/text`) is a place in their stream too, which joins both. A place never changes how the node cleans the string: one inside an escape sequence, or between `\r` and `\n`, is taken where cleaning the whole string puts the text around it. Nothing else changes: an item from an
adapter without `chat.cuts`, or without `cuts`, is read as before ADR-0111.

## Rules

- Adapters never receive hub credentials or other agents' data; the node mediates everything.
- Adapters must tolerate unknown fields and methods (ignore, do not crash).
- A crashing adapter is restarted with backoff; the agent shows `unknown` until it recovers, and ends with an error once the node gives up on it.
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
and after the budget is spent the agent stays stopped (`GaveUp`). #902 (RM-13): each restart is said in the agent's conversation (the node's
own notice `adapter.restarting`, param `attempt`); giving up records `AgentExited` (and no `TerminalClosed`: it had no terminal) with
the last adapter process's own non-zero exit code, a signal as a shell reports it (128 + the signal, so SIGKILL is 137: a window reads
only crash signals as failures, and an agent given up on always failed), or exit code 1 when that is not known or was 0, so a window
shows the agent ended with an error and takes no more prompts; the conversation, and the window's summary of the closed agent, say so
(notice `adapter.gave_up`, params `restarts` and, only when the node knows it, `exit_code`). A user stop cancels a pending restart and ends the adapter process (`session/stop`, then `shutdown`, then kill). Not yet: the
registry of installed adapters.

## L2: ACP agents through the bridge (`agora-adapters::acp`)
An agent that speaks the Agent Client Protocol over stdio needs no adapter. Its manifest says `[structured] acp = { argv = [...] }`
and the node runs `agora acp-bridge -- <argv>` as the adapter: the bridge is the ACP *client* towards the agent and an adapter
towards the node, so chat items, approvals, state and cancel work as for any L3 agent and nothing else knows about ACP.
`agora agent kinds` reports such a kind as level L2. On Windows the bridge starts a `.cmd` shim such as `npx` by its full path
(`agora_platform::program_path`, see agent-manifest.md). `agora acp-bridge --runner-cwd <dir> -- <argv>` starts the agent's command
in `<dir>` instead of the bridge's own working folder (the worktree); the node passes it only for an ACP registry kind that starts
through a package runner (#1150, agent-manifest.md), and the agent still gets the worktree as the session's `cwd`.

Each input accepts lines up to 4 MiB, excluding the line feed. The two readers share an eight-entry queue of raw lines and
wait for space when it fills. The bridge parses lines as it dispatches them, preserving each sender's order. Malformed JSON
and invalid UTF-8 within the limit are ignored. A line over the limit closes that input as soon as the extra byte is detected.
On the agent side the bridge stops the agent and uses the existing `adapter.agent_exited` reason with the current stage.
On the node side the bridge stops the agent and exits with an invalid-input error, which the host sees as an adapter exit.

The bridge writes to the agent on a separate thread so it can continue dispatching updates while the agent reads a
command. The writer holds up to 64 queued commands, and at most 8 MiB of queued bytes (one line is always accepted into an empty
queue), in addition to the current write of at most 4 MiB, so its worst-case memory is about 12 MiB (ADR-0022). A healthy agent
that is busy and not reading stdin therefore survives a burst of permission answers, cancels and mode switches. If either
limit is hit, the bridge stops the agent and uses `adapter.agent_exited`. The node's adapter host reads bridge replies independently of its
command writes.

Every `chat/append` line the bridge makes from the agent's content stays within the node's 4 MiB line limit, measured on the
complete serialized line (envelope and JSON escaping included; the caps in the table count decoded text, and a rich plan carries
its text twice). A line that would be longer has every string of its item longer than 256 bytes (head and tail kept) and every
list cut to half, repeatedly, until it fits; the session id is never cut. The item arrives shortened instead of being dropped
whole by the node. ADR-0111: to a node that said `chat.rich` the bridge declares `chat.cuts` and reports every place it removed text
from a string of a `chat/append` item ("Reported cuts"): each place of that halving (one halving moves the places an earlier one, or
the bridge's own cut, left in that string; a place inside what it removes becomes its own), the head-and-tail cuts of a tool's
arguments, output and location paths, a plan step (in its entry, `/plan/entries/N/text`) and a diff, and in a notice's stderr tail
the place after its leading `…` and both sides of each line break where lines were left out. At most 64 places per item: should more be
needed (only a tool call whose title, arguments and output are cut and more than 61 of its location paths can), those of the last
locations are left out. The `[redacted]` marks of its
shape check are not reported: the node finds them itself. Not reported: the home folder written `~` in a stderr tail (threat model
T61). A node without `chat.rich` gets neither the capability nor `cuts`, as before ADR-0090. Session ids are at most 1024 bytes: a `session/new` (`session/load`) answer with a longer id is refused like an
agent error (`adapter.agent_refused`, `stage` `session`; the notice's `rpc_code` is `0` and its `detail` `session_id_too_long`), and
a `session/start` whose `resumeSessionId` is longer is answered with JSON-RPC `-32602` and `data` `protocol.invalid_message`
(`field` `resumeSessionId`) without asking the agent; an id is never cut, as that would name another session. With that, a cut
item always fits; should one still not, the item is dropped and a `log` line says so (`adapter.line_too_long`, below) rather than
an overlong line being sent.

| ACP (agent → client) | Adapter protocol (bridge → node) |
| --- | --- |
| `session/update` `agent_message_chunk` | `chat/append` text, chunks joined for up to 150 ms (or 4 KiB; a chunk that would take the joined text past 4 KiB first sends what is buffered). An explicit stop or the end of the node's request stream flushes buffered text while the reply stream remains open |
| `session/update` `tool_call` (title, `rawInput`) | `chat/append` tool call; a `diff` in its content (or in `tool_call_update`) becomes a diff item (old and new text shown as a whole-file replacement); the tool's arguments and the diff are each cut to 64 KiB (head and tail kept) |
| `session/update` `plan` | `chat/append` plan, one step per entry; with `chat.rich` also `entries` (`text`, `status`, `priority`), at most 256 entries with text cut at 4 KiB and status and priority at 40 bytes |
| `session/update` `agent_thought_chunk` | With `chat.rich`: `chat/append` `thought`, chunks joined like a message (150 ms, 4 KiB; a message and a thought never share an item); thinking does not count as the agent saying something (a turn of only thinking is still an empty turn). Otherwise dropped |
| `session/update` `tool_call` and `tool_call_update` with `toolCallId` | With `chat.rich`: `chat/append` tool call with `callId`, `kind`, `status`, `locations` (at most 64) and `output`: the text blocks of `content`, or else `rawOutput`, cleaned as the node cleans a tool's output (escape sequences, control characters, unbalanced direction isolates) and with anything that looks like a secret replaced, then cut to 64 KiB (head and tail kept); arguments, plan steps and location paths are likewise cleaned as the node cleans them before the bridge cuts them (the secret removal is the same as for a stderr tail); only the fields the update carries are sent. A `diff` in `content` still becomes a diff item. Without `chat.rich`, or without an id, as before |
| `session/update` `available_commands_update` | With `chat.rich`: `session/commands` (at most 100) |
| `session/new` `configOptions` and `config_option_update` | With `chat.rich`: `session/config` (at most 64 options of 64 choices; the choices of a grouped select flattened). When the agent gave no `modes` (or no `models`) of its own, the mode (model) list of `session/options` is built from its option of category `mode` (`model`), and `session/set_option` then calls `session/set_config_option` on that option |
| `session/request_permission` | `approval/request` (action: the tool call's title, detail: its input cut at 300 characters) and state `blocked` |
| answer to `session/prompt` (stop reason) | state `idle`; the prompt itself set `working`. A turn that failed, was stopped by the agent or said nothing carries a reason (below) |
| `session/update` `usage_update` | `usage/report` when the cost grew or the context's level or size changed: `costMicros` is how much the session's running cost grew since the last report (in micro-units of its `currency`, only when it grew), `model` the current model, tokens 0, `contextTokens` and `contextWindow` the update's `used` and `size` (ADR-0058) |
| `session/new` `modes` and `models` | `session/options` (ADR-0058) right after the session started: `availableModes` (`id`, `name`, `description`) and `availableModels` (`modelId`, `name`, `description`), at most 200 of each, ids over 200 bytes dropped, names cut at 200 bytes and descriptions at 500; `currentModeId` and `currentModelId`. ADR-0090: `skipsPermissions` follows `agora_adapters::perm`, shared by the bridge and the node. ACP has no field for it, so the rule errs on the side of asking the user, in two parts. (1) A value whose description says `without asking` or `approve ... automatically`, whose id or name has the whole word `auto` (not for a model), or whose id or name (lowercase, letters and digits only) contains `bypass`, `yolo`, `danger`, `skippermission`, `nopermission`, `noapproval`, `fullaccess`, `autoapprove`, `autoaccept`, `acceptedits`, `acceptall` or `unsafe` skips. (2) In an option that is about permissions (ACP category `mode`, the older `modes` list, or an option id or name containing `permission`, `approval`, `approve`, `sandbox`, `autonomy`, `trust`, `safety` or `access`) a value is safe only when its id or name is one of `default`, `ask`, `askeachtime`, `askbeforeedits`, `plan`, `readonly`, `suggest`, `manual`, `normal`, `standard`, `confirm`, `build`, `code`, `architect`, `debug`, `review`, `chat`, `explain`, `orchestrator` and the like; any other value is flagged, so a name no list knows is not taken for safe. A boolean option about permissions is flagged whichever way it is set. A model or a reasoning level (`model`, `model_config`, `thought_level`) is never a permission option by its category alone; only the strong words of (1) flag it (not `auto` as a name, nor a description that merely has `approve` or `accept` and `automatically`). An option is about permissions only when it is category `mode`, is named exactly `mode` (or `agent mode`, `execution mode`, ...), or has `permission`, `approval`, `approve`, `sandbox`, `autonomy`, `confirm`, `allow` or `execution` in its name: a name that merely has `mode`, `access` or `trust` in it (`display mode`, `access token`) does not make its unknown values skipping. The ordinary modes `build`, `code`, `architect`, `debug`, `default`, `plan` are never flagged by name: only the adapter's flag or a description that says it skips or auto-approves does. The node applies the same rule itself to what any adapter reports, and an adapter's own flag is never cleared |
| `session/update` `current_mode_update` | `session/options` again with the new current mode |
| other updates | dropped |

The bridge accepts updates with a session ID for the active session or a matching pending `session/load`. An update without a
session ID uses the active session for compatibility. A successful change of session retires pending prompts from the previous
session. Only a reply to an active prompt ends its turn. A prompt whose cancel was sent does not count for the turn: a prompt the
node sends while it is still unanswered (the node's cancel grace passed) goes to the agent at once, its reply ends the turn with
`idle`, and the cancelled prompt's late reply ends and changes nothing. The cancelled turn's permissions fail closed: while any
cancelled prompt is unanswered, every ACP permission request is answered `cancelled` and never shown to the user, since one names the
session, not the prompt, and may be the stopped turn's. This also refuses a new turn's permission requests in that window; it ends
when the cancelled prompt answers, the session changes, or the agent exits. When prompts overlap otherwise (the node sent one into a
running turn, such as to an adapter without `turns`), only the reply to the last of them ends the turn: an earlier reply reports its
failure or stop as a notice but no `idle`, and the state returns to `working`. Any prompt's reply (success, error, or a refusal as
busy) answers every pending permission request `cancelled` first: ACP names the session in one, not the prompt, so it may be the
finished prompt's. This fails closed: an overlapping prompt's own request made before that reply is cancelled too, and the agent
asks again if it still needs it. A new prompt that comes while a permission request is pending reports `blocked`, not `working`:
the agent still waits for the user. A permission request that arrives while no prompt runs (none at all, or only cancelled ones;
for example a straggler of a turn that ended) belongs to no turn: it fails closed, answered `cancelled` at once, never shown to the
user and never reported as `blocked`, so the node cannot read it as the start of its next prompt.

Node → bridge → agent: `session/start` → `session/new` (or `session/load` when the agent said it can and a resume id is known),
with the node's `mcpServers` (the agent's mesh server) as ACP stdio servers `{name, command, args, env: []}` (at most 16; an entry
without a name or command, or with an argument that is not a string, is dropped). ACP requires every agent to take stdio servers, so
they are always sent unless the agent's `initialize` says `agentCapabilities.mcpCapabilities.stdio: false`;
`session/prompt` → `session/prompt` with one text block. `session/cancel` sends the ACP `session/cancel` notification. The bridge
replies `cancelled` to pending ACP permission requests and to requests that arrive before the cancelled turn completes. A pending
permission request the bridge drops for any other reason (the turn's last prompt was answered, the session changed) is answered
`cancelled` too, so the agent never waits for an answer that cannot come.
`session/set_option` → ACP `session/set_mode { sessionId, modeId }` or `session/set_model { sessionId, modelId }` for an id the
agent offered (anything else is refused by the bridge without asking the agent), answered once the agent answered, and followed by
`session/options` with the new current mode or model; an agent's error answer is `adapter.option_refused`;
`session/set_config` (`chat.rich` only) → ACP `session/set_config_option { sessionId, configId, value }` for an option and a value the bridge
reported (a boolean as `{type: "boolean", value}`; anything else is refused by the bridge with `adapter.option_refused`), answered once the agent
answered and followed by `session/config` from the agent's full reply;
`approval/resolve` → exactly one option the bridge captured when the agent asked (ADR-0101): an approval with scope ONCE (or none,
from a node before ADR-0101) → the first `allow_once`, **never** `allow_always`; with scope ALWAYS → the one `allow_always`, never
anything narrower or wider; a denial → `reject_once`, else `cancelled` (never `reject_always`: a remembered refusal is reserved). A
decision whose answer the request did not offer is refused (`-32602`, `data` `{code: "approval.scope_unavailable"}`), a scope the bridge
does not know or ALWAYS with a denial too (`data` `{code: "protocol.invalid_message", params: {field: "scope"}}`); the agent is sent
nothing and the request stays held (answered again, or `cancelled` when the turn ends). An option of a kind ACP does not define is
never selected. `session/request_permission` becomes an `approval/request` whose `scopes` hold ONCE when the agent offers `allow_once`
and ALWAYS when it offers exactly one `allow_always` (two or more are ambiguous: ALWAYS is not offered), with that option's `name` as
`alwaysLabel` and its `optionId` as `alwaysOptionId`. A request that offers neither raises no approval: the bridge answers it
`reject_once` (else `cancelled`) and reports `adapter.permission_unanswerable` (params `action`) as a `log` line and a conversation
notice. The bridge declares the capability `approval_scopes`. A decision for an approval the bridge no longer holds (it
answered the request `cancelled` itself, by the rules above, or never knew it) is refused with a JSON-RPC error (`-32602`) and the
agent is sent nothing; the node then marks the conversation's entry cancelled and publishes the approval as resolved, not approved
and unsigned (as when a turn is cancelled), so the hub clears it. The bridge acknowledges the decision once it is queued for the
agent. A full or disconnected writer queue gets a JSON-RPC error (`-32603`); the node cancels its entry in this case too. The node learns of a
cancelled permission when its decision is refused, and clears it: telling it the moment the bridge cancels would need a new adapter
→ node notification (follow-up). The bridge offers no capabilities: `fs/*` and `terminal/*`
requests from the agent are refused (the agent works in its own worktree), and mesh delivery to ACP agents goes through the mesh MCP
endpoint like for any agent (the server the bridge passed in `session/new`). If the agent exits the bridge exits, which the node sees as the adapter exiting (restart policy as
for L3); before that the bridge reports why (below). The bridge declares `approval_scopes` (ADR-0101), `usage`, `set_mode` and `set_model` among its capabilities, and to a node that said `chat.rich` also `thought`, `tool_status`, `plan_status`, `config`, `commands` and `turns`. The fake ACP agent (`agora-fake-agent --acp`) is the test double; real agents (Claude Code, Codex, WrongStack) need their
manifests and fixtures from real accounts (P2-09).

### Turn and exit reasons

The bridge treats the ACP `cancelled` stop reason and the ACP cancellation code `-32800` as a normal transition to `idle`.

The bridge never writes text for people. When something went wrong it gives a stable code and parameters (values are strings):

| Code | Params | When | Where |
| --- | --- | --- | --- |
| `adapter.turn_empty` | `stderr_tail` (only when the agent wrote to stderr during the turn) | The agent answered the prompt with `end_turn` (or no stop reason) and sent nothing: no text, tool call, plan, diff or permission request. opencode does this when its provider refuses the request | the closing `state/update` `idle` (`message`), a `log` line (`LOG_LEVEL_WARN`) and a `chat/append` notice (ADR-0058) |
| `adapter.turn_stopped` | `stop_reason` (`refusal`, `max_tokens`, `max_turn_requests`, ...; `other` for anything that is not a lowercase word) | The agent stopped the turn itself | same |
| `adapter.turn_failed` | `rpc_code`, `stderr_tail` (as for `turn_empty`) | The agent answered `session/prompt` with a JSON-RPC error | same |
| `adapter.agent_exited` | `stage` (`initialize`, `session`, `turn`, `idle`, `start`, `option`), `exit_code` (when it exited with one), `stderr_tail` | The agent's output ended. Every request the node still waits for (`initialize`, `session/start`, `session/set_option`) is answered with a JSON-RPC error whose `data` is `{code, params}` with the stage of that request. A `log` line (`LOG_LEVEL_ERROR`) and a `chat/append` notice carry the agent's stage, including `session` after successful initialization and before the first session opens. If the agent dies before opening a session, the bridge waits up to 2 s for a late request and answers it with the exit reason, using `initialize` or `session` for the corresponding request. Then the bridge exits | error `data` (`HostError::reason` on the node side), notice and `log` |
| `adapter.agent_refused` | `stage` (`initialize`, `session`); the notice also `rpc_code` and `detail` (the agent's error message, scrubbed and cut like a stderr tail; `rpc_code` `0` and `detail` `session_id_too_long` when the agent's session id is over 1024 bytes) | The agent answered its own `initialize` or `session/new` (`session/load`) with an error (opencode 2.x: -32603 `ServiceUnavailableError` without `opencode service`) | error `data` of the node's request (`stage` only) and a `chat/append` notice |
| `adapter.line_too_long` | `method`, `bytes` | A `chat/append` item could not be cut to fit the 4 MiB line limit (not reached while session ids are within their 1024-byte limit) and was dropped | a `log` line (`WARN`) |
| `adapter.option_refused` | `id` | The id was not one the agent offered, or the agent answered `session/set_mode` / `session/set_model` / `session/set_config_option` with an error | error `data` of `session/set_option` and `session/set_config` |
| `adapter.restarting` | `attempt` | #902: the adapter ended on its own and the node starts it again (`attempt` of the restart budget); the node's own notice, an adapter does not send it | a `CHAT_KIND_NOTICE` entry |
| `adapter.gave_up` | `restarts`, `exit_code` (only when known) | #902: the adapter ended on its own once more after its restart budget was spent (or could not be started again), so the node gave up on it; `restarts` is how often it was started again, `exit_code` the value of the `AgentExited` it recorded, left out when the node could not read how the last process ended (that `AgentExited` then carries 1). The node's own notice | a `CHAT_KIND_NOTICE` entry |
| `adapter.queue_dropped` | `count` | Prompts that waited behind a turn will not be sent: `AgentCancelTurn` with `drop_queue`, or the adapter ended (the node's own notice, ADR-0090; an adapter does not send it) | a `CHAT_KIND_NOTICE` entry; the dropped prompts' entries leave the transcript |

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
