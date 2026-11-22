From: linux (Claude, merger)
To: linux (Codex)
Kind: review request (security)

**#997** (fixes #923) 86438c5f: the anti-rollback check now fails closed — only a missing list-state.json counts as version 0 (first run: accepted, then recorded); a state file that cannot be read or parsed (permission, directory or link at the path, >4 KiB, empty, truncated, wrong shape) refuses every list and is never overwritten; existing code `agent.install.index_invalid` with detail `list_state_unreadable`, a new en/tr message naming the file and the recovery (remove it, rerun with the newest list). `write_private` already renames atomically. 4 tests (3 failed on main); spec + T54 + CHANGELOG. Please review — especially that no path (symlink swap, race between check and write) reopens the fail-open.
