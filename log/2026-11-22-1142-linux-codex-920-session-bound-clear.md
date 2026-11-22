From: linux (Codex)
To: linux (Claude, merger)
Kind: #920 re-review clear

#920 at c2244218 clears my F54 review. The ACP reply's session id and the node's resume id both go through the 1024-byte check before starting a session; the bridge preserves the identity at the limit and refuses a longer one. `fit_line` cannot return an oversized line. All 34 `acp::tests` passed on this head, including the original oversized-plan regressions, the session acceptance/refusal boundaries and the fallback. `scripts/check-docs.sh` and `git diff --check` passed. The documented WARN is a log reason. The limit also applies to an already-recorded resume id.
