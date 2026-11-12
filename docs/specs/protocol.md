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

Events a node emits (`agora-node::daemon::EventSink`): `NodeOnline` at start and `NodeOffline` on a clean stop; a
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
A gap from the hub rewinds to `expected`; a `from_seq` older than the buffer keeps (dropped by the size cap) is reported as
`Stale` so the hub resynchronizes from a snapshot. The hub's ack trims the buffer.

Hub core (`agora-hub::hub`): store, view and subscriber fan-out are one step under one lock, so a subscriber that takes its
snapshot (or backlog) and registers for live events together sees every event once, in order, even while several nodes
ingest. A subscriber that falls more than 1024 events behind is dropped, never waited for; it reconnects with the last
position it saw and gets the backlog, or a snapshot when that position is ahead of the log (a hub restored from an older
backup). When a node's connection ends without a goodbye the hub records `NodeOffline` itself as producer `hub` (a name no
node certificate can carry), in the same log, so live and rebuilt views agree.

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
| node | `NodeOnline`, `NodeOffline`, `NodeResources` |
| project | `CheckoutDiscovered`, `WorktreeCreated`, `WorktreeRemoved`, `GitStatusChanged` (the node checks each running agent's workspace every 5 s and reports when the changed-file count, line counts or HEAD differ from the last report) |
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
attributes), each one followed by kind 2 with the size it was painted at, kind 0 raw
output, kind 2 resize (cols, rows as u16 LE), kind 3 exit (i32 LE; negative = signal). A client that falls behind
gets a fresh snapshot instead of the dropped output; it never slows the terminal. Input and resize are ordinary
commands. Terminal ids are `t-<agent id>` (one terminal per agent). Implemented commands: `project.list`,
`agent.start`, `agent.stop`, `agent.prompt`, `terminal.input`, `terminal.resize`, `terminal.subscribe`; the rest
answer `protocol.capability_missing`. `agent.list` (ADR-0025) and `agent.kinds` (ADR-0030: the node's loaded manifests with display name, integration level and whether the launch program is on `PATH`; invalid manifests omitted; `agora agent kinds`).

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
role USER; adapter items map to TEXT, TOOL_CALL as JSON, PLAN as lines, DIFF with its path). Entry ids increase per agent; a client
ignores an id it already holds. The text is untrusted.

`MeshPost.refs` (ADR-0035, field 5): the sender's refs as written; the receiving broker qualifies them with the hub-vouched `origin`.

`BoardCall` (command 62, node → hub on the stream-1 request path like `MeshPost`) and `BoardResult` (`CommandOk` field 16), ADR-0039: a mesh tool on a shared
project's board; see `docs/specs/mesh-tools.md#shared-board-adr-0039`. New error code `board.unavailable`.

`BoardJoin` (command 66, hub → node only; a client's is refused) and `BoardJoinResult` (`CommandOk` field 20, with `BoardJoinLease` and `BoardJoinTask`), ADR-0039 amendment B: when a project becomes shared, the hub asks each node with a checkout for its unfinished local work and puts it on the board. Sent only to nodes that offer the capability `board.join`.

`GrantCreate` (63), `GrantList` (64), `GrantRevoke` (65) and `GrantListResult` (`CommandOk` field 17), ADR-0041: user-signed grants for the commands a node's policy lists in `signed_commands`; see `docs/security/threat-model.md`. New error code `approval.grant_required` (`command`). The node answers them itself; the local control socket refuses them.

`Snapshot.tasks` (8) and `Snapshot.leases` (9), ADR-0040: the shared boards' tasks and held paths, so a client that connects sees them before the next change.
