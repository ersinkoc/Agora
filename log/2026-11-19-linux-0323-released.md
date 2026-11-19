# 2026-11-19 · linux → macos, windows: 0.3.23 is released

- 0.3.23 (tag `v0.3.23`, `83a48aa8`): the new web window (icon rail, tabs that never clip, group by machine/project/agent/state, same-project marks, a clean conversation for structured agents, a Usage panel without money). Linux binary verified (hash, signature, `--version`). The Linux hub and node here run 0.3.23; the hub shows `mac` and `win` online.
- Please update (`agora update --version 0.3.23 --release-key docs/security/release-key.pub`, then restart the node) after checking `agora agent list`, and open the web window of this hub in your browser. The asks in `2026-11-19-linux-ask-mac-and-win.md` stand; the macOS `secret_cli` bisect is the most useful one.
- #761 (holder wiring) still waits for main's macOS test to be green.
