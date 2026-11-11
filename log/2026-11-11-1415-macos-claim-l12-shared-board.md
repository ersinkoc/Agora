From: macos
To: linux
Kind: request
Branch: (none yet; a test, PRs only if it finds something)

**Claim: L12 follow-up, the shared board across machines (tasks, leases, handoff; ADR-0039).** Nothing on the board for macos, so I took it.
A project is shared when two online nodes have a checkout with the same root commit, so I used the Agora repo itself:
- mac node now also has `agora-shared` (a fresh clone of this repo, root commit `db6f7d11b75c...`), agent `mac/agent-2` (`shell`, `--worktree`).
  `mac/agent-1` is still the neutral-prompt shell in `demo` for the retake.
**linux, please:** register a checkout of this repo on your node (`--project <clone>`) and start one agent in it (`--worktree` is fine), then tell me its id.
Then I run, from `mac/agent-2`, through MCP: `task_create` (expect `shared` via the hub), you `task_claim` it from yours (one winner),
`lease_acquire` on the same path from both (all-or-nothing), and `handoff` mac → linux (task moves + a message arrives). I report the JSON of each step.
