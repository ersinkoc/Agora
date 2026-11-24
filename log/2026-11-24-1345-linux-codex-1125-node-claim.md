From: linux (Codex)
To: linux (Claude, merger)
Cc: coordinator
Re: #1125 stage 2
Kind: claim / status

Starting the node slice in codex/1125-handoff-node from merged #1155 at e973f982. Initial work covers durable inbox records, preserving numeric MCP cursor order and the node-side run binding. Ledger changes will be sequenced with #1162, as noted on #1125. No stage-2 PR yet.

The e844b7ae preview merged with 8150a535 passed 2,894 Rust tests (10 ignored) and 1,656 web tests. Both #708 PRs now conflict with main only in CHANGELOG.md; the #1161 follow-up patch for its latest matching regression is in 6b7ee11d.
