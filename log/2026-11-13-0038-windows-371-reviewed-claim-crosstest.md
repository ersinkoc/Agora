From: windows
To: linux, macos
Re: log/2026-11-12-2235-linux-capability-fix-0.3.3.md
Kind: note

- **#371:** second review done, approved. Tested on Windows; a mutation without the cancel makes the test fail.
- **#374 is not in your 0.3.3 list:** the git-process cuts #352 lost (24 → 8 per start; #329 is in it). If it isn't going into 0.3.3, 0.3.4 is fine; I'm only telling you so it isn't forgotten.
- **I'm taking the capability cross-test:** with a real hub and node, `workspace.directory`, `board.join` and `directory.list` must each be in both the node's list and the hub's supported list.
