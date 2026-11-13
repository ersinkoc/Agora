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
backup). When a node's connection ends without a goodbye the hub records `NodeOffline` itself as producer `hub` (a name no
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
current connection ends. The snapshot's `NodeSnapshot.last_event_ms` includes `last_heard_ms`, so it is "last seen".

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
have 256 requests waiting. The node runs the command on its own control socket, so it gets the handler, policy and approval
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

Client sign-in (`agora-hub::signin`): `agora hub login` (and `agora hub run`, once at start) prints a one-time code (`XXXX-XXXX`, about 39
bits, 10 minutes, single use). The web app trades it with `POST /api/session` (`{"code": "..."}` → `{"token": "..."}`, 401 for a
wrong, expired or used code, 429 after five misses in a row) for a session token: 128 random bits, valid 12 hours, stored only as a
hash. The token is the bearer credential above. `<hub dir>/token` stays valid for scripts. 

Screen password (ADR-0067, `agora-hub::password`; additive JSON on the same HTTP routes, no protocol message): `GET /api/session` →
`{"passwordRequired": bool, "passwordSet": bool}` (required only when one is set and the TCP peer is not loopback). `POST /api/session`
also takes `"password"`; without the right one from a non-loopback peer it is 401 like a bad code (the code is not spent). Wrong codes,
passwords and passkeys are limited per address (5 tries, then 1 minute doubling to 1 hour; loopback at most 1 minute) and codes and
passwords also hub-wide (50), 429 while locked out; every 401/429 is answered after 250 ms. `DELETE /api/session` with a bearer ends that
session ("Lock"). `POST /api/password` (`{"password", "current"?}`, owner session; `current` needed when one is set) sets it and ends
every other session (400 `{"code":"password.too_short"|"password.too_long"}`, 401 wrong current, 403 not the owner).
From a non-loopback peer with a password set, `POST /api/passkey/register-begin` and `register-finish` also need `"password"` (401
without it, counted like a sign-in); changing or clearing the password deletes passkeys added from elsewhere without it (their ids are
not in the setting `passkeys_vouched`). Passkey `login-finish` counts only toward the per-address limit. The idle lock of a window
uses `DELETE /api/session`. The hash is a setting (`screen_password`: `pbkdf2-sha256$<rounds>$<salt hex>$<hash hex>`), not a schema change.

Passkeys (`agora-hub::passkey`, WebAuthn, ES256 only, no attestation): a signed-in session adds one with
`POST /api/passkey/register-begin` (`{}` → `{"challenge","rpId"}`, `Authorization: Bearer <session>`) and `register-finish`
(`{"id","publicKey"(SPKI),"clientDataJSON"}`, all base64url). Without a session: `login-begin` (→ `{"challenge","rpId","ids"}`) and
`login-finish` (`{"id","authenticatorData","clientDataJSON","signature"}` → `{"token"}`, 401 on any failed proof, 429 while locked
out). The relying party id is the request's `Host` without the port and the origin must be that host; challenges are single use and
last 5 minutes; user presence and verification are required; a signature counter that counts must rise. The hub stores public keys only
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
all-added, at most 50 files of at most 64 KiB), and an answer is cut at 512 KiB. `files.list` hides `.git`; `files.read` refuses
files above 512 KiB (`files.too_large`) and paths that leave the workspace, symlinks included (`files.outside_root`), checked again on the opened file so a link swapped in after the check is still refused.

### Selected directories (ADR-0052)

Capability `workspace.directory` adds commands without changing `AgentStart`: its empty `worktree_id` still means
create a worktree. New clients refuse an unavailable directory launch rather than falling back to isolation.

