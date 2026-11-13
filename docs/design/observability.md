# Observability and diagnostics

- **Logs:** structured logs via `tracing`, JSON in production, with redaction of tokens, keys, and
  environment values. Log files rotate; level adjustable at runtime.
- **Metrics:** hub exposes a Prometheus-format endpoint (opt-in): connected nodes/clients, event lag, queue
  depths, approval latency, terminal bytes/s. Nodes report resource metrics as `NodeResources` events.
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
  - The projection (and the window's view) takes `online` from the latest presence fact by position: `NodeLink`, or for data an
    older hub wrote, `NodeOnline` / `NodeOffline`. A `NodeLink` from any producer but `hub` is ignored, and a node that sends one
    is refused (`protocol.invalid_message`, field `node_link`).
  - No reconnect grace: a node that redials at once after dropping a frozen hub registers its new connection before the hub reads
    the old one's end, so the old one ends as `replaced` and the machine is never shown offline; a grace timer would add state
    for nothing.
- **Silence:** a node pings every 10 s (`agora-node::uplink_client::PING_EVERY`; nodes before ADR-0070 every 15 s, which the hub
  also accepts). The hub closes a node connection on which nothing arrived for 25 s (`agora-hub::server::NODE_SILENCE`; it keeps
  answering every `PING`, a node's or a window's, with a `PONG`) and records it `silent`. The node drops a link on which nothing
  (a `PONG` or any frame) arrived for 25 s (`SILENCE_DEADLINE`) and redials with its backoff; it gives up a dial (TCP, TLS,
  WebSocket) that has not finished in 25 s; and after its process was suspended (a wait for the next frame, normally at most
  50 ms, took longer than a ping interval on the monotonic or the wall clock) it redials at once without a backoff. A frame that
  cannot be sent within the deadline (a peer that stopped reading) counts as silence on both sides.
- **The window** pings the hub every 15 s over the same `PING`/`PONG` while its socket is open and starts over when nothing at
  all comes back within 10 s (`PING_EVERY_MS`, `PONG_WITHIN_MS`); "Retry now" and waking up (online, visible, focus) ping a live
  link at once. It gives up a connection that has not opened within 8 s (a frozen hub accepts the TCP connection and answers
  nothing), waits at most 5 s (with jitter) between attempts and dials at once when the browser is back online, the tab becomes
  visible or the window gets the focus (`apps/web/src/net/live.ts`). An offline machine shows "last seen" on its row and in its
  tooltip, on the Machines page and in the TUI's machine rows.
- **Acceptance:** `scripts/e2e-link-silent.mjs` (a frozen node, a frozen hub, a killed and restarted hub, in a real browser;
  the freezes need POSIX signals, so Linux and macOS).
- **`agora doctor`** (implemented: `agora-node::doctor`, local checks only; the redacted bundle and hub connectivity come with the hub client): collects versions, config (redacted), connectivity checks, agent detection status,
  and recent errors into one bundle for bug reports.
- **Tracing across hops:** commands carry a trace id from client through hub to node and adapter.
