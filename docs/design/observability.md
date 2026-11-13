# Observability and diagnostics

- **Logs:** structured logs via `tracing`, JSON in production, with redaction of tokens, keys, and
  environment values. Log files rotate; level adjustable at runtime.
- **Metrics:** hub exposes a Prometheus-format endpoint (opt-in): connected nodes/clients, event lag, queue
  depths, approval latency, terminal bytes/s. Nodes report resource metrics as `NodeResources` events.
- **Health:** heartbeats; the window shows node health (connected, lagging, offline since).
  Implemented heartbeats (protocol section 1, `PING`/`PONG` on stream 0): a node pings the hub every 10 s
  (`agora_node::uplink_client::PING_EVERY`) and drops and dials again when nothing at all (a `PONG` or any other frame) arrived
  for 25 s (`SILENCE_DEADLINE`). The hub closes a node connection on which nothing arrived for the same 25 s
  (`agora_hub::server::NODE_SILENCE`) and records `NodeOffline` with the time, so the window shows the machine offline and
  "last seen" from then; an idle node keeps its link with its pings and is never dropped. A send that waits as long as the
  deadline (a peer that stopped reading) ends the connection too. The window pings the hub every 15 s, starts over when
  nothing comes back within 10 s and shows "Hub unreachable" with a "Retry now" button; it waits at most 5 s (with jitter)
  between attempts and dials at once when the browser is back online, the tab becomes visible or the window gets the focus
  (`apps/web/src/net/live.ts`). The node and hub log the reason (`hub link: Silent`, `node silent: <id>`).
- **`agora doctor`** (implemented: `agora-node::doctor`, local checks only; the redacted bundle and hub connectivity come with the hub client): collects versions, config (redacted), connectivity checks, agent detection status,
  and recent errors into one bundle for bug reports.
- **Tracing across hops:** commands carry a trace id from client through hub to node and adapter.
