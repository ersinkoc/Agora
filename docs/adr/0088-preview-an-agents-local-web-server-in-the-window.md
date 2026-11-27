# 0088. Preview an agent's local web server in the window
Status: Accepted
Date: 2026-11-19

Accepted: 2026-11-19 (the maintainer's explicit message in the merger chat: "ADR-0088'i kabul ediyorum"). The open questions' proposed answers stand.
Amended: Amendment A (Proposed, 2026-11-27) answers how the hub carries a preview (step 3); it waits for the maintainer's decision.

Builds on ADR-0020 (signed approvals), ADR-0067 (remote access and the screen password), ADR-0068 (a customizable workspace) and ADR-0070
(the hub owns whether a machine is online). Touches the threat model (networking) and the invariant "Secrets never leave the node".

Origin: the maintainer asked for a browser next to the terminal, the changes and the files in the window, so an agent's running app
(a dev server on `localhost:3000` on the node's machine) can be seen and used without leaving Agora.

## Context
- An agent builds a web app and starts its dev server on the node's machine. The server listens on `127.0.0.1` there. The window may be on
  another machine, a phone, or behind the hub on a VPS, so the address means nothing to the window.
- Reaching that port from the window needs a path that goes window -> hub -> node -> port. That path is a new network surface: whoever can
  open it can talk to any program listening on the node's loopback (databases, admin tools, the node's own control socket if it were TCP).
- The page it loads is code the agent wrote. It must not be able to read the window's session, the hub token or the approval keys, and it
  must not be able to reach other loopback ports by itself.

## Decision
1. **A preview is a named, owner-approved forward.** The node opens a forward only for a port the user (or an approved agent request)
   named, to the loopback of that machine (`127.0.0.1` or `::1`: dev servers on Windows often listen only on `::1`), never to another host. A forward has an id, a port, an expiry (default 8 hours) and a creator.
   Forwards are listed in the window and can be closed there; closing or node restart ends them.
2. **Opening a forward is a sensitive action** (ADR-0020): a user-signed approval verified on the node, naming the port. An agent may
   *suggest* a port ("my app is on 3000"), shown as a card with an Open button; it can never open one by itself. An allow-list of ports
   for a project is allowed later, set by the owner, never by the hub.
3. **Never the node's own ports.** The node refuses its own listening ports (control, hub link, holder, any port it knows it owns) and
   ports below 1024 unless the owner names them in policy. The hub cannot widen this list.
4. **Transport reuses the node-hub link.** The bytes ride a new multiplexed stream on the existing authenticated link (no new listener
   on the node). The hub relays opaque streams; it does not parse them. A stream has a size and rate budget, and input outranks output
   (ADR-0022); preview traffic never delays terminal bytes.
5. **The window shows the preview in an isolated frame.** The preview is served from its own origin (a separate host name or a
   per-forward subdomain on the hub, never the window's origin), loaded in a sandboxed frame without access to the window's storage,
   cookies or scripts. The forward URL is bound to the signed-in window (a short-lived, per-forward token, not the session cookie) so it
   is not a link that works for anyone who has it. No `allow-same-origin` together with `allow-scripts` on the window's origin.
6. **Remote-access rules still apply.** With a screen password (ADR-0067), a forward is reachable only after it. Without one, forwards
   are available only over the local or the owner's private network paths, and the window says so.
7. **WebSockets work (dev-server hot reload), nothing else is rewritten.** The hub does not edit the page; links that point at
   `localhost` inside the preview stay inside the sandbox and do not escape to the window's machine.
8. **Visible and stoppable.** The window shows an indicator when any forward is open, per machine, and a one-click "Close all
   previews". Opening and closing are written to the audit log (ADR-0020) with the port, never the content.

## Open questions for the maintainer
- Default expiry of 8 hours, or "until closed"? (Proposed: 8 hours.)
- A per-forward subdomain needs the hub to answer wildcard host names; with only an IP address or a path-only reverse proxy (a tunnel
  that gives one host name) that is not possible. Fall back to a separate port on the hub, or refuse previews in that setup?
  (Proposed: separate hub port where possible, otherwise refuse and say why.)
- Should a preview be allowed over a Cloudflare Tunnel? (Proposed: yes, behind the screen password.)

## Consequences
Schema (new messages for forward open/close/list and a stream channel; an ADR-gated schema PR with golden fixtures), the node (a
forwarder with the checks above), the hub (a relay, an origin per forward, audit rows), the web client (a Browser pane with address bar
limited to the forward, reload, open-in-new-window, device frame sizes, an indicator), the TUI (none at first), docs, threat model
(a new row: an agent's page against the window; a forward to a sensitive loopback port; a leaked forward link), tests with the fake
agent and a tiny local server, including an escape test (the preview page tries to read the window's storage, to reach another loopback
port and to open the node's control port: all must fail).

## Build order
1. Schema PR (messages and capability, fixtures) after acceptance.
2. Node forwarder and policy checks, with the escape tests.
3. Hub relay, origin per forward, audit.
4. Web Browser pane.

## Amendment A (Proposed, 2026-11-27): how the hub carries a preview (step 3)

The step-3 questions the decision left open (`docs/specs/preview.md`, "Not decided"), answered for the maintainer's decision. It
changes decision 4's "the hub does not parse them" for one thing only: the head of each HTTP request, as below.

1. **A preview origin is a port of the hub's, one per open forward.** The hub listens on a small pool of extra ports (default the 16
   ports after its own; `agora hub run --preview-ports 7450-7465`), the same TLS certificate, and gives each forward it carries one
   free port while the forward is open (released when it ends). Each forward is then its own origin: its page cannot read another
   forward's or the window's storage, and the window's session token lives in the window's `localStorage`, which a page on another
   port never sees (the hub sets no cookie of its own). A hub with no free port, or reached through one host name and port only (a
   tunnel or a path-only proxy), answers `preview.unavailable` with `no_origin` and opens nothing. Per-forward host names (a wildcard
   name) are a later option for hubs that have one.
2. **Only the opener connects** (the maintainer, 2026-11-27). The window asks the hub, with its session, for an entry ticket for a
   forward (`POST /api/preview/<forward>/enter`). The hub gives one only to the person whose device opened it (the forward's
   `creator`): 32 random bytes, single use, 60 seconds. The frame loads `https://<hub>:<port>/.agora-preview/enter?t=<ticket>`; the
   hub answers with a cookie named for that port, `agora_preview_<port>=<forward token>` (`HttpOnly`, `Secure`, `SameSite=Strict`,
   `Path=/`, no `Domain`; one name per port, because a browser keeps one cookie per name, host and path whatever the port, so one
   shared name would let a second preview log the first out), and a redirect to `/` with `Referrer-Policy: no-referrer` and
   `Cache-Control: no-store` (the ticket was in the URL; the hub never logs a query string). The forward token is random, bound to the forward and the person, and ends with the forward, the
   person's sign-out or removal, or after 8 hours.
3. **The hub reads every request head, and only the head.** Browsers send a host's cookies to every port of that host, so every
   request on a preview port carries the cookies of every forward on that host. For each request (HTTP/1.1, keep-alive included) the
   hub reads the request line and headers (at most 16 KiB, else 431), requires a valid `agora_preview_<port>` token for this port's
   forward (another port's name is not accepted), **removes every `agora_preview_*` cookie** before the bytes go on, and passes the
   rest unchanged. A WebSocket
   upgrade is checked the same way, and after its `101` the connection is relayed as opaque bytes. Bodies and responses are never
   read. Without a valid token: 401 and nothing reaches the node. HTTP/2 is not offered on preview ports (ALPN `http/1.1` only), so
   the head is always readable.
4. **The screen password applies** (ADR-0067): a remote window gets an entry ticket only after it, as for any command. Audit rows:
   `preview.enter` (forward, port, person), never content.
5. **What the window shows.** A Browser pane with the forward's address (path only), reload, open in a new tab (the same entry flow),
   phone and tablet sizes, and the forward's end time. The frame is `sandbox="allow-scripts allow-forms allow-same-origin
   allow-popups"` on the preview's own origin, which is safe there because that origin is not the window's; without
   `allow-popups-to-escape-sandbox`, so a popup the previewed app opens stays sandboxed (open in a new tab goes through the entry flow).
6. **The programs behind different forwards share one cookie jar**; the maintainer chose (a) on 2026-11-27. Cookies ignore the port, so app A's
   `session=` goes with every request to app B's port; B can also set cookies over A's names, or set so many that every preview's
   head goes over 16 KiB (431). Two forwards may front different programs (another project, an agent's server).
   - **(a) Chosen: partition the jar.** On a response head, `Set-Cookie: n=v` becomes `p<port>_n=v`; on a request head, only this
     port's `p<port>_*` cookies go on, with the prefix taken off. The hub then reads response heads too (a second, equally bounded
     exception to "does not parse"; bodies still never), and a page's `document.cookie` sees prefixed names (a script that reads its
     own cookies by name breaks; most dev servers set `HttpOnly` session cookies and are unaffected).
   - (b), not chosen: keep the jar shared, say so plainly in the window and the threat model (cookies are shared among the previews open on
     one hub), and leave per-forward host names as the real fix.
7. **Suggestions and changes** go to the opener only (`PreviewSuggested` to the project's owner and the person who started the agent
   that suggested it, `PreviewChanged` to the opener), until a wider rule is decided.

Threat-model rows to add: cookies across previews (shared by the browser across ports; the hub strips its own and partitions the apps'
cookies by port, point 6), a leaked entry ticket (single use, 60 s, opener only, no referrer, not logged), and
the head parser (bounded, rejects malformed heads, never reads bodies).

## Status of the build
1. Schema: done. `schema/agora/v1/preview.proto` (commands 102..105, results 26..28, events 80..81, frame kinds 20..22), capability
   `preview.v1`, error codes `preview.port_refused`, `preview.limit`, `preview.expired`, `preview.unknown`, `preview.unavailable`, golden
   fixtures, the signing domain `preview-action-v1` with vectors, and `docs/specs/preview.md`. The capability is not advertised yet (steps
   2 and 3 add it); node and hub refuse the commands with `protocol.capability_missing`.
2. Node forwarder and policy checks: done. The forward table (`crates/agora-node/src/preview.rs`, tests in
   `crates/agora-node/tests/preview.rs`): open, close and list on the control socket and, signed under `preview-action-v1`, through the
   hub; the port rules (1024 and up, never the node's own ports, the port must answer on `127.0.0.1` or `::1`), at most 8, expiry
   (default 8 hours, at most 24), `PreviewChanged` on every open and end, every forward ended when the node stops. A port that shows a
   known hub's certificate, or a desktop app's recorded tunnel port, is refused too (review of 96786517). The relay is done on the
   control socket (`crates/agora-node/src/preview_relay.rs`): frames 20..22 under credit, chunk, rate and total budgets, at most 32
   connections per forward and 64 per node (one more ends at once with `BUDGET`, the maintainer's choice of 2026-11-27 instead of a new
   error code), the port checked again at every connect. The hub link carries connections too (`uplink_client.rs`: a local
   connection per preview connection, a 16-frame queue sent only while no answer or terminal frame waits, frames tied to their
   connection), and the node offers `preview.v1`. Measured (release, blackview, 2026-11-27): a command's round trip through the link
   p50 98 us / p95 147 us alone, p50 294 us / p95 459 us while one preview connection carries 749 MB/s, well inside ADR-0022's
   5 ms of Agora overhead for a remote node's keystroke round trip. Forward ids are 128 random bits (review of 6221f9c9). Done.
3. Hub relay, origin per forward, audit: not started. Decided 2026-11-27 (the maintainer): only the person who opened a forward may
   connect to it through the hub; the per-forward origin follows the accepted open question (a separate hub port where possible,
   otherwise refuse and say why).
4. Web Browser pane: not started.
