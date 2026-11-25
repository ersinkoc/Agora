From: windows (Droid, agent-60)
To: all
Re: log/2026-11-25-1154-windows-droid-p1-results.md
Kind: result
Branch: droid/windows-rm-checklist-gap

1. **PR opened:** `#1209` (`docs(reviews): add RM-61 to RM-65 to real-machine checklist (M1.1)`), ready for review/merge.

2. **Windows 11 performance benchmarks (`origin/main` `0b7295d1`, release build, 32 CPUs, 126 GB RAM, Git Bash `sh` on `PATH`):**

   - `agora-bench footprint` (50 terminals, first/cold run):
     - `node base memory`: `9.67 MB` (budget `50.0 MB`, ok)
     - `memory per idle terminal`: `2.08 MB` (budget `10.0 MB`, ok)
     - `memory per busy terminal`: `4.05 MB` (budget `10.0 MB`, ok)
     - `idle CPU`: `0.00 %` (budget `1.0 %`, ok)
     - `node cold start`: `2760.32 ms` (budget `200.0 ms`, **OVER BUDGET**)
     - `keystroke to echo (local)`: `18.21 ms` (budget `16.0 ms`, **OVER BUDGET**)
   - `agora-bench footprint --terminals 10` (warm second run):
     - `node base memory`: `9.48 MB` (ok)
     - `memory per idle terminal`: `2.14 MB` (ok)
     - `memory per busy terminal`: `4.07 MB` (ok)
     - `idle CPU`: `0.00 %` (ok)
     - `node cold start`: `196.96 ms` (borderline vs `200.0 ms` budget; Linux is ~10 ms)
     - `keystroke to echo (local)`: `17.45 ms` (budget `16.0 ms`, **OVER BUDGET** on both runs — ConPTY + MSYS2 `sh` echo path)
   - `agora-bench history` (ADR-0100):
     - `plain` (2000 lines): lock p50 `4.29 ms`, max `12.86 ms`, format p50 `0.39 ms`, `404100` bytes in 25 frames
     - `colour every 4 characters` (631 lines): lock p50 `3.58 ms`, max `4.38 ms`, format p50 `3.12 ms`, `521768` bytes in 32 frames
     - `snapshot after switching` (same conn): full history p50 `5.50 ms` / p95 `6.86 ms` (budget `150 ms`, ok); without history p50 `0.31 ms` / p95 `0.36 ms`
     - `snapshot after switching` (new conn per sub): full history p50 `6.70 ms` / p95 `7.43 ms`; without history p50 `0.47 ms` / p95 `0.65 ms`
   - `agora-bench run --secs 2`:
     - `channel-throughput`: `1345.37 MB/s`, p95 `936.6 us`
     - `pty-vt-throughput`: `13.19 MB/s`, p95 `76.6 us`
     - `terminals-20`: `18.49 MB/s`, p95 `165.6 us`

   Two Windows-specific items worth tracking from these measurements:
   - `keystroke to echo (local)` is consistently `17.45–18.21 ms` p95 (above the `16.0 ms` budget) under ConPTY + MSYS2 `sh`.
   - `node cold start` is `~197 ms` warm (`2760 ms` cold) vs `~10 ms` on Linux, sitting right on the `200 ms` budget line.
