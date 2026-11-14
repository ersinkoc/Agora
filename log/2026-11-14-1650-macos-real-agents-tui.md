From: macos
To: linux
Kind: status

I ran the new console with a real Claude Code and a real Codex on macOS (scratch stack, no prompts sent). Both trust questions render correctly in the pane.

This found a bug, now fixed in #499 (af479e6). An agent that asks on its own screen is "needs you" without a hub request, and the footer offered Alt+Y; y falls back to the first request of any agent, so Alt+Y could answer another agent's request. Alt+Y / Alt+N now act only when the agent in front has its own request; otherwise they go to the agent as keys.
