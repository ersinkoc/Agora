# Protocol specification (draft)

Status: draft. Message names are normative once published in `schema/`; this document explains semantics.
Encoding and transport: ADR-0009, ADR-0010, ADR-0011.

## 1. Connection lifecycle

```
connect (WSS + mTLS for nodes, session token for clients)
  → Hello      { role, protocol_generations[], capabilities[], build, locale? }
  ← Welcome    { chosen_generation, capabilities[], peer_id, server_time, person_role }
  → Resume     { last_acked_seq? }         (nodes and clients reconnecting)
  ← ResumeOk   { from_seq }  |  ← SnapshotRequired
  → SnapshotRequired                       (node only, when it no longer has from_seq; ADR-0054)
  ... streams ...
  → / ← Ping / Pong: a node pings every 10 s (15 s before ADR-0070), a window every 15 s; the hub answers each Ping at once
           with a Pong carrying the same payload. Deadlines (ADR-0070, docs/design/observability.md): the hub closes a node
           connection silent for 25 s; a node drops a hub link silent for 25 s; a window one with no answer within 10 s
  → / ← Goodbye { reason_code }
```

`SnapshotRequired` (control stream 0, kind 4, empty payload) is direction-neutral (ADR-0026): the sender cannot replay from
the requested point. From a node it means the events from the hub's `from_seq` up to its oldest retained event are gone; the
node sends it before any further event and the hub takes the node's next new event as the new base (section 3, ADR-0054).

**A window's connection** (the web window, the terminal client; ADR-0106). A window sends no `Hello`: it opens with
`Subscribe` on the events stream ("Hub service" below). Since ADR-0106 the hub's first frame to it is an unasked `Welcome` on stream 0, before
the snapshot or any event, whose `person_role` (`PersonRole`: `OWNER`, `MEMBER`, `VIEWER`) is the role of the person the window signed
in as (ADR-0049), from the same access the hub checks every command against. A hub not in team mode sends `OWNER`. Nothing was
negotiated, so `chosen_generation` is the hub's generation (1), `capabilities` is empty and `peer_id` is empty. The value holds for the
connection: the hub ends a connection whose role or grants changed (checked once a second and on every command), and the window
reconnects into a new `Welcome`; no event carries a role change. It is a hint for what the window shows and asks, never a permission:
the hub still checks every command. A window that gets something else first is talking to an older hub (`PERSON_ROLE_UNSPECIFIED`) and
keeps its earlier behaviour (learning the role from a refused `PairStatus`); an older window ignores the frame, as it ignores every
control frame it does not use. A node's `Welcome` (the answer to its `Hello`) leaves `person_role` unspecified.

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
hub's ack, in order. One log holds the events of one producer, named in the `producer` file beside it: when the node starts under
another id than that file names, or (for a log from before the file) any retained event, acknowledged or not, names another
producer (it ran as `local` before it was enrolled, was enrolled again under a new id, or runs un-enrolled again), it never
replays them. It renames the log to
`events.<old id>.<unix seconds>.log` (and its ack file to `.ack`) in the same folder, never deleting it, starts a fresh log
numbered from 1, and says so in its output (`cli-node-events-set-aside`). The hub would refuse them anyway (section 3, hub node
session). The log is size-capped (oldest events dropped first). If the hub asks for events the node no longer
has (unacknowledged events dropped by the cap, or a hub restored from a backup older than what the node has trimmed), the
node resynchronizes instead of stalling (ADR-0054): it appends a resync announcement to its buffer, sends
`SnapshotRequired`, and replays from its oldest retained event. The announcement states facts (ADR-0055): the
`CheckoutDiscovered` and `WorkspaceRegistered` it knows; for each agent running now `AgentWorkspaceBound` where it applies,
`AgentStarted`, `AgentStateChanged` with its current state, `AgentSessionReported` when known and `TerminalOpened` when it has
one; for each agent of this process that has ended `AgentExited` with the last exit code the node recorded for it
(`i32::MIN` when none was) and `TerminalClosed` when it had a terminal; `ApprovalResolved` again for the last 256 approvals
resolved in this process; then `ApprovalRequested` for every approval still waiting. It begins with `NodeOnline` only when
the node's last `NodeOnline` (its process start, or an earlier announcement's) is in the lost range, at or after `from_seq`
and before the oldest retained event; then the agents before it really died, and the announcement names only what runs now
(no ended agents, no resolved approvals), as after a restart. If the hub has that `NodeOnline`, or the replay delivers it,
the hub already reset what came before it, so the announcement leaves what the hub knows of running agents in place: their
view never shows them as ended, and their shared-board tasks and leases stay theirs. If the announcement itself pushes that
`NodeOnline` out of the buffer under the cap, a full announcement led by `NodeOnline` follows it. The hub applies the
retained events and then the announcement, so it ends at the node's present state; the re-stated `AgentExited`,
`TerminalClosed` and `ApprovalResolved` change nothing the hub already had, and the board's next call releases the work of
agents that ended. The lost events stay lost and are recorded as such (store, log, audit trail), never silently. Limits: a
managed working copy created inside the lost window (`WorktreeCreated`) stays missing from the hub's list, and resource
reports and git status are not re-announced (the next report or change brings them back); the hub's usage counters for
agents that still run start again from the announcement. Terminal bytes never enter this log.

Hub store (`agora-hub::store`, ADR-0014): SQLite in WAL mode behind an `EventStore` trait. `append(producer, seq, ..)`
returns `Applied` (stored, with a global position), `Duplicate` (already applied, ignored) or `Gap { expected }` (not the
next event; nothing stored, the node replays from `expected`). `append_resync(.., resync_at_ms)` is the same for a node's first
event after `SnapshotRequired`, except that an event above the next number is stored too: in one transaction the producer's
numbering moves to it and the skipped range `(producer, from_seq, to_seq, at_ms)` is written to the internal `resyncs` table
(`Resynced { pos, from_seq, to_seq }`). It never moves backwards: at or below the ack is a `Duplicate`. `acked(producer)` is
the acknowledgement to send back. Clients page the log by global position. Payloads are opaque encoded events.

Hub node session (`agora-hub::session`): a state machine over decoded messages, independent of transport and framing.
The producer id is the node's authenticated identity; an event naming another producer is `protocol.producer_mismatch`
(`node_id`, `producer_id`).
An event whose body names a node (its `node_id`) must name that producer, or it is `protocol.invalid_message` (field `node_id`);
`NodeLink`, `NodeForgotten` and `AgentForgotten` are written by the hub only, and one from a node is `protocol.invalid_message`
(field `node_link`, `node_forgotten`, `agent_forgotten`). Threat model T19.
When the hub ends a node's connection because the node broke the protocol (such as
`protocol.producer_mismatch`), it first sends a WebSocket close (code 1008) whose reason is the stable error code, logs
`node refused: <node> <code> <params>`, and reads until the node's side of the close (at most 5 s) so the reason is not lost
to a reset; the node's link reports it as `Refused(<code>)` in its log. A close reason is transport framing, not a schema message:
a peer that does not look at it sees an ordinary close. Order is `Hello` (role must be node) → `Resume` → events. The hub answers `Resume` with `ResumeOk { from_seq }` where
`from_seq` is one past what the hub already holds, regardless of what the node claims. Each event yields `Applied`,
`Duplicate` or `Gap { expected }`. After the handshake the node may send `SnapshotRequired` (before it is
`protocol.invalid_message`): then its next event with a `seq` above the hub's ack becomes the new base, once, and yields
`Resynced` (or `Applied` when it is the next number anyway). Events at or below the ack stay `Duplicate` and do not use it up;
a `SnapshotRequired` not followed by such an event changes nothing, and it does not carry over to the next connection. The
hub logs a resync (`node resync: <node> skipped <from>..=<to>`) and writes an audit row (`who` = `hub`, action
`node.resynced`, the node, outcome `<from>..=<to>`). `NodeOnline` in the hub's view ends that node's running agents and drops
its waiting approvals and its terminals; a resync announcement then re-adds what still runs. A node that takes up agents its terminal
holder kept (ADR-0079, ADR-0093) re-adds each right after its start `NodeOnline` the same way (`AgentStarted`, `AgentStateChanged`, the
session it knew, `TerminalOpened` for a terminal agent; `docs/specs/holder-link.md`). Both say `AgentStarted.same_process`, so the hub
keeps the agent's counters (section "A kept agent's counters", ADR-0104).

SSH stdio transport (ADR-0011): `agora node stdio` copies the node's local-socket frames to stdin/stdout, so a person or a
tool can reach a node with `ssh host agora node stdio` and no open port. Frames are exactly those of the local control
socket, so whoever speaks them has the owner's own authority there (the routed-command checks, grants included, apply to the
hub's connection only). It is not the hub's path (ADR-0032: the hub reaches an SSH-only node through a tunnel to its own TLS
port); nothing depends on the hub.
Node authentication (`agora-hub::tls`, ADR-0011/0015): the hub's TLS endpoint requires a client certificate issued by
its own CA. The node's id and certificate serial come from that certificate, not from any message; the session's
producer id is that node id. A certificate from another authority, or none, never completes the handshake. Revocation
is checked against the device registry by serial.

