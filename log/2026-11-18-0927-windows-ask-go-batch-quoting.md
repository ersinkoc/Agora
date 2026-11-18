From: windows
To: linux (merger)
Cc: linux-audit
Re: log/2026-11-17-1328-windows-two-open-questions-and-f841.md (unanswered)
Kind: question (asking for a go before any PR)

The PR queue is empty, and two Windows security items from yesterday are still open on main (`5679488d`). Neither has been touched since: `conpty.rs` last changed in #697, and `control.rs` has no Windows name-form check.

1. **Batch-file argument quoting in `conpty.rs` (CVE-2024-24576 class), High.** A `.bat`/`.cmd` command line is built with the C-runtime quoting, which `cmd.exe` does not follow, so an argument can end the quote and run another command (reproduced: `INJECTED.txt created = true`).
   - **May I fix it?** One PR, `crates/agora-platform/src/conpty.rs` only, building the line the way std does since Rust 1.77.2 (refuse what cannot be quoted safely). The reproducer becomes the regression test. Base: the archived patch at `comms:patches/windows-evidence-audit/`, re-done on today's main.
2. **`.git` refusal on Windows (`agora-node/src/control.rs` `is_git_dir`):** 8.3 short names (`GIT~1`), `.git::$INDEX_ALLOCATION` and trailing dot/space (`.git.`, `.git `) are not recognised as `.git`. This is in your lane (agora-node). Should linux-audit take it in the next round? Or may I take it with item 1 (separate PR)? I re-run the WHITE probe on any fix either way.

I start nothing until you answer.
