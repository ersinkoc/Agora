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
its waiting approvals and its terminals; a resync announcement then re-adds what still runs.

SSH stdio transport (ADR-0011): `agora node stdio` copies the node's local-socket frames to stdin/stdout, so a hub (or
a person) can reach a node with `ssh host agora node stdio` and no open port. Frames are exactly those of the local
control socket; the node's agents and policy are unchanged and nothing depends on the hub.
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
or 0, with `last_heard_ms` the node's last time in the log). The snapshot's `NodeSnapshot.last_event_ms` includes `last_heard_ms`, so it is "last seen".

Schema additions (ADR-0025/0026/0027, all new messages, nothing published changed): `agent.list` (`AgentList` -> `AgentListResult` of
`AgentSummary`; implemented on the node and as `agora agent list`), `EventAck` (hub -> producer, cumulative), `client.proto`
(`Subscribe`, `Snapshot` with `*Snapshot` tables mirroring the hub view, `PositionedEvent`; named `*Snapshot` because
`AgentState` is already the enum) and `enroll.proto` (`EnrollRequest`, `EnrollResponse`). Golden fixtures added for each.
Frame kinds on streams 0 and 2 are the ADR-0026 table.

Hub service (`agora-hub::server`, ADR-0011/0026/0027): one TLS port. A client certificate from the hub CA that is not revoked makes the
peer a **node** (named by the certificate): `Hello`/`Welcome`, `Resume`/`ResumeOk`, then `Event` frames; the hub answers with a cumulative
`EventAck` after each batch and when idle, and a `ResumeOk { from_seq }` on a gap. A peer **without** a certificate is a client if its first
frame is `Subscribe` and its `Authorization: Bearer` token (or, for browsers that cannot set headers, the subprotocols `agora.v1, agora.bearer.<token>`; the hub echoes only `agora.v1`) is accepted (`Snapshot` or backlog, then `PositionedEvent`s; too slow = dropped,
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
other command on a dynamic stream, or a subscription on stream 1, is `protocol.invalid_message`). The hub reserves a dynamic
stream on the node's connection, forwards the request on it, answers on stream 1, and then copies the node's terminal frames
(kinds 0..=3 above) to the client's stream without decoding or storing them. Input and resize are ordinary routed commands.
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
| agent | `AgentStarted`, `AgentStateChanged`, `AgentSessionReported`, `AgentUsageReported`, `AgentExited` |
| terminal | `TerminalOpened`, `TerminalTitleChanged`, `TerminalClosed` |
| approval | `ApprovalRequested`, `ApprovalResolved`, `AgentStartRefused` (ADR-0057) |
| mesh | `MessagePosted`, `TaskChanged`, `LeaseGranted`, `LeaseReleased`, `HandoffCreated` |
| files | `FilesChanged` (batched paths, from watch) |

## 4. Commands (client → hub → node)

Request/response with `request_id` and `idempotency_key`. Responses are `Ok { result }` or
`Err { code, params }` (see `error-codes.md`).

Initial command set: `project.list`, `agent.start`, `agent.stop`, `agent.prompt`, `agent.set_mode`,
`terminal.open`, `terminal.input`, `terminal.resize`, `terminal.subscribe`, `files.list`, `files.read`,
`files.write`, `files.watch`, `git.diff`, `git.status`, `worktree.create`, `worktree.remove`,
`approval.resolve` (signed, ADR-0020), `mesh.post`, `task.update`, `node.policy.get`.

Implemented by the node today: `project.list`, `agent.*`, `terminal.*`, `approval.resolve`, `mesh.post`, `agent.list`, `agent.kinds`,
`chat.subscribe` (ADR-0038: a conversation stream, below), `git.status`, `git.log` (ADR-0037: newest first, default 30, at most 200), `git.diff`, `files.list` and `files.read`. The git and files commands answer only for the workspace of one of the node's own agents (`worktree_id` is the id
the node reported in `WorktreeCreated`); a path that is absolute, climbs with `..` or starts with `:` or `-` is refused as
`protocol.invalid_message`. `git.diff` is the unified diff against HEAD plus the new files git does not track yet (shown as
all-added, at most 50 files of at most 64 KiB), and an answer is cut at 512 KiB. It covers the workspace folder only and names
every file from that folder, so each name is a path `git.diff` and `files.read` take: for a workspace that is the folder `web/`
of a repository, `web/a.txt` is `a.txt`; a rename across the folder's edge shows as the file added or deleted inside it. At a
repository's root the names are the repository's own. `files.list` hides `.git`; `files.read` refuses
files above 512 KiB (`files.too_large`) and paths that leave the workspace, symlinks included (`files.outside_root`), checked again on the opened file so a link swapped in after the check is still refused.

