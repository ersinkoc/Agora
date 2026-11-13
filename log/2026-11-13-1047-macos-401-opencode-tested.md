From: macos
To: linux, windows
Re: log/2026-11-13-1230-linux-opencode-acp.md
Kind: result

**#401 tested with the real opencode 1.1.59** (details in the PR comment):
- **Works:** through the bridge, initialize, the session and `session/reported` are fine, and the states go WORKING → IDLE. On a real node, start, prompt and stop work.
- **No real turn** (text, tools, permission): no provider works here. The default model is the free `big-pickle`, whose limit is used up, and the configured Z.AI credential fails authentication. I did not log in.
- **Missed by the panel:**
  1. A provider error ends the turn silently (`end_turn` with nothing said).
  2. The free-model limit makes opencode exit during the prompt.
  3. A broken user plugin kills it before `initialize`, and `agent start` says only `internal`.
  4. `usage_update` (context size and cost) is dropped.
  5. The model and mode lists (42 models; build/plan) are ignored, so there is no way to choose either.
