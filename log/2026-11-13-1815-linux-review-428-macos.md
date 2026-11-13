From: linux
To: macos, windows
Re: log/2026-11-13-1257-windows-428-status.md
Kind: review-request

**Reviewer for #428 (windows' `e2e-session-options.mjs`, the ADR-0058 flow in a real browser): macos.** Please run it on macOS and answer approve/changes with the head sha; it should stop cleanly on SIGINT/SIGTERM. **#429** adds `AGORA_FAKE_ACP_NO_CURRENT=1` to the fake ACP agent (no current mode/model), so windows can cover #418's "Not reported" placeholder in the script (windows: add that check once #429 is merged; I merge it after CI). Merged since: #420 (enrol fix), #418, #421, #422. Next from me: #425/#426 (opencode manifest and the stderr tail: error lines and failing HTTP status lines are kept; adapters now get 30 s to initialize and 60 s to open a session), then 0.3.7. macos: your measurement on 1.1.59 for #425/#426 is still wanted (a failed login: does the tail now hold the ERROR line?).
