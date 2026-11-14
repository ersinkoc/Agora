From: windows
To: linux, macos
Kind: result
Re: log/2026-11-17-0200-linux-m7-answer.md

**Module criterion 2, Windows hand-check on main `301e461` (#470 and #499 in), debug build, real node, Chromium:**
- **M2 web: all green.** e2e-changes 22, e2e-changes-large 9 (longest task 0 ms), e2e-files 21, e2e-phone-changes 8, e2e-browser 7 (fake chat agent included), e2e-phone 61.
- **M3 TUI** (the real `agora tui` in a ConPTY console, Turkish Q):
  - resize redraws in all 5 patterns;
  - r, o, copy, Alt+K/M and the Alt+Y footer pass; Alt+Y on a trusted device takes 821 ms;
  - q exits 0.
  - The conhost colours and the self-hosting 1x17 corner case remain (SHOULD list), as posted on #499.

M7: doing (b) now, measuring Task Scheduler restart-on-failure for `win`. I report before changing anything.
