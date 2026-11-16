# PR #517: atomic board fix verified

Reviewed head: `41ef7bee2f15169024429451278387ab6adb08d8` (includes transaction fix and merge of main).
Decision: **approve**. The original P2 is verified fixed; native CI remains a merge requirement.

`SqliteStore::append_batch_with_setting` now writes every board event, the producer sequence and the changed board snapshot in one SQLite transaction. `Hub::board_call` updates its sequence, projection and subscribers only after the transaction returns success. A failed transaction restores the in-memory checkpoint and produces no handoff/fanout. The compensating database write is gone.

The two unchanged original proofs now pass. Failing the second lease event leaves zero projected/live grants and does not advance the log. A subsequent beta claim shows only beta as holder. Failing the task event with the original compensation-failure trigger installed leaves the task unowned in memory, storage and after reopening; the client projection agrees. The promoted tracked tests preserve these original assertions.

An additional proof injects faults in the first snapshot INSERT, an existing snapshot UPDATE, and the producer sequence UPDATE after event rows have been inserted. All three cases roll back event rows, snapshot bytes, sequence and projection together, publish nothing, and restore the authoritative broker. Once the fault is removed, the same two-path acquire succeeds using exactly the original next sequence/positions, delivers two grants once and survives a hub reopen.

Reproduce using the retained proofs:

```sh
cp docs/reviews/0517-board-atomicity-proof.rs crates/agora-hub/tests/review_517.rs
cp docs/reviews/0517-final-write-failure-proof.rs crates/agora-hub/tests/review_517_final_writes.rs
cargo test -p agora-hub --test review_517 -- --nocapture
cargo test -p agora-hub --test review_517_final_writes -- --nocapture
rm crates/agora-hub/tests/review_517.rs crates/agora-hub/tests/review_517_final_writes.rs
```

The original review files remain as historical evidence of the failure. Temporary tests were removed before final tracked checks; no product edits remain. Native CI and tracked-test results are recorded in the adjacent JSON and review comment. The PR description's old partial-write/failed-compensation limits no longer describe the implementation and should be updated by its author.

Tests use isolated SQLite files and synthetic shared-project agents; no deployed service or real coding agent was touched. This review covers the board_call transaction fix; it does not claim to audit other pre-existing board lifecycle write paths.
