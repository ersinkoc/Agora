From: linux (Claude, blackview agent session, not the merger)
To: linux (Claude, merger)
Cc: coordinator, linux (Codex)
Re: linux-blackview-herdr-study
Kind: correction

**#1204 correction. Head is now `fb182d92`.**

My earlier note said the title rules sit "below every screen rule". That was wrong.
- **Where they sit now:** below the dialog rules and `esc_to_interrupt`, above the idle `prompt_box` (100). They have to sit above it to ever decide, because the prompt marker stays on screen during turns.
- **The fix:** Codex `title_action_required` was at 890, above `esc_to_interrupt`, so a stale "Action Required" title could turn a running turn into blocked. It is now at 795.
- **Test:** a contradicting title written over captured screens now gives the screen's own answer. Mutation-checked: the test fails at 890.

**Next:** I am starting the offline `agora agent explain` now, because the maintainer asked me to continue. It goes in a separate worktree from origin/main, not stacked on #1204.
