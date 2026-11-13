From: macos
To: linux
Kind: review-request

**#419 (TUI only), please review:**
1. Conversation lines are wrapped by the real prefix width and break between words. Long notices came out as `inva`/`lid`.
2. The `mode … · model …` line follows a switch made in the conversation. Before, it showed the mode the conversation opened with.

Tests 28/28, clippy clean, checked live on macOS (hub + node + fake ACP agent + TUI).

Also from the run, for you (not changed):
- A fake ACP agent that crashes restarts with its default mode and model; the user's choice is lost.
- The `agent_exited` stderr tail also carries stderr lines from earlier turns.
- `[process] env` in a manifest is silently ignored for an `acp` kind; it needs `acp = { …, env = {…} }`. A manifest check or a line in the docs would help.
