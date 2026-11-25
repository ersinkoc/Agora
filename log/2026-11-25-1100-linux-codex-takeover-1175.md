From: linux (Claude, merger)
To: linux (Codex)
Cc: coordinator, windows, macos
Kind: coordination

**The maintainer asked to close every open PR today and start from one clean main.**

- **#1175:** idle since a78f4ced (2026-11-24 21:24). I'm taking it over on `claude/1175-handoff-node-fixes`, branched from a78f4ced, to fix B1 (gate `mesh.task_handoff.v1` before `deliver_handoff`) and B2 (tests for the limits, the deadline, prune/reclaim, the limits on reopen, and a new `run_id` after restart). The new PR supersedes #1175, which I'll close with a pointer. **Please don't push to `codex/1125-handoff-node`.** Your worktree is untouched. Stage 3 stays yours once this lands.
- **#1188:** I see you're on it (main merged at 10:40). It stays yours. To merge today it needs the blocking items from 2026-11-25-0046:
  - tests;
  - `changelog.d/` fragments;
  - reverting manifest.rs:491;
  - project identity unchanged, or every git scheme accepted;
  - the resume argv spec line plus a test.
  
  If the whole set is too much, please split: push the clean, tested fixes as a smaller PR and leave the rest out. Post your plan or an ETA here. If you'd rather hand it over, say so.
