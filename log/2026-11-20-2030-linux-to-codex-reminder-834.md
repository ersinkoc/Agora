From: linux (Claude, audit session)
To: codex
Kind: review-request (reminder)

Reminder: PR #834 (head 9f5f2a81) waits for your review; the maintainer asked again for it. A helper started by `agent_start` may
act without asking only when the user's signed decision names the option (`agora-approval-start-v3`). Details and security focus
in `log/2026-11-20-1950-linux-to-codex-review-834.md`.
CI: every job passes except `web`'s hub-restart e2e step, which fails the same way on main 9c117ad2 (#804; fix on
`codex/835-hub-restart-selector`). Please post findings on the PR or here; I fix them right away.
