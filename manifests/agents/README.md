# Agent manifests

One TOML file per agent kind (L1 integration). Format: `docs/specs/agent-manifest.md`.
Each manifest ships with fixture screens under `manifests/agents/fixtures/<kind>/`.
Every manifest here is compiled into the `agora` binary as a built-in kind (`crates/agora-node/src/manifests.rs`; add a new file
to `BUILT_IN` there, a test fails until you do). A file in the node's manifests folder with the same `kind` replaces it
(`docs/specs/agent-manifest.md`, "Where agent kinds come from").

Shipped: `antigravity-acp` (`agy acp`, unverified against the real program yet), `opencode-acp` (opencode as an ACP server; unverified against the real program yet), `claude-code` (screens captured from Claude Code 2.1.319), `codex` (codex-cli 0.167.0) `wrongstack-acp` (the same agent as an ACP server, L2: no terminal, so no screens) and `wrongstack` (WrongStack 1.5.3; fixtures are the rows of the 120x40 screen as plain text, replayed from a raw capture, because its animated side panels made the raw streams megabytes long). Fixtures are raw terminal output (`<name>.screen`, captured on a 120x40 terminal) with the expected result in `<name>.expect` (`<state> <rule id>` or `none`); `crates/agora-adapters/tests/shipped_manifests.rs` checks every manifest against its screens.
`claude-code` and `codex` also have title rules (`region = "title"`, the OSC 0/2 window title the captures carry): below the dialog
rules and `esc_to_interrupt`, above the idle `prompt_box` rule (whose marker stays on screen during turns and dialogs); the same test
checks that they agree with every captured screen whose title they match. A title that is also shown during a dialog (Claude Code's `✳`) is never a rule.

## The ACP registry (`registry/`, ADR-0092)
`registry/` holds the agents of the official ACP registry (`github.com/agentclientprotocol/registry`), converted by
`cargo run -p agora-adapters --example registry_convert -- <registry clone> manifests/agents/registry <commit> <date>` and reviewed
in a pull request. Nothing in it is written by hand and nodes never read the registry. Per agent: `<id>.toml` (an L2 manifest of
kind `registry-<id>`, `[structured] acp`, no launch options), `<id>.package.json` (the unsigned ADR-0087 draft a reviewer turns into
signed-list entries, `docs/specs/agent-packages.md`) and `icons/<id>.svg` (only when the icon passed `check_icon`: a plain one-colour
drawing of at most 16 KiB). `catalog.json` is what the window's "Add an agent" list shows, `SKIPPED` names each entry left out with
its reason code, and `SOURCE` the registry commit and date. Every `.toml` is compiled in (`REGISTRY` in
`crates/agora-node/src/manifests.rs`; a test fails until the list matches the folder). These kinds are **untested**: nobody has run
them through Agora. A hand-made kind of the same `kind`, or one named by the registry id (`antigravity-acp` for
`registry-antigravity-acp`), wins and the registry one is not offered. At registry commit `6f2077c` (2026-11-23): 46 entries, 41
converted; left out: `agoragentic-acp`, `deepagents`, `minion-code` and `vtcode` (`registry.quarantined`: the registry holds them back
itself) and `antigravity-acp` (`registry.inconsistent_command`: its arguments differ per platform; the hand-made `antigravity-acp`
covers it). `registry-opencode` runs the same `opencode acp` as the hand-made `opencode-acp`; both are listed. The maintainer's
answers (#1133): quarantined entries stay skipped (the conversion refuses a clone without `quarantine.json`); `poolside` keeps its registry id as its program name; the npx and uvx kinds start
only on a start a person's device signed; a node's policy line `agents.registry = off` (or `agents.registry.deny = ...`) switches the
registry kinds off there (`docs/specs/protocol.md`, "ACP registry kinds").

Launch options (ADR-0059, `[[launch.option]]`, `min_engine = 2`): only flags checked on a real version are declared, each with the
version in a comment. `claude-code`: `--dangerously-skip-permissions` (Claude Code 2.1.323); `codex`:
`--dangerously-bypass-approvals-and-sandbox` (codex-cli 0.167.1); `wrongstack`: `--yolo` (WrongStack 1.6.0; it still asks for the
destructive kinds it gates); `antigravity-acp`: `--dangerously-skip-permissions` on the ACP server command (agy 1.5.6, seen on
Windows, not run where the manifest was written). `wrongstack-acp` and `opencode-acp` declare none: no such flag was found on a real
version. Each is a "does not ask" option (`skips_permissions = true`): never preselected, confirmed in the start window, signed by
the device that starts the agent, audited, shown as a badge, and allowed only as the node's `skip_permissions` policy says.
