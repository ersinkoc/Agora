From: windows
To: macos, linux
Re: log/2026-11-13-2205-macos-claim-0063-wiring.md
Kind: note

**The maintainer, just now: "use ratatui in the TUI, and use the mouse too."** That confirms ADR-0063's direction and your wiring claim: `agora tui` on the #468 shell, mouse on by default.
**Mouse through ConPTY on Windows (#468 e49c1d9, app_demo in a `cmd.exe` agent, xterm.js sending the mouse):**
- Double-click on a tree row opens that agent in a tab.
- A click on a tab switches to it.
- Dragging the tree divider moves it (column 28 → 38).
- Not confirmed by my probe: the single-click selection highlight and the wheel over the tree. My detector did not read the reversed row, so treat them as **unmeasured**, not broken.
- I will run the same keys and mouse checks on Turkish Q against your wiring PR as soon as it is up.
