# PR #551: lock ownership race fix verified

Reviewed head: `ea24c4b1bbcf97984320d5b0521ffc4458f3d0ec`.
Decision: **approve**. The original #548 P2 is verified fixed by this follow-up. Native CI remains a merge requirement.

The role lock now has a stable sibling guard protected by `File::lock`. Claim/replacement, front-door handover and compare/removal all use that same guard. The guard file is kept in place, and dropping its file handle or ending the process releases the OS lock. Failure cleanup now removes only the front door's own role lock.

## Same critical interleaving, with guard contention

The proof pauses down at the same selected unlink used to expose #548, after the final pid/birth-time comparison. A real replacement supervisor reaches its actual flock call (the scheduling helper marks that arrival), while down holds the guard. A separate nonblocking OS-lock probe confirms exclusive ownership. The role-lock contents remain unchanged while the replacement waits.

After release, down removes the old stale lock under the guard and finishes. The waiting supervisor then claims and starts its real node child. Its replacement role lock stays visible and names that supervisor. A second down finds it and stops **both** supervisor and node; the role lock is gone afterwards. The proof exits 0 and cleans up its exact processes and temporary home.

This adapts the original proof's wait condition to the corrected synchronization: waiting for the replacement to own the lock before releasing down would now be a deadlock in the test harness, because blocking that replacement is the fix. Syscall arguments and results are unmodified; the helper only controls scheduling and marks arrival at flock.

Reproduce:

```sh
cargo build -p agora
cc -shared -fPIC -O2 -o /tmp/agora-551-scheduling-gate.so docs/reviews/0551-guard-scheduling-gate.c -ldl
python3 docs/reviews/0551-guard-ownership-proof.py target/debug/agora /tmp/agora-551-scheduling-gate.so
```

With a custom Cargo target, pass its debug/agora path instead. The recorded observation is in `0551-guard-observed.json`.

## Validation and housekeeping

All 9 frontdoor unit tests pass, including guard contention and ownership preservation. Ordinary 20-round Linux race and native CI results are recorded in the adjacent JSON and review comment. The loop-bound change in the integration test was temporary and restored; no product edits remain.

Nine completed clean review/verification worktrees and their local branches were removed only after confirming every HEAD existed on its remote branch. Evidence links remain available remotely. The active review worktree/build artifacts are also discarded after the final report, as required by the repository's cleanup workflow.

The scheduling proof is Linux-specific. It uses a synthetic stale record, actual isolated supervisor/node/down processes and no coding agent or deployed service. It does not claim a timing reproduction on other OSes.
