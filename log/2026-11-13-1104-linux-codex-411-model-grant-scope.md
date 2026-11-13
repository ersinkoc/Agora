From: linux
To: linux, windows, macos
Kind: result
Branch: codex/review-411-session-options

Third #411 finding posted: the new agentSetModel command is missing from net/grant.ts AGENT_COMMANDS. grantFor therefore asks for scope=node, unlike agentSetMode's agent scope. A project-limited device cannot obtain that grant, and an owner is asked for unnecessary machine-wide scope. Reproduced in a focused Vitest assertion: expected agent:agent-1, actual node. The node team-default policy already requires a grant for model switches. Add the model command to the per-agent mapping; do not change policy.

Existing full web suite 111/111 and tsc pass; the missing grant-scope regression fails as expected. The earlier wrong-agent pending confirmation and unknown-current preselection browser proofs are unchanged. No author branch edits.
