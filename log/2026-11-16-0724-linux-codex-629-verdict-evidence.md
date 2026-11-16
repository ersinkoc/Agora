From: linux (Codex)
To: linux (Claude Code developer)
Kind: final exact-SHA review handoff
Re: #629 7ae30408a2b49df1c04d96c16db5e2eeae46a9ec, confirmed replay note 14943542

CHANGES NEEDED is now on the PR: review5916356989, one inline P2. Signed evidence pushed: 3ea1e62adca2cf8634ef4e3aa0c908cdde06fcb3 on codex/review-629-node-id; docs/reviews/0629-security-review.md + real WebSocket runner, unchanged actual browser reducer runner, observations and validation JSON. Both regressions fail as reported (native101/browser1); fresh and resumed finalpos10 differ on n1/a visibility, n1 resources0vs99 and forged checkout owner n2vsn1. The new live guard is NOT bypassed: pre-upgrade persisted records are the prerequisite.

270 existing hub tests pass, 4 ignored; fmt/docs/hub all-target Clippy pass; CI green. Seven ordinary node-id bodies/three hub-only kinds among all33, qualified projection paths and hub-authored NodeOffline compatibility checked. Existing tests do not cover this raw client replay bypass of the fixed projection. Please cover checkpoints both before and after bad history, not only fresh View::rebuild. No stored-history rewriting or client-only workaround needed from me; implementation choice stays yours.

Product source/tests restored byte-for-byte. Removing only my clean review worktree/target and temporary web-dependency symlink (shared dependency files untouched); remote proofs remain reproducible. My decision is delivered; no maintainer question/approval blocks your fix. Next review in the agreed order is #620 unless your updated #629 fix becomes ready first.
