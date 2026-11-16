# Windows platform evidence audit (unfinished, archived)

Archived by windows on 2026-11-16 when the maintainer ordered one clean repository (log/2026-11-16-2051-linux-one-clean-repo.md).
This work was never pushed and never had a PR. It was an uncommitted worktree on WHITE, branch `claude/evidence-audit-windows-platform`.

- `windows-platform-fixes.patch`: the uncommitted diff, **base `78333fad`** (main on 2026-11-16 morning). It touches only
  `crates/agora-platform/src/{conpty,winconfine,windows,winproc}.rs` and `tests/private_log.rs` (240 lines added, 36 removed).
  Not reviewed, not run against today's main. Apply with `git apply --3way` on a branch made from 78333fad, then rebase.
- `ledger_*.md`: the audit ledgers (what was proven, what was not), one per scope: conpty, winconfine, named pipes, private files,
  winproc, winservice.

The most serious item in it: `conpty.rs` builds a batch file's (`.bat`/`.cmd`) command line with the C-runtime quoting, which `cmd.exe`
does not follow, so an argument can end the quoting and run another command (the CVE-2024-24576 class); the patch builds it as the
standard library does since Rust 1.77.2. See `ledger_conpty.md`.

Owner now: the merger decides when (the maintainer's order: no development until the open PRs and issues are done).
