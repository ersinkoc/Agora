# Ledger: crates/agora-platform/src/winservice.rs (Round 1)
Worktree: ../agora-worktrees/windows-evidence-audit (branch claude/evidence-audit-windows-platform, base origin/main 78333fad)
Proof crate: .temp_files/audit_proofs (CARGO_TARGET_DIR=.temp_files/audit_target)

## Invariants
- A task without TASK_MARKER in <Source> is never replaced, started or deleted (foreign/remove/install/start).
- The command line written to <Arguments> reaches agora.exe as the same argv after `conhost --headless` re-forwards it.
- No `%` reaches the task (Task Scheduler expands it); every value is XML-escaped.
- `status` reads <Enabled> from <Settings>, not from a trigger.

## Confirmed
none

## Not reproduced (dropped)
- W1 conhost --headless re-quoting mangles args (spaces, trailing backslash, empty arg, embedded quote):
  .temp_files/audit_proofs/src/bin/prove_W1_conhost_args.rs → all four cases arrive unchanged
  (<D:\my agora\home>, <C:\my home\>, <>, <say "hi">). Dropped.
- has_marker / settings_enabled / xml_escape / text: read against the invariants; existing unit tests cover the
  edge cases (re-indented marker, prefix-only marker, trigger vs settings Enabled, %, trailing backslash). No candidate.

## Boundaries
- listed(): a task named `\<name>,x` makes listed(<name>) true (split on the first comma). It is only consulted when
  `schtasks /query /tn` already failed, and then the caller returns that error (fail closed). Proving it needs a real
  scheduled task on this machine (no seam to fake schtasks output); not done: no changes to the user's Task Scheduler.
