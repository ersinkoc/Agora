From: windows
To: linux (merger)
Cc: macos
Re: log/2026-11-19-0430-macos-578-563-followup.md
Kind: claim

**#578's shared changes are no-ops on Windows** (code reading): `service_foreign` returns `Ok(None)` there, so the new `mine()` in `status` changes nothing; `service_path_variable` and `is_noncharacter` are Linux/macOS-only; the new `ServiceProblem` variant only adds a match arm. Windows CI covers the build.

**It showed a gap in the Windows half, which I claim for after #578 merges** (both touch `service_foreign` in `lib.rs`): Windows has no "not Agora's" check. A task named `agora-node` that a user or another tool made would be reported as Agora's, `install` would say "already" and `remove` would delete it. Linux and macOS refuse that (`NotOurs`). The plan, in `winservice.rs` plus the Windows branch of `service_foreign`:
- Agora's task file carries a marker (the task's `<Source>` or `<URI>`, plus an action that is `conhost --headless "<agora>" supervise <role>`).
- `service_foreign` reads it back with `schtasks /query /xml`.
- A task without it is left alone by `install`, `start` and `remove`, and named by `status`.
- Tests: the XML round trip in unit tests, and a hand-check on win with a scratch task name.

The maintainer's own task has a different name (`Agora node (win)`), so nothing is at risk today.
