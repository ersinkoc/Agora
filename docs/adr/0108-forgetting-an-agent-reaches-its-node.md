# 0108. Forgetting an agent in the window reaches its node, which deletes its saved conversation
Status: Proposed
Date: 2026-11-23

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.

Origin: issue #1104. ADR-0089 decision 3 (Accepted) says "forgetting an agent forgets its history (ADR-0053): the file is removed with
the closed agent's other traces". #1103 (ADR-0104 step 4, C1) builds that file, `<data>/transcripts/<agent id>.jsonl` and a fresh
start's `<agent id>.earlier.jsonl`, but a forget never reaches the node, so the file stays until retention removes it (30 days). The
gap is written in ADR-0089 and ADR-0104 "Build status", in the threat model (data table and T61) and in #1103's body.

Touches ADR-0053 (`agent.forget`, Accepted; its decision 2 says "Node: no node involvement"), ADR-0089 decision 3, ADR-0104 C1,
ADR-0006 (append-only schema), ADR-0041 (grants on routed commands), ADR-0060 (interrupted agents, Resume) and the threat model
(T61). Code references are at main `9f0dea92` and #1103's head.

## Context

What happens today when a person forgets a closed agent:

- The hub answers `AgentForget` itself (`dispatch::route` gives `Route::ForgetAgent`; `server.rs` calls `Hub::forget_agent`). It
  refuses `agent.running` from its own view, appends `AgentForgotten` (producer `hub`) and stops listing the agent. A client
  `AgentForget` with a `target_node` is refused (`protocol.invalid_message`), so it is never forwarded.
- Nothing reaches the node. A node does not receive hub events: on its link it gets event acks, `ResumeOk`, routed commands
  (`STREAM_COMMANDS`, kind 0) and terminal relays (`uplink_client.rs`). The node's own command table answers `AgentForget` with
  `protocol.capability_missing` (`control.rs`, the fall-through arm). No resync tells a node which of its agents the hub forgot.
- The schema says of `AgentForget`: "The hub stops listing a closed agent. Answered by the hub", and `protocol.md` says the same.
  ADR-0053 says a node's own data is not deleted by it.

So the node cannot learn of a forget without either a new direction for an existing message or a new message. Both change what the
protocol means and ADR-0053's node clause, which needs this ADR. Two more facts matter:

- A closed agent's machine can be offline when it is forgotten (`Hub::forget_agent` checks only `running`), so a message sent at that
  moment can be lost.
- `agent.forget` works on an interrupted agent (ADR-0060, `protocol.md`), and the node keeps offering that agent for Resume from its
  ledger; retention never removes the file of an agent offered for Resume. A forgotten interrupted agent stays hidden at the hub when
  the node reports it interrupted again (`AgentInterrupted` does not clear `forgotten`), but a resume lists it again (the node reports a
  resumed agent as started, and `AgentStarted` makes a new, unforgotten record). With `resume = auto` (ADR-0060, ADR-0062 item 4) the
  node may resume it by itself at its next start.

## Options

1. **The hub forwards the existing `AgentForget` to the agent's node.** After `AgentForgotten` is stored, the hub sends
   `CommandRequest { target_node: <node>, AgentForget { agent_id: <local id> } }` on the node's link, as it routes any command
   (the local id, as every routed agent command carries). Only to a node whose `Hello` lists a new capability `agent.forget.local`;
   an older node is never sent it and keeps today's behaviour (retention). No new message and no field; the schema comment and
   `protocol.md` gain the hub-to-node direction.
2. **A new hub-to-node command** (for example `AgentHistoryForget { agent_id }`, appended to `commands.proto`) with the same delivery
   as option 1. Keeps "`AgentForget` is answered by the hub" literally true, at the cost of a second message with the same content.
3. **The window sends a second, routed command to the node** after the hub's forget (allowing `AgentForget` with a `target_node`).
   No hub state, but it fails for a machine that is offline at that moment, a forget from the terminal client or another window must
   repeat it, and the two steps can half succeed.
