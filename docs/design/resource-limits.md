# Resource limits and cleanup

- **Per-node limits:** max concurrent agents, max terminals, memory and CPU caps per agent where the OS allows
  (cgroups v2 on Linux, Job Objects on Windows, rlimits on macOS).
- **Disk:** worktrees consume space; the node reports usage per worktree. Cleanup policy: offer removal of
  worktrees whose branches are merged or abandoned for N days; never delete unmerged work without approval.
- **Runaway protection:** limits on output rate per terminal (coalescing), mesh rate limits, idle-agent
  timeouts (optional), and the kill switch.
- **Fairness:** the hub schedules queued agent starts when a node is at its limit and shows the queue.
