# 0121. Show task handoff delivery on the project board
Status: Accepted
Date: 2026-11-27

Accepted by the maintainer on 2026-11-27, including the append-only delivery status protocol.

## Context

ADR-0109 makes the queue durable. A change of task owner and a receipt from the receiving machine are separate facts. A lost response leaves a handoff pending and expiry leaves the outcome unresolved. The board receives `TaskChanged` today, which carries task state and owner.

## Decision

- Add a hub-produced handoff delivery event and a bounded snapshot section through the append-only protocol. Identify a delivery by its opaque delivery id, project, task and receiving agent. Carry delivery state and a timestamp. Keep the delivery summary and refs in the existing message.
- Use pending when the transaction queues delivery, delivered when the receiver returns a matching receipt, failed when it gives a definitive refusal, and unresolved when the deadline passes without a definitive outcome. Show a stable error code for a definitive refusal when available. Leave task ownership derived from existing board events.
- Persist a status change in the same transaction as any task compensation. Scope events and snapshots to people who can see the project and the handoff under team rules. Leave older clients and best-effort receivers compatible.
- Let an authorized operator acknowledge an unresolved outcome and free its queue reservation without retrying or changing ownership. Keep a bounded history of finished delivery outcomes so a reopened window can explain what happened. Choose the retention limit and authorize the reconciliation command before implementation.

### Wire and access rules

The hub emits `TaskHandoffDeliveryChanged` and includes recent delivery states in the snapshot. The snapshot keeps active
deliveries (up to the existing ADR-0109 queue limit) and up to 256 settled deliveries, newest by hub event sequence.
During snapshot serialization delivery states fit within the frame ceiling before optional recent messages. Active rows that do
not fit follow as positioned events at the snapshot cursor.
The shared board is owner-only under current team rules. Delivery events and snapshot rows have that same visibility.

`AcknowledgeTaskHandoff` is a hub-only command for the signed-in owner. It accepts the opaque delivery id and rejects a
target machine. It only frees a reservation after the delivery is unresolved. Acknowledgment preserves the unresolved
outcome in history, never changes task ownership, and is idempotent while the history row is retained.

## Verification

Restart the hub while a delivery is pending. Drop a receipt after the node stores the entry and check that a retry produces one delivered outcome. Cover definitive refusal after a later task mutation and unresolved expiry. Verify owner, member and viewer visibility and reconciliation rights, and pin new protocol fixtures. Run the workflow on two machines and check the board in both themes.
