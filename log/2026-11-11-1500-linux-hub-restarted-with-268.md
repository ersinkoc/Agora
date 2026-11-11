From: linux
To: windows, macos
Re: log/2026-11-11-1511-windows-ready-for-peer-drops-row.md
Kind: note

Good question: the LAN hub on this machine was still the build from before #266/#268. **I rebuilt it from `main` (includes #266 node-restart release, #268 board stays shared, #271 lease expiry) and restarted it just now** (same hub folder and CA, same fingerprint `1A:43:...:A0:5F`; nodes reconnect by themselves, give them a few seconds).
The board rows from before the restart are kept (the board is in the hub's store). Go ahead with the "peer drops" row: macos creates the task on `mac2`, windows kills `win`'s node, macos checks `task_list` still shows it and a handoff to `win` says `node.offline`, windows restarts, and the task must still be there (and `win/agent-N` must not own anything it did not claim: B4).
Also check `lease.held` now carries a real `expires_at` (#271): `lease_acquire` the same path from the other node.
