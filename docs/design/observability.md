# Observability and diagnostics

- **Logs:** structured logs via `tracing`, JSON in production, with redaction of tokens, keys, and
  environment values. Log files rotate; level adjustable at runtime.
- **Metrics:** hub exposes a Prometheus-format endpoint (opt-in): connected nodes/clients, event lag, queue
  depths, approval latency, terminal bytes/s. Nodes report resource metrics as `NodeResources` events.
  **Built (first slice, architecture review R8):** `agora hub run --metrics` turns on `GET /api/metrics` (the owner's bearer
  token; 401 without one, 403 for a member or viewer, 404 when the hub runs without `--metrics`). Prometheus text format,
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
  | `agora_hub_events_stored` | gauge | the position of the last event in the hub's log |
  | `agora_hub_resyncs_total` | counter | lost ranges recorded by resyncs (ADR-0054) |

  Counting costs two clock reads and a few atomics per stored event: 50 000 ingests measured 15.4–15.6 µs each with it and
  15.3–16.0 µs without (noise). Not yet: approval latency, per-subscriber queue depth, relay resets, terminal bytes/s.
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
    connection ends; such a client shows a machine that reconnected as offline until it reloads (as before).
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
- **`agora doctor`** (implemented: `agora-node::doctor`, local checks only; the redacted bundle and hub connectivity come with the hub client): collects versions, config (redacted), connectivity checks, agent detection status,
  and recent errors into one bundle for bug reports.
  It also notes that the node reads its policy file (`<data>/policy`) only when it starts, and warns when that file changed after
  the running node started (compared with its local socket's time), since such a change waits for a restart.
  It states who may make the machine act, as the node applies its files when it starts: which commands routed through the hub
  need a device-signed grant (the `signed_commands` line, the team default a limited device turns on, or none: a note, since a
  compromised hub could then send them, threat model T1), how many devices are trusted and how many of them may act in every
  project, the `skip_permissions` setting, and how many policy lines the node did not understand.
- **Tracing across hops:** commands carry a trace id from client through hub to node and adapter.
