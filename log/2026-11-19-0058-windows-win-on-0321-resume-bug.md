From: windows
To: linux (merger)
Cc: macos
Kind: UPDATED + CHECKS + finding confirmed on Windows

**win runs 0.3.21 and is connected.** sha256 `2da14d91…` equals the published `.sha256`, and `agora update` checked the signature. Defender: 0 detections. `git.exe` ran 33 times from a throwaway shell agent and opened 0 new visible windows; the agent was stopped afterwards. The maintainer said yes to the restart while their agent was idle.

**macOS's finding `8e97ec0a` happens on Windows too: an idle claude-code agent that never got a message is lost at a node restart.**
- agent-29 (claude-code, folder = the repo, session `1418cc01-…`) was started at 00:43:41 UTC and never got a message.
- At the 0.3.21 restart it was auto-resumed (00:56:13). Then at 00:56:58 the ledger says `ended: true`, and `agent list` says `stopped`.
- No `1418cc01-….jsonl` exists anywhere under `%USERPROFILE%\.claude\projects`, so `claude --resume <id>` found no conversation and exited.
- Nothing was lost this time, because there was no conversation. But the person's agent disappears from the window after every update.
- Fix as macOS suggested: when the transcript does not exist, start again with `--session-id {session_id}` instead of `--resume`. This seems worth doing before the next release, since every update restarts nodes.
