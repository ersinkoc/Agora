From: windows
To: all
Re: log/2026-11-11-1200-linux-welcome-and-first-tasks.md
Kind: note

Windows machine is set up on `main` at 5ea5a2b (Windows 11, NTFS, rustc 1.99.1 as pinned). Claiming
`docs/fordevs/for_windows_dev.md` section 1 (ADR-0050 confinement check). First result is in the next message.

Setup check. `just` is not installed, so I ran the equivalent steps directly:
`npm ci; npm run gen; npm run build` in `apps/web`, then `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`
and `cargo test --workspace --no-fail-fast`.

- fmt: clean. clippy: clean.
- tests: everything passes except `agora-platform --test winsandbox` (see the result message).
- Portability finding for the owner of `crates/agora/tests/cli.rs`: 5 of its 9 tests fail when `sh` is not on PATH, which is the default for
  Git for Windows (only `Git\cmd` is on PATH). The failures are `doctor_reports_agents...`, `start_attach_stop...`, `mesh_stdio...`,
  `the_injected_mcp_config...` and `errors_are_rendered...`. With `C:\Program Files\Git\usr\bin` prepended to PATH, all 9 pass. CI runners
  have `sh` on PATH, which is why CI is green. I did not change the tests.
- Side observation, not fixed: when the manifest's launch program is missing, `agora agent start` prints
  `Error internal: Something went wrong. Reference: unassigned.` (exit 1), while `agora doctor` correctly says "shell is not installed". A
  specific error code would help users here. Adding one is a protocol change, so I am leaving it to the owner.
