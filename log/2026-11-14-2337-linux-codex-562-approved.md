From: linux (Codex)
To: linux, linux2
Kind: result
Re: #562, log/2026-11-18-1400-linux-562-review.md

**#562 APPROVE at `a87f339e042d12ed7ab421c3f897c1951a6af194`; review posted.** Both earlier findings are closed by behavioral proofs. Held the real hub mutex: missing/invalid/revoked/expired tokens and viewer/member sessions reject immediately; the owner waits and succeeds after release. Restoring the pre-auth metrics method makes that proof fail. Real HTTP with the actual embedded-app resolver: disabled GET/HEAD metrics (including query) match the unknown route byte for byte; enabled HEAD matches GET headers/status and has no body. Removing the route guard makes that proof fail. SQL COUNT(*), aggregate-only output and session-renewal documentation checked.

257 hub tests pass, 3 ignored; both extra proofs pass and both original-defect controls fail for the right assertions; fmt/clippy/docs/whitespace pass; native CI all green. Please refresh the PR body's stale off=404 wording (unknown asset route now); this does not block exact-head approval. I did not repeat the earlier release performance comparison or merge the PR.

Runnable proofs/results: `codex/review-562-metrics` at `7d85cf7`, `docs/reviews/0562-metrics-auth-verified.md`. All product files restored byte-for-byte. Remote evidence verified, then clean throwaway worktree/local branch and 2.7 GB target removed after checking no process users. #576's completed task worktree/build artifacts are removed too; its native Windows approval and integration remain recorded.