4. **No protocol change.** Keep retention as the only removal, add a local `agora history purge --agent ID`, and amend ADR-0089
   decision 3 to say a forget at the hub leaves the file to retention (at most 30 days, policy can lower it to 0: "once the agent
   ended").

## Decision (proposed)

Option 1.

1. **Delivery.** The hub keeps a pending list of forgets per node: the forgotten agents of that node not yet answered, stored with the
   hub's settings so a hub restart keeps it (as `board.<project>` is). It sends each pending forget when the node is connected with
   `agent.forget.local` (at once, and again after each `Welcome`), and drops it when the node answers, whatever the answer. At most
   1000 per node (oldest dropped first: retention still removes those files). Forgetting a machine (ADR-0073) drops its pending list.
2. **The node decides from its own state, not the hub's view.**
   - The id must be a valid local agent id (the transcript store's own check); otherwise `protocol.invalid_message` and nothing is
     touched.
   - If the agent runs or is held (a holder still runs it after a restart, a take-up in progress), the node refuses with
     `agent.running` and deletes nothing. The hub drops the pending forget: the agent is live, and a person can forget it again once it
     has ended.
   - Otherwise the node deletes `<id>.jsonl` and `<id>.earlier.jsonl` and drops the agent from its ledger, so it is no longer offered
     for Resume (an interrupted agent the person forgot is not brought back, without its conversation, by `resume = auto` at the next
     node start or by a later Resume). The
     agent tool's own session files, the terminal's scrollback and the worktree stay, as ADR-0053 says.
   - An agent the node does not know (already forgotten, removed by retention or `agora history purge`) is answered `ok`: a forget is
     idempotent.
3. **One owner of the files.** The delete goes through the transcript writer thread (#1103 made it the only one that touches those
   files): a delete job after any batch already queued for that agent, so a late batch cannot recreate the file. The in-memory ring of
   that agent is dropped with it, so no take-up or resume seeds from what was deleted, and a later read of the id finds nothing.
4. **No signature.** A forget deletes only what the node keeps for the person's convenience, only for an agent that no longer runs,
   and retention would delete it anyway. It never widens node policy (ADR-0020 is about widening). It passes through the node's grant
   gate like any routed command (ADR-0041), so a node whose policy lists `agent.forget` needs a live grant. The hub audits the
   person's forget as today; the node logs the delete (codes only).
5. **Docs.** ADR-0053 gets an amendment note that a node with `agent.forget.local` deletes the agent's conversation and Resume offer;
   ADR-0089 and ADR-0104 "Build status" and the threat model (data table, T61) lose the "does not reach the node" gap; `protocol.md`
   lists the hub-to-node direction and the capability; the schema comment on `AgentForget` gains one line. No field or message is
   added.

Why not option 2: its content is exactly `AgentForget`'s, and a capability already tells the hub which nodes understand the direction.
Why not option 3: the offline machine is the common case for a closed agent. Why not option 4: it leaves decision 3 of an Accepted ADR
unbuilt, which the maintainer accepted as a promise that a forget removes the conversation.

## Consequences

- A compromised hub can make a node delete the saved conversation (and the Resume offer) of any of its agents that does not run, which
  it cannot do today. Bounded: never a running agent, never the agent tool's own session or the worktree, and the file is the node's
  copy of what the window already showed. Written into T61.
- The hub keeps a small pending list per node; tests cover a forget while the node is offline (delivered on reconnect, once), a hub
  restart in between, a node without the capability (never sent), a forget of an agent that runs on the node (refused, file kept,
  pending dropped) and a forget for another node (this node deletes nothing).
- Node tests: forget, then both files are gone and a take-up or resume seeds an empty ring; a batch queued before the forget does not
  recreate the file; an invalid id touches nothing.

## Open questions for the maintainer

1. Option 1 (reuse `AgentForget` with a capability) or option 2 (a new message)?
2. Should a forget also drop the agent's Resume offer on the node (recommended), or delete only the conversation and leave Resume?
3. Is unsigned acceptable (recommended), or should a forget that deletes node data need the person's signature like `agent.resume`?