| New command / request oneof tag | Fields and result |
| --- | --- |
| `directory.inspect` / `DirectoryInspect` (81) | `path`, `create_project_id`, `authorization` → `DirectoryInfo` (Ok tag 18): canonical `path`, `project_id`, `git_root`, `branch`, `portable`, `previous_project_id` |
| `directory.list` / `DirectoryList` (84) | `path` (empty: the user's home folder), `authorization` → `DirectoryEntries` (Ok tag 21): canonical `path`, `parent` (empty at a root), `entries[]` of `{name, is_git}`, `truncated`. Folders only, sorted by name, at most 500 per answer; folders ADR-0052 refuses to launch in are not listed; capability `directory.list` (ADR-0053) |
| `agent.forget` / `AgentForget` (85) | `agent_id`; answered by the hub, which appends `AgentForgotten` (event 36, `agent_id` is the hub-wide key `<node>/<id>`) and stops listing the agent; `agent.running` while it runs; the earlier events stay in the store (ADR-0053) |
| `agent.launch` / `AgentLaunch` (82) | canonical `path`, expected `project_id`, `kind`, `separate_worktree`, `authorization`, `accept_identity_change` → `AgentLaunchResult` (Ok tag 19): `agent_id`, `terminal_id`, `workspace_id`, actual cwd `path` |
| `workspace.query` / `WorkspaceQuery` (83) | `workspace_id`, `operation` (`list`, `read`, `status`, `diff`, `history`), relative `path`, `limit` → existing file/Git results |

The node canonicalizes and re-inspects before launch; a path or project ID mismatch is refused. It persists confirmed
path/identity associations; changed metadata requires `accept_identity_change`. Metadata creation is explicit, uses
create-new semantics and never commits or overwrites `.agent-project.json`. The metadata contains `{"project_id":"UUID"}`;
portable domain IDs use `id:<uuid>`. Without metadata, Git discovery remains unchanged; standalone folders use
`folder:<node>:<canonical path>` without a fabricated root commit. Metadata must be a normal file, at most 4096 bytes,
with a valid UUID. An owner confirms its association; metadata is not an access grant.

`WorkspaceRegistered` (event tag 24: `workspace_id`, `node_id`, `project_id`, `path`, `git_root`, `branch`,
`managed_worktree`, `files_changed`) and `AgentWorkspaceBound` (35: `agent_id`, `workspace_id`) precede `AgentStarted`.
That `AgentStarted` leaves legacy `worktree_id` empty. `WorkspaceStatusChanged` (25) carries changed-file/line counts
and HEAD for the workspace. Node workspace IDs are `directory:<canonical cwd>`; the hub qualifies IDs with the
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
Generic start grants cannot authorize a substituted directory. Initially only unrestricted node-trusted owner devices
may sign these operations; scoped member devices are refused even for their projects. The local owner-only control
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
gets a fresh snapshot instead of the dropped output; it never slows the terminal. Input and resize are ordinary
commands. Terminal ids are `t-<agent id>` (one terminal per agent). Implemented commands: `project.list`,
`agent.start`, `agent.stop`, `agent.prompt`, `terminal.input`, `terminal.resize`, `terminal.subscribe`; the rest
answer `protocol.capability_missing`. `agent.list` (ADR-0025) and `agent.kinds` (ADR-0030: the node's built-in and folder manifests, one per kind (agent-manifest.md, "Where agent kinds come from"), with display name, integration level and whether the launch program is on `PATH`; invalid manifests omitted; `agora agent kinds`).

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

`GrantCreate` (63), `GrantList` (64), `GrantRevoke` (65) and `GrantListResult` (`CommandOk` field 17), ADR-0041: user-signed grants for the commands a node's policy lists in `signed_commands`; see `docs/security/threat-model.md`. New error code `approval.grant_required` (`command`). The node answers them itself; the local control socket refuses them.

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
  `kind` (6), `folder` (7) and `remaining` (8), and `expires_at_ms = 0` (it ends when the asking agent exits or is stopped). `GrantRevoke { grant_id }`
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
  hub cannot change it.

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
  (`auth.forbidden` otherwise). A revocation dated more than 5 minutes ahead of the node's or the hub's clock is refused. It then
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
  (`since`) at or before `issued_at_ms`, and neither are the devices it endorsed; endorsements of it signed at or before `issued_at_ms`
  are refused from then on. A revocation sent again later never removes trust given after it (an endorsement signed later, or the
  owner trusting it again on the machine with `agora node trust ID KEY`).
- **Fingerprint.** What people compare: the first 12 bytes (96 bits) of the SHA-256 of the 65-byte key, lowercase hex in groups of 4
  hex digits joined by `-` (`f947-3cb5-e304-81be-ecf2-64cd`). Every client shows it wherever a device is named, computed by the client
  from the key itself, never taken from the hub.
- **Trusted devices.** Event `NodeTrustedDevices` (`Event` field 13: `device_ids[]`, sorted, never keys), produced by a node when it
  starts and whenever the set changes; the hub keeps the last one per node in `NodeSnapshot.trusted_device_ids` (10), so a window can
  tell which machines trust its device. Node capability `device.trust`: the node verifies `DeviceEndorse` and `DeviceRevoke` and reports
  `NodeTrustedDevices`; the hub never sends either message to a node without it.

Shared vectors for the four signed messages and the fingerprint, checked by the node, the terminal client and the web window:
`schema/fixtures/signing/device-trust-v1.json`.
