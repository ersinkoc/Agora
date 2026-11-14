# PR #519: allocation fix verified

Reviewed head: `60fcc62026e30584adfd38c2f6211cb9c5a9966d`.
Decision: **approve**. The original P2 is verified fixed.

The store now queries the total encoded payload length before copying any replay payloads. The hub count guard runs before that query, limiting the rows considered to the existing 10,000-event budget. SQLite computes BLOB `length()` from record metadata; an over-budget subscription immediately takes the existing snapshot path. The original page-loop guard remains as a defensive check. No protocol or authority change.

The exact preserved original allocation proof passes against this head: **238,448 additional peak Rust heap bytes**, down from **65,609,682** at `b67c898`, for the same 65,296,746 bytes of stored events. The author's promoted regression in `tests/replay_allocation.rs` matches the preserved proof apart from attribution and its test name.

Two additional independent tests pass through the public API: exactly 8 MiB replays, 8 MiB + 1 byte snapshots; suffix-only replay excludes prefix payload bytes; both paths deliver the next live event once; exactly 10,000 events replays and 10,001 snapshots; an at-head resume is empty; a position ahead of the log safely snapshots. All named native CI checks, including Linux/macOS/Windows Rust and the benchmark, passed at the reviewed head.

Run the original and boundary proofs from this worktree:

```sh
cp docs/reviews/0519-replay-allocation-proof.rs crates/agora-hub/tests/review_519.rs
cp docs/reviews/0519-replay-boundary-proof.rs crates/agora-hub/tests/review_519_boundaries.rs
cargo test -p agora-hub --test review_519 -- --nocapture --test-threads=1
cargo test -p agora-hub --test review_519_boundaries
rm crates/agora-hub/tests/review_519.rs crates/agora-hub/tests/review_519_boundaries.rs
```

The temporary tests were removed after these runs; no product source modifications remain. Tests use synthetic protocol-valid events through an in-process negotiated NodeSession. No deployed service or coding agent was changed. The allocator excludes SQLite's C allocations; debug timing is not a release performance claim. The previous review files remain as evidence of the original failure.
