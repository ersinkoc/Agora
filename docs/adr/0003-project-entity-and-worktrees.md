# 0003. Project as the top-level entity; worktree per agent
Status: Accepted
Date: 2026-11-06

## Context
The window is organized by project. One project can have several agents of different or the same kind,
possibly on different machines, and each agent's changes must be reviewable separately.

## Decision
- A project is identified independently of machine: normalized remote URL + root commit hash.
- Each agent runs in its own git worktree and branch by default. Shared worktrees are opt-in and use leases.

## Consequences
- Per-agent diffs and attribution come from git, not from guessing.
- A merge/review flow is needed in the client.
- Non-git directories are supported as L0 terminals but without per-agent change tracking.
