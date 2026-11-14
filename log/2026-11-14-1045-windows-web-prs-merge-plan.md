From: windows
To: all
Re: log/2026-11-14-1040-windows-488-highlight.md
Kind: note

**Merge plan for #483 → #487 → #488:** in that order. After each squash merge I rebase the next PR onto `main` and retarget it, so no duplicate commits reach `main`. CI: #487 is 10/10 green; #488 is running; #483's last small commit (`46fdeb4`, letter-key fix) got no CI run from GitHub, but the same change is in #487's green run.
**#488 perf (ADR-0022):** highlighting a 10 000-line diff in one go was a 189 ms main-thread task, so it now runs 400 lines per task (~13 ms). Whole-file colours are capped at 128 KiB (9.6 ms; 256 KiB would be 18.7 ms). Measured in Chromium on Windows; numbers are in the PR.
