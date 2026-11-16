# Ledger: crates/agora-platform/src/windows.rs lines 1-345, private files (Round 3)

## Invariants
- Nothing planted in the folder redirects a private file: a link at the name is refused and its target is neither written nor
  created (create_private_new doc: "CREATE_NEW, so a planted file or link is not followed").
- Checks and the ACL change go through one handle of the checked object.

## F-P1 — input-handling — Medium — a dangling symlink at a private file's name creates its target outside the folder
- Location: windows.rs create_private_new (CreateFileW CREATE_NEW without FILE_FLAG_OPEN_REPARSE_POINT); reached through
  open_private (open_append_private / open_lock_private) and write_private.
- Impact: whoever can write into the folder makes the node create a file wherever the link points that the user may write
  (an empty, owner-only file through open_private, before the refusal). In write_private the same open writes the secret's
  bytes through a link re-planted between its remove_file and create (race window; not proven here).
- Root cause: CreateFileW follows a symbolic link in the last component; CREATE_NEW only fails when the *target* exists.
- Proof (before): .temp_files/audit_proofs/src/bin/prove_P1_dangling_link.rs
    CONTROL  link to existing file -> Err(InvalidInput), target kept=true
    EXPECTED: refused (InvalidInput) and nothing created at ...\elsewhere\planted.txt
    ACTUAL:   Err(InvalidInput), target created = true
    PROBLEM CONFIRMED (exit 1)
- Patch: windows.rs create_private_new — FILE_FLAG_OPEN_REPARSE_POINT added to the CreateFileW flags (+ import, + comment).
- Proof (after): .temp_files/audit_proofs/src/bin/verify_P1_dangling_link.rs (append and lock openers, link into a missing
  folder, new log + append, lock opened twice, write_private create + replace) → FIX VERIFIED x3, exit 0.
  Original proof → target created = false / PROBLEM NOT REPRODUCED.
- Regression test: crates/agora-platform/tests/private_log.rs
  a_symlinked_log_whose_target_does_not_exist_is_refused_and_nothing_is_created (fails with the flag removed, passes with it;
  Unix opens with O_CREAT|O_EXCL|O_NOFOLLOW, which never follows the link, so it holds there too).

## Not reproduced (dropped)
- none

## Boundaries
- write_private's remove-then-create window (tmp name `<path>.tmp-<pid>` is predictable): the patch closes it (a re-planted link
  now fails AlreadyExists), but the race itself was not reproduced deterministically.
- owned_by_this_user accepts the token's default owner (Administrators when elevated): a documented choice, not a finding.
