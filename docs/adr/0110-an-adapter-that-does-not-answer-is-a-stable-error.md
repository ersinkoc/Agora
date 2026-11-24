# 0110. A prompt or decision the adapter does not take is a stable error
Status: Accepted
Date: 2026-11-24

Accepted: 2026-11-24 (the maintainer, in the merger chat: "Accept and build"). Before merge, the build found that the ACP bridge
does not answer a prompt with `adapter.agent_exited` (Context and decision 2 corrected) and that a timed-out decision lost its
card; the maintainer chose "Keep the card, retry allowed" (decision 6), the same day.

## Context

The node sends a prompt and the answer to an approval request to an adapter (L3) or the ACP bridge (L2) as JSON-RPC calls with a
10 s answer wait (`AdapterCall::run` in `crates/agora-node/src/supervisor.rs`). When the call fails, the node used to answer the client `internal` with
`incident_id` `unassigned` and log "agent start failed". PR #1166 (#1150) named the failure: the node now logs which call failed and
answers `internal` with `incident_id` `agent.prompt` or `approval.resolve`
([adapter protocol, start failures](../specs/adapter-protocol.md)). The answer is still `internal`, which the error-code spec reserves
for failures the node cannot explain. These failures have known causes:

- **No answer in time.** The adapter is alive but does not answer within the wait (a hung adapter, a stalled agent behind it).
- **The agent exited.** An L3 adapter can answer a waiting prompt or decision with `adapter.agent_exited`; the adapter spec allows
  that reason only for the `initialize` and `session` stages at the node, so the node drops it and answers `internal`. (The ACP
  bridge answers a prompt as soon as it has passed it on and reports an exit during a turn as a notice, so for ACP agents this case
  does not arise; corrected before merge, see below.)
- **The adapter answered with an error** that carries no stable reason.

A window or the terminal client cannot tell these apart, so it can only say "something went wrong". The person cannot tell whether to
wait, start the agent again, or report a bug.

## Decision

1. Add one error code, `adapter.no_answer`, with parameters `kind`, `call` (`prompt` or `approval`) and `waited_ms`. The node answers
   it when an adapter call made for a client (a prompt, an approval answer) runs out its answer wait. It replaces `internal` for
   that case only.
2. Pass `adapter.agent_exited` on for a prompt and an approval answer as it already is for a start: the node answers the client with
   that code and only its parameters (`stage`, `exit_code` as a string, `stderr_tail` scrubbed and cut at 600 bytes, as for a start).
   The adapter spec's start-failure rule is widened to these two calls (an L3 adapter may send it; the ACP bridge does not, as
   above).
3. Any other failure of these calls stays `internal` with its named `incident_id` (#1166).
4. Clients show both codes from the locale (`locales/<lang>/errors.ftl`), with a short next step: wait and try again for
   `adapter.no_answer`, start the agent again for `adapter.agent_exited`.
6. **What stays after `adapter.no_answer`.** A decision whose answer ran out its wait keeps its approval card pending: the person
   can answer it again, and a late answer from the adapter still settles it. A prompt whose call ran out its wait stays in the
   conversation, marked as not confirmed by the adapter, instead of being taken back (the adapter may already hold it). Other
   failures keep today's handling.
5. Schema and fixtures: the error code is a value of the existing error-code field. Adding it is append-only (a client that does not
   know it shows its generic text); golden fixtures are added, none edited.

## Alternatives considered

- **Keep `internal` with the named reference** (#1166 as it is). No protocol change, but the person still gets "something went wrong"
  for causes the node knows.
- **Reuse `node.timeout`.** It means the machine did not answer, and its text names the machine; the machine did answer here.
- **One generic `adapter.failed` with a `reason` parameter.** Fewer codes, but a client would have to branch on a free-form
  parameter, which the error-code spec avoids.

## Verification

- A fake adapter that never answers a prompt: the client gets `adapter.no_answer` with `call = prompt` after the wait, and the node log
  names the prompt.
- A fake L3 adapter that answers a prompt with `adapter.agent_exited` (stage `turn`): the client gets that code.
- After `adapter.no_answer` on a decision, the card is still pending and a second answer is accepted; after one on a prompt, the
  prompt is still in the conversation, marked as not confirmed.
- An adapter error without a stable reason stays `internal` with `agent.prompt`.
- The same three for an approval answer.

## Open questions

- Should a prompt that ran out its wait also be marked on the agent (its state `unknown`) or only answered to the caller?
- Is 10 s still the right answer wait for a prompt, now that the person sees the timeout?
