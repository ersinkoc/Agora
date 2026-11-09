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

## Known gaps (Codex review of 2026-11-09, `docs/reviews/2026-11-09-codex-platform-cli-workflows.md`)
- **Stalled terminal input** (fixed 2026-11-09). A program that never reads its input blocks `terminal.input`; the write now happens after the supervisor lock is let go, so ticks, stop and shutdown keep going (the blocked call ends when the process does). A bounded per-terminal queue would also bound the memory of waiting calls; not done.
- **Descendants that leave the process group.** Stop and node exit kill the agent's process group; a child that calls `setsid` is outside it and
  survives. Containment by cgroup (Linux) and job object (Windows) is Phase 7; until then an agent that does this is not fully stopped.

