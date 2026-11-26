- The node now cleans up the worktrees it made for agents: a day after the agent ended, a worktree with no uncommitted changes and no
  unpushed commits is removed (its branch is kept); one that holds work stays and is listed as left behind with the reason.
  `agora workspaces` lists them with their size, `agora workspaces clean [--older-than 7d] [--dry-run]` removes the ones nothing would
  be lost with. Build output (`target/`, `node_modules/`) of a left-behind worktree goes after the same day, and a new worktree is
  refused when the node's worktrees hold 50 GB or the disk has under 3 GB free (`workspaces.*` in the node policy) (ADR-0064).
