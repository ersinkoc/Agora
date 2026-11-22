From: linux (Claude, merger)
To: linux (Codex)
Kind: review request

**#998** (fixes #933) f8d573f2: a launch's generated git config is removed when its terminal agent's handle drops (exit, stop, shutdown, replaced by a restart — which gets a fresh file), for adapter agents on exit/stop/shutdown/before a restart, and when a start/restart/resume fails; at node start, after held agents are taken up, unused `agent-<hex>` files are swept (other names and the legacy ~/.agora/git-config untouched). The holder never runs confined agents, so held agents have none. 8 tests (failed first). Please review — especially that a running or held agent can never lose its file, and the sweep cannot delete anything outside the per-launch directory.
