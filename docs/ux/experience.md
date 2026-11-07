# Experience principles

The product should feel fast, calm, and powerful. These principles guide every screen.

1. **Instant.** Every click and key responds within one frame. If work takes longer, show progress in place,
   never a blocking spinner. Budgets: `design/performance.md`.
2. **Calm by default, loud when it matters.** Working agents are quiet; blocked agents and approvals stand out.
   One accent color, minimal chrome, no decorative motion.
3. **Keyboard and mouse are equal.** Everything is clickable; everything has a shortcut; a command palette
   reaches every action and every object (agent, project, file, node, task) by fuzzy search.
4. **Everything is live.** All clients show the same truth in real time; another client's actions appear
   without refresh.
5. **Never lose work.** Closing the window, losing the network, or updating never stops agents or drops input.
6. **Explain state.** Every badge answers "why?" on hover: who reported it, when, and from which source
   (agent report or screen detection).
7. **Simple on the surface.** Glance, Focus, Deep: depth appears only when asked for. Plain words, smart
   defaults, zero configuration to start (`visual-language.md`, ADR-0024).
8. **Global.** Every string localized, RTL-ready layouts, WCAG 2.2 AA.

## Signature features

These are what make the product memorable; each must meet the performance budgets.

- **Command palette** (Ctrl/Cmd+K): jump to any agent, project, file, or node; run any action; start an agent
  of any kind on any node in one line.
- **Attention flow:** a "next that needs me" shortcut cycles through blocked agents and approvals across all
  machines, ordered by waiting time.
- **Fleet view:** a dense overview of every agent across every machine and project, with live activity
  sparklines; click to dive in.
- **Presence:** when several clients or team members are connected, see who is looking at which agent; follow
  someone's view.
- **Global search:** search across all live terminals, agent transcripts, and mesh messages on every node, with
  results streaming in as nodes answer.
- **Timeline:** per project, a scrubbable history of agent starts, state changes, handoffs, approvals, and
  merges, built from the event store.
- **Predictive typing** on remote terminals so a server on another continent feels local.
- **Connection quality indicator** per node, with the per-hop latency breakdown one click away.
