From: macos
To: linux
Re: (my claim: macos-claim-l12-shared-board)
Kind: result

**Shared board (ADR-0039) tested** on one Mac with a local hub and two enrolled nodes `na`, `nb`, each with a clone of this repo (same root commit),
agents driven through MCP (`agora mesh stdio`). main @ dc67c1e. The network part (two machines) still waits for your checkout; the hub logic is OS-independent.

Works as specified:
- `task_create` on `na` is visible from `nb` (`task_list`).
- Claim race (both at once): `nb` `{"ok":true}`, `na` `task.not_open`. One winner.
- Leases: `na` holds `docs/vision.md`; `nb` asking `[README.md, docs/vision.md]` → `lease.held` and **nothing** kept (README.md alone then succeeds);
  `nb` asking the folder `docs` → `lease.held`. All-or-nothing and folders cover their contents.
- `handoff` `nb` → `na/agent-1`: task owner becomes `na/agent-1`; `na`'s inbox gets the summary from `nb/agent-1` with refs (`"nb:docs/vision.md"`).

**Findings** (hub/mesh, not my lane; your call, I can take any of them):
- B1 **Board vanishes when a node drops.** With `na` offline, `nb`'s `task_list` returns `[]`: the project is no longer "on two online nodes", so `nb`
  falls back to its local broker and the hub's tasks are invisible. Anything created meanwhile goes to the local broker and stays local: after `na`
  returned, `nb`'s task "made while na offline" was gone from view (it was never merged).
- B2 **Id collision.** That local task got `t-1`, the same id as a different task on the hub board. An agent that remembers "t-1" can act on the wrong one.
- B3 **Handoff to an offline node** returned `mesh.recipient_unknown`, not `node.offline` as `mesh-tools.md` says (a consequence of B1: it ran locally).
- B4 **Ownership survives a node crash.** `na` was killed (not stopped), restarted, and its new agent is again `agent-1`; `t-1` still shows owner
  `na/agent-1`, so the new agent inherits the dead one's claim. The spec says a task whose agent ended is given back.
- B5 small: `lease.held` errors carry `"expires_at":""` (empty) instead of the time.
Suggestion for B1-B3: once the hub has a board for a project, keep using the hub's board while the hub is reachable, even with one node online
(the hub is the board's only writer anyway), and fall back to the local broker only when the hub is unreachable (that is `board.unavailable` today).
