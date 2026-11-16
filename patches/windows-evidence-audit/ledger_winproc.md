# Ledger: crates/agora-platform/src/winproc.rs (Round 5)

## Invariants
- process_start_time(pid): None when no such process runs; "an ended one that was not reaped yet counts as ended"
  (lib.rs doc). Lock files compare it to tell a live holder from a dead one (agora/src/frontdoor.rs:743, :762).
- children(pid) lists only processes created after pid (reused parent ids).

## F-R1 — bug — Low — a process that ended with exit code 259 is reported as running
- Location: winproc.rs start_time (GetExitCodeProcess == STILL_ACTIVE taken as "running").
- Impact: while any handle to it is held (its parent's Child, a waiter), a process that exited with 259 keeps a start time:
  process_start_time is Some, so a lock it held reads as held. Ends when the last handle closes.
- Root cause: 259 is both STILL_ACTIVE and a valid exit code; the exit code alone cannot tell them apart.
- Proof (before): .temp_files/audit_proofs/src/bin/prove_R1_exit_259.rs
    CONTROL  exit Some(0): process_start_time = None
    EXPECTED: exit Some(259): process_start_time = None (the process is over)
    ACTUAL:   exit Some(259): process_start_time = Some(134392907263161132)
    PROBLEM CONFIRMED (exit 1)
- Patch: winproc.rs start_time opens one handle with PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE (falling back to
  query-only, Process::open, when that is refused) and also polls it (WaitForSingleObject 0 == WAIT_OBJECT_0 → ended). Without
  SYNCHRONIZE the wait fails and the old answer (running) stays. Imports WAIT_OBJECT_0, PROCESS_SYNCHRONIZE, WaitForSingleObject.
  (A first version used a second OpenProcess; replaced after the benchmark below showed its cost.)
- Cost (AGENTS.md hot paths): every live process reads 259, so this runs on every successful start_time. Callers:
  process_children → children (start_time per matching child), process_tree (agora-node daemon.rs:1934 agents_memory, every 30 s,
  MEMORY_CHECK), frontdoor.rs:743/762/1238. Not on the detection path. Bench .temp_files/audit_proofs/src/bin/bench_R1_start_time.rs,
  release, best of 5:
    origin/main          process_start_time(self) 1.47 / 1.44 us   process_tree(self, 2 children) 72.2 / 65.8 ms
    two handles (v1)     2.80 / 2.92 us                            84.2 / 91.0 ms
    one handle (final)   1.57 us                                   71.3 ms   (within the baseline's run-to-run spread)
- Proof (after): .temp_files/audit_proofs/src/bin/verify_R1_exit_259.rs (ended with 259/0/1 with handle held → None twice; a
  process blocked on stdin → Some, stable; then exits 259 → None; this process → Some) → FIX VERIFIED x3 (final version), exit 0.
  Original proof → ACTUAL: None / PROBLEM NOT REPRODUCED.
- Regression test: crates/agora-platform/src/winproc.rs tests::a_process_that_ended_with_code_259_has_no_start_time
  (fails with the wait disabled, passes with it). Low severity, promoted because conpty already guards the same 259 trap.

## Not reproduced (dropped)
- spawn_detached quoting through cmd.exe (the F-C1 class): its only caller (agora/src/frontdoor.rs:955) starts agora.exe
  itself, never a batch file. No candidate.

## Boundaries
- terminate(pid) acts on a raw pid; callers check process_start_time first (check-then-act across pid reuse). No gated
  reproduction of pid reuse is possible; recorded only.
