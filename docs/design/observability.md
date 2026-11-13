# Observability and diagnostics

- **Logs:** structured logs via `tracing`, JSON in production, with redaction of tokens, keys, and
  environment values. Log files rotate; level adjustable at runtime.
- **Metrics:** hub exposes a Prometheus-format endpoint (opt-in): connected nodes/clients, event lag, queue
  depths, approval latency, terminal bytes/s. Nodes report resource metrics as `NodeResources` events.
- **Health:** heartbeats; the window shows node health (connected, lagging, offline since).
  Not done yet (see `docs/backlog/lan-test-findings.md` R1): a link that goes silent while the connection stays open is not
  noticed; it needs a hub-owned presence event first (an ADR). Today a node pings the hub every 15 s and the hub answers any
  peer's `PING` (a node's or a window's) with a `PONG`; the hub records `NodeOffline` when a node's connection ends. The window
  shows "Hub unreachable" with a "Retry now" button, gives up a connection that has not opened within 8 s (a frozen hub accepts
  the TCP connection and answers nothing; an open connection has no deadline), waits at most 5 s (with jitter) between attempts
  and dials at once when the browser is back online, the tab becomes visible or the window gets the focus (`apps/web/src/net/live.ts`).
- **`agora doctor`** (implemented: `agora-node::doctor`, local checks only; the redacted bundle and hub connectivity come with the hub client): collects versions, config (redacted), connectivity checks, agent detection status,
  and recent errors into one bundle for bug reports.
- **Tracing across hops:** commands carry a trace id from client through hub to node and adapter.
