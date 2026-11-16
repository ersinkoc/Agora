# #574: counter fix and relay measurement verified

Reviewed head: `8c35c59a3bf03d6b4dcaef9d11debd1c2be27b11`. **Approve.** The original chat-accounting P2 is fixed and its thread
is resolved. The required relay measurement is now supplied and independently checked. All required native CI is green.

## Correctness

The [original unchanged proof](0574-chat-bytes-proof.rs) now passes: a valid 4,104-byte ChatSnapshot plus 4 terminal-output bytes
leaves terminal_bytes at **4**, rather than 4,108. The author's exact mixed-stream test also passes: chat snapshots/entries and
chat relay drops count for neither terminal metric; terminal snapshot/output bytes count exactly; queued terminal frames count,
the rejected send does not, and the terminal drop counts once. The source keeps all terminal counting conditional on the relay's
terminal identity. Docs state that scope.

The latest head also saturates histogram sums. Its far-future-time test passes, so multiple extreme node timestamps do not wrap
approval_wait_seconds_sum. Approval ids remain producer-qualified and timestamps remain on the requesting machine's clock.

`cargo test -p agora-hub`: **261 passed, 4 ignored**, plus the original proof passes separately. Formatting, clippy for all hub
targets, docs and whitespace checks pass. No production source changes are retained by this review.

## Independent release measurement

The author added a reproducible relay_path_cost measurement. I first repeated that harness in separate fixed/base/fixed builds;
the third run slowed substantially while other builds were active. Those unpaired samples cannot establish the counter's cost
and are preserved in the raw record rather than silently discarded.

The [paired control](0574-paired-relay-bench.rs) compiles the actual #562 baseline Router (`b2fe875`, without its tests) beside the
actual reviewed Router in **one release test binary**. It alternates before/after order for nine pairs per case, 100,000 frames
per sample, draining each client's queue every 256 rounds. Every sample asserts no relay was dropped. The only added baseline
code is its test-module wrapper; the timed implementation is the original source, not a reimplementation.

| Clients | Payload | Baseline median ns/frame | Fixed median ns/frame | Change |
| --- | --- | --- | --- | --- |
| 1 | 64 B | 79.0 | 81.5 | +3.16% |
| 1 | 4 KiB | 155.5 | 158.4 | +1.86% |
| 8 | 64 B | 82.2 | 84.8 | +3.16% |
| 8 | 4 KiB | 403.3 | 399.5 | -0.94% |
| 32 | 4 KiB | 767.1 | 750.7 | -2.14% |

The largest measured increase is about 3.2%, below the repository's 10% regression threshold. This is a warm synthetic queue-path
comparison including payload allocation and draining, on one Linux machine; it does not certify network/PTY capacity or the full
terminal latency budgets. Bulk-payload absolute times differ between harnesses because allocation and process state dominate;
the paired check compares both paths under the same harness and alternates their order.

Run in a disposable worktree at the reviewed head:

```sh
python3 docs/reviews/0574-run-paired-relay.py
```

The [runner](0574-run-paired-relay.py) creates a test-only baseline module, appends the paired test, runs it, then restores lib.rs
and removes the module in finally. [Structured results](0574-fixed-validation.json) contain all 45 paired samples, the noisy
unpaired matrix, test counts and the original fixed regression. No deployed hubs, accounts or agents were involved.

## Integration

#562 is merged. The author repaired #574's base; it is mergeable with all native CI green on this SHA. A simultaneous clean replay
was briefly opened as draft #587 to start CI, then closed when the author's repaired branch arrived; #574 remains the sole
integration candidate. No author branch/worktree was rewritten and no PR was merged by Codex. Remote evidence is preserved and the
throwaway task worktree/build artifacts are removed after posting the verdict.
