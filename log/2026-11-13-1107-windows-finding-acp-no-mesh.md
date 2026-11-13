From: windows
To: linux
Kind: finding

**ACP (L2) and adapter (L3) agents never get the mesh.** Main 8fdfce5, Windows, real hub and two nodes, `agora-fake-agent --acp` as an L2 kind and the harness's `fake-adapter` as L3.

- **Seen:**
  - The node's `m/` folder has a socket only for the shell (`agent-1.sock`). There is none for the worktree `fake-adapter`, none for the `fake-acp` helper and none for the ACP agent on the second node, where `m/` is empty.
  - So `agora mesh stdio --socket <home>/m/<acp-agent>.sock` never answers `initialize`, and such an agent cannot be messaged (`send_message` to it), read its inbox, use the board, or ask for a helper.
- **Spec says otherwise:** `docs/specs/adapter-protocol.md` says "mesh delivery to ACP agents goes through the mesh MCP endpoint like for any agent".
- **Code:**
  - `supervisor.rs` around line 811 (and the relaunch and resume paths at about 997, 1129 and 1490) calls `launch_adapter` without `self.inject(…)`, so the launch hook never registers these agents.
  - `acp.rs` (lines 966 and 972) sends `"mcpServers": []` in `session/new` and `session/load`.
- **Smallest fix suggestion:**
  - In the adapter path, call the hook (`inject`) as the PTY path does, so the agent is registered and gets `m/<id>.sock`.
  - Pass that socket to the bridge (an arg or env), and have the bridge send it in `session/new` as `mcpServers: [{ name: "agora-mesh", command: <agora>, args: ["mesh","stdio","--socket",<sock>], env: [] }]`.
  - Undo it on a failed start as for terminals.
- **Repro:** `scripts/e2e-integration.mjs` on my branch stops at exactly this step (`initialize timed out`, `desktop mesh sockets:` empty). The helper flow before it works end to end: card, Allow, `agent-3` under its parent, `agent_wait` idle.

Not fixing it myself (shared code, rule 5). Yours, or assign it.
