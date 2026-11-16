# #596: Windows parent-time filter re-check

Reviewed head: `54c61dfa0e9649d3c224702edabe70614bef7393`. **Approve.** No blocking finding in this scoped review.

- A listed child is excluded only when both creation times are known and the child's is strictly earlier. Equal times and all
  unknown-time combinations retain the child. The existing decision test covers those cases.
- Parent creation time is read once before the snapshot. Child time is queried only after the snapshot entry matches the parent id
  and is not the process itself; it is not queried for every process in the snapshot.
- The existing start_time helper requests limited-query access, checks the live status and reads creation FILETIME. Query and
  snapshot handles are owned by Process and closed on drop, including early-return paths. No new access rights or termination is added.
- The implementation documents the existing snapshot race: a child's id reused before its time is read can still identify the new
  holder for that observation. The filter is best effort; #595 separately bounds traversal by visiting each pid once.
- This changes only winproc.rs, keeps the public API unchanged and adds no dependency, schema, user-facing string or runtime lock.

## Checks and attribution

`cargo test -p agora-platform` on Linux: **60 passed**. Windows and Linux all-target clippy with -D warnings, formatting, docs and
whitespace checks pass. All required native CI is green on the reviewed head.

The [portable runner](0596-run-rule-test.py) extracts the actual started_after helper and its **existing test** unchanged, compiles
them with rustc --test and executes that one pure test on Linux: pass. This verifies the decision test, not Windows APIs or process
snapshots. It introduces no copied/reimplemented decision logic or additional mirror test.

Native Windows evidence is the author's [3/3 winproc test report](https://github.com/ersinkoc/Agora/pull/596#issuecomment-6893351668)
and Windows CI. The Linux reviewer did not execute Toolhelp or GetProcessTimes. The native test suite includes a real child being
found under its parent; the author also reported the stale-parent observation in the PR description.

Run from a disposable worktree of this head:

```sh
python3 docs/reviews/0596-run-rule-test.py
```

[Structured results](0596-validation.json) record source hashes, checks and attribution. Product source remains unchanged. No real
agent/account, deployed node, author worktree or security setting was touched. Remote evidence remains; the clean throwaway
worktree/local branch/target is removed after the verdict.
