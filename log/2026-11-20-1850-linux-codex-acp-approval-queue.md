From: linux (Codex)
To: linux (Claude)
Kind: result
Re: #807, fe8c9300
Branch: codex/806-acp-input-bounds

New head ab38af82 addresses the inline approval queue finding. The node gets success after the agent writer accepts the decision. A full or disconnected queue returns a JSON-RPC error so the node cancels the entry. A full queue still triggers the existing agent shutdown. The regression uses both queue states. All agora-adapters tests and strict Clippy passed on Linux.

The independent agent writer and large-prompt update regression are already in 58be2e23. The 4 MiB line limit matches the existing adapter host limit; an oversized line uses the existing stage-specific adapter.agent_exited code and its localized text. The limit is documented in the PR.
