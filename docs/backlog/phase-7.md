# Phase 7 backlog (scale out)

Started with the Windows node (ADR-0045). The only machine used for development is Linux: Windows code is compiled for `x86_64-pc-windows-gnu` and linted locally, and everything that runs is proved by the `windows-latest` CI job.

| Task | State | Notes |
| --- | --- | --- |
| P7-01 Windows node W1: named-pipe control channel | in progress | `crates/agora-platform/src/windows.rs`: `local_listen`/`local_connect`/`LocalStream` over a named pipe `\\.\pipe\agora-<file name>-<hash of the path>`, owner-only security descriptor (`D:P(A;;GA;;;OW)(A;;GA;;;SY)`), remote clients rejected, first instance exclusive (a second listener gets `AddrInUse`), overlapped I/O so a waiting read never blocks a write from another thread, read timeouts and `shutdown` by cancelling the pending read. `LOCAL_SOCKETS_SUPPORTED` is true on Windows. Unit tests in `windows.rs` (they run only on Windows) |
| P7-02 W2: ConPTY terminals and job objects | not started | `PTY_SUPPORTED` stays false until this lands |
| P7-03 W3: process view (tree, name, stats; no cwd) | not started | |
| P7-04 W4: supervisor, terminal and node integration tests on `windows-latest` | not started | they are `#![cfg(unix)]` today |
| P7-05 W5: docs and the install story | not started | |
| P7-06 Desktop app (Tauri), team mode (Postgres, audit), mobile approvals, agent sandboxing | needs-decision | each needs its own ADR; none started |
