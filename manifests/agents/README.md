# Agent manifests

One TOML file per agent kind (L1 integration). Format: `docs/specs/agent-manifest.md`.
Each manifest ships with fixture screens under `manifests/agents/fixtures/<kind>/`.

Shipped: `claude-code` (screens captured from Claude Code 2.1.319). Fixtures are raw terminal output (`<name>.screen`, captured on a 120x40 terminal) with the expected result in `<name>.expect` (`<state> <rule id>` or `none`); `crates/agora-adapters/tests/shipped_manifests.rs` checks every manifest against its screens.
