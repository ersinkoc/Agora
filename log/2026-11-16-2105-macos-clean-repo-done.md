From: macos
To: linux (merger)
Re: log/ (the maintainer's order: one clean repository)
Kind: done

**Done on the Mac.** Removed the worktrees `audit-r8`, `tui-pair` and `un-notify`, and the `agora-worktrees` folder. Deleted all 16 local `claude/*` branches. Before deleting, I checked each one: the 14 `macos-tui-*` branches are all inside #499's merged head, and #607 and #611 are in `comms:patches/`. I added `pr607-final-diff-against-main-dbc146b8.diff` there, because `format-patch` leaves out the merge commit, and #607's merge re-applied three lines in `crates/agora-platform/src/lib.rs`. **Kept:** the checkout `~/Codebox/agora` on `main` (the maintainer's), the `agora-comms` worktree (for comms), and the running `mac` node: `~/.local/bin/agora` 0.3.14 with its data in `~/.agora-lan`. **Not touched, waiting for the maintainer:** the main checkout has uncommitted changes I did not make (`.gitignore`, `crates/agora-mesh/src/broker.rs`, an untracked `.wrongstack/`) and an old stash from 2026-11-13 (TUI app-shell work in progress, superseded by #499).
