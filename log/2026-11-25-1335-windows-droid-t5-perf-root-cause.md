From: windows (Droid, agent-60)
To: all
Re: log/2026-11-25-1332-windows-droid-bench-and-pr1209.md
Kind: result

Root-cause diagnosis for the two Windows 11 footprint budget findings (`t-5`):

1. **`node cold start` (`196.96 ms` warm / `2760.32 ms` cold vs `200.0 ms` budget; Linux `~10 ms`):**
   - `crates/agora-node/src/git.rs:320` (`discover_checkout`) spawns **three sequential `git.exe` processes per project** during `Node::start_inner`:
     1. `git rev-parse --show-toplevel`
     2. `git rev-list --max-parents=0 HEAD`
     3. `git remote get-url origin`
   - Measured directly on this Windows 11 machine against the Agora checkout: `rev-parse` `418.6 ms` + `rev-list` `67.0 ms` + `remote` `436.0 ms` = **`921.6 ms`** (and ~65 ms × 3 ≈ `195 ms` warm on the tiny bench repo, accounting for nearly all of `Node::start`). With multiple configured projects this scales linearly (3 spawns × N projects).
   - Minor bonus: `Node::start_inner` (`crates/agora-node/src/daemon.rs:1805-1988`) also reads `POLICY_FILE` 6 separate times instead of reusing `policy`.
   - **Actionable suggestion for merger:** parallelize `rev-list` and `remote get-url` (and/or multi-project discovery) or reduce `discover_checkout` process spawns; reuse the parsed `policy` in `start_inner`.

2. **`keystroke to echo (local)` (`17.45–18.21 ms` p95 vs `16.0 ms` budget):**
   - Inspected `crates/agora-platform/src/conpty.rs` (`WriteFile` on caller thread, blocking `ReadFile` on reader thread) and `crates/agora-node/src/terminal.rs`: **Agora's own input/output hot path has zero sleeps or polling** (confirmed by `pty-vt-throughput` p95 = `76.6 us`).
   - The `17.45–18.21 ms` p95 comes from the **ConPTY + MSYS2 `sh.exe`** echo path that `bench/src/footprint.rs` launches on Windows (aligns with Windows' default `15.625 ms` 64 Hz timer tick), not Agora overhead.
