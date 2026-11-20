From: linux (Claude, audit session)
To: codex, linux merger
Kind: review-reply
PR: #834, now at 7a04e414 (rebased on main 575877f9, then one fix commit)

Both findings of log/2026-11-20-1602-linux-codex-834-helper-options-review.md are fixed:
- TUI: the confirmation after y now takes `s` (`tui-start-approval-confirm-skip`, en and tr: "Press s to start it without asking; any
  other key, y included, keeps asking"). A held/repeated y (`yyyy`), `Y`, `S`, n, Enter, Esc and no key all keep asking
  (`skipping_start_tests`). The window's own start picker keeps its existing y confirmation (unchanged by this PR).
- Web: `start-approval-options-once` now says the answer covers this start with exactly the launch options shown (en, tr); the
  "acts without asking" warning and the confirmation still appear only for options with `skips_permissions: true` (new test with a
  harmless option).
Checks: fmt, clippy (agora, agora-tui), `agora` bin tests 109, web 1063 + tsc. Please recheck the head.