Newly issued hub-server and node-client certificates include a non-critical Authority Key Identifier matching the existing CA's
Subject Key Identifier, including a custom identifier in a loaded CA. This supports strict X.509 chain validation
([RFC 5280 §4.2.1.1](https://www.rfc-editor.org/rfc/rfc5280.html#section-4.2.1.1)). CA certificates/keys, node identities,
server names, authentication purposes and revocation checks are unchanged; existing certificates remain accepted by the hub.

Events a node emits (`agora-node::daemon::EventSink`): `NodeOnline` at start (and at the head of a resync announcement when
the hub lost that start, section 3) and `NodeOffline` on a clean stop; a
`CheckoutDiscovered` per registered project; `NodeResources` when the running count changes (not on a timer, so an idle
node is silent; `memory_used_bytes` is the resident memory of every running agent's process tree, measured then and every 30 s
while agents run, and sent again only when it moved by more than a quarter or 256 MiB since the last report); and for each agent `WorktreeCreated`, `AgentStarted`,
`TerminalOpened` in that order, then `AgentStateChanged` as the screen changes, and on the end `AgentExited` and
`TerminalClosed` (once). A stop on request reports exit code `i32::MIN` (unknown), the same convention as the terminal
exit frame. A reader that folds events into a view therefore always sees a workspace before the agent that uses it.

Hub-wide ids: ids in events are local to the machine that made them (`agent-1` exists on every node). The hub's view keys
every entity by `<producer>/<local id>` (`laptop/agent-1`; node ids are limited to `[A-Za-z0-9._-]`, so the split is
unambiguous) and uses that key in what it shows clients and when it routes a command back to the owning node.

Node uplink (`agora-node::uplink`): the node's state machine mirrors the hub's session: `Hello` → `Welcome` → `Resume` (carries
the node's own idea of the ack) → `ResumeOk { from_seq }` (the hub's record decides) → events in batches from the durable buffer.
A gap from the hub rewinds to `expected`. When `from_seq` (from `ResumeOk` or a gap answer) is older than the buffer keeps, the
uplink returns `Replay::Resync` instead of `Replay::Resumed`: it has appended the resync announcement (section 3) and moved its
cursor to just before the oldest retained event, and `uplink_client` sends `SnapshotRequired` before the next event. A later
gap answer for the same point was sent by the hub before it read `SnapshotRequired` and needs no second announcement; at most
one such answer per event sent before the resync is accepted, and one more ends the connection as `Stale` (a hub from before
ADR-0054 ignores `SnapshotRequired`; the node then retries with backoff as before). `Stale` also ends a connection whose next
events the cap dropped while it was sending; the next connection resynchronizes. The hub's ack trims the buffer.

Hub core (`agora-hub::hub`): store, view and subscriber fan-out are one step under one lock, so a subscriber that takes its
snapshot (or backlog) and registers for live events together sees every event once, in order, even while several nodes
ingest. A subscriber that falls more than 1024 events behind is dropped, never waited for; it reconnects with the last
position it saw and gets the backlog, or a snapshot when that position is ahead of the log (a hub restored from an older
backup) or more than 10 000 events or 8 MiB of them behind (`MAX_REPLAY_EVENTS`, `MAX_REPLAY_BYTES`): replay runs under the hub's
lock, so its cost is bounded instead of growing with how long the client was away (a client 100 000 events behind held the lock for
about 155 ms before; its snapshot now takes well under a millisecond). When a node's connection ends without a goodbye the hub records `NodeOffline` itself as producer `hub` (a name no
node certificate can carry), in the same log, so live and rebuilt views agree.

Presence (ADR-0070): the hub also records `NodeLink { node_id, up, reason, generation, last_heard_ms }` as producer `hub`, from
the certificate-authenticated connection only: `up` with reason `opened` when a node's connection is open after the handshake
(a first connect or a reconnect; it resets nothing, unlike the node's `NodeOnline`), and `up = false` with `closed`, `silent`
(nothing for 25 s; the hub closed it) or `replaced` (a newer connection of that node was already registered) when it ends.
`generation` counts the node's connections (1, 2, …, kept across hub restarts); `last_heard_ms` is the hub's clock when it last
received anything on that connection. A reader takes a machine's `online` from its latest presence fact by log position (this
event, or `NodeOnline`/`NodeOffline` from an older hub), ignores a `NodeLink` whose generation is below the node's latest `up`,
never takes `replaced` as offline, and ignores a `NodeLink` from any producer but `hub`; a node that sends one is refused
(`protocol.invalid_message`, field `node_link`). For clients that predate it the hub keeps writing `NodeOffline` when the
current connection ends. A hub that starts has no connection: every node its rebuilt view shows online is recorded offline
before it accepts connections (`NodeOffline`, then `NodeLink { up: false, reason: closed }` for the node's latest generation,
or 0, with `last_heard_ms` the node's last time in the log). An end the store refuses to record (busy, I/O error) is kept in
memory, in order, and written before any later hub fact (so a node that dials again is recorded down, then up, and its new
connection is refused while the end still cannot be written) and by a retry that waits 100 ms, doubling up to 5 s, until the store works (#996); the
machine is shown offline from that write on. The snapshot's `NodeSnapshot.last_event_ms` includes `last_heard_ms`, so it is "last seen".

Schema additions (ADR-0025/0026/0027, all new messages, nothing published changed): `agent.list` (`AgentList` -> `AgentListResult` of
`AgentSummary`; implemented on the node and as `agora agent list`), `EventAck` (hub -> producer, cumulative), `client.proto`
(`Subscribe`, `Snapshot` with `*Snapshot` tables mirroring the hub view, `PositionedEvent`; named `*Snapshot` because
`AgentState` is already the enum) and `enroll.proto` (`EnrollRequest`, `EnrollResponse`). Golden fixtures added for each.
Frame kinds on streams 0 and 2 are the ADR-0026 table.

Hub service (`agora-hub::server`, ADR-0011/0026/0027): one TLS port. A client certificate from the hub CA that is not revoked makes the
peer a **node** (named by the certificate): `Hello`/`Welcome`, `Resume`/`ResumeOk`, then `Event` frames; the hub answers with a cumulative
`EventAck` after each batch and when idle, and a `ResumeOk { from_seq }` on a gap. A peer **without** a certificate is a client if its first
frame is `Subscribe` and its `Authorization: Bearer` token (or, for browsers that cannot set headers, the subprotocols `agora.v1, agora.bearer.<token>`; the hub echoes only `agora.v1`) is accepted (a `Welcome` naming the person's role (ADR-0106, section 1), then the `Snapshot` or backlog, then `PositionedEvent`s; too slow = dropped,
resubscribe with the last position), or an enrolling node if its first frame is an `EnrollRequest` command (one command, nothing else).
Enrollment answers are the same for a wrong, expired or used code (`auth.unauthenticated`), and guessing is locked out for everyone
(`auth.forbidden`). Frame kinds are in `agora_protocol::frame::kinds`.

Routed commands (ADR-0029, `agora-hub::router`): a signed-in client sends `CommandRequest` frames (stream 1, kind 0) with
`target_node` set. The hub answers `node.unknown` itself for a missing or never-seen node, a command it must not forward
(enrollment) is `protocol.invalid_message`, and a node that is not connected is `node.offline`. Otherwise it forwards the request
to that node's connection under a hub-chosen `request_id` (the idempotency key passes through) and maps the answer back to
the client's id; a node that disconnects first gives `node.offline`, no answer within 30 s `node.timeout`, and a client may
have 256 requests waiting. At most 1024 commands from all clients together wait to be written to one node connection
(`router::NODE_QUEUE_MAX`; the queue only grows while a write to that node waits); one more is `node.resource_limit`
`{resource: requests, limit: 1024}` and was never sent, so retrying it is safe. The node runs the command on its own control socket, so it gets the handler, policy and approval
rules of a local command (the hub cannot widen them).

Terminal relay (ADR-0029): a client sends `TerminalSubscribe` (with `target_node`) on a dynamic stream id it chooses (3+; any
other command on a dynamic stream, or a subscription on stream 1, is `protocol.invalid_message`; the hub refuses it, and the node
refuses it again on its hub connection, never relying on the hub, as it does a second subscription on a stream it still has
open, and judges an allowed one by the grants it applies on stream 1).
The hub reserves a dynamic
stream on the node's connection, forwards the request on it, answers on stream 1, and then copies the node's terminal frames
(kinds 0..=4 above; any kind is copied) to the client's stream without decoding or storing them. Input and resize are ordinary routed commands.
Kind 255 (`STREAM_CLOSE`, empty payload) ends a dynamic stream from either side: the client closing, the node leaving, or the
terminal exiting (after the exit frame) ends the relay. A client that falls 512 frames behind is not waited for: the hub closes
its relay on both sides and the client resubscribes for a fresh snapshot (a hub-side simplification of ADR-0029's "the node
resends a snapshot"; the result for the client is the same and the node is never slowed). On the node, a thread bridges the
stream from the node's own control socket to the hub connection through a bounded queue.

Shared terminal size (ADR-0072, Accepted; `agora-hub::router`, no new message): several windows may watch one terminal, and it gets the smallest size
any of them asked for (tmux's "smallest"), so no window is clipped. The hub keeps, per `(node, terminal_id)`, the last
`TerminalResize` size of each client connection that has a relay for that terminal, and forwards to the node the smallest
`cols` and, separately, the smallest `rows` over those connections, only when that differs from the size the hub last sent or saw
in the terminal's own kind 2 frame (so a size set by a local client is noticed); otherwise it answers the request `ok` itself and
sends nothing. When a connection closes its terminal stream (kind 255 from the client) or goes away, its size no longer counts and
the others get the new smallest size (the terminal grows back); the last window leaving changes nothing. A refused subscription
counts as leaving. A relay the hub ends (slow client, node gone, exit) changes nothing by itself; the client's size counts again
when it resubscribes on the same connection. Rules: a connection that has not sent a size does not count (a viewer cannot, its
`terminal.resize` stops at the hub); a size of 0 is forwarded unchanged for the node to refuse as before; a dimension above the
node's limit (1000) asks for 1000. A `TerminalResize` from a connection that does not watch the terminal has no say while one
that does has a size there (answered `ok`, nothing sent), and is forwarded unchanged as before when none does (a script). The
hub-chosen size travels as an ordinary `TerminalResize` under a hub request id; nobody waits for its answer when the hub sends it
on its own. Within one connection the last size counts (two views of one terminal in one window share it). This is the one
command whose content the hub chooses (ADR-0072 amends ADR-0029 there, which otherwise forwards commands unchanged); it changes a
size, never what the policy allows. Clients on the node's own control socket (the terminal client, `agora` commands on that machine) talk to the node
directly: there the last resize still wins, and the hub's windows see that size in the terminal's resize frame. Clients report the
size that fits their view and show the terminal at the size of the node's resize frame, never reporting that size back (the web
window: `apps/web/src/terminal/sizing.ts`; it says its size again right after every subscription).

Client sign-in (`agora-hub::signin`): `agora hub login` (and `agora hub run`, once at start) prints a one-time code (`XXXX-XXXX`, about 39
bits, 10 minutes, single use). The web app trades it with `POST /api/session` (`{"code": "..."}` → `{"token": "..."}`, 401 for a
wrong, expired or used code, 429 after five misses in a row) for a session token: 128 random bits, valid 12 hours, stored only as a
hash. The token is the bearer credential above. `<hub dir>/token` stays valid for scripts. 

Metrics token (ADR-0076, `agora-hub::metrics_token`; no protocol message, the hub's HTTP surface only): `GET` or `HEAD /api/metrics`
(only with `agora hub run --metrics`, else answered as an unknown path) accepts `Authorization: Bearer agm_<64 hex digits>` besides the
owner's session. That token is made with `agora hub metrics-token create NAME [--expires DAYS]`, kept only as a SHA-256 hash in the
`metrics_tokens` table, looked up on every request, and accepted nowhere else: any other path and the WebSocket treat it as an unknown
token (401). A bearer that is neither a live metrics token nor the owner's session is 401, counts toward the address's sign-in
lockout, and is 429 while locked out; a member's or viewer's session is 403.

Screen password (ADR-0067, `agora-hub::password`; additive JSON on the same HTTP routes, no protocol message): `GET /api/session` →
`{"passwordRequired": bool, "passwordSet": bool}` (required only when one is set and the TCP peer is not loopback). `POST /api/session`
also takes `"password"`; without the right one from a non-loopback peer it is 401 like a bad code (the code is not spent). Wrong codes,
passwords and passkeys are limited per address (5 tries, then 1 minute doubling to 1 hour; loopback at most 1 minute) and codes and
passwords also hub-wide (50), 429 while locked out; every 401/429 is answered after 250 ms. `DELETE /api/session` with a bearer ends that
session ("Lock"). `POST /api/password` (`{"password", "current"?}`, owner session; `current` needed when one is set) sets it and ends
every other session (400 `{"code":"password.too_short"|"password.too_long"}`, 401 wrong current, 403 not the owner).
From a non-loopback peer with a password set, `POST /api/passkey/register-begin` and `register-finish` also need `"password"` (401
without it, counted like a sign-in); changing or clearing the password deletes passkeys added from elsewhere without it (their ids are
not in the setting `passkeys_vouched`; a passkey added from elsewhere without it under the id of a removed passkey first takes that
id off the list). Passkey `login-finish` counts only toward the per-address limit. The idle lock of a window
uses `DELETE /api/session`. The hash is a setting (`screen_password`: `pbkdf2-sha256$<rounds>$<salt hex>$<hash hex>`), not a schema change.
Remote password work shares two blocking-work slots per hub. Code sign-in and passkey registration need a slot when they
have a screen password to verify. Password settings and passkey registration validate their session first. Password settings
for the owner on loopback use a reserved slot. A request that requires a busy slot gets HTTP 503 with an empty body before it
counts as an authentication attempt.

Passkeys (`agora-hub::passkey`, WebAuthn, ES256 only, no attestation): a signed-in session adds one with
`POST /api/passkey/register-begin` (`{}` → `{"challenge","rpId"}`, `Authorization: Bearer <session>`) and `register-finish`
(`{"id","publicKey"(SPKI),"clientDataJSON"}`, all base64url). Without a session: `login-begin` (→ `{"challenge","rpId","ids"}`) and
`login-finish` (`{"id","authenticatorData","clientDataJSON","signature"}` → `{"token"}`, 401 on any failed proof, 429 while locked
out). The relying party id is the request's `Host` without the port and the origin must be that host; challenges are single use and
last 5 minutes, at most 4 waiting per address (an IPv6 address by its /64; a fifth drops that address's oldest) and 256 hub-wide (when
full, the address holding the most loses its oldest); user presence and verification are required; a signature counter that counts must rise. The hub stores public keys only
(schema v4, table `passkeys`, at most 32).

Mesh posts between nodes (ADR-0031): a node sends `CommandRequest{mesh_post{to: "<node>/<agent>", body, from_agent}}` on stream 1
(kind 0) and the hub answers on the same stream (kind 1) with the node's request id. This is the only command the hub accepts from a
node. The hub forwards it to the target node as an ordinary routed command with `to` rewritten to the local agent id and `origin`
set to the sending node's id (the connection's certificate), overwriting anything the node wrote; at most 64 per node are waiting at
once. Errors: `node.unknown`, `node.offline`, `node.timeout`, `node.resource_limit`, `protocol.invalid_message`, and the target
broker's `mesh.*` codes.

`usage.report` (ADR-0033): a client command the hub answers itself (no `target_node`; one that names a node is refused). It returns
`UsageReportResult`: per agent (`<node>/<agent>`) the model, tokens and an optional `micro_usd` estimate (absent when no price is known),
the sum, and when the prices were refreshed (0 = the table bundled with the release).

Initial event families:

| Family | Examples |
| --- | --- |
| node | `NodeOnline`, `NodeOffline`, `NodeResources`, `NodeLink` (hub-written presence, ADR-0070) |
| project | `CheckoutDiscovered`, `WorktreeCreated`, `WorktreeRemoved`, `GitStatusChanged` (the node checks each running agent's workspace every 5 s and reports when the changed-file count, line counts or HEAD differ from the last report) |
| agent | `AgentStarted`, `AgentStateChanged`, `AgentSessionReported`, `AgentUsageReported`, `AgentQuotaObserved` (ADR-0091), `AgentAlwaysAllowed` (ADR-0101), `AgentExited` |
| terminal | `TerminalOpened`, `TerminalTitleChanged`, `TerminalClosed` |
| approval | `ApprovalRequested`, `ApprovalResolved`, `AgentStartRefused` (ADR-0057) |
| mesh | `MessagePosted`, `TaskChanged`, `LeaseGranted`, `LeaseReleased`, `HandoffCreated` |
| files | `FilesChanged` (batched paths, from watch) |

## 4. Commands (client → hub → node)

Request/response with `request_id` and `idempotency_key`. Responses are `Ok { result }` or
`Err { code, params }` (see `error-codes.md`).

Team audit (ADR-0049) includes `project_id` and `agent_id` when the command or the hub's node-qualified view
identifies those targets. For a hub-side `AgentForget`, the audit row takes the node and local agent id from the
qualified key and resolves the project before forgetting. `TerminalOpen`, `GrantRevoke` and `PreviewOpen` record only
node attribution. `TaskUpdate` and `TaskDispatch` record the `project_id` sent in the request when supplied. Signed
commands record `requested_device_id` from the request. A fresh, successful node reply establishes `signer_device_id`. Replies to requests with a caller-supplied idempotency key may be cached, so
such replies leave the signer field empty. Earlier rows and commands without the corresponding identifier use empty strings.

Initial command set: `project.list`, `agent.start`, `agent.stop`, `agent.prompt`, `agent.set_mode`,
`terminal.open`, `terminal.input`, `terminal.resize`, `terminal.subscribe`, `files.list`, `files.read`,
`files.write`, `files.watch`, `git.diff`, `git.status`, `worktree.create`, `worktree.remove`,
`approval.resolve` (signed, ADR-0020), `mesh.post`, `task.update`, `node.policy.get`.

Implemented by the node today: `project.list`, `agent.*`, `terminal.*`, `approval.resolve`, `mesh.post`, `agent.list`, `agent.kinds`,
`chat.subscribe` (ADR-0038: a conversation stream, below), `git.status`, `git.log` (ADR-0037: newest first, default 30, at most 200), `git.diff`, `files.list` and `files.read`. The git and files commands answer only for the workspace of one of the node's own agents (`worktree_id` is the id
the node reported in `WorktreeCreated`); a path that is absolute, climbs with `..` or starts with `:` or `-` is refused as
`protocol.invalid_message`. `git.diff` is the unified diff against HEAD plus the new files git does not track yet (shown as
all-added, at most 50 files of at most 64 KiB), and an answer is cut at 512 KiB. In a repository with no commit yet there is no HEAD:
`git.diff` and the counts of `git.status` compare with git's empty tree, so every file is new, and `head_commit` is empty (#941). It covers the workspace folder only and names
every file from that folder, so each name is a path `git.diff` and `files.read` take: for a workspace that is the folder `web/`
of a repository, `web/a.txt` is `a.txt`; a rename across the folder's edge shows as the file added or deleted inside it. At a
repository's root the names are the repository's own. `files.list` hides `.git`; `files.read` refuses
files above 512 KiB (`files.too_large`) and paths that leave the workspace, symlinks included, and links that never resolve (a missing target, a link to itself, a ring of links) (`files.outside_root`), checked again on the opened file so a link swapped in after the check is still refused.

UTF-16 text (#940). A file that is UTF-16 with a byte-order mark (`FF FE` or `FE FF`; Windows PowerShell 5.1's `>` writes the
former), of even length, valid UTF-16 and with no control character other than a tab, a line break, a carriage return or a form
feed, is text. `files.read` still answers its bytes as they are (a download is the file itself); clients decode it for showing.
Where git prints `Binary files ... differ` for such a file (it holds NUL bytes, and the node runs git with `--no-textconv`), or
prints its raw bytes as text (a file with no zero byte: one line of characters above U+00FF), the `git.diff` section is written
by the node as git writes a text diff (three lines of context), from the decoded text of both sides (HEAD's and the workspace's,
each at most 64 KiB, at most 50 such files); a new file that git does not track is shown decoded the same way. The `diff --git`,
mode and `index` lines are git's, so the blob ids are those of the bytes. A section stays git's when a side is not text, cannot be
read, or the two texts are the same (only the encoding changed). A secret file is still cut to its name, and the workspace's file
is read with the secret check of `files.read` (on where the opened file really is) unless `files.show_secret` is on. Nothing is
ever written back in another encoding.

Secret files (ADR-0080). The node decides by file name (case-insensitive, any folder): `.env` and `.env.*` (not `.env.example`, `.env.sample`, `.env.template`), `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.kdbx`, `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519` (not their `.pub`), `.npmrc`, `.pypirc`, `.netrc`, `.git-credentials`, anything inside a folder named `.ssh`, `.aws` or `.gnupg`, and `.docker/config.json`. Added to the schema (old clients ignore them, so no capability): `FileEntry.secret` (4), which `files.list` sets on such entries (it still lists them); the error `files.secret` (`path`), which `files.read` answers for the requested path or for where a link leads; `GitDiffResult.secret_files` (2, `SecretFileChange { path, added, removed }`) and, in `diff`, a changed secret file as its `diff --git` line plus mode and rename lines, with no hunk, blob id or content. The change counts of `git.status` are unchanged. The owner's policy line `files.show_secret = true` (any other value is off; the hub cannot change it) turns all of this off for the machine; `agora doctor` says when it is on.

### Selected directories (ADR-0052)

Capability `workspace.directory` adds commands without changing `AgentStart`: its empty `worktree_id` still means
create a worktree. New clients refuse an unavailable directory launch rather than falling back to isolation.

| New command / request oneof tag | Fields and result |
| --- | --- |
| `directory.inspect` / `DirectoryInspect` (81) | `path`, `create_project_id`, `authorization` → `DirectoryInfo` (Ok tag 18): canonical `path`, `project_id`, `git_root`, `branch`, `portable`, `previous_project_id` |
| `directory.list` / `DirectoryList` (84) | `path` (empty: the user's home folder), `authorization` → `DirectoryEntries` (Ok tag 21): canonical `path`, `parent` (empty at a root), `entries[]` of `{name, is_git}`, `truncated`. Folders only, sorted by name, at most 500 per answer; folders ADR-0052 refuses to launch in are not listed; capability `directory.list` (ADR-0053) |
| `agent.forget` / `AgentForget` (85) | `agent_id`; answered by the hub, which appends `AgentForgotten` (event 36, `agent_id` is the hub-wide key `<node>/<id>`) and stops listing the agent; `agent.running` while it runs; the earlier events stay in the store (ADR-0053). ADR-0108: the hub then passes it on, as `AgentForget` with the node's own id (`agent-7`), to a node that offers the capability `agent.forget.local` (never to one that does not): at once when the node is connected, otherwise after its next `Welcome`. It keeps the forgets each node owes in its settings (`forget.<node>`, at most 1000 per node, oldest dropped first; a hub restart keeps them, forgetting the machine drops them) and drops one when the node answers, whatever the answer. The node refuses `protocol.invalid_message` (`field: agent_id`) for an id that is not a plain name (letters, digits, `-`, `_`), and `agent.running` while the agent runs there (also in the holder, waiting for a restart, or being resumed), touching nothing; otherwise it removes `<id>.jsonl` and `<id>.earlier.jsonl` through its transcript writer, empties the agent's ring and no longer offers it for Resume, and answers `ok` (also for an agent it does not know). Not signed; it passes the node's grant gate (ADR-0041) like any routed command. A client's `AgentForget` with a `target_node` is still refused |
| `node.forget` / `NodeForget` (93) | `node_id`, `refuse_return`; answered by the hub, owner only (`auth.forbidden` otherwise), with no `target_node` (`protocol.invalid_message` otherwise). `node.online` while the machine is connected, `node.unknown` for one the hub never saw. With `refuse_return` every certificate enrolled under `node_id` is revoked first. The hub appends `NodeForgotten` (event 15, producer `hub`): its snapshot leaves out that machine and everything keyed under it (folders, worktrees, terminals, approvals, agents); a later `NodeLink { up }` (or `NodeOnline`) of the same node lists it again with the agents it still runs, and the ones closed before stay forgotten. A client that joined while the machine was hidden holds none of its records and no event repeats them, so when it comes back the hub lets every live client go, and a client resuming from a position before that return gets a snapshot instead of a replay (`View::remembered_at`, rebuilt at start). History and usage stay (ADR-0073). The web window is served by its hub, so it always matches it; there is no separate client capability |
| `agent.launch` / `AgentLaunch` (82) | canonical `path`, expected `project_id`, `kind`, `separate_worktree`, `authorization`, `accept_identity_change` → `AgentLaunchResult` (Ok tag 19): `agent_id`, `terminal_id`, `workspace_id`, actual cwd `path` |
| `workspace.query` / `WorkspaceQuery` (83) | `workspace_id`, `operation` (`list`, `read`, `status`, `diff`, `history`), relative `path`, `limit` → existing file/Git results |

A start of a kind whose program the machine cannot find (`AgentStart`, `AgentLaunch`, `AgentResume`, and the mesh `agent_start`) is
refused with `agent.program_missing` (`kind`, `program`; #903): the program the node looks for (the adapter command, the ACP command
or the launch command) is not installed by `agora agent install`, not on the node's `PATH`, and not an existing path, the same check that
marks the kind `installed: false` in `AgentKinds`. Older nodes answered `internal`.

The node canonicalizes and re-inspects before launch; a path or project ID mismatch is refused. It persists confirmed
path/identity associations; changed metadata requires `accept_identity_change`. Metadata creation is explicit, uses
create-new semantics and never commits or overwrites `.agent-project.json`. The metadata contains `{"project_id":"UUID"}`;
portable domain IDs use `id:<uuid>`. Without metadata, Git discovery remains unchanged; standalone folders use
`folder:<node>:<canonical path>` without a fabricated root commit. A folder whose canonical path is not valid UTF-8 is
refused (`directory.path`): the path travels as text, and a lossy spelling would name another folder. Metadata must be a
normal file, at most 4096 bytes, with a valid UUID, checked again on the open file before it is read (a link swapped in after
the check is refused). An owner confirms its association; metadata is not an access grant.

`WorkspaceRegistered` (event tag 24: `workspace_id`, `node_id`, `project_id`, `path`, `git_root`, `branch`,
`managed_worktree`, `files_changed`) and `AgentWorkspaceBound` (35: `agent_id`, `workspace_id`) precede `AgentStarted`.
That `AgentStarted` leaves legacy `worktree_id` empty. `WorkspaceStatusChanged` (25) carries changed-file/line counts
and HEAD for the workspace. Each agent started in a folder registers it again, also a folder other agents share; such a
registration leaves `files_changed` at 0, and the node sends the folder's `WorkspaceStatusChanged` after it, even when the
counts did not change, so the latest counts follow it (#1017). A registration the node restates (a resync) carries the
counts it last sent. The counts (here, in `GitStatusChanged`, `git.status` and the mesh's `files_changed`) cover the
workspace folder only, the same scope as `git.diff`: for a folder inside a repository, changes elsewhere in the repository
are not counted, and a rename across the folder's edge counts as the file added or removed inside it. Node workspace IDs are `directory:<canonical cwd>`; the hub qualifies IDs with the
producer node and does not trust a supplied `node_id`. `Snapshot.workspaces` (10) and `agent_workspaces` (11) retain
these associations through replay/reload. Published worktree messages/fixtures remain unchanged.

Inspection and launch over the hub always need `DirectoryAuthorization`: trusted `device_id`, random 128-bit hex
`nonce`, `expires_at_ms`, ECDSA P-256/SHA-256 `signature` in IEEE P1363 format. The signed UTF-8 fields are NUL-separated,
with no trailing NUL:

```
directory-action-v1\0node\0operation\0path\0kind\0project_id\0option\0identity_change\0nonce\0expires_at_ms
```

Here `operation` is `inspect` or `launch`; inspect leaves kind/project empty; `option` is `1`/`0` for metadata creation
or separate-worktree launch respectively. `identity_change` is `0` for inspect and the confirmation flag for launch.
No field may contain NUL. Expiry must be in the next five minutes (browser signs for two). Used device/nonce pairs
are persisted until expiry, surviving a node restart; the bounded cache fails closed on corruption or exhaustion.
Generic start grants cannot authorize a substituted directory. A device the node limits to some projects (ADR-0049:
`device.<id>.projects`, or inherited through an endorsement) may sign these operations (and `list`, `resume`) only inside
those projects, judged after its signature by the node's own records, never by what the request or the folder claims: the
canonical folder must be one the node already knows for a project, or lie inside one (the nearest known folder decides;
known folders are the owner-confirmed launch bindings and the checkouts registered with `node run`; a registered project's
id or name for `inspect`, an id before a name and a name two projects share for none); that project must be one of the device's, and for `launch` also the signed `project_id` (the
launch then checks the folder's identity against it as for every launch). `resume` uses the project the node's session
ledger holds for the agent. Refused with `auth.forbidden` (`action: directory`): a folder the node knows for no project or
for another one (also its parent folders, `..`, the home folder), `inspect` with `create_project_id`, `launch` with
`identity_change`, an agent the ledger does not place, and any directory action when the node cannot place folders.
Unlimited devices are unchanged. The local owner-only control
socket has no extra signature requirement. Workspace diffs are rooted at the selected directory, including an empty requested path; they do not include parent-checkout changes.
Workspace queries inherit the existing file/Git path, symlink, `.git` and
size guards and require a workspace used by a managed session. Hub members/viewers can query only granted workspaces.

### Node-local control socket

A node serves the same commands on a local socket (owner-only; `agora node run`, default `$AGORA_HOME/node.sock`)
so it can be driven with no hub. Local commands and replies are protobuf payloads in the binary frame envelope above.
`CommandRequest` goes on stream 1 with kind 0 and
`CommandResponse` comes back on stream 1 with kind 1, matched by `request_id`; a retry with the same
`idempotency_key` gets the stored response (addressed to the retry) and repeats nothing. A terminal stream is
opened by sending `TerminalSubscribe` on a dynamic stream id (3+) chosen by the client (`ChatSubscribe` and `PreviewConnect` too; any
other command on a dynamic stream but `ControlAuth`, which only proves the connection, and any of these three on stream 1, is
`protocol.invalid_message` here as well). After the response the
node writes that terminal's frames on that stream. When the subscription asks for history (`history_lines`, ADR-0100 decision 2; 0
or absent asks for none, and a client that knows nothing of the field never gets any), they start with kind 4 history frames: ANSI
that puts the newest `min(history_lines, lines the node keeps)` scrollback lines (2000 kept today; at most 512 KiB, the oldest
dropped past that) into the client's scrollback at the size of the snapshot that follows. A kind 4 frame is at most 16 KiB and
is cut only between sequences and characters, never inside one; there are at most 32. Their shape: `CSI ? 1049 l`; full-screen
margins (`CSI r`), origin and insert off, autowrap on, a plain pen and ASCII in G0; `CSI 2 J`, then `CSI 3 J`; each line from the top
row with its colours and attributes (re-encoded from the node's cells; nothing the program printed is copied), a line the node holds
as wrapped without a line break (the newest line always ends in one); then the bottom row and one line feed per replayed line still
on the screen. Fed to an empty emulator of that size, its scrollback holds exactly the replayed lines and its screen is clear. No
history is sent while the program is on the alternate screen, or when the node keeps none. A client holds the kind 4 frames with
the snapshot and paints both once it has the size (the web window in one synchronized write, mode 2026), and never waits for
history: when sent, it always comes before the kind 1 it belongs to. A client asks for history only when its terminal has none of its
own: the web window asks for its history setting (1000 to 20 000 lines) on a first open, a reload or a remount and for 0 on a reconnect
or after falling behind; the terminal client asks for its scrollback (2000) on a fresh pane and for 0 when it watches again, and never
feeds history again when it feeds the last snapshot again on a resize (its full-screen terminal asks for none). History belongs to
the subscription it came on: a client drops it when that subscription ends (the connection lost, the relay ended) before its kind 1,
and never holds more than one replay. Then: kind 1 snapshot (ANSI that repaints the screen; ADR-0100 decision 1, in this order: the program's screen, `CSI ? 1049 h` or `l`; origin and insert off, a plain pen and ASCII in G0 shifted in (`ESC ( B`, SI); the screen cleared and every row drawn at an absolute position with its colours and attributes; every mode the node's emulator tracks as an explicit set or reset: mouse tracking 1000/1002/1003 (all reset, then the program's one), encodings 1005/1006 (the same), 1004, 1007, 2004, DECCKM, DECKPAM or DECKPNM, DECAWM, IRM, LNM and the kitty keyboard flags (`CSI < 32 u`, then `CSI = flags ; 1 u`; a client that writes the repaint to a user's own terminal, as `agora terminal attach` does, leaves that pop out, pushes an entry of its own and on detach pops it and the entries the program left pushed on the main screen); the cursor's position, visibility and shape (`CSI 0 SP q` when the program set none, keeping the window's own; mode 12 is never written); the program's current style last. Modes the node does not track (mouse 9, 1015, modifyOtherKeys, 47/1047, the scroll region) are left alone, and so is origin mode (DECOM) until the scroll region is tracked, though xterm.js switches 9 off on the 1000/1002/1003 resets; a client applies it all as ordinary ANSI), each one followed by kind 2 with the size it was painted at, kind 0 raw
output, kind 2 resize (cols, rows as u16 LE), kind 3 exit (i32 LE; negative = signal). A client takes the size of the kind 2 frame that follows a snapshot before it paints the snapshot (a snapshot is made at the node's size; parsed at another width it wraps and scrolls its first rows away); the web window waits for that frame, at most 150 ms. A client that falls behind
gets a fresh snapshot instead of the dropped output, never with history (a client that wants it subscribes again); it never slows the terminal. When the exit was among what it
missed it still gets the exit frame, with `i32::MIN` (unknown) when the node no longer has the code. Input and resize are ordinary
commands. Terminal ids are `t-<agent id>` (one terminal per agent). Implemented commands: `project.list`,
`agent.start`, `agent.stop`, `agent.prompt`, `terminal.input`, `terminal.resize`, `terminal.subscribe`; the rest
answer `protocol.capability_missing`. `agent.list` (ADR-0025) and `agent.kinds` (ADR-0030: the node's built-in and folder manifests and the ACP registry's converted ones (ADR-0092, kinds `registry-<id>`), one per kind (agent-manifest.md, "Where agent kinds come from"), with display name, integration level and whether the launch program is on `PATH`; invalid manifests omitted; `agora agent kinds`). ADR-0069 (capability `kinds.program`): each kind also names `program`, the file name the node looks for on its `PATH` (never a path, and never the node's `PATH` itself), so a kind that is not installed can say what is missing; `NodeOnline.binary` (and `NodeSnapshot.binary`, from the node's latest start) is the node's own executable only when its folder is not on the node's `PATH`, empty otherwise, so a client can say to run `agora` from that folder. It is a full path that may contain a user name: only the owner receives it whole; the hub sends members and viewers its file name alone (snapshot, live and replayed events alike), since any folder, the one it is in included, may be the user's home; the owner's client shows the last two components unless asked for the whole path. Old nodes send both empty.

**Control token (ADR-0082, `ControlAuth` = `CommandRequest` field 101, `message ControlAuth { string token }`, append-only).** On Linux, when the node's
policy confines agents (`sandbox` is not `off`), the node writes 32 random bytes, hex-encoded (64 characters), to `<home>/control.token`
(`<socket>.token` when the socket is not named `node.sock`; 0600, replaced at every start, removed when the node stops) and requires the first
request on every connection to be `ControlAuth`. Until it succeeds every other command answers `auth.unauthenticated`; a wrong token answers
`auth.unauthenticated` and the node closes the connection. A node that requires no token (not confining, or macOS and Windows) answers `ok` to any
`ControlAuth`; an older node answers `protocol.invalid_message` (it does not know the command), and a client takes either as "no token needed".
`agora_node::control::Client::connect` reads the file when it exists and presents it before returning, so callers keep their contract; a client
that cannot read the file (a confined agent) sends nothing and is refused at its first command. `ControlAuth` is accepted on the local socket
only: a hub-routed one is refused (`protocol.capability_missing`, `capability` = `control.auth`). The token is never logged, never in an event
and never sent to the hub. A script that speaks the protocol itself must read the file. Capability: none (a node that requires the token
says so with `auth.unauthenticated`).

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

### Chat transcript stream (ADR-0038)
`chat.subscribe { agent_id }` is sent on a dynamic stream like `terminal.subscribe` and routed by the hub the same way (a relay, not
stored). After the response the node writes on that stream: frame kind 10, an encoded `ChatSnapshot` (the transcript so far), then
frame kind 11, an encoded `ChatEntry` for each line added. Kind 3 closes a relay and is not used here. The node keeps the last 500
entries or 256 KiB of each chat agent's conversation in memory (one entry at most 16 KiB; the user's own requests are entries with
role USER; adapter items map to TEXT, TOOL_CALL as JSON, PLAN as lines, DIFF with its path, and NOTICE with `code` and `params` and
no text, ADR-0058). Entry ids increase per agent; a client ignores an id it already holds. The text is untrusted.

What the stream carries is redacted (ADR-0089 Amendment A, #1147): the node replaces the values of the secrets it holds (8 bytes or
more: its secret store and the environment variables its policy passes to agents) by `<redacted>` in every entry and every version
it sends and in the snapshot, by the same rule and code as the file below. A field is redacted before it is first sent; never an
unredacted version first. What it does not cover is listed in the threat model (T61): a value's first 3 bytes or fewer can go before
the next words show the value goes on, and text sent before a value was added to the store stays as it was sent on streams already
open (a later snapshot replaces it). The agent's words (text and thinking)
are one stream: a value split between two entries is found whole, and words whose end is a part of a value (4 bytes or more) wait
until the next words show it is not one, until any other entry of the agent (so ids still increase on the stream), until its turn
ends, Start again or the ring's bound, or at most 300 ms (checked on the node's 100 ms tick), and then go with that part replaced. Words that cannot end in such a
part go at once, and a node that holds no value sends everything as it did. A snapshot leaves out the words still waiting; the stream
brings them. Unknown secrets are not found; a value added to the node's store is replaced live from the node's next read of its
values (at most a second later). The same values are replaced in the agent's own words the node records as hub events: an approval
card's action and detail (before the request is queued, so the card and the always signature hold the same words), a mesh message's
body, a task's title, a handoff's summary, and the names, descriptions and hints of the agent's modes, options and commands; ids are
kept.

The node also writes each structured agent's transcript to its own disk (ADR-0089 decision 1, ADR-0104 C1:
`<data>/transcripts/<agent id>.jsonl`, owner-only, batched off the agent's path at most once a second or per 64 KiB, the node's own
secret values of 8 bytes or more replaced: whole anywhere, also when an agent's streamed text splits one between pieces, and as a piece of 4 bytes or more at a place the node cut a text; ids are never changed; see the threat model, T61). A node that takes an agent up from the holder, or resumes it (ADR-0060), fills the ring from the file's end
before it reads anything new, so the snapshot shows the earlier turns with their ids, and numbers on after the highest id the file
shows (a rotated file keeps that mark even when no whole entry fits): ids never start again at 1, unless the file was purged or
removed by retention. A tool call or a plan from before the restart is changed in place under its id. A permission entry
still pending in the file is shown cancelled (a request still open is asked again under a new approval id), and a prompt still queued
there is dropped with an `adapter.queue_dropped` notice (also one the old node sent in its last unwritten second: the file cannot tell).
What the file holds is held to the shapes and bounds of live entries before it is shown. An agent that loads its session on resume
replays its conversation (ACP `session/load`, passed on by `agora acp-bridge` as chat before it answers the session's start): the
node shows that replay only when it holds none of the conversation, so every turn appears once. A fresh start ("Start again") that went through begins a new file (the earlier
one is kept as `<agent id>.earlier.jsonl`, replacing an older one) and numbers on after the earlier session's ids; a refused resume
changes neither. A node that stops cleanly writes everything first, the words it held back included, with a secret value's start at
their end masked (ADR-0104 O2: `agora node` on `SIGTERM`, Unix). A node killed loses at most the last batch (up to a second), and while
it holds a secret value the agent's newest words it held back in a turn: those entries may have been shown live (redacted) and are
not in the file, and the next node may give their ids again; every client then holds the new node's snapshot, since a relay closes when its node
goes and a client subscribes again. When the history cannot be written (a full disk), the agent is not touched, its entries stay in
memory, and its conversation gets the notice `chat.history_not_saved` once (no parameters; only the node makes it). The node policy
`chat.history_max_bytes = N` (per agent; default and at most 32 MiB, 0 writes nothing) and `chat.history_days = N` (default and
at most 30; 0: as soon as the agent ended) bound it: over its size a file keeps its newest half, and a file is removed that many
days after its agent ended (as the node's session ledger says; after its last write for an agent the ledger no longer knows), never
while the agent runs or is offered for Resume (2 GiB per node in all, the files of agents not kept first). A failed batch is not
written later. `agora history purge` removes every transcript. Reading older entries in pages
(`chat.history`) is ADR-0089's later schema step.

`MeshPost.refs` (ADR-0035, field 5): the sender's refs as written; the receiving broker qualifies them with the hub-vouched `origin`.

`BoardCall` (command 62, node → hub on the stream-1 request path like `MeshPost`) and `BoardResult` (`CommandOk` field 16), ADR-0039: a mesh tool on a shared
project's board; see `docs/specs/mesh-tools.md#shared-board-adr-0039`. New error code `board.unavailable`.

`BoardJoin` (command 66, hub → node only; a client's is refused) and `BoardJoinResult` (`CommandOk` field 20, with `BoardJoinLease` and `BoardJoinTask`), ADR-0039 amendment B: when a project becomes shared, the hub asks each node with a checkout for its unfinished local work and puts it on the board. A node that goes offline and comes back to a shared project is asked again, also when the hub records its end late, together with its next connection (#1036). Sent only to nodes that offer the capability `board.join`.

`GrantCreate` (63), `GrantList` (64), `GrantRevoke` (65) and `GrantListResult` (`CommandOk` field 17), ADR-0041: user-signed grants for the commands a node's policy lists in `signed_commands`; see `docs/security/threat-model.md`. New error code `approval.grant_required` (`command`). The node answers them itself; the local control socket refuses them. A grant covers commands only while the device that signed it is still trusted on the node, in the same term and within its limits: when that device is removed or revoked, its endorsement runs out, or its limits no longer allow the grant's scope, the grant ends and `GrantList` no longer shows it; trusting the device again does not bring it back.

### Agents that start agents (ADR-0057)

Capability `mesh.spawn` (offered by a node that has the mesh tools `agent_start`, `agent_wait` and `agent_output`; see
`mesh-tools.md`). All additions are new fields and one new event; nothing published changes meaning.

- `AgentStarted.parent_agent_id` (7) and `AgentSnapshot.parent_agent_id` (13): the agent on the same node that started this one
  through the mesh; empty when the user started it. Every `AgentStarted` the node sends for that agent (launch, restart, resync)
  carries it.
- An agent's request to start another is an `ApprovalRequested` with `action = "agent.start"`. Its `detail` is JSON with the keys
  `node`, `agent` (the asking agent), `agent_kind`, `kind` (to start), `folder` (canonical), `inside` (the folder is the asking
  agent's own folder or below it), `worktree` (the helper gets its own worktree of it), `depth` (the helper's depth),
  `grant_allowed` (a scoped grant may be given: inside, and not a shell or plain terminal program) and `prompt`
  (the first prompt in full, at most 16 KiB; a longer one is refused before anything is asked). Clients show all of it; the prompt
  is untrusted text written by an agent.
- The decision on an `agent.start` approval is signed over
  `agora-approval-start-v1 NUL node_id NUL approval_id NUL verdict NUL helper_grant NUL helpers_may_start NUL sha256(detail)`, where
  the verdict and `helpers_may_start` are the ASCII digits `0`/`1`, `helper_grant` is decimal and the hash is lowercase hex, instead of the ADR-0028 message. The node keeps the request it announced and starts the helper
  from that, so the signature binds exactly what the user was shown: a hub that changes the prompt, folder or kind on the way to the
  window, or adds or strips a grant on the way back, makes the signature fail.
- `ApprovalResolve.helper_grant` (5): with an approval, allows the asking agent up to this many helpers of the same kind in the same
  folder while it runs, this start included. `ApprovalResolve.helpers_may_start` (6): the helpers started under this decision may
  themselves ask (each of their starts still needs its own approval, and their helpers cannot ask further). Neither is honoured for
  a folder outside the asking agent's own. `ApprovalResolved.helper_grant` (4) and `.helpers_may_start` (5) record what was signed.
- `GrantList` lists live helper grants as `GrantInfo` with `commands = ["mesh.agent_start"]`, `scope = "agent:<id>"`, the new fields
  `kind` (6), `folder` (7) and `remaining` (8), and `expires_at_ms = 0` (it ends when the asking agent exits or is stopped, or when the device that signed it is no longer trusted in the
  same term or within its limits, audit F290). `GrantRevoke { grant_id }`
  with the approval id ends one.
- `AgentStop.stop_helpers` (2): also stop the helpers this agent started, and theirs. Old nodes ignore it.
- `AgentStartRefused` (event 65: `agent_id` of the asking agent, `kind`, `folder`, `code`): the node refused a start before asking
  the user (helper limit, depth, folder, rate, policy). Together with `ApprovalRequested`/`ApprovalResolved` and `AgentStarted` it
  is the audit trail of agent starts.

New error codes: `mesh.folder_not_allowed` (`folder`), `mesh.helper_limit` (`rule`, `limit`), `mesh.start_denied`
(`approval_id`).

`Snapshot.tasks` (8) and `Snapshot.leases` (9), ADR-0040: the shared boards' tasks and held paths, so a client that connects sees them before the next change.

`Snapshot.recent_messages` (12), ADR-0103: the newest mesh messages the window may see, oldest first, each the stored event as it is
(`PositionedEvent`: its log position, at or before the snapshot's `pos`, and the `Event` with its producer, time and `MessagePosted`
body). The hub keeps, per project (`<node>/<project>`), the newest 50 `MessagePosted` events and at most 256 KiB of them encoded,
whichever is smaller, and at most 8 MiB over all projects (the oldest goes first; one message larger than 256 KiB alone is not kept).
A message goes to the ring of `<producer>/<from_agent_id>`'s project when that is an agent the hub knows (the sender's own copy), else
to the ring of `<producer>/<to>`'s project when that is one (a receiver's copy, each `#channel` copy), else to a ring that only the
owner reads; the hub never resolves an already-qualified `from` to another machine's agent. The rings are memory only, folded from the
events the hub applies, so a hub rebuilds them from its log at start. A window is sent the newest 50 it may see, then fewer until they
encode to at most 256 KiB together and fit in the space left in the 1 MiB snapshot frame after the other fields are encoded.
If no message fits, the field is empty. The owner is eligible for every ring; a member or viewer sees only the copies placed by their
sender in a granted project (the copies the live stream gives them); no message whose sender is a forgotten agent or under a forgotten machine, or that a
forgotten machine recorded (they come back with the machine). Copies are sent as stored: the window pairs the two copies of a message
between machines as it does live. A client folds them in as it folds a live `MessagePosted`, not by the position rule (their
positions are not after the snapshot's), each position once, and ignores any other body. No capability: an older window ignores the
field, an older hub sends none. These limits are hub constants, not policy.

### Notices, modes and models, context and cost (ADR-0058)

All additions; nothing published changed.

- **Notices.** `ChatKind.CHAT_KIND_NOTICE` (5) with `ChatEntry.code` (7) and `ChatEntry.params` (8, string map): something a person
  should know about the conversation (a failed, stopped or empty turn: `adapter.turn_failed` `{rpc_code}`, `adapter.turn_stopped`
  `{stop_reason}`, `adapter.turn_empty`; the agent exiting: `adapter.agent_exited` `{stage, exit_code?, stderr_tail}`). Clients
  render it from their locale files by its code (`notice-` + the code with `.` as `-`) and show an unknown code as a generic notice.
  The adapter's `NoticeItem` (`ChatAppendParams` field 6) is what the node turns into one, checked and bounded
  (`docs/specs/adapter-protocol.md`).
- **Modes and models.** Event `AgentSessionOptions` (`Event` field 37: `agent_id`, `modes[]`, `models[]` of `SessionOption { id, name,
  description, skips_permissions }`, `current_mode`, `current_model`) replaces the agent's previous report; the snapshot carries the
  last one in `AgentSnapshot` fields 14-17. A client offers the lists only when the agent reported them; a mode with
  `skips_permissions` needs the user's confirmation and is never chosen by default. Command `agent.set_mode` (`AgentSetMode`, as
  published: `agent_id`, `mode`) and the new `agent.set_model` (`AgentSetModel`, request field 86: `agent_id`, `model`) take one of the
  reported ids. Node capabilities `agent.set_mode` and `agent.set_model`: the hub refuses either command with
  `protocol.capability_missing` for a node that did not offer it and never forwards it; the node refuses it for an agent whose adapter
  did not declare the switch (`protocol.capability_missing`) and for an id the agent did not offer (`protocol.invalid_message`,
  `field` `mode` / `model`). Both commands are the user's own: viewers cannot send them, the hub audits every one like any routed
  command (action `AgentSetMode` / `AgentSetModel`, the node and the outcome), the event log records the new current mode or model,
  and no mesh tool lets an agent send them. They need no approval (ADR-0058's open questions).
- **Context and cost.** `AgentUsageReported` gains `cost_micros` (5, optional: the growth of the cost the agent reported itself, in
  micro-units of `currency`), `currency` (6, three letters), `context_tokens` (7) and `context_window` (8) (the context's level and
  size now; 0 when not reported). The hub keeps the last level and size and the sum of the reported cost per agent (a report in
  another currency starts the sum again) and puts them in `AgentSnapshot` fields 18-21. `usage.report` answers an agent's own cost
  in US dollars as its `micro_usd`, and estimates from the price table only for agents that reported none.
- **Start failures.** `agent.start` of an adapter agent whose adapter failed with a stable reason answers with that code instead of
  `internal`: `adapter.agent_exited` `{stage, exit_code, stderr_tail}` or `adapter.agent_refused` `{stage}` (`error-codes.md`).

### Session ledger and Resume (ADR-0060)

All additions; nothing published changed.

- **Ledger.** A node keeps `<data>/sessions/ledger.json` (folder `0700`, file `0600` on Unix, an owner-only ACL on Windows; replaced
  whole through a temporary file in the same folder and a rename; at most 512 entries, 4 MiB read). Per agent: id, kind, project,
  folder (its workspace path or its own worktree), whether it is a directory agent (ADR-0052) and its separate worktree's Git root
  and branch, parent agent (ADR-0057), the agent's own session id once reported (`AgentSessionReported`), the launch options of
  ADR-0059 (recorded from `AgentStarted`, never applied by `agent.resume`: a resumed or restarted-again agent asks before acting,
  and its `AgentStarted` carries no options, so the "does not ask" badge goes), start time, last state, whether it ended, and when it was found interrupted. Never
  terminal contents, prompts, or secrets. It learns from the events the node records: written when an agent starts, reports its
  session and ends; a state change is written with the next write and when the node stops cleanly. The ledger also keeps the next
  agent id, so an id is never given twice across restarts. A file that cannot be read is renamed (`ledger.json.unreadable-<ms>`),
  never deleted. Entries are dropped 14 days after they were last touched (an interrupted one 14 days after it was found).
- **Ended agents in `agent.list`** (#1120). A node lists in `agent.list` the agents its ledger keeps as ended, also those an earlier
  process of the node ran, as that process listed them: `running` false, the ledger's last state, its folder as `worktree_id`, no
  terminal and no restarts, whatever folder they ran in (a project worktree, a directory agent's folder or its separate worktree).
  An agent the supervisor knows is listed from the supervisor. Nothing is announced to the hub for them: the hub keeps its own closed
  agents. An interrupted agent is reported with `AgentInterrupted` (below) and is also listed in `AgentListResult.interrupted`
  (ADR-0107, field 2: each entry the `AgentInterrupted` the node announces for it, built by the same code, so `resumable` and
  `still_running` are what the window shows; oldest id first; an id the supervisor holds, one being resumed or started again, is in
  `agents` only; old nodes send none and old readers skip it). The list is in id order. An ended agent
  listed only from the ledger cannot be addressed: `agent.stop`, prompts and the other agent commands answer `agent.not_found`, and its
  folder is not a `worktree_id` the node accepts.
- **Interrupted.** An agent whose entry had not ended when the node started again was interrupted by the node stopping. Event
  `AgentInterrupted` (`Event` field 38: `agent_id`, `kind`, `project_id`, `folder`, `parent_agent_id`, `resumable`, `started_at_ms`,
  `interrupted_at_ms`, `last_state`) says so after the node's `NodeOnline`, and again in every resync announcement, until the agent
  is resumed or started again (its next `AgentStarted`) or no longer kept. `resumable` is true when the node has the agent's own
  session id and its kind can resume one (a manifest `[resume]` argv, or an adapter or ACP agent, whose bridge asks the agent for
  `session/load` when it offers `loadSession` and opens a new session otherwise). The hub keeps the agent with the closed ones
  (`running` false, state `done`, so it reads "finished") and adds `AgentSnapshot.interrupted` (40), `resumable` (41),
  `interrupted_at_ms` (42) and `folder` (43); its next `AgentStarted` clears them. An agent the hub never saw (its store was reset)
  is listed from the event alone. `agent.forget` works on it like on any closed agent.
- **Still running.** The ledger also records each agent's pid, process group and start time. `AgentInterrupted.still_running` (10,
  snapshot `AgentSnapshot.still_running` 44) says its process from before the restart still runs (same pid, same start time; a node
  killed without stopping its agents). Then `agent.resume` answers `agent.still_running`, and `agent.stop` with its id ends exactly
  that process (its group when it leads one), with `stop_helpers` also its helpers that outlived the node, and says
  `AgentInterrupted` again without `still_running`; a pid whose start time, boot (Linux `boot_id`) or program name differs is never
  signalled. `agent.stop` of any interrupted agent succeeds: when its old process has ended meanwhile nothing is signalled, and it is
  announced again without `still_running`, so clients offer Resume and Start again. Not on Windows, where a node's agents end with it.
- **Resume.** Command `agent.resume` (`AgentResume`, request field 87: `agent_id` (the node's own id), `new_session`,
  `authorization`) → `AgentLaunchResult` (Ok tag 19). Node capability `agent.resume`: the hub refuses the command with
  `protocol.capability_missing` for a node that did not offer it. Same agent id, same folder, same project, same kind. Without
  `new_session` the node runs the kind's `[resume]` argv with `{session_id}` replaced, or starts the adapter with the session to
  load; `agent.session_missing` when it has no session id or the kind cannot resume one. With `new_session` (Start again) it starts
  a new session of the same kind in the same folder and forgets the old session id. Refusals: `agent.not_interrupted` (not held as
  interrupted), `agent.folder_changed` (the folder is gone, or a directory agent's folder now has another project identity,
  ADR-0052), `agent.parent_not_running` (a helper is resumed only while the agent that started it runs; its grants and approvals are
  not restored), and the same node policy and limits as a start (`agent.sandbox_unavailable`, `node.resource_limit`, ...). Over the
  hub it is signed like `agent.launch`, with the same `DirectoryAuthorization` and message:
  `directory-action-v1\0node\0resume\0agent_id\0\0\0new_session\00\0nonce\0expires_at_ms` (operation `resume`, the agent's
  local id in the `path` field, kind and project empty, `option` the `new_session` flag, `identity_change` `0`); a hub cannot resume
  anything by itself. The local owner-only socket needs no signature. Viewers cannot send it, and the hub audits it like every routed
  command (action `AgentResume`).
- **Session ids.** Adapter and ACP agents report theirs (`session/reported`). A terminal agent's comes from its manifest's `[session]`
  (`docs/specs/agent-manifest.md`): an id the node chooses and passes at the first launch (Claude Code), or one read from the first line
  of the session file the agent writes (Codex), only when that cannot be confused with another agent's. Without either, a terminal
  agent is offered Start again only. A resumed helper keeps its parent (stopped with it, listed under it) but may not ask to start
  helpers and none of its parent's grants cover it until a new decision is signed.
- **Automatic resume.** The node's own `<data>/policy` may say `resume = auto` with `resume.kinds = a, b` and `resume.folders = /x,
  /y`: after a start, interrupted agents of a listed kind in a listed folder (or below it) that can resume their own session are
  resumed without anyone asking (a helper only after its parent was). The default is `resume = ask`; any other value is `ask`. The
  hub cannot change it. `agora node run --resume auto` (ADR-0062 item 4, what `agora` passes when the owner answered yes) resumes
  every such agent whatever its kind and folder, but only when the policy has no `resume` line: a `resume` line always wins (`ask`
  stops it, `auto` keeps it to the listed kinds and folders). Either way at most 16 at one start, 8 at a time (the next round 5 s
  later), and never an agent the node brought back by itself in the last 10 minutes (`auto_resumed_at_ms` in the ledger).
  `resume.continue = on | off` (default on, wherever automatic resume is on; Amendment A of ADR-0060) decides whether an agent that was *working* when the node stopped is given its kind's one "carry on" message (`[resume] continue_argv`, `docs/specs/agent-manifest.md`) with that resume; never for an agent that was waiting for a person, idle, finished or unknown, never when a person pressed Resume, and once for each interruption. No command or message carries the policy line or the sentence.

### Launch settings (ADR-0066)

All additions; nothing published changed.
- **What may be given.** `AgentKind.settings` (7) lists a kind's `[[launch.setting]]`s as `LaunchSettingSpec { id, label, values,
  default_value }` (the manifest's order; the arguments stay on the node). Old nodes send none.
- **The choice.** `AgentStart.settings` (9) and `AgentLaunch.settings` (9) carry `LaunchSetting { id, value }` pairs. The node checks
  them against its own manifest (`agent.setting_unknown {kind, setting}`, `agent.setting_invalid {kind, setting}`, nothing starts) and
  puts each value into its argument template as one argument (agent-manifest.md, "Launch settings").
- **Through the hub: signed.** Node capability `agent.launch_settings` (offered by a node that verifies the signatures below; the hub
  refuses a command with settings for a node without it, `protocol.capability_missing {capability: agent.launch_settings}`, instead
  of letting an older node start the agent with the tool's own defaults). `AgentLaunch` with settings is authorized over its own
  domain `directory-action-v4`: the `directory-action-v3` fields, then the settings, as a length-prefixed pair list (`<count>` then
  `:<id byte length>:<id>:<value byte length>:<value>` per pair, in the order the request carries them: `[("model","x")]` is
  `1:5:model:1:x`), before the nonce:

  ```
  directory-action-v4\0node\0launch\0path\0kind\0project_id\0option\0identity_change\0launch_options\0confirmed\0settings\0nonce\0expires_at_ms
  ```

  A launch with settings verifies only over v4; one without keeps `directory-action-v3` (options) or `-v1` exactly, so a hub can
  neither add, change, drop nor reorder a setting, and an older signature does not carry over. At most 8 settings, each id at most 64
  and each value at most 128 bytes, or the signature is refused. Shared vectors checked by the node, the terminal client and the web
  window: `schema/fixtures/signing/directory-action-v4.json`. A routed `AgentStart` with settings carries no signature and is
  refused (`agent.option_forbidden {kind, option: <setting id>, policy: signature}`); a grant never covers one. On the node's own
  control socket (the owner) no signature is needed. The audit trail's action reads `AgentLaunch[model=x,effort=high]`.
- **From an agent (`agent_start`, ADR-0057).** The mesh tool takes `settings: { <id>: <value> }`. The node checks them against its manifest
  for the kind before anyone is asked (the codes above; nothing is announced and nothing starts), puts them into the request in the
  manifest's order, and the `agent.start` approval's `detail` gains `settings`: a list of `{id, label, value}` (`label` is the
  manifest's Fluent id, e.g. `setting-model`; present only when there are settings, so a request without them is byte for byte what it was)
  and `grant_allowed` is `false` (a grant never covers a start with settings, and a later start with settings is always asked). The
  decision on such a request is signed over `agora-approval-start-v2`, which names the settings before the hash of the detail:

  ```
  agora-approval-start-v2\0node_id\0approval_id\0verdict\0helper_grant\0helpers_may_start\0settings\0sha256(detail)
  ```

  with `settings` as above (read from the detail's `settings`, the pairs `(id, value)` in the order listed). A request whose detail has
  settings is decided only over v2 (a client that signs the v1 message does not name the settings and is refused with
  `approval.signature_invalid`); one without keeps `agora-approval-start-v1`. Vectors: `schema/fixtures/signing/approval-start-v2.json`
  (node and web window). The helper starts with exactly the settings shown and signed, and with a launch option only as below. The node enforces "no grant" itself: a signed `helper_grant` or `helpers_may_start` on a request with settings is ignored.
- **Launch options from an agent (ADR-0059 decision 5).** The mesh tool takes `options: [<id>]`. The node checks them as a window's
  `AgentStart.launch_options` (`agent.option_invalid`, `agent.option_forbidden` from its own manifest and `skip_permissions` policy)
  before anyone is asked, and the `agent.start` detail gains `launch_options`: a list of `{id, label, skips_permissions}` in the
  manifest's order (`label` is the manifest's Fluent id; `skips_permissions` the manifest's word; present only when there are options)
  and `grant_allowed: false`. The decision on such a request is signed over

  ```
  agora-approval-start-v3\0node_id\0approval_id\0verdict\0helper_grant\0helpers_may_start\0settings\0launch_options\0sha256(detail)
  ```

  with `settings` as for v2 (`0` when there are none) and `launch_options` the ids as a length-prefixed list (`1:16:skip-permissions`),
  read from the detail in the order listed. A request with options is decided only over v3 (a v1 or v2 signature is refused with
  `approval.signature_invalid`). Vectors: `schema/fixtures/signing/approval-start-v3.json` (node, web window, terminal client). The
  signed decision is the confirmation of each option that skips permissions; clients show the same "act without asking?" confirmation
  before they sign an approval. The helper launches with exactly those ids, its `AgentStarted` carries `launch_options` and
  `skips_permissions`, and the node ignores a signed grant or `helpers_may_start` on it.
- **Clients.** The web start window and the TUI's approval card are built. The window offers each `AgentKind.settings` entry (a list only
  among its `values`; with no list a text box limited to the node's plain-token rule), sends nothing for "the agent's own default" (that
  `AgentLaunch` has no `settings` and is signed over `directory-action-v1`/`v3` as before) and signs the chosen pairs in the kind's order
  over `directory-action-v4`. The TUI card lists the settings under the request line and signs `agora-approval-start-v2` for an
  `agent.start` that carries them (v1 byte for byte without); there is no other message it signs for a start.
- **What is not built yet.** The TUI's own start picker; remembering the last choice per kind and machine; the owner's `start.models`
  allow-list and the price class on the card; settings are not kept across a node restart's resume.

### Launch options (ADR-0059)

All additions; nothing published changed. Node capability `agent.launch_options`.

- **What may be chosen.** `AgentKind.launch_options` (5) lists a kind's `[[launch.option]]`s as `LaunchOption { id, label,
  skips_permissions }` (the manifest's order; the arguments stay on the node). A window offers them only for a node with the
  capability; "Ask before acting" (no option) is always the default and a `skips_permissions` option is never preselected and needs the
  user's confirmation (the ADR-0058 dialog, Keep asking as the primary button).
- **The choice.** `AgentStart.launch_options` (7) and `AgentLaunch.launch_options` (7) name the chosen ids; `confirmed_launch_options`
  (8 on both) names those whose "Let the agent act without asking?" confirmation the window showed and the user accepted. Every id must
  be `[a-z0-9-]{1,32}` and declared by the node's manifest for the kind, at most 8, no repeats, and each confirmed id must be chosen;
  otherwise `agent.option_invalid {kind, option}` and nothing starts (so a `!` or `,` in an advertised id gets nowhere). Which options
  skip permissions is the node's own manifest's word: one not listed as confirmed is refused with `agent.option_unconfirmed {kind,
  option}`, so a hub that relays `AgentKind.launch_options` relabelled as harmless (the window asks nothing) gets the start refused.
  The window's list is a display hint only; windows drop advertised ids outside `[a-z0-9-]{1,32}` before offering them. The node adds
  the arguments to the launch command (terminal kinds) or the ACP server command (structured kinds) for the agent's whole life,
  restarts and resumes included. Through the hub, `AgentLaunch` with options or confirmations is authorized over its own domain,
  with both lists length-prefixed (`<count>` then `:<byte length>:<id>` per id: `["a","b"]` is `2:1:a:1:b`, `["a,b"]` is `1:3:a,b`):

  ```
  directory-action-v3\0node\0launch\0path\0kind\0project_id\0option\0identity_change\0launch_options\0confirmed\0nonce\0expires_at_ms
  ```

  Shared vectors checked by the node, the terminal client and the web window: `schema/fixtures/signing/directory-action-v3.json`.
  An `AgentLaunch` without either list keeps `directory-action-v1` exactly as published, so a hub can neither add, remove or swap an
  option nor add or strip a confirmation. A routed `AgentStart` with options carries no signature and is refused
  (`agent.option_forbidden` `{kind, option, policy: signature}`); a grant (ADR-0041) never covers one. On the node's own control
  socket (the owner) no signature is needed. An agent's `agent_start` helper (ADR-0057) never inherits an option; it gets one only
  when the user's signed decision names it (ADR-0059 decision 5, above).
- **Node policy.** `skip_permissions = forbid | allow | confined_only` (quoted or not) in the node's own policy file (default `allow`
  when there is no file; any other value, an unknown key, a line without `=`, or an existing file that cannot be read counts as
  `forbid`, and the node says `node.policy_not_understood` when it starts; the hub cannot change it). A start with a `skips_permissions` option that the policy does not allow is refused
  before anything starts with `agent.option_forbidden` `{kind, option, policy}` (`policy` is `forbid`, or `confined_only` when the
  agent's confinement, ADR-0047/0050, would not be active).
- **ACP registry kinds (ADR-0092, the maintainer's answers on #1133).**
  - *Always signed when they fetch.* A kind loaded from the registry's converted manifests (by origin, never by its `registry-` name
    alone) whose program is a package runner (`npx`, `uvx`, or any other of `registry_convert::PACKAGE_RUNNERS`, under any extension)
    fetches and runs code when it starts. A `registry-*` kind the node has not read yet (a manifest added after its folder was last
    read) is held to this too. Where the hub's commands enter the node, an unsigned
    routed `AgentStart` of one is refused with `node.policy_denied` `{node_id, rule: registry_signed_start, kind}`, whatever the
    node's `signed_commands` says and whatever grant is live (a grant lets the hub act for a while; it is not a person signing this
    start). It starts from a signed `AgentLaunch` (its `directory-action` signature names the kind), a helper start the person's signed
    decision approved (ADR-0057), a signed `AgentResume`, or the node's own control socket (the owner). The window always sends a
    signed launch; the terminal client's unsigned start in a project (`s`) sends the person to its signed folder start (`f`) for such
    a kind. A hand-made kind is never held to this, whatever its name or program, and neither is a registry kind that runs its own program
    (one the owner installed and the node finds on its `PATH`, with or without a SHA-256 in the registry: no code is fetched at start).
    An automatic resume the owner's `resume = auto` and `resume.kinds` lines name is the owner's own choice and is not refused.
  - *A policy line switches them off.* `agents.registry = on | off` (default `on`; quoted or not; any other value, or a policy file
    that cannot be read, counts as `off` and is a policy problem) and `agents.registry.deny = registry-a, registry-b` in the node's own
    policy file (names quoted or not; every deny line adds to the list; a name that is not one of this program's registry kinds is a
    policy problem). They apply to kinds of registry origin only.
    A registry kind switched off is not listed by `agent.kinds` and is not started by any path (`node.policy_denied`, rule
    `registry_off` or `registry_denied`, with `kind`); `agora doctor` says so (`registry-off`, `registry-denied`). Hand-made kinds are
    never affected. Read when the node starts.
- **What everyone sees.** `AgentStarted.launch_options` (30) and `skips_permissions` (31), carried by the hub in `AgentSnapshot`
  fields 30 and 31 (30-39 are kept for ADR-0059): a "does not ask" badge on the agent in every client for its whole life. The hub's
  audit row for such a start names the options (action `AgentLaunch[skip-permissions]`).

### Devices (ADR-0061)

All additions; nothing published changed. A device is a signing key a person uses (a browser's WebCrypto key, the terminal client's
key; ES256, ADR-0028). The hub keeps each person's **registered** devices; a node trusts devices. The hub's list is only a list of
candidates: no message from the hub makes a node trust a device. A node trusts a device when its owner says yes on the machine
(`agora node trust ID KEY`, or the question `agora node enroll` and `agora node trust --from-hub` ask about the registered devices),
or when a device it already trusts signs an **endorsement** of it. The hub only carries endorsements and revocations.

- **Registration.** `DeviceRegister` (request field 88: `device_id`, `name`, `public_key`, `signed_at_ms`, `signature`) is sent by a
  signed-in client when it connects; the hub answers it itself and keeps the device under the session's person (`id`, name, key,
  created, last used, and the proof). A device id matches `[A-Za-z0-9._-]{1,64}`, a name is at most 64 characters with no control,
  format, bidi or other hidden character (agora-core's list), a key is a 65-byte uncompressed P-256 point. **Proof of possession:**
  `signature` is the device's own ES256 signature by `public_key` over

  ```
  "agora-device-register-v1" 0x00 F(device_id) F(name) F(hex(public_key)) F(signed_at_ms)
  ```

  and `signed_at_ms` is within 10 minutes of the hub's clock (`device.signature_invalid` otherwise), so nobody can register a key they
  do not hold. `DeviceInfo.signed_at_ms` (10) and `registration_signature` (11) carry the proof to nodes, which verify it again before
  offering the device, so a hub cannot invent one either. (The person is not in the signed bytes: a browser does not know its hub-side
  id; the hub binds the device to the session's person, and nodes never rely on that binding for trust.) A device id stays bound to its
  key and its person: another key or another person under a registered id is refused with `device.key_mismatch`; sending it again
  updates the name and the time it was last used. At most 64 devices per person that are not revoked (`node.resource_limit`
  `{resource: devices}`); `agora hub devices forget ID` deletes a revoked one from the list (its revocations stay).
- **Listing.** `DeviceList` (request field 89) answers `DeviceListResult` (`CommandOk` field 22) with the caller's own devices
  (`DeviceInfo`: id, name, key, `created_ms`, `last_used_ms`, `user_id`, `approval_asked_ms`, `endorsed_by[]`, `revoked_ms`). A node
  asks too, on a TLS connection made with its certificate whose first frame is that `CommandRequest` (stream 2, kind 0, before any
  `Hello`): the hub answers once with every registered device and closes it, so the node's own connection is not replaced. Nothing
  else is answered on such a connection (`protocol.invalid_message`); without a certificate it is refused like anything but
  enrollment. The hub offers the capability `device.list`. `EnrollResponse.devices` (4) carries the same list at enrollment. A node is
  sent only the devices of the people who may act on it: the owner (sessions without a person and people with the owner role) and
  every person with a grant on one of its projects (ADR-0049).
- **Asking.** `DeviceAskApproval` (request field 92: `device_id`, `signed_at_ms`, `signature`) marks one of the caller's devices as
  waiting for another of their devices (`approval_asked_ms`). It is signed by that device's registered key over
  `"agora-device-ask-v1" 0x00 F(device_id) F(signed_at_ms)`, at most 5 minutes from the hub's clock, so only the device itself can ask; the person's other devices list it and show a card. The first endorsement of it the hub carries
  clears the mark.
- **Endorsement.** `DeviceEndorse { endorsement }` (request field 90) from a client: the hub checks that both devices are the caller's,
  that the key is the registered one (`device.key_mismatch`), that it lasts at most 90 days and has not expired
  (`device.endorsement_expired`), and that it verifies under the endorser's registered key (`device.signature_invalid`; verified before
  it is stored), keeps it once (the same signature is not stored twice), and sends the same message to every connected node with the capability `device.trust`
  (for an endorsement that names machines, only to those), and again to each such node when it connects while it has not expired.
  A viewer cannot send it (`auth.forbidden`). The signed bytes (`DeviceEndorsement.signature`, ES256 r || s) are

  ```
  "agora-device-endorse-v1" 0x00 F(device_id) F(hex(public_key)) F(name) F(count of nodes) F(node)... F(issued_at_ms) F(expires_at_ms) F(endorser_device_id)
  ```

  where `F(x)` is the decimal byte length of `x`, `:`, then `x` in UTF-8; numbers are decimal and the key is lowercase hex. A node
  accepts it only when: the endorser is a device it trusts there (and that trust has not expired; `device.untrusted`), the signature
  verifies (`device.signature_invalid`), this node is in `nodes` or `nodes` is empty (`device.not_in_scope`), now is before
  `expires_at_ms`, `issued_at_ms` is at most 5 minutes ahead of the node's clock and `expires_at_ms - issued_at_ms` is at most 90 days
  (`device.endorsement_expired`), no revocation of the device was signed at or after `issued_at_ms` (`device.revoked`), and the node
  does not already trust that id with another key (`device.key_mismatch`). Its trust there begins at the earlier of `issued_at_ms`
  and the node's clock (a revocation signed now always wins over an endorsement dated ahead). A device the node's policy limits to
  some projects (ADR-0049) endorses only within them: the new device is limited to the same projects (written as `projects=` on its
  line, so it survives restarts), and a limited device revokes only itself or devices whose limits are within its own
  (`auth.forbidden` otherwise). A device's limit is the policy's line for it, what it inherited, and its endorser's limit as it is now
  (up the chain), all at once: the policy narrows, never widens, an inherited limit, and a demoted endorser takes the devices it
  endorsed down with it; a renewal by use happens only while the endorser's current limit still covers the device. For a device the node already trusts through an endorsement: its own endorser may renew it, also after it
  ran out, while that endorser's current limit still covers what it was given (it keeps its endorser and limits); another endorser
  may take it over only if its current limit covers both the device's current limit and its current endorser's, and only with an
  endorsement whose start (the earlier of its `issued_at_ms` and the node's clock) is at or after the device's trust ran out
  (its effective expiry, or its own expiry when its chain is broken);
  `auth.forbidden` otherwise. A limit naming a project the file cannot hold (empty, or with a comma or a space) refuses the endorsement
  (`node.policy_denied`, rule `device.projects`). A revocation dated more than 5 minutes ahead of the node's or the hub's clock is
  refused. The hub carries an endorsement only to machines the device's person may act on, a revocation to every machine. It then
  trusts the device until `expires_at_ms` (never
  longer than its endorser's own trust there), and each use of the device on that node renews it to 90 days from the use.
- **Revocation.** `DeviceRevoke { revocation }` (request field 91): both devices must be the caller's; the hub keeps it and sends it to
  every node with `device.trust`, now and whenever one connects. Signed bytes:

  ```
  "agora-device-revoke-v1" 0x00 F(device_id) F(issued_at_ms) F(revoker_device_id)
  ```

  The hub verifies it under the revoker's registered key before keeping it, keeps each signature once, at most 256 per person
  (`node.resource_limit` `{resource: revocations}`), and forgets one after 400 days (every endorsement signed before it expired long
  before). A node applies one signed by a device it trusts at once: the device is no longer trusted there if its trust there began
  (`since`) at or before `issued_at_ms`, and neither are the devices it endorsed; endorsements of it, and endorsements it signed, at or before `issued_at_ms`
  are refused from then on. A revocation sent again later never removes trust given after it (an endorsement signed later, or the
  owner trusting it again on the machine with `agora node trust ID KEY`). Because a revocation may be dated up to 5 minutes ahead of
  the node's clock, the node keeps each verified revocation dated ahead of its clock when it arrives, identified by the fields its
  revoker signed (`device_id`, `revoker_device_id`, `issued_at_ms`; never by the signature), in `<data>/device-revocations-ahead`
  (node-local, at most 256 lines, the earliest-dated dropped first; a line nobody overrode is dropped once the clock passes its date).
  The owner trusting the device on the machine overrides every such revocation of it dated at or after that trust began (ADR-0081):
  sent again, it removes nothing (the node logs `overridden`) for as long as that trust lasts, and the command says so. A revocation
  the node had not received before that, or one signed afterwards (another date or another revoker), applies by the date rule above.
  Trusting a device again keeps its revocation remembered, so endorsements of it and by it signed at or before that revocation stay
  refused.
- **Fingerprint.** What people compare: the first 12 bytes (96 bits) of the SHA-256 of the 65-byte key, lowercase hex in groups of 4
  hex digits joined by `-` (`f947-3cb5-e304-81be-ecf2-64cd`). Every client shows it wherever a device is named, computed by the client
  from the key itself, never taken from the hub.
- **Trusted devices.** Event `NodeTrustedDevices` (`Event` field 13: `device_ids[]`, sorted, never keys), produced by a node when it
  starts and whenever the set changes; the hub keeps the last one per node in `NodeSnapshot.trusted_device_ids` (10), so a window can
  tell which machines trust its device. Node capability `device.trust`: the node verifies `DeviceEndorse` and `DeviceRevoke` and reports
  `NodeTrustedDevices`; the hub never sends either message to a node without it.

Shared vectors for the four signed messages and the fingerprint, checked by the node, the terminal client and the web window:
`schema/fixtures/signing/device-trust-v1.json`.

### Pairing (ADR-0065)

The hub answers these messages (`schema/agora/v1/pairing.proto`, `docs/specs/pairing.md` is the design and section 12 says how the hub core
behaves); a machine sends them with `agora join` (`agora_node::pairing_client`, `docs/specs/pairing.md` section 13). All additions; nothing published changed. Capability `pairing.v1` is offered by both the hub and the node
and means "speaks these messages"; a hub that cannot pair (it does not know its certificate's hash) answers `protocol.capability_missing`. A peer that gets a pairing command it does not handle answers
`protocol.capability_missing`, as for any command it does not know.

- **Commands** (`CommandRequest` fields): `PairOpen` (94), `PairClose` (95), `PairStatus` (96), `PairApprove` (97: `pair_id`, `replace`),
  `PairDeny` (98), all from an owner window to the hub; `PairRequest` (99: `commitment`, `node_name`, `os`, `version`) and `PairReveal`
  (100: `pair_id`, `nonce_n`, `csr_pem`) from a machine, accepted only on a pairing connection (like `enroll`). `pair_id` is 16 bytes.
- **Results** (`CommandOk` fields): `PairChallenge` (23: `pair_id`, `nonce_h`), `PairResult` (24: `approved`, `cert_pem`, `ca_pem`; the
  last two are empty unless approved), `PairStatusResult` (25: `open`, `closes_at_ms`, `pending`).
- **Standalone** (not in the command or event oneofs): `PairRequested` (`pair_id`, `kind`, `name`, `os`, `address`, `number` as six digits,
  `expires_ms`) and `PairDecided` (`pair_id`, `outcome`) are what a window renders as a card; they are memory-only state of a session and
  never events in the event store (the number is never logged). `HubAnnouncement` is the UDP discovery reply; the request is 128 raw bytes,
  not a message. `BrowserPairRequest`, `BrowserPairCreated` (the poll token is returned once, here), `BrowserPairPoll` and
  `BrowserPairState` are proto3 JSON over HTTP; the poll token travels in a request header and the session cookie in `Set-Cookie`. A
  browser approves or denies with `PairApprove` and `PairDeny`. Enums `PairKind` (machine, browser) and `PairOutcome` (approved, denied,
  expired, closed). The hub sends the two notices to owner sessions only, on the events stream, as frame kinds `PAIR_REQUESTED` (5) and
  `PAIR_DECIDED` (6) (`agora_protocol::frame::kinds`), memory only; other clients never receive them and clients ignore kinds they do not know.
- **Errors**: `pair.closed`, `pair.limit` (`seconds`), `pair.expired`, `pair.mismatch`, `pair.denied`, `pair.unknown`, `pair.state`
  (`state`), `pair.insecure_transport`, `pair.names_missing` (`reason`, `names`; the hub refuses `PairOpen` when it cannot name its addresses in its certificate), and the machine-local, never-sent `pair.name_unverified` (`name`), `pair.discovery_unconfirmed` and `pair.ca_mismatch`; see `error-codes.md`.

### Preview (ADR-0088)

Schema only so far (`schema/agora/v1/preview.proto`, `docs/specs/preview.md` is the design); no node or hub sends or handles these messages yet.
All additions; nothing published changed. Capability `preview.v1` is **not offered yet**: the node forwarder (build step 2) and the hub
relay (step 3) add it to their capability lists when they handle the messages, and a hub never sends a preview command to a node that did
not offer it. **Older peers refuse, they do not drop:** until then every peer answers `protocol.capability_missing`. The hub answers
`PreviewConnect` with `capability` = `preview.v1`; a node that does not handle a preview command answers, as for any command it does
not know, with `capability` = the command's name (`preview.open`, `preview.close`, `preview.list`, `preview.connect`). A client treats
either as "previews are not available here" and never assumes an open worked from silence.

- **Commands** (`CommandRequest` fields): `PreviewOpen` (102: `port`, `expires_seconds`, `authorization`), `PreviewClose` (103: `forward_id`, `all`;
  `all` with a non-empty `forward_id`, or neither, is `protocol.invalid_message {field: forward_id}`), `PreviewList` (104) and
  `PreviewConnect` (105: `forward_id`). `PreviewConnect` is opened on a dynamic stream like `TerminalSubscribe` (any other command on a
  dynamic stream stays `protocol.invalid_message`; the hub's own check for `PreviewConnect` is the capability refusal above).
- **Results** (`CommandOk` fields): `PreviewOpenResult` (26: `forward`), `PreviewListResult` (27), `PreviewConnectResult` (28:
  `max_chunk_bytes`, `window_bytes`, `rate_bytes_per_second`, `total_bytes_limit`); `PreviewClose` answers `empty`. A forward is a
  `PreviewForward { forward_id, port, opened_at_ms, expires_at_ms, creator, state, close_reason }`.
- **Events** (`Event` fields): `PreviewSuggested` (80: `agent_id`, `port`, `note`) is a fact "my app is on port N". It is shown as a
  card with an Open button and is never an open and never carries authority; the note is untrusted text. `PreviewChanged` (81: a
  `PreviewForward`) is written when a forward opens or ends, for the indicator and the audit trail (the port, never the content).
- **Stream frames** (dynamic stream opened by `PreviewConnect`, one stream per TCP connection to the forward's port; the hub relays
  frames without decoding them, like the terminal relay). Kinds 20..22 are new and do not reuse 0..3 (terminal) or 10..11 (chat):

  | Kind | Name | Payload | Direction |
  | --- | --- | --- | --- |
  | 20 | `PREVIEW_DATA` | raw bytes, at most `max_chunk_bytes` (default 16384) | both |
  | 21 | `PREVIEW_CREDIT` | `PreviewCredit { bytes }` | both |
  | 22 | `PREVIEW_END` | `PreviewEnd { reason, bytes_to_target, bytes_from_target }` | either, once, before kind 255 |
  | 255 | `STREAM_CLOSE` | empty | either (as for every dynamic stream) |

  Each side starts with `window_bytes` (default 262144) of credit toward the other and sends no `PREVIEW_DATA` beyond the credit it
  holds; the receiver grants more with `PREVIEW_CREDIT` as it drains. A frame over `max_chunk_bytes`, or data beyond the credit, ends the
  stream with reason `PROTOCOL`. `rate_bytes_per_second` and `total_bytes_limit` (0 = none) are ceilings the node enforces; going over
  ends the stream with reason `BUDGET`. Preview frames are the lowest priority (section 6): they never delay terminal bytes, and input
  outranks them. `PreviewCloseReason`: `CLOSED`, `EXPIRED`, `NODE_STOPPED`, `TARGET_GONE`, `BUDGET`, `PROTOCOL`, `POLICY`. Constants:
  `agora_protocol::frame::kinds::{PREVIEW_DATA, PREVIEW_CREDIT, PREVIEW_END}`.
- **Signing: `preview-action-v1`.** Through the hub a `PreviewOpen` carries a `PreviewAuthorization { device_id, nonce, expires_at_ms,
  signature }` (the shape of `DirectoryAuthorization`, defined in `preview.proto` because `commands.proto` imports it), signed by a
  device the node trusts (ADR-0020) over the domain, one NUL byte, and then every field length-prefixed, in the order the approval card
  shows them (the node, the port, how long) and the request carries them:

  ```
  preview-action-v1 0x00 F(node) F("open") F(port) F(expires_seconds) F(nonce) F(expires_at_ms)
  ```

  `F(x)` is the decimal byte length of `x`, `:`, then `x` (UTF-8, concatenated with no separator), as in the device messages and the
  ADR-0059 `signed_list`. Numbers are rendered canonically: decimal digits, no leading zeros, no `+`, no whitespace, `0` as `0`
  (`expires_seconds` is what the request carries, `0` for the default, so the signature covers exactly what was shown). The node and the
  nonce may contain any byte, a NUL included; the prefixes mean no two different tuples sign the same bytes. The node verifies, never the
  hub: the device is trusted there (ADR-0020), the signature verifies, `expires_at_ms` is in the future and at most 5 minutes ahead of
  the node's clock, and the nonce has not been seen. A nonce is single-use and the node keeps it until the signature's expiry
  (persisted, so a restart does not forget it). A signature made for another node, port or lifetime, or by another device, does not
  verify, so the hub can neither open a forward nor change its port or lifetime. An agent cannot send `PreviewOpen`; a suggestion is only
  an event. `PreviewClose` and `PreviewList` need no signature (closing only removes access; listing shows ports and creators to people
  who may see the machine). On the node's own control socket (the owner) no signature is needed. Vectors:
  `schema/fixtures/signing/preview-action-v1.json` (`crates/agora-protocol/tests/preview_signing.rs`, which also checks that fields holding
  NUL bytes cannot be shifted between the node and the nonce).
- **Errors**: `preview.port_refused` (`port`, `reason`), `preview.limit` (`limit`), `preview.expired` (`forward_id`),
  `preview.unknown` (`forward_id`), `preview.unavailable` (`reason`); see `error-codes.md`.

### Certificate renewal (ADR-0085)

All additions; nothing published changed. A machine renews its client certificate itself, over a connection it opens with the certificate
it has, so no code and no owner step is needed while it is still valid.

- **Messages** (`schema/agora/v1/enroll.proto`): `RenewRequest { csr_pem }` and `RenewResponse { cert_pem, not_after_ms }`. `CommandRequest`
  field 108 (`renew`) and `CommandOk` field 30 (`renew`) carry them. Golden fixtures: `RenewRequest` and `RenewResponse` in
  `schema/fixtures/v1/`, the two oneof arms in `schema/fixtures/enroll-renew/`.
- **Capability `enroll.renew`.** Offered by a hub that handles the command (the hub offers it from step 1). A node offers it too, from step 2 (ADR-0085), so that the hub's `Welcome` says whether the hub handles it: the intersection contains `enroll.renew` only when both do, and a node asks only then (an older hub never sees
  the message). A node offering it means "I understand renewal"; it is not a message the hub sends to a node.
- **Where it is accepted.** Only as the first command (`STREAM_COMMANDS`, kind 0) on a TLS connection whose client certificate verifies and
  that the hub lists as active (the connection of ADR-0061's one-shot questions). It is answered once and the connection ends. Without a
  certificate it is `auth.unauthenticated`; from a client it is `protocol.invalid_message`.
- **What the hub does.** The CSR comes from the machine, which makes a fresh key pair each time. The hub issues a certificate for the same
  node id as the connection's certificate, for client authentication only; any name, extension or usage in the CSR is ignored, exactly as at
  enrollment. It records the new serial and an audit row (action `enroll.renew`, node, outcome `ok serial=NEW presented=OLD`), and does not revoke the previous certificate:
  that stays valid until its own end, and the hub revokes a machine's older serials 7 days after the machine first connects with a newer one.
- **Only the newest certificate renews.** A machine's older certificate that is still valid (its grace) may connect but not renew: a copy of
  an old certificate and key could otherwise renew hourly, become the newest, and have the real machine's certificate revoked a week later.
  The hour limit is checked after that, so a copy of an older certificate cannot use up the real machine's hour.
- **Lost answer: an idempotent re-issue.** The hub keeps, for each certificate a renewal issued, its PEM (public data) and the SHA-256 of
  the public key of the request it was issued for (`devices.cert_pem`, `devices.csr_key_hash`, part of schema 13). When a machine presents
  a certificate that is not the newest and sends a `RenewRequest` whose request carries EXACTLY that public key, while the newest
  certificate was issued by a renewal and has never connected (`first_seen_ms` empty), the hub answers with that same certificate again:
  same serial, nothing issued, nothing revoked, no new certificate row, only an audit row `enroll.renew_resent` (`presented=OLD serial=NEW`).
  Resends are outside the hour limit for new certificates and limited to 5 an hour per machine (`too_frequent` after that). This gives a
  thief nothing: the private key of the request never left the machine, so someone holding only an older certificate cannot send that key,
  and any other request from an older certificate is `superseded`. Only the newest issued certificate is resent, and not after it has
  connected. **The node must keep the fresh key and the request of a renewal until it has received and stored the answer** (the next
  step's requirement), and send the same request again after a failure.
- **Refusals** are `enroll.renew_refused` with a `reason`, checked in this order: `unknown`, `revoked`, `expired` (the end the certificate
  carries has passed), `too_old` (the hub's record says it issued the certificate more than one current lifetime ago; it differs from
  `expired` when the certificate carries a later end than the hub's lifetime now allows, for example after the lifetime was lowered),
  `superseded` (a newer certificate of the machine exists), `too_frequent` (a certificate was issued for this machine within the last hour). A CSR that does not parse is
  `protocol.invalid_message` (`field` `csr_pem`).
- **Grace and limits.** `devices` records when each certificate was issued and when the machine first connected with it
  (`first_seen_ms`, schema 13). A machine's older certificates are revoked 7 days after that first connection with a newer one (checked at
  each connection and daily); an open connection on a revoked one is closed by the usual check. One certificate per machine per hour counts
  every certificate issued for it, enrollment included.
- **Refusals are audited** as `enroll.renew_refused` (node, outcome `reason=R presented=SERIAL`), at most one row per machine per reason per
  hour, so a flood cannot grow the table. `superseded` and `too_frequent` are the signal that someone holds a copy of a machine's certificate
  : the owner sees them in the audit view; a thief with the machine's CURRENT certificate
  and key is indistinguishable from the machine until the owner notices, and revocation (ADR-0015, ADR-0073) is the answer. A machine that
  lost the answer of a renewal asks again with the same request and is answered again (see Lost answer).
- **Clock.** The grace and the hour limit use the hub's clock. A clock that jumps forward can revoke a machine's older certificates early
  (only ones a newer certificate already replaced and that has connected); one that jumps back only delays it.
- **The node side (step 2, `agora-node` `link_renewal` and `cert_renew`).** A node looks at its certificate at start and then hourly. When
  a third of the certificate's own life is left (computed from its `notBefore` and `notAfter`; 30 of 90 days) and the hub's welcome listed
  `enroll.renew`, it makes a fresh key and request, **writes both to `<link dir>/renew-pending.pem` (private, whole-file replace) before it
  sends anything**, and sends the request over a one-shot certificate connection made with its current certificate. It keeps the file until the
  answer is stored; after a crash or a lost answer the next start sends the SAME request (the hub answers it again with the same
  certificate). An answer is used only when it chains to the pinned CA, is valid now, lasts at most a year, is for client authentication
  only and not a CA, names this node, carries the public key of the request and of the persisted key, and ends later than the certificate in
  use; otherwise it is dropped, the pair in use is kept, and the same request is asked again after an hour. A good answer is stored in this
  order: the pair in use to `previous.pem`, then `cert.pem`, then `key.pem`, then `renew-pending.pem` removed; a start that finds the
  certificate and key as two halves finishes the install from the pending file, or, when the key cannot be written, rolls back to
  `previous.pem` (only for the same node id as `node-id`; a link save removes `previous.pem`, `renew-pending.pem` and `renew-state` first);
  when nothing can be written the node still dials with a certificate and key that match (the previous pair, or the new certificate with the
  pending key from memory), never with two that do not. A pending request is bound to the fingerprint of the certificate it was made for
  and dropped when that is not the certificate in use. Leftover `<file>.tmp-<digits>` files of the folder's writers are removed at each load. The uplink reads the pair before
  each dial, so the next connection uses the new one, and `previous.pem` is removed once a connection made with the new certificate was
  accepted. Refusals are never final, because a hub can be at fault (a database restored from a backup answers `unknown` or `superseded`,
  a wrong clock `expired` or `too_old`): `unknown`, `revoked`, `expired` and `too_old` are remembered (`renew-state`, for `agora doctor`: the
  machine must join again, the reason and when the hub was last asked) and the hub is asked again about once a day (a tenth more at random,
  a fresh request each time); `superseded` is the same and logged as security relevant; a success clears the state. `too_frequent` waits
  an hour. An answer that does not check out is asked for again after an hour, five times in a row, then the request is given up and a new one
  is made after a day. A hub that cannot be reached or answers anything else is tried again with a growing wait (a minute, doubling, at most an
  hour) and logged once per change; a hub that welcomed the node without `enroll.renew` is asked about once an hour, logged once, and `agora
  doctor` says it does not support renewal yet. Text from the hub that is kept, logged or shown is cut to 200 characters without control
  characters, and a state file that cannot be read is ignored. Whether the certificate has ended is worked out from the clock at every look
  and never written down (a clock that jumps forward and back must not leave a verdict behind); a certificate that has ended is not asked
  about: only a new join helps.
  A revoked or unknown certificate is refused at the TLS connection before any command, so the node sees a failed connection, not the reason.
- **The hub's own certificate** is not a message: once at start and then daily the hub replaces its server certificate when fewer than 30 days
  remain (same names, same CA, a fresh key), rewrites `<hub dir>/server-cert.pem`, and new connections use the new one while open ones keep
  theirs (nodes pin the CA, not the certificate). The new certificate is switched on first; when `server-cert.pem` cannot be written afterwards the hub logs a warning and counts it, and keeps
  serving the new one. A failed round (or a panic) is logged at warn level and retried after 5 minutes, doubling up to an hour. A pairing
  session in flight keeps working across a swap: it froze the hash of the certificate its own connection was shown, and that connection keeps
  its own certificate; the hub's current hash is refreshed for later ones.

### A structured agent's conversation (ADR-0090)

All additions; nothing published changed. **Built (steps 3 and 4 of the ADR):** the ACP bridge and the node send and handle everything
below and advertise the four capabilities. The hub relays and gates them and keeps the last of each per agent (step 5); the clients (steps 6 and 7) are built after. A node that lacks a capability refuses `AgentSetConfig` and `AgentCancelTurn` with `protocol.capability_missing`, and the hub never
forwards either to a node that did not offer it.

- **Chat entries** (`ChatEntry`, `schema/agora/v1/commands.proto`). New `ChatKind` values `CHAT_KIND_THOUGHT` (6, the agent's thinking in
  `text`) and `CHAT_KIND_PERMISSION` (7, a marker). New fields: `call_id` (9), `tool_kind` (10, `ToolKind`, the ACP names), `status` (11,
  `ToolStatus`), `paths` (12, `ToolLocation { path, line }`), `output` (13) and `output_truncated` (14), `started_ms` (15) and `ended_ms`
  (16, node clock; the duration is their difference, `ended_ms` is 0 while the call runs), `plan_steps` (17, `PlanStep { text, status,
  priority }`), `queued` (18, a user entry waiting behind a turn), `approval_id` (19) and `permission_outcome` (20, `PermissionOutcome`).
  `text` keeps its old meaning for every kind (tool name and arguments as JSON; plan lines joined by `"\n"`), so an old client still reads a
  complete conversation. Unknown kinds and enum values read as `UNSPECIFIED`.
- **An entry changes in place.** A tool call and a plan are ONE entry whose later versions carry the same `entry_id`. A client that knows the
  node's capability `chat.rich` replaces the entry it holds; a client without it ignores the repeat (ADR-0038) and keeps the first version.
  The existing fixtures are not touched: the new fields have fixtures of their own in `schema/fixtures/chat-rich/`.
- **Events.** `AgentSessionConfig` (`Event` field 39: `agent_id`, `options[]` of `SessionConfigOption { id, name, description, category,
  type, current, choices[], skips_permissions }`; replaces the previous report; the reasoning effort is the option with category
  `SESSION_CONFIG_CATEGORY_THOUGHT_LEVEL`) and `AgentCommands` (`Event` field 66: `agent_id`, `commands[]` of `AgentCommand { name,
  description, input_hint }`; replaces the previous list). The hub stores and relays both and keeps the LAST of each per agent in its view (at most 64 KiB encoded, 64 options of 64 choices, 100 commands: a larger report is not kept and the one before it is dropped; gone when the agent exits, is interrupted or is forgotten). `AgentSnapshot` has no field for them (adding one needs a schema change), so a window that takes a snapshot is sent each kept report right after it, as the event the node sent, at the snapshot's `pos`; a window applies these two events, which only replace, although their position is not after its own. A window that resumes from a position gets them from the backlog as before. Only for agents the window may see.
- **Commands.** `AgentSetConfig` (request field 106: `agent_id`, `config_id`, `value`; capability `agent.set_config`) and `AgentCancelTurn`
  (107: `agent_id`, `drop_queue`; capability `agent.cancel_turn`), both the user's own and unsigned. Distinct from `AgentStop`: cancelling
  a turn ends no process. The hub audits and scopes them like `AgentSetModel`.
- **Queued prompts.** `AgentPrompt` is unchanged. A node with `chat.rich` answers it for an agent without a terminal with
  `AgentPromptResult` (`CommandOk` field 29: `queued`, `queue_len`); a node without it, and every terminal agent, answers `empty`, read as
  sent. The node queues a prompt that arrives while the agent's turn is open (its adapter declared `turns`, a prompt was sent and no `idle`
  followed), at most 8 per agent (`agent.queue_full`, `limit` 8), and sends them one per `idle`, in order. A queued prompt is a user entry
  with `queued` set until it is sent, then the same entry is sent again with `queued` unset. Clarification (#908): `at_ms` is the
  node's wall-clock time of the entry's event, never adjusted; for a queued prompt that event is its send, so the unqueued version
  carries the time it was sent (the id stays: ids are never reused). A client places the person's entries by `at_ms`, ties and
  out-of-order times broken by entry id, each just before the first other entry that is not older, and draws a prompt still waiting
  after everything; a client that saw the entry become unqueued may place it after the newest agent entry it held at that moment
  (times can tie within a millisecond). Otherwise two turns' answers read as one reply (RM-44). A tool call that is `pending` or
  `in_progress` when its turn is over will not finish; ACP has no cancelled status and leaves marking such calls to the client, so the
  window shows them as stopped once a later prompt was sent, or once the agent has been `idle` or finished for a moment (not while
  `unknown`), and the node sends no invented status. Prompts that will not be sent (the adapter
  ended, or `AgentCancelTurn` with `drop_queue`, or its send failed) leave the transcript and a notice `adapter.queue_dropped` (`count`,
  and since #908 `entries`: the ids of exactly those entries joined by commas, so a client marks only them; a notice without it covers
  every queued entry older than it) says so: the
  schema has no way to delete an entry, so a client that holds one clears its waiting mark when it sees that notice (a tombstone entry would
  need an ADR). The queue is node memory: it is lost when the node restarts (ADR-0089). A prompt that would wait is at most 64 KiB
  (`protocol.invalid_message`, `field` `text`); one sent at once is not bounded here. The turn is an explicit state, `Idle` or
  `Busy`: a sent prompt makes it busy; the adapter's `working` or `blocked` marks the turn as running; its `idle` ends the turn only
  after that (a stray or repeated `idle` ends nothing and never sends a prompt into a running turn), or, for an adapter that never
  says `working`, once 3 s have passed since the prompt (the clock ends it when its `idle` came earlier); `done` or `unknown` ends it
  whatever came before; every turn and every restart has a generation, and a send, a cancel or a failure that comes back to another
  generation changes nothing (each queued prompt is reported once: sent, or not sent, after its send ended); the user's own entry is
  in the transcript before the prompt is sent, so a fast agent never answers ahead of it; a queued prompt whose send fails is dropped (its entry leaves with the notice) and the next one goes; and a
  turn that was cancelled and that the adapter did not end is read as over 5 s after the cancel was taken. The sends of queued prompts
  are made off the node's tick and off the supervisor's lock: no adapter that hangs stalls the tick or another agent's command.
  `AgentSetMode`, `AgentSetConfig` and `AgentCancelTurn` likewise wait for the adapter after the lock is let go.
- **A permission entry is a view, never an answer.** `ChatEntry.approval_id` of a `CHAT_KIND_PERMISSION` entry is minted by the node (the
  id of the approval it queued, ADR-0020), never taken from agent text or from an adapter item; `permission_outcome` derives only from the
  node's approval state (pending, or how the user's signed decision, a cancelled turn or the agent leaving ended it). A chat entry can never
  answer an approval: approving and denying stay `ApprovalResolve`, signed and verified on the node.
- **Size caps the node must enforce (ADR-0090 build step 4), before the hub sees anything:** `output` 16 KiB per tool call (head and tail
  kept, `output_truncated` set), 8 KiB per thought entry, 64 plan steps of 512 bytes each, 64 `paths` per tool call, 100 commands, 64 config
  options of 64 choices, at most 8 queued prompts per agent, and at most one entry update per call per 150 ms; the ring bound of ADR-0038 still
  holds. Secret removal (ADR-0080) happens before an entry exists. The schema carries no limit itself: a peer that does not enforce them is
  not conforming, and a client truncates what it shows.
- **`skips_permissions` for options** is the adapter's flag or the node's own rule (`agora_adapters::perm`, `docs/specs/adapter-protocol.md`), never the
  agent's category alone: a denylist of words that say the agent acts without asking, and, in an option about permissions (category `mode`
  or a name that says so), an allowlist of the clearly asking or read-only values: a value the rule cannot place counts as skipping, so the
  unknown case is never read as safe (the window asks the user to confirm; a node whose policy forbids skipping refuses it). Neither the
  adapter nor the node can clear the other's flag. For a select the flag is on each choice; for a boolean it is on the option and holds for
  both values in the flag, but the node judges each value: a change TO the value that keeps protection (turning `auto_approve` off,
  `ask_for_approval` on) is never refused, and where the option's words do not say which value protects, both count as risky.
  `AgentSetMode` and `AgentSetConfig` of a choice that skips follow the node's `skip_permissions`
  policy: refused with `agent.option_forbidden` (`kind`, `option`: the mode, the model or the config id, `policy` `forbid` or
  `confined_only`) when the policy does not allow it. `confined_only` asks how the agent's own process was launched (#1009, as for
  "always", ADR-0101 section 7 item 7), never the node's sandbox setting now: an agent the holder runs is not confined, also under a node
  that confines now, and one with nothing recorded counts as not confined; the same holds for `AgentSetModel` below and for an
  approved switch when it is applied. When the policy allows it, the node neither applies nor refuses the choice: it raises an
  `agent.skip_permissions` approval and answers `agent.approval_needed { approval_id }` (next item). A choice that keeps protection is
  never held to any of this. (`AgentSetModel` of a model the node judges as skipping is still refused, `agent.option_forbidden` or
  `agent.option_unconfirmed`: Amendment A covers modes and config options.) All three commands are in the team-default `signed_commands`
  of a node with limited devices. The node forgets an adapter's reported options when it restarts, so a stale list cannot decide a choice.
- **A running agent switched to a mode or option that skips permission questions (ADR-0090 Amendment A).** Raised by the node, from a
  window's `AgentSetMode` or `AgentSetConfig` only (an agent, the mesh and the hub have no path to it); no schema message is new:
  - `ApprovalRequested { action: "agent.skip_permissions", agent_id, detail, expires_at_ms }`, expiring after 10 minutes like every
    request. `detail` is the node's own rendering, a JSON object: `node`, `agent` (the agent id), `agent_kind`, `folder` (the agent's
    folder on that machine), `option_type` (`mode` or `config`), `option` (for a mode the literal `mode`, otherwise the config id),
    `option_name` (the agent's own name for the option, untrusted, at most 64 characters, `""` for a mode), `value` (the mode id or the
    value chosen), `value_name` (the agent's own name for it, untrusted, same bounds), `skips` (always `true`: the node's own judgement
    that the choice makes the agent act without asking, never the agent's word) and `scope` (`session`: the choice lasts until the
    agent ends, restarts or is switched back). Every string that came from the agent or the machine is shown without control and
    format characters (Cc, Cf: bidirectional controls, zero-width characters, the byte order mark, line and paragraph separators);
    `agent_kind` is cut at 64 characters, `folder` at 256, the two names at 64, a cut marked by a final `…` inside the bound. `option` and
    `value` are never altered (they are signed as shown): a choice whose option id or value is empty, longer than 128 bytes or holds such a
    character gets no card and the command is answered `agent.option_unknown`. A window writes its own words from these fields (UI vocabulary, ADR-0024) and shows the
    ids `option` and `value` beside the agent's names, which are text a model chose.
  - The command is answered with the error `agent.approval_needed { approval_id }`; the choice is not applied. A repeat of the same
    command while that request waits gets the same id (one card per agent, option and value); a command naming another value of the same
    option withdraws the waiting card and raises a new one.
  - The decision is the existing `ApprovalResolve { approval_id, approved, signer_device_id, signature }` (`helper_grant` 0 and
    `helpers_may_start` false: anything else is refused), signed over the domain `agora-approval-skip-v1`, one NUL byte, then
    `F(node_id) F(approval_id) F(verdict) F(agent_id) F(option_id) F(value) F(sha256(detail))`, concatenated with no separator, where
    `F(x)` is the decimal byte length of `x`, `:`, then `x`; `verdict` is `1` or `0`; `agent_id`, `option_id` and `value` are the
    detail's `agent`, `option` and `value`; the hash is SHA-256 of `detail` exactly as announced, lowercase hex. Lengths are
    canonical decimal (no leading zeros, no sign); the fields are raw bytes (UTF-8 as given, no normalisation). Vectors:
    `schema/fixtures/signing/approval-skip-v1.json` (checked in `crates/agora-protocol/tests/approval_skip_signing.rs` and by the node).
  - The node verifies the signature against `trusted-devices` as for any approval (an untrusted device or a signature over other bytes is
    `approval.signature_invalid`), the id is single use (a replay is `protocol.invalid_message`, `field: approval_id`) and a late
    decision is `approval.expired`. Only an `approved` verdict applies anything, and it applies exactly the option and value the node
    rendered and stored when it raised the request, never anything in the decision. A denial, an expiry and a withdrawal change nothing.
  - Scope is the agent's session: the waiting request is withdrawn (announced as `ApprovalResolved { approved: false }` with no signer)
    when the agent ends, is stopped or its adapter crashes, when the option is set to any value by another command, and when it
    is superseded; an approval decided after the adapter restarted applies nothing and is answered `agent.not_found` (the run it named is gone, and the node's
    own check of what the agent offers fails). A node restart forgets every waiting request and every approval: the agent that comes
    back asks again. At most 8 requests wait on a node and at most 6 are raised per minute (`node.resource_limit { resource:
    "skip_approvals", limit }`), and one agent may have at most 2 waiting and raise at most 3 per minute (`resource:
    "skip_approvals_agent"`), so one agent's flood never keeps another agent's card from being raised; a replacement counts as a new card
    for these limits. Policy `forbid` and `confined_only` (the agent's process not launched confined) refuse with
    `agent.option_forbidden` before any request exists, and again when an approval arrives for an agent no longer launched confined. Switching back to a value that keeps protection needs no approval.
  - An adapter can never raise this card or any other node-made one: an approval request from an adapter whose action is in the `agent.`
    namespace (`agent.start`, `agent.skip_permissions`, ...) gets no card, consumes nothing and is answered "denied" at once.
  - Node capability `approval.skip_permissions`, advertised only by a node that handles the above and passed by the hub. A client that
    sees `agent.approval_needed` shows the card of the `ApprovalRequested` it already receives; against a node without the capability
    the command is still refused with `agent.option_unconfirmed`.
- **Capabilities:** `chat.rich` (entries that change in place, thinking, permission entries, queued prompts, `AgentPromptResult`),
  `agent.set_config`, `agent.cancel_turn` and `agent.commands` (`AgentCommands` events), advertised by every node of this build in
  `uplink_client.rs` and passed by the hub (`hubcli.rs`).
- **What the node does with each item.** A tool call is one entry, found again by `call_id` (an id over 256 bytes is dropped, not cut); the
  first sight sets `started_ms`, the first `completed` or `failed` sets `ended_ms` (the node's clock; a status that is not final clears
  it); a field an update leaves out keeps its value. The output of a call whose paths name a secret file (ADR-0080) is not kept, and a diff
  of one keeps its path and no text; the bridge has already replaced anything that looks like a secret in output. A plan is one entry
  replaced in place while it is in the ring (a plan step is never `FAILED`; an unknown status or priority word is `UNSPECIFIED`). A
  permission entry is made by the node when its own approvals queue accepts a request; it is `PENDING` and only the node's approval
  state changes it (once: a decision is never rewritten): `APPROVED` or `DENIED` by the verified decision, `CANCELLED` when the turn was
  cancelled, the agent was stopped or ended, or the request ran out of time. `AgentCancelTurn` withdraws the permission requests of the
  agent's adapter and announces them resolved as not approved; it does not touch an `agent.start` request the agent waits for.
- **Untrusted text.** Before an entry exists the node removes from thinking, tool name, arguments and output, paths, plan steps and
  permission labels every escape sequence (CSI, OSC, DCS and the like, with their payload), every control character (C0, DEL, C1) and
  the characters that reorder text; a one-line field (name, path, step, label) also loses its line breaks. Only a bounded head and tail
  (64 KiB) of an item is looked at, and all of this is done before the transcript's lock is taken. A notice code that only the node makes
  (`adapter.queue_dropped`) is dropped when an adapter sends it. A follower that falls 1024 entries behind is dropped and TOLD: its
  stream ends with a close frame (kind 255), which the uplink relays to the hub, so the client subscribes again and gets the ring as a
  fresh snapshot instead of keeping a transcript with a hole in it. Direction marks: the override and embedding controls (U+202A to
  U+202E) are always removed; in prose (thinking, output, plan steps) LRM, RLM, ALM and balanced isolates stay, as Arabic and Hebrew
  text needs them; in names, paths, labels and arguments every direction control goes. A lone carriage return is a line break (a space
  in a one-line field); an escape sequence that is never closed (CSI, OSC, DCS, ...) swallows at most to the next line break, which stays, or 4096 characters.
- **Coalescing.** At most one version of an entry is sent per 150 ms, repeats of a final status included (the first `completed` goes at once); a version held back is stored (a new subscriber sees it) and sent by
  the node's tick once 150 ms have passed; a final tool status is never held back. A client therefore may see fewer versions than the
  agent sent, never a stale last one.
- **Cost.** One entry costs one pass over at most its cap in bytes (a length test first), one clone per follower per version and one
  binary search of the ring (at most 500 entries). Nothing is added to a terminal's byte path.
- **Errors:** `agent.option_unknown` (`id`), `agent.queue_full` (`limit`); a refusal by the agent itself is `adapter.option_refused`.

### Plan limits of subscription agents (ADR-0091)
- **`AgentQuotaObserved`** (`Event` arm 67, capability `usage.quota`): `node_id` (1, the producer's own id, checked like `NodeResources`; a
  node naming another machine is refused as forged), `tool` (2, the agent kind, `codex`), `window` (3), `used_percent` (4, 0 to 100),
  `resets_at_ms` (5), `source` (6, `file`), `observed_at_ms` (7, the node's clock when it read a live line; for what it replays from the start of a file, the line's own time or the file's, never later than the node's clock). One observation of one plan window of a tool on a
  machine; the hub keeps the newest one per `(node_id, tool, window)`.
- **Hub table (ADR-0091 step 3):** at most 64 windows per machine (the oldest `observed_at_ms` goes, found among that machine's own windows);
  labels of 1 to 64 bytes; only for the machine that sent it. A reading replaces the stored one only when its `observed_at_ms` is not older.
  The hub cuts `observed_at_ms` back to its own clock plus 2 minutes when a session receives it (the stored event carries the cut time), so a
  node whose clock runs ahead cannot hold a window with a time in the future; the projection also never takes a time more than 2 minutes
  after the event's own `at_ms`. Clients cut a time ahead of their own clock the same way, and a client treats a missing or passed
  `resets_at_ms` as a reset.
- **Retention:** a forgotten machine's last readings stay in the table (nothing is sent for it) until the machine returns, and are
  rebuilt with the rest of the view when the hub restarts.
- **Privacy rule:** a plan limit is the use of a whole machine, not of a project. It is replayed and routed only to a window with no
  project scope (the owner's, full access). A member who was given some projects never receives it, even for a machine that hosts their
  project.
- **`window`** is `five_hour`, `seven_day`, or `minutes:<n>` (a length in minutes, `n` from 1), derived from the window's length and never
  from a field name such as "primary". A client must accept any `minutes:<n>`.
- **Bounds, by the node:** `used_percent` is clamped to 0..100; `resets_at_ms` is not more than a day in the past and not more than 31 days
  ahead of the node's clock, else the observation is not sent. An observation is sent when its numbers change, at most once per
  `(tool, window)` every 5 seconds.
- **What it never carries:** credits, balances, plan names, account ids, tokens, or any price. There is no field for them; a node forwards
  an allow-list of the four numbers and the label, and nothing else the tool wrote.
- **Fresh, reset, unknown are the client's call:** from `observed_at_ms` (its age) and `resets_at_ms` (after it passes the window is
  "reset", not the old percentage, until a new observation arrives); no observation is "unknown". A limit is never inferred.
- **Capability:** `usage.quota` is passed by the hub (`hubcli.rs`) and advertised by every node of this build (`uplink_client.rs`), which has the
  reader (manifest `[quota]`, `docs/specs/agent-manifest.md`; Codex only).
  Golden fixtures: `schema/fixtures/v1/agora.v1.AgentQuotaObserved.*` and `schema/fixtures/usage-quota/`.

### Giving a task to an agent (ADR-0095)
Schema step approved by the maintainer on 2026-11-21. Handled by every node of this build, which advertises `task.dispatch`; the hub
passes it (`hubcli.rs`). To an older node without the capability the hub refuses `TaskDispatch` with `protocol.capability_missing`.
- **`TaskDispatch`** (`CommandRequest` field 109, capability `task.dispatch`): `project_id` (1), `task_id` (2), `agent_id` (3, an agent
  of the receiving node), `by_user` (4). The node claims the open task for that agent where the project's board is (its broker; for a
  shared project the hub's board, as the agent's own `task_claim` would, ADR-0039), sends the agent one prompt built from the node's
  template around the task's title and detail (`agora_mesh::dispatch::build_prompt`: a fenced block marked untrusted task data, cleaned and
  cut to the mesh message limit), not typed over a running turn as far as the node can tell, and writes `TaskDispatched`.
  A structured agent gets it as an `AgentPrompt` would (sent now, or queued behind the turn, `agent.queue_full` past 8). A terminal agent
  has no queue, and its prompt always arrives as one bracketed paste (`ESC[200~` … `ESC[201~`, then Enter), so a line break in it never
  submits part of it: a terminal program that has not turned on bracketed paste (DECSET 2004, as the node's terminal emulator sees it) is
  refused with `protocol.capability_missing` (`capability`: `terminal.bracketed_paste`) before anything is claimed; the prompt holds no ESC
  or other control character but line break and tab, so its text cannot end the paste. The agent's one slot is reserved
  before the task is claimed, so a second dispatch meanwhile is `agent.queue_full` (`limit` 1) with nothing claimed. After the claim (which
  may wait for the hub) the node looks at the agent's screen again and decides: its rules are run now on the live bottom of the buffer (never
  the user-visible viewport, and never the paced detection's last result, which a screen no rule names, such as a dialog, leaves as it was;
  a screen no rule matches is `unknown`). Between turns (idle or done, and nothing pasted into it in the last 5 seconds) it is pasted at
  once; otherwise the node holds it. A held prompt is pasted only after the same fresh look says the agent is between turns, taken right
  before the paste, for agents with a held prompt only and no more often than that agent's own detection looks at its screen
  (its pacing interval). That look is separate from detection's own, so an agent with a held prompt may have its screen read up to
  twice per interval (once by detection, once by this check) for as long as the prompt is held (at most 15 minutes). `unknown` counts as
  between turns only for a kind whose manifest has no `[[rules]]` entry with `state = "idle"` or `"done"` (it never reaches either, and
  would otherwise never get a held prompt): for such a kind the paste, and the Enter after it, may arrive while the program works or while
  it shows a dialog (an Enter can answer it). A kind that can say idle or done and is `unknown` (starting, or a screen no rule names, such
  as a startup dialog) is waited for. A prompt is held at most 15 minutes: an agent that stays busy, blocked or unknown longer gets it
  dropped and its task on the node's broker given back, so a stuck agent does not keep the task and its slot. A program that turns
  bracketed paste off after the check is refused the same way (`protocol.capability_missing`) when the prompt would be pasted at once,
  and its held prompt is dropped (logged on the machine) with the task given back. A paste that
  was written but whose Enter failed means the terminal is gone (a write fails only then), so no input box keeps the text; its task is given
  back like any lost prompt.
  A prompt that never reaches the agent gives its task on the node's broker back: a held one when the agent is no longer running (stopped
  by a person or its parent, ended, crashed; checked every supervision round, so every stop path is covered), a paste that cannot start or
  fails, a queued one that the adapter refused or dropped (the adapter ended or was stopped, by a person's `AgentStop` too, or `AgentCancelTurn` with `drop_queue`; a
  queued prompt is settled only by the queue's own reports, so one reported sent keeps its claim even when the agent ends in the same
  moment). Held prompts live in
  memory: a node restart drops them and leaves their claims. For a shared project the node asks the hub's board as the agent (`task_list` with `task_id`, which answers that one task, for its text
  and status, then `task_claim`; an older hub ignores `task_id` and answers the whole board, in which the node finds the task the same
  way; a hub of this build answers a board result too big for one frame with the tool error `mesh.body_too_large` instead of sending
  it, which a dispatch passes on as its refusal; an older hub still sends it, and one over the frame limit ends that node's connection
  as an agent's own `task_list` of such a board always did)
  and still builds the prompt itself; a project known to be shared with no hub link is `board.unavailable`. A prompt that cannot be sent gives a task on the node's own broker back; the shared board has no release yet, so
  there the task stays claimed by that agent. Refusals: `mesh.recipient_unknown` (the agent is not of `project_id`, or the node has no mesh),
  `task.not_found`, `task.blocked`, `task.not_open` (also a second dispatch of a claimed task), `agent.queue_full` (`limit` 8 behind a
  structured agent's turn, 1 for a terminal agent's held dispatch), `agent.not_found` (the agent stopped while its task was being claimed or
  pasted; the task is given back), `board.unavailable` (a shared project whose hub cannot be asked), `protocol.capability_missing`
  (`capability`: `terminal.bracketed_paste`, a terminal program without bracketed paste). Not signed and not an approval: it changes no permission
  of the agent, and every sensitive action the agent later asks for still needs the person (ADR-0020). Like `AgentPrompt` it needs the
  sender's access to the agent (a viewer may not send it; a scoped window needs the project and the agent); on a node with limited
  devices it is in the team default of `signed_commands` and a grant covers it by the agent's own project (ADR-0041).
- **`by_user`** is written by the hub from the window's sign-in; whatever a client puts there is replaced. The node cannot tell a command
  the hub carried from one sent on its own control socket (the uplink runs the hub's commands through that socket), so a local process
  that can open the socket can write any `by_user`. The hub's audit row of the command is the record of who asked; the event carries
  the node's copy, which is the sender's word.
- **`TaskDispatchResult`** (`CommandOk` arm 31): `queued` (1) and `queue_len` (2) as in `AgentPromptResult`, `prompt_sha256` (3, lowercase
  hex SHA-256 of the prompt's UTF-8 bytes).
- **`TaskDispatched`** (`Event` arm 68): `project_id` (1), `task_id` (2), `agent_id` (3), `by_user` (4), `prompt_sha256` (5), `queued` (6),
  `agent_skips_permissions` (7, the agent acts without asking as the node knows it at the dispatch: a launch option that skips permissions,
  ADR-0059, or for a structured agent the mode or a config option in use that does, ADR-0058, ADR-0090 Amendment A). The prompt itself is not stored. The task's
  move is its own `TaskChanged` as before. Names no machine (the agent is the producer's); a window sees it when it may see the agent.
- **Limits:** at most 8 tries per minute per person, counted by the hub across all that person's windows and connections, every try that
  passed the access check, also one the node then refuses (`task.dispatch_limited`, `limit`); one dispatch per task at a time follows from
  the claim. A command sent on a node's own control socket is not counted (it never reaches the hub).
- Golden fixtures: `schema/fixtures/v1/agora.v1.TaskDispatch*.*`, `agora.v1.TaskDispatched.*` and `schema/fixtures/task-dispatch/`.

### A kept agent's counters (ADR-0104)

One appended field; nothing published changed. No capability.

- `AgentStarted.same_process` (8): this `AgentStarted` re-states an agent whose current process the node has announced before under the
  same id: one it took up from the holder after its restart (ADR-0079, ADR-0093), or one a resync announcement re-states (ADR-0054,
  ADR-0055, `Announce::Present` and `Announce::Restart`). It is false for a new start, a resume (ADR-0060) and a restart after a crash.
- **The hub** keeps, when it is set and the agent is already listed from that producer (the key is `<producer>/<agent>`), what it summed
  for the agent: `input_tokens`, `output_tokens`, `model`, the context level and size, and its own reported cost and currency
  (`AgentSnapshot` fields 18-21). Everything else starts again as for any start: the session's options, config, commands and always
  answers (the node re-states them right after), its session id, state and interruption. Usage reported after it adds to the kept
  totals, so `usage.report` keeps the agent's earlier usage too. A view rebuilt from the log folds the same totals.
- **The window** folds it the same way: the agent's tokens, context level, own cost and the time it saw the agent start stay.
- **Compatibility.** An older hub ignores the field and starts the counters again from 0, as before. An older node never sets it, so a
  newer hub does the same. The window is served by its hub. The terminal client draws the hub's snapshot, which carries the kept totals.
- Golden fixture: `schema/fixtures/same-process/` (`crates/agora-protocol/tests/same_process.rs`); the published `AgentStarted`
  fixture is unchanged and reads as `same_process` false.

### Allowing a structured agent always for its session (ADR-0101)
Schema step approved with ADR-0101 by the maintainer on 2026-11-22. Handled by every node of this build, which advertises `approval.always`;
the hub passes it (`hubcli.rs`). The window does not branch on it: the evidence it uses is ALWAYS in a request's `scopes`, which only such a
node sets. The window's button and the terminal client's key come with the next step; until then every client sends single answers.

- **Enum** `ApprovalScope` (`common.proto`): `APPROVAL_SCOPE_UNSPECIFIED` (0, read as ONCE: what every decision meant before), `ONCE` (1),
  `ALWAYS` (2: the agent remembers the answer, as the agent defines it, ACP `allow_always`). A value this build does not know is refused
  (`protocol.invalid_message`, `field: scope`), never read as either.
- **The offer.** `ApprovalRequested` / `ApprovalSnapshot` gain `scopes` (6), `always_label` (7) and `always_option_id` (8): the answers the
  person may give after the node's policy (empty: ONCE only, as every node before said it), the agent's own name for its always option
  (untrusted text, control and format characters removed, at most 120 characters) and that one option's exact id (never altered: an id
  that is empty, longer than 128 bytes or holds such a character is never offered). The node takes them from the adapter's
  `approval/request` (`docs/specs/adapter-protocol.md`), only from an adapter that declared `approval_scopes`.
- **The decision.** `ApprovalResolve.scope` (7). UNSPECIFIED and ONCE are signed over `agora-approval-v1` exactly as before. ALWAYS, only
  with `approved` and never on an `agent.` request, is signed over the domain `agora-approval-always-v1`, one NUL byte, then
  `F(node_id) F(approval_id) F(verdict) F(agent_id) F(sha256(action)) F(sha256(detail)) F(always_label) F(always_option_id)`, where `F(x)` is
  the decimal byte length of `x`, `:`, then `x` (canonical decimal, raw bytes); `verdict` is `1`; `agent_id` is the node's own id of the
  agent (`ApprovalRequested.agent_id` as the node sent it; in an `ApprovalSnapshot` the part after `<node_id>/`); the hashes are of
  `action` and `detail` exactly as announced, lowercase hex; the label and option id are as announced. Vectors:
  `schema/fixtures/signing/approval-always-v1.json` (checked in `crates/agora-protocol/tests/approval_always_signing.rs` and by the node).
- **The node checks, in order:** the shape (an unknown scope; ALWAYS with a denial, a grant or on an `agent.` request is
  `approval.signature_invalid`); the signature against `trusted-devices`, over the bytes the decision's scope names, so a hub that turns
  ONCE into ALWAYS or ALWAYS into ONCE fails here (`approval.signature_invalid`); the offer it stored (an Allow on a request that offers
  only ALWAYS, or ALWAYS on one that does not offer it: `approval.scope_unavailable`; a Deny needs no offer); the policy as the node
  read it at start, for the agent's own launch (`approval.scope_forbidden`); for an agent behind an ACP bridge that did not declare `approval_scopes` (one from before ADR-0101, kept
  across a node update) a single Allow while the policy refuses ALWAYS (`approval.adapter_outdated`: that bridge may answer it with the
  agent's remembered option); then expiry and single use. A refusal before expiry spends nothing: the card keeps waiting.
- **Policy.** The existing `skip_permissions` line (ADR-0059), read from the node's policy file at node start: `forbid` refuses every
  ALWAYS, `confined_only` refuses it for an agent whose process was not launched confined (one the holder runs never is; an unknown launch
  is refused), `allow` permits it; a kind whose manifest says `[structured] allow_always = false` is never offered it. Under a refusing policy
  the node leaves ALWAYS out of `scopes`. A request that offers only ALWAYS under such a policy gets no card: the node announces it and at
  once `ApprovalResolved { approved: false }` with no signer, answers the adapter denied (the agent gets `reject_once` or `cancelled`) and
  adds the conversation notice `approval.always_only_forbidden` (params `action`).
- **The record.** `ApprovalResolved.scope` (6): ALWAYS for an always decision the adapter delivered, ONCE for any other person's decision,
  UNSPECIFIED for one nobody decided. `ChatEntry.permission_always` (21) marks the permission entry of an approved ALWAYS. The new event
  `AgentAlwaysAllowed` (`Event.body` 69, producer `node`: `agent_id`, `approval_id`, `action` cleaned and cut at 120 characters,
  `signer_device_id`, `at_ms`) is written only after the adapter delivered the always answer to the agent (its `approval/resolve`
  result says so: the ACP bridge answers once its writer wrote the answer to the agent's stdin; a refused or undelivered one records the
  request cancelled, as before), and only while the run of the agent's process it was delivered to is still the agent's (a former
  process's late result is audited in `ApprovalResolved` but marks no replacement). The node keeps an agent's always answers beside its kept session (`kept-<id>.pb`, at most 16) and says
  them again after the `AgentStarted` it restates (a resync, or a node that takes a kept agent up); a new process of the agent, its end
  and its interruption drop them, and a resumed agent's are not replayed.
- **The hub** relays `scopes`, `always_label` and `always_option_id` into `ApprovalSnapshot`, and derives `AgentSnapshot.always_allowed` (45):
  the actions of the agent's `AgentAlwaysAllowed`, newest last, at most 16, a repeated `approval_id` once; cleared by its `AgentExited`,
  `AgentInterrupted` and every `AgentStarted` (the restated answers that follow a restated start bring them back). A window sees
  `AgentAlwaysAllowed` when it may see the agent.
- **Compatibility.** An old window sends no scope: single answers work as before, except an Allow on a request that offers only ALWAYS
  (`approval.scope_unavailable`, the card stays; Deny works). A new window facing an old node sees no `scopes` and offers no always answer.
  An old hub drops the new fields, so its windows offer single answers only; an ALWAYS that crossed it would arrive as ONCE and fail the
  signature. An ACP bridge kept across a node update (ADR-0075, ADR-0093) until its agent restarts may still answer a single Allow with the
  agent's remembered option when the agent offers no single one; under `allow` that residual is kept (named in ADR-0101), under `forbid`
  or `confined_only` the node refuses such an Allow (`approval.adapter_outdated`).
- Golden fixtures: `schema/fixtures/v1/agora.v1.AgentAlwaysAllowed.*` and `schema/fixtures/approval-always/`.
