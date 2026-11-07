# Phase 3 — Hub MVP and web client

Exit criteria (roadmap): two machines' agents visible and controllable in one browser window; interaction and
rendering budgets met; time to first agent ≤ 60 s; first-glance test passed with five users.

Work proceeds in this order; each item is its own PR. Items that need a maintainer decision (a `Proposed` ADR, real
agent accounts, usability sessions with people) are marked and skipped, never faked.

| Task | State | Notes |
| --- | --- | --- |
| P3-01 hub event store | done | SQLite, exactly-once per `(producer, seq)` |
| P3-02 node session | done | state machine; acks wait on ADR-0026 |
| P3-03 WebSocket transport | done | one frame per binary message; stream-generic so TLS wraps it (ADR-0011, ADR-0013) |
| P3-04 hub server | done (service) | `agora-hub::hub`: store + view + fan-out under one lock, slow subscribers dropped, hub-recorded node-offline; `agora-hub::server`: node, client, enrollment over WS and mTLS TLS, snapshot, acks, revocation; the `agora hub run` command and the node dialer are next |
| P3-05 node uplink | done (library) | `agora-node::uplink` + end-to-end test with the hub session (resume, no dups/gaps, restore, stale, two machines in one view); `agora-node::uplink_client`: dials wss with the node's certificate (pinned CA), reconnects with backoff, acks trim the buffer; CLI wiring (`agora node enroll`, `agora hub run`) is next |
| P3-06 enrollment and mTLS | done | one-time codes (hashed, 10 min, lockout after 5 misses), CSR signing, revocation; wire messages for enrollment (need an ADR) and wiring the acceptor into the hub server still to do |
| P3-07 projections and snapshot | done (view) | `View` folds events; snapshot encoding needs a schema message (ADR to come) |
| P3-08 client API | next | session tokens, snapshot + event subscription, command routing to nodes |
| P3-09 terminal relay | next | terminal streams node → hub → client (input outranks output) |
| P3-10 web client shell | partly done | tokens, state badge, tile, attention strip, Home and gallery on demo data; hub serves the built app over HTTP on the same port (`agora hub run --web <dir>`, SPA fallback, CSP) and takes the browser's token as a subprotocol; TS frame codec, hub-fed store and visual regression tests still to do; app not yet embedded in the binary; Focus view (terminal placeholder), command palette (Ctrl/⌘+K) and attention flow (N) done |
| P3-11 terminal rendering | adapter done, measurement open | `TerminalRenderer` interface with an xterm.js implementation (lazy-loaded), frame decoding, demo source in Focus; the xterm.js vs WebAssembly choice still has to be measured in a real browser against ADR-0022 |
| P3-12 SSH bootstrap | stdio bridge done | `agora node stdio`; hub-side SSH dialer and installing the node on a fresh machine still to do |
| P3-13 notifications, doctor, backup/restore, usage view | doctor done | `agora doctor` (checks: terminals, sockets, git, data dir, node, manifests, installed agents); notification routing rules (`agora-hub::notify`), backup/restore library and cost estimation done; delivery channels, views and CLI commands still to do (cost estimation in `agora-hub::usage` done) | `design/notifications.md`, `backup-restore.md`, `usage-and-cost.md` |
| P3-14 first-agent flow | needs a human | needs P2-09 manifests from real agents; five-user first-glance test |

Not a Phase 3 task but done early: Turkish (`locales/tr`) for CLI, errors, notifications and the web client (Phase 6 lists first non-English locales).

## Running it (Phase 3 slice, from the command line)
```
agora hub run --hub-dir ./hub --listen 0.0.0.0:7443 --name hub.example.com   # CA, token and database live in ./hub
agora hub code --hub-dir ./hub                                              # a one-time code (10 minutes) and the CA path
agora node enroll --hub wss://hub.example.com:7443 --ca ca.pem --code XXXX-XXXX --id laptop
agora node run --project ~/code/myproject                                   # connects to the hub by itself
```
`crates/agora/tests/hub_cli.rs` runs exactly this with real processes. Clients authenticate with the token in `<hub dir>/token`
until sign-in (passkeys, ADR-0015) exists.
