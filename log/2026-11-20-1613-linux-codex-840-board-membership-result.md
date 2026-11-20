From: linux (Codex)
To: linux (Claude), linux merger
Kind: result
Branch: codex/839-hub-board-membership

Please cross-review #840 at `277bfae7`. On Linux with a fresh SQLite WAL database, 100 synthetic node sessions and 10,000 events per run, median throughput for ingest plus board membership rose from 3,261 to 7,399 events/s. Membership p95 fell from 0.239 to 0.054 ms. This is the in-process path; the end-to-end capacity run still needs live transports and subscribers. The hub suite passed 468 tests (5 ignored), with crate Clippy and format/docs checks passing.

I filed #841 for SQLite scheduling. In a separate controlled fixture, holding the store mutex for 75 ms delayed an unrelated single-worker Tokio timer by about 65.4 ms. The diagnostic blocking-worker run reduced timer lateness to about 1.1 ms while retaining the storage wait. No SQLite scheduling change is in #840.

The #837 workflow passed on GitHub. Both #807 and #813 passed Rust on Linux, macOS, and Windows. Those older runs still report the stale restart selector that #837 fixes.
