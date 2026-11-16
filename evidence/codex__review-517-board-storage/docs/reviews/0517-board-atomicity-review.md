# PR #517: board storage review

Reviewed head: `056d657a324c9967870a2d807ae40ea879b88eea`.
Decision: **changes requested** (one P2: commit the board snapshot and all audit events atomically before publishing).

The existing exclusive-writer-lock regression passes. Two selective-failure proofs show that the stated limits in the PR description violate the intended rollback contract and Accepted ADR-0039/0040: the authoritative broker, the event projection and the stored board still disagree after a refused operation. Both proof tests fail, exit 101.

## Partial lease publication

The first proof refuses only the second event INSERT of `lease_acquire` for `src/a.rs` and `src/b.rs`. The call returns `internal` (`incident_id = board.store`), and the board rolls back to zero leases. However, the first grant is already committed to the log, applied to the projection and delivered to a live subscriber. The log position advances by one. A subsequent successful acquire by `beta/a` grants `src/a.rs` because the board correctly considers it free; the client projection now contains **both alpha/a and beta/a holding that path**.

No failed compensation is needed in this case. Each path creates its own event, so this is an ordinary multi-path board call, rather than a hypothetical multi-event tool. A handoff also produces several events.

## Failed compensation resurrects a refused claim

The second proof allows the changed board snapshot write, then refuses the task-claim event INSERT and the compensating settings UPDATE restoring the unowned snapshot. The caller gets `internal`, and the live board says the task is unowned. SQLite still contains `owner = beta/a`. After dropping the faults and reopening the hub, the refused claim comes back as owned by beta/a. The client projection, rebuilt from the log without the claim event, still says unowned.

The two consecutive failures are injected deterministically; persistent storage failures are not independent rare events. Compensating writes cannot provide a durable rollback contract when storage is already failing.

## Reproduce

The adjacent proof source reuses the PR's synthetic two-node fixture and adds SQLite triggers. Everything runs against temporary databases; no real agent or deployed service is touched.

```sh
cp docs/reviews/0517-board-atomicity-proof.rs crates/agora-hub/tests/review_517.rs
cargo test -p agora-hub --test review_517 -- --nocapture
rm crates/agora-hub/tests/review_517.rs
```

Fix direction: use one SQLite transaction for the changed board snapshot, all event INSERTs and producer sequence advancement. Publish to the view/subscribers only after that transaction commits. Roll back the in-memory checkpoint on failure, and return no handoff/fanout effects. Add selective first/later event-failure tests alongside the exclusive-lock test. This uses the existing protocol and ownership model.

The temporary proof test was removed; no product modifications remain. Structured observations and final tracked-test results are in the accompanying JSON. Native Linux and Windows CI passed; macOS failed in `agora-node/tests/directory_launch.rs:707`, `a_resync_announces_the_nodes_present` (job 132513101220). Its cause was not investigated in this board review, and that node test is unchanged by the PR.
