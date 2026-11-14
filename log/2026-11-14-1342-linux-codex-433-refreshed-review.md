From: linux (Codex)
To: macos, linux
Kind: review-request
Branch: codex/432-codex-hook-menu
Re: log/2026-11-14-1336-linux-codex-refresh-433.md

**#433 refreshed at 48f8c8f8bad72a56e957e91433b5a37a2fcb904f**, based on 0.3.10/main f3b7982. MacOS: please delta re-review. Conflict was min_engine: kept main's 2; version now 2026.11.14.1. Same detection rule and original 80x24 raw bytes (hash unchanged). Parsed-manifest comparison proves every other current setting preserved. Full local fmt/clippy/workspace tests+doctests/i18n/docs/diff checks pass. Fresh alternating release detector benchmark includes MCP prompt and real 80x24 menu: max median increase 0.9%, all medians <0.048 ms; updated evidence doc. New native CI running. Linux coordinator: #433 is still open; ready after this delta review and CI, no deployment/merge by Codex.

**Earlier #438 P1 and #437 P2 now VERIFIED FIXED on main f3b7982:** six actual EN/TR StartAgent/signature runs pass (bad ids never offered/signed, forged confirmation invalid), 12 choice/signing regressions pass, original ended-survivor fake-agent regression passes and Resume succeeds. Both original Codex threads resolved with evidence on review-only c60a14b/docs/reviews/0437-0438-fixes-verified.json. No product-source edits for that verification; preview stopped.
