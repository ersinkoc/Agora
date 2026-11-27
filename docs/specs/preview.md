# Preview: an agent's local web server in the window (ADR-0088)

Status: the node keeps forwards and relays their connections, on its control socket and through its hub link. The messages, codes, frame kinds and signing
domain exist (`schema/agora/v1/preview.proto`, `docs/specs/protocol.md` "Preview"). A node answers `PreviewOpen`, `PreviewClose` and
`PreviewList`, checks every rule below, records `PreviewChanged`, and relays `PreviewConnect` on a dynamic stream of its control socket
(`crates/agora-node/src/preview.rs`, `preview_relay.rs`) and carries them through its hub link (`uplink_client.rs`). No hub or window
sends these messages yet. The node offers capability `preview.v1`; the hub step (3) adds it to the hub's list and decides who may
connect (the node accepts a routed `PreviewConnect` for an open forward from its hub, as it accepts a terminal subscription).

## What it is
A **forward** is a named, owner-approved path from the window to one port on the **loopback** of one node (`127.0.0.1` or `::1`), for a limited time. The window
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
The node's own ports are read again at every open from where the binary writes them: the hub on this machine (`<home>/hub/listen`,
also any `--hub-dir`), the hub the node dials when its address is this machine or the node cannot tell (`<home>/link/url`), the
panel (`<home>/link/panel-port`), the desktop app's tunnels (`<home>/desktop/tunnels/<port>`, written by each running app and
removed when its tunnel closes: inside the app a page's requests are addressed to the tunnel itself and would pass its own check), and
any port that answers a TLS handshake with a certificate signed by a hub CA the node knows (`<home>/hub/ca.pem`, `<home>/link/ca.pem`)
or issued under a hub CA's name: that covers a hub under another home and a hub whose `listen` file was not written.
The probe starts the handshake, takes the certificate and closes; it trusts nothing and sends no data. The node itself listens on no TCP port (its control socket and the holder's are local sockets). A
hub on this machine trusts a peer on its loopback more than a remote one (proxy headers, ADR-0067's local exemptions), so a forward to
it would hand the window that trust; that is why those ports are refused even for the owner.

- Port 1..65535 on the node's loopback only: `127.0.0.1` or `::1`, tried in the order `localhost` resolves on that machine (development servers such as Vite and Node on Windows often listen only on `[::1]`); below 1024 only when the owner's policy names it; never one of the node's own ports (control,
  hub link, holder, any port it knows it owns): `preview.port_refused` with `reason` `invalid`, `privileged`, `node_port` or `not_loopback` (the port answers on neither `127.0.0.1` nor `::1`).
- At most 8 open forwards per node (`preview.limit`). Expiry default 28800 s, cap 86400 s. A forward ends at expiry, when closed, or
  when the node restarts; it is never restored (`preview.expired`, `preview.unknown`).
- Through the hub, `PreviewClose` and `PreviewList` need no signature, whoever opened the forward: closing only removes access, and the
  list shows what `PreviewChanged` already says.
- The hub address in `link/url` is resolved at each open (only to tell whether it is this machine); a slow resolver delays the open,
  never anything else.
- The forward's `creator` is the device that signed the open, or `owner` for an open on the control socket.
- A device the owner limited to some projects (`device.<id>.projects`, ADR-0049) opens no preview: a forward reaches the machine's
  loopback, not a project (`auth.forbidden {action: preview}`).
- A node that cannot check signatures (no grants) opens nothing for the hub (`node.policy_denied {rule: preview.authorization}`), and a
  grant never stands in for the signature.
- Through the hub, an open needs a device signature over `preview-action-v1` (protocol.md: length-prefixed fields, a single-use nonce kept until its expiry, at most 5 minutes ahead), verified by the node, never by the hub. A suggestion never opens anything.
- `PreviewClose` with `all` and a `forward_id`, or with neither, is `protocol.invalid_message {field: forward_id}`.
- Stream budgets: `max_chunk_bytes` 16384, `window_bytes` 262144 of credit per direction, optional rate and total ceilings (none by
  default); over any of them the stream ends with `PreviewEnd.reason`. Preview bytes are the lowest priority and never delay terminal
  bytes (ADR-0022).
- Forward ids are `fwd-` and 128 random bits in hex: nobody finds one by counting; it is known only where the forward is listed or
  announced. Until the hub step (3) decides who may connect, a routed `PreviewConnect` is authorised by the hub alone (the node checks
  that the forward is open and its port still allowed), and the connection can send anything to the program, a `POST` included.
- Connections: one per `PreviewConnect`, to the forward's port on `127.0.0.1` or `::1` only, whatever the bytes name. Each costs about
  five threads on the node (two for the relay, two for the hub link's bridge, one for its control connection). At most 32 at a
  time per forward and 64 for the node; one more is answered `ok` and ended at once with `BUDGET` (it never reaches the program). At
  every connect the node's own ports are checked again (a hub's certificate at most every 5 s): a forward whose port became one of them
  ends with `POLICY`. Closing, expiry and the node's stop end the live connections with `CLOSED`, `EXPIRED` and `NODE_STOPPED`.
- Priority on the hub link: preview frames wait in one 16-frame queue sent only while no answer and no terminal frame waits. Under a
  steady terminal flood that queue fills and every bridge waits to send; a waiting bridge notices the hub's close only when the queue
  moves (or the link ends), so its threads and its place under the caps stay held until then.

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