Secret files (ADR-0080). The node decides by file name (case-insensitive, any folder): `.env` and `.env.*` (not `.env.example`, `.env.sample`, `.env.template`), `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.kdbx`, `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519` (not their `.pub`), `.npmrc`, `.pypirc`, `.netrc`, `.git-credentials`, anything inside a folder named `.ssh`, `.aws` or `.gnupg`, and `.docker/config.json`. Added to the schema (old clients ignore them, so no capability): `FileEntry.secret` (4), which `files.list` sets on such entries (it still lists them); the error `files.secret` (`path`), which `files.read` answers for the requested path or for where a link leads; `GitDiffResult.secret_files` (2, `SecretFileChange { path, added, removed }`) and, in `diff`, a changed secret file as its `diff --git` line plus mode and rename lines, with no hunk, blob id or content. The change counts of `git.status` are unchanged. The owner's policy line `files.show_secret = true` (any other value is off; the hub cannot change it) turns all of this off for the machine; `agora doctor` says when it is on.

### Selected directories (ADR-0052)

Capability `workspace.directory` adds commands without changing `AgentStart`: its empty `worktree_id` still means
create a worktree. New clients refuse an unavailable directory launch rather than falling back to isolation.

| New command / request oneof tag | Fields and result |
| --- | --- |
| `directory.inspect` / `DirectoryInspect` (81) | `path`, `create_project_id`, `authorization` → `DirectoryInfo` (Ok tag 18): canonical `path`, `project_id`, `git_root`, `branch`, `portable`, `previous_project_id` |
| `directory.list` / `DirectoryList` (84) | `path` (empty: the user's home folder), `authorization` → `DirectoryEntries` (Ok tag 21): canonical `path`, `parent` (empty at a root), `entries[]` of `{name, is_git}`, `truncated`. Folders only, sorted by name, at most 500 per answer; folders ADR-0052 refuses to launch in are not listed; capability `directory.list` (ADR-0053) |
| `agent.forget` / `AgentForget` (85) | `agent_id`; answered by the hub, which appends `AgentForgotten` (event 36, `agent_id` is the hub-wide key `<node>/<id>`) and stops listing the agent; `agent.running` while it runs; the earlier events stay in the store (ADR-0053) |
| `node.forget` / `NodeForget` (93) | `node_id`, `refuse_return`; answered by the hub, owner only (`auth.forbidden` otherwise), with no `target_node` (`protocol.invalid_message` otherwise). `node.online` while the machine is connected, `node.unknown` for one the hub never saw. With `refuse_return` every certificate enrolled under `node_id` is revoked first. The hub appends `NodeForgotten` (event 15, producer `hub`): its snapshot leaves out that machine and everything keyed under it (folders, worktrees, terminals, approvals, agents); a later `NodeLink { up }` (or `NodeOnline`) of the same node lists it again with the agents it still runs, and the ones closed before stay forgotten. A client that joined while the machine was hidden holds none of its records and no event repeats them, so when it comes back the hub lets every live client go, and a client resuming from a position before that return gets a snapshot instead of a replay (`View::remembered_at`, rebuilt at start). History and usage stay (ADR-0073). The web window is served by its hub, so it always matches it; there is no separate client capability |
| `agent.launch` / `AgentLaunch` (82) | canonical `path`, expected `project_id`, `kind`, `separate_worktree`, `authorization`, `accept_identity_change` → `AgentLaunchResult` (Ok tag 19): `agent_id`, `terminal_id`, `workspace_id`, actual cwd `path` |
| `workspace.query` / `WorkspaceQuery` (83) | `workspace_id`, `operation` (`list`, `read`, `status`, `diff`, `history`), relative `path`, `limit` → existing file/Git results |

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
and HEAD for the workspace. The counts (here, in `GitStatusChanged`, `git.status` and the mesh's `files_changed`) cover the
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
so it can be driven with no hub. Frames are the envelope above. `CommandRequest` goes on stream 1 with kind 0 and
`CommandResponse` comes back on stream 1 with kind 1, matched by `request_id`; a retry with the same
`idempotency_key` gets the stored response (addressed to the retry) and repeats nothing. A terminal stream is
opened by sending `TerminalSubscribe` on a dynamic stream id (3+) chosen by the client. After the response the
node writes that terminal's frames on that stream: kind 1 snapshot (ANSI that repaints the screen, with its colours and
attributes, then restores the program's bracketed-paste and application-cursor modes and its current style), each one followed by kind 2 with the size it was painted at, kind 0 raw
output, kind 2 resize (cols, rows as u16 LE), kind 3 exit (i32 LE; negative = signal). A client that falls behind
gets a fresh snapshot instead of the dropped output; it never slows the terminal. When the exit was among what it
missed it still gets the exit frame, with `i32::MIN` (unknown) when the node no longer has the code. Input and resize are ordinary
commands. Terminal ids are `t-<agent id>` (one terminal per agent). Implemented commands: `project.list`,
`agent.start`, `agent.stop`, `agent.prompt`, `terminal.input`, `terminal.resize`, `terminal.subscribe`; the rest
answer `protocol.capability_missing`. `agent.list` (ADR-0025) and `agent.kinds` (ADR-0030: the node's built-in and folder manifests, one per kind (agent-manifest.md, "Where agent kinds come from"), with display name, integration level and whether the launch program is on `PATH`; invalid manifests omitted; `agora agent kinds`). ADR-0069 (capability `kinds.program`): each kind also names `program`, the file name the node looks for on its `PATH` (never a path, and never the node's `PATH` itself), so a kind that is not installed can say what is missing; `NodeOnline.binary` (and `NodeSnapshot.binary`, from the node's latest start) is the node's own executable only when its folder is not on the node's `PATH`, empty otherwise, so a client can say to run `agora` from that folder. It is a full path that may contain a user name: only the owner receives it whole; the hub sends members and viewers its file name alone (snapshot, live and replayed events alike), since any folder, the one it is in included, may be the user's home; the owner's client shows the last two components unless asked for the whole path. Old nodes send both empty.

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

`MeshPost.refs` (ADR-0035, field 5): the sender's refs as written; the receiving broker qualifies them with the hub-vouched `origin`.

`BoardCall` (command 62, node → hub on the stream-1 request path like `MeshPost`) and `BoardResult` (`CommandOk` field 16), ADR-0039: a mesh tool on a shared
project's board; see `docs/specs/mesh-tools.md#shared-board-adr-0039`. New error code `board.unavailable`.

`BoardJoin` (command 66, hub → node only; a client's is refused) and `BoardJoinResult` (`CommandOk` field 20, with `BoardJoinLease` and `BoardJoinTask`), ADR-0039 amendment B: when a project becomes shared, the hub asks each node with a checkout for its unfinished local work and puts it on the board. Sent only to nodes that offer the capability `board.join`.

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
  (node and web window). The helper starts with exactly the settings shown and signed, and never with a launch option. The node enforces "no grant" itself: a signed `helper_grant` or `helpers_may_start` on a request with settings is ignored.
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
  socket (the owner) no signature is needed. An agent's `agent_start` helper (ADR-0057) never gets an option.
- **Node policy.** `skip_permissions = forbid | allow | confined_only` (quoted or not) in the node's own policy file (default `allow`
  when there is no file; any other value, an unknown key, a line without `=`, or an existing file that cannot be read counts as
  `forbid`, and the node says `node.policy_not_understood` when it starts; the hub cannot change it). A start with a `skips_permissions` option that the policy does not allow is refused
  before anything starts with `agent.option_forbidden` `{kind, option, policy}` (`policy` is `forbid`, or `confined_only` when the
  agent's confinement, ADR-0047/0050, would not be active).
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

### A structured agent's conversation (ADR-0090, schema only)

All additions; nothing published changed. **Schema only:** no node, bridge or window sends or reads these yet (build steps 3 to 7 of the
ADR), and no capability below is advertised by any build. A node refuses `AgentSetConfig` and `AgentCancelTurn` with
`protocol.capability_missing`, and the hub never forwards either to a node that did not offer its capability.

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
  description, input_hint }`; replaces the previous list). The hub stores and relays both and keeps nothing of them in its view yet.
- **Commands.** `AgentSetConfig` (request field 106: `agent_id`, `config_id`, `value`; capability `agent.set_config`) and `AgentCancelTurn`
  (107: `agent_id`, `drop_queue`; capability `agent.cancel_turn`), both the user's own and unsigned. Distinct from `AgentStop`: cancelling
  a turn ends no process. The hub audits and scopes them like `AgentSetModel`.
- **Queued prompts.** `AgentPrompt` is unchanged. A node with `chat.rich` answers it with `AgentPromptResult` (`CommandOk` field 29:
  `queued`, `queue_len`); a node without it answers `empty`, read as sent. The queue holds at most 8 prompts.
- **Capabilities (not advertised yet):** `chat.rich`, `agent.set_config`, `agent.cancel_turn`, `agent.commands`.
- **Errors:** `agent.option_unknown` (`id`), `agent.queue_full` (`limit`); a refusal by the agent itself is `adapter.option_refused`.

