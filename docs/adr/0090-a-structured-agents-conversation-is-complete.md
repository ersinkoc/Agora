# 0090. A structured agent's conversation is complete: thinking, tool calls with status, plan progress, effort, cancel and usage
Status: Accepted
Date: 2026-11-19

Accepted: 2026-11-19 (the maintainer's explicit message in the merger chat: "ADR-0090'ı kabul ediyorum"). The open questions' proposed answers stand. Origin: the maintainer: in structured (ACP) mode the chat
screen must be a real coding-agent experience where everything works: choose the model and the reasoning effort (also while the agent runs),
see tool calls with kind, status, duration, files and output, messages, thinking, the plan with step status, stop the current turn (not the
whole agent), queued prompts, permission requests inline, the slash commands the agent offers, token usage in and out, and history.
Builds on ADR-0038, ADR-0058, ADR-0066; complements ADR-0089 (persistence); touches the schema, so it needs the maintainer.

## Research: what I read, and what I could not verify
- **ACP**: the official JSON schema of release `schema-v1.33.0` (`agentclientprotocol/agent-client-protocol`, downloaded and parsed) and the
  docs pages `protocol/schema` and `protocol/session-config-options` on agentclientprotocol.com (fetched through a summariser, so long
  pages were read in part; the first 100,000 characters of the schema page). Everything below about ACP names is from the JSON schema.
- **Agents**: only public READMEs: `zed-industries/claude-code-acp` and `zed-industries/codex-acp` (the latter says development moved to
  `agentclientprotocol/codex-acp`, which I did not read), and the manifests in `manifests/agents/`.
- **Not verified**: how any real agent behaves at run time (which updates it sends, whether it sends `agent_thought_chunk` or
  `usage_update`, which config options it offers); the READMEs are silent or partial on thinking, usage, effort, cancel. The
  opencode, antigravity and wrongstack manifests say they are not yet run against the real program. Nothing was run. ACP marks some
  parts unstable or opt-in (a client `session.configOptions.boolean` capability, `compaction_update`, a `notice` update); I did not
  establish which of them real agents send. The first build PR must capture real sessions as fixtures before the node mapping is built.

### ACP 1.33.0 as the schema defines it
- `session/update` kinds: `user_message_chunk`, `agent_message_chunk`, `agent_thought_chunk` (all `ContentChunk`: content block plus optional
  `messageId`), `tool_call`, `tool_call_update`, `plan`, `available_commands_update`, `current_mode_update`, `config_option_update`,
  `session_info_update`, `usage_update`, `notice`, `compaction_update`, `compaction_summary_chunk`.
- `ToolCall`: `toolCallId`, `title`, optional `name`, `kind` (`read edit delete move search execute think fetch switch_mode other`),
  `status` (`pending in_progress completed failed`), `content[]` (`content`, `diff {path, oldText?, newText}`, `terminal`),
  `locations[] {path, line?}`, `rawInput`, `rawOutput`. `tool_call_update` carries the same fields, all optional but the id.
- `plan`: `entries[] {content, priority high|medium|low, status pending|in_progress|completed}`; each update replaces the whole plan.
- `usage_update`: `used`, `size`, optional `cost {amount, currency}` (the context level and size, and the running cost). There is no
  token in/out split in this schema, and `PromptResponse` holds only `stopReason`
  (`end_turn max_tokens max_turn_requests refusal cancelled`) and `_meta`. Input and output token counts, if an agent sends them, are in `_meta` only.
- `session/cancel`: a notification; the agent stops, then answers the pending `session/prompt` with `cancelled`.
- `session/request_permission`: `options[]` of kind `allow_once allow_always reject_once reject_always`, and a `toolCall` update.
- Config options: `configOptions[] {id, name, description?, category mode|model|model_config|thought_level|_custom, type select|boolean,
  currentValue, options}`; client call `session/set_config_option {sessionId, configId, value}` whose reply is the full list; the agent may push
  `config_option_update`. The spec says config options supersede session modes (`session/set_mode`, `session/set_model`), which are to be
  removed later; agents should send both during the transition. Optional for agents ("MAY"). Reasoning effort is the `thought_level` category.
- `available_commands_update`: `availableCommands[] {name, description, input?}`.
- A prompt is queued by the client only; ACP has no queue, and a second `session/prompt` during a turn is not defined.

### What the supported agents publish (public docs only)
Claude Code ACP adapter: tool calls with permission requests, todo lists (plan), custom slash commands, an opt-in extension for model and
effort defaults; silent on thinking, usage, and cancel for v1 (cancel is named in its v2 section). Codex ACP adapter: tool calls with permission
requests, todo lists, slash commands (`/review`, `/init`, `/compact`, ...); silent on thinking, modes, models, effort, cancel, usage. The Agora
manifests today pass model and effort for the terminal kinds as launch arguments (ADR-0066: `claude --effort`, Codex `model_reasoning_effort`)
and say nothing of effort for ACP kinds.

## Context: ACP capability, carried today?, where lost

| ACP capability | Carried today? | Where it is lost |
|---|---|---|
| `agent_message_chunk` | Yes, text, chunks joined for 150 ms | Message boundaries (`messageId`) not kept |
| `agent_thought_chunk` | No | Dropped in `acp.rs` `update()` (`_ => {}`) |
| `user_message_chunk` | Only in replay of a loaded session | Dropped otherwise |
| `tool_call` | Partly: title (or kind) and `rawInput` as JSON in `CHAT_KIND_TOOL_CALL` | `toolCallId`, `status`, `kind`, `locations`, `content`, `rawOutput` dropped in the bridge |
| `tool_call_update` | Only a `diff` in `content`, as a separate `diff` entry | Status changes, output, duration, locations dropped; updates cannot be matched to a call (no id) |
| `plan` | Partly: step text joined with "\n" | `status` and `priority` dropped in the bridge; the web client cannot show progress |
| `usage_update` | Yes: cost growth, `used`, `size` become `AgentUsageReported` (ADR-0058) | Token in/out only from the files-based reader (ADR-0042); none from ACP |
| `available_commands_update` | No | Dropped in the bridge |
| `current_mode_update`, modes | Yes (`SessionOption`, `AgentSetMode`) | none |
| models | Yes (`AgentSetModel`) | `session/set_model` (the older API) only |
| `configOptions`, `session/set_config_option`, `config_option_update`, `thought_level` (effort) | No | Bridge does not read them; no command; effort is only a start-time setting (ADR-0066) for terminal kinds |
| `session/cancel` | The bridge handles the adapter message `session/cancel` | No client command reaches it: the node has no cancel-turn command, only `AgentStop` (stops the agent process) |
| `session/request_permission` | Yes, as signed approvals (`approval/request`) | Shown in the approvals list; not as an item in the conversation |
| `StopReason` | Yes, as notices with stable codes (ADR-0058) | `cancelled` is a normal return to idle, no notice |
| `session_info_update`, `compaction_*`, `notice` | No | Dropped |
| Queued prompts | No | `AgentPrompt` is a plain request; the web client cannot tell a queued prompt from a sent one |
| Tool duration | Not in ACP | Must be measured by the node (first `pending` to terminal status) |
| History | Ring of 500 entries or 256 KiB (ADR-0038) | Persistence is ADR-0089 |

The web client (`apps/web/src/net/chat.ts`) reads `ChatLine.kind` as `text | tool | plan | diff | notice`, so it cannot show more than the
node sends. The cause is the protocol and the bridge, not only the client.

## Decision
Everything is additive (ADR-0006): new enum values, new fields and new messages with new numbers; no existing message changes meaning.

1. **Chat entries** (`ChatEntry`, new `ChatKind` values and fields):
   - `CHAT_KIND_THOUGHT`: the agent's thinking, `text`. Joined like messages. Untrusted, collapsed by default in the window.
   - Tool call lifecycle: a tool call is ONE entry the node updates in place. New fields on `ChatEntry`: `call_id` (the ACP `toolCallId`),
     `tool_kind` (enum mirroring ACP `ToolKind`), `status` (enum `pending|in_progress|completed|failed`), `paths` (repeated
     `{path, line}` from `locations`), `output` (bounded text from `content` and `rawOutput`), `started_ms` and `ended_ms` (measured by the
     node, so a duration). A change is sent as a new `ChatEntry` with the same `entry_id`, which a client now REPLACES (today it ignores a repeat; an
     old client keeps the first version, which is the current behavior). Existing `CHAT_KIND_TOOL_CALL` is used; `text` stays the tool name and arguments as JSON.
   - Plan: new repeated field `plan_steps {text, status, priority}` on a `CHAT_KIND_PLAN` entry; `text` stays the joined lines for old clients. A
     new plan replaces the old one in the same entry.
   - `CHAT_KIND_PERMISSION`: a marker entry that points to an approval id and says its outcome, so the request appears inline in the
     conversation. The decision itself stays the signed approval (ADR-0020); the entry holds no decision.
   - A `queued` flag on a user entry: the prompt is waiting behind the current turn (see 5).
2. **Session options** (extends ADR-0058): new event `AgentSessionConfig {agent_id, options[]}` and `SessionConfigOption {id, name, description,
   category, type (select|boolean), current, choices[]}`, replacing the previous report. `AgentSessionOptions` (modes, models) stays and is still sent,
   built from config options when the agent offers them (the spec says agents send both). New command `AgentSetConfig {agent_id, config_id, value}` (capability
   `agent.set_config`); the node maps it to ACP `session/set_config_option` and publishes the reply. Reasoning effort is the option with category
   `thought_level`. It works while the agent runs, between turns, and during a turn if the agent accepts it; a refusal is a stable error. For an agent that
   offers no config option, effort stays a start-time setting (ADR-0066) and the window says so (UI text in `locales/`).
3. **Cancel the turn**: new command `AgentCancelTurn {agent_id}` (capability `agent.cancel_turn`), distinct from `AgentStop`. The node sends the
   adapter message `session/cancel` (it exists: docs/specs/adapter-protocol.md) and the agent returns to `idle`; the agent process lives on. Queued prompts are
   kept unless `drop_queue` is set. Not signed: it ends no process and widens nothing; it needs only the sender's agent access, like `AgentPrompt`.
4. **Slash commands**: new event `AgentCommands {agent_id, commands[] {name, description, input_hint}}`, replacing the previous list. The window offers them
   when the user types `/`; choosing one sends ordinary prompt text `/name args` through `AgentPrompt`. No new execution path.
5. **Queued prompts**: the node queues a prompt that arrives during a turn (bounded: 8 entries, each as large as `AgentPrompt` allows) and sends it when the
   turn ends; the entry carries `queued` until sent. The reply to `AgentPrompt` says queued or sent. ACP has no queue, so this is node behavior, and it makes a
   user prompt during a turn defined instead of undefined.
6. **Usage**: `AgentUsageReported` already carries input and output tokens, cost, context level and size. Add nothing for tokens from ACP until real agents are shown to
   send them (they are in `_meta` if at all); the window shows context level and cost from `usage_update`, and tokens from the file reader (ADR-0042) where it exists,
   and shows none rather than a guess.
7. **Node mapping** (in `agora-adapters/src/acp.rs`, the bridge; the node maps adapter items to entries, the hub is untouched): ACP kind to
   `ChatKind` and fields as in the table; the adapter protocol gets additive items (`chat/append` item kinds `thought`, `tool` with id and status, and
   `chat/update`), a `session/config` report, a `session/commands` report and a `session/set_config` request, each behind adapter capabilities
   (`thought`, `tool_status`, `plan_status`, `config`, `commands`). An adapter without the capability gets today's behavior.
8. **Hub**: routing only. It relays the new commands (checking the node's capabilities as for `agent.set_model`, `protocol.capability_missing`) and the new
   events; it adds no meaning, stores no chat (ADR-0038), and never sees an API key. Terminal bytes are not involved.
9. **Negotiation**: node capabilities `chat.rich`, `agent.set_config`, `agent.cancel_turn`, `agent.commands`; the window shows a control only when the node
   has the capability, and a node that lacks one is shown the plain conversation of today. A new client against an old node, and an old client against a new node
   (unknown enum values read as `UNSPECIFIED`, shown as text), both work.
10. **Web client** (a later PR, ADR-0024 rules apply): tool cards with kind icon, status, duration, files and output (collapsed past a small size); a thinking block
    collapsed by default; a plan with step status; Stop turn separate from Stop agent; queue markers; a `/` command menu; a model and effort picker; context and
    cost line. All text from `locales/`, UI vocabulary only.

## Security
- All new text (thinking, tool output, titles, command descriptions, config names) is untrusted data from a model or tool (ADR-0038, the mesh invariant): rendered as text, never
  as HTML or a link target the client follows by itself, and never interpreted as a command. A slash command is sent to the agent as the user's own prompt, nothing else.
- **Thinking** is shown only on user request in the window and is never routed to other agents or the mesh. It can hold secrets or private reasoning; it is part of the transcript
  and gets the same retention and forgetting as the rest (ADR-0089).
- **Size caps** at the node, before the hub sees anything: `output` 16 KiB per tool call (head and tail kept, a flag says it was cut), 8 KiB per thought entry, 64 plan steps of 512 bytes,
  100 commands, 64 config options of 64 choices; the ring bound of ADR-0038 still holds. A burst of updates for one call is coalesced (at most one entry update per 150 ms).
- **Secrets**: `rawInput` and `output` can hold what an agent read. The node applies the same secret removal it uses for stderr tails and for secret files (ADR-0080) before an entry is
  made; a secret-file change shows its name and counts only. The hub stores no chat, so nothing new is stored there.
- **Approvals** stay signed and verified on the node (ADR-0020). The inline permission entry is a view of an approval; a client cannot approve through a chat entry, and the hub cannot widen policy.
  A config option never grants permission: an option that skips permissions (a mode, as ADR-0058 flags `skips_permissions`) keeps the node policy check and the user's confirmation,
  and is refused by the node when policy forbids it, whatever category the agent gave it.
- `AgentSetConfig` is checked against the options the agent last reported (like ADR-0058); an unknown id or value is `agent.option_unknown`. Values are length-bounded and never reach a shell.
- Cancelling a turn is allowed to every sender who may prompt the agent; it cannot be used to stop an approval from being decided (a pending approval is answered `cancelled`, as now).

## Compatibility
- **ADR-0038**: the transcript stream and `chat.subscribe` are unchanged; an entry with a repeated `entry_id` replaces the earlier one for new clients. Old clients ignore a repeat, which shows the first version of a tool
  call (a card without a final status) and is a degraded but correct view. Unknown `ChatKind` values read as `UNSPECIFIED`, which old clients show as text.
- **ADR-0089**: the persisted transcript writes one record per entry version or one record per final entry; the choice belongs to ADR-0089 (open question 3). The new fields are optional, so older records read.
- **ADR-0066**: start-time model and effort stay; a running agent changes them through `AgentSetConfig` when it offers a config option. The launch setting and the config option share the same value names where the agent gives them.
- **ADR-0058**: modes and models stay as published; config options are the new source and `AgentSetMode` and `AgentSetModel` stay as the path for agents that offer only the older API.
- Protocol: schema change, golden fixtures only added (AGENTS.md). Spec updates (`docs/specs/protocol.md`, `adapter-protocol.md`, `error-codes.md`) move with the build PRs, not here.

## Open questions for the maintainer
1. Should a tool's `output` be kept in the 500-entry ring (up to 16 KiB per call), or only a short head (2 KiB) with the full text fetched on request? Proposed: 16 KiB, because the window must work with no second round trip.
2. Is thinking shown by default (collapsed) or only after a click? Proposed: a collapsed block; the text is sent either way (an agent's thinking can be large; a per-agent switch to not send it is possible).
3. Does ADR-0089 store each entry version or only the last one? Proposed: the last version per `entry_id` (compaction on write).
4. Prompt during a turn: queue it (proposed, up to 8) or refuse it with a stable error? Queueing is node behavior and not in ACP.
5. Effort for terminal agents while running: out of scope here (their effort is a start-time argument, ADR-0066). Accept that, or add a restart-with-effort action?
6. Token in and out for ACP agents: ACP 1.33.0 has no field. Accept "context and cost only" until a real agent shows the numbers, or read them from files (ADR-0042) for all supported kinds?
7. Which real agents must be captured first as fixture sessions (Claude Code, Codex, opencode)? Their accounts and real runs are the maintainer's to approve.

## Consequences
- The conversation of a structured agent becomes complete and live without a terminal, for any agent that sends the ACP updates; agents are still data (agents are data), no agent-specific code.
- A tool call is a mutable entry, which makes the ring, the stream and ADR-0089 handle updates, not only appends; that is the main cost.
- The adapter protocol, the schema and three crates change; every change is additive and capability-gated.
- Agents that send none of these updates look as today. Effort while running exists only where the agent offers a config option; the window says when it does not.
- Chat volume grows (output, thinking); the caps and coalescing above bound it. A benchmark is needed for the chat path before the web PR (ADR-0022).

## Build order (small PRs)
1. Fixtures first: capture real ACP sessions (fake-agent scripted from the schema, then real agents with the maintainer's approval) for the update kinds above. Tests only.
2. Schema and codegen only: new `ChatKind` values and `ChatEntry` fields, `AgentSessionConfig`, `AgentSetConfig`, `AgentCancelTurn`, `AgentCommands`, capabilities, error codes, golden fixtures. Needs this ADR accepted.
3. Adapter protocol and bridge: thought, tool status and fields, plan status, config options, commands; unit tests from the fixtures. `agora-fake-agent` gains the new updates.
4. Node: chat ring replaces entries by id, caps and secret removal, duration measuring, the new commands (`AgentCancelTurn`, `AgentSetConfig`), prompt queue, capabilities.
5. Hub: relay and capability checks (small).
6. Web, in slices: tool cards and status; thinking and plan; Stop turn and queue; model and effort picker; `/` commands; usage line. Screenshots in both themes.
7. TUI: the same views as far as they fit (can follow).
8. Docs with each PR: `docs/specs/*`, `locales/en`, ADR status when built.

## Status of the build
- Step 1 (real-agent fixtures): not done; the maintainer's to approve.
- Step 2 (schema and codegen): done in PR #771, schema only: `CommandRequest` 106 (`AgentSetConfig`) and 107 (`AgentCancelTurn`), `CommandOk` 29
  (`AgentPromptResult`), `Event` 39 (`AgentSessionConfig`) and 66 (`AgentCommands`), `ChatEntry` fields 9-20, `ChatKind` 6 and 7, the errors
  `agent.option_unknown` and `agent.queue_full`, golden fixtures (`schema/fixtures/chat-rich/` for the new fields and oneof arms). No capability is
  advertised by any build; a node answers both commands with `protocol.capability_missing` and the hub never forwards them. Spec: `docs/specs/protocol.md`.
- Same PR, node: a mode that skips permissions (`AgentSetMode`) now follows the node's `skip_permissions` policy (`agent.option_forbidden`); and
  `agent.set_config` / `agent.cancel_turn` are in the team-default `signed_commands`.
- Steps 3 and 4 (the ACP bridge and the node): done in one PR (the bridge, the node and the docs, three commits). The bridge maps thinking
  (`agent_thought_chunk` to a `thought` item), tool calls and updates (one item per version, with the call id, kind, status, locations and
  output, secrets replaced), plan entries with status and priority, `available_commands_update` and ACP config options
  (`session/config`, `session/commands`) for a node that said `chat.rich` in `initialize`; `session/set_config` becomes ACP
  `session/set_config_option` (schema 1.33.0 form: a select value as a string, a boolean as `{type: "boolean", value}`, the reply is
  the full list); `session/cancel` is reached by `AgentCancelTurn`. The node keeps tool calls and plans as entries that change in place
  by call id (duration measured by the node), enforces every cap of the Security section with tests, coalesces updates to one per
  150 ms, shows permission requests as entries whose outcome follows only the node's approval state, queues up to 8 prompts per agent
  behind a turn (`AgentPromptResult`, `agent.queue_full`), and handles `AgentSetConfig` and `AgentCancelTurn`. `skips_permissions` is
  decided by `agora_adapters::perm` (a denylist, and for options about permissions an allowlist of clearly asking values: unknown means
  skipping, so the window asks and a node policy of `forbid` refuses it); `AgentSetConfig` is held to the node's `skip_permissions`
  policy like `AgentSetMode`; a restarted adapter's reports are forgotten. The four capabilities are advertised and passed by the hub.
  Answers to the open questions are as proposed (1: 16 KiB kept in the ring; 4: queue of 8; 6: context and cost only).
- Review of #779 fixed in the same PR: a mode or config option that skips permissions is also refused with `agent.option_unconfirmed`
  (it needs the user's confirmation as a launch option does; the schema has no field for it on `AgentSetMode` / `AgentSetConfig`, so these
  are refused always until a signed `confirmed` field is added by an ADR; the web client's switch to a mode that acts freely is
  unavailable meanwhile: a grant's signature covers command names and a time, not the mode, so it cannot count as the confirmation); the classifier also reads descriptions, word order, `mode` /
  `allow` / `execution` options, full-width text, and a boolean's protective value is never refused; the prompt queue is an explicit
  `Idle` / `Busy` state machine (failed send, `done` / `unknown`, stray `idle`, cancel timeout); no blocking adapter call is made under
  the supervisor's lock; control characters and escape sequences are stripped from entries; repeated final updates are rate limited and
  followers are bounded; adapters cannot forge the node's notices; a decision that cannot be delivered cancels its entry; a queued prompt
  is at most 64 KiB.
- **Known gap (not fixed here, `docs/backlog/phase-2.md` P2-16):** the node never checks an adapter's CURRENT mode or config value
  against the `skip_permissions` policy at start: an adapter that starts in a bypass mode is not refused.
- Step 5 (hub): done: the hub keeps the last `AgentSessionConfig` and `AgentCommands` per agent (bounded, dropped with the agent) and sends them after a snapshot, since `AgentSnapshot` has no field for them. Steps 6 (web) and 7 (TUI): not done in that PR. Step 1 (real-agent
  fixtures) is still the maintainer's: nothing here was run against a real agent, and what real agents send (thinking, config options,
  `usage_update`) is unverified.
- Step 6 (web): done in a separate PR: tool calls as one row that changes in place (status mark, duration, files, output tail), thinking, the
  plan with step status, permission records, Stop turn, the queue with Drop the queue, the agent's own settings as menus (a choice that skips
  permissions is shown disabled and never sent), the `/` menu. The window cannot read a node's capabilities (`NodeSnapshot` has no field for
  them; adding one is a schema change and needs an ADR), so it goes by what the node sent (rich entries, settings or commands, or an
  `AgentPromptResult`); a node's capabilities in the snapshot would make this exact. Step 7 (TUI): not started.
- Fix #908 (RM-44, found on Linux and Windows): after Stop turn with prompts waiting, the call that ran kept spinning, the composer kept
  "Waiting for this turn to end." while the agent was idle, and the waiting prompts' answers read as one glued reply (a waiting prompt's
  entry is numbered before the answers of the turn it waits behind, so its answers followed each other). A clarification, no new rule:
  `at_ms` is the wall-clock time of an entry's event, and a queued prompt's event is its send, so its unqueued version carries that time;
  the window places the person's entries by it (ties by id; after the agent entry it followed when it saw the send) and a waiting prompt
  after everything. A call still `pending` or `in_progress` when its turn is over (a later prompt, or the agent idle for a moment; not
  while `unknown`) is shown stopped (ACP leaves that mark to the client; no `ToolStatus` value is added, which would need a schema
  change); the composer's note ends when nothing waits; `adapter.queue_dropped` names its entries (`entries`), so one failed send among
  several waiting prompts marks only that one. No message changed shape. The terminal client keeps the first version of each entry (step 7 not started) and shows each answer on
  its own line, as before.
- Step 7 (TUI): done in #1242, in the console's chat tab and the plain list's full-screen chat: `chat::merge` keeps the last version of each entry; `render_chat_view` with `chat::View` draws a tool
  call's status and duration (a call still waiting or running after its turn is shown stopped, as #908), a plan's steps, a permission's
  outcome, the prompts that wait, and the thinking on `/thinking`; `chat::ask` reads `/stop` and `/stop all` (`AgentCancelTurn`, the second
  with `drop_queue`), `/effort NAME` (the option of category thought level; an agent without one is told effort is a start setting) and
  `/set ID VALUE` for any option the agent reported (a value that skips permissions is typed twice, then the machine raises Amendment A's
  approval, `agent.approval_needed`); `Session::cancel_turn` and `Session::set_config` send them; `chat::session_line` adds the effort to
  the line above the conversation. Both send through the same per-agent grant as a prompt when a machine asks for one, and a viewer's
  window sends neither. No message or schema change. Not built in the TUI: the `/` menu of the agent's own commands, a tool
  call's output, and the web's time ordering of a waiting prompt (#908).


## Amendment A (Accepted, 2026-11-20): switching a running agent to a mode that skips permission questions needs the owner's signed approval, and then it works
Accepted: 2026-11-20 (the maintainer's explicit message in the merger chat: "ADR-0090 Ek A, kabul ediyorum", naming ADR-0090 Amendment A). The open questions' proposed answers stand: one approval covers one agent session only (no project-wide or timed cover), and "always for agents of this kind on this machine" is a later node policy the owner edits on the machine, never a card.

Origin: the maintainer: "in ACP mode the agent keeps asking me for approval: isn't there a yolo mode?" Today there is one when an agent is STARTED with it
(`--dangerously-skip-permissions` and the like, a launch option signed by the owner, ADR-0059), but for a structured agent that reports a bypass
mode of its own (an ACP session mode such as `bypassPermissions`), switching to it from the window is **refused** since step 4 (the node answers
`agent.option_unconfirmed`, the window shows the choice switched off): the schema has no signed confirmation for it, and a blanket device grant
(`signed_commands`) names the command but not the mode, so it cannot stand for the owner's confirmation. Some kinds (`wrongstack-acp`, `opencode-acp`)
have no launch flag at all, so for them a bypass mode the agent reports in its session is the only way. This amendment gives that switch the same
protection a launch has and turns the feature back on.

### Decision
1. **The node turns the switch into an approval, not a silent command.** When a client sends `AgentSetMode` or `AgentSetConfig` for a value the node
   judges as skipping permissions (the classifier of step 3-4 stays the judge, never the client) and node policy allows it
   (`skip_permissions = allow`; `forbid` and `confined_only` rules unchanged), the node does NOT apply it and does NOT refuse it: it raises an
   **approval request** of a new kind `agent.skip_permissions` (ADR-0020), exactly like a start approval: the card shows, in the node's own words,
   the agent, its folder, the machine, the option and the value, and that the agent will then run tools and change files without asking until it
   ends or is switched back. The request answers the command with `agent.approval_needed { approval_id }` (a new code, append-only) so the window
   can show the card at once.
2. **The owner's signed decision applies it.** The decision is signed over a new domain, length-prefixed like every approval domain:
   `agora-approval-skip-v1` `F(node_id) F(approval_id) F(verdict) F(agent_id) F(option_id) F(value) F(sha256(detail))`, verified by the node against
   `trusted-devices`, single use, expiring in minutes. Only an `approved` verdict applies the choice; the node applies exactly the option and value it
   rendered, never anything the decision carries.
3. **Scope: this agent's session.** The permission lasts until the agent ends, is restarted or is switched back to a safe value. A resume after a
   node restart does NOT restore it (the agent comes back asking again), because the approval named a session. The window shows the "Acts freely" badge
   for as long as it holds. Switching back to a safe value never needs approval.
4. **Standing yolo is a start choice, as today** (ADR-0059): an owner who always wants an agent kind to skip asking keeps using the launch option
   (and, later, a per-kind default in the owner's settings). Nothing in this amendment lets an agent or the hub widen it. An agent cannot request this
   switch for itself: the request comes from a window command; mesh messages and agent text never produce one (T-rows of ADR-0090).
5. **Approvals stay on the node and signed**, the hub only relays the request and the decision; a hub that invents an approval cannot sign it.
   The decision text is the node's rendering, and a device that has not been trusted cannot sign it (`agora node trust`).

### Open questions for the maintainer
- Should one approval be able to cover a whole project's agents of a kind for a time (e.g. "this project, 8 hours")? (Proposed: no, per agent session
  only; a per-kind default belongs in launch settings.)
- Should the card offer "always for agents of this kind on this machine"? (Proposed: later, as a node policy the owner edits on the machine, never from
  a card.)

### Consequences
Schema: a new approval kind and its signing domain, the error code `agent.approval_needed`, fixtures (append-only, ADR-gated); node: the approval
request path in `adapter_set_option` / `adapter_set_config`, applying exactly what was rendered, forgetting on end/restart/resume; web: the existing
approval card and the SessionBar showing the choice as "needs your approval" instead of switched off; docs: protocol.md, threat model (an approval card
for a skipping mode; a stale card after the agent ended), the "Temporarily unavailable" notice of step 4 is removed. Build: schema, node, web, as small PRs.

### Status of the build (Amendment A)
- **Schema** (this PR, first commit): additive and without a new message. The existing `ApprovalRequested` / `ApprovalResolve` /
  `ApprovalResolved` carry the new kind `agent.skip_permissions`; the new signing domain `agora-approval-skip-v1` (length-prefixed fields,
  vectors in `schema/fixtures/signing/approval-skip-v1.json`, checked by `crates/agora-protocol/tests/approval_skip_signing.rs`); the error code
  `agent.approval_needed { approval_id }` (en, tr); `docs/specs/protocol.md`, `docs/specs/error-codes.md`, threat-model row T51.
- **Node** (same PR, second commit): `AgentSetMode` / `AgentSetConfig` of a skipping value under a policy that allows it raise the approval and
  answer `agent.approval_needed`; the owner's signed decision applies exactly the rendered option and value; withdrawal on agent end, supersession
  and restart; limits (8 waiting, 6 per minute); node capability `approval.skip_permissions`. The "Temporarily unavailable" notice of step 4 no longer holds
  for the node. `AgentSetModel` of a skipping model stays refused (the amendment names modes and config options).
- **Review of #795 (security):** the detail carries the node's own `skips: true`; every agent- or machine-supplied string is shown without
  control and format characters and bounded, and an id or value that cannot be signed as shown gets no card (`agent.option_unknown`); adapters
  can raise no `agent.*` request (closes the `agent.start` forgery); per-agent limits (2 waiting, 3 per minute) beside the node's.
- **Known dead end, documented:** `AgentSetModel` of a model the node judges as skipping stays `agent.option_unconfirmed` (no card exists for models; the
  amendment names modes and config options). **Known gap:** P2-16 (`docs/backlog/phase-2.md`): the node never checks an adapter's CURRENT mode or
  config value at start or resume, so an agent whose own session comes back in a bypass mode is not asked.
- **Web** (built): `SkipApproval` draws the card for the new kind from the node's `detail` (the window writes its own words, shows the real `option` and
  `value` ids beside the agent's names, strict shape check, anything else says "update your window" and signs nothing); the decision is signed over
  `agora-approval-skip-v1` (`apps/web/src/net/skipApproval.ts`, vectors reproduced in `skipApproval.test.ts`). The SessionBar sends a skipping choice like any
  other, shows "Waiting for your approval" on the control with no value applied, an "Acts freely" badge while the current value skips (gone when the
  node reports a safe one), and returns to the previous value with a plain message when the card is declined, expires or is withdrawn; a policy refusal
  (`agent.option_forbidden`) keeps the choice marked unavailable. A card whose agent ended is dropped at once; `agent.not_found` on a decision says the
  request is no longer valid. The window cannot read node capabilities (the snapshot carries none): it treats `agent.approval_needed` from a node as the
  evidence that it advertises `approval.skip_permissions`, and answers (signs) only for such a node. The old "not available yet" texts are gone (en, tr).
- **Left open by the amendment, decided here:** for `AgentSetMode` the signed `option_id` is the literal `mode` (a mode has no option id of its own) and
  the detail's `option_type` says mode or config; the wait is the standard 10 minutes; the limits above are the node's own numbers; a decision for a
  card whose adapter restarted is refused (`agent.not_found`: the run it named is gone) and changes nothing.
