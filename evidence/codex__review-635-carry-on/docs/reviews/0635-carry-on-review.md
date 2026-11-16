# PR #635: automatic carry-on review

Reviewed head: `11ea0a5692e870bde042903f629e8f4f215c86a1`.
PR already merged by the Linux developer at `552bcb6d2f816ab9645b6b57c9c2076b01fe3fc6` when reviewed. The reviewer did not merge it.

Verdict: **APPROVE**. No actionable defect found in this increment. This is independent cross-review after merge, not the author's self-approval or a claim to have tested real coding agents.

The automatic path computes carry-on from node-local policy and an interrupted ledger entry whose previous state was exactly `working`. The explicit Resume path passes false, including Start again. The working-state gate, once-per-interruption ledger field, automatic-resume cooldown and reset of the supervisor's carry-on flag prevent the tested repeats. A safe native session id is required; the prompt is a manifest literal inserted as its own argv argument, without shell interpolation. Existing launch confinement/options still apply. The unreadable-policy case disables carry-on. The hub has no new message or field for supplying this text or enabling it.

Validation on Linux at the exact head:

- `cargo test -p agora-adapters -p agora-node`: **463 passed**, none failed or ignored (including the existing session-resume and policy tests).
- Three independent real-node/fake-agent controls: unknown and done states plus a typo policy (`ON`) resume without a prompt; a working agent gets metacharacters literally in one argument with the same native session and no sentinel shell side effect; a person's Start again creates a fresh session without the prompt. All passed.
- Negative control: removing only `last_state == "working"` makes the existing native blocked-state test fail with **"blocked: the sentence was sent"**. The runner verifies this specific failure and restores the production file in `finally`. Production diff is empty afterwards.
- `cargo fmt --all -- --check`, `sh scripts/check-docs.sh`, `git diff --check`, and `cargo clippy -p agora-adapters -p agora-node --all-targets -- -D warnings` passed after the temporary target was removed and the source restored.
- GitHub checks for the reviewed PR head were green, including Ubuntu, macOS and Windows. Local native evidence is Linux only.

Reproduce the independent controls: `python3 docs/reviews/0635-run-controls.py`. Reproduce the guard-negative control: append `--negative`. The runner copies the existing session-resume fixture into a temporary test target and appends `0635-independent-controls.rs`; it always removes that target. Run it in an isolated review worktree without another build using the same source. No real agents, accounts, tokens or user ledgers are used.

Scope limits: the shipped continuation commands cover Claude Code and Codex. ACP and Wrongstack remain resumed without a first prompt as explicitly disclosed in this increment. Fake-agent argv receipt demonstrates delivery at process launch, not completion of the resumed user's task. The existing cooldown intentionally leaves a recently auto-resumed agent offered after another quick node restart.
