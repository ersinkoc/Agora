# #557: tree benchmark phase attribution

The benchmark now classifies Long Tasks entries by their start time inside separate scroll, pick and filter windows. It drains
pending observer records after the final task and disconnects the observer before counting. A task outside those windows has
its own count; all four counts sum to the total.

## Measured workloads

- [500-agent static fleet](0557-500-agents.json): 0 scroll / 0 pick / 0 filter long tasks; 0 outside; total 0.
- [2,000-agent static fleet](0557-2000-agents.json): 89 scroll / 10 pick / 26 filter; 0 outside; total 125.
- [Controlled post-scroll task](0557-post-scroll-control.json): the one deliberately inserted 80 ms task starts after scrolling
  and before picking. Scroll/pick/filter stay at 0; outside is 1 and total is 1. The live control assertions pass.

The runs use headless Chromium with software rendering at 1440×900 on one Linux machine. No node connects, no real agent runs,
and no terminal streams. These are static tree measurements. The binary/web assets were reused read-only from the clean author
build; the benchmark changes are JavaScript only. No additional Cargo target was built.

The updated table in [performance.md](../design/performance.md) reports warm reload until rows exist in the DOM, programmatic
row click to frame and local Explorer-filter change to frame. None is claimed to measure actual paint or complete live-view
model derivation. The observed 500-agent frame tail misses a 60 Hz interval, and 2,000 agents miss both scroll and selection
budgets; the requirements themselves are unchanged.

## Reproduce

With built binary/web assets and a Playwright installation:

```sh
PW_DIR=/path/to/playwright-parent AGORA_PERF_BIN=/path/to/agora AGORA_PERF_WEB_DIR=/path/to/web/dist AGORA_PERF_FLEET="10 5 10" node scripts/perf-tree.mjs
PW_DIR=/path/to/playwright-parent AGORA_PERF_BIN=/path/to/agora AGORA_PERF_WEB_DIR=/path/to/web/dist AGORA_PERF_FLEET="20 5 20" node scripts/perf-tree.mjs
PW_DIR=/path/to/playwright-parent AGORA_PERF_BIN=/path/to/agora AGORA_PERF_WEB_DIR=/path/to/web/dist AGORA_PERF_FLEET="10 5 10" node docs/reviews/0557-phase-control.mjs
```

The control instruments the **current** benchmark rather than running a copied implementation. It temporarily creates a script
in scripts/, inserts the task between measured phases and checks the native Long Tasks observation, phase attribution and sum.
The temporary file is removed in finally. Restoring only the original global scroll counter makes this same control fail with
`Scroll counter includes another phase` (exit 1); restoring the fixed source leaves the positive result above. This is an
attribution regression check, not an injected product performance claim.

Machine-specific environment paths and login codes are not recorded in the results. Browser profiles, temporary hubs and the
control script are cleaned up; no deployed services, real accounts or coding agents are touched. No dependencies or product UI
changes are introduced. The structured validation record is [0557-validation.json](0557-validation.json).
