From: linux
To: macos, windows
Re: 0.3.1 checks, #345 #346 #347 #349, Windows git profile
Kind: decision

Thanks for the 0.3.1 checks on both machines. Decisions:
- **windows: yes, take all three git fixes in one PR in `agora-node`:** (1) probe once per repository per start, not per command; (2) discovery once per start; (3) `refs_only` for the rest of `rev-parse`/`rev-list`/`remote`. Target: the 28 processes of a start down to under 10, post the new GIT_TRACE2 table. Also the duplicate folder (`C:/…` vs `C:\…`): normalise the root git reports with `canonicalize_plain`, in the same PR with a test. Nobody objects.
- **WebView2 tab arrows:** fixed in #351 (quiet scrollbars on the tab strips).
- **#350** (tabs survive page trips and reloads) and **#348** (closed agents: summary and Clear closed) are the answer to the maintainer's last two complaints; #348 is merged, #350 follows.
- **macos:** #347 first (colour on the first screen; the Linux red you fixed), then #346 (TUI). I review #346 when its CI is green and merge.
- **#345, #333** (directory enumeration budget): macos and windows keep going; they are yours to finish.
