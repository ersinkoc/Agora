# Resource limits and cleanup

- **Per-node limits:** max concurrent agents, max terminals, memory and CPU caps per agent where the OS allows
  (cgroups v2 on Linux, Job Objects on Windows, rlimits on macOS).
- **Disk:** worktrees consume space; the node reports usage per worktree. Cleanup policy: offer removal of
  worktrees whose branches are merged or abandoned for N days; never delete unmerged work without approval.
- **Runaway protection:** limits on output rate per terminal (coalescing), mesh rate limits, idle-agent
  timeouts (optional), and the kill switch.
- **Fairness:** the hub schedules queued agent starts when a node is at its limit and shows the queue.

## Implementation status (Phase 2)

- **Done:** `max_agents` and `max_terminals` (`NodeLimits`; a refused start returns `node.resource_limit` with
  `resource` and `limit` and leaves no worktree behind); per-process `RLIMIT_CPU` everywhere on Unix and
  `RLIMIT_AS` on Linux (`ProcessLimits`, applied in the child before exec; macOS does not enforce address-space
  limits, so none is applied there); worktree disk usage (`files::disk_usage`, `Supervisor::disk_usage`);
  output rate coalescing (`coalesce::Coalescer`: 16 ms / 64 KiB, so a lone keystroke waits at most one frame).
- **Not yet:** cgroups v2 memory/CPU caps on Linux (needs a delegated cgroup, so it is best-effort and will be
  added with the platform's service integration), Job Objects (Windows, Phase 4), worktree cleanup offers,
  mesh rate limits, idle-agent timeouts, the kill switch, hub-side start queue.
