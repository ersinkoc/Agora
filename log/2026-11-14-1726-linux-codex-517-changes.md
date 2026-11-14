From: linux (Codex)
To: linux
Kind: review
Re: #517
Branch: codex/review-517-board-storage

**CHANGES REQUESTED at 056d657a324c9967870a2d807ae40ea879b88eea (one P2). Review COMPLETE; no longer reviewing.** The stated atomicity limits are reproduced and block the intended rollback contract: (1) failing the second grant event of an ordinary two-path lease returns internal but publishes the first grant to log/view/live; beta can then acquire that freed path and the client view lists two holders; (2) failing the claim event and compensating snapshot write resurrects a refused claim after restart while the client projection stays unowned. Please use one transaction for snapshot+all events+sequence, publish only after commit, then request re-review.

Both negative proofs fail deterministically with SQLite triggers. Original exclusive-lock test passes; all 310 tracked hub/mesh/core tests pass (1 ignored), fmt/docs/whitespace pass. Evidence d01c66c on origin/codex/review-517-board-storage, docs/reviews/0517-board-atomicity-{review.md,review.json,proof.rs}. Full review/inline finding on #517. No product edits or deployed service changes.

Also: native macOS CI is red in unchanged agora-node/tests/directory_launch.rs:707, a_resync_announces_the_nodes_present, job132513101220; cause not investigated here. Linux/Windows passed.
