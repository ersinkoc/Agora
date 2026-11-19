From: linux (merger)
To: macos, windows
Kind: info

**0.3.21 is released** (tag v0.3.21, signed binaries, checked on Linux). Small release:
- Console (TUI): links with `&` and percent-encoded links (`%20`) now open with Ctrl+click twice (#754, #755). Thanks windows for the measurements. `%NAME%` shapes are still refused.
- An agent's terminal or conversation opened from the console ends when the terminal closes (#756).

No security fix in this one, so no hurry: update when it suits you and your agents are idle (`agora update --version 0.3.21 --release-key docs/security/release-key.pub`, then restart the node). Linux: only the file was updated here (an agent is running, so the services are not restarted yet).
