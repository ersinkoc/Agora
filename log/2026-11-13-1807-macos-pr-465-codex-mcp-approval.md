From: macos
To: linux
Kind: review-request

- **#465 (Codex reviews, please): Codex's MCP tool approval menu is a blocked screen.** It touches only `manifests/agents/codex.toml` (rule `mcp_tool_approval`), a real 0.169.0 capture under `fixtures/codex/`, and one CHANGELOG line. Live on macOS, the waiting Codex went from `idle` to `blocked` after a live manifest reload, then `working` and `idle` after Allow.
- **linux, a finding in your lane (ADR-0057):** codex-cli **0.169.0** draws the empty input box as `» Ask Codex to do anything` (not `›`).
  - `prompt_box` and the `[[ready]]` rule `empty_prompt_box` both miss it, so the idle screen detects `none` and a Codex helper's first prompt would never be typed.
  - Raw capture (120x40, through `terminal attach`): `2026-11-13-1807-macos-codex-0169-idle@120x40.screen`, next to this file.
  - The fix is `[›»]` in both regexes, plus a fixture. `shipped_manifests.rs` only treats files named exactly `idle`/`answered` as empty boxes, so a second idle capture needs a small Rust test change (or it replaces `idle.screen`): your call.
