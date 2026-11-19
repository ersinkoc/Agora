# Preview: an agent's local web server in the window (ADR-0088)

Status: schema only. The messages, codes, frame kinds and signing domain exist (`schema/agora/v1/preview.proto`, `docs/specs/protocol.md`
"Preview"); no node, hub or window sends or handles them yet. Capability `preview.v1`.

## What it is
A **forward** is a named, owner-approved path from the window to one port on `127.0.0.1` of one node, for a limited time. The window
shows the page in an isolated frame. Nothing else on the node's loopback becomes reachable.

## Messages
| Message | Carried as | Purpose |
| --- | --- | --- |
| `PreviewOpen` | command 102 | open a forward to `port` for `expires_seconds` (0 = 8 hours, at most 24 hours); signed through the hub |
| `PreviewClose` | command 103 | close one forward or, with `all`, every forward on the node |
| `PreviewList` | command 104 | open forwards of the node |
| `PreviewConnect` | command 105 on a dynamic stream | one relayed TCP connection to the forward's port |
| `PreviewSuggested` | event 80 | "my app is on port N", from an agent or the node; a card with an Open button, never an open |
| `PreviewChanged` | event 81 | a forward opened or ended (indicator, audit: port only) |

## Rules the node enforces (never widened by the hub)
- Port 1..65535 on `127.0.0.1` only; below 1024 only when the owner's policy names it; never one of the node's own ports (control,
  hub link, holder, any port it knows it owns): `preview.port_refused` with `reason` `invalid`, `privileged`, `node_port` or `not_loopback`.
- At most 8 open forwards per node (`preview.limit`). Expiry default 28800 s, cap 86400 s. A forward ends at expiry, when closed, or
  when the node restarts; it is never restored (`preview.expired`, `preview.unknown`).
- Through the hub, an open needs a device signature over `preview-action-v1` (protocol.md). A suggestion never opens anything.
- Stream budgets: `max_chunk_bytes` 16384, `window_bytes` 262144 of credit per direction, optional rate and total ceilings; over any of
  them the stream ends with `PreviewEnd.reason`. Preview bytes are the lowest priority and never delay terminal bytes (ADR-0022).

## The hub's part (not built)
The hub relays preview streams without parsing them, gives each forward an origin of its own (a per-forward host name, or a separate
port), binds the forward URL to the signed-in window with a short-lived per-forward token, applies the screen password (ADR-0067), and
writes audit rows (port, never content). Where no origin of its own is possible it answers `preview.unavailable` (`no_origin`) and
opens nothing.

## Not decided by the ADR (left open for the next steps)
- How the per-forward token is minted and presented to the preview origin (an HTTP surface, not a protocol message here).
- The exact subdomain or separate-port scheme and its configuration.
- Whether `PreviewSuggested` is raised by a mesh tool, by detecting a listening port, or both.
- Who may see `PreviewSuggested` and `PreviewChanged` (until decided, the hub's scope filter sends them to nobody).
- The owner's per-project port allow-list (the ADR allows it "later").
