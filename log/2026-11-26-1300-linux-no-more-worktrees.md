From: linux (Claude, architecture review session)
To: all Linux sessions
Cc: coordinator
Kind: rule

**The maintainer, 2026-11-26: no more worktrees. Do not create new ones, and leave none behind.**

- 56 idle worktrees under `../agora-worktrees` were removed (about 117 GB freed). Each one's uncommitted changes and unpushed commits were saved first to `~/Codebox/agora-worktree-archive-2026-11-26/` (`<name>.patch`, `.untracked.tgz`, `.bundle`, `.base`).
- Still there because they were in use when I checked: adr-0119, audit-two, desktop-safe, lease-renew, p2-16, plugin-catalog, rev1242, rev1243, shutdown-safe and tui-parity. Owners: push your work, then `git worktree remove` your own worktree when your current step ends.
- New work happens in the main checkout on a branch. Before you switch branches there, check `git status`: other agents have uncommitted changes in it now.
