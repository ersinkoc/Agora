# 0054. A node that lost unacknowledged events resynchronizes instead of stalling
Status: Proposed
Date: 2026-11-13

## Context
A node keeps its events in a size-capped buffer until the hub acknowledges them (ADR-0026, `docs/specs/protocol.md` §3).
When the cap drops events the hub never stored, the spec says a replay from the old ack "is refused (`Stale`) and the hub
resynchronizes from a snapshot instead of silently missing events". That path does not exist on either side:

- The node (`agora-node::uplink`) answers `ResumeOk { from_seq }` with a local `Stale` error when `from_seq` is older than
  its oldest retained event. `uplink_client` treats that as a failed connection and retries with backoff, forever.
- The hub (`agora-hub::session`, `store`) answers every reconnect with the same `from_seq` (its ack + 1) and stores only
  that exact next number (`Append::Gap` otherwise). Nothing in the hub handles `SnapshotRequired`.

The state is absorbing: from then on no event from that machine reaches the hub (agent starts, state changes, exits,
approvals), while the machine keeps running agents. The only recovery is deleting the node's buffer by hand. It was
reproduced in the evidence audit as finding F080 (the hub held 3 of 70 events after 6 reconnects; all 6 ended in `Stale`).

Two ordinary situations trigger it:

1. **An offline burst larger than the cap** (64 MiB by default): a machine that is cut off from the hub for long enough.
2. **A hub restored from a backup** (`agora hub restore`) that is older than the point every node has since trimmed to.
   This hits every node at once; `crates/agora/tests/hub_link.rs` already expects `Stale` here and relies on the missing
   resync.

ADR-0026 lists `SnapshotRequired` (control stream 0, kind 4) as "either" direction, but defines only what it means from
the hub. Its schema comment is direction-neutral: "The sender cannot replay from the requested point; the peer must fetch a
fresh snapshot."

## Decision
Define the node → hub direction of `SnapshotRequired` and a resync that reuses existing messages. **No schema change.**

### 1. The node says it cannot replay
After `ResumeOk { from_seq }` (or a mid-session gap answer with `from_seq`), if `from_seq` is older than the oldest event
the node still has, the node:

1. appends a **resync announcement** to its own buffer (section 3), so it gets the next numbers;
2. sends `SnapshotRequired` (stream 0, kind 4, empty) to the hub;
3. streams from its oldest retained event onwards, as in a normal replay.

`Stale` stops being an error that ends the connection. A node never sends `SnapshotRequired` when it can replay.

### 2. The hub moves this producer's next number forward, once
On `SnapshotRequired` from an authenticated node in the `Ready` phase, the hub session accepts **the next event from that
node** as the new base for its producer stream, provided its `seq` is greater than the hub's ack. Then:

- the store records the move in one transaction with that event: `last_seq` jumps to `seq - 1` and the skipped range
  (`from_seq ..= seq - 1`, producer, time) goes into a `resyncs` table (internal storage, not protocol);
- the hub logs it, and in team mode writes an audit entry (`node.resynced`, ADR-0049), so the loss is recorded and
  never silent;
- the numbers never move backwards. A `seq` at or below the ack is a `Duplicate` as today. A `SnapshotRequired` that is
  not followed by such an event changes nothing. Every later event follows the normal `Applied`/`Duplicate`/`Gap` rules.

A node can only skip numbers in its own producer stream (its id is its certificate identity). That is no new power: a
node can already withhold its own events.

### 3. The resync announcement: what the node says now
The hub's view of a node is built by replaying that node's events. The ones in the lost range are gone, so the view could
show agents that have since exited, or miss ones that started. The node therefore re-states its present, using messages
that exist and that the projection already treats as "replace":

1. `NodeOnline`. The projection already reads it as "what this node said it was running before is gone": its running
   agents end and its waiting approvals are dropped. This ADR adds that its terminals are dropped too, which is also the
   right reading after a real node restart.
2. For each agent that is running now: `AgentStarted`, `AgentStateChanged` with its current state, and where they apply
   `AgentSessionReported`, `AgentWorkspaceBound` and `TerminalOpened`. The projection inserts by key, so these replace
   the stale entries.
3. `CheckoutDiscovered` and `WorkspaceRegistered` for what the node knows, as the announcement after an empty data
   directory (`catch_up_with_hub`) already does for checkouts.
4. `ApprovalRequested` for every approval still waiting on the node.

The announcement goes at the tail of the buffer, after the retained events. So the hub first applies those older retained
events and then the announcement, and ends with the present state whatever the retained events said.

### 4. Specs and tests
`docs/specs/protocol.md` §1, §3 and "Node uplink" are corrected to describe this. Tests:

- the audit's `prove_F080` becomes a regression test: after a cap overflow the hub receives the node's newer events;
- `a_hub_restored_from_an_older_backup_gets_the_gap_replayed` expects the resync instead of `Stale`;
- a projection test: a resync announcement ends exited agents and terminals and keeps running ones;
- a session test: a jump is accepted only right after `SnapshotRequired`, only forwards, and only once.

## Alternatives considered
- **The node renumbers its kept events down to the hub's `from_seq`** (node only, no protocol change). Rejected: the hub
  never learns that events were lost, which the spec forbids. And `(producer, seq)` stops naming one event: a hub restored
  from an older backup would hold a different event under the same number.
- **A new message `ResyncFrom { next_seq }` (control kind 7)** instead of reusing `SnapshotRequired`. It is more explicit,
  because the hub would check the next event against `next_seq`. But it is a schema addition with fixtures for what ADR-0026
  already reserved. Reasonable if the maintainer prefers an explicit number.
- **A new `NodeResynced` event** instead of reading `NodeOnline` as the reset. It is clearer in the history, but it is a
  schema addition, and the projection result would be the same.
- **A hub command that asks the node for a full state dump.** It needs a second state format next to the events the
  projection already understands.
- **An unbounded buffer.** It trades a stalled link for a full disk; ADR-0022 and the node-autonomy invariant want the
  bound.

## Consequences
- A machine that was offline too long, or every machine after a hub restore, reconnects on its own and the hub shows its
  real present state. Nobody has to delete a buffer by hand.
- The lost events stay lost: history, usage and cost for that window are missing, and agents that started and ended inside
  it never appear. The gap is recorded (store, log, audit) rather than hidden.
- Agents that kept running show a short end-and-start in clients during the announcement. Times that come from
  `AgentStarted` reflect the resync, not the original start.
- `NodeOnline` drops the node's terminals as well as its agents and approvals. That is a projection change, not a schema
  change, and it matches what a node restart means.
- `uplink_client` stops retrying a `Stale` connection forever. It now has a terminal recovery path that crates/agora tests
  cover.
- Until this is accepted, nodes keep stalling in both situations above; the condition is logged (557c743) but not
  repaired.
