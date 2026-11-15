From: linux (Codex)
To: linux2, linux
Kind: result
Re: #574, #587

**#574 APPROVE at `8c35c59a3bf03d6b4dcaef9d11debd1c2be27b11`; review posted.** Original unchanged proof now reports 4 terminal bytes with 4,104 chat bytes excluded; P2 thread resolved. Exact mixed-stream/drop assertions and the saturation test pass. 261 hub tests pass, 4 ignored, plus the original proof; fmt/clippy/docs/whitespace and all native CI green. #562 integrated, #574 mergeable; no remaining Codex review blocker for the maintainer.

Relay measurement independently checked: separate-build repeats were noisy under concurrent load (all raw runs retained). Compiled the actual baseline/current Routers in one release binary, nine alternating pairs per case, 100,000 frames/sample, no drops. Median deltas: 1×64 B +3.16%, 1×4 KiB +1.86%, 8×64 B +3.16%, 8×4 KiB -0.94%, 32×4 KiB -2.14%. Synthetic queue path only, not full network/PTY capacity certification.

Evidence/runnable paired control/raw samples: `codex/review-574-fixed` at `a30ab86f`, `docs/reviews/0574-fixed-verified.md`. Product source restored byte-for-byte. Clean review worktree/local branch and 3.2 GB target removed after remote-backup/process-user checks. Temporary duplicate draft #587 is closed; keep #574 as the sole integration candidate. No author branch/worktree rewrite or merge by Codex.
