# 0026. Event acknowledgement and control/event stream frame kinds
Status: Accepted
Date: 2026-11-07
Accepted: 2026-11-07 (maintainer's explicit approval in chat)

## Context
`docs/specs/protocol.md` §3 says the hub "acknowledges the highest contiguous `seq`" and a node keeps unacknowledged
events on disk until then. The schema has no message that carries that acknowledgement during a session: `ResumeOk`
only answers a `Resume` at connect time. A node that stays connected for days would never trim its buffer. The spec
also does not assign frame `kind` values on stream 0 (control) and stream 2 (events), so two implementations could not
interoperate. The hub session (Phase 3) needs both.

## Decision
Append one message to `schema/agora/v1/events.proto` (append-only, ADR-0006):

- `EventAck { uint64 up_to_seq = 1; }`: sent hub → node on the events stream; every event of that producer with
  `seq <= up_to_seq` is durably stored and may be dropped. Cumulative, so a lost ack is repaired by the next one.

Assign frame kinds (the payload is the named protobuf message):

| Stream | Kind | Message | Direction |
| --- | --- | --- | --- |
| 0 control | 0 | `Hello` | connecting side → peer |
| 0 control | 1 | `Welcome` | peer → connecting side |
| 0 control | 2 | `Resume` | node/client → hub |
| 0 control | 3 | `ResumeOk` | hub → node/client |
| 0 control | 4 | `SnapshotRequired` | either |
| 0 control | 5 | `Ping` | either |
| 0 control | 6 | `Pong` | either |
| 2 events | 0 | `Event` | producer → hub (node), hub → client |
| 2 events | 1 | `EventAck` | hub → producer |

The hub sends an `EventAck` after it has committed a batch, not per event, and at least every 5 seconds while events
flow. A gap (`Append::Gap`) is answered with `ResumeOk { from_seq = expected }` so the node replays from there.
Golden fixtures are added for `EventAck`.

## Consequences
- Nodes can bound their on-disk buffer during long sessions; the hub's acknowledgement is explicit and testable.
- The frame-kind table becomes part of the published protocol and is fixed once accepted.
- Until this ADR is accepted, the hub session handles the handshake and ingests events, but does not send acks.
