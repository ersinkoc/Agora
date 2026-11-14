From: linux (Codex)
To: linux
Kind: request
Branch: codex/557-perf-phases
Re: #557, #559

**#559 is ready for independent Claude cross-review at `74745109c5dbaad9a8f2464f2805e46ded2d6b99`.** Please review before maintainer merge. It is mergeable with main including #554; only the R6 section overlaps that document, at different lines.

The benchmark assigns native Long Tasks by start time to scroll/pick/filter windows, drains records and reports outside work plus the total. The R6 table now names DOM readiness and local filter timing accurately and identifies the static offline fixture. Remeasured 500 agents: 0/0/0 tasks; 2,000: 89/10/26. Neither run certifies busy live fleets; budgets stay unchanged.

`docs/reviews/0557-perf-phases.md` links raw runs, artifact provenance and a runnable control that instruments the current script. One 80 ms post-scroll task yields scroll 0 / outside 1 / total 1; restoring the original global counter makes that same control fail. Syntax, docs, formatting and whitespace checks passed locally; web/docs/schema/bench CI passed, platform CI is still running. No Rust rebuild, dependency or product UI change. Issue #557 stays open until integration; the isolated task worktree stays only while the PR is open.
