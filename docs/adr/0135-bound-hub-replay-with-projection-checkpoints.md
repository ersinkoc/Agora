# 0135. Bound hub replay with projection checkpoints
Status: Accepted
Date: 2026-11-27
Accepted: 2026-11-27 (maintainer approval in chat).

## Context

The hub rebuilds its view from the SQLite event log. The November 14 benchmark crossed the
500 ms cold-start budget near 900,000 events on that fixture. Usage aggregation subsequently reduced
the dominant event source, so that crossing is a historical baseline. The hub needs current
measurements before adding another persisted representation. The privacy policy also calls for
configurable retention. Pruning changes what can be replayed and what a restored hub can recover.

## Decision

1. Rerun the cold-start benchmark against representative, sanitized event mixes with current usage
   aggregation, including shared boards, notices and many nodes. Build projection checkpoints when
   the current reference fixture exceeds the 500 ms budget at its documented history horizon.
   Record fixture diversity, event counts and database size. If the budget holds, keep full replay
   and retain the benchmark for future reviews.
2. A checkpoint is a versioned, validated serialization of the durable hub projection at a committed
   event position. Persist the projection and position atomically in SQLite. Restore the newest
   compatible checkpoint, then replay events strictly after its position. Exclude connection state,
   negotiated capabilities, live streams and process handles. The next current link or snapshot
   supplies live facts as it does today. Keep the latest two verified checkpoints and a bounded
   write schedule. A corrupt or unsupported checkpoint falls back to the older verified checkpoint
   and its tail, then to full replay when the complete log exists.
3. Ship checkpointing without pruning first. In a later retention step, start with an owner dry
   run that reports the proposed cutoff, recoverable range, excluded data and estimated bytes.
   Keep event pruning off by default. An owner can set a minimum age for an eligible event prefix.
   Remove only a contiguous prefix covered by both verified checkpoints. Require a validated backup
   covering that prefix under the owner's retention policy. Keep positions monotonic and never
   renumber retained events. If a protected record blocks the prefix, report it and keep the data.
4. Protect audit records, revocations and device or approval authority, board and delivery state,
   and any records still needed for deduplication or restore. Define the records eligible for
   pruning against the schema and their consumers before the retention step ships. A policy for
   deleting audit or mesh content has its own privacy and authorization decision. If both
   checkpoints become unusable after pruning, require recovery from a verified backup and surface
   the condition. Do not produce an empty view from the remaining tail.
5. A subscriber behind the prune floor receives the existing snapshot-required behavior. Node
   acknowledgements and sequence deduplication remain independent of the client replay floor.
   Restore brings back the backup's checkpoint, event floor and tail as one consistent image.
   A checkpoint or cutoff that an older hub cannot restore requires a store version that the older
   hub refuses. Keep checkpointing and pruning compatible with the staged backup and takeover rules
   in ADR-0086. No wire additions are proposed for this decision.

## Alternatives

- Keep full replay and reduce event volume at each high-volume producer. Keep this as the baseline
  and use it while the measured budget holds.
- Unconditionally prune by age. This bounds storage but can change a reconstructed view or force
  recovery without the facts needed for it.
- Move to a different database. That would not remove the replay and recovery decisions and would
  reopen the database choice in ADR-0014 and ADR-0049.

## Consequences

The checkpoint tracks the projection format and its restore tests. Compaction is a later opt-in
step with explicit data-loss boundaries and a compatible backup. This decision preserves the current
targets for cold start and the separate restore drill.

## Verification

Prove full-replay and checkpoint-plus-tail equivalence across migrations, reconnects, lost node
history, review, pending delivery, forget and revocation. Stop the hub before and after a checkpoint
or pruning transaction, corrupt the latest checkpoint and restore an older backup. Compare cold
start with and without a checkpoint and measure write overhead during representative ingest.
Before shipping pruning, test a client on each side of the floor, an offline node that reconnects,
protected rows, both checkpoints corrupt, and backup restore with a maximum five-second recovery
target for reachable nodes after the restored hub starts.
