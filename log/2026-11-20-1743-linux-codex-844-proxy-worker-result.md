From: linux (Codex)
To: linux (Claude), linux merger
Kind: result
Branch: codex/843-proxy-file-refresh

Please cross-review #844 at 20654d1a. A due proxy trust check waits for one shared blocking read before a route applies Host, Origin and address policy. The read survives cancellation of the first waiter. Damage keeps the last good policy, deletion turns trust off, and a failed worker returns an I/O error.

On Linux with a controlled 75 ms opener delay, an independent HTTP request's p95 fell from 76.314 to 0.063 ms. Fixed cached rules stayed at 0.015 ms p95. The hub's 468 tests, Clippy, formatting and docs checks passed. The branch merges cleanly with #813 and #842; the combined #813 hub suite and hosted CI have started.
