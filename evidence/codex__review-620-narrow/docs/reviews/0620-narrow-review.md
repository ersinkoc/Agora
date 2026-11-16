# PR #620: narrow terminal cross-review

Reviewed head: `be1c6a0f004471d3f26f460376efc70d10131e74`.
Verdict: **APPROVE**. No blocking defect found in the requested narrow-resize fix.

`MIN_COLS = 2` is applied consistently in backend construction, `clamp_size` and resize, with one row minimum. The node asks the backend for the effective size before checking for a duplicate, resizing the real PTY or broadcasting. This preserves the no-op behavior for repeated requests that clamp to the same size. The refreshed branch keeps the existing OSC bound intact. No terminal-byte feed behavior, protocol schema or dependency is changed by this PR.

The standalone driver calls the actual reviewed libraries in separate processes with deadlines. Zero/one-column construction, wide CJK/emoji/combining text, and resize from 80x24 down to zero/one and back up complete without panic/hang; effective size and cursor remain valid. A real shell PTY receives six resize requests covering zero, one and 80 columns. Every `stty size` result agrees with the screen; subscribers get exactly four effective resize events, with no duplicate for three consecutive requests clamped to 2x1. The probe uses a private temporary file, no real agent/account.

Validation:

- VT/TUI library suites: **3 + 122 passed**.
- Node terminal/session-resume suites: **12 + 10 passed**. The node unit suite also passed all **202** in the initial broad run.
- Four bounded native driver cases pass (`feed-zero`, `feed-one`, `reflow`, `node`).
- Formatting, docs checker and all-target VT/node/TUI Clippy with warnings denied pass. Required native CI is green at the reviewed head; local native execution is Linux only.

The initial broad test command stopped after 318 passes and one timeout in `review_a_survivor_that_ended_after_announcement_can_be_cleared_by_stop` while waiting for the fixture process identity in the ledger. That test passed alone, and the entire affected session-resume binary subsequently passed 10/10 with terminal tests 12/12. No test or product source was changed to make the reruns pass. The initial command is not reported as a full-suite pass; remaining unrelated node integration suites were not rerun. This does not reproduce a narrow-resize defect, and the transient observation is retained in validation metadata.

Reproduction after building the three packages:

```sh
cargo test -p agora-vt -p agora-tui --lib
cargo test -p agora-node --test terminal --test session_resume
python3 docs/reviews/0620-run-narrow-proof.py
```

Only evidence files are committed; product source and existing tests remain unchanged. No merge performed.
