# 0124. Preserve a task claim when its receiver reattaches
Status: Proposed
Date: 2026-11-27

## Context

ADR-0109 binds a durable handoff to an opaque receiver run ID. The terminal holder can keep that exact process alive through
a node restart. The node takes up kept processes before recording `NodeOnline`, then publishes their `AgentStarted` events
with the same run IDs. The board currently releases the old claims as soon as it reads `NodeOnline`. That leaves an open
task even when the live holder process keeps its inbox and subsequently receives the handoff.

The hub's test entry points reproduce this with a pending handoff to `receiver/a`, a `NodeOnline` from the receiver and an
`AgentStarted` for the same agent and run ID with `same_process = true`. The reannounced agent is running with the expected
run ID and the task's owner is empty.

## Proposal

1. Add a bounded, optional list to `NodeOnline` through the append-only schema. Each entry names a node-local agent ID and its
   16-byte run ID. Populate it with holder processes the node actually took up before it records that event. An announcement
   that restates a missed start can list the live runs it is about to reannounce. A restarted or resumed process gets a new
   run ID under ADR-0109.
2. Keep a claim across this restart only when the task still has the revision and owner produced by the pending durable
   handoff. Its recipient's node, agent ID and run ID must match one of the entries and the hub's previous agent and project
   record. Empty evidence, a replaced run, or an ended run follows existing cleanup. This exception applies to the matching
   handoff claim. Ordinary task and lease cleanup keeps its existing contract.
3. Register the reattached agent for board commands only after its ordinary `AgentStarted`. Hold the claim during that gap
   so another agent cannot take the task. Release the claim if subsequent authenticated agent events establish that the
   retained run ended or that a different run took its local ID. Use the task revision to avoid changing later board work.
4. Apply the same rules to a live board and one loaded after a hub restart. Keep the restart evidence recoverable from the
   stored node event. Persist any changed claims together with their board events so replay and the live view agree.
5. Limit the list to 1,024 entries with node-local IDs of at most 256 bytes. Validate the agent and run before retaining a
   claim and ignore duplicate or invalid entries. Do not send the list to windows. Machines can contain several projects
   with different viewers. A node that predates the field supplies no retention evidence and gets existing cleanup.

Waiting for a later `AgentStarted` alone requires deciding when a restart has finished even if the previous run no longer
exists. The startup event can carry the evidence the node already has from the holder, so the hub can reconcile immediately
and use the same evidence on replay.

## Verification

Cover a pending handoff whose receiver is taken up with the same run ID, a replacement process with the same local ID, and
a restart without retention evidence. Verify the matching claim survives a hub restart before a lazy board load. Check that
board calls between `NodeOnline` and `AgentStarted` cannot give the claim to another run and that a later board edit wins.
Run the holder integration with a hub and a disposable node, verifying the receiver's single inbox entry and the task owner
after reconnect. Add new golden fixtures for the startup field.
