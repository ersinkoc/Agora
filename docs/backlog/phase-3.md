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
| P3-04 hub server | next | accept nodes, run sessions, store events |
| P3-05 node uplink | next | node dials hub, replays its buffer after the hub's `from_seq` |
| P3-06 enrollment and mTLS | codes + CA done | one-time codes (hashed, 10 min, lockout after 5 misses), CSR signing, revocation; wire messages for enrollment (need an ADR) and the mTLS listener still to do |
| P3-07 projections and snapshot | done (view) | `View` folds events; snapshot encoding needs a schema message (ADR to come) |
| P3-08 client API | next | session tokens, snapshot + event subscription, command routing to nodes |
| P3-09 terminal relay | next | terminal streams node → hub → client (input outranks output) |
| P3-10 web client shell | partly done | tokens, state badge, tile, attention strip, Home and gallery on demo data; hub-fed store and visual regression tests still to do; Focus view (terminal placeholder), command palette (Ctrl/⌘+K) and attention flow (N) done |
| P3-11 terminal rendering | next | renderer choice measured against ADR-0022 |
| P3-12 SSH bootstrap | next | framed protocol over SSH stdio (ADR-0011) |
| P3-13 notifications, doctor, backup/restore, usage view | doctor done | `agora doctor` (checks: terminals, sockets, git, data dir, node, manifests, installed agents); notifications, backup/restore and usage view still to do | `design/notifications.md`, `backup-restore.md`, `usage-and-cost.md` |
| P3-14 first-agent flow | needs a human | needs P2-09 manifests from real agents; five-user first-glance test |
