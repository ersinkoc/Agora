# 0074. A reply keeps its link to the message it answers, across machines
Status: Proposed
Date: 2026-11-16
Origin: the real-machine mesh round trip (macOS and Windows agents, 2026-11-16): `mac/agent-2` sent `win/agent-5` a ping and got the pong back in 607 ms, but the pong's `reply_to` was `null`. The Windows agent confirmed it is not the responder: on one machine the same pong carries `reply_to` (m-6 → m-7). Across machines the link is lost at the uplink.

## Context
ADR-0031 routes `send_message` to `<node>/<agent>` through the hub. `MeshPost` carries `to`, `body`, `origin`, `from_agent` and `refs`; it has no `reply_to` and no id of the message itself. Message ids (`m-N`) are per node: the receiving node numbers the message in its own inbox (win calls mac's ping `m-11`, mac knows it under its own id), so even a `reply_to` field alone could not name the right message. Today the spec's `reply_to` therefore holds only within one node; an agent that wants to correlate a remote reply must put its own token in the body. `delivery_failed` already works across machines because the failing node answers with the id the sender used.

## Decision
1. **Add two optional fields to `MeshPost`** (append-only, new numbers; older nodes ignore them, a post without them behaves as today): `message_id`, the sender node's id for this message (a node can only vouch for its own ids, like `from_agent`), and `reply_to` + `reply_to_node`, naming the message this one answers as the answering node knows it: the **original sender's** id and node. The receiving node stores `message_id` with the inbox item as the message's remote id.
2. **Inbox items carry `reply_to` when the answered message was sent from the receiving node:** if `reply_to_node` is the receiving node, `reply_to` is the id that node gave the original message and is shown as before (`m-6`); if it names another node, the item shows it as `<node>:<id>` (the same shape as `refs`), so an agent can tell a local id from a remote one.
3. **The hub overwrites `origin` as today** and also rejects a `message_id` or `reply_to` longer than the id length limit; it never invents or rewrites them. Mesh messages stay untrusted data (AGENTS.md): `reply_to` is a hint for correlation and never grants anything; a reply naming a message the receiver never sent is accepted as a plain message with `reply_to` dropped.
4. **`docs/specs/mesh-tools.md`** says that, until this lands, `reply_to` holds only within one node (one line, now), and describes the cross-node form after.

## Consequences
- Two optional fields on one message, a golden fixture added (never edited), `docs/specs/protocol.md` and the mesh spec updated; an amendment of ADR-0031's message shape only.
- Agents (and the web's conversation view) can follow a thread across machines.
- Cost: a few bytes per cross-node message; one id map per node, bounded like the inbox.
## Alternatives considered
- **`reply_to` alone:** names an id the receiver does not know. Rejected.
- **Global message ids (hub-assigned):** the hub would have to number every message and a node could no longer answer offline. Rejected (nodes are autonomous).
- **Leave it in the body:** works for a model, not for a client that draws threads. Rejected for the product, kept as today's workaround.
## Open questions
- Should the web's conversation view group by `reply_to`? Proposed: later, a separate change.
