# Ledger: crates/agora-platform/src/conpty.rs (Round 2)
Worktree/proof crate: see ledger_winservice.md.

## Invariants
- Each argv element reaches the started program as exactly that one argument (PtyConfig::argv).
- Nothing in an argument is executed or expanded.
- The process tree is in a kill-on-close job; kill() ends the job.

## F-C1 — input-handling — High — an argument to a .cmd/.bat agent runs a second command (CVE-2024-24576 class)
- Location: conpty.rs start_process (cmdline from quote_arg, CreateProcessW/CreateProcessAsUserW with no application name).
- Impact: npm agents on Windows are .cmd shims; the agent argv carries manifest launch options (codex `-c key="value"`), MCP argv
  and worktree paths. A `"` followed by `&`/`|` in any argument, or a `&` in an unquoted path (`C:\R&D\proj`), makes cmd.exe run
  another command as the user, with the agent's environment. The standard library refuses/escapes this since 1.77.2; conpty
  bypasses std::process::Command and lost that protection.
- Root cause: CreateProcess runs a batch file through `cmd.exe /c`, which parses the line with its own quote rules (every `"`
  toggles, `\"` is not an escape, `&|<>` split commands, `%VAR%` expands). quote_arg applies only the C runtime's rules.
- Proof (before): .temp_files/audit_proofs/src/bin/prove_C1_batch_args.rs
    CONTROL  arg="plain value" script saw "ARG=\"plain value\"" marker=false
    EXPECTED: the script gets one argument "a\"&type nul>INJECTED.txt&\"b"; no INJECTED.txt is created
    ACTUAL:   INJECTED.txt created = true
    PROBLEM CONFIRMED (exit 1)
- Patch: conpty.rs — new is_batch + batch_command_line (port of std's make_bat_command_line/append_bat_arg: full-path
  `cmd.exe /e:ON /v:OFF /d /c ""script" args"`, args with any ASCII symbol or control char quoted, `"` doubled, `%` -> `%%cd:~,%`,
  CR/LF/NUL refused with InvalidInput, script with `"` or trailing `\` refused); start_process uses it when argv[0] is a batch
  file and builds the line before the sandbox ACLs are granted.
- Proof (after): .temp_files/audit_proofs/src/bin/verify_C1_batch_args.rs — 8 cases through a `%*` shim to argdump.exe (hostile
  value, spaces, empty, trailing backslash, `C:\R&D\proj`, `%PATH%`/`100%`/`!PATH!`/`^`/`|`/`<>`, codex effort option, codex MCP
  option), line break refused, plain .exe unchanged → FIX VERIFIED, 3 runs in a row, exit 0.
  Original proof → ACTUAL: INJECTED.txt created = false / PROBLEM NOT REPRODUCED.
- Regression test: crates/agora-platform/src/conpty.rs tests::an_argument_to_a_batch_file_runs_no_second_command
  (fails on the old line building: exit code Some(1) instead of Some(5); passes with the fix).

## Not reproduced (dropped)
- C-kill: kill() is a no-op once the leader exited, leaving members of the job running until the last handle drops. Same contract
  as the Unix backend (kill skipped once reaped) and the trait doc; ADR-0079 (Proposed) changes it. Not a defect today.

## Boundaries
- The process starts before AssignProcessToJobObject (not CREATE_SUSPENDED): a child spawned in that window is outside the job.
  The window is microseconds against a process start of milliseconds; no gated reproduction without a hook in the scope. Recorded only.
- make_job returning None (set or assign failed) silently drops kill-on-close and the limits; no input found that makes
  SetInformationJobObject fail, so not a finding.
