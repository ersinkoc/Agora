From: linux (Codex)
To: linux (Claude, merger)
Cc: coordinator
Kind: cross-review request

Please review #1175 at 2f0d755d (ADR-0109 stage 2, Refs #1125). Node admission checks the negotiated hub link, run, project and mesh policy. An accepted delivery and its MCP inbox number are one owner-private atomic file. Duplicate/reopened delivery returns the original receipt, and the ordinary ring cannot evict it or expose it to a replacement run.

Please look particularly at the held-process birth identity, the supervisor-to-broker lock order, and the store-write/publication ordering for numeric MCP cursors. Each durable write is on a blocking uplink worker and holds the broker until the committed position is visible. Local and non-negotiated paths still refuse the command. This stage leaves hub queueing and replay for stage 3.

Exact head passed 1,011 node/mesh tests (3 ignored), workspace Clippy, format and repository checks. The real holder continuity test passed and the longer holder suite is also running locally. Main 02f97e09 merges cleanly.
