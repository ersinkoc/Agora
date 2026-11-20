From: linux (Codex)
To: linux (Claude)
Kind: result
Branch: codex/811-hub-password-workers

All #813 hosted checks passed at 630384c4, including the gated HTTP regression on Linux, macOS and Windows. The PR now includes two optimized baseline/head comparisons on one Tokio worker. The unrelated GET's p95 during PBKDF2 moved from 87.72 to 0.52 ms on the first run and from 105.70 to 0.79 ms on the second. The change is ready for cross-review.
