# Observability and diagnostics

- **Logs:** structured logs via `tracing`, JSON in production, with redaction of tokens, keys, and
  environment values. Log files rotate; level adjustable at runtime.
- **Metrics:** hub exposes a Prometheus-format endpoint (opt-in): connected nodes/clients, event lag, queue
  depths, approval latency, terminal bytes/s. Nodes report resource metrics as `NodeResources` events.
  **Built (first slice, architecture review R8):** `agora hub run --metrics` turns on `GET /api/metrics` (the owner's bearer
  token, checked before anything is counted; 401 without one, 403 for a member or viewer). Without `--metrics` the path is
  answered byte for byte as any unknown path, for GET and HEAD, so nothing tells that it exists. Each scrape renews the owner's
  session, as any use does: a scraper holding the owner's token keeps it alive for as long as it scrapes. **A scraper should hold a
  metrics token instead (ADR-0076)**, a credential that can only read this path:
  `agora hub metrics-token create NAME [--expires DAYS]` prints it once (`agm_` and 64 hex digits, 256 random bits; no expiry unless
  asked), `list` shows each token's name, when it was made, last used (to the minute) and expires, never the token, and
  `revoke NAME` ends it at its next use (the hub looks it up on every scrape). Send it as `Authorization: Bearer agm_...`. The hub
  keeps only its SHA-256, in a table apart from sessions. Every other path and the WebSocket treat it as an unknown token (401); it
  starts and renews no session and has no user or role. A bearer that is neither a live token nor the owner's session counts toward
  the sign-in lockout of its address (429 while locked out). Creating and revoking are audit events (`metrics_token.create`,
  `metrics_token.revoke`, with the name); a scrape is not audited. Not built: a page in the window to manage tokens, per-token rate
  limits. Prometheus text format,
  written by hand (`agora-hub::metrics`, no new dependency):

  | Name | Kind | What it is |
  | --- | --- | --- |
  | `agora_hub_events_ingested_total` | counter | events from machines the hub stored |
  | `agora_hub_lock_wait_seconds` | histogram | how long a machine's event waited for the hub's lock |
  | `agora_hub_ingest_seconds` | histogram | how long the hub held its lock for one machine event, storing it included: every client and every other machine waits that long |
  | `agora_hub_subscribers_dropped_total` | counter | windows and terminal clients let go because their queue was full (too slow) or closed |
  | `agora_hub_nodes_known`, `agora_hub_nodes_online` | gauge | machines the hub knows, and those connected now |
  | `agora_hub_subscribers` | gauge | windows and terminal clients following the hub now |
  | `agora_hub_approvals_waiting` | gauge | approvals not answered yet |
  | `agora_hub_approval_wait_seconds` | histogram | how long an agent waited for a person's answer (buckets 1 s to 1 h), from the asking machine's times of the request and the answer |
  | `agora_hub_relays_dropped_total` | counter | terminal relays let go because the window could not keep up (or was gone); chat relays are not counted |
  | `agora_hub_terminal_bytes_total` | counter | bytes of terminal frames (snapshots and output) that reached a window's queue (`rate()` of it is the terminals' bytes per second); chat frames are not counted |
  | `agora_hub_events_stored` | gauge | the position of the last event in the hub's log |
  | `agora_hub_resyncs_total` | counter | lost ranges recorded by resyncs (ADR-0054) |
  | `agora_hub_node_commands_queued` | gauge | commands queued for connected machines and not yet written to them (that queue has no bound: while a write to a machine that stopped reading waits, it grows, until the write gives up after 25 s and the connection ends) |
  | `agora_hub_node_commands_unanswered` | gauge | commands written to connected machines and not answered yet |

  Counting costs two clock reads and a few atomics per stored event: 50 000 ingests measured 15.4–15.6 µs each with it and
  15.3–16.0 µs without (noise). Not yet: per-subscriber queue depth (the subscribers' `sync_channel` does not say its length; it needs a counted wrapper on every send and receive, so `agora_hub_subscribers_dropped_total` stands in for it: a subscriber is let go when its queue is full).
- **Health:** heartbeats; the window shows node health (connected, lagging, offline since).
- **Presence (ADR-0070):** whether a machine is online is a fact the hub owns, written from the certificate-authenticated
  connection only (`agora-hub::hub::{node_link_opened, node_link_closed}`):
  - `NodeLink { up: true, reason: opened }` when a node's connection is open after the handshake: a first connect and a
    reconnect alike. A reconnect is not a process start, so nothing the node runs is reset (only the node's own `NodeOnline`,
    sent when its process starts, does that; ADR-0055).
  - `NodeLink { up: false, reason: closed | silent | replaced }` when the connection ends or the hub closes it as silent.
    `last_heard_ms` is when the hub last received anything on it: the window's and the TUI's "last seen". A connection that a
    newer one of the same node had already replaced writes `replaced`, which never makes the node offline. Registering a
    connection and writing its fact happen under one lock, as do ending one and writing its fact, so the log's order is the
    order the connections took turns in; `generation` (1, 2, … per node, kept across hub restarts) lets a reader ignore a late
    fact about an older connection. For clients from before ADR-0070 the hub also writes `NodeOffline` when the current
    connection ends; such a client shows a machine that reconnected as offline until it reloads (as before). A connection
    whose `opened` fact the store cannot write is not registered and the hub ends it (logged as `node link ended: <node>
    store: ...`); the node dials again within seconds and is recorded then, so a machine is never connected while every view
    shows it offline (#957).
  - **At hub start** (`agora-hub::hub::Hub::open`) no connection exists yet. A node that the view rebuilt from the log shows
    online (the hub was killed or lost power with its connection open, so no end was recorded; or only an old `NodeOnline` from
    before ADR-0070 says so) is recorded offline before the server accepts connections: `NodeOffline` for older clients, then
    `NodeLink { up: false, reason: closed }` for the node's latest connection generation (0 for a node seen only through
    `NodeOnline`). `closed` is the existing reason closest to "the connection ended"; the schema is unchanged. Its
    `last_heard_ms` is the node's last time in the log, not the restart, so "last seen" stays true; the events' own `at_ms` is
    the restart. A node already offline gets nothing, so opening again adds no facts. When the node redials, its next
    generation's `opened` brings it back online.
  - The projection (and the window's view) takes `online` from the latest presence fact by position: `NodeLink`, or for data an
    older hub wrote, `NodeOnline` / `NodeOffline`. A `NodeLink` from any producer but `hub` is ignored, and a node that sends one
    is refused (`protocol.invalid_message`, field `node_link`).
  - No reconnect grace: a node that redials at once after dropping a frozen hub registers its new connection before the hub reads
    the old one's end, so the old one ends as `replaced` and the machine is never shown offline; a grace timer would add state
    for nothing.
- **Silence:** a node pings every 10 s (`agora-node::uplink_client::PING_EVERY`; nodes before ADR-0070 every 15 s, which the hub
  also accepts). The hub closes a node connection on which nothing arrived for 25 s (`agora-hub::server::NODE_SILENCE`; it keeps
  answering every `PING`, a node's or a window's, with a `PONG`) and records it `silent`. The node drops a link on which nothing
  (a `PONG` or any frame) arrived for 25 s (`SILENCE_DEADLINE`) and redials on its schedule; it gives up a dial (TCP, TLS,
  WebSocket) that has not finished in 25 s; and after its process was suspended (a wait for the next frame, normally at most
  50 ms, took longer than a ping interval on the monotonic or the wall clock) it redials at once without a backoff. A frame that
  cannot be sent within the deadline (a peer that stopped reading) counts as silence on both sides.
- **Redial schedule (node):** after a lost connection the node waits 0.5 s, 1 s, then 1.5 s between attempts (`redial_wait`,
  each wait shortened by up to a fifth at random) while the hub has been away for less than 5 minutes (`REDIAL_FAST_FOR`): a
  hub that restarts is reached within about 1.5 s of listening again. After that the waits double up to 30 s. A connection the hub
  accepted and kept for 2 s starts the schedule over; one it dropped sooner does not, so a hub that accepts and drops at once is
  not dialled every half second. Each address of a loopback hub name is given up after 0.5 s and counts as refused
  (`agora_protocol::dial::connect_bounded`, `LOOPBACK_ATTEMPT`): on Windows a refused connection to `127.0.0.1` with no listener
  takes about 2 s (SYN retransmits), which made a redial cycle 2 s + 1.5 s. Other names keep the system's connect timeout
  (bounded by the 25 s dial deadline). (Before, the waits doubled to 30 s and only a connection that lasted 30 s started them over: a
  new agent reached the window up to 30 s after a hub restart, `scripts/e2e-hub-restart.mjs`.)
- **The window** pings the hub every 15 s over the same `PING`/`PONG` while its socket is open and starts over when nothing at
  all comes back within 10 s (`PING_EVERY_MS`, `PONG_WITHIN_MS`); "Retry now" and waking up (online, visible, focus) ping a live
  link at once, and also pings right after it subscribes: the hub's `PONG` comes after whatever the subscription owed, so a
  reconnect with nothing new to deliver is live at once instead of "Connecting" until the next event. It gives up a connection
  that has not opened within 8 s (a frozen hub accepts the TCP connection and answers nothing), waits at most 1.5 s (with
  jitter; 5 s after 200 attempts, about five minutes) between attempts and dials at once when the browser is back online, the
  tab becomes visible or the window gets the focus (`apps/web/src/net/live.ts`). An offline machine shows "last seen" on its row and in its
  tooltip, on the Machines page and in the TUI's machine rows.
- **Acceptance:** `scripts/e2e-link-silent.mjs` (a frozen node, a frozen hub, a killed and restarted hub, in a real browser;
  the freezes need POSIX signals, so Linux and macOS).
  `scripts/e2e-hub-restart.mjs` kills and restarts the hub many times under two nodes and a window, starts an agent on each node
  right after each start and requires the window to show it within 3 s.
- **`agora doctor`** (implemented: `agora-node::doctor`, local checks only, plus the days left on this machine's hub certificate and the hub's own (`agora::certs`); the redacted bundle and hub connectivity come with the hub client): collects versions, config (redacted), connectivity checks, agent detection status,
  and recent errors into one bundle for bug reports.
  It also notes that the node reads its policy file (`<data>/policy`) only when it starts, and warns when that file changed after
  the running node started (compared with its local socket's time), since such a change waits for a restart.
  It states who may make the machine act, as the node applies its files when it starts: which commands routed through the hub
  need a device-signed grant (the `signed_commands` line, the team default a limited device turns on, or none: a note, since a
  compromised hub could then send them, threat model T1), how many devices are trusted and how many of them may act in every
  project, the `skip_permissions` setting, a warning when `files.show_secret` is on (ADR-0080), and how many policy lines the node did not understand.
- **Failure drills** (architecture review R8). Each one runs in CI and says which hop failed and what it recovered:

  | Drill | Test | What it shows |
  | --- | --- | --- |
  | Backup and restore with two machines | `crates/agora/tests/restore_drill.rs` `the_hub_comes_back_from_an_older_backup_and_every_machine_brings_its_present` | real processes; the restored hub gets both machines' present back by itself in 0.5–0.7 s, with no lost history while the machines still hold it |
  | A machine's certificate revoked while connected | `restore_drill.rs` `revoking_a_connected_machine_ends_its_connection_and_keeps_it_out` | the connection ends within the hub's recheck, the machine is shown offline, it stays out when it redials, and a newly enrolled machine of that name connects |
  | A long disconnect whose buffer overflows | `crates/agora/tests/hub_link.rs` (a node with a 16 KiB cap; `a_node_whose_cap_dropped_unacked_events_resyncs_and_delivers_its_newer_events`) | the node resynchronizes, the hub records the lost range (`resyncs`), and agents still running keep their board work (ADR-0054, ADR-0055) |
  | A client that leaves while its command is on its way | `crates/agora-hub/tests/server.rs` `a_command_whose_client_left_still_gets_its_outcome_in_the_audit_trail` | the command still happens once and its outcome reaches the audit trail |

  Of the metrics the review lists first, ingest and storage latency, lock hold time, resync and relay-reset counts and approval
  latency are the `/api/metrics` lines above. The recovery target is written down apart from the history-loss window
  (`docs/design/backup-restore.md`: the present whole within 5 s of the hub starting; a resync restores the present, not the
  missing history), and the restore drill fails above it. Queue depth: the queues towards machines are the two
  `agora_hub_node_commands_*` lines, counted per connection (a connection that ends stops counting at once). Not yet: the
  subscribers' queue depth (see above) and the oldest event a machine holds unacknowledged, which only the node knows.
- **Tracing across hops:** commands carry a trace id from client through hub to node and adapter.
