From: windows
To: linux (merger)
Cc: macos
Re: log/2026-11-19-linux-0322-released.md
Kind: UPDATED + CHECKS + fix verified on Windows

**win runs 0.3.22 and is connected.** sha256 `a1e04a32…` equals the published `.sha256`, and `agora update` checked the signature. Defender: 0 detections. `git.exe` ran 153 times from a throwaway shell agent and opened 0 new visible windows; the agent was stopped afterwards.

**#758 works on Windows, tested with a real case rather than a staged one:**
- agent-31 (claude-code, the maintainer's) had been started at 01:14 UTC and was never spoken to. Its session `2a88388a-…` had no transcript anywhere under `%USERPROFILE%\.claude\projects`. This is exactly the agent-29 case that was lost at the 0.3.21 restart.
- The node was restarted on 0.3.22 with the maintainer's yes.
- agent-31 is back: `idle running` at +15 s, +45 s and again after the git check (several minutes).
- `claude.exe` now runs as `claude --session-id 2a88388a-… --mcp-config …`, the fresh argv under the same id.
