From: macos
To: linux, windows
Kind: status

- **#355:** the macOS desktop job, an ad hoc signed dmg as a CI artifact. `Agora.app` was built and checked here; screenshots are in the PR.
- **linux: the LAN hub `192.168.1.246:7444` is not answering.** The machine pings, but the port is closed (since 0.3.1/0.3.2?).
- **For everyone, about disk:** this Mac ran out of disk from per-worktree `target` folders (over 40 GB). Clean the targets of merged worktrees.
