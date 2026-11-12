# Failed-start worktree and branch rollback (#314)

L20 reported that `Supervisor::abandon` removes a newly made worktree after a failed
start but leaves its branch. Explicit selected-worktree launches have the same gap;
adapter launch failures also leave the new worktree itself.

## Change

`git::create_start_worktree` resolves the requested starting commit before creation,
creates the worktree at that exact commit, and returns a private receipt with its
checkout, worktree, branch and initial commit. No destructor performs cleanup.
Terminal/injection failures, selected-worktree failures and adapter launch failures
explicitly consume the receipt. Empty adapter commands are rejected before creation.

Rollback removes the node-created worktree first. If removal fails, the branch stays.
It then checks for another worktree using the branch, refuses symbolic refs and deletes
the direct ref with `git update-ref --no-deref -d REF INITIAL`. The expected object ID
comparison happens under Git's ref lock, so a changed commit is not deleted; disabling
dereferencing prevents a rewritten ref from deleting its target. Like ordinary Git
operations, repository mutation by external processes is not one global transaction;
the worktree-use check is best effort, while commit comparison/deletion is atomic.

Normal `create_worktree`/`remove_worktree` behavior still preserves branches. Selected
user-owned directories never acquire this receipt. Original launch errors are retained
even when best-effort rollback fails. This changes only `agora-node` and related docs;
there are no schema, sandbox-policy, dependency or hot-path changes.

## Verification

Ten public-API tests in `crates/agora-node/tests/start_rollback.rs` use disposable Git
repositories and a missing fake-agent executable, never real agents or credentials:

- Failed terminal start cleans up and can retry with the same name.
- Selected-worktree and adapter failures clean up their own worktree/branch.
- Injection failure cleans up and calls the endpoint abort hook exactly once.
- An explicit starting commit is compared against itself, not current checkout HEAD.
- A new commit made during injection remains reachable on its branch.
- A branch checked out elsewhere, a locked worktree, and a rewritten symbolic ref stay.
- Pre-existing branches and uncommitted files in selected user directories stay.

Before the fix, the original eight-test suite has three failures: terminal and selected
launches leave their branch, and adapter failure leaves its worktree. The five retention
tests pass already. With the fix all ten pass. Temporarily removing the expected-OID,
checked-out-branch and symbolic-ref guards separately makes the corresponding retention
test fail; each mutation is restored before final validation.

Full workspace tests, Clippy with warnings denied, formatting, locale lint and docs
checks pass locally; final CI status is recorded in the PR. No real
workspace branches were removed, and the installed node/active agent are unchanged.
Independent cross-review and maintainer merge remain.
