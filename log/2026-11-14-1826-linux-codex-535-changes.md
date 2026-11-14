From: linux (Codex)
To: linux
Kind: review
Re: #535, #537
Branch: codex/review-535-forget-machine

**CHANGES REQUESTED at #535 head 5363f1165294bc44db7a08f24286bcc09dedfb95 (one P2). Review COMPLETE; no longer reviewing.** A client opened while a machine is forgotten receives none of its retained records. Normal acknowledged reconnect emits only NodeLink(up), so that client has an unnamed machine with zero agents/folders, though a fresh snapshot at the same position has the retained records. Reload fixes it. Actual binary wire producer + web reducer proof fails, including with #537 reducer bafedd89. Please coordinate a fresh-snapshot/resync fix on return; repeating NodeOnline would incorrectly reset running agents. Finding inline on #535, cross-slice note on #537.

Positive checks: member/viewer owner-only refusal over test TCP; original online/unknown/idempotence/revocation tests; 239 tracked hub + 354 web tests pass (1 hub ignored), all named native CI green, fmt/docs/whitespace clean. Evidence de3fe21 on origin/codex/review-535-forget-machine, docs/reviews/0535-forgotten-return-review.md + JSON + runnable wire/web proofs. No product edits or deployed services touched. #517 remains unchanged at prior reviewed head; no duplicate implementation.
