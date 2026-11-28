# 0132. Show negotiated node capabilities in the window
Status: Accepted
Date: 2026-11-27
Accepted: 2026-11-27 (maintainer approval in chat).

## Context

M2.13 has a schema gate. The hub knows a node connection's negotiated capabilities, while a web
window connects through `Subscribe` and receives its own `Welcome` without those capabilities.
A task picker, for example, can show an action that the hub will refuse because the selected
node does not speak `task.dispatch`. Other refusals are about project access, policy or runtime
readiness. The client needs to distinguish these cases.

## Decision

1. The hub publishes a bounded, allow-listed subset of the capabilities negotiated on the current
   authenticated node link. Start with `task.dispatch`. Add another capability to the list when a
   client consumes it and its visibility is defined. Carry the value as an optional nested field on
   `NodeSnapshot`. Presence with an empty list means the hub knows that the link negotiated none
   of the published subset. Absence means unknown. Apply the same node visibility as the snapshot.
2. Append one hub-only positioned event for changes to that optional value. Publish it after the
   link becomes current and clear the value when that link ends. A stale disconnect cannot clear a
   replacement link. A live restart starts with unknown for each node until its current link
   negotiates again. Store only changes to the published subset. If publication fails, consumers
   recover through the existing snapshot or reconnect path. The hub must never advertise values
   recovered solely from a past link.
3. An older client ignores the new fields and event. A new client with an older hub keeps its
   existing action and refusal behavior when the value is unknown. A known unsupported action is
   disabled with an explanation. An offline node follows the existing offline behavior. A known
   capability only means the hub can route that protocol feature to the node. The hub and node still
   check all grants, policy and runtime conditions on every request.
4. Use new field and event tags, with no change to the client handshake. Keep the list canonical,
   sorted and bounded by the hub-owned allow-list. Budget it inside the existing snapshot and
   replay limits. Obtain separate schema approval before implementation.

## Alternatives

- Ask the node at each action. That adds latency and churn to the picker and still needs a refresh
  rule after link replacement.
- Expose the complete handshake or rework the client handshake. The client needs a few scoped hints.
  A machine's complete handshake exposes details the interface does not consume.

## Consequences

Clients can explain known incompatibility before submission. Mapping feature hints to protocol
capability names remains a small, explicit list in the hub and the clients that use it.

## Verification

Use older and newer hubs, nodes and windows. Exercise known empty, unknown, online change, late
disconnect from an old connection, hub restart, hidden node and reconnect from a saved position.
Check snapshot and live convergence, frame limits at fleet size, and normal authorization refusal
when the advertised protocol capability is present. Append protocol fixtures.

## Status of the build

Built (2026-11-28) for `task.dispatch`. Schema: `NodeCapabilities`, `NodeCapabilitiesChanged` (event arm 97, hub only), `NodeSnapshot.capabilities`
(19), fixtures and `crates/agora-protocol/tests/node_capabilities.rs`. Hub: `Hub::node_link_opened` records the published part of what
the link negotiated right after its `NodeLink { up }` (a store failure leaves it unknown); the projection keeps it as
`(generation, names)` and shows it only for the current, online link (`projection.rs` `PUBLISHED_CAPABILITIES`,
`published_capabilities`); a newer link or the link's end clears it, and a hub that starts records every stale link closed, so a
past link's word is never shown. Window: the view keeps it per machine for the current link; `give.ts` `noteCapabilities` takes a
machine known not to speak `task.dispatch` out of "Give to an agent" and "Start a new agent for this task", with the existing words
for an unsupported machine; a machine not known is offered as before and the window still learns from a refusal. One word is stored per link opening: every opening makes the value unknown, so each word is a change from unknown ("store only changes").
A machine whose current link is known to take tasks is offered again even if an older link refused (agora-ac's read of 8f213602). Tests:
`a_nodes_published_capabilities_are_its_current_links_only`, `an_opened_link_publishes_what_it_negotiated_of_the_published_list`,
`apps/web/src/net/nodeCapabilities.test.ts`.
