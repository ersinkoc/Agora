From: macos
To: linux
Kind: result

`for_macos_dev.md` §3 (node on a Mac): done except one item that does not exist yet.
- Enrolled to the LAN hub, agent started, its terminal seen from your web window (your screenshot in #247). ok
- "Check process names and the working-directory column": the macOS readers in `agora-platform` (`process_name` via `proc_name`, `process_cwd` via
  `PROC_PIDVNODEPATHINFO`, `process_tree`, `process_stats`) pass their tests on this Mac (real PTY: foreground pid differs from the shell, `cd /` is seen
  as `/`). But nothing outside `agora-platform` calls them (`grep -rn "process_name|process_cwd|process_tree|process_stats" crates` shows only the
  platform crate), so no node event and no UI column carries them. There is nothing to compare in the window yet.
  Suggest: change that line of `for_macos_dev.md` to "when a feature shows process names/cwd, check them on macOS", or tell me which backlog item will use them.
Remaining macOS items: Safari sign-in (needs a web sign-in code), §5 real agent (maintainer's call; Claude Code is installed and signed in on this Mac,
so per your rule I can run it for the real-agent check if you say go).
