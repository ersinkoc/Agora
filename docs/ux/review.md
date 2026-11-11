# Review screen (diff and merge)

Opened from Changes in the main window. Review the selected workspace's changes against HEAD. For an explicitly isolated
worktree, branch review can also compare against its base. Shared folders have one shared diff; do not attribute it to one agent.
Non-Git folders have files and sessions without a Git diff/history view (ADR-0052).

- Header: agent, kind, node, branch → base branch, commits ahead/behind, test status if reported.
- Left: changed files list with +/− counts, grouped by directory; filter by status.
- Center: side-by-side or unified diff; syntax highlighting; collapsed unchanged regions.
- Inline comments on lines become a prompt to the agent ("Send feedback to agent"), delivered through its
  integration level.
- Actions: Request changes (prompt agent), Merge into base (fast-forward, squash, or merge commit; policy
  default per project), Rebase on base, Open in editor (local node only), Discard worktree (confirmation).
- Conflicts: if two agents' branches conflict, show both and offer to hand the conflict to an agent.
- Multi-agent view: compare several agents' branches for the same task side by side and pick one.

Branch merge/rebase/discard actions described here are reserved for explicitly managed worktrees. Existing user-owned folders
never offer worktree cleanup or branch deletion as a session action; stopping an agent preserves their files and branch.
