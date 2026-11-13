# 0070. The hub owns whether a machine is online
Status: Accepted
Date: 2026-11-14
Accepted: 2026-11-14 (maintainer's explicit approval in chat: "0070'i onaylıyorum"). The open questions were not answered; the proposed answers stand (25 s silence deadline with a 10 s ping; "last seen" shown in the window and the TUI).
Origin: the Windows and macOS agents' freeze probes on real machines (2026-11-14): a node or a hub that goes silent without its TCP connection ending (a laptop asleep, a Wi-Fi switch, a NAT timeout) is **not noticed in either direction**, and the first attempt to notice it (PR #453, since taken out of 0.3.8) showed why the design is not finished.

## Context
Today "online" is derived from node-produced events: `NodeOnline` (sent once, when the node *process* starts) and `NodeOffline` (written by the hub when a connection ends). ADR-0055 made `NodeOnline` mean "a start the hub never saw", with reset semantics (its agents end in the hub's view). So a machine whose connection ends and comes back **has no event that says "online again"**: the hub marked it offline and nothing in the model can mark it online without a reset.
Measured: after a hub restart the machine returns (its link resyncs); after a *silent* freeze nothing is noticed; when an attempt closed a silent link and recorded offline the machine **stayed Offline after the node redialled**; and a window that connected while the hub was frozen stayed on "Connecting…" because the browser's connect had no deadline (fixed separately in 0.3.8).

## Decision
1. **Presence is a fact the hub owns**, not a claim a node makes. A new hub-produced event `NodeLink { node_id, up: bool, at, reason }` (additive, new event number, golden fixtures) is written by the hub when a node connection **opens** (after the handshake) and when it **ends or is closed as silent**; the projection takes `online` from the latest `NodeLink`, and `NodeOnline`/`NodeOffline` keep their meaning for old data and old nodes (clients treat a machine as online if its latest presence fact says so, whichever kind it is).
2. **The hub notices silence** (the part of #453 that was right): the node pings every 10 s; the hub closes a node connection after 25 s with nothing received (it keeps answering pings), writes `NodeLink{up:false, reason: silent}` and the window shows the machine offline with "last seen"; **when the node redials, the open of the new connection writes `NodeLink{up:true}` and the machine is online again by itself**, with no reset of its agents (a link that goes down and up is not a process start).
3. **The node notices silence too:** no frame (a pong or anything) for 25 s drops and redials; after a process was suspended (the monotonic clock jumped) it redials at once.
4. **The window notices silence too:** its own ping every 15 s over the existing PING/PONG, reconnect when nothing comes back in 10 s, and the connect deadline of 8 s (already in 0.3.8); "Retry now" stays.
5. **Old nodes and old hubs interoperate:** a new hub with an old node still pings-by-reading (the old node already pings every 15 s) and writes `NodeLink`; an old hub ignores nothing new because only the new hub writes the event; clients without the event fall back to `NodeOnline`/`NodeOffline`.
6. **Agents are never affected by presence:** a link going down does not end an agent in the hub's view until the node reports so or the hub's resync rules (ADR-0054/0055) say so; "machine offline" is shown separately from "agent finished" (the closed/"Stopped with the machine" cards of ADR-0060 stay as they are).

## Open questions (proposed answers stand)
- Silence deadline 25 s with a 10 s ping (proposed): fine for laptops on Wi-Fi? (A shorter deadline flaps; a longer one hides sleep.)
- Show "last seen 3 min ago" in the window and the TUI (proposed: yes).

## Consequences
One new event and fixtures, projection change, hub/node/web silence logic (the tested parts of #453 are the starting point), tests with a fake peer (frozen node, frozen hub, a real hub outage, a flapping link), `observability.md` and the backlog item R1 closed, and probes `scripts/link-silent.mjs`/`link-resilience.mjs` (Windows) as the acceptance tests on all three platforms.
