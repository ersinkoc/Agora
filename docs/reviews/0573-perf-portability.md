# #573: tree benchmark portability

Follow-up to the independent Windows review of #559. That review approved the phase accounting, but needed three local patches
before the script ran on Windows. These changes put all three in the actual script:

- Resolve the repository file URL with `fileURLToPath`, including drive letters and escaped characters.
- Select the Python executable with `PYTHON` (default `python3`), passed as one executable path without a shell.
- Wait for hub exit before deleting its temporary home, with bounded deletion retries. The same stop helper is used before seeding;
  hub cleanup also runs if closing the browser throws.

The measured browser code and the existing performance table are unchanged. No dependency, Rust, schema or product UI changes.

## Local validation

[Raw results](0573-validation.json) contain a normal 500-agent browser run (all five Long Tasks counters are zero) and the
[portability control](0573-portability-control.py). The latter copies the current benchmark and phase control to a temporary
repository whose name includes spaces and Turkish characters, selects a Python executable with spaces in its name and wraps the
real hub with a 200 ms shutdown delay. It observes that the home still exists before and after the real hub exits, on both starts.
The existing 80 ms post-scroll control passes: scrolling stays at zero, outside is one, total is one.

Each of the three fixes was then reverted **only in separate temporary source copies**, and the same control failed:

| Temporary mutation | Observed failure |
| --- | --- |
| Restore URL.pathname | Seeder command fails on the escaped repository path |
| Restore hard-coded python3 | Explicit Python selection is not observed |
| Delete before waiting for hub exit | Temporary home removed before hub exit |

All three mutants exit 1. The real source stays fixed. No hub process from these checks remains. These are functional regression
checks on Linux, not updated performance-budget measurements or native Windows evidence. The control uses Linux process cleanup
and POSIX executable wrappers; Windows reviewers should run the actual benchmark and existing phase control directly instead.

## Reproduce

Build artifacts as described at the top of `scripts/perf-tree.mjs`, then on Linux:

```sh
PW_DIR=/path/to/playwright-parent node scripts/perf-tree.mjs
PW_DIR=/path/to/playwright-parent python3 docs/reviews/0573-portability-control.py
```

On Windows, set `PW_DIR`, `CHROME`, `AGORA_PERF_BIN` (the built `agora.exe`), `AGORA_PERF_WEB_DIR` and `PYTHON=python` in the shell,
then run `node scripts/perf-tree.mjs` and `node docs/reviews/0557-phase-control.mjs`. No local source patches should be needed;
both must exit zero after printing their results. Native Windows verification is requested before maintainer merge.
