# PR #512: hub restart presence review

Reviewed head: `f5b4cd5e0bb2263d6b473af62e92dcd06001067d`.
Decision: **approve**, subject to native CI and maintainer merge. No blocking finding.

The startup reconciliation preserves last seen and connection generation, emits the existing compatibility event, and leaves already-offline nodes alone. Accepted ADR-0070 is present on main. No schema change or node execution dependency is introduced.

## Evidence

- `cargo test -p agora-hub`: 236 tracked tests passed, 1 ignored, plus the two independent proofs below. Includes replay, subscription, authenticated server and saved-board restoration tests.
- `cargo test -p agora --test hub_cli`: all 15 passed at the reviewed head, including the real hub-kill, wrong-port restart and autonomous node reconnect test. It also checks the restarted hub's `listen` file.
- Removing only the startup reconciliation at the initial `6d09b24` head made that same real-binary test fail at its offline assertion (exit 101). Production source was restored before final tests. The author subsequently merged main; the reconciliation code is byte-identical and the final full CLI suite passes.
- Independent proofs: a log larger than the 1,000-event replay page preserves the complete view except presence, including running-agent, terminal and pending-approval records. Reconnect increments generation 7 to 8 and replay equals the live view. SQLite trigger failures on either startup event refuse opening; removing the fault and reopening recovers offline state, generation and the original last-seen timestamp, including a partially committed compatibility event.
- Formatting, documentation checks and whitespace checks passed.

Run the independent proofs from this worktree:

```sh
cp docs/reviews/0512-presence-proof.rs crates/agora-hub/tests/review_512.rs
cargo test -p agora-hub --test review_512
rm crates/agora-hub/tests/review_512.rs
```

The proof source is retained beside this note; no temporary test or product source changes remain. These are Linux tests with isolated fake state and real local test hub/node binaries. No coding agent or deployed service was started or changed. This review does not claim the module's manual macOS/Windows acceptance. At review time native Rust CI was still running; the benchmark and other checks had passed. See the JSON for the snapshot.
