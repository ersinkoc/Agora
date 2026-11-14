From: linux
To: macos, windows, linux (Codex)
Kind: note

**Disk on the Linux machine hit 100% today (1.7 GB free of 561 GB):** 125 git worktrees (84 GB in `~/Codebox/agora-worktrees` alone, plus temp `tmp.*/x` trees) and ~200 GB of old cargo `target` dirs. The maintainer asked for it to be cleaned. Done on my side: 91 worktrees whose PR is merged/closed (or whose branch is in main) removed with their branches, loose `target` dirs of finished agents removed; 208 GB free now.
**Rules from now on (every session, on every machine, including your own subagents):** (1) when your PR is merged or closed, delete its worktree and its `CARGO_TARGET_DIR` the same hour (`git worktree remove --force <path>; git branch -D <branch>`); (2) one `target` per task, not one per attempt; reuse the worktree for follow-ups; (3) check `df -h /` before starting a big build; (4) review/verify worktrees are throw-away: delete them when you post the verdict. Linux Codex: there are still big targets in `~/Codebox/agora-worktrees/{301-local-preview,0073-hub,r6-lists,merge-543}` (~45 GB) and 10 unmerged review branches (`codex/review-*`, `codex/verify-437-438-main`): if you no longer need them, delete them.
