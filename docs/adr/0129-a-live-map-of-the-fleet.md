# 0129. A live map of the fleet
Status: Accepted
Date: 2026-11-27
Accepted: 2026-11-27 (the maintainer's answer, "Accept": a fixed radial layout, no new dependency, this window only). The open questions'
proposed answers stand.

Origin: roadmap M8.8 ("a live graph of the hub, machines, windows, agents and helpers, with the mesh messages, tasks and handoffs moving
between them, in the web window and the desktop app"; **decision**: an ADR and a graph-drawing dependency first).

## Context
- The window already has everything a map needs except other people's windows: machines and their links (`NodeOnline`, `NodeLink`),
  agents and their machines, helpers and their parents (`parentId`), mesh messages (`MessagePosted`), tasks, dispatches and handoffs
  (`TaskChanged`, `TaskDispatched`, `TaskHandoffDeliveryChanged`), previews. The hub does not tell a window about other windows.
- The fleet has a fixed shape: one hub, its machines around it, agents on their machine, helpers below their parent. It is a tree
  with a few cross-links (messages, handoffs), not an arbitrary graph.
- ADR-0022 holds the window to its budgets; ADR-0024 to a quiet, simple look. The window has no graph library today.

## Decision (proposed)
1. **A `map` panel** (global, like Usage), in the web window and so in the desktop app; the terminal client names it and says "open
   in the window" (M8.9 may draw a text version later).
2. **No new dependency: a fixed radial layout drawn as SVG.** The hub in the middle, machines on a ring around it (sorted by name, so
   the same fleet always draws the same way), each machine's agents on an arc beside it, a helper one step further out from its
   parent. Offline machines are drawn faded, with their agents. The layout is a pure function of the view (render is pure); a graph
   library (d3-force, ELK) would buy automatic layout this shape does not need, and moving pictures that never settle.
3. **What moves.** A mesh message is a dot that travels from its sender to its recipient (through the hub when they are on two
   machines); a task given to an agent, a dispatch and a handoff are a small card that does the same; each takes about a second, at
   most 20 at once (the oldest finishes at once beyond that), and nothing moves under `prefers-reduced-motion` (the edge flashes
   instead). Animation runs only while something moves (`requestAnimationFrame` stops when idle).
4. **What a click does.** An agent opens it as the agent list does; a machine opens its page; a moving item shows its words
   (sender, recipient, the message's first line, the task's title) in a tooltip, as untrusted text.
5. **Windows.** This window is drawn beside the hub. Other people's windows are not shown: the hub does not send them, and showing who
   is watching is a privacy decision of its own (a later ADR with a schema message if wanted).
6. **Budgets.** At 100 agents and 20 moving items the map draws a frame in under 4 ms on the reference machine; a test holds the
   layout's cost, and the panel draws nothing while it is not on screen.

## Consequences
- The person sees at a glance which machines are up, which agents run where, who started whom, and the traffic between them.
- No dependency to vet or update; the layout is deliberately simple and will not suit a fleet of thousands (the window does not either).

## Alternatives considered
- **d3-force or ELK**: automatic layout of any graph; rejected for the reasons in decision 2, and a dependency of its own.
- **A full page instead of a panel**: a panel can be maximized and sits beside a terminal; one place for both.

## Open questions (with proposed answers)
1. No graph library, a fixed radial layout: yes.
2. Other people's windows on the map: not now (privacy, schema).
3. The terminal client: names the panel only, until M8.9.
