# PR #519: replay byte budget review

Reviewed head: `b67c89852d07b745542a53cc2f1aef8d43457b02`.
Decision: **changes requested** (P2).

## Apply the byte budget before allocating the replay page

`Hub::replay` line 548 calls `events_after(at, PAGE)` with `PAGE = 1000`. SQLite's implementation collects every payload into a `Vec<StoredEvent>` before returning. Lines 551–553 then check the total bytes, after those allocations and reads have already happened under the hub lock.

An owner client resuming below the 10,000-event count limit therefore still reads a complete large page before choosing the snapshot. The 8 MiB limit bounds the returned backlog, but does not bound the memory and storage work that the fallback is intended to prevent. The protocol permits individual payloads up to 1 MiB, so the first 1,000-row page alone can approach 1 GiB. This is an incomplete resource-bound fix, rather than a new authorization bypass.

## Runnable proof

The adjacent `0519-replay-allocation-proof.rs` sends 1,000 valid `FilesChanged` events through a negotiated `NodeSession` and `Hub::ingest`, each about 65 KiB and below the normal frame payload cap. Each batch contains 256 relative paths with legal filename-component lengths. No deployed service or filesystem project is used. The projected snapshot is empty, isolating replay allocation from snapshot size.

A global allocator counts outstanding Rust allocations and resets the peak just before public `hub.subscribe(Some(0))`. Fixture creation and ingestion happen before the measured interval. Run alone:

```sh
cp docs/reviews/0519-replay-allocation-proof.rs crates/agora-hub/tests/review_519.rs
cargo test -p agora-hub --test review_519 -- --nocapture --test-threads=1
rm crates/agora-hub/tests/review_519.rs
```

| Observation | Reviewed head | Diagnostic one-row fetch control |
| --- | ---: | ---: |
| Stored payload | 65,296,746 bytes | same |
| Configured encoded replay budget | 8,388,608 bytes | same |
| Additional peak Rust heap | **65,609,682 bytes** | 9,378,482 bytes |
| Snapshot returned, empty backlog | yes | yes |
| Generous 2× budget allocation assertion | **fails, exit 101** | passes |
| Debug elapsed time | 92.24 ms | 28.06 ms |

The one-row fetch control changes only `events_after(at, PAGE)` to `events_after(at, 1)` in `replay`; it confirms the page allocation is the cause. It is not a proposed production fix (one query per event could cost too much). The control and temporary test were removed before final tracked tests. No product source changes remain.

Suggested direction: bound payload materialization in the store read, for example a byte-aware page/read iterator that checks lengths before copying payloads, or a bounded metadata preflight. Allow one individual record of overhead and keep snapshot/live registration atomic. Add a regression with a whole large page; the existing tiny-byte-budget test only checks which result is returned.

Global allocator counts exclude SQLite's C heap. Debug timings are illustrative, not release performance claims. Existing tracked test and CI results are recorded separately in the review comment; their success does not check this allocation bound.
