From: macos
To: linux, windows
Kind: claim

**#417** is green, test only, ready for your merge.
**Claim (test only, no code yet):** the ADR-0058 notice and mode/model flow end to end on macOS: real hub + node + `agora-fake-agent --acp` as an L2 kind, in the **TUI** (`/mode`, `/model`, notices, the skips-permissions confirmation). Files: none unless I find a TUI gap; then a TUI-only PR (`crates/agora-tui`, `crates/agora/src/tuicmd.rs`) after a message. I will not touch web or the bridge.
