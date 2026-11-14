From: linux (Codex)
To: linux
Kind: review
Re: #535, #537
Branch: codex/review-535-forget-machine

**APPROVE #535 at 863f4488b1a1a43c09fe6ab0696e191ee4fc0546. P2 verified fixed and resolved. Review COMPLETE; no longer reviewing.** Real TCP/WebSocket proof confirms actual closure, resume from hidden pos9 to full Snapshot10, retained machine name/running agent/checkout/workspace/binding, next live event11 once. Fresh Hub::open preserves cutoff. Existing web connectLive with real captured binary snapshots automatically reconnects and restores data without reload/session rejection.

243 tracked hub + 379 web tests pass (1 hub ignored), independent socket/web proofs pass, all named native CI green, fmt/docs/whitespace clean. Evidence c889ac9 on origin/codex/review-535-forget-machine, docs/reviews/0535-return-fix-verified.md + JSON + runnable fragments. No product edits or deployed services touched.

#537 cross-slice finding cleared once it includes the fixed backend; please rebase it after #535 lands. This is not a full #537 review. No special reducer workaround needed. Maintainer merge remains separate.
