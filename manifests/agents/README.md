# Agent manifests

One TOML file per agent kind (L1 integration). Format: `docs/specs/agent-manifest.md`.
Each manifest ships with fixture screens under `manifests/agents/fixtures/<kind>/`.
Every manifest here is compiled into the `agora` binary as a built-in kind (`crates/agora-node/src/manifests.rs`; add a new file
to `BUILT_IN` there, a test fails until you do). A file in the node's manifests folder with the same `kind` replaces it
(`docs/specs/agent-manifest.md`, "Where agent kinds come from").

Shipped: `antigravity-acp` (`agy acp`, unverified against the real program yet), `opencode-acp` (opencode as an ACP server; unverified against the real program yet), `claude-code` (screens captured from Claude Code 2.1.319), `codex` (codex-cli 0.167.0) `wrongstack-acp` (the same agent as an ACP server, L2: no terminal, so no screens) and `wrongstack` (WrongStack 1.5.3; fixtures are the rows of the 120x40 screen as plain text, replayed from a raw capture, because its animated side panels made the raw streams megabytes long). Fixtures are raw terminal output (`<name>.screen`, captured on a 120x40 terminal) with the expected result in `<name>.expect` (`<state> <rule id>` or `none`); `crates/agora-adapters/tests/shipped_manifests.rs` checks every manifest against its screens.

Launch options (ADR-0059, `[[launch.option]]`, `min_engine = 2`): only flags checked on a real version are declared, each with the
version in a comment. `claude-code`: `--dangerously-skip-permissions` (Claude Code 2.1.323); `codex`:
`--dangerously-bypass-approvals-and-sandbox` (codex-cli 0.167.1); `wrongstack`: `--yolo` (WrongStack 1.6.0; it still asks for the
destructive kinds it gates); `antigravity-acp`: `--dangerously-skip-permissions` on the ACP server command (agy 1.5.6, seen on
Windows, not run where the manifest was written). `wrongstack-acp` and `opencode-acp` declare none: no such flag was found on a real
version. Each is a "does not ask" option (`skips_permissions = true`): never preselected, confirmed in the start window, signed by
the device that starts the agent, audited, shown as a badge, and allowed only as the node's `skip_permissions` policy says.
