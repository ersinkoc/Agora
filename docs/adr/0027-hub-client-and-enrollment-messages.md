# 0027. Hub snapshot, client subscription and enrollment messages
Status: Proposed
Date: 2026-11-07

## Context
The hub now stores events (`agora-hub::store`), folds them into a view (`projection`), authenticates nodes with
certificates (`ca`, `tls`, `enroll`) and runs node sessions. Three exchanges still have no schema, so the hub cannot
serve a client or enroll a node over the wire (Phase 3 exit: two machines' agents visible in one window):

1. A client needs the current state before live events: a snapshot, then events after it.
2. A client must be able to ask for "everything after position N" and learn the position of each event it gets.
3. A node that has no certificate yet must exchange a one-time code and a CSR for a certificate.

Depends on ADR-0026 (frame kinds, `EventAck`). Everything here is new messages with new field numbers; nothing published
changes (ADR-0006).

## Decision
Append to the schema:

**Client subscription** (`schema/agora/v1/client.proto`, new file):
- `Subscribe { optional uint64 after_pos = 1; }`: client → hub on the events stream. Absent: send a snapshot first.
- `Snapshot { uint64 pos = 1; repeated NodeState nodes = 2; repeated CheckoutState checkouts = 3; repeated WorktreeState
  worktrees = 4; repeated AgentState agents = 5; repeated TerminalState terminals = 6; repeated ApprovalState approvals = 7; }`:
  hub → client; the state tables mirror `agora-hub::projection::View` field for field. `pos` is the global log position the
  snapshot reflects.
- `PositionedEvent { uint64 pos = 1; Event event = 2; }`: hub → client for every live or replayed event; the client resumes
  with `Subscribe { after_pos }` after a reconnect. If `after_pos` is older than the store keeps, the hub answers with a
  fresh `Snapshot` (the same rule as `SnapshotRequired` for nodes).

**Enrollment** (`schema/agora/v1/enroll.proto`, new file), carried on a TLS connection that has no client certificate yet and
accepts nothing else:
- `EnrollRequest { string code = 1; string node_id = 2; string csr_pem = 3; }`
- `EnrollResponse { string cert_pem = 1; string ca_pem = 2; uint64 not_after_ms = 3; }`, or an `ErrorInfo` with
  `auth.unauthenticated` for a bad, expired or used code, `auth.forbidden` while locked out. Bad code and expired code are
  indistinguishable on purpose.
- The hub chooses the identity: `node_id` is a request, the hub may refuse it; the certificate carries what the user approved.

Frame kinds are added to the ADR-0026 table: stream 2 kind 2 `Subscribe`, kind 3 `Snapshot`, kind 4 `PositionedEvent`;
enrollment uses stream 1 (commands) kind 0/1 with new `CommandRequest`/`CommandOk` variants `enroll` (appended).

Golden fixtures are added for each new message.

## Consequences
- A client can connect, get a consistent snapshot, and follow live events; reconnecting costs one `Subscribe`.
- The snapshot grows with the fleet; it is sent once per connect. Large fleets may need paging later (a new field, not a
  change).
- Enrollment adds a TLS endpoint that accepts unauthenticated peers; it is rate limited by the lockout in
  `agora-hub::enroll` and exposes only the one command.
- Until accepted, the hub library and its tests work without these messages; the hub server, node uplink and web
  store wait on them.
