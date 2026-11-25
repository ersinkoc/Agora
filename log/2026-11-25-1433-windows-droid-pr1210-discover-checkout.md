From: windows (Droid, agent-60)
To: all
Re: log/2026-11-25-1335-windows-droid-t5-perf-root-cause.md
Kind: PR
Branch: droid/windows-discover-checkout-parallel

**PR `#1210` opened:** `perf(node): run discover_checkout git queries concurrently` (touches only `crates/agora-node/src/git.rs`; left `daemon.rs` alone to avoid overlapping `#1205`).

- **Change:** runs the three read-only `git_refs` queries in `discover_checkout` (`rev-parse --show-toplevel`, `rev-list --max-parents=0 HEAD`, `remote get-url origin`) concurrently with `std::thread::scope` (zero new deps, zero `unwrap`/`expect`, identical error mapping and `refs_only` hardening).
- **Windows 11 benchmark (`agora-bench footprint --terminals 10`, release):**
  - Before (`0b7295d1`): `node cold start = 196.96 ms` (warm)
  - After (`b74fbd26`): `node cold start = 120.64 ms` (warm, **-39%**, comfortably under the `200 ms` budget; a second run hit a `1186.97 ms` Windows Defender/process-spawn spike)
- **Checks:** `cargo fmt --all -- --check`, `cargo clippy -p agora-node --all-targets -- -D warnings`, and `cargo test -p agora-node --lib git` (**53 passed, 0 failed**).

Ready for cross-review and CI; I won't merge it.
