# MCP ask ownership on disconnect (#366)

The F063 fix in #363 preserves an answered ask until its caller polls it. The node's MCP server previously dropped its
waiting list on transport EOF without canceling broker asks. A long-lived agent could reconnect and leave more completed
answers behind. They neither counted toward the pending-ask limit nor expired while the agent remained registered.

An independent review proof created 2,048 MCP asks, discarded their originating connections before polling, and answered
them from a peer. With default limits, one ask/reply per 60,001 ms and the audit events drained, all 2,048 answers remained;
the first deadline was 1,000 ms and the final logical time was 122,882,048 ms. Restoring only the earlier cleanup predicate
left zero completed records, but that predicate also loses replies that a live caller has not consumed yet.

## Change

`Mesh::cancel_ask` removes a pending or answered ask only for its authenticated originator. The pure broker adds no clock,
socket or runtime state. MCP transports must cancel their asks when they can no longer deliver the result.

The node keeps a per-connection set of owned ask IDs. This set is separate from the temporary queue drained while polling:
a failed response write must still release both earlier pending items and later items the poll never reached. Final
responses remove their IDs from ownership after `ask_status` consumes the record. The ownership guard cancels every
remaining ID when the connection exits, including EOF, read/write errors, oversized input and shutdown.

This preserves normal delayed polling, the current request deadlines, all configured limits, another connection of the
same agent, and the agent's registration. Request/reply history still uses the existing bounded inbox. No dependency,
schema, wire message, policy or approval behavior changes. No installed service, credential or real agent is used.

## Regression checks

- The local-socket test failed before the fix: an abandoned request remained pending after EOF while its agent stayed
  registered. It passes after the fix through eight reconnect cycles. A second connection of the same agent keeps its
  own request and consumes its reply normally; late replies cannot resurrect canceled asks. A query at logical time zero
  proves disappearance rather than ordinary deadline expiry.
- A unit check discards an already completed, unpolled answer when its owning connection ends, preserving a different
  connection's pending ask.
- An injected broken-pipe error during the real polling helper checks cleanup of a previously retained pending item,
  the completed item, and an unvisited later item. A different connection's request still works.
- The broker refuses cancellation by the peer and permits the originator to cancel both pending and answered asks.
- The existing F063 delayed-answer regression remains enabled.

```sh
cargo test -p agora-node --test mesh_ask_lifetime
cargo test -p agora-node mesh_server::tests
cargo test -p agora-mesh
```

The new filesystem fixture is created exclusively before cleanup ownership is established. Existing paths are retried,
never adopted or removed. Tests use the platform's local socket abstraction, including named pipes on Windows, and no
operating-system conditionals outside the platform crate.
