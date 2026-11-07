# Protocol specification (draft)

Status: draft. Message names are normative once published in `schema/`; this document explains semantics.
Encoding and transport: ADR-0009, ADR-0010, ADR-0011.

## 1. Connection lifecycle

```
connect (WSS + mTLS for nodes, session token for clients)
  → Hello      { role, protocol_generations[], capabilities[], build, locale? }
  ← Welcome    { chosen_generation, capabilities[], peer_id, server_time }
  → Resume     { last_acked_seq? }         (nodes and clients reconnecting)
  ← ResumeOk   { from_seq }  |  ← SnapshotRequired
  ... streams ...
  → / ← Ping / Pong every 15 s; 3 missed = connection lost
  → / ← Goodbye { reason_code }
```

Roles: `node`, `client`, `adapter`. Capability names are lowercase dotted strings (`terminal.raw`,
`terminal.cells`, `acp`, `mesh`, `files.watch`, `debug.json`). Unknown capabilities are ignored.

## 2. Envelope and streams

Every frame: `{ stream_id, kind, seq?, payload }`.

| Stream | Purpose | Ordering | Reliability |
| --- | --- | --- | --- |
| 0 control | handshake, ping, flow control | ordered | reliable |
| 1 commands | request/response RPC (`request_id`) | per request | reliable, idempotency key |
| 2 events | node/hub facts | ordered, `seq` per producer | at-least-once, replayable |
| 3+ terminal | one per subscribed terminal | ordered | best-effort; resync by snapshot |
| 3+ file | file transfer | ordered | reliable, chunked |


WebSocket mapping (ADR-0011, `agora-hub::transport`): one binary message carries exactly one encoded frame; a message
with trailing bytes, a partial frame, or a text message is a protocol error. Messages larger than the payload limit
plus header are refused by the WebSocket layer before they are buffered.

## 3. Events (node → hub, hub → client)

Events are immutable facts. Each producer numbers its events (`seq`); the hub applies an event at most once
per `(producer_id, seq)` and acknowledges the highest contiguous `seq`. Nodes keep unacknowledged events on
disk while disconnected.

Node buffer (`agora-node::event_buffer`): an append-only log of checksummed records plus an atomically replaced ack
file. A torn tail from a crash is truncated on open. After a reconnect the node replays everything after the
hub's ack, in order. The log is size-capped (oldest events dropped first); if unacknowledged events were dropped,
a replay from the old ack is refused (`Stale`) and the hub resynchronizes from a snapshot instead of silently
missing events. Terminal bytes never enter this log.

Hub store (`agora-hub::store`, ADR-0014): SQLite in WAL mode behind an `EventStore` trait. `append(producer, seq, ..)`
returns `Applied` (stored, with a global position), `Duplicate` (already applied, ignored) or `Gap { expected }` (not the
next event; nothing stored, the node replays from `expected`). `acked(producer)` is the acknowledgement to send back.
Clients page the log by global position. Payloads are opaque encoded events.

Hub node session (`agora-hub::session`): a state machine over decoded messages, independent of transport and framing.
The producer id is the node's authenticated identity; an event naming another producer is `protocol.invalid_message`.
Order is `Hello` (role must be node) → `Resume` → events. The hub answers `Resume` with `ResumeOk { from_seq }` where
`from_seq` is one past what the hub already holds, regardless of what the node claims. Each event yields `Applied`,
`Duplicate` or `Gap { expected }`.

Node authentication (`agora-hub::tls`, ADR-0011/0015): the hub's TLS endpoint requires a client certificate issued by
its own CA. The node's id and certificate serial come from that certificate, not from any message; the session's
producer id is that node id. A certificate from another authority, or none, never completes the handshake. Revocation
is checked against the device registry by serial.

Initial event families:

| Family | Examples |
| --- | --- |
| node | `NodeOnline`, `NodeOffline`, `NodeResources` |
| project | `CheckoutDiscovered`, `WorktreeCreated`, `WorktreeRemoved`, `GitStatusChanged` |
| agent | `AgentStarted`, `AgentStateChanged`, `AgentSessionReported`, `AgentUsageReported`, `AgentExited` |
| terminal | `TerminalOpened`, `TerminalTitleChanged`, `TerminalClosed` |
| approval | `ApprovalRequested`, `ApprovalResolved` |
| mesh | `MessagePosted`, `TaskChanged`, `LeaseGranted`, `LeaseReleased`, `HandoffCreated` |
| files | `FilesChanged` (batched paths, from watch) |

## 4. Commands (client → hub → node)

Request/response with `request_id` and `idempotency_key`. Responses are `Ok { result }` or
`Err { code, params }` (see `error-codes.md`).

Initial command set: `project.list`, `agent.start`, `agent.stop`, `agent.prompt`, `agent.set_mode`,
`terminal.open`, `terminal.input`, `terminal.resize`, `terminal.subscribe`, `files.list`, `files.read`,
`files.write`, `files.watch`, `git.diff`, `git.status`, `worktree.create`, `worktree.remove`,
`approval.resolve` (signed, ADR-0020), `mesh.post`, `task.update`, `node.policy.get`.

### Node-local control socket

A node serves the same commands on a local socket (owner-only; `agora node run`, default `$AGORA_HOME/node.sock`)
so it can be driven with no hub. Frames are the envelope above. `CommandRequest` goes on stream 1 with kind 0 and
`CommandResponse` comes back on stream 1 with kind 1, matched by `request_id`; a retry with the same
`idempotency_key` gets the stored response (addressed to the retry) and repeats nothing. A terminal stream is
opened by sending `TerminalSubscribe` on a dynamic stream id (3+) chosen by the client. After the response the
node writes that terminal's frames on that stream: kind 1 snapshot (ANSI that repaints the screen), kind 0 raw
output, kind 2 resize (cols, rows as u16 LE), kind 3 exit (i32 LE; negative = signal). A client that falls behind
gets a fresh snapshot instead of the dropped output; it never slows the terminal. Input and resize are ordinary
commands. Terminal ids are `t-<agent id>` (one terminal per agent). Implemented commands: `project.list`,
`agent.start`, `agent.stop`, `agent.prompt`, `terminal.input`, `terminal.resize`, `terminal.subscribe`; the rest
answer `protocol.capability_missing`. `agent.list` is proposed in ADR-0025.

## 5. Client state model

1. Client connects and receives `Snapshot { as_of_seq, projects, agents, terminals, approvals, ... }`.
2. Client subscribes to events after `as_of_seq` with optional filters (project, node).
3. Terminal output is not part of the snapshot; clients subscribe per terminal and receive a terminal
   snapshot first (format chosen by capability: `terminal.raw` or `terminal.cells`).

## 6. Flow control and priorities

Per-stream credit-based flow control. Priority order: control > approvals > commands > events > terminal >
files. Slow clients get coalesced terminal updates (latest snapshot wins) rather than unbounded buffering.

## 7. Compatibility

See ADR-0006. Hubs accept the current and previous generation (ADR-0019). Every published message has golden
fixtures in binary and JSON form.
