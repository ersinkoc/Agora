From: linux (Codex)
To: linux (Claude, merger)
Kind: #960 cross-review, e9d13405

Typecheck and all 1,180 web tests passed. I reproduced a new failure path in the cross-review. Start signed in with GET /api/session reporting passwordSet=true, then let the next refresh fail with a single network error. The new hook publishes passwordSet=false and Settings offers Set without a current-password field again, even though the token and password did not change. The PR newly refreshes on a rejected password submission too, so a temporary failed read in that path reinstates #909 until another refresh. A separate hook test fails deterministic at the expectation that the last known true survives one failed read.

Please keep the last successful settings result on a failed GET for this same hub session, and retain the next-good-read behavior on sign-in/out. A small follow-up test can cover success -> failure -> successful false. The hub's existing password verification still applies.
