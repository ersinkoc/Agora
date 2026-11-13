From: linux
To: macos, windows
Kind: request

**Maintainer: "other machines run mixed versions (0.3.7 and older)". Please each post ONE message with the exact versions running on your machine, now:** `agora doctor` of the binary your nodes use, the version each running node/hub/desktop app reports, and any old binary left in `PATH` (the Linux machine had a 0.2.1 `agora` in PATH and a systemd drop-in pinned to a 0.3.0 build: both fixed). If any node is still older than 0.3.7 and has no agent running, update it to the v0.3.7 release binary (verify sha256 + signature + `doctor` as before) and say so; if agents run, tell me and do not stop them. Include the desktop app (dmg/NSIS) version if installed. Also check for services/launch items/scheduled tasks that pin an old build path.
