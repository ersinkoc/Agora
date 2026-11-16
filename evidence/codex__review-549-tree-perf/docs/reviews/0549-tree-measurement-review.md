# PR #549: large-tree measurement review

Reviewed head: `601a30f3d074755bb26f3d449da4c57b89d63039`.
Decision: **changes requested** (one P2: isolate the long-task counter to the phase it names).

## Reproduced baseline

The unmodified 500-agent run broadly matches the reported result: first load 135 ms; scroll frame gap 16.8 ms p50 / 21.8 ms p95; pick-to-frame 31.6 / 41.9 ms; filter re-render 26.1 / 35.1 ms; 500 rows, 3,810 tree nodes, 11 MB heap and no long tasks. This is a static synthetic-fleet result, not evidence of a product regression.

## Long-task counter covers later phases

The observer at lines 58–59 stays active through the later 12 picks and 20 filter changes. Its final `long.length` is reported at line 80 as `longTasksWhileScrolling` and appears in the documentation's scroll column.

A natural 2,000-agent run with timestamped entries returned **85** as that metric. Only **49** entries started inside the scroll window; **36** started after scrolling ended. No workload was injected in this run.

A controlled 500-agent run inserted one separate **80 ms** task strictly after scrolling ended. The actual scroll window had **0** long tasks, yet the original metric returned **1**. The timestamped entry started after the recorded scroll end and had duration 80 ms. This pins the attribution defect independently of the fleet's speed.

Suggested fix: timestamp/separate observers for scroll, pick and filter phases, or stop/drain the scroll observer at its phase boundary. Re-run the table after correcting the counter. Keep any total-suite count under a separate name. This does not require a product UI change.

## Scope clarifications

- All seeded machines are **offline** after the hub reopens (the diagnostic saw 10/20 offline machine headings). The static tree can still be measured, but record that limitation rather than treating it as a live-agent navigation workload.
- The reported full-rerender action changes Explorer's local `filter` state (`Explorer.tsx`: its useState/onChange). It does not update App's `live` state or re-run its view-to-model work (`App.tsx`: onView/setLive and agents useMemo). The documentation should call this a filter/tree re-render, or separately measure a real view-changing event before equating their costs.
- The first-load endpoint waits for DOM row count. Label it accordingly unless paint is explicitly observed.

These are scope clarifications; the single formal finding is the demonstrably misattributed long-task count.

## Reproduce the diagnostic

The adjacent instrumentation is the PR's script with event timestamps, phase counters and an optional post-scroll controlled task. Put it in `scripts/` so its existing root calculation stays correct:

```sh
cp docs/reviews/0549-phase-instrumentation.mjs scripts/review-549-tree-phases.mjs
PW_DIR=/path/to/playwright-parent AGORA_PERF_FLEET="10 5 10" AGORA_REVIEW_POST_SCROLL_MS=80 node scripts/review-549-tree-phases.mjs
PW_DIR=/path/to/playwright-parent AGORA_PERF_FLEET="20 5 20" node scripts/review-549-tree-phases.mjs
rm scripts/review-549-tree-phases.mjs
```

As with the original script, build the binary/web assets first. The review reused the author's cached binary/assets from a clean worktree at the reviewed commit through read-only symlinks; it did not build a second Cargo target. The archived JSON files retain the unmodified baseline, controlled 500 run and uninjected 2,000 run (including timestamps).

Original script syntax, seed Python AST and documentation checks passed; native CI was green. No product source was changed, the temporary diagnostic script was removed, and the review worktree/symlinks are discarded after publishing. Borrowed author artifacts are untouched. Browser/hub state is temporary; no coding agent, real account or deployed service was used. All timings are one Linux machine with headless software rendering; the controlled 80 ms task is an attribution proof, not a real performance regression.
