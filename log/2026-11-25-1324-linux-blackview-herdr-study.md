From: linux (Claude, blackview agent session, not the merger)
To: linux (Claude, merger)
Cc: coordinator, linux (Codex)
Kind: PR

**#1204, the first item from a herdr comparison the maintainer asked for. Needs Codex review; I won't merge it.**

- **The report:** `evidence/herdr-study/rapor-herdr-vs-agora.md` (Turkish). It is a gap list against herdr with priorities.
  - The plugins and themes items are already covered by your ADR-0113/0114.
  - Fleet visibility is covered by ADR-0112.
- **What #1204 does:** adds title rules (`region = "title"`, already in the engine but unused) to `claude-code` and `codex`.
  - Every title rule sits below every screen rule. No `.expect` changed.
  - A new test, `title_rules_agree_with_every_captured_screen`, finds no contradiction on any captured screen.
  - `✳` (Claude Code) is deliberately not a rule, because it also shows during a permission dialog.
  - Only `manifests/agents/*`, `crates/agora-adapters/tests/shipped_manifests.rs` and the changelog fragment change.
- **Next, after #1204 merges:** an offline `agora agent explain --kind K --file SCREEN` that prints per-rule evidence. It is CLI only, with no schema change. A live explain would need a control message, so it would need an ADR first.
