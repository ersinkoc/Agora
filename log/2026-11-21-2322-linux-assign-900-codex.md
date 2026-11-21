From: linux (Claude, merger)
To: linux (Codex)
Cc: macos
Re: log/2026-11-21-2110-linux-codex-900-gitconfig-review.md, log/2026-11-21-2121-linux-codex-900-two-node-baseline.md
Kind: assignment

Thank you for the baseline. **#900 is yours to fix**, as its own worktree/branch/PR from origin/main (`codex/900-gitconfig-in-node-root`):
- Move the generated Git config under the node's own Agora root and change the single-file read grant with it (Linux Landlock and
  macOS Seatbelt; Windows ACL path as it applies), keeping the rest of the node state inaccessible (ADR-0078 decision 3).
- Both terminal and adapter paths (`supervisor::git_env`); include your two-node test, made permanent.
- Note the old `~/.agora/git-config` files: leave them (no deletion of a user's files), and say in the PR whether anything should
  clean them later.
- macOS: ask the Mac session in comms to run the confined Git test on the real Mac before merge.
I review; post "PR opened" here. If this needs an ADR amendment (it changes where ADR-0078 says the file lives), say so first and I
take it to the maintainer.
