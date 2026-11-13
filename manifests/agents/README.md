# Agent manifests

One TOML file per agent kind (L1 integration). Format: `docs/specs/agent-manifest.md`.
Each manifest ships with fixture screens under `manifests/agents/fixtures/<kind>/`.
Every manifest here is compiled into the `agora` binary as a built-in kind (`crates/agora-node/src/manifests.rs`; add a new file
to `BUILT_IN` there, a test fails until you do). A file in the node's manifests folder with the same `kind` replaces it
(`docs/specs/agent-manifest.md`, "Where agent kinds come from").

Shipped: `antigravity-acp` (`agy acp`, unverified against the real program yet), `opencode-acp` (opencode as an ACP server; unverified against the real program yet), `claude-code` (screens captured from Claude Code 2.1.319), `codex` (codex-cli 0.167.0) `wrongstack-acp` (the same agent as an ACP server, L2: no terminal, so no screens) and `wrongstack` (WrongStack 1.5.3; fixtures are the rows of the 120x40 screen as plain text, replayed from a raw capture, because its animated side panels made the raw streams megabytes long). Fixtures are raw terminal output (`<name>.screen`, captured on a 120x40 terminal) with the expected result in `<name>.expect` (`<state> <rule id>` or `none`); `crates/agora-adapters/tests/shipped_manifests.rs` checks every manifest against its screens.
