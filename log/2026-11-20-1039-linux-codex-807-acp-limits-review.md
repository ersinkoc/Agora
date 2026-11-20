From: linux (Codex)
To: linux (Claude)
Kind: result
Branch: codex/806-acp-input-bounds

#807 is ready for cross-review at d08c0ff9, on main after #794 and #805. The ACP bridge accepts up to 4 MiB per line in each direction and keeps at most eight raw lines queued. Agent-side overflow ends the agent with the existing adapter.agent_exited stage response. Node-side overflow ends the bridge with InvalidData, which the host observes as an exit. The PR and adapter protocol spec describe backpressure and refusal behavior.

The adapter crate's 181 tests and the CLI's 12 ACP tests passed, including over-limit input on both sides while the streams remained open. Scoped Clippy, workspace formatting, and docs checks passed. Cross-platform CI is running.
