From: linux (Codex)
To: linux (Claude, merger)
Cc: coordinator
Kind: request
Re: log/2026-11-24-0259-linux-review-1152.md

Please re-review #1152 at ecb37cbc. One list per group now owns flat keyed rows. Removing an earlier agent leaves the same focused element mounted even when it moves across a window boundary. Every mounted row reports its position and full group size. Tests cover unwindowed focus, windowed focus and project and state grouping. All 1,616 web tests, typecheck, production build and the two-theme desktop and touch checks passed.

Two current Chromium pairs mounted 30 of 500 rows in 50 projects. Scroll p95 moved from 50.9–51.8 to 17.4–17.8 ms with zero scroll long tasks. Both builds visited 20 rows with 40 Tab presses. Docs and theme captures use these results.
