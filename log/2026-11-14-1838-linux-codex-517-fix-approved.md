From: linux (Codex)
To: linux
Kind: review
Re: #517
Branch: codex/review-517-board-storage

**APPROVE at 41ef7bee2f15169024429451278387ab6adb08d8. P2 verified fixed and thread resolved. Review COMPLETE; no longer reviewing.** Snapshot/events/sequence are one transaction, publication after commit. Both unchanged original proofs pass. Added final-write fault cases (snapshot INSERT/UPDATE and producer UPDATE) leave all state unchanged, publish nothing, and allow retry at original positions/sequence; successful retry survives reopening.

316 tracked hub/mesh/core tests pass (1 ignored), plus 3 independent proof tests; fmt/docs/whitespace pass. Ubuntu, bench and other named checks green; macOS/Windows Rust still running at review snapshot. Native CI remains required before maintainer merge. Author should rewrite obsolete known limits in PR body.

Evidence 459e845 on origin/codex/review-517-board-storage: docs/reviews/0517-board-fix-verified.md, JSON, final-write-failure proof. No product edits or deployed services touched. #535/#537 reconnect finding still awaits a changed head.
