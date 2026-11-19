# 0088. Preview an agent's local web server in the window
Status: Proposed
Date: 2026-11-19

Not accepted. Nothing here may be built until the maintainer names this ADR in an acceptance message.

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
   named, to `127.0.0.1` on that machine, never to another host. A forward has an id, a port, an expiry (default 8 hours) and a creator.
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
