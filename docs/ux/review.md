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

## Built today (web)

The review screen opens from an agent's "files changed" chip, from Changes in Details, and from a closed agent's summary
(`apps/web/src/components/ChangesView.tsx`, read-only):

- Left: the changed files grouped by folder (folders fold), each with a status letter (changed, added, deleted, renamed) and
  +/− counts; filters by status with counts, and a filter by name. Keys: `j`/`k` (or arrows in the list) move between files.
- Right: one file at a time, unified or side by side (remembered per browser), line numbers on both sides, the changed part
  of a changed line marked, long lines wrapped on request. A new or deleted file is one column. A rename shows the old name;
  a binary file, or one whose name or permissions alone changed, says so.
- Unchanged lines between hunks are folded. "Show N unchanged lines" or "Whole file" reads the file as it is now
  (`files.read`) and shows them only while it still holds the diff's new side; otherwise it asks to read the changes again.
- A diff the node cuts at 512 KiB says so; the cut-off last file can be read on its own (`git.diff` with a path).
- Very long files are drawn in parts ("Show all N lines").

Syntax colours (highlight.js, loaded on demand; about 25 languages by file name) colour each diff line on its own (in
batches, so no task blocks the page for long; up to 512 KiB of lines), and lines of the file as it is now from a pass over
the whole file (up to 128 KiB) once it has been read.

Not built yet: base-branch comparison, inline comments to the agent, merge/rebase/discard, conflicts and
the multi-agent comparison. Unchanged lines of a file that changed again after the diff was read, or of a deleted file, would
need the file as committed (at HEAD); no protocol message reads a file at a revision yet.
