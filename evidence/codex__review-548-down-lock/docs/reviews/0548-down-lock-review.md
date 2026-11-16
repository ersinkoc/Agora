# PR #548: remaining ownership-check/unlink race

Reviewed head: `73a1cc4766cc03b0416f2e067b1c76e22905bfb1`.
Decision: **changes requested** (one P2).

`remove_lock_of` checks the path's pid and birth time, then performs a separate `remove_file`. A normal claimant can replace that path between the check and the unlink. The comparison narrows the original race but does not eliminate it. The stale-lock branch of `down` then breaks, so the new supervisor becomes invisible to subsequent down calls.

## Deterministic real-process proof

The adjacent Python harness sets up an isolated short temporary AGORA_HOME and an empty local Git project. It writes a valid stale lock record (a PID with an impossible old birth time, as an abandoned role lock would be). The selected down process is paused immediately before libc unlink, after its ownership comparison has succeeded. A real `agora supervise node` reclaims the stale lock normally and starts its actual node child. Its valid replacement pid/start lock is recorded, then the pending unlink is released.

Observed twice at the reviewed binary:

- Down exits **0** and says **Nothing to stop**.
- The replacement's lock is gone, while its supervisor and node child are both alive.
- A second down also exits **0**, says **Nothing to stop**, and leaves both alive.

The Linux LD_PRELOAD helper changes only scheduling at the selected unlink. It does not change file contents, unlink arguments/results, process signals or application source. The replacement lock is written by the actual supervisor. No production code was modified to reproduce the defect.

The harness cleans up its exact child PIDs and checks node birth time before signalling it; it waits for cleanup and removes its temporary home. No deployed service or coding agent is used.

## Reproduce

Build this review branch's binary, then run the scheduling proof (the expected result is the failed final invariant assertion, exit 1):

```sh
cargo build -p agora
cc -shared -fPIC -O2 -o /tmp/agora-548-unlink-gate.so docs/reviews/0548-unlink-scheduling-gate.c -ldl
python3 docs/reviews/0548-unlink-ownership-proof.py target/debug/agora /tmp/agora-548-unlink-gate.so
```

With a custom Cargo target directory, pass its debug/agora path instead. The harness emits a result JSON and an assertion, and cleans up even on failure. The attached observed JSON records one run; the failure repeated after generalizing the harness to accept binary/helper arguments.

## Ordinary race and other checks

The original deterministic ownership unit test passes. The requested Linux stress run, using the PR's actual front_door integration test with only its loop bound increased from **6 to 20**, also passes all rounds in **68.31 s**. The test file was restored afterwards. This stress success does not cover the interleaving pinned by the scheduling proof. All named native CI checks are green, including Linux/macOS/Windows Rust and the benchmark. Formatting, documentation and whitespace checks pass. No product changes remain.

Fix direction: serialize ownership comparison/removal with claim/replacement under a shared guard, so the checked role-lock path cannot change before its removal. Another pid/start read alone leaves the same gap. Preserve a replacement holder's visible lock even when down has reached its bounded stop limit, and add a regression that pins replacement after the final ownership check.

This is a Linux proof of the shared filesystem protocol; no claim is made about reproducing the timing on other OSes. The synthetic initial stale record represents a legitimate abandoned-lock state, while the replacement supervisor and node are real processes.
