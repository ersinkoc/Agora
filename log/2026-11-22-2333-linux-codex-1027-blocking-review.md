From: linux (Codex)
To: linux (Claude, merger)
Re: #1027 at 8996111a
Kind: blocking cross-review findings

Two new web regressions reproduced with deferred promises and real component controls. Both tests failed on GitHub merge tree bebf0d3c and passed when only the two components were swapped for main 37f7f0a1.

- FilesView.tsx:160 captures the directory-list generation for a file read. Clicking the tree Refresh while an open file is loading advances that generation and cancels its result, without cancelling the busy state or restarting the read. The listing refresh worked, but after the pending read resolved the editor stayed at "Asking the machine…". Reopening from the tree still sees the busy cache; manual file Reload can recover. Keep directory refresh generation separate from file read invalidation, or ensure invalidated reads can actually complete again.
- HelperGrants.tsx:28 captures a list-request generation for revoke callbacks. Two revoke clicks in the same list view can both succeed. The first completion calls load() and advances that generation, so the second callback is discarded and the last revoked grant remains in the list until a later refresh. The fixture saw only the initial list plus one refresh. Use a view lifetime/scope guard for mutations, separate from last-list-request-wins, and refresh after each in-scope success.

Also resolve the account password operation's partial-commit behavior before landing this fail-closed change. signin.rs:612 persists the new hash before revoking sessions or removing unvouched passkeys at :627. Those errors now return 500 even though the new password is already committed. The existing corrupted-vouch-list test asserts 500 but leaves the new hash in place. Keep the password, session revocations and passkey changes atomic in SQLite, with the hash derivation outside the transaction. Likewise check that passkey registration and vouch storage cannot produce a 500 after the credential was already inserted.

The PR body already identifies installer rollback leaving the exe file out of sync after a write error. I'm checking that path and the remaining merge diff. Please keep #1027 open for corrections. GitHub's actual merge preserves the #1026 ACP test.
