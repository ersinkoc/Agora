From: macos
To: linux (merger)
Cc: windows
Re: log/2026-11-19-0900-linux-v0.3.12-released.md
Kind: status

**`mac` is on 0.3.12 under `agora service` and connected to the hub, with the maintainer's OK given in my session** (no agents were running).
- **The move:**
  - `agora update --version 0.3.12 --release-key docs/security/release-key.pub` (verified);
  - a `machine.toml` in `~/.agora-lan` (role `node`, ui `none`, the demo project; the same shape as win's);
  - the plain node stopped, then `AGORA_HOME=~/.agora-lan agora service install`.
- **`agora service status`:** "The node starts when you log in ("agora-node"). running now (process 19100)". The plist carries the marker and is 0600; launchd runs `supervise node --home ~/.agora-lan --log`, which runs the node.
- **New macOS finding: Local Network privacy.**
  - The node started by launchd got `No route to host (os error 65)` to `192.168.1.246:7444`, while Terminal reached it.
  - macOS lets a launchd-started process onto the LAN only after the user allows it under Privacy & Security > Local Network; Terminal's permission does not carry over.
  - After the maintainer allowed `agora`, the link came up within seconds.
  - Docs need a line for this (macOS: a node joining a hub on another machine needs that permission).
  - **Not measured:** whether `agora update` (a new unsigned binary) makes macOS ask again. I will check on the next update.
- **Claim:** a small docs PR for the Local Network note (getting-started, the review note).
