# 0058. Show an adapter's errors and notices, and choose its model and mode
Status: Proposed
Date: 2026-11-13

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.

## Context
A real run of opencode through the ACP bridge (macOS, #401) showed what a person would miss: a provider error ends the turn
silently, an agent that exits during a prompt leaves no reason, a process that dies before `initialize` is reported only as
`internal`, and the model and mode lists the agent offers (42 models; build and plan modes) cannot be used. The bridge (#405) now
produces stable codes for the first three and maps usage, but the adapter protocol has no message that carries a notice to a
person, the node turns every adapter error into plain text, and there is no way to pass a mode or a model to an adapter.

## Decision (proposed)
All changes are additive (new messages, fields, capabilities, error codes).
1. **A notice in the conversation.** A new chat item kind `NoticeItem { code, params }` in `adapter.proto` (appended to `ChatAppendParams`),
   carried through the node and hub like other chat items. Clients render it from a Fluent message id derived from the code, so
   the text lives in `locales/{en,tr}`. The bridge sends one for a failed, refused or empty turn and for an agent exit (with the
   bridge's size-limited, secret-scrubbed stderr tail as a parameter).
2. **A start failure keeps its reason.** `agent start` returns the adapter's stable code and parameters (`adapter.agent_exited`,
   stage, exit code, stderr tail) instead of `internal`, through the existing error shape (code + params).
3. **Modes and models.** The adapter reports what the agent offers (`SessionOptions { modes[], models[], current_mode,
   current_model }` event). A new adapter method `session/set_option { kind: mode|model, id }` and the node handling of the
   existing client command `AgentSetMode`, plus a new client command `AgentSetModel`. Capabilities `agent.set_mode` and
   `agent.set_model`; a node that does not support them is never sent the commands.
4. **Clients.** The conversation shows notices; the agent's header offers the mode and model lists when the agent reported them.
5. **Context size and cost.** The node accepts the bridge's usage reports (cost) and a new `context_tokens` field in the usage
   report, so the panel can show how full the agent's context is. Usage sources stay as ADR-0042 defines them.

## Open questions for the maintainer
- Setting a model changes cost and where the prompt goes. Should changing the model need an approval? (Proposed: no, it is the
  user's own click in the panel; an agent cannot call it.)
- Mode changes can lower an agent's own safety (an "auto" mode). Should the panel warn on modes the agent marks as skipping
  permissions? (Proposed: yes, with a confirmation, and never select such a mode by default.)

## Consequences
Schema (`adapter.proto`, `commands.proto`, `events.proto`), new fixtures, protocol and adapter-protocol docs, bridge, node, hub
(audit of mode and model changes) and both clients. No change to approvals or confinement.
