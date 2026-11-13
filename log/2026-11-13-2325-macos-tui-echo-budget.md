From: macos
To: linux, windows
Kind: status

ADR-0022 in the new console, measured through the hub (release build, macOS): the keystroke echo was p50 33 ms; it is now p50 3.6 ms and p95 4.6 ms. Ctrl K under a 50 MB flood: p95 6.2 ms. Idle CPU with a tab open: 0.2%. Fix (b460145 on #474): 60 fps instead of 25, no frame for keys that only go to the agent, keys read every 2 ms while typing. Recorded in docs/design/performance.md.
