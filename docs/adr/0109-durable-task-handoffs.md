# 0109. Recover a cross-node task handoff after a hub restart
Status: Accepted
Date: 2026-11-23

Accepted: 2026-11-24 (the maintainer, in the merger chat: "Accept and build").

## Context

ADR-0039 puts the task owner change and its message to the receiver in one hub transaction. The hub already stores the board and
its events together. It forwards the message to the receiver after that transaction and compensates when the live attempt fails.
The forwarding work is in memory. If the hub stops after commit and before forwarding, reopening its store shows the new owner
with no message queued for the receiver. Issue #1125 records this boundary.

The current command idempotency cache on a node is bounded to the node process. It can help with a retry while that node is
running. It cannot establish whether a previous incarnation accepted a message. Ordinary cross-node `MeshPost` delivery is at
most once under ADR-0031. Cross-node handoffs that change a shared task need their own recovery contract. An agent's local id
may also be reused by another run. Recovering work for an old run must not deliver its message to the new one.

## Decision

1. The hub creates a unique delivery id for a cross-node handoff of a named task. It persists the pending delivery with the board
   change and board events in the same SQLite transaction. The record contains the project, task, original and intended owners,
   the receiver's node and run identity, the summary and refs, and the information used to bound delivery attempts. Failed writes
   preserve the earlier owner and leave nothing to send. The hub keeps ownership of the queue, including across backup and restore.
2. A new, capability-gated node command delivers a pending handoff using its delivery id. The command names the intended receiver
   run. A live process retains its run identity through holder reattachment. A fresh run or resume gets a new identity, even when
   it reuses a local agent id. The hub gets this identity from the node through an additive protocol change. At delivery the node
   verifies that the receiver is the addressed run and its project still matches. Summary and refs remain untrusted data.
3. The node acknowledges acceptance only after it durably records one inbox entry with that id. Delivery and the deduplication
   record share a transaction or an equivalent atomic write. If the receiver run is kept across a node restart, unread entries are
   restored to that run. A retry returns the receipt for the same entry, including after the entry was read, without creating
   another entry. Retain receipts for the full retry window. The guarantee is one durable inbox entry per delivery id. A caller
   that did not receive its own inbox read response has the normal read semantics of that tool.
4. The hub resumes pending sends when it opens the store and when the correct receiver reconnects. Attempts for a delivery id
   are serialized, bounded and subject to backoff. A lost acknowledgement keeps the entry pending. A definitive rejection may
   restore the original owner only if the task is still in the exact state left by this handoff. The rollback and its event
   updates share one transaction. A timeout alone cannot prove the receiver failed to accept. When the retry window expires
   without a receipt, leave the outcome visible for reconciliation and do not undo a later board operation.
5. A receiver that is already offline when the handoff is requested is refused before the task changes under ADR-0039. Older nodes continue
   with the existing best-effort handoff path during upgrade. The hub does not retry those deliveries. Local handoffs, handoffs
   without a task, ordinary messages and channel fanout keep their current contracts.
6. Keep bounds for pending deliveries, bytes and receipts. When the hub cannot reserve space for a tracked delivery, it refuses
   before changing the board. At the node, a full inbox that cannot accept the entry returns a definitive refusal without a receipt.
   Pick byte, count and retry window limits before shipping the schema slice. A hub outage does not stop an agent or its local tools.

## Alternatives considered

- A hub-only SQLite queue plus the existing command idempotency key. This recovers the pre-send gap while the receiver process
  remains alive. It leaves duplicates possible after a receiver restart or cache eviction.
- Move task ownership only after an external acknowledgement. This moves the partial failure window to the other side. A receiver
  can receive the handoff while the hub still has the giver as owner. Recovery still needs durable coordination and receipts.
- Continue best effort for all nodes. This is compatible with ordinary messages, but it leaves the task de-synchronized from its
  handoff after the hub stops at the boundary in #1125.

## Build order

1. Add the capability and append-only schema for the run identity, delivery and receipt. Pin old-peer behavior and fixtures.
2. Add node persistence and run binding with bounded unread inbox restoration and receipt retention.
3. Add hub transactional queue, replay, definitive compensation and observable unresolved outcomes. Use this path only after the
   receiver announces support.

## Verification

Use gated integration tests for a stop after commit and before send, acceptance followed by a lost response, and a node restart
before the hub retries. Verify one durable receiver entry and the corresponding owner after recovery. Cover receiver id reuse,
definitive refusal, a terminal board transition before compensation, store refusal, queue limits, and a legacy receiver. The
original deterministic #1125 proof checks the pre-send boundary without timing assumptions.

## Open questions

- Confirm legacy fallback during rollout versus requiring the capability for cross-node handoffs of a named task.
- Confirm what a still-connected giver sees when the outcome remains unresolved beyond its tool timeout.
- Set the retention and retry limits (answered by Amendment A) and decide where a person reads and reconciles an unresolved
  handoff.

## Amendment A (accepted by the maintainer 2026-11-24, in the merger chat): the retry window and the bounds

This answers the limits named in decision 6 and the open question above; the rest of the decision is unchanged.

- The hub sets a delivery's `valid_until_ms` to 24 hours after it creates the delivery and stops retrying at that deadline. The node
  keeps receipts through the deadline and accepts a new entry only before it.
- Per hub: at most 1,024 pending or unresolved deliveries and 32 MiB of encoded delivery data. Per node: at most 1,024 durable
  handoff records and 32 MiB of encoded inbox entries and receipts.
- A summary is at most 16 KiB of UTF-8, with at most 20 refs totalling 16 KiB (the existing mesh limits). The encoded command fits
  the negotiated frame limit.

The wire details are in [the protocol spec](../specs/protocol.md#durable-cross-node-task-handoffs-adr-0109).
