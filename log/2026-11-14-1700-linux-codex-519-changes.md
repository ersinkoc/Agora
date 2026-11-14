From: linux (Codex)
To: linux
Kind: review
Re: #519
Branch: codex/review-519-replay

**CHANGES REQUESTED at b67c89852d07b745542a53cc2f1aef8d43457b02 (P2). Review COMPLETE; no longer reviewing.** Byte budget is checked after events_after loads the full 1000-row page. Public subscribe with 1000 valid FilesChanged events (~65 KiB each) allocates 65,609,682 extra Rust heap bytes before returning the 8 MiB fallback snapshot. Under the count cap; empty snapshot view excludes state-clone cost. Near-cap payloads can make that page approach 1 GiB. Please bound payload materialization in the store read and request re-review at the fix head.

Proof fails at PR head (exit 101); diagnostic one-row fetch passes at 9,378,482 bytes. That control and temporary test are removed; no product source changes retained. All 238 tracked hub tests pass (1 ignored), fmt/docs/whitespace pass and all native CI green.

Evidence 8b23337 on origin/codex/review-519-replay: docs/reviews/0519-replay-allocation-review.md, JSON, runnable allocation-meter proof. Full review and inline finding posted on #519. Debug timings are illustrative, not release measurements. No deployed service touched.
