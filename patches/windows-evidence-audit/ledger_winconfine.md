# Ledger: crates/agora-platform/src/winconfine.rs (Round 6)
Confinement is off by default; every proof here set AGORA_WINDOWS_CONFINEMENT=1, as the crate's winsandbox tests do. Sandbox
folders had to be on C: (%TEMP%): a D: data-volume folder is refused with LabelFailed (ADR-0050 Amendment B decision 2).

## Invariants
- A confined agent cannot affect the node or other agents beyond its own folders (restricted token + per-agent SID + Low label).
- Every start/end/recovery decision about shared folders is serialized by the ledger lock; nobody else can hold that lock.

## F-K1 — ownership — Medium — a confined agent can take the ledger lock and freeze every confined start and end
- Location: winconfine.rs LedgerLock::acquire (CreateMutexW `Local\agora-acl-ledger`, no security attributes,
  WaitForSingleObject INFINITE), used by Confinement::in_ledger_dir, Confinement::drop and recover_in.
- Impact: (feature on) an agent under confinement opens the mutex (default DACL gives the logon SID, which is in its restricting
  list, SYNCHRONIZE) or, when no node holds it at that moment, creates it first; it then owns it for as long as it runs. Every
  later confined start on the machine for that user, every agent end (Drop) and recovery waits forever: a confined agent denies
  service to the node and to every other agent.
- Root cause: the lock is a named object in the session namespace, which a restricted, Low-integrity process can open and create in.
- Proof (before):
  - .temp_files/audit_proofs/src/bin/prove_K1_ledger_mutex.rs (helper .../mutexgrab.rs run confined):
      CONTROL  owner-only mutex, confined helper: open=denied err=5
      ACTUAL:   open=ok owned=true wait=0x0           → PROBLEM CONFIRMED (exit 1)
  - .temp_files/audit_proofs/src/bin/probe_K1_squat.rs (helper mutexmake.rs): confined create in Local: create=ok (squat possible).
  - .temp_files/audit_proofs/src/bin/prove_K1b_start_blocked.rs, end to end, run on the exact origin/main winconfine.rs:
      CONTROL  nobody holds the lock: start+end -> Ok(Some(6)) in 66.0335ms
      confined helper: create=ok owned=true wait=0x0
      ACTUAL:   Err(Timeout)                          → PROBLEM CONFIRMED (exit 1)
- Patch: winconfine.rs LedgerLock — File::lock on an owner-only `<ledger folder>.lock` opened with crate::open_lock_private
  (beside the folder, not in it: recover_in reads every file there); thread-local depth so the same thread may take it again (a
  failed start drops its Confinement while the start holds the lock — the named mutex was reentrant); unlockable → unlocked, as
  before when the mutex could not be made. Call sites pass the ledger folder (in_ledger_dir: dir; drop: ledger's parent;
  recover_in: ledgers).
- Proof (after):
  - .temp_files/audit_proofs/src/bin/verify_K1_ledger_lock.rs (helpers filegrab.rs, mutexgrab.rs; real confined processes):
    plain confined start ok; lock file at %LOCALAPPDATA%\agora\acl-ledger.lock; confined helper opening it → PermissionDenied;
    helper squats and owns Local\agora-acl-ledger → a second confined start and its end finish Ok(Some(6)) → FIX VERIFIED x3.
  - prove_K1b_start_blocked on the patched code → ACTUAL: Ok(Some(6)) / PROBLEM NOT REPRODUCED.
  - prove_K1_ledger_mutex still prints PROBLEM CONFIRMED by design: it checks only that the named mutex can be owned, which stays
    true; nothing waits on that mutex any more (K1b is the end-to-end proof of the defect).
- Regression test: crates/agora-platform/src/winconfine.rs
  tests::a_start_and_its_end_do_not_wait_on_a_named_lock_another_process_holds — with the old named wait put back it fails after
  60 s (Err(Timeout)); with the fix it passes in 0.02 s. The whole winconfine unit target (12) and winsandbox (12) pass, including
  the failing-start paths that need the reentrant lock.

## Not reproduced / not proven
- owner_alive (winconfine.rs ~908) also reads exit code 259 as "running" (the F-R1 trap). Reachable only through recovery of the
  real ledger folder with a node that exited with 259 while its handle is held; no proof seam outside %LOCALAPPDATA%. Not patched.

## Boundaries
- During an in-place update an old node (named mutex) and a new node (lock file) do not exclude each other until the old one ends.
  The feature is off by default.
- Dropping a confined Pty does not release its Confinement synchronously (the waiter thread holds the last Arc): a process that
  exits right after leaves a stale ledger for recovery. Seen with the proofs; recovery (recover_sandbox_leftovers) removed all five
  ledgers they left, and icacls shows no agent SID entry left on C:\Users, the profile, AppData, Local or the audit folders.
- %TEMP% carries 5 inherited Modify grants for other SIDs (next to CodexSandboxUsers). Not created by this audit (its sandboxes
  granted only their own work subfolders, and only traverse on %TEMP%); left untouched.
- The unlocked fallback now also applies when open_lock_private refuses the lock file (e.g. %LOCALAPPDATA%\agora is a link or
  another user's), not only when a lock cannot be made at all: starts then go on without the cross-node lock, as they did when
  CreateMutexW failed.
- New file in the real profile: %LOCALAPPDATA%\agora\acl-ledger.lock (made by the proofs' confined starts; the installed 0.3.13
  node does not use it).
